use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use xilem::view::worker;

use crate::app::AppState;

pub const PROTOCOL: &str = "fast-explorer-tailnet/1";
#[cfg(fastexplorer_tsnet_dynamic)]
const DLL_NAME: &str = "fast_explorer_tsnet.dll";

static STATE_DIR: OnceLock<PathBuf> = OnceLock::new();
static SHARE_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn configure_state_dir(path: PathBuf) {
    if let Some(existing) = STATE_DIR.get() {
        debug_assert_eq!(existing, &path);
        return;
    }
    let _ = STATE_DIR.set(path);
}

pub fn configure_share_root(path: PathBuf) {
    if let Some(existing) = SHARE_ROOT.get() {
        debug_assert_eq!(existing, &path);
        return;
    }
    let _ = SHARE_ROOT.set(path);
}

pub fn desktop_state_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("FastExplorer/tailscale");
    }
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config).join("fast-explorer/tailscale");
    }
    #[cfg(target_os = "windows")]
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile).join("AppData/Local/FastExplorer/tailscale");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(".config/fast-explorer/tailscale")
}

fn valid_profile_id(profile_id: &str) -> bool {
    !profile_id.is_empty()
        && profile_id.len() <= 64
        && profile_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn profile_state_dir(profile_id: &str) -> Result<PathBuf, String> {
    if !valid_profile_id(profile_id) {
        return Err("invalid Tailscale profile ID".to_owned());
    }
    STATE_DIR
        .get()
        .map(|base| base.join(profile_id))
        .ok_or_else(|| "embedded Tailscale state directory is not configured".to_owned())
}

#[derive(Debug, Clone, Deserialize)]
pub struct TailscaleStatus {
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub library_version: String,
    #[serde(default = "not_started")]
    pub state: String,
    #[serde(default)]
    pub auth_url: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub dns_name: String,
    #[serde(default)]
    pub tailnet_name: String,
    #[serde(default)]
    pub magic_dns_suffix: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub ips: Vec<String>,
    #[serde(default)]
    pub service_ready: bool,
    #[serde(default)]
    pub webdav_url: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub peers: Vec<TailscalePeer>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub taildrive_devices: Vec<TaildriveDevice>,
    #[serde(default)]
    pub taildrive_scanning: bool,
    #[serde(default)]
    pub taildrive_error: String,
    #[serde(default)]
    pub error: String,
}

fn not_started() -> String {
    "NotStarted".to_owned()
}

fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

impl Default for TailscaleStatus {
    fn default() -> Self {
        Self {
            protocol: PROTOCOL.to_owned(),
            library_version: String::new(),
            state: not_started(),
            auth_url: String::new(),
            hostname: String::new(),
            dns_name: String::new(),
            tailnet_name: String::new(),
            magic_dns_suffix: String::new(),
            ips: Vec::new(),
            service_ready: false,
            webdav_url: String::new(),
            peers: Vec::new(),
            taildrive_devices: Vec::new(),
            taildrive_scanning: false,
            taildrive_error: String::new(),
            error: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TailscalePeer {
    pub hostname: String,
    pub dns_name: String,
    pub os: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub ips: Vec<String>,
    pub online: bool,
    pub target: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaildriveDevice {
    pub id: String,
    pub hostname: String,
    pub dns_name: String,
    pub os: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub ips: Vec<String>,
    pub online: bool,
    pub target: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub shares: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaildriveListEntry {
    pub name: String,
    pub path: String,
    pub directory: bool,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub modified: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TaildriveListPayload {
    #[serde(default, deserialize_with = "null_to_default")]
    entries: Vec<TaildriveListEntry>,
    #[serde(default)]
    error: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaildriveTransferProgress {
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub bytes_done: u64,
    #[serde(default)]
    pub bytes_total: u64,
    #[serde(default)]
    pub items_done: u64,
    #[serde(default)]
    pub items_total: u64,
    #[serde(default)]
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub paused: bool,
    #[serde(default)]
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub cancelled: bool,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HelloPeer {
    pub protocol: String,
    pub hostname: String,
    pub dns_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TailscalePing {
    pub ok: bool,
    #[serde(default)]
    pub latency_ms: i64,
    #[serde(default)]
    pub remote: Option<HelloPeer>,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone)]
pub enum Command {
    Start {
        profile_id: String,
        hostname: String,
    },
    Restart {
        profile_id: String,
        hostname: String,
    },
    Refresh {
        profile_id: String,
    },
    Ping {
        profile_id: String,
        target: String,
        label: String,
    },
    TaildriveList {
        profile_id: String,
        device_id: String,
        share: String,
        path: String,
        generation: u64,
    },
    TaildriveDownload {
        profile_id: String,
        device_id: String,
        share: String,
        path: String,
        destination: PathBuf,
        display_name: String,
        source_location: PathBuf,
        transfer_id: String,
        open_after: bool,
        source_was_cut: bool,
        replace: bool,
    },
    TaildriveUpload {
        profile_id: String,
        device_id: String,
        share: String,
        path: String,
        source: PathBuf,
        source_location: PathBuf,
        source_was_cut: bool,
        replace: bool,
        transfer_id: String,
    },
    TaildriveRelay {
        transfer_id: String,
        source_profile_id: String,
        source_device_id: String,
        source_share: String,
        source_path: String,
        target_profile_id: String,
        target_device_id: String,
        target_share: String,
        target_path: String,
        display_name: String,
        target_location: PathBuf,
        source_was_cut: bool,
        replace: bool,
    },
    TaildriveMkdir {
        profile_id: String,
        device_id: String,
        share: String,
        path: String,
        source_location: PathBuf,
    },
    TaildriveDelete {
        profile_id: String,
        device_id: String,
        share: String,
        path: String,
        source_location: PathBuf,
    },
    TaildriveRename {
        profile_id: String,
        device_id: String,
        share: String,
        path: String,
        new_name: String,
        source_location: PathBuf,
    },
    Stop {
        profile_id: String,
    },
    Logout {
        profile_id: String,
    },
}

#[derive(Debug)]
pub enum Event {
    Status {
        profile_id: String,
        result: Result<Box<TailscaleStatus>, String>,
    },
    Ping {
        profile_id: String,
        label: String,
        result: Result<TailscalePing, String>,
    },
    TaildriveList {
        profile_id: String,
        device_id: String,
        share: String,
        path: String,
        generation: u64,
        result: Result<Vec<TaildriveListEntry>, String>,
    },
    TaildriveTransferProgress {
        transfer_id: String,
        progress: TaildriveTransferProgress,
    },
    TaildriveDownload {
        transfer_id: String,
        destination: PathBuf,
        display_name: String,
        source_location: PathBuf,
        open_after: bool,
        source_was_cut: bool,
        result: Result<(), String>,
    },
    TaildriveUpload {
        transfer_id: String,
        source: PathBuf,
        source_location: PathBuf,
        remote_path: String,
        source_was_cut: bool,
        result: Result<(), String>,
    },
    TaildriveRelay {
        transfer_id: String,
        target_location: PathBuf,
        display_name: String,
        source_was_cut: bool,
        result: Result<(), String>,
    },
    TaildriveMkdir {
        source_location: PathBuf,
        remote_path: String,
        result: Result<(), String>,
    },
    TaildriveDelete {
        source_location: PathBuf,
        remote_path: String,
        result: Result<(), String>,
    },
    TaildriveRename {
        source_location: PathBuf,
        remote_path: String,
        new_name: String,
        result: Result<(), String>,
    },
    Stopped {
        profile_id: String,
        result: Result<(), String>,
    },
    LoggedOut {
        profile_id: String,
        result: Result<(), String>,
    },
}

#[cfg(fastexplorer_tsnet)]
mod ffi {
    use std::ffi::{c_char, c_int};

    unsafe extern "C" {
        #[cfg(target_os = "android")]
        pub fn FE_TS_SetAndroidInterfacesJSON(value: *const c_char) -> c_int;
        pub fn FE_TS_SetShareRoot(profile_id: *const c_char, root: *const c_char) -> c_int;
        pub fn FE_TS_Start(profile_id: *const c_char, state_dir: *const c_char) -> c_int;
        pub fn FE_TS_StatusJSON(profile_id: *const c_char) -> *mut c_char;
        pub fn FE_TS_TaildriveListJSON(
            profile_id: *const c_char,
            device_id: *const c_char,
            share: *const c_char,
            remote_path: *const c_char,
        ) -> *mut c_char;
        pub fn FE_TS_TaildriveDownloadProgress(
            profile_id: *const c_char,
            device_id: *const c_char,
            share: *const c_char,
            remote_path: *const c_char,
            destination: *const c_char,
            transfer_id: *const c_char,
        ) -> c_int;
        pub fn FE_TS_TaildriveUploadProgress(
            profile_id: *const c_char,
            device_id: *const c_char,
            share: *const c_char,
            remote_path: *const c_char,
            source: *const c_char,
            transfer_id: *const c_char,
        ) -> c_int;
        pub fn FE_TS_TaildriveUploadReplaceProgress(
            profile_id: *const c_char,
            device_id: *const c_char,
            share: *const c_char,
            remote_path: *const c_char,
            source: *const c_char,
            transfer_id: *const c_char,
        ) -> c_int;
        pub fn FE_TS_TaildriveProgressJSON(transfer_id: *const c_char) -> *mut c_char;
        #[cfg(target_os = "android")]
        pub fn FE_TS_TaildriveControl(transfer_id: *const c_char, action: *const c_char) -> c_int;
        pub fn FE_TS_TaildriveMkdir(
            profile_id: *const c_char,
            device_id: *const c_char,
            share: *const c_char,
            remote_path: *const c_char,
        ) -> c_int;
        pub fn FE_TS_TaildriveDelete(
            profile_id: *const c_char,
            device_id: *const c_char,
            share: *const c_char,
            remote_path: *const c_char,
        ) -> c_int;
        pub fn FE_TS_TaildriveRename(
            profile_id: *const c_char,
            device_id: *const c_char,
            share: *const c_char,
            remote_path: *const c_char,
            new_name: *const c_char,
        ) -> c_int;
        pub fn FE_TS_PingJSON(profile_id: *const c_char, target: *const c_char) -> *mut c_char;
        pub fn FE_TS_Stop(profile_id: *const c_char);
        pub fn FE_TS_Logout(profile_id: *const c_char) -> c_int;
        pub fn FE_TS_LastError(profile_id: *const c_char) -> *mut c_char;
        pub fn FE_TS_Free(value: *mut c_char);
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
mod dynamic {
    use std::ffi::{c_char, c_int};
    use std::sync::OnceLock;

    use libloading::os::windows::{
        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, Library,
    };

    type SetShareRoot = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
    type Start = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
    type Status = unsafe extern "C" fn(*const c_char) -> *mut c_char;
    type TaildriveList = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
    ) -> *mut c_char;
    type TaildriveDownload = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
    ) -> c_int;
    type TaildriveUpload = TaildriveDownload;
    type TaildriveTransfer = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
    ) -> c_int;
    type TaildriveProgress = unsafe extern "C" fn(*const c_char) -> *mut c_char;
    type TaildrivePathMutation =
        unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *const c_char) -> c_int;
    type TaildriveRename = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
    ) -> c_int;
    type Ping = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char;
    type Stop = unsafe extern "C" fn(*const c_char);
    type Logout = unsafe extern "C" fn(*const c_char) -> c_int;
    type LastError = unsafe extern "C" fn(*const c_char) -> *mut c_char;
    type Free = unsafe extern "C" fn(*mut c_char);

    pub struct Bridge {
        _library: Library,
        pub set_share_root: SetShareRoot,
        pub start: Start,
        pub status: Status,
        pub taildrive_list: TaildriveList,
        pub taildrive_download: TaildriveDownload,
        pub taildrive_upload: TaildriveUpload,
        pub taildrive_download_progress: TaildriveTransfer,
        pub taildrive_upload_progress: TaildriveTransfer,
        pub taildrive_upload_replace_progress: TaildriveTransfer,
        pub taildrive_progress: TaildriveProgress,
        pub taildrive_mkdir: TaildrivePathMutation,
        pub taildrive_delete: TaildrivePathMutation,
        pub taildrive_rename: TaildriveRename,
        pub ping: Ping,
        pub stop: Stop,
        pub logout: Logout,
        pub last_error: LastError,
        pub free: Free,
    }

    static BRIDGE: OnceLock<Result<Bridge, String>> = OnceLock::new();

    pub fn get() -> Result<&'static Bridge, String> {
        BRIDGE.get_or_init(load).as_ref().map_err(Clone::clone)
    }

    fn load() -> Result<Bridge, String> {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let path = exe
            .parent()
            .ok_or_else(|| "FastExplorer executable has no parent directory".to_owned())?
            .join(super::DLL_NAME);
        // SAFETY: the DLL is built from FastExplorer's pinned bridge source and remains
        // loaded for the process lifetime through the Bridge owner below. Restrict dependent
        // DLL lookup to the bridge directory and Windows' safe default search directories.
        let library = unsafe {
            Library::load_with_flags(
                &path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        }
        .map_err(|error| format!("cannot load {}: {error}", path.display()))?;
        // SAFETY: names and C ABI signatures are defined by tailscale-bridge/bridge.go.
        unsafe {
            let set_share_root = *library
                .get::<SetShareRoot>(b"FE_TS_SetShareRoot\0")
                .map_err(|e| e.to_string())?;
            let start = *library
                .get::<Start>(b"FE_TS_Start\0")
                .map_err(|e| e.to_string())?;
            let status = *library
                .get::<Status>(b"FE_TS_StatusJSON\0")
                .map_err(|e| e.to_string())?;
            let taildrive_list = *library
                .get::<TaildriveList>(b"FE_TS_TaildriveListJSON\0")
                .map_err(|e| e.to_string())?;
            let taildrive_download = *library
                .get::<TaildriveDownload>(b"FE_TS_TaildriveDownload\0")
                .map_err(|e| e.to_string())?;
            let taildrive_upload = *library
                .get::<TaildriveUpload>(b"FE_TS_TaildriveUpload\0")
                .map_err(|e| e.to_string())?;
            let taildrive_download_progress = *library
                .get::<TaildriveTransfer>(b"FE_TS_TaildriveDownloadProgress\0")
                .map_err(|e| e.to_string())?;
            let taildrive_upload_progress = *library
                .get::<TaildriveTransfer>(b"FE_TS_TaildriveUploadProgress\0")
                .map_err(|e| e.to_string())?;
            let taildrive_upload_replace_progress = *library
                .get::<TaildriveTransfer>(b"FE_TS_TaildriveUploadReplaceProgress\0")
                .map_err(|e| e.to_string())?;
            let taildrive_progress = *library
                .get::<TaildriveProgress>(b"FE_TS_TaildriveProgressJSON\0")
                .map_err(|e| e.to_string())?;
            let taildrive_mkdir = *library
                .get::<TaildrivePathMutation>(b"FE_TS_TaildriveMkdir\0")
                .map_err(|e| e.to_string())?;
            let taildrive_delete = *library
                .get::<TaildrivePathMutation>(b"FE_TS_TaildriveDelete\0")
                .map_err(|e| e.to_string())?;
            let taildrive_rename = *library
                .get::<TaildriveRename>(b"FE_TS_TaildriveRename\0")
                .map_err(|e| e.to_string())?;
            let ping = *library
                .get::<Ping>(b"FE_TS_PingJSON\0")
                .map_err(|e| e.to_string())?;
            let stop = *library
                .get::<Stop>(b"FE_TS_Stop\0")
                .map_err(|e| e.to_string())?;
            let logout = *library
                .get::<Logout>(b"FE_TS_Logout\0")
                .map_err(|e| e.to_string())?;
            let last_error = *library
                .get::<LastError>(b"FE_TS_LastError\0")
                .map_err(|e| e.to_string())?;
            let free = *library
                .get::<Free>(b"FE_TS_Free\0")
                .map_err(|e| e.to_string())?;
            Ok(Bridge {
                _library: library,
                set_share_root,
                start,
                status,
                taildrive_list,
                taildrive_download,
                taildrive_upload,
                taildrive_download_progress,
                taildrive_upload_progress,
                taildrive_upload_replace_progress,
                taildrive_progress,
                taildrive_mkdir,
                taildrive_delete,
                taildrive_rename,
                ping,
                stop,
                logout,
                last_error,
                free,
            })
        }
    }
}

fn c_string(value: &str, field: &str) -> Result<std::ffi::CString, String> {
    std::ffi::CString::new(value).map_err(|_| format!("{field} contains a NUL byte"))
}

#[cfg(all(target_os = "android", fastexplorer_tsnet))]
pub fn set_android_interfaces_json(value: &str) -> Result<(), String> {
    let value = c_string(value, "Android network interfaces JSON")?;
    // SAFETY: value is a valid NUL-terminated string for this synchronous bridge call.
    if unsafe { ffi::FE_TS_SetAndroidInterfacesJSON(value.as_ptr()) } == 1 {
        Ok(())
    } else {
        Err("embedded Tailscale rejected Android network state".to_owned())
    }
}

#[cfg(all(target_os = "android", not(fastexplorer_tsnet)))]
pub fn set_android_interfaces_json(_value: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(fastexplorer_tsnet)]
fn free_bridge_string(pointer: *mut std::ffi::c_char) {
    // SAFETY: pointer was allocated by C.CString in the linked Go bridge.
    unsafe { ffi::FE_TS_Free(pointer) };
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn free_bridge_string(pointer: *mut std::ffi::c_char) {
    if let Ok(bridge) = dynamic::get() {
        // SAFETY: pointer was allocated by the currently loaded Go bridge.
        unsafe { (bridge.free)(pointer) };
    }
}

#[cfg(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic))]
fn take_bridge_string(pointer: *mut std::ffi::c_char) -> Result<String, String> {
    if pointer.is_null() {
        return Err("embedded Tailscale returned a null string".to_owned());
    }
    // SAFETY: Go bridge strings are NUL-terminated until freed below.
    let value = unsafe { std::ffi::CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    free_bridge_string(pointer);
    Ok(value)
}

#[cfg(fastexplorer_tsnet)]
fn bridge_set_share_root(
    profile: &std::ffi::CString,
    root: &std::ffi::CString,
) -> Result<(), String> {
    // SAFETY: pointers are valid for this synchronous bridge call.
    let ok = unsafe { ffi::FE_TS_SetShareRoot(profile.as_ptr(), root.as_ptr()) };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_set_share_root(
    profile: &std::ffi::CString,
    root: &std::ffi::CString,
) -> Result<(), String> {
    let bridge = dynamic::get()?;
    // SAFETY: pointers are valid and symbol signature is checked at load.
    let ok = unsafe { (bridge.set_share_root)(profile.as_ptr(), root.as_ptr()) };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_set_share_root(
    _profile: &std::ffi::CString,
    _root: &std::ffi::CString,
) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_start(profile: &std::ffi::CString, state: &std::ffi::CString) -> Result<(), String> {
    // SAFETY: pointers are valid for this synchronous FFI call.
    let ok = unsafe { ffi::FE_TS_Start(profile.as_ptr(), state.as_ptr()) };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_start(profile: &std::ffi::CString, state: &std::ffi::CString) -> Result<(), String> {
    let bridge = dynamic::get()?;
    // SAFETY: pointers are valid and symbol signature is checked at load.
    let ok = unsafe { (bridge.start)(profile.as_ptr(), state.as_ptr()) };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet)]
fn bridge_status(profile: &std::ffi::CString) -> Result<String, String> {
    // SAFETY: pointer is valid for the synchronous FFI call.
    take_bridge_string(unsafe { ffi::FE_TS_StatusJSON(profile.as_ptr()) })
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_status(profile: &std::ffi::CString) -> Result<String, String> {
    let bridge = dynamic::get()?;
    // SAFETY: pointer is valid and symbol signature is checked at load.
    take_bridge_string(unsafe { (bridge.status)(profile.as_ptr()) })
}

#[cfg(fastexplorer_tsnet)]
fn bridge_taildrive_list(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
) -> Result<String, String> {
    // SAFETY: pointers are valid for the synchronous FFI call.
    take_bridge_string(unsafe {
        ffi::FE_TS_TaildriveListJSON(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
        )
    })
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_taildrive_list(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
) -> Result<String, String> {
    let bridge = dynamic::get()?;
    // SAFETY: pointers are valid and symbol signature is checked at load.
    take_bridge_string(unsafe {
        (bridge.taildrive_list)(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
        )
    })
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_taildrive_list(
    _profile: &std::ffi::CString,
    _device: &std::ffi::CString,
    _share: &std::ffi::CString,
    _path: &std::ffi::CString,
) -> Result<String, String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_taildrive_download(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
    destination: &std::ffi::CString,
    transfer_id: &std::ffi::CString,
) -> Result<(), String> {
    let ok = unsafe {
        ffi::FE_TS_TaildriveDownloadProgress(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
            destination.as_ptr(),
            transfer_id.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_taildrive_download(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
    destination: &std::ffi::CString,
    transfer_id: &std::ffi::CString,
) -> Result<(), String> {
    let bridge = dynamic::get()?;
    let ok = unsafe {
        (bridge.taildrive_download_progress)(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
            destination.as_ptr(),
            transfer_id.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_taildrive_download(
    _profile: &std::ffi::CString,
    _device: &std::ffi::CString,
    _share: &std::ffi::CString,
    _path: &std::ffi::CString,
    _destination: &std::ffi::CString,
    _transfer_id: &std::ffi::CString,
) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_taildrive_upload(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
    source: &std::ffi::CString,
    transfer_id: &std::ffi::CString,
) -> Result<(), String> {
    let ok = unsafe {
        ffi::FE_TS_TaildriveUploadProgress(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
            source.as_ptr(),
            transfer_id.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_taildrive_upload(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
    source: &std::ffi::CString,
    transfer_id: &std::ffi::CString,
) -> Result<(), String> {
    let bridge = dynamic::get()?;
    let ok = unsafe {
        (bridge.taildrive_upload_progress)(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
            source.as_ptr(),
            transfer_id.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_taildrive_upload(
    _profile: &std::ffi::CString,
    _device: &std::ffi::CString,
    _share: &std::ffi::CString,
    _path: &std::ffi::CString,
    _source: &std::ffi::CString,
    _transfer_id: &std::ffi::CString,
) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_taildrive_upload_replace(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
    source: &std::ffi::CString,
    transfer_id: &std::ffi::CString,
) -> Result<(), String> {
    let ok = unsafe {
        ffi::FE_TS_TaildriveUploadReplaceProgress(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
            source.as_ptr(),
            transfer_id.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_taildrive_upload_replace(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
    source: &std::ffi::CString,
    transfer_id: &std::ffi::CString,
) -> Result<(), String> {
    let bridge = dynamic::get()?;
    let ok = unsafe {
        (bridge.taildrive_upload_replace_progress)(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
            source.as_ptr(),
            transfer_id.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_taildrive_upload_replace(
    _profile: &std::ffi::CString,
    _device: &std::ffi::CString,
    _share: &std::ffi::CString,
    _path: &std::ffi::CString,
    _source: &std::ffi::CString,
    _transfer_id: &std::ffi::CString,
) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_taildrive_progress(transfer_id: &std::ffi::CString) -> Result<String, String> {
    take_bridge_string(unsafe { ffi::FE_TS_TaildriveProgressJSON(transfer_id.as_ptr()) })
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_taildrive_progress(transfer_id: &std::ffi::CString) -> Result<String, String> {
    let bridge = dynamic::get()?;
    take_bridge_string(unsafe { (bridge.taildrive_progress)(transfer_id.as_ptr()) })
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_taildrive_progress(_transfer_id: &std::ffi::CString) -> Result<String, String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_taildrive_mkdir(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
) -> Result<(), String> {
    let ok = unsafe {
        ffi::FE_TS_TaildriveMkdir(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_taildrive_mkdir(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
) -> Result<(), String> {
    let bridge = dynamic::get()?;
    let ok = unsafe {
        (bridge.taildrive_mkdir)(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_taildrive_mkdir(
    _profile: &std::ffi::CString,
    _device: &std::ffi::CString,
    _share: &std::ffi::CString,
    _path: &std::ffi::CString,
) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_taildrive_delete(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
) -> Result<(), String> {
    let ok = unsafe {
        ffi::FE_TS_TaildriveDelete(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_taildrive_delete(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
) -> Result<(), String> {
    let bridge = dynamic::get()?;
    let ok = unsafe {
        (bridge.taildrive_delete)(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_taildrive_delete(
    _profile: &std::ffi::CString,
    _device: &std::ffi::CString,
    _share: &std::ffi::CString,
    _path: &std::ffi::CString,
) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_taildrive_rename(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
    new_name: &std::ffi::CString,
) -> Result<(), String> {
    let ok = unsafe {
        ffi::FE_TS_TaildriveRename(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
            new_name.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_taildrive_rename(
    profile: &std::ffi::CString,
    device: &std::ffi::CString,
    share: &std::ffi::CString,
    path: &std::ffi::CString,
    new_name: &std::ffi::CString,
) -> Result<(), String> {
    let bridge = dynamic::get()?;
    let ok = unsafe {
        (bridge.taildrive_rename)(
            profile.as_ptr(),
            device.as_ptr(),
            share.as_ptr(),
            path.as_ptr(),
            new_name.as_ptr(),
        )
    };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_taildrive_rename(
    _profile: &std::ffi::CString,
    _device: &std::ffi::CString,
    _share: &std::ffi::CString,
    _path: &std::ffi::CString,
    _new_name: &std::ffi::CString,
) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_ping(profile: &std::ffi::CString, target: &std::ffi::CString) -> Result<String, String> {
    // SAFETY: pointers are valid for the synchronous FFI call.
    take_bridge_string(unsafe { ffi::FE_TS_PingJSON(profile.as_ptr(), target.as_ptr()) })
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_ping(profile: &std::ffi::CString, target: &std::ffi::CString) -> Result<String, String> {
    let bridge = dynamic::get()?;
    // SAFETY: pointers are valid and symbol signature is checked at load.
    take_bridge_string(unsafe { (bridge.ping)(profile.as_ptr(), target.as_ptr()) })
}

#[cfg(fastexplorer_tsnet)]
fn bridge_stop(profile: &std::ffi::CString) -> Result<(), String> {
    // SAFETY: pointer is valid for this synchronous FFI call.
    unsafe { ffi::FE_TS_Stop(profile.as_ptr()) };
    Ok(())
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_stop(profile: &std::ffi::CString) -> Result<(), String> {
    let bridge = dynamic::get()?;
    // SAFETY: pointer is valid and symbol signature is checked at load.
    unsafe { (bridge.stop)(profile.as_ptr()) };
    Ok(())
}

#[cfg(fastexplorer_tsnet)]
fn bridge_logout(profile: &std::ffi::CString) -> Result<(), String> {
    // SAFETY: pointer is valid for this synchronous FFI call.
    let ok = unsafe { ffi::FE_TS_Logout(profile.as_ptr()) };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_logout(profile: &std::ffi::CString) -> Result<(), String> {
    let bridge = dynamic::get()?;
    // SAFETY: pointer is valid and symbol signature is checked at load.
    let ok = unsafe { (bridge.logout)(profile.as_ptr()) };
    if ok == 1 {
        Ok(())
    } else {
        Err(bridge_last_error(profile))
    }
}

#[cfg(fastexplorer_tsnet)]
fn bridge_last_error(profile: &std::ffi::CString) -> String {
    // SAFETY: pointer is valid for the synchronous FFI call.
    take_bridge_string(unsafe { ffi::FE_TS_LastError(profile.as_ptr()) })
        .unwrap_or_else(|_| "embedded Tailscale operation failed".to_owned())
}

#[cfg(fastexplorer_tsnet_dynamic)]
fn bridge_last_error(profile: &std::ffi::CString) -> String {
    let Ok(bridge) = dynamic::get() else {
        return "embedded Tailscale operation failed".to_owned();
    };
    // SAFETY: pointer is valid and symbol signature is checked at load.
    take_bridge_string(unsafe { (bridge.last_error)(profile.as_ptr()) })
        .unwrap_or_else(|_| "embedded Tailscale operation failed".to_owned())
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_start(_profile: &std::ffi::CString, _state: &std::ffi::CString) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_status(_profile: &std::ffi::CString) -> Result<String, String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_ping(
    _profile: &std::ffi::CString,
    _target: &std::ffi::CString,
) -> Result<String, String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_stop(_profile: &std::ffi::CString) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

#[cfg(not(any(fastexplorer_tsnet, fastexplorer_tsnet_dynamic)))]
fn bridge_logout(_profile: &std::ffi::CString) -> Result<(), String> {
    Err("embedded Tailscale is not available on this platform".to_owned())
}

pub fn save_hostname(profile_id: &str, hostname: &str) -> Result<(), String> {
    let state_dir = profile_state_dir(profile_id)?;
    std::fs::create_dir_all(&state_dir)
        .map_err(|error| format!("create Tailscale state directory: {error}"))?;
    if !hostname.is_empty() {
        std::fs::write(state_dir.join("hostname"), format!("{hostname}\n"))
            .map_err(|error| format!("save Tailscale hostname: {error}"))?;
    }
    Ok(())
}

pub fn start(profile_id: &str, hostname: &str) -> Result<(), String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    if !hostname.is_empty() {
        save_hostname(profile_id, hostname)?;
    }
    let state_dir = profile_state_dir(profile_id)?;
    std::fs::create_dir_all(&state_dir)
        .map_err(|error| format!("create Tailscale state directory: {error}"))?;
    let state = c_string(&state_dir.to_string_lossy(), "Tailscale state path")?;
    let share_root = SHARE_ROOT
        .get()
        .ok_or_else(|| "FastExplorer WebDAV share root is not configured".to_owned())?;
    let share = c_string(&share_root.to_string_lossy(), "WebDAV root")?;
    bridge_set_share_root(&profile, &share)?;
    bridge_start(&profile, &state)
}

pub fn status(profile_id: &str) -> Result<TailscaleStatus, String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    let json = bridge_status(&profile)?;
    let status: TailscaleStatus = serde_json::from_str(&json).map_err(|error| error.to_string())?;
    if status.protocol != PROTOCOL {
        return Err(format!(
            "embedded Tailscale protocol mismatch: expected {PROTOCOL}, got {}",
            status.protocol
        ));
    }
    Ok(status)
}

pub fn ping(profile_id: &str, target: &str) -> Result<TailscalePing, String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    let target = c_string(target, "Tailscale peer target")?;
    let json = bridge_ping(&profile, &target)?;
    let ping: TailscalePing = serde_json::from_str(&json).map_err(|error| error.to_string())?;
    if let Some(remote) = &ping.remote
        && remote.protocol != PROTOCOL
    {
        return Err(format!(
            "peer protocol mismatch: expected {PROTOCOL}, got {}",
            remote.protocol
        ));
    }
    Ok(ping)
}

pub fn taildrive_list(
    profile_id: &str,
    device_id: &str,
    share: &str,
    remote_path: &str,
) -> Result<Vec<TaildriveListEntry>, String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    let device = c_string(device_id, "Taildrive device ID")?;
    let share = c_string(share, "Taildrive share")?;
    let path = c_string(remote_path, "Taildrive path")?;
    let json = bridge_taildrive_list(&profile, &device, &share, &path)?;
    let payload: TaildriveListPayload =
        serde_json::from_str(&json).map_err(|error| error.to_string())?;
    if !payload.error.is_empty() {
        return Err(payload.error);
    }
    Ok(payload.entries)
}

pub fn taildrive_download(
    profile_id: &str,
    device_id: &str,
    share: &str,
    remote_path: &str,
    destination: &std::path::Path,
    transfer_id: &str,
) -> Result<(), String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    let device = c_string(device_id, "Taildrive device ID")?;
    let share = c_string(share, "Taildrive share")?;
    let path = c_string(remote_path, "Taildrive path")?;
    let destination = c_string(&destination.to_string_lossy(), "download destination")?;
    let transfer_id = c_string(transfer_id, "Taildrive transfer ID")?;
    bridge_taildrive_download(&profile, &device, &share, &path, &destination, &transfer_id)
}

pub fn taildrive_upload(
    profile_id: &str,
    device_id: &str,
    share: &str,
    remote_path: &str,
    source: &std::path::Path,
    transfer_id: &str,
) -> Result<(), String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    let device = c_string(device_id, "Taildrive device ID")?;
    let share = c_string(share, "Taildrive share")?;
    let path = c_string(remote_path, "Taildrive path")?;
    let source = c_string(&source.to_string_lossy(), "upload source")?;
    let transfer_id = c_string(transfer_id, "Taildrive transfer ID")?;
    bridge_taildrive_upload(&profile, &device, &share, &path, &source, &transfer_id)
}

pub fn taildrive_upload_replace(
    profile_id: &str,
    device_id: &str,
    share: &str,
    remote_path: &str,
    source: &std::path::Path,
    transfer_id: &str,
) -> Result<(), String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    let device = c_string(device_id, "Taildrive device ID")?;
    let share = c_string(share, "Taildrive share")?;
    let path = c_string(remote_path, "Taildrive path")?;
    let source = c_string(&source.to_string_lossy(), "upload source")?;
    let transfer_id = c_string(transfer_id, "Taildrive transfer ID")?;
    bridge_taildrive_upload_replace(&profile, &device, &share, &path, &source, &transfer_id)
}

pub fn taildrive_transfer_progress(transfer_id: &str) -> Result<TaildriveTransferProgress, String> {
    let transfer_id = c_string(transfer_id, "Taildrive transfer ID")?;
    let json = bridge_taildrive_progress(&transfer_id)?;
    serde_json::from_str(&json).map_err(|error| error.to_string())
}

#[cfg(all(target_os = "android", fastexplorer_tsnet))]
pub fn taildrive_transfer_control(transfer_id: &str, action: &str) -> Result<(), String> {
    let action_name = action.to_owned();
    let transfer_id = c_string(transfer_id, "Taildrive transfer ID")?;
    let action = c_string(action, "Taildrive transfer action")?;
    // SAFETY: both pointers are valid NUL-terminated strings for this synchronous call.
    if unsafe { ffi::FE_TS_TaildriveControl(transfer_id.as_ptr(), action.as_ptr()) } == 1 {
        Ok(())
    } else {
        Err(format!("cannot {action_name} TailDrive transfer"))
    }
}

#[cfg(all(target_os = "android", not(fastexplorer_tsnet)))]
pub fn taildrive_transfer_control(_transfer_id: &str, _action: &str) -> Result<(), String> {
    Err("embedded Tailscale transfer controls are unavailable".to_owned())
}

pub fn taildrive_mkdir(
    profile_id: &str,
    device_id: &str,
    share: &str,
    remote_path: &str,
) -> Result<(), String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    let device = c_string(device_id, "Taildrive device ID")?;
    let share = c_string(share, "Taildrive share")?;
    let path = c_string(remote_path, "Taildrive path")?;
    bridge_taildrive_mkdir(&profile, &device, &share, &path)
}

pub fn taildrive_delete(
    profile_id: &str,
    device_id: &str,
    share: &str,
    remote_path: &str,
) -> Result<(), String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    let device = c_string(device_id, "Taildrive device ID")?;
    let share = c_string(share, "Taildrive share")?;
    let path = c_string(remote_path, "Taildrive path")?;
    bridge_taildrive_delete(&profile, &device, &share, &path)
}

pub fn taildrive_rename(
    profile_id: &str,
    device_id: &str,
    share: &str,
    remote_path: &str,
    new_name: &str,
) -> Result<(), String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    let device = c_string(device_id, "Taildrive device ID")?;
    let share = c_string(share, "Taildrive share")?;
    let path = c_string(remote_path, "Taildrive path")?;
    let new_name = c_string(new_name, "Taildrive new name")?;
    bridge_taildrive_rename(&profile, &device, &share, &path, &new_name)
}

pub fn stop(profile_id: &str) -> Result<(), String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    bridge_stop(&profile)
}

pub fn logout(profile_id: &str) -> Result<(), String> {
    let profile = c_string(profile_id, "Tailscale profile ID")?;
    bridge_logout(&profile)
}

fn remove_local_path(path: &std::path::Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        std::fs::remove_file(path).map_err(|error| error.to_string())
    }
}

fn local_replacement_backup(destination: &std::path::Path) -> Result<PathBuf, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "replacement destination has no parent".to_owned())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for index in 0..1000_u32 {
        let candidate = parent.join(format!(
            ".fastexplorer-replaced-{}-{stamp}-{index}",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("cannot allocate replacement backup path".to_owned())
}

fn publish_downloaded_path(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    if !destination.exists() {
        return std::fs::rename(source, destination).map_err(|error| error.to_string());
    }
    let source_metadata = std::fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    let destination_metadata =
        std::fs::symlink_metadata(destination).map_err(|error| error.to_string())?;
    let source_dir = source_metadata.is_dir() && !source_metadata.file_type().is_symlink();
    let destination_dir =
        destination_metadata.is_dir() && !destination_metadata.file_type().is_symlink();
    if source_dir && destination_dir {
        for entry in std::fs::read_dir(source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            publish_downloaded_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
        return std::fs::remove_dir(source).map_err(|error| error.to_string());
    }
    if source_dir != destination_dir {
        return Err("cannot replace a file with a folder or a folder with a file".to_owned());
    }

    let backup = local_replacement_backup(destination)?;
    std::fs::rename(destination, &backup).map_err(|error| error.to_string())?;
    if let Err(error) = std::fs::rename(source, destination) {
        let _ = std::fs::rename(&backup, destination);
        return Err(format!("publish replacement: {error}"));
    }
    if let Err(error) = remove_local_path(&backup) {
        eprintln!(
            "FastExplorer: cannot remove replacement backup {}: {error}",
            backup.display()
        );
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| format!("embedded Tailscale worker failed: {error}"))?
}

async fn blocking_transfer<T: Send + 'static>(
    proxy: xilem::core::MessageProxy<Event>,
    transfer_id: String,
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let bridge_transfer_id = transfer_id.clone();
    blocking_transfer_mapped(
        proxy,
        transfer_id,
        bridge_transfer_id,
        |progress| progress,
        operation,
    )
    .await
}

async fn blocking_transfer_mapped<T, M>(
    proxy: xilem::core::MessageProxy<Event>,
    event_transfer_id: String,
    bridge_transfer_id: String,
    map_progress: M,
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
    M: Fn(TaildriveTransferProgress) -> TaildriveTransferProgress,
{
    let mut task = tokio::task::spawn_blocking(operation);
    let mut tick = tokio::time::interval(Duration::from_millis(150));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            result = &mut task => {
                let result = result.map_err(|error| format!("embedded Tailscale worker failed: {error}"))?;
                if let Ok(progress) = taildrive_transfer_progress(&bridge_transfer_id) {
                    let _ = proxy.message(Event::TaildriveTransferProgress {
                        transfer_id: event_transfer_id.clone(),
                        progress: map_progress(progress),
                    });
                }
                return result;
            }
            _ = tick.tick() => {
                if let Ok(progress) = taildrive_transfer_progress(&bridge_transfer_id) {
                    let _ = proxy.message(Event::TaildriveTransferProgress {
                        transfer_id: event_transfer_id.clone(),
                        progress: map_progress(progress),
                    });
                }
            }
        }
    }
}

fn relay_download_progress(mut progress: TaildriveTransferProgress) -> TaildriveTransferProgress {
    progress.phase = "Downloading".to_owned();
    progress.done = false;
    progress.error.clear();
    if progress.bytes_total > 0 {
        progress.bytes_total = progress.bytes_total.saturating_mul(2);
    } else if progress.items_total > 0 {
        progress.items_total = progress.items_total.saturating_mul(2);
    }
    progress
}

fn relay_upload_progress(mut progress: TaildriveTransferProgress) -> TaildriveTransferProgress {
    progress.phase = "Uploading".to_owned();
    progress.done = false;
    progress.error.clear();
    if progress.bytes_total > 0 {
        let phase_total = progress.bytes_total;
        progress.bytes_done = phase_total.saturating_add(progress.bytes_done.min(phase_total));
        progress.bytes_total = phase_total.saturating_mul(2);
    } else if progress.items_total > 0 {
        let phase_total = progress.items_total;
        progress.items_done = phase_total.saturating_add(progress.items_done.min(phase_total));
        progress.items_total = phase_total.saturating_mul(2);
    }
    progress
}

#[cfg(target_os = "android")]
fn blocking_transfer_sync<T, M, F>(
    event_transfer_id: &str,
    bridge_transfer_id: &str,
    map_progress: M,
    on_progress: &mut F,
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
    M: Fn(TaildriveTransferProgress) -> TaildriveTransferProgress,
    F: FnMut(&str, TaildriveTransferProgress),
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name(format!("fast-explorer-io-{bridge_transfer_id}"))
        .spawn(move || {
            let _ = sender.send(operation());
        })
        .map_err(|error| format!("cannot start transfer I/O thread: {error}"))?;
    loop {
        match receiver.recv_timeout(Duration::from_millis(150)) {
            Ok(result) => {
                if let Ok(progress) = taildrive_transfer_progress(bridge_transfer_id) {
                    on_progress(event_transfer_id, map_progress(progress));
                }
                return result;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(progress) = taildrive_transfer_progress(bridge_transfer_id) {
                    on_progress(event_transfer_id, map_progress(progress));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("transfer I/O thread stopped unexpectedly".to_owned());
            }
        }
    }
}

#[cfg(target_os = "android")]
pub(crate) fn execute_background_transfer<F>(command: Command, mut on_progress: F) -> Option<Event>
where
    F: FnMut(&str, TaildriveTransferProgress),
{
    match command {
        Command::TaildriveDownload {
            profile_id,
            device_id,
            share,
            path,
            destination,
            display_name,
            source_location,
            transfer_id,
            open_after,
            source_was_cut,
            replace,
        } => {
            let final_destination = destination.clone();
            let query_destination = if replace {
                final_destination
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(format!(".fastexplorer-download-replace-{transfer_id}"))
            } else {
                final_destination.clone()
            };
            let query_profile = profile_id;
            let query_device = device_id;
            let query_share = share;
            let query_path = path;
            let query_transfer = transfer_id.clone();
            let bridge_transfer = transfer_id.clone();
            let result = blocking_transfer_sync(
                &transfer_id,
                &bridge_transfer,
                |progress| progress,
                &mut on_progress,
                move || {
                    if replace && query_destination.exists() {
                        remove_local_path(&query_destination)?;
                    }
                    let result = taildrive_download(
                        &query_profile,
                        &query_device,
                        &query_share,
                        &query_path,
                        &query_destination,
                        &query_transfer,
                    );
                    if let Err(error) = result {
                        if replace && query_destination.exists() {
                            let _ = remove_local_path(&query_destination);
                        }
                        return Err(error);
                    }
                    if replace {
                        publish_downloaded_path(&query_destination, &final_destination)?;
                    }
                    Ok(())
                },
            );
            Some(Event::TaildriveDownload {
                transfer_id,
                destination,
                display_name,
                source_location,
                open_after,
                source_was_cut,
                result,
            })
        }
        Command::TaildriveUpload {
            profile_id,
            device_id,
            share,
            path,
            source,
            source_location,
            source_was_cut,
            replace,
            transfer_id,
        } => {
            let query_profile = profile_id;
            let query_device = device_id;
            let query_share = share;
            let query_path = path.clone();
            let query_source = source.clone();
            let query_transfer = transfer_id.clone();
            let bridge_transfer = transfer_id.clone();
            let result = blocking_transfer_sync(
                &transfer_id,
                &bridge_transfer,
                |progress| progress,
                &mut on_progress,
                move || {
                    if replace {
                        taildrive_upload_replace(
                            &query_profile,
                            &query_device,
                            &query_share,
                            &query_path,
                            &query_source,
                            &query_transfer,
                        )
                    } else {
                        taildrive_upload(
                            &query_profile,
                            &query_device,
                            &query_share,
                            &query_path,
                            &query_source,
                            &query_transfer,
                        )
                    }
                },
            );
            Some(Event::TaildriveUpload {
                transfer_id,
                source,
                source_location,
                remote_path: path,
                source_was_cut,
                result,
            })
        }
        Command::TaildriveRelay {
            transfer_id,
            source_profile_id,
            source_device_id,
            source_share,
            source_path,
            target_profile_id,
            target_device_id,
            target_share,
            target_path,
            display_name,
            target_location,
            source_was_cut,
            replace,
        } => {
            let staging_root = std::env::temp_dir()
                .join("FastExplorer")
                .join("taildrive-transfer")
                .join(&transfer_id);
            let staging_path = staging_root.join("payload");
            let _ = std::fs::remove_dir_all(&staging_root);
            let result = std::fs::create_dir_all(&staging_root)
                .map_err(|error| format!("create TailDrive transfer staging directory: {error}"))
                .and_then(|()| {
                    let bridge_id = format!("{transfer_id}-download");
                    let query_bridge_id = bridge_id.clone();
                    let download_destination = staging_path.clone();
                    blocking_transfer_sync(
                        &transfer_id,
                        &bridge_id,
                        relay_download_progress,
                        &mut on_progress,
                        move || {
                            taildrive_download(
                                &source_profile_id,
                                &source_device_id,
                                &source_share,
                                &source_path,
                                &download_destination,
                                &query_bridge_id,
                            )
                        },
                    )?;
                    let bridge_id = format!("{transfer_id}-upload");
                    let query_bridge_id = bridge_id.clone();
                    let upload_source = staging_path.clone();
                    blocking_transfer_sync(
                        &transfer_id,
                        &bridge_id,
                        relay_upload_progress,
                        &mut on_progress,
                        move || {
                            if replace {
                                taildrive_upload_replace(
                                    &target_profile_id,
                                    &target_device_id,
                                    &target_share,
                                    &target_path,
                                    &upload_source,
                                    &query_bridge_id,
                                )
                            } else {
                                taildrive_upload(
                                    &target_profile_id,
                                    &target_device_id,
                                    &target_share,
                                    &target_path,
                                    &upload_source,
                                    &query_bridge_id,
                                )
                            }
                        },
                    )
                });
            let _ = std::fs::remove_dir_all(&staging_root);
            Some(Event::TaildriveRelay {
                transfer_id,
                target_location,
                display_name,
                source_was_cut,
                result,
            })
        }
        _ => None,
    }
}

async fn send_status(proxy: &xilem::core::MessageProxy<Event>, profile_id: String) {
    let query = profile_id.clone();
    let result = blocking(move || status(&query)).await.map(Box::new);
    let _ = proxy.message(Event::Status { profile_id, result });
}

#[cfg(target_os = "android")]
fn should_refresh_status() -> bool {
    crate::android_platform::is_activity_resumed()
}

#[cfg(not(target_os = "android"))]
fn should_refresh_status() -> bool {
    true
}

pub fn network_task()
-> impl xilem::core::View<AppState, (), xilem::ViewCtx, Element = xilem::core::NoElement> {
    worker(
        |proxy, mut receiver: UnboundedReceiver<Command>| async move {
            let mut desired = HashSet::<String>::new();
            let mut started = HashSet::<String>::new();
            let (started_tx, mut started_rx) =
                tokio::sync::mpsc::unbounded_channel::<(String, Result<(), String>)>();
            let mut refresh_tick = tokio::time::interval(Duration::from_secs(2));
            refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    command = receiver.recv() => match command {
                        Some(Command::Start { profile_id, hostname }) => {
                            desired.insert(profile_id.clone());
                            let completion = started_tx.clone();
                            tokio::spawn(async move {
                                let id = profile_id.clone();
                                let result = blocking(move || start(&id, &hostname)).await;
                                let _ = completion.send((profile_id, result));
                            });
                        }
                        Some(Command::Restart { profile_id, hostname }) => {
                            desired.insert(profile_id.clone());
                            started.remove(&profile_id);
                            let completion = started_tx.clone();
                            tokio::spawn(async move {
                                let id = profile_id.clone();
                                let result = blocking(move || {
                                    stop(&id)?;
                                    start(&id, &hostname)
                                })
                                .await;
                                let _ = completion.send((profile_id, result));
                            });
                        }
                        Some(Command::Refresh { profile_id }) => {
                            let status_proxy = proxy.clone();
                            tokio::spawn(async move {
                                send_status(&status_proxy, profile_id).await;
                            });
                        }
                        Some(Command::Ping { profile_id, target, label }) => {
                            let ping_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let id = profile_id.clone();
                                let result = blocking(move || ping(&id, &target)).await;
                                let _ = ping_proxy.message(Event::Ping {
                                    profile_id,
                                    label,
                                    result,
                                });
                            });
                        }
                        Some(Command::TaildriveList {
                            profile_id,
                            device_id,
                            share,
                            path,
                            generation,
                        }) => {
                            let list_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let query_profile = profile_id.clone();
                                let query_device = device_id.clone();
                                let query_share = share.clone();
                                let query_path = path.clone();
                                let result = blocking(move || {
                                    taildrive_list(
                                        &query_profile,
                                        &query_device,
                                        &query_share,
                                        &query_path,
                                    )
                                })
                                .await;
                                let _ = list_proxy.message(Event::TaildriveList {
                                    profile_id,
                                    device_id,
                                    share,
                                    path,
                                    generation,
                                    result,
                                });
                            });
                        }
                        #[cfg(target_os = "android")]
                        Some(command @ (
                            Command::TaildriveDownload { .. }
                            | Command::TaildriveUpload { .. }
                            | Command::TaildriveRelay { .. }
                        )) => {
                            crate::android_transfer::submit_taildrive(command);
                        }
                        #[cfg(not(target_os = "android"))]
                        Some(Command::TaildriveDownload {
                            profile_id,
                            device_id,
                            share,
                            path,
                            destination,
                            display_name,
                            source_location,
                            transfer_id,
                            open_after,
                            source_was_cut,
                            replace,
                        }) => {
                            let download_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let query_profile = profile_id;
                                let query_device = device_id;
                                let query_share = share;
                                let query_path = path;
                                let final_destination = destination.clone();
                                let query_destination = if replace {
                                    final_destination
                                        .parent()
                                        .unwrap_or_else(|| std::path::Path::new("."))
                                        .join(format!(".fastexplorer-download-replace-{transfer_id}"))
                                } else {
                                    final_destination.clone()
                                };
                                let query_transfer = transfer_id.clone();
                                let result = blocking_transfer(
                                    download_proxy.clone(),
                                    transfer_id.clone(),
                                    move || {
                                        if replace && query_destination.exists() {
                                            remove_local_path(&query_destination)?;
                                        }
                                        let result = taildrive_download(
                                            &query_profile,
                                            &query_device,
                                            &query_share,
                                            &query_path,
                                            &query_destination,
                                            &query_transfer,
                                        );
                                        if let Err(error) = result {
                                            if replace && query_destination.exists() {
                                                let _ = remove_local_path(&query_destination);
                                            }
                                            return Err(error);
                                        }
                                        if replace
                                            && let Err(error) = publish_downloaded_path(
                                                &query_destination,
                                                &final_destination,
                                            )
                                        {
                                            if query_destination.exists() {
                                                let _ = remove_local_path(&query_destination);
                                            }
                                            return Err(error);
                                        }
                                        Ok(())
                                    },
                                )
                                .await;
                                let _ = download_proxy.message(Event::TaildriveDownload {
                                    transfer_id,
                                    destination,
                                    display_name,
                                    source_location,
                                    open_after,
                                    source_was_cut,
                                    result,
                                });
                            });
                        }
                        #[cfg(not(target_os = "android"))]
                        Some(Command::TaildriveUpload {
                            profile_id,
                            device_id,
                            share,
                            path,
                            source,
                            source_location,
                            source_was_cut,
                            replace,
                            transfer_id,
                        }) => {
                            let upload_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let query_profile = profile_id;
                                let query_device = device_id;
                                let query_share = share;
                                let query_path = path.clone();
                                let query_source = source.clone();
                                let query_transfer = transfer_id.clone();
                                let result = blocking_transfer(
                                    upload_proxy.clone(),
                                    transfer_id.clone(),
                                    move || {
                                        if replace {
                                            taildrive_upload_replace(
                                                &query_profile,
                                                &query_device,
                                                &query_share,
                                                &query_path,
                                                &query_source,
                                                &query_transfer,
                                            )
                                        } else {
                                            taildrive_upload(
                                                &query_profile,
                                                &query_device,
                                                &query_share,
                                                &query_path,
                                                &query_source,
                                                &query_transfer,
                                            )
                                        }
                                    },
                                )
                                .await;
                                let _ = upload_proxy.message(Event::TaildriveUpload {
                                    transfer_id,
                                    source,
                                    source_location,
                                    remote_path: path,
                                    source_was_cut,
                                    result,
                                });
                            });
                        }
                        #[cfg(not(target_os = "android"))]
                        Some(Command::TaildriveRelay {
                            transfer_id,
                            source_profile_id,
                            source_device_id,
                            source_share,
                            source_path,
                            target_profile_id,
                            target_device_id,
                            target_share,
                            target_path,
                            display_name,
                            target_location,
                            source_was_cut,
                            replace,
                        }) => {
                            let relay_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let staging_root = std::env::temp_dir()
                                    .join("FastExplorer")
                                    .join("taildrive-transfer")
                                    .join(&transfer_id);
                                let staging_path = staging_root.join("payload");
                                let _ = std::fs::remove_dir_all(&staging_root);
                                let create_result = std::fs::create_dir_all(&staging_root)
                                    .map_err(|error| format!("create TailDrive transfer staging directory: {error}"));
                                let result = match create_result {
                                    Err(error) => Err(error),
                                    Ok(()) => {
                                        let download_profile = source_profile_id.clone();
                                        let download_device = source_device_id.clone();
                                        let download_share = source_share.clone();
                                        let download_path = source_path.clone();
                                        let download_destination = staging_path.clone();
                                        let download_transfer = format!("{transfer_id}-download");
                                        let download_bridge_transfer = download_transfer.clone();
                                        let first = blocking_transfer_mapped(
                                            relay_proxy.clone(),
                                            transfer_id.clone(),
                                            download_transfer,
                                            relay_download_progress,
                                            move || {
                                                taildrive_download(
                                                    &download_profile,
                                                    &download_device,
                                                    &download_share,
                                                    &download_path,
                                                    &download_destination,
                                                    &download_bridge_transfer,
                                                )
                                            },
                                        )
                                        .await;
                                        match first {
                                            Err(error) => Err(error),
                                            Ok(()) => {
                                                let upload_profile = target_profile_id;
                                                let upload_device = target_device_id;
                                                let upload_share = target_share;
                                                let upload_path = target_path;
                                                let upload_source = staging_path.clone();
                                                let upload_transfer = format!("{transfer_id}-upload");
                                                let upload_bridge_transfer = upload_transfer.clone();
                                                blocking_transfer_mapped(
                                                    relay_proxy.clone(),
                                                    transfer_id.clone(),
                                                    upload_transfer,
                                                    relay_upload_progress,
                                                    move || {
                                                        if replace {
                                                            taildrive_upload_replace(
                                                                &upload_profile,
                                                                &upload_device,
                                                                &upload_share,
                                                                &upload_path,
                                                                &upload_source,
                                                                &upload_bridge_transfer,
                                                            )
                                                        } else {
                                                            taildrive_upload(
                                                                &upload_profile,
                                                                &upload_device,
                                                                &upload_share,
                                                                &upload_path,
                                                                &upload_source,
                                                                &upload_bridge_transfer,
                                                            )
                                                        }
                                                    },
                                                )
                                                .await
                                            }
                                        }
                                    }
                                };
                                let _ = std::fs::remove_dir_all(&staging_root);
                                let _ = relay_proxy.message(Event::TaildriveRelay {
                                    transfer_id,
                                    target_location,
                                    display_name,
                                    source_was_cut,
                                    result,
                                });
                            });
                        }
                        Some(Command::TaildriveMkdir {
                            profile_id,
                            device_id,
                            share,
                            path,
                            source_location,
                        }) => {
                            let mkdir_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let query_profile = profile_id;
                                let query_device = device_id;
                                let query_share = share;
                                let query_path = path.clone();
                                let result = blocking(move || {
                                    taildrive_mkdir(
                                        &query_profile,
                                        &query_device,
                                        &query_share,
                                        &query_path,
                                    )
                                })
                                .await;
                                let _ = mkdir_proxy.message(Event::TaildriveMkdir {
                                    source_location,
                                    remote_path: path,
                                    result,
                                });
                            });
                        }
                        Some(Command::TaildriveDelete {
                            profile_id,
                            device_id,
                            share,
                            path,
                            source_location,
                        }) => {
                            let delete_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let query_profile = profile_id;
                                let query_device = device_id;
                                let query_share = share;
                                let query_path = path.clone();
                                let result = blocking(move || {
                                    taildrive_delete(
                                        &query_profile,
                                        &query_device,
                                        &query_share,
                                        &query_path,
                                    )
                                })
                                .await;
                                let _ = delete_proxy.message(Event::TaildriveDelete {
                                    source_location,
                                    remote_path: path,
                                    result,
                                });
                            });
                        }
                        Some(Command::TaildriveRename {
                            profile_id,
                            device_id,
                            share,
                            path,
                            new_name,
                            source_location,
                        }) => {
                            let rename_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let query_profile = profile_id;
                                let query_device = device_id;
                                let query_share = share;
                                let query_path = path.clone();
                                let query_name = new_name.clone();
                                let result = blocking(move || {
                                    taildrive_rename(
                                        &query_profile,
                                        &query_device,
                                        &query_share,
                                        &query_path,
                                        &query_name,
                                    )
                                })
                                .await;
                                let _ = rename_proxy.message(Event::TaildriveRename {
                                    source_location,
                                    remote_path: path,
                                    new_name,
                                    result,
                                });
                            });
                        }
                        Some(Command::Stop { profile_id }) => {
                            desired.remove(&profile_id);
                            started.remove(&profile_id);
                            let stop_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let id = profile_id.clone();
                                let result = blocking(move || stop(&id)).await;
                                let _ = stop_proxy.message(Event::Stopped { profile_id, result });
                            });
                        }
                        Some(Command::Logout { profile_id }) => {
                            desired.remove(&profile_id);
                            started.remove(&profile_id);
                            let logout_proxy = proxy.clone();
                            tokio::spawn(async move {
                                let id = profile_id.clone();
                                let result = blocking(move || logout(&id)).await;
                                let _ = logout_proxy.message(Event::LoggedOut { profile_id, result });
                            });
                        }
                        None => break,
                    },
                    Some((profile_id, result)) = started_rx.recv() => {
                        if !desired.contains(&profile_id) {
                            continue;
                        }
                        match result {
                            Ok(()) => {
                                started.insert(profile_id.clone());
                                let status_proxy = proxy.clone();
                                tokio::spawn(async move {
                                    send_status(&status_proxy, profile_id).await;
                                });
                            }
                            Err(error) => {
                                desired.remove(&profile_id);
                                let _ = proxy.message(Event::Status {
                                    profile_id,
                                    result: Err(error),
                                });
                            }
                        }
                    }
                    _ = refresh_tick.tick() => {
                        if !should_refresh_status() {
                            continue;
                        }
                        for profile_id in started.iter().cloned() {
                            let status_proxy = proxy.clone();
                            tokio::spawn(async move {
                                send_status(&status_proxy, profile_id).await;
                            });
                        }
                    }
                }
            }
        },
        |state: &mut AppState, sender: UnboundedSender<Command>| {
            state.install_tailscale_sender(sender);
        },
        |state: &mut AppState, event| state.apply_tailscale_event(event),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_ids_are_path_safe() {
        assert!(valid_profile_id("tailnet-2"));
        assert!(!valid_profile_id("../tailnet"));
        assert!(!valid_profile_id("tail/net"));
    }

    #[test]
    fn default_status_is_disconnected_protocol_state() {
        let status = TailscaleStatus::default();
        assert_eq!(status.protocol, PROTOCOL);
        assert_eq!(status.state, "NotStarted");
        assert!(status.peers.is_empty());
    }

    #[test]
    fn status_json_accepts_tailnet_and_peer_snapshot() {
        let status: TailscaleStatus = serde_json::from_str(
            r#"{"protocol":"fast-explorer-tailnet/1","state":"Running","tailnet_name":"example.com","magic_dns_suffix":"example.ts.net","peers":[{"hostname":"phone","dns_name":"phone.example.ts.net","os":"android","ips":["100.64.0.2"],"online":true,"target":"phone.example.ts.net"}],"taildrive_devices":[{"id":"node-1","hostname":"desktop","dns_name":"desktop.example.ts.net","os":"windows","ips":["100.64.0.3"],"online":true,"target":"desktop.example.ts.net","shares":["documents","media"]}]}"#,
        )
        .expect("status JSON");
        assert_eq!(status.tailnet_name, "example.com");
        assert_eq!(status.peers.len(), 1);
        assert_eq!(status.peers[0].hostname, "phone");
        assert!(status.peers[0].online);
        assert_eq!(status.taildrive_devices.len(), 1);
        assert_eq!(status.taildrive_devices[0].hostname, "desktop");
        assert_eq!(status.taildrive_devices[0].shares, ["documents", "media"]);
    }

    #[test]
    fn relay_progress_is_monotonic_across_download_and_upload() {
        let download = relay_download_progress(TaildriveTransferProgress {
            phase: String::new(),
            bytes_done: 25,
            bytes_total: 100,
            items_done: 0,
            paused: false,
            cancelled: false,
            items_total: 0,
            done: false,
            error: String::new(),
        });
        assert_eq!(download.phase, "Downloading");
        assert_eq!(download.bytes_done, 25);
        assert_eq!(download.bytes_total, 200);

        let download_done = relay_download_progress(TaildriveTransferProgress {
            phase: String::new(),
            bytes_done: 100,
            bytes_total: 100,
            items_done: 0,
            paused: false,
            cancelled: false,
            items_total: 0,
            done: true,
            error: String::new(),
        });
        assert_eq!(download_done.bytes_done, 100);
        assert_eq!(download_done.bytes_total, 200);
        assert!(!download_done.done);

        let upload = relay_upload_progress(TaildriveTransferProgress {
            phase: String::new(),
            bytes_done: 20,
            bytes_total: 100,
            items_done: 0,
            paused: false,
            cancelled: false,
            items_total: 0,
            done: false,
            error: String::new(),
        });
        assert_eq!(upload.phase, "Uploading");
        assert_eq!(upload.bytes_done, 120);
        assert_eq!(upload.bytes_total, 200);
        assert!(upload.bytes_done >= download_done.bytes_done);
    }

    #[test]
    fn downloaded_folder_replacement_merges_and_preserves_destination_only_files() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "fast-explorer-download-replace-test-{}-{stamp}",
            std::process::id()
        ));
        let source = root.join("source");
        let destination = root.join("destination");
        std::fs::create_dir_all(&source).expect("source");
        std::fs::create_dir_all(&destination).expect("destination");
        std::fs::write(source.join("same.txt"), b"new").expect("source same");
        std::fs::write(source.join("added.txt"), b"added").expect("source added");
        std::fs::write(destination.join("same.txt"), b"old").expect("destination same");
        std::fs::write(destination.join("keep.txt"), b"keep").expect("destination keep");

        publish_downloaded_path(&source, &destination).expect("merge replacement");

        assert_eq!(std::fs::read(destination.join("same.txt")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(destination.join("added.txt")).unwrap(),
            b"added"
        );
        assert_eq!(
            std::fs::read(destination.join("keep.txt")).unwrap(),
            b"keep"
        );
        assert!(!source.exists());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn status_json_treats_null_collections_as_empty() {
        let status: TailscaleStatus = serde_json::from_str(
            r#"{"state":"Running","ips":null,"peers":[{"hostname":"phone","dns_name":"","os":"android","ips":null,"online":true,"target":"phone"}],"taildrive_devices":[{"id":"node-1","hostname":"desktop","dns_name":"","os":"android","ips":null,"online":true,"target":"desktop","shares":null}]}"#,
        )
        .expect("null collections should be accepted");
        assert!(status.ips.is_empty());
        assert!(status.peers[0].ips.is_empty());
        assert!(status.taildrive_devices[0].ips.is_empty());
        assert!(status.taildrive_devices[0].shares.is_empty());

        let empty: TailscaleStatus =
            serde_json::from_str(r#"{"state":"NeedsLogin","peers":null,"taildrive_devices":null}"#)
                .expect("null top-level collections should be accepted");
        assert!(empty.peers.is_empty());
        assert!(empty.taildrive_devices.is_empty());
    }
}
