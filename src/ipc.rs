use std::fmt;
#[cfg(any(unix, target_os = "windows"))]
use std::path::Path;
use std::path::PathBuf;

#[cfg(any(unix, target_os = "windows"))]
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use xilem::core::MessageProxy;
use xilem::tokio::sync::oneshot;
use xilem::view::task_raw;

use crate::app::AppState;
use crate::settings::{AppSettings, AppSettingsPatch};

pub const PROTOCOL: &str = "fast-explorer/1";
const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[cfg(unix)]
static OWNED_SOCKET: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

#[cfg(unix)]
fn owned_socket() -> &'static Mutex<Option<PathBuf>> {
    OWNED_SOCKET.get_or_init(|| Mutex::new(None))
}

#[cfg(unix)]
pub fn cleanup_owned_socket() {
    let Ok(mut owned) = owned_socket().lock() else {
        return;
    };
    if let Some(path) = owned.take() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(target_os = "windows")]
static WINDOWS_PIPE: OnceLock<PathBuf> = OnceLock::new();

#[cfg(target_os = "windows")]
static OWNED_WINDOWS_ENDPOINT: OnceLock<Mutex<Option<(PathBuf, PathBuf)>>> = OnceLock::new();

#[cfg(target_os = "windows")]
fn owned_windows_endpoint() -> &'static Mutex<Option<(PathBuf, PathBuf)>> {
    OWNED_WINDOWS_ENDPOINT.get_or_init(|| Mutex::new(None))
}

#[cfg(target_os = "windows")]
pub fn cleanup_owned_socket() {
    let Ok(mut owned) = owned_windows_endpoint().lock() else {
        return;
    };
    let Some((endpoint_file, pipe_path)) = owned.take() else {
        return;
    };
    if std::fs::read_to_string(&endpoint_file)
        .ok()
        .is_some_and(|value| value.trim() == pipe_path.to_string_lossy())
    {
        let _ = std::fs::remove_file(endpoint_file);
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
pub fn cleanup_owned_socket() {}

#[derive(Debug, Deserialize)]
pub struct IpcRequest {
    pub protocol: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct IpcResponse {
    pub protocol: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<IpcError>,
}

#[derive(Debug, Serialize)]
pub struct IpcError {
    pub code: &'static str,
    pub message: String,
}

pub struct IpcEvent {
    request: IpcRequest,
    respond_to: oneshot::Sender<IpcResponse>,
}

impl fmt::Debug for IpcEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IpcEvent")
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Deserialize)]
struct SetSettingsParams {
    #[serde(flatten)]
    patch: AppSettingsPatch,
    #[serde(default)]
    persist: bool,
}

#[derive(Debug, Deserialize)]
struct NavigateParams {
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    query: String,
}

#[cfg(target_os = "windows")]
fn random_windows_pipe_nonce() -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };

    let mut bytes = [0_u8; 16];
    // SAFETY: null algorithm + BCRYPT_USE_SYSTEM_PREFERRED_RNG requests the OS CSPRNG;
    // bytes is a valid writable output buffer for the duration of the call.
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(format!("BCryptGenRandom failed with NTSTATUS {status:#x}"));
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(target_os = "windows")]
fn windows_app_data_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|profile| profile.join("AppData/Local"))
        })
        .ok_or_else(|| "LOCALAPPDATA and USERPROFILE are unavailable".to_owned())?;
    Ok(base.join("FastExplorer"))
}

#[cfg(target_os = "windows")]
fn protect_windows_path(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
        SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    };

    let sid = current_windows_user_sid()?;
    let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
    let wide_sddl = sddl
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide_sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }

    let result = (|| {
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = null_mut();
        if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
            == 0
            || present == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut wide_path = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let status = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                dacl,
                null_mut(),
            )
        };
        (status == 0)
            .then_some(())
            .ok_or_else(|| format!("SetNamedSecurityInfoW failed with Win32 error {status}"))
    })();
    unsafe { LocalFree(descriptor.cast()) };
    result
}

#[cfg(target_os = "windows")]
fn publish_windows_endpoint(pipe_path: &Path) -> Result<PathBuf, String> {
    let directory = windows_app_data_dir()?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    protect_windows_path(&directory)?;
    let endpoint_file = directory.join("control-endpoint");
    std::fs::write(&endpoint_file, pipe_path.to_string_lossy().as_bytes())
        .map_err(|error| error.to_string())?;
    protect_windows_path(&endpoint_file)?;
    Ok(endpoint_file)
}

#[cfg(target_os = "windows")]
pub fn default_socket_path() -> PathBuf {
    WINDOWS_PIPE
        .get_or_init(|| {
            let sid = current_windows_user_sid().unwrap_or_else(|error| {
                eprintln!("FastExplorer IPC: cannot resolve current Windows SID: {error}");
                "unknown-user".to_owned()
            });
            let nonce = random_windows_pipe_nonce().unwrap_or_else(|rng_error| {
                eprintln!(
                    "FastExplorer IPC: OS RNG unavailable ({rng_error}); IPC endpoint is degraded"
                );
                format!("process-{}", std::process::id())
            });
            PathBuf::from(format!(r"\\.\pipe\FastExplorer-control-{sid}-{nonce}"))
        })
        .clone()
}

#[cfg(not(target_os = "windows"))]
pub fn default_socket_path() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime).join("fast-explorer/control.sock");
    }
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(state).join("fast-explorer/control.sock");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(".local/state/fast-explorer/control.sock")
}

pub fn control_task(
    socket_path: Option<PathBuf>,
) -> impl xilem::core::View<AppState, (), xilem::ViewCtx, Element = xilem::core::NoElement> {
    task_raw(
        move |proxy| run_server(socket_path.clone(), proxy),
        |state: &mut AppState, event: IpcEvent| {
            let response = handle_request(state, event.request);
            let _ = event.respond_to.send(response);
        },
    )
}
fn handle_request(state: &mut AppState, request: IpcRequest) -> IpcResponse {
    let id = request.id.clone();
    if request.protocol != PROTOCOL {
        return IpcResponse::error(
            id,
            "unsupported_protocol",
            format!("expected {PROTOCOL}, got {}", request.protocol),
        );
    }
    let result = match request.method.as_str() {
        "ping" => Ok(json!({ "protocol": PROTOCOL })),
        "get_settings" => Ok(json!({
            "effective": AppSettings::new(
                state.effective_theme_settings(),
                state.search_mode(),
                state.ui_font(),
                state.remote_cache_settings(),
                state.tailscale_profile_settings(),
            ),
            "saved": AppSettings::new(
                state.saved_theme_settings(),
                state.saved_search_mode(),
                state.saved_ui_font(),
                state.saved_remote_cache_settings(),
                state.tailscale_profile_settings(),
            ),
        })),
        "set_settings" => parse_params::<SetSettingsParams>(request.params).and_then(|params| {
            if params
                .patch
                .theme
                .intensity
                .is_some_and(|value| value > 100)
            {
                return Err((
                    "invalid_params",
                    "intensity must be between 0 and 100".to_owned(),
                ));
            }
            state.apply_theme_patch(params.patch.theme, params.persist);
            if let Some(search_mode) = params.patch.search_mode {
                state.set_search_mode(search_mode, params.persist);
            }
            if let Some(ui_font) = params.patch.ui_font {
                state.set_ui_font(ui_font, params.persist);
            }
            if let Some(remote_cache) = params.patch.remote_cache {
                state.set_remote_cache_settings(remote_cache, params.persist);
            }
            if let Some(profiles) = params.patch.tailscale_profiles {
                state.set_tailscale_profiles(profiles, params.persist);
            } else if let Some(enabled) = params.patch.tailscale_enabled {
                state.set_tailscale_enabled(enabled, params.persist);
            }
            Ok(json!({
                "effective": AppSettings::new(
                    state.effective_theme_settings(),
                    state.search_mode(),
                    state.ui_font(),
                    state.remote_cache_settings(),
                    state.tailscale_profile_settings(),
                ),
                "persisted": params.persist,
            }))
        }),
        "reload_settings" => {
            state.reload_settings();
            Ok(json!({
                "effective": AppSettings::new(
                    state.effective_theme_settings(),
                    state.search_mode(),
                    state.ui_font(),
                    state.remote_cache_settings(),
                    state.tailscale_profile_settings(),
                )
            }))
        }
        "get_state" => Ok(json!({
            "active_tab": state.active_tab_index(),
            "tab_count": state.tab_count(),
            "path": state.active_tab().current_dir,
            "selected_path": state.active_tab().selected_path,
            "search_query": state.active_tab().search_input,
            "search_active": state.active_tab().search_active,
        })),
        "navigate" => parse_params::<NavigateParams>(request.params).map(|params| {
            state.navigate_to(params.path);
            json!({ "path": state.active_tab().current_dir })
        }),
        "search" => parse_params::<SearchParams>(request.params).map(|params| {
            state.submit_search(params.query);
            json!({
                "mode": state.search_mode(),
                "status": state.active_tab().status,
                "result_count": state.active_tab().entries.len(),
            })
        }),
        "clear_search" => {
            state.clear_search();
            Ok(json!({ "status": state.active_tab().status }))
        }
        "refresh" => {
            state.refresh();
            Ok(json!({ "status": state.active_tab().status }))
        }
        "new_tab" => {
            state.new_tab();
            Ok(json!({ "active_tab": state.active_tab_index(), "tab_count": state.tab_count() }))
        }
        _ => Err((
            "method_not_found",
            format!("unknown method: {}", request.method),
        )),
    };
    match result {
        Ok(result) => IpcResponse::ok(id, result),
        Err((code, message)) => IpcResponse::error(id, code, message),
    }
}
fn parse_params<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, (&'static str, String)> {
    serde_json::from_value(value).map_err(|error| ("invalid_params", error.to_string()))
}

impl IpcResponse {
    fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            protocol: PROTOCOL,
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: &'static str, message: String) -> Self {
        Self {
            protocol: PROTOCOL,
            id,
            ok: false,
            result: None,
            error: Some(IpcError { code, message }),
        }
    }
}

#[cfg(unix)]
async fn run_server(socket_path: Option<PathBuf>, proxy: MessageProxy<IpcEvent>) {
    let Some(socket_path) = socket_path else {
        std::future::pending::<()>().await;
        return;
    };
    if let Err(error) = run_unix_server(&socket_path, proxy).await {
        eprintln!("FastExplorer IPC disabled: {error}");
    }
}

#[cfg(target_os = "windows")]
async fn run_server(pipe_path: Option<PathBuf>, proxy: MessageProxy<IpcEvent>) {
    let Some(pipe_path) = pipe_path else {
        std::future::pending::<()>().await;
        return;
    };
    if let Err(error) = run_windows_server(&pipe_path, proxy).await {
        eprintln!("FastExplorer IPC disabled: {error}");
    }
}

#[cfg(target_os = "windows")]
fn current_windows_user_sid() -> Result<String, String> {
    use std::mem::size_of;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = null_mut();
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle; token is an out parameter.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let result = (|| {
        let mut byte_len = 0u32;
        // The first call intentionally queries the required buffer size.
        unsafe {
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut byte_len);
        }
        if byte_len == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let word = size_of::<usize>();
        let mut buffer = vec![0usize; (byte_len as usize).div_ceil(word)];
        // SAFETY: buffer is pointer-aligned and at least byte_len bytes long.
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                byte_len,
                &mut byte_len,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: GetTokenInformation(TokenUser) initialized TOKEN_USER at buffer start.
        let token_user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let mut sid_text = null_mut();
        // SAFETY: token_user.User.Sid is valid while buffer is alive; API owns output allocation.
        if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_text) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut len = 0usize;
        // SAFETY: ConvertSidToStringSidW returns a NUL-terminated UTF-16 buffer.
        while unsafe { *sid_text.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: sid_text points to len initialized UTF-16 code units.
        let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid_text, len) });
        // SAFETY: ConvertSidToStringSidW documents LocalFree for the returned string.
        unsafe { LocalFree(sid_text.cast()) };
        Ok(sid)
    })();
    // SAFETY: OpenProcessToken returned this real handle.
    unsafe { CloseHandle(token) };
    result
}

#[cfg(target_os = "windows")]
fn create_private_windows_pipe(
    options: &xilem::tokio::net::windows::named_pipe::ServerOptions,
    pipe_name: &std::ffi::OsStr,
) -> std::io::Result<xilem::tokio::net::windows::named_pipe::NamedPipeServer> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    let sid = current_windows_user_sid().map_err(std::io::Error::other)?;
    let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
    let wide = sddl
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: wide is NUL terminated; descriptor is an out parameter owned by LocalFree.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: SECURITY_ATTRIBUTES and descriptor remain alive through CreateNamedPipeW.
    let result = unsafe {
        options.create_with_security_attributes_raw(
            pipe_name,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
        )
    };
    // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW uses LocalAlloc.
    unsafe { LocalFree(descriptor.cast()) };
    result
}

#[cfg(target_os = "windows")]
async fn run_windows_server(
    pipe_path: &std::path::Path,
    proxy: MessageProxy<IpcEvent>,
) -> Result<(), String> {
    use xilem::tokio::net::windows::named_pipe::ServerOptions;

    let pipe_name = pipe_path.as_os_str().to_owned();
    let mut first_options = ServerOptions::new();
    first_options
        .first_pipe_instance(true)
        .reject_remote_clients(true);
    let mut server = create_private_windows_pipe(&first_options, &pipe_name)
        .map_err(|error| error.to_string())?;

    // Publish the unpredictable endpoint only after the first pipe instance is already bound.
    // This closes the TOCTOU window where another same-user process could observe the nonce
    // and create the named pipe before FastExplorer.
    if WINDOWS_PIPE
        .get()
        .is_some_and(|default_pipe| default_pipe.as_path() == pipe_path)
    {
        let endpoint_file = publish_windows_endpoint(pipe_path)?;
        let mut owned = owned_windows_endpoint()
            .lock()
            .map_err(|_| "Windows IPC endpoint ownership lock poisoned".to_owned())?;
        *owned = Some((endpoint_file, pipe_path.to_path_buf()));
    }

    loop {
        server.connect().await.map_err(|error| error.to_string())?;
        let connected = server;
        let mut next_options = ServerOptions::new();
        next_options.reject_remote_clients(true);
        server = create_private_windows_pipe(&next_options, &pipe_name)
            .map_err(|error| error.to_string())?;
        let proxy = proxy.clone();
        xilem::tokio::spawn(async move {
            if let Err(error) = handle_client(connected, proxy).await {
                eprintln!("FastExplorer IPC client error: {error}");
            }
        });
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
async fn run_server(_socket_path: Option<PathBuf>, _proxy: MessageProxy<IpcEvent>) {
    std::future::pending::<()>().await;
}

#[cfg(unix)]
struct SocketCleanup;

#[cfg(unix)]
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        cleanup_owned_socket();
    }
}

#[cfg(unix)]
async fn run_unix_server(socket_path: &Path, proxy: MessageProxy<IpcEvent>) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use xilem::tokio::net::UnixListener;

    let parent = socket_path
        .parent()
        .ok_or_else(|| "IPC socket path has no parent".to_owned())?;
    let parent_existed = parent.exists();
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    if !parent_existed {
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    if let Ok(metadata) = std::fs::symlink_metadata(socket_path) {
        if !metadata.file_type().is_socket() {
            return Err(format!(
                "refusing to replace non-socket path: {}",
                socket_path.display()
            ));
        }
        if xilem::tokio::net::UnixStream::connect(socket_path)
            .await
            .is_ok()
        {
            return Err(format!(
                "IPC socket is already in use: {}",
                socket_path.display()
            ));
        }
        std::fs::remove_file(socket_path).map_err(|error| error.to_string())?;
    }

    let listener = UnixListener::bind(socket_path).map_err(|error| error.to_string())?;
    {
        let mut owned = owned_socket()
            .lock()
            .map_err(|_| "IPC ownership lock poisoned".to_owned())?;
        *owned = Some(socket_path.to_path_buf());
    }
    let _cleanup = SocketCleanup;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;

    loop {
        let (stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
        let proxy = proxy.clone();
        xilem::tokio::spawn(async move {
            if let Err(error) = handle_client(stream, proxy).await {
                eprintln!("FastExplorer IPC client error: {error}");
            }
        });
    }
}
async fn handle_client<S>(stream: S, proxy: MessageProxy<IpcEvent>) -> Result<(), String>
where
    S: xilem::tokio::io::AsyncRead + xilem::tokio::io::AsyncWrite + Unpin,
{
    use xilem::tokio::io::AsyncReadExt;

    let (mut read_half, mut write_half) = xilem::tokio::io::split(stream);
    let mut pending = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];
    loop {
        let read = read_half
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(());
        }
        pending.extend_from_slice(&chunk[..read]);

        while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
            let line = pending.drain(..=newline).collect::<Vec<_>>();
            if line.len() > MAX_REQUEST_BYTES {
                write_response(
                    &mut write_half,
                    &IpcResponse::error(
                        None,
                        "request_too_large",
                        "request exceeds 64 KiB".to_owned(),
                    ),
                )
                .await?;
                continue;
            }
            process_request_line(&line[..line.len() - 1], &proxy, &mut write_half).await?;
        }

        if pending.len() > MAX_REQUEST_BYTES {
            write_response(
                &mut write_half,
                &IpcResponse::error(
                    None,
                    "request_too_large",
                    "request exceeds 64 KiB".to_owned(),
                ),
            )
            .await?;
            return Ok(());
        }
    }
}

async fn process_request_line<W>(
    line: &[u8],
    proxy: &MessageProxy<IpcEvent>,
    writer: &mut W,
) -> Result<(), String>
where
    W: xilem::tokio::io::AsyncWrite + Unpin,
{
    let request = match serde_json::from_slice::<IpcRequest>(line) {
        Ok(request) => request,
        Err(error) => {
            return write_response(
                writer,
                &IpcResponse::error(None, "invalid_json", error.to_string()),
            )
            .await;
        }
    };
    let (respond_to, response) = oneshot::channel();
    proxy
        .message(IpcEvent {
            request,
            respond_to,
        })
        .map_err(|error| error.to_string())?;
    let response = response.await.map_err(|error| error.to_string())?;
    write_response(writer, &response).await
}

async fn write_response<W>(writer: &mut W, response: &IpcResponse) -> Result<(), String>
where
    W: xilem::tokio::io::AsyncWrite + Unpin,
{
    use xilem::tokio::io::AsyncWriteExt;

    let mut bytes = serde_json::to_vec(response).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .await
        .map_err(|error| error.to_string())?;
    writer.flush().await.map_err(|error| error.to_string())
}
