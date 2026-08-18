use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "android"))]
use std::process::Command;
#[cfg(target_os = "android")]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;
use xilem::masonry::peniko::{Color, ImageData};

use crate::settings::{
    AppSettings, PathOverflowBehavior, RemoteCacheSettings, SearchMode, TailnetProfileSettings,
    UiFont, automatic_tailnet_path, normalize_taildrive_device_name, normalize_tailnet_path,
};
use crate::theme::{AppearanceMode, ThemeColor, ThemePalette, ThemePatch, ThemeSettings};

const SESSION_VERSION: u32 = 1;
const TRANSFER_HISTORY_LIMIT: usize = 32;
static TRANSFER_COUNTER: AtomicU64 = AtomicU64::new(1);
static TAB_COUNTER: AtomicU64 = AtomicU64::new(1);
static TAB_GROUP_COUNTER: AtomicU64 = AtomicU64::new(1);

pub type SearchRequest = (u64, PathBuf, String, SearchMode, bool);
pub type DirectoryRequest = (u64, PathBuf, bool);
pub type ThumbnailRequest = PathBuf;

#[derive(Debug, Default)]
struct TabStripDragPreviewState {
    offsets: Vec<f64>,
    direct_slots: Vec<bool>,
    group_candidate_slot: Option<usize>,
    group_candidate_border: Option<Color>,
}

#[derive(Debug, Clone)]
pub(crate) struct TabStripDragPreviewHandle(Arc<Mutex<TabStripDragPreviewState>>);

impl Default for TabStripDragPreviewHandle {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(TabStripDragPreviewState::default())))
    }
}

impl PartialEq for TabStripDragPreviewHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for TabStripDragPreviewHandle {}

impl TabStripDragPreviewHandle {
    pub(crate) fn set_targets(&self, offsets: Vec<f64>, direct_slots: Vec<bool>) {
        if let Ok(mut preview) = self.0.lock() {
            preview.offsets = offsets;
            preview.direct_slots = direct_slots;
        }
    }

    pub(crate) fn target_for_slot(&self, slot: usize) -> (f64, bool) {
        let Ok(preview) = self.0.lock() else {
            return (0.0, false);
        };
        (
            preview.offsets.get(slot).copied().unwrap_or(0.0),
            preview.direct_slots.get(slot).copied().unwrap_or(false),
        )
    }

    pub(crate) fn set_group_candidate(&self, slot: Option<usize>, border: Option<Color>) {
        if let Ok(mut preview) = self.0.lock() {
            preview.group_candidate_slot = slot;
            preview.group_candidate_border = border;
        }
    }

    pub(crate) fn group_candidate_slot(&self) -> Option<usize> {
        self.0
            .lock()
            .ok()
            .and_then(|preview| preview.group_candidate_slot)
    }

    pub(crate) fn group_candidate_border(&self) -> Option<Color> {
        self.0
            .lock()
            .ok()
            .and_then(|preview| preview.group_candidate_border)
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut preview) = self.0.lock() {
            preview.offsets.clear();
            preview.direct_slots.clear();
            preview.group_candidate_slot = None;
            preview.group_candidate_border = None;
        }
    }
}

#[derive(Debug, Clone)]
pub enum LocalFileCommand {
    CreateDir {
        current: PathBuf,
        path: PathBuf,
    },
    CopyMove {
        current: PathBuf,
        source: PathBuf,
        destination: PathBuf,
        cut: bool,
        replace: bool,
    },
    Delete {
        current: PathBuf,
        path: PathBuf,
    },
    Rename {
        current: PathBuf,
        source: PathBuf,
        destination: PathBuf,
    },
}

#[derive(Debug)]
pub struct LocalFileEvent {
    pub command: LocalFileCommand,
    pub result: Result<(), String>,
}

#[derive(Debug, Clone)]
pub enum CacheCommand {
    Maintain {
        root: PathBuf,
        settings: RemoteCacheSettings,
        protected: BTreeSet<String>,
        usage_refresh_id: u64,
    },
    Clear {
        root: PathBuf,
        settings: RemoteCacheSettings,
        protected: BTreeSet<String>,
        usage_refresh_id: u64,
    },
    Record {
        root: PathBuf,
        source_key: String,
        destination: PathBuf,
        display_name: String,
        remote_size: u64,
        remote_modified: String,
        settings: RemoteCacheSettings,
        protected: BTreeSet<String>,
    },
    RemoveTemp {
        root: PathBuf,
        path: PathBuf,
        settings: RemoteCacheSettings,
        protected: BTreeSet<String>,
    },
}

#[derive(Debug)]
pub struct CacheEvent {
    pub result: Result<u64, String>,
    pub usage_refresh_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PersistCommand {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum RemotePreparePurpose {
    Open,
    Share,
    ImportArchive {
        current: crate::archive::ArchiveLocation,
        name: String,
        replace: bool,
    },
}

#[derive(Debug, Clone)]
pub struct RemotePrepareRequest {
    pub source: TaildriveLocation,
    pub source_location: PathBuf,
    pub display_name: String,
    pub remote_size: u64,
    pub remote_modified: String,
    pub cache_root: PathBuf,
    pub cache_settings: RemoteCacheSettings,
    pub purpose: RemotePreparePurpose,
}

#[derive(Debug)]
pub enum RemotePrepareResult {
    Cached(PathBuf),
    Download {
        destination: PathBuf,
        cache_file_name: String,
        source_key: String,
    },
}

#[derive(Debug)]
pub struct RemotePrepareEvent {
    pub request: RemotePrepareRequest,
    pub result: Result<RemotePrepareResult, String>,
}

const TAILDRIVE_ROOT_COMPONENT: &str = "__fast_explorer_taildrive__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaildriveLocation {
    Root,
    Profile {
        profile_id: String,
    },
    Device {
        profile_id: String,
        device_id: String,
    },
    Remote {
        profile_id: String,
        device_id: String,
        share: String,
        remote_path: String,
    },
}

pub fn display_path(path: &Path) -> String {
    if let Some(location) = crate::archive::parse_virtual_path(path) {
        return crate::archive::display_path(&location);
    }
    parse_taildrive_path(path)
        .map(|location| taildrive_display_path(&location))
        .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"))
}

fn encode_virtual_component(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_virtual_component(value: &str) -> Option<String> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for index in (0..value.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&value[index..index + 2], 16).ok()?);
    }
    String::from_utf8(bytes).ok()
}

pub fn taildrive_path(location: &TaildriveLocation) -> PathBuf {
    let mut path = PathBuf::from(TAILDRIVE_ROOT_COMPONENT);
    match location {
        TaildriveLocation::Root => {}
        TaildriveLocation::Profile { profile_id } => {
            path.push("p");
            path.push(encode_virtual_component(profile_id));
        }
        TaildriveLocation::Device {
            profile_id,
            device_id,
        } => {
            path.push("p");
            path.push(encode_virtual_component(profile_id));
            path.push("d");
            path.push(encode_virtual_component(device_id));
        }
        TaildriveLocation::Remote {
            profile_id,
            device_id,
            share,
            remote_path,
        } => {
            path.push("p");
            path.push(encode_virtual_component(profile_id));
            path.push("d");
            path.push(encode_virtual_component(device_id));
            path.push("s");
            path.push(encode_virtual_component(share));
            for component in remote_path
                .split('/')
                .filter(|component| !component.is_empty())
            {
                path.push(encode_virtual_component(component));
            }
        }
    }
    path
}

pub fn parse_taildrive_path(path: &Path) -> Option<TaildriveLocation> {
    let components = path
        .iter()
        .map(|c| c.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if components.first().map(String::as_str) != Some(TAILDRIVE_ROOT_COMPONENT) {
        return None;
    }
    if components.len() == 1 {
        return Some(TaildriveLocation::Root);
    }
    if components.len() < 3 || components[1] != "p" {
        return None;
    }
    let profile_id = decode_virtual_component(&components[2])?;
    if components.len() == 3 {
        return Some(TaildriveLocation::Profile { profile_id });
    }
    if components.len() < 5 || components[3] != "d" {
        return None;
    }
    let device_id = decode_virtual_component(&components[4])?;
    if components.len() == 5 {
        return Some(TaildriveLocation::Device {
            profile_id,
            device_id,
        });
    }
    if components.len() < 7 || components[5] != "s" {
        return None;
    }
    let share = decode_virtual_component(&components[6])?;
    let remote_path = components[7..]
        .iter()
        .map(|component| decode_virtual_component(component))
        .collect::<Option<Vec<_>>>()?
        .join("/");
    Some(TaildriveLocation::Remote {
        profile_id,
        device_id,
        share,
        remote_path,
    })
}

fn taildrive_profile_id(location: &TaildriveLocation) -> Option<&str> {
    match location {
        TaildriveLocation::Root => None,
        TaildriveLocation::Profile { profile_id }
        | TaildriveLocation::Device { profile_id, .. }
        | TaildriveLocation::Remote { profile_id, .. } => Some(profile_id),
    }
}

fn taildrive_parent(location: &TaildriveLocation) -> Option<TaildriveLocation> {
    match location {
        TaildriveLocation::Root => None,
        TaildriveLocation::Profile { .. } => Some(TaildriveLocation::Root),
        TaildriveLocation::Device { profile_id, .. } => Some(TaildriveLocation::Profile {
            profile_id: profile_id.clone(),
        }),
        TaildriveLocation::Remote {
            profile_id,
            device_id,
            share,
            remote_path,
        } => {
            let mut components = remote_path
                .split('/')
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>();
            if components.pop().is_some() {
                Some(TaildriveLocation::Remote {
                    profile_id: profile_id.clone(),
                    device_id: device_id.clone(),
                    share: share.clone(),
                    remote_path: components.join("/"),
                })
            } else {
                Some(TaildriveLocation::Device {
                    profile_id: profile_id.clone(),
                    device_id: device_id.clone(),
                })
            }
        }
    }
}

fn encode_taildrive_display_component(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('/', "%2F")
        .replace('\\', "%5C")
}

fn decode_taildrive_display_component(value: &str) -> String {
    value
        .replace("%5C", "\\")
        .replace("%2F", "/")
        .replace("%25", "%")
}

#[cfg(test)]
fn parse_taildrive_display_path(value: &str) -> Option<TaildriveLocation> {
    let normalized = value.trim().replace('\\', "/");
    let components = normalized
        .split('/')
        .filter(|c| !c.is_empty())
        .collect::<Vec<_>>();
    if !components
        .first()
        .is_some_and(|c| c.eq_ignore_ascii_case("TailDrive") || c.eq_ignore_ascii_case("TD"))
    {
        return None;
    }
    match components.as_slice() {
        [_] => Some(TaildriveLocation::Root),
        [_, profile_id] => Some(TaildriveLocation::Profile {
            profile_id: decode_taildrive_display_component(profile_id),
        }),
        [_, profile_id, device_id] => Some(TaildriveLocation::Device {
            profile_id: decode_taildrive_display_component(profile_id),
            device_id: decode_taildrive_display_component(device_id),
        }),
        [_, profile_id, device_id, share, rest @ ..] => Some(TaildriveLocation::Remote {
            profile_id: decode_taildrive_display_component(profile_id),
            device_id: decode_taildrive_display_component(device_id),
            share: decode_taildrive_display_component(share),
            remote_path: rest
                .iter()
                .map(|component| decode_taildrive_display_component(component))
                .collect::<Vec<_>>()
                .join("/"),
        }),
        [] => None,
    }
}

fn taildrive_display_path(location: &TaildriveLocation) -> String {
    let component = encode_taildrive_display_component;
    match location {
        TaildriveLocation::Root => "TD".to_owned(),
        TaildriveLocation::Profile { profile_id } => {
            format!("TD/{}", component(profile_id))
        }
        TaildriveLocation::Device {
            profile_id,
            device_id,
        } => format!("TD/{}/{}", component(profile_id), component(device_id)),
        TaildriveLocation::Remote {
            profile_id,
            device_id,
            share,
            remote_path,
        } => {
            let mut display = format!(
                "TD/{}/{}/{}",
                component(profile_id),
                component(device_id),
                component(share)
            );
            for remote_component in remote_path
                .split('/')
                .filter(|remote_component| !remote_component.is_empty())
            {
                display.push('/');
                display.push_str(&component(remote_component));
            }
            display
        }
    }
}

#[cfg(target_os = "android")]
static ANDROID_HOME: OnceLock<PathBuf> = OnceLock::new();
#[cfg(target_os = "android")]
static ANDROID_STATE_DIR: OnceLock<PathBuf> = OnceLock::new();
#[cfg(target_os = "android")]
static ANDROID_LEGACY_CONFIG_LOADED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "android")]
pub(crate) fn set_android_home(path: PathBuf) {
    if let Some(existing) = ANDROID_HOME.get() {
        debug_assert_eq!(existing, &path);
        return;
    }
    let _ = ANDROID_HOME.set(path);
}

#[cfg(target_os = "android")]
pub(crate) fn set_android_state_dir(path: PathBuf) {
    if let Some(existing) = ANDROID_STATE_DIR.get() {
        debug_assert_eq!(existing, &path);
        return;
    }
    let _ = ANDROID_STATE_DIR.set(path);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppPage {
    #[default]
    Files,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsEntryPoint {
    #[default]
    General,
    ActionGuide,
    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    DeviceSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Folder,
    Text,
    Image,
    Video,
    Audio,
    Archive,
    Code,
    Spreadsheet,
    Presentation,
    Json,
    Network,
    Symlink,
    Generic,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Name,
    DateModified,
    Type,
    Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

impl SortDirection {
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Ascending => "↑",
            Self::Descending => "↓",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TabGroupColor {
    #[default]
    #[serde(alias = "grey")]
    Blue,
    Red,
    Yellow,
    Green,
    Pink,
    Purple,
    Cyan,
    Orange,
}

impl TabGroupColor {
    pub const ALL: [Self; 8] = [
        Self::Blue,
        Self::Red,
        Self::Yellow,
        Self::Green,
        Self::Pink,
        Self::Purple,
        Self::Cyan,
        Self::Orange,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Blue => "Blue",
            Self::Red => "Red",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Pink => "Pink",
            Self::Purple => "Purple",
            Self::Cyan => "Cyan",
            Self::Orange => "Orange",
        }
    }

    fn palette_index(self) -> usize {
        Self::ALL
            .iter()
            .position(|color| *color == self)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabGroupState {
    id: u64,
    name: String,
    color: TabGroupColor,
    collapsed: bool,
}

impl TabGroupState {
    pub const fn id(&self) -> u64 {
        self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub const fn color(&self) -> TabGroupColor {
        self.color
    }
    pub const fn collapsed(&self) -> bool {
        self.collapsed
    }
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub modified_sort_key: u64,
    pub remote: Option<TaildriveLocation>,
    pub remote_modified: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardMode {
    Copy,
    Cut,
}

#[derive(Debug, Clone)]
struct FileClipboard {
    path: PathBuf,
    name: String,
    kind: EntryKind,
    size: u64,
    remote_modified: Option<String>,
    mode: ClipboardMode,
}

#[derive(Debug, Clone)]
struct PendingDeleteConfirmation {
    path: PathBuf,
    name: String,
}

#[derive(Debug, Clone)]
struct PendingPasteConflict {
    clipboard: FileClipboard,
    target_location: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasteConflictResolution {
    Replace,
    KeepBoth,
}

#[derive(Debug, Clone)]
pub struct FileTransferProgress {
    pub transfer_id: String,
    pub label: String,
    pub phase: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub items_done: u64,
    pub items_total: u64,
    pub paused: bool,
    pub cancelling: bool,
    pub cancelled: bool,
    pub done: bool,
    pub error: Option<String>,
    started_at: Instant,
    last_sample_at: Instant,
    last_sample_bytes: u64,
    bytes_per_second: f64,
}

impl FileTransferProgress {
    pub fn fraction(&self) -> f64 {
        if self.done && self.error.is_none() {
            return 1.0;
        }
        if self.bytes_total > 0 {
            return (self.bytes_done as f64 / self.bytes_total as f64).clamp(0.0, 1.0);
        }
        if self.items_total > 0 {
            return (self.items_done as f64 / self.items_total as f64).clamp(0.0, 1.0);
        }
        0.0
    }

    pub fn detail_text(&self) -> String {
        if let Some(error) = self.error.as_ref() {
            return format!("Failed — {error}");
        }
        if self.cancelled {
            return "Cancelled".to_owned();
        }
        if self.cancelling {
            return "Stopping…".to_owned();
        }
        if self.paused {
            return "Paused".to_owned();
        }
        if self.done {
            if self.bytes_total > 0 {
                return format!("Completed — {}", format_size(self.bytes_total));
            }
            return "Completed".to_owned();
        }
        if self.bytes_total > 0 {
            let percent = (self.fraction() * 100.0).round() as u64;
            let base = format!(
                "{percent}% · {} / {}",
                format_size(self.bytes_done),
                format_size(self.bytes_total)
            );
            let elapsed = self.started_at.elapsed().as_secs_f64();
            let rate = if self.bytes_per_second > 1.0 {
                self.bytes_per_second
            } else if elapsed >= 1.0 && self.bytes_done > 0 {
                self.bytes_done as f64 / elapsed
            } else {
                0.0
            };
            if rate > 1.0 && self.bytes_done < self.bytes_total {
                let remaining = (self.bytes_total - self.bytes_done) as f64 / rate;
                return format!("{base} · {} left", format_eta(remaining));
            }
            return base;
        }
        if self.bytes_done > 0 {
            return format!("{} transferred", format_size(self.bytes_done));
        }
        if self.items_total > 0 {
            return format!("{} / {} items", self.items_done, self.items_total);
        }
        "Starting…".to_owned()
    }

    pub fn display_detail_text(&self) -> String {
        let detail = self.detail_text();
        if self.error.is_some() || self.cancelled || self.cancelling || self.paused || self.done {
            return detail;
        }
        if detail == "Starting…" {
            return format!("{}…", self.phase.trim_end_matches('…'));
        }
        format!("{} · {detail}", self.phase)
    }
}

impl FileEntry {
    pub fn category(&self) -> FileCategory {
        if matches!(
            self.remote,
            Some(TaildriveLocation::Profile { .. } | TaildriveLocation::Device { .. })
        ) {
            return FileCategory::Network;
        }
        match self.kind {
            EntryKind::Directory => return FileCategory::Folder,
            EntryKind::Symlink => return FileCategory::Symlink,
            EntryKind::Other => return FileCategory::Other,
            EntryKind::File => {}
        }
        let extension = Path::new(&self.name)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "txt" | "md" | "rtf" | "log" | "ini" | "cfg" => FileCategory::Text,
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "heic" | "avif" => {
                FileCategory::Image
            }
            "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" => FileCategory::Video,
            "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "opus" => FileCategory::Audio,
            "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" => FileCategory::Archive,
            "rs" | "c" | "h" | "cpp" | "hpp" | "py" | "js" | "ts" | "tsx" | "jsx" | "java"
            | "kt" | "go" | "zig" | "html" | "css" | "sh" | "ps1" => FileCategory::Code,
            "xls" | "xlsx" | "xlsm" | "ods" | "csv" => FileCategory::Spreadsheet,
            "ppt" | "pptx" | "odp" => FileCategory::Presentation,
            "json" | "jsonc" | "toml" | "yaml" | "yml" | "xml" => FileCategory::Json,
            _ => FileCategory::Generic,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self.category() {
            FileCategory::Folder => "File folder",
            FileCategory::Text => "Text document",
            FileCategory::Image => "Image",
            FileCategory::Video => "Video",
            FileCategory::Audio => "Audio",
            FileCategory::Archive => "Archive",
            FileCategory::Code => "Source file",
            FileCategory::Spreadsheet => "Spreadsheet",
            FileCategory::Presentation => "Presentation",
            FileCategory::Json => "Structured data",
            FileCategory::Network => "Network location",
            FileCategory::Symlink => "Symbolic link",
            FileCategory::Generic => "File",
            FileCategory::Other => "Other",
        }
    }

    pub fn size_label(&self) -> String {
        if self.kind == EntryKind::Directory {
            return String::new();
        }
        format_size(self.size)
    }
}

#[derive(Debug, Clone)]
pub struct TabState {
    id: u64,
    group_id: Option<u64>,
    scroll_generation: u64,
    pub current_dir: PathBuf,
    pub address_input: String,
    pub entries: Vec<FileEntry>,
    pub status: String,
    pub show_hidden: bool,
    pub sort_field: SortField,
    pub sort_direction: SortDirection,
    pub search_input: String,
    pub search_active: bool,
    pub search_field_expanded: bool,
    pub selected_path: Option<PathBuf>,
    pub rename_input: Option<String>,
    rename_replace_on_type: bool,
    rename_keyboard_suffix: Option<String>,
    pending_remote_folder: Option<PathBuf>,
    pending_remote_delete: Option<(PathBuf, Instant)>,
    last_click: Option<(PathBuf, Instant)>,
    typeahead_buffer: String,
    last_typeahead: Option<Instant>,
    back_stack: Vec<PathBuf>,
    forward_stack: Vec<PathBuf>,
    restore_validation_pending: bool,
    restore_warning: Option<String>,
}

impl Default for TabState {
    fn default() -> Self {
        let current_dir = home_dir()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"));
        Self::from_path(current_dir)
    }
}
impl TabState {
    fn from_path(current_dir: PathBuf) -> Self {
        let mut tab = Self {
            id: TAB_COUNTER.fetch_add(1, Ordering::Relaxed),
            group_id: None,
            scroll_generation: 0,
            address_input: display_path(&current_dir),
            current_dir,
            entries: Vec::new(),
            status: "Ready".to_owned(),
            show_hidden: false,
            sort_field: SortField::Name,
            sort_direction: SortDirection::Ascending,
            search_input: String::new(),
            search_active: false,
            search_field_expanded: false,
            selected_path: None,
            rename_input: None,
            rename_replace_on_type: false,
            rename_keyboard_suffix: None,
            pending_remote_folder: None,
            pending_remote_delete: None,
            last_click: None,
            typeahead_buffer: String::new(),
            last_typeahead: None,
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            restore_validation_pending: false,
            restore_warning: None,
        };
        if parse_taildrive_path(&tab.current_dir).is_none() {
            #[cfg(test)]
            tab.reload_current();
            #[cfg(not(test))]
            {
                tab.status = "Loading folder…".to_owned();
            }
        } else {
            tab.status = "TailDrive".to_owned();
        }
        tab
    }

    fn from_saved(saved: SavedTab) -> Self {
        let current_dir = saved.current_dir;
        let restore_validation_pending = parse_taildrive_path(&current_dir).is_none();
        let mut tab = Self {
            id: TAB_COUNTER.fetch_add(1, Ordering::Relaxed),
            group_id: saved.group_id,
            scroll_generation: 0,
            address_input: display_path(&current_dir),
            current_dir,
            entries: Vec::new(),
            status: "Restored".to_owned(),
            show_hidden: saved.show_hidden,
            sort_field: saved.sort_field,
            sort_direction: saved.sort_direction,
            search_input: String::new(),
            search_active: false,
            search_field_expanded: false,
            selected_path: None,
            rename_input: None,
            rename_replace_on_type: false,
            rename_keyboard_suffix: None,
            pending_remote_folder: None,
            pending_remote_delete: None,
            last_click: None,
            typeahead_buffer: String::new(),
            last_typeahead: None,
            back_stack: saved.back_stack,
            forward_stack: saved.forward_stack,
            restore_validation_pending,
            restore_warning: None,
        };
        if crate::archive::parse_virtual_path(&tab.current_dir).is_some() {
            tab.status = "Loading restored archive…".to_owned();
        } else if parse_taildrive_path(&tab.current_dir).is_none() {
            tab.status = "Loading restored folder…".to_owned();
        } else {
            tab.status = "Connecting to TailDrive…".to_owned();
            tab.restore_validation_pending = false;
        }
        tab
    }

    fn saved(&self) -> SavedTab {
        SavedTab {
            group_id: self.group_id,
            current_dir: self.current_dir.clone(),
            show_hidden: self.show_hidden,
            sort_field: self.sort_field,
            sort_direction: self.sort_direction,
            back_stack: self.back_stack.clone(),
            forward_stack: self.forward_stack.clone(),
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub const fn group_id(&self) -> Option<u64> {
        self.group_id
    }

    pub(crate) fn scroll_reset_key(&self) -> u64 {
        self.id.rotate_left(17) ^ self.scroll_generation
    }

    pub fn title(&self) -> String {
        if let Some(location) = parse_taildrive_path(&self.current_dir) {
            let raw = match location {
                TaildriveLocation::Root => "TailDrive".to_owned(),
                TaildriveLocation::Profile { profile_id } => profile_id,
                TaildriveLocation::Device { device_id, .. } => device_id,
                TaildriveLocation::Remote {
                    share, remote_path, ..
                } => remote_path
                    .rsplit('/')
                    .find(|part| !part.is_empty())
                    .map(str::to_owned)
                    .unwrap_or(share),
            };
            return raw;
        }
        if let Some(location) = crate::archive::parse_virtual_path(&self.current_dir) {
            return crate::archive::title(&location);
        }
        if home_dir().as_ref() == Some(&self.current_dir) {
            return "Home".to_owned();
        }
        if self.current_dir.parent().is_none() {
            return "/".to_owned();
        }
        self.current_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| display_path(&self.current_dir))
    }
    pub(crate) fn apply_sort(&mut self) {
        let field = self.sort_field;
        let direction = self.sort_direction;
        self.entries.sort_by(|a, b| {
            let folder_order =
                (b.kind == EntryKind::Directory).cmp(&(a.kind == EntryKind::Directory));
            if folder_order != std::cmp::Ordering::Equal {
                return folder_order;
            }
            let primary = match field {
                SortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortField::DateModified => a.modified_sort_key.cmp(&b.modified_sort_key),
                SortField::Type => a
                    .kind_label()
                    .to_lowercase()
                    .cmp(&b.kind_label().to_lowercase()),
                SortField::Size => a.size.cmp(&b.size),
            };
            let primary = if direction == SortDirection::Descending {
                primary.reverse()
            } else {
                primary
            };
            primary.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    fn can_go_back(&self) -> bool {
        !self.back_stack.is_empty()
    }

    fn can_go_forward(&self) -> bool {
        !self.forward_stack.is_empty()
    }

    fn set_address_input(&mut self, value: String) {
        self.address_input = value;
    }

    fn set_search_input(&mut self, value: String) {
        self.search_input = value;
    }

    fn clear_search(&mut self) {
        self.search_input.clear();
        self.search_active = false;
        self.reload_current();
    }

    #[cfg(test)]
    fn submit_address(&mut self, value: String) {
        self.address_input = value.clone();
        let raw = value.trim();
        let path = if raw == "~" {
            home_dir().unwrap_or_else(|| self.current_dir.clone())
        } else if let Some(relative) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
            home_dir()
                .map(|home| home.join(relative))
                .unwrap_or_else(|| self.current_dir.join(raw))
        } else {
            let candidate = PathBuf::from(raw);
            if candidate.is_absolute() {
                candidate
            } else {
                self.current_dir.join(candidate)
            }
        };
        self.navigate_to(path);
    }

    #[cfg(test)]
    fn navigate_to(&mut self, path: PathBuf) {
        if !path.is_dir() {
            self.status = format!("Not a directory: {}", display_path(&path));
            self.address_input = display_path(&self.current_dir);
            return;
        }
        if path == self.current_dir {
            self.reload_current();
            return;
        }
        self.back_stack.push(self.current_dir.clone());
        self.forward_stack.clear();
        self.set_current_dir(path);
    }

    #[cfg(test)]
    fn go_back(&mut self) {
        if let Some(path) = self.back_stack.pop() {
            self.forward_stack.push(self.current_dir.clone());
            self.set_current_dir(path);
        }
    }

    #[cfg(test)]
    fn go_forward(&mut self) {
        if let Some(path) = self.forward_stack.pop() {
            self.back_stack.push(self.current_dir.clone());
            self.set_current_dir(path);
        }
    }

    fn select_entry(&mut self, path: PathBuf) {
        let name = self
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.name.clone())
            .or_else(|| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| display_path(&path));
        if self.selected_path.as_ref() != Some(&path) {
            self.pending_remote_delete = None;
        }
        self.selected_path = Some(path);
        self.status = format!("Selected: {name}");
    }

    #[cfg(test)]
    fn click_entry(&mut self, path: PathBuf) {
        let now = Instant::now();
        let is_double_click = self
            .last_click
            .as_ref()
            .is_some_and(|(last_path, last_time)| {
                last_path == &path && now.duration_since(*last_time) <= Duration::from_millis(500)
            });
        self.select_entry(path.clone());
        if is_double_click {
            self.last_click = None;
            self.activate_entry(path);
        } else {
            self.last_click = Some((path, now));
        }
    }

    #[cfg(test)]
    fn activate_entry(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.navigate_to(path);
            return;
        }

        self.select_entry(path.clone());
        let name = path.file_name().map_or_else(
            || display_path(&path),
            |name| name.to_string_lossy().into_owned(),
        );
        self.status = match open_path_with_system(&path) {
            Ok(()) => format!("Opened: {name}"),
            Err(error) => format!("Cannot open {name}: {error}"),
        };
    }

    fn fallback_restored_location(&mut self, original: &Path, reason: &str) -> PathBuf {
        let fallback = default_directory();
        self.current_dir = fallback.clone();
        self.address_input = display_path(&fallback);
        self.entries.clear();
        self.selected_path = None;
        self.search_input.clear();
        self.search_active = false;
        self.rename_input = None;
        self.restore_validation_pending = false;
        self.restore_warning = Some(format!(
            "Restored location is unavailable: {} ({reason}) — opened {} instead",
            display_path(original),
            display_path(&fallback)
        ));
        fallback
    }

    fn set_current_dir(&mut self, path: PathBuf) {
        self.current_dir = path;
        self.scroll_generation = self.scroll_generation.wrapping_add(1);
        self.address_input = display_path(&self.current_dir);
        self.restore_validation_pending = false;
        self.restore_warning = None;
        self.search_input.clear();
        self.search_active = false;
        self.selected_path = None;
        self.rename_input = None;
        self.rename_replace_on_type = false;
        self.rename_keyboard_suffix = None;
        self.pending_remote_folder = None;
        self.pending_remote_delete = None;
        self.last_click = None;
        self.typeahead_buffer.clear();
        self.last_typeahead = None;
        if parse_taildrive_path(&self.current_dir).is_none() {
            self.reload_current();
        } else {
            self.entries.clear();
            self.status = "Loading TailDrive…".to_owned();
        }
    }

    fn reload_current(&mut self) {
        let read_dir = match fs::read_dir(&self.current_dir) {
            Ok(read_dir) => read_dir,
            Err(error) => {
                self.entries.clear();
                self.status = format!("Cannot read directory: {error}");
                return;
            }
        };

        let mut entries = Vec::new();
        for entry in read_dir.flatten() {
            if let Some(item) = file_entry(entry.path(), self.show_hidden) {
                entries.push(item);
            }
        }
        entries.sort_by(|a, b| {
            let a_dir = a.kind == EntryKind::Directory;
            let b_dir = b.kind == EntryKind::Directory;
            b_dir
                .cmp(&a_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.status = format!("{} items", entries.len());
        self.entries = entries;
        self.apply_sort();
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionState {
    version: u32,
    active_tab: usize,
    tabs: Vec<SavedTab>,
    #[serde(default)]
    tab_groups: Vec<TabGroupState>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedTab {
    #[serde(default)]
    group_id: Option<u64>,
    #[serde(default)]
    current_dir: PathBuf,
    show_hidden: bool,
    #[serde(default)]
    sort_field: SortField,
    #[serde(default)]
    sort_direction: SortDirection,
    back_stack: Vec<PathBuf>,
    forward_stack: Vec<PathBuf>,
}

const TAILDRIVE_DIRECTORY_CACHE_VERSION: u32 = 4;
const TAILDRIVE_DIRECTORY_CACHE_LIMIT: usize = 32;
static TAILDRIVE_DIRECTORY_CACHE_MEMORY: OnceLock<Mutex<TaildriveDirectoryCache>> = OnceLock::new();
static TAILDRIVE_DIRECTORY_CACHE_WRITER: OnceLock<
    std::sync::mpsc::Sender<(PathBuf, TaildriveDirectoryCache)>,
> = OnceLock::new();

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TaildriveDirectoryCache {
    version: u32,
    directories: BTreeMap<String, CachedTaildriveDirectory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedTaildriveDirectory {
    updated_unix_ms: u64,
    entries: Vec<CachedTaildriveEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedTaildriveEntry {
    name: String,
    remote_path: String,
    directory: bool,
    size: u64,
    #[serde(default)]
    modified: String,
}

const REMOTE_OPEN_CACHE_VERSION: u32 = 1;
const REMOTE_CACHE_LIMIT_STEPS_MIB: &[u32] = &[128, 256, 512, 1024, 2048, 4096, 8192];
const REMOTE_CACHE_EXPIRATION_STEPS_HOURS: &[u32] = &[1, 6, 12, 24, 72, 168, 720];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RemoteOpenCacheIndex {
    version: u32,
    entries: BTreeMap<String, RemoteOpenCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemoteOpenCacheEntry {
    file_name: String,
    display_name: String,
    remote_size: u64,
    #[serde(default)]
    remote_modified: String,
    cached_unix_ms: u64,
    last_accessed_unix_ms: u64,
}

#[derive(Debug, Clone)]
struct PendingRemoteCacheDownload {
    source_key: String,
    file_name: String,
    display_name: String,
    remote_size: u64,
    remote_modified: String,
    purpose: RemotePreparePurpose,
}

#[derive(Debug, Clone)]
pub struct TailnetProfileState {
    pub config: TailnetProfileSettings,
    pub status: crate::tailscale::TailscaleStatus,
    pub ping_status: String,
}

impl TailnetProfileState {
    fn from_config(config: TailnetProfileSettings) -> Self {
        Self {
            config,
            status: crate::tailscale::TailscaleStatus::default(),
            ping_status: String::new(),
        }
    }
}

fn generated_tailscale_hostname(profile_id: &str) -> String {
    let mut hash = 0x811c9dc5_u32;
    for byte in profile_id.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    format!("fe-{:06x}", hash & 0x00ff_ffff)
}

fn normalize_tailscale_hostname(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len().min(32));
    let mut previous_dash = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        let next = if ch.is_ascii_alphanumeric() { ch } else { '-' };
        if next == '-' {
            if previous_dash || normalized.is_empty() {
                continue;
            }
            previous_dash = true;
        } else {
            previous_dash = false;
        }
        normalized.push(next);
        if normalized.len() >= 32 {
            break;
        }
    }
    while normalized.ends_with('-') {
        normalized.pop();
    }
    normalized
}

fn default_taildrive_device_name(device: &crate::tailscale::TaildriveDevice) -> String {
    let candidate = if !device.hostname.trim().is_empty() {
        device.hostname.as_str()
    } else if !device.dns_name.trim().is_empty() {
        device
            .dns_name
            .split('.')
            .next()
            .unwrap_or(&device.dns_name)
    } else if !device.target.trim().is_empty() {
        device.target.split('.').next().unwrap_or(&device.target)
    } else {
        device.id.as_str()
    };
    let compact = normalize_taildrive_device_name(candidate);
    if compact.is_empty() {
        normalize_taildrive_device_name(&device.id)
    } else {
        compact
    }
}

fn taildrive_device_stable_name(base: &str, device_id: &str) -> String {
    let mut hash = 0x811c9dc5_u32;
    for byte in device_id.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    let suffix = format!("-{:06x}", hash & 0x00ff_ffff);
    let keep = 24usize.saturating_sub(suffix.chars().count()).max(1);
    let mut stem = normalize_taildrive_device_name(base)
        .chars()
        .take(keep)
        .collect::<String>();
    stem = stem
        .trim_end_matches(|ch: char| ch == '-' || ch.is_whitespace())
        .to_owned();
    if stem.is_empty() {
        stem = "d".to_owned();
    }
    format!("{stem}{suffix}")
}

fn taildrive_device_name_with_suffix(base: &str, suffix: usize) -> String {
    let suffix_text = format!("-{suffix}");
    let keep = 24usize.saturating_sub(suffix_text.chars().count()).max(1);
    let mut stem = base.chars().take(keep).collect::<String>();
    stem = stem
        .trim_end_matches(|ch: char| ch == '-' || ch.is_whitespace())
        .to_owned();
    if stem.is_empty() {
        stem = "d".to_owned();
    }
    format!("{stem}{suffix_text}")
}

fn taildrive_device_path_names(profile: &TailnetProfileState) -> BTreeMap<String, String> {
    let devices = profile
        .status
        .taildrive_devices
        .iter()
        .map(|device| (device.id.clone(), device))
        .collect::<BTreeMap<_, _>>();
    let mut device_ids = devices.keys().cloned().collect::<BTreeSet<_>>();
    device_ids.extend(profile.config.device_names.keys().cloned());

    let mut used_names = BTreeSet::new();
    let mut names = BTreeMap::new();
    for device_id in device_ids {
        let configured = profile
            .config
            .device_names
            .get(&device_id)
            .map(|name| normalize_taildrive_device_name(name))
            .filter(|name| !name.is_empty());
        let mut base = if let Some(configured) = configured {
            configured
        } else if let Some(device) = devices.get(&device_id) {
            taildrive_device_stable_name(&default_taildrive_device_name(device), &device_id)
        } else {
            taildrive_device_stable_name("d", &device_id)
        };
        if base.is_empty() || matches!(base.as_str(), "." | "..") {
            base = taildrive_device_stable_name("d", &device_id);
        }

        let mut candidate = base.clone();
        let mut suffix = 2usize;
        while used_names.contains(&candidate.to_lowercase()) {
            candidate = taildrive_device_name_with_suffix(&base, suffix);
            suffix += 1;
        }
        used_names.insert(candidate.to_lowercase());
        names.insert(device_id, candidate);
    }
    names
}

#[derive(Debug)]
pub struct AppState {
    tabs: Vec<TabState>,
    active_tab: usize,
    tab_groups: Vec<TabGroupState>,
    tab_context_menu_tab_id: Option<u64>,
    tab_group_editor_id: Option<u64>,
    tab_group_scroll_anchor_id: Option<u64>,
    tab_group_scroll_anchor_epoch: u64,
    tab_overlay_anchor_x: f64,
    tab_strip_drag_preview: TabStripDragPreviewHandle,
    tab_strip_drag_preview_active: bool,
    page: AppPage,
    settings_entry_point: SettingsEntryPoint,
    persistence_enabled: bool,
    persistence_sender: Option<tokio::sync::mpsc::UnboundedSender<PersistCommand>>,
    theme_settings: ThemeSettings,
    saved_theme_settings: ThemeSettings,
    theme_overrides: ThemePatch,
    search_mode: SearchMode,
    saved_search_mode: SearchMode,
    search_override: Option<SearchMode>,
    ui_font: UiFont,
    saved_ui_font: UiFont,
    remote_cache_settings: RemoteCacheSettings,
    saved_remote_cache_settings: RemoteCacheSettings,
    path_overflow_behavior: PathOverflowBehavior,
    path_reset_delay_ms: u32,
    remote_cache_usage_bytes: u64,
    pending_remote_cache_downloads: BTreeMap<String, PendingRemoteCacheDownload>,
    pending_upload_info: BTreeMap<String, (EntryKind, u64)>,
    pending_temporary_uploads: BTreeSet<PathBuf>,
    file_clipboard: Option<FileClipboard>,
    pending_delete_confirmation: Option<PendingDeleteConfirmation>,
    pending_paste_conflict: Option<PendingPasteConflict>,
    paste_conflict_resolution: Option<PasteConflictResolution>,
    confirm_mobile_delete: bool,
    delete_warning_suppressed_until_ms: u64,
    file_transfers: Vec<FileTransferProgress>,
    transfer_popup_open: bool,
    sort_popup_open: bool,
    file_more_popup_open: bool,
    system_dark: bool,
    context_actions_visible: bool,
    pinned_paths: Vec<PathBuf>,
    tailscale_profiles: Vec<TailnetProfileState>,
    tailnet_path_drafts: BTreeMap<String, String>,
    taildrive_device_name_drafts: BTreeMap<(String, String), String>,
    supabase: crate::supabase_sync::UiState,
    tailscale_offline_peers_expanded: BTreeSet<String>,
    tailscale_offline_devices_expanded: BTreeSet<String>,
    tailscale_sender: Option<tokio::sync::mpsc::UnboundedSender<crate::tailscale::Command>>,
    directory_sender: Option<tokio::sync::mpsc::UnboundedSender<DirectoryRequest>>,
    directory_generation: u64,
    directory_request_started_at: Option<Instant>,
    archive_sender: Option<tokio::sync::mpsc::UnboundedSender<crate::archive::Command>>,
    archive_generation: u64,
    local_file_sender: Option<tokio::sync::mpsc::UnboundedSender<LocalFileCommand>>,
    cache_sender: Option<tokio::sync::mpsc::UnboundedSender<CacheCommand>>,
    remote_prepare_sender: Option<tokio::sync::mpsc::UnboundedSender<RemotePrepareRequest>>,
    remote_prepare_pending: BTreeSet<String>,
    remote_cache_usage_pending: Option<u64>,
    remote_cache_usage_next_request_id: u64,
    remote_cache_usage_refresh_queued: bool,
    thumbnail_sender: Option<tokio::sync::mpsc::UnboundedSender<ThumbnailRequest>>,
    thumbnail_cache: BTreeMap<PathBuf, Option<ImageData>>,
    thumbnail_pending: BTreeSet<PathBuf>,
    search_sender: Option<tokio::sync::mpsc::UnboundedSender<SearchRequest>>,
    search_generation: u64,
    taildrive_generation: u64,
    remote_mutations: Vec<PathBuf>,
    #[cfg(target_os = "windows")]
    explorer_replacement_enabled: bool,
    #[cfg(target_os = "android")]
    android_app: Option<AndroidApp>,
    #[cfg(target_os = "android")]
    android_storage_access: bool,
    #[cfg(target_os = "android")]
    android_insets: crate::android_platform::SystemBarInsets,
    #[cfg(target_os = "android")]
    android_window_width_dp: f64,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(ThemePatch::default(), None)
    }
}

#[cfg(any(target_os = "android", test))]
fn mobile_primary_action_capacity_for_width(
    window_width_dp: f64,
    inset_left: f64,
    inset_right: f64,
) -> usize {
    // Keep this in sync with file_action_icon_button. One slot is always
    // reserved for the overflow button so it can never be pushed off-screen.
    const MOBILE_ACTION_SLOT_WIDTH_DP: f64 = 48.0;
    let usable_width = (window_width_dp - inset_left - inset_right - 8.0).max(0.0);
    let total_slots = (usable_width / MOBILE_ACTION_SLOT_WIDTH_DP).floor() as usize;
    total_slots.saturating_sub(1).min(10)
}

impl AppState {
    pub fn new(theme_overrides: ThemePatch, search_override: Option<SearchMode>) -> Self {
        let mut state = Self::load_session().unwrap_or_else(Self::fresh);
        let saved = load_settings().unwrap_or_default().migrate_legacy();
        state.saved_theme_settings = saved.theme;
        state.theme_overrides = theme_overrides;
        state.theme_settings = theme_overrides.apply(saved.theme);
        state.saved_search_mode = saved.search_mode;
        state.search_override = search_override;
        let requested_search_mode = search_override.unwrap_or(saved.search_mode);
        state.search_mode = if requested_search_mode == SearchMode::Everything
            && !crate::search::everything_available()
        {
            SearchMode::Default
        } else {
            requested_search_mode
        };
        state.saved_ui_font = saved.ui_font;
        state.ui_font = saved.ui_font;
        state.remote_cache_settings = saved.remote_cache;
        state.saved_remote_cache_settings = saved.remote_cache;
        state.path_overflow_behavior = saved.path_overflow_behavior;
        state.path_reset_delay_ms = saved.path_reset_delay_ms.clamp(500, 10_000);
        state.confirm_mobile_delete = saved.confirm_mobile_delete;
        state.delete_warning_suppressed_until_ms = saved.delete_warning_suppressed_until_ms;
        state.pinned_paths = saved.pinned_paths.clone();
        state.supabase.settings = saved.supabase.clone();
        if state.supabase.settings.device_id.trim().is_empty() {
            state.supabase.settings.device_id = uuid::Uuid::new_v4().to_string();
        }
        if state.supabase.settings.device_name.trim().is_empty() {
            state.supabase.settings.device_name = default_sync_device_name();
        }
        state.supabase.email_input = state.supabase.settings.email.clone();
        state.supabase.status = if state.supabase.settings.is_configured() {
            "Supabase configured; restoring sign-in…".to_owned()
        } else {
            "Supabase is not configured".to_owned()
        };
        state.tailscale_profiles = saved
            .tailscale_profiles
            .into_iter()
            .map(TailnetProfileState::from_config)
            .collect();
        state.refresh_taildrive_address_inputs();
        for tab in &mut state.tabs {
            let Some(location) = parse_taildrive_path(&tab.current_dir) else {
                continue;
            };
            if matches!(location, TaildriveLocation::Remote { .. }) {
                let cached = load_taildrive_directory_cache_entries(&location, tab.show_hidden);
                let cached_count = cached.len();
                tab.entries = cached;
                tab.apply_sort();
                tab.status = if cached_count == 0 {
                    "Connecting to TailDrive…".to_owned()
                } else {
                    format!("Reconnecting to TailDrive… Showing {cached_count} cached item(s).")
                };
            } else {
                tab.status = "Connecting to TailDrive…".to_owned();
            }
        }
        state.system_dark = detect_system_dark();
        state
    }

    #[cfg(target_os = "android")]
    pub fn attach_android_app(&mut self, app: AndroidApp) {
        self.android_storage_access = crate::android_platform::has_storage_access(&app);
        self.android_insets = crate::android_platform::system_bar_insets(&app);
        self.android_window_width_dp = crate::android_platform::window_width_dp(&app);
        if let Ok(snapshot) = crate::android_platform::network_interfaces_json(&app)
            && let Err(error) = crate::tailscale::set_android_interfaces_json(&snapshot)
        {
            eprintln!("FastExplorer: cannot initialize Tailscale Android network state: {error}");
        }

        if !self.android_storage_access {
            if let Some(private) = app.external_data_path() {
                let shared = home_dir().unwrap_or_else(|| private.clone());
                for tab in &mut self.tabs {
                    if tab.current_dir.starts_with(&private) {
                        let show_hidden = tab.show_hidden;
                        *tab = TabState::from_path(shared.clone());
                        tab.show_hidden = show_hidden;
                        tab.status = "Loading folder…".to_owned();
                    }
                }
            }
        }
        self.android_app = Some(app);
        if crate::android_transfer::has_active_transfers() {
            self.ensure_android_transfer_service();
        }
        self.poll_android_transfers();
        self.refresh_remote_cache_usage();
        match self.persist_session_result() {
            Ok(()) => remove_legacy_android_session(),
            Err(error) => eprintln!("FastExplorer: failed to save Android session: {error}"),
        }
    }

    fn fresh() -> Self {
        Self {
            tabs: vec![TabState::default()],
            active_tab: 0,
            tab_groups: Vec::new(),
            tab_context_menu_tab_id: None,
            tab_group_editor_id: None,
            tab_group_scroll_anchor_id: None,
            tab_group_scroll_anchor_epoch: 0,
            tab_overlay_anchor_x: 8.0,
            tab_strip_drag_preview: TabStripDragPreviewHandle::default(),
            tab_strip_drag_preview_active: false,
            page: AppPage::Files,
            settings_entry_point: SettingsEntryPoint::General,
            persistence_enabled: true,
            persistence_sender: None,
            theme_settings: ThemeSettings::default(),
            saved_theme_settings: ThemeSettings::default(),
            theme_overrides: ThemePatch::default(),
            search_mode: SearchMode::Default,
            saved_search_mode: SearchMode::Default,
            search_override: None,
            ui_font: UiFont::System,
            saved_ui_font: UiFont::System,
            remote_cache_settings: RemoteCacheSettings::default(),
            saved_remote_cache_settings: RemoteCacheSettings::default(),
            path_overflow_behavior: PathOverflowBehavior::default(),
            path_reset_delay_ms: 3000,
            remote_cache_usage_bytes: 0,
            pending_remote_cache_downloads: BTreeMap::new(),
            pending_upload_info: BTreeMap::new(),
            pending_temporary_uploads: BTreeSet::new(),
            file_clipboard: None,
            pending_delete_confirmation: None,
            pending_paste_conflict: None,
            paste_conflict_resolution: None,
            confirm_mobile_delete: true,
            delete_warning_suppressed_until_ms: 0,
            file_transfers: Vec::new(),
            transfer_popup_open: false,
            sort_popup_open: false,
            file_more_popup_open: false,
            system_dark: detect_system_dark(),
            context_actions_visible: false,
            pinned_paths: Vec::new(),
            tailscale_profiles: Vec::new(),
            tailnet_path_drafts: BTreeMap::new(),
            taildrive_device_name_drafts: BTreeMap::new(),
            supabase: crate::supabase_sync::UiState::default(),
            tailscale_offline_peers_expanded: BTreeSet::new(),
            tailscale_offline_devices_expanded: BTreeSet::new(),
            tailscale_sender: None,
            directory_sender: None,
            directory_generation: 0,
            directory_request_started_at: None,
            archive_sender: None,
            archive_generation: 0,
            local_file_sender: None,
            cache_sender: None,
            remote_prepare_sender: None,
            remote_prepare_pending: BTreeSet::new(),
            remote_cache_usage_pending: None,
            remote_cache_usage_next_request_id: 0,
            remote_cache_usage_refresh_queued: false,
            thumbnail_sender: None,
            thumbnail_cache: BTreeMap::new(),
            thumbnail_pending: BTreeSet::new(),
            search_sender: None,
            search_generation: 0,
            taildrive_generation: 0,
            remote_mutations: Vec::new(),
            #[cfg(target_os = "windows")]
            explorer_replacement_enabled: crate::windows_integration::is_registered(),
            #[cfg(target_os = "android")]
            android_app: None,
            #[cfg(target_os = "android")]
            android_storage_access: false,
            #[cfg(target_os = "android")]
            android_insets: crate::android_platform::SystemBarInsets::default(),
            #[cfg(target_os = "android")]
            android_window_width_dp: 360.0,
        }
    }

    fn load_session() -> Option<Self> {
        let path = session_path()?;
        if let Some(state) = Self::load_session_from(&path) {
            return Some(state);
        }
        #[cfg(target_os = "android")]
        {
            let legacy = legacy_android_session_path()?;
            if legacy != path {
                return Self::load_session_from(&legacy);
            }
        }
        None
    }

    fn load_session_from(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        let session: SessionState = serde_json::from_str(&text).ok()?;
        if session.version != SESSION_VERSION || session.tabs.is_empty() {
            return None;
        }
        let SessionState {
            version: _,
            active_tab: saved_active_tab,
            tabs: saved_tabs,
            tab_groups,
        } = session;
        let valid_groups = tab_groups
            .iter()
            .map(|group| group.id)
            .collect::<BTreeSet<_>>();
        let mut tabs = saved_tabs
            .into_iter()
            .map(TabState::from_saved)
            .collect::<Vec<_>>();
        for tab in &mut tabs {
            if tab.group_id.is_some_and(|id| !valid_groups.contains(&id)) {
                tab.group_id = None;
            }
        }
        if let Some(max_id) = tab_groups.iter().map(|group| group.id).max() {
            TAB_GROUP_COUNTER.fetch_max(max_id.saturating_add(1), Ordering::Relaxed);
        }
        let active_tab = saved_active_tab.min(tabs.len() - 1);
        let state = Self {
            tabs,
            active_tab,
            tab_groups,
            tab_context_menu_tab_id: None,
            tab_group_editor_id: None,
            tab_group_scroll_anchor_id: None,
            tab_group_scroll_anchor_epoch: 0,
            tab_overlay_anchor_x: 8.0,
            tab_strip_drag_preview: TabStripDragPreviewHandle::default(),
            tab_strip_drag_preview_active: false,
            page: AppPage::Files,
            settings_entry_point: SettingsEntryPoint::General,
            persistence_enabled: true,
            persistence_sender: None,
            theme_settings: ThemeSettings::default(),
            saved_theme_settings: ThemeSettings::default(),
            theme_overrides: ThemePatch::default(),
            search_mode: SearchMode::Default,
            saved_search_mode: SearchMode::Default,
            search_override: None,
            ui_font: UiFont::System,
            saved_ui_font: UiFont::System,
            remote_cache_settings: RemoteCacheSettings::default(),
            saved_remote_cache_settings: RemoteCacheSettings::default(),
            path_overflow_behavior: PathOverflowBehavior::default(),
            path_reset_delay_ms: 3000,
            remote_cache_usage_bytes: 0,
            pending_remote_cache_downloads: BTreeMap::new(),
            pending_upload_info: BTreeMap::new(),
            pending_temporary_uploads: BTreeSet::new(),
            file_clipboard: None,
            pending_delete_confirmation: None,
            pending_paste_conflict: None,
            paste_conflict_resolution: None,
            confirm_mobile_delete: true,
            delete_warning_suppressed_until_ms: 0,
            file_transfers: Vec::new(),
            transfer_popup_open: false,
            sort_popup_open: false,
            file_more_popup_open: false,
            system_dark: false,
            context_actions_visible: false,
            pinned_paths: Vec::new(),
            tailscale_profiles: Vec::new(),
            tailnet_path_drafts: BTreeMap::new(),
            taildrive_device_name_drafts: BTreeMap::new(),
            supabase: crate::supabase_sync::UiState::default(),
            tailscale_offline_peers_expanded: BTreeSet::new(),
            tailscale_offline_devices_expanded: BTreeSet::new(),
            tailscale_sender: None,
            directory_sender: None,
            directory_generation: 0,
            directory_request_started_at: None,
            archive_sender: None,
            archive_generation: 0,
            local_file_sender: None,
            cache_sender: None,
            remote_prepare_sender: None,
            remote_prepare_pending: BTreeSet::new(),
            remote_cache_usage_pending: None,
            remote_cache_usage_next_request_id: 0,
            remote_cache_usage_refresh_queued: false,
            thumbnail_sender: None,
            thumbnail_cache: BTreeMap::new(),
            thumbnail_pending: BTreeSet::new(),
            search_sender: None,
            search_generation: 0,
            taildrive_generation: 0,
            remote_mutations: Vec::new(),
            #[cfg(target_os = "windows")]
            explorer_replacement_enabled: crate::windows_integration::is_registered(),
            #[cfg(target_os = "android")]
            android_app: None,
            #[cfg(target_os = "android")]
            android_storage_access: false,
            #[cfg(target_os = "android")]
            android_insets: crate::android_platform::SystemBarInsets::default(),
            #[cfg(target_os = "android")]
            android_window_width_dp: 360.0,
        };
        #[cfg(test)]
        let mut state = state;
        #[cfg(test)]
        if crate::archive::parse_virtual_path(&state.active_tab().current_dir).is_none()
            && parse_taildrive_path(&state.active_tab().current_dir).is_none()
        {
            state.request_directory_reload();
        }
        Some(state)
    }

    fn snapshot(&self) -> SessionState {
        SessionState {
            version: SESSION_VERSION,
            active_tab: self.active_tab,
            tabs: self.tabs.iter().map(TabState::saved).collect(),
            tab_groups: self.tab_groups.clone(),
        }
    }

    fn persist_session_result(&self) -> Result<(), String> {
        if !self.persistence_enabled {
            return Ok(());
        }
        let Some(path) = session_path() else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(&self.snapshot()).map_err(|error| error.to_string())?;
        #[cfg(test)]
        if self.persistence_sender.is_none() {
            return write_bytes_atomic(&path, &bytes);
        }
        let Some(sender) = self.persistence_sender.as_ref() else {
            return Ok(());
        };
        sender
            .send(PersistCommand { path, bytes })
            .map_err(|_| "persistence worker stopped".to_owned())
    }

    pub fn persist_session(&self) {
        if let Err(error) = self.persist_session_result() {
            eprintln!("FastExplorer: failed to save session: {error}");
        }
    }

    fn persist_mobile_browsing_state(&self) {
        #[cfg(target_os = "android")]
        self.persist_session();
    }

    fn persist_settings(&self) {
        if !self.persistence_enabled {
            return;
        }
        let Some(path) = config_path() else {
            return;
        };
        let mut settings = AppSettings::new(
            self.saved_theme_settings,
            self.saved_search_mode,
            self.saved_ui_font,
            self.saved_remote_cache_settings,
            self.tailscale_profile_settings(),
        );
        settings.path_overflow_behavior = self.path_overflow_behavior;
        settings.path_reset_delay_ms = self.path_reset_delay_ms;
        settings.supabase = self.supabase.settings.clone();
        settings.pinned_paths = self.pinned_paths.clone();
        settings.confirm_mobile_delete = self.confirm_mobile_delete;
        settings.delete_warning_suppressed_until_ms = self.delete_warning_suppressed_until_ms;
        let bytes = match serde_json::to_vec_pretty(&settings) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("FastExplorer: failed to serialize config: {error}");
                return;
            }
        };
        #[cfg(test)]
        if self.persistence_sender.is_none() {
            if let Err(error) = write_bytes_atomic(&path, &bytes) {
                eprintln!("FastExplorer: failed to save config: {error}");
            }
            return;
        }
        if let Some(sender) = self.persistence_sender.as_ref() {
            let _ = sender.send(PersistCommand { path, bytes });
        }
    }

    #[cfg(test)]
    fn save_session_to(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(&self.snapshot()).map_err(|error| error.to_string())?;
        write_bytes_atomic(path, &bytes)
    }

    pub fn tabs(&self) -> &[TabState] {
        &self.tabs
    }

    pub fn tab_title(&self, index: usize) -> String {
        let Some(tab) = self.tabs.get(index) else {
            return String::new();
        };
        let Some(location) = parse_taildrive_path(&tab.current_dir) else {
            return tab.title();
        };
        match location {
            TaildriveLocation::Root => "TD".to_owned(),
            TaildriveLocation::Profile { profile_id } => self.tailnet_path(&profile_id),
            TaildriveLocation::Device {
                profile_id,
                device_id,
            } => self.taildrive_device_name(&profile_id, &device_id),
            TaildriveLocation::Remote {
                share, remote_path, ..
            } => remote_path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .map(str::to_owned)
                .unwrap_or(share),
        }
    }

    pub fn tab_groups(&self) -> &[TabGroupState] {
        &self.tab_groups
    }

    pub fn tab_group(&self, group_id: u64) -> Option<&TabGroupState> {
        self.tab_groups.iter().find(|group| group.id == group_id)
    }

    pub fn tab_context_menu_tab_id(&self) -> Option<u64> {
        self.tab_context_menu_tab_id
    }

    pub fn tab_group_editor_id(&self) -> Option<u64> {
        self.tab_group_editor_id
    }

    pub(crate) fn tab_group_scroll_anchor(&self) -> Option<(u64, u64)> {
        self.tab_group_scroll_anchor_id
            .filter(|group_id| self.tab_group(*group_id).is_some())
            .map(|group_id| (group_id, self.tab_group_scroll_anchor_epoch))
    }

    pub fn tab_overlay_anchor_x(&self) -> f64 {
        self.tab_overlay_anchor_x
    }

    pub(crate) fn tab_strip_drag_preview_handle(&self) -> TabStripDragPreviewHandle {
        self.tab_strip_drag_preview.clone()
    }

    pub(crate) fn tab_strip_drag_preview_active(&self) -> bool {
        self.tab_strip_drag_preview_active
    }

    pub(crate) fn begin_tab_strip_drag_preview(&mut self) {
        // The source widget writes the first prospective offsets before this
        // action reaches AppState. Do not clear them here or the first preview
        // frame would briefly jump back to the original layout.
        self.tab_strip_drag_preview_active = true;
    }

    pub(crate) fn end_tab_strip_drag_preview(&mut self) {
        self.tab_strip_drag_preview_active = false;
        self.tab_strip_drag_preview.clear();
    }

    pub fn open_tab_context_menu_at(&mut self, tab_id: u64, anchor_x: f64) {
        self.tab_overlay_anchor_x = anchor_x.max(8.0);
        self.open_tab_context_menu(tab_id);
    }

    pub fn open_tab_context_menu(&mut self, tab_id: u64) {
        if self.tabs.iter().any(|tab| tab.id == tab_id) {
            self.file_more_popup_open = false;
            self.sort_popup_open = false;
            self.transfer_popup_open = false;
            self.tab_group_editor_id = None;
            self.tab_context_menu_tab_id = Some(tab_id);
        }
    }

    pub fn close_tab_overlays(&mut self) {
        self.tab_context_menu_tab_id = None;
        self.tab_group_editor_id = None;
    }

    pub fn open_tab_group_editor_at(&mut self, group_id: u64, anchor_x: f64) {
        self.tab_overlay_anchor_x = anchor_x.max(8.0);
        self.open_tab_group_editor(group_id);
    }

    pub fn open_tab_group_editor(&mut self, group_id: u64) {
        if self.tab_groups.iter().any(|group| group.id == group_id) {
            self.file_more_popup_open = false;
            self.sort_popup_open = false;
            self.transfer_popup_open = false;
            self.tab_context_menu_tab_id = None;
            self.tab_group_editor_id = Some(group_id);
        }
    }

    pub fn toggle_tab_group_collapsed(&mut self, group_id: u64) {
        if let Some(group) = self
            .tab_groups
            .iter_mut()
            .find(|group| group.id == group_id)
        {
            group.collapsed = !group.collapsed;
            // Keep the clicked group header at the same on-screen X while its
            // member tabs expand/collapse. Explicit tab selection releases it.
            self.tab_group_scroll_anchor_id = Some(group_id);
            self.tab_group_scroll_anchor_epoch = self.tab_group_scroll_anchor_epoch.wrapping_add(1);
            self.close_tab_overlays();
            self.persist_session();
        }
    }

    pub fn set_tab_group_name(&mut self, group_id: u64, value: String) {
        if let Some(group) = self
            .tab_groups
            .iter_mut()
            .find(|group| group.id == group_id)
        {
            group.name = value.chars().take(48).collect();
            self.persist_session();
        }
    }

    pub fn set_tab_group_color(&mut self, group_id: u64, color: TabGroupColor) {
        if let Some(group) = self
            .tab_groups
            .iter_mut()
            .find(|group| group.id == group_id)
        {
            group.color = color;
            self.persist_session();
        }
    }

    pub fn active_tab_index(&self) -> usize {
        self.active_tab
    }

    pub fn restore_warning(&self) -> Option<&str> {
        self.active_tab().restore_warning.as_deref()
    }

    pub fn dismiss_restore_warning(&mut self) {
        self.active_tab_mut().restore_warning = None;
    }

    pub fn active_tab(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }

    pub(crate) fn active_tab_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub const fn path_overflow_behavior(&self) -> PathOverflowBehavior {
        self.path_overflow_behavior
    }

    pub fn set_path_overflow_behavior(&mut self, behavior: PathOverflowBehavior) {
        self.path_overflow_behavior = behavior;
        self.persist_settings();
    }

    pub const fn path_reset_delay_ms(&self) -> u32 {
        self.path_reset_delay_ms
    }

    pub fn set_path_reset_delay_ms(&mut self, delay_ms: u32) {
        self.path_reset_delay_ms = delay_ms.clamp(500, 10_000);
        self.persist_settings();
    }

    pub fn file_scroll_reset_key(&self) -> u64 {
        self.active_tab().scroll_reset_key()
    }

    pub fn page(&self) -> AppPage {
        self.page
    }

    pub fn open_settings(&mut self) {
        self.open_settings_from(SettingsEntryPoint::General);
    }

    pub fn open_action_guide(&mut self) {
        self.open_settings_from(SettingsEntryPoint::ActionGuide);
    }

    #[cfg(any(target_os = "android", test))]
    pub fn open_device_sync_settings(&mut self) {
        self.open_settings_from(SettingsEntryPoint::DeviceSync);
    }

    fn open_settings_from(&mut self, entry_point: SettingsEntryPoint) {
        self.close_tab_overlays();
        self.file_more_popup_open = false;
        self.sort_popup_open = false;
        self.settings_entry_point = entry_point;
        // Opening Settings is an explicit retry point. A previous worker response may
        // have been lost while the view/task tree was being rebuilt; never let that
        // stale request keep Usage on "Calculating…" forever.
        self.force_refresh_remote_cache_usage();
        self.page = AppPage::Settings;
    }

    pub const fn settings_entry_point(&self) -> SettingsEntryPoint {
        self.settings_entry_point
    }

    pub fn close_settings(&mut self) {
        self.page = AppPage::Files;
    }

    #[cfg(target_os = "android")]
    pub const fn confirm_mobile_delete_enabled(&self) -> bool {
        self.confirm_mobile_delete
    }

    #[cfg(target_os = "android")]
    pub fn set_confirm_mobile_delete_enabled(&mut self, enabled: bool) {
        self.confirm_mobile_delete = enabled;
        if enabled {
            self.delete_warning_suppressed_until_ms = 0;
        } else {
            self.pending_delete_confirmation = None;
        }
        self.persist_settings();
    }

    pub fn delete_confirmation_name(&self) -> Option<&str> {
        self.pending_delete_confirmation
            .as_ref()
            .map(|pending| pending.name.as_str())
    }

    pub fn cancel_delete_confirmation(&mut self) {
        self.pending_delete_confirmation = None;
    }

    pub fn confirm_delete_once(&mut self) {
        let Some(pending) = self.pending_delete_confirmation.take() else {
            return;
        };
        self.delete_path(pending.path);
    }

    pub fn confirm_delete_for_today(&mut self) {
        let Some(pending) = self.pending_delete_confirmation.take() else {
            return;
        };
        #[cfg(target_os = "android")]
        {
            self.delete_warning_suppressed_until_ms = self
                .android_app
                .as_ref()
                .and_then(|app| crate::android_platform::local_day_end_unix_ms(app).ok())
                .unwrap_or_else(next_utc_day_boundary_ms);
            self.persist_settings();
        }
        self.delete_path(pending.path);
    }

    pub fn paste_conflict_name(&self) -> Option<&str> {
        self.pending_paste_conflict
            .as_ref()
            .map(|pending| pending.clipboard.name.as_str())
    }

    pub fn paste_conflict_destination(&self) -> Option<String> {
        self.pending_paste_conflict
            .as_ref()
            .map(|pending| self.display_path_for(&pending.target_location))
    }

    pub fn cancel_paste_conflict(&mut self) {
        self.pending_paste_conflict = None;
        self.paste_conflict_resolution = None;
        self.active_tab_mut().status = "Paste cancelled".to_owned();
    }

    pub fn replace_paste_conflict(&mut self) {
        let Some(pending) = self.pending_paste_conflict.take() else {
            return;
        };
        self.file_clipboard = Some(pending.clipboard);
        if self.active_tab().current_dir != pending.target_location {
            self.active_tab_mut().status = "Destination changed; paste cancelled".to_owned();
            return;
        }
        self.paste_conflict_resolution = Some(PasteConflictResolution::Replace);
        self.paste();
    }

    pub fn keep_both_paste_conflict(&mut self) {
        let Some(pending) = self.pending_paste_conflict.take() else {
            return;
        };
        self.file_clipboard = Some(pending.clipboard);
        if self.active_tab().current_dir != pending.target_location {
            self.active_tab_mut().status = "Destination changed; paste cancelled".to_owned();
            return;
        }
        self.paste_conflict_resolution = Some(PasteConflictResolution::KeepBoth);
        self.paste();
    }

    pub fn tailscale_profiles(&self) -> &[TailnetProfileState] {
        &self.tailscale_profiles
    }

    pub fn tailscale_profile_settings(&self) -> Vec<TailnetProfileSettings> {
        self.tailscale_profiles
            .iter()
            .map(|profile| profile.config.clone())
            .collect()
    }

    fn tailscale_profile(&self, profile_id: &str) -> Option<&TailnetProfileState> {
        self.tailscale_profiles
            .iter()
            .find(|profile| profile.config.id == profile_id)
    }

    fn tailscale_profile_mut(&mut self, profile_id: &str) -> Option<&mut TailnetProfileState> {
        self.tailscale_profiles
            .iter_mut()
            .find(|profile| profile.config.id == profile_id)
    }

    fn tailscale_profile_hostname_value(profile: &TailnetProfileState) -> String {
        let configured = normalize_tailscale_hostname(&profile.config.hostname);
        if configured.is_empty() {
            generated_tailscale_hostname(&profile.config.id)
        } else {
            configured
        }
    }

    pub fn tailscale_profile_hostname(&self, profile_id: &str) -> String {
        self.tailscale_profile(profile_id)
            .map(Self::tailscale_profile_hostname_value)
            .unwrap_or_default()
    }

    pub fn tailnet_path(&self, profile_id: &str) -> String {
        self.tailscale_profile(profile_id)
            .map(|profile| {
                let configured = normalize_tailnet_path(&profile.config.path);
                if configured.is_empty() {
                    automatic_tailnet_path(&profile.config.id)
                } else {
                    configured
                }
            })
            .unwrap_or_else(|| automatic_tailnet_path(profile_id))
    }

    pub fn tailnet_path_edit_value(&self, profile_id: &str) -> String {
        self.tailnet_path_drafts
            .get(profile_id)
            .cloned()
            .unwrap_or_else(|| self.tailnet_path(profile_id))
    }

    pub fn taildrive_device_name(&self, profile_id: &str, device_id: &str) -> String {
        let Some(profile) = self.tailscale_profile(profile_id) else {
            return taildrive_device_stable_name("d", device_id);
        };
        taildrive_device_path_names(profile)
            .get(device_id)
            .cloned()
            .unwrap_or_else(|| taildrive_device_stable_name("d", device_id))
    }

    pub fn taildrive_device_name_edit_value(&self, profile_id: &str, device_id: &str) -> String {
        self.taildrive_device_name_drafts
            .get(&(profile_id.to_owned(), device_id.to_owned()))
            .cloned()
            .unwrap_or_else(|| self.taildrive_device_name(profile_id, device_id))
    }

    fn taildrive_display_path_for(&self, location: &TaildriveLocation) -> String {
        let component = encode_taildrive_display_component;
        match location {
            TaildriveLocation::Root => "TD".to_owned(),
            TaildriveLocation::Profile { profile_id } => {
                format!("TD/{}", component(&self.tailnet_path(profile_id)))
            }
            TaildriveLocation::Device {
                profile_id,
                device_id,
            } => format!(
                "TD/{}/{}",
                component(&self.tailnet_path(profile_id)),
                component(&self.taildrive_device_name(profile_id, device_id))
            ),
            TaildriveLocation::Remote {
                profile_id,
                device_id,
                share,
                remote_path,
            } => {
                let mut display = format!(
                    "TD/{}/{}/{}",
                    component(&self.tailnet_path(profile_id)),
                    component(&self.taildrive_device_name(profile_id, device_id)),
                    component(share)
                );
                for remote_component in remote_path.split('/').filter(|part| !part.is_empty()) {
                    display.push('/');
                    display.push_str(&component(remote_component));
                }
                display
            }
        }
    }

    pub fn display_path_for(&self, path: &Path) -> String {
        if let Some(location) = crate::archive::parse_virtual_path(path) {
            return crate::archive::display_path(&location);
        }
        parse_taildrive_path(path)
            .map(|location| self.taildrive_display_path_for(&location))
            .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"))
    }

    pub fn path_label_for(&self, path: &Path) -> String {
        if let Some(location) = parse_taildrive_path(path) {
            return match location {
                TaildriveLocation::Root => "TD".to_owned(),
                TaildriveLocation::Profile { profile_id } => self.tailnet_path(&profile_id),
                TaildriveLocation::Device {
                    profile_id,
                    device_id,
                } => self.taildrive_device_name(&profile_id, &device_id),
                TaildriveLocation::Remote {
                    share, remote_path, ..
                } => remote_path
                    .rsplit('/')
                    .find(|part| !part.is_empty())
                    .map(str::to_owned)
                    .unwrap_or(share),
            };
        }
        if let Some(location) = crate::archive::parse_virtual_path(path) {
            return crate::archive::title(&location);
        }
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.display_path_for(path))
    }

    fn refresh_taildrive_address_inputs(&mut self) {
        let values = self
            .tabs
            .iter()
            .map(|tab| self.display_path_for(&tab.current_dir))
            .collect::<Vec<_>>();
        for (tab, value) in self.tabs.iter_mut().zip(values) {
            tab.address_input = value;
        }
    }

    fn refresh_active_address_input(&mut self) {
        let value = self.display_path_for(&self.active_tab().current_dir);
        self.active_tab_mut().address_input = value;
    }

    fn refresh_taildrive_profile_labels(&mut self, profile_id: &str) {
        let Some(profile) = self.tailscale_profile(profile_id) else {
            return;
        };
        let profile_label = profile.config.label.clone();
        let device_names = taildrive_device_path_names(profile);

        for tab in &mut self.tabs {
            let Some(current) = parse_taildrive_path(&tab.current_dir) else {
                continue;
            };
            let mut changed = false;
            match current {
                TaildriveLocation::Root => {
                    for entry in &mut tab.entries {
                        if matches!(
                            entry.remote.as_ref(),
                            Some(TaildriveLocation::Profile { profile_id: current }) if current == profile_id
                        ) {
                            entry.name.clone_from(&profile_label);
                            changed = true;
                        }
                    }
                }
                TaildriveLocation::Profile {
                    profile_id: current,
                } if current == profile_id => {
                    for entry in &mut tab.entries {
                        let Some(TaildriveLocation::Device {
                            profile_id: current,
                            device_id,
                        }) = entry.remote.as_ref()
                        else {
                            continue;
                        };
                        if current != profile_id {
                            continue;
                        }
                        if let Some(name) = device_names.get(device_id) {
                            entry.name.clone_from(name);
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
            if changed {
                tab.apply_sort();
            }
        }
    }

    fn reconcile_taildrive_profile_references(&mut self) -> bool {
        let valid_profile_ids = self
            .tailscale_profiles
            .iter()
            .map(|profile| profile.config.id.clone())
            .collect::<BTreeSet<_>>();
        let profile_is_missing = |path: &Path| {
            parse_taildrive_path(path)
                .as_ref()
                .and_then(taildrive_profile_id)
                .is_some_and(|profile_id| !valid_profile_ids.contains(profile_id))
        };
        let root = taildrive_path(&TaildriveLocation::Root);
        let active_index = self.active_tab;
        let mut active_reset = false;

        for (index, tab) in self.tabs.iter_mut().enumerate() {
            tab.back_stack.retain(|path| !profile_is_missing(path));
            tab.forward_stack.retain(|path| !profile_is_missing(path));
            if tab
                .selected_path
                .as_deref()
                .is_some_and(&profile_is_missing)
            {
                tab.selected_path = None;
            }
            if profile_is_missing(&tab.current_dir) {
                tab.set_current_dir(root.clone());
                tab.status = "Tailnet removed; opened TD root".to_owned();
                active_reset |= index == active_index;
            }
        }
        self.pinned_paths.retain(|path| !profile_is_missing(path));
        self.refresh_taildrive_address_inputs();
        active_reset
    }

    fn resolve_tailnet_profile_id(&self, value: &str) -> Option<String> {
        self.tailscale_profiles
            .iter()
            .find(|profile| {
                self.tailnet_path(&profile.config.id)
                    .eq_ignore_ascii_case(value)
            })
            .or_else(|| {
                self.tailscale_profiles.iter().find(|profile| {
                    profile.config.id.eq_ignore_ascii_case(value)
                        || profile.config.label.eq_ignore_ascii_case(value)
                })
            })
            .map(|profile| profile.config.id.clone())
    }

    fn remembered_taildrive_device_ids(&self, profile_id: &str) -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        let mut remember = |path: &Path| {
            let Some(location) = parse_taildrive_path(path) else {
                return;
            };
            match location {
                TaildriveLocation::Device {
                    profile_id: current,
                    device_id,
                }
                | TaildriveLocation::Remote {
                    profile_id: current,
                    device_id,
                    ..
                } if current == profile_id => {
                    ids.insert(device_id);
                }
                _ => {}
            }
        };
        for tab in &self.tabs {
            remember(&tab.current_dir);
            for path in &tab.back_stack {
                remember(path);
            }
            for path in &tab.forward_stack {
                remember(path);
            }
            if let Some(path) = tab.selected_path.as_deref() {
                remember(path);
            }
        }
        for path in &self.pinned_paths {
            remember(path);
        }
        ids
    }

    fn resolve_taildrive_device_id(&self, profile_id: &str, value: &str) -> Option<String> {
        let profile = self.tailscale_profile(profile_id)?;
        if let Some((device_id, _)) = taildrive_device_path_names(profile)
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(value))
        {
            return Some(device_id.clone());
        }
        profile
            .status
            .taildrive_devices
            .iter()
            .find(|device| {
                device.id.eq_ignore_ascii_case(value)
                    || device.hostname.eq_ignore_ascii_case(value)
                    || device.dns_name.eq_ignore_ascii_case(value)
                    || device.target.eq_ignore_ascii_case(value)
            })
            .map(|device| device.id.clone())
            .or_else(|| {
                profile
                    .config
                    .device_names
                    .keys()
                    .find(|device_id| device_id.eq_ignore_ascii_case(value))
                    .cloned()
            })
            .or_else(|| {
                self.remembered_taildrive_device_ids(profile_id)
                    .into_iter()
                    .find(|device_id| {
                        self.taildrive_device_name(profile_id, device_id)
                            .eq_ignore_ascii_case(value)
                    })
            })
    }

    fn parse_taildrive_address(&self, value: &str) -> Option<TaildriveLocation> {
        let normalized = value.trim().replace('\\', "/");
        let components = normalized
            .split('/')
            .filter(|component| !component.is_empty())
            .collect::<Vec<_>>();
        let root = *components.first()?;
        let legacy = root.eq_ignore_ascii_case("TailDrive");
        if !legacy && !root.eq_ignore_ascii_case("TD") {
            return None;
        }
        if components.len() == 1 {
            return Some(TaildriveLocation::Root);
        }

        let profile_component = decode_taildrive_display_component(components[1]);
        let profile_id = if legacy {
            self.tailscale_profiles
                .iter()
                .find(|profile| profile.config.id.eq_ignore_ascii_case(&profile_component))
                .map(|profile| profile.config.id.clone())
                .or_else(|| self.resolve_tailnet_profile_id(&profile_component))
                .unwrap_or_else(|| profile_component.clone())
        } else {
            self.resolve_tailnet_profile_id(&profile_component)?
        };
        if components.len() == 2 {
            return Some(TaildriveLocation::Profile { profile_id });
        }

        let device_component = decode_taildrive_display_component(components[2]);
        let device_id = if legacy {
            self.tailscale_profile(&profile_id)
                .and_then(|profile| {
                    profile
                        .status
                        .taildrive_devices
                        .iter()
                        .find(|device| device.id.eq_ignore_ascii_case(&device_component))
                        .map(|device| device.id.clone())
                        .or_else(|| {
                            profile
                                .config
                                .device_names
                                .keys()
                                .find(|device_id| device_id.eq_ignore_ascii_case(&device_component))
                                .cloned()
                        })
                })
                .or_else(|| self.resolve_taildrive_device_id(&profile_id, &device_component))
                .unwrap_or_else(|| device_component.clone())
        } else {
            self.resolve_taildrive_device_id(&profile_id, &device_component)?
        };
        if components.len() == 3 {
            return Some(TaildriveLocation::Device {
                profile_id,
                device_id,
            });
        }

        let share = decode_taildrive_display_component(components[3]);
        let remote_path = components[4..]
            .iter()
            .map(|component| decode_taildrive_display_component(component))
            .collect::<Vec<_>>()
            .join("/");
        Some(TaildriveLocation::Remote {
            profile_id,
            device_id,
            share,
            remote_path,
        })
    }

    pub fn set_tailnet_label(&mut self, profile_id: &str, value: String) {
        let value = value.chars().take(40).collect::<String>();
        if value.trim().is_empty() {
            return;
        }
        if let Some(profile) = self.tailscale_profile_mut(profile_id) {
            profile.config.label = value;
        }
        self.refresh_taildrive_profile_labels(profile_id);
        self.persist_settings();
    }

    pub fn edit_tailnet_path(&mut self, profile_id: &str, value: String) {
        if self.tailscale_profile(profile_id).is_none() {
            return;
        }
        let value = value
            .chars()
            .filter(|ch| !matches!(ch, '\r' | '\n'))
            .take(64)
            .collect::<String>();
        self.tailnet_path_drafts
            .insert(profile_id.to_owned(), value);
    }

    pub fn apply_tailnet_path_edit(&mut self, profile_id: &str, value: String) {
        if !self.tailnet_path_drafts.contains_key(profile_id)
            && value == self.tailnet_path_edit_value(profile_id)
        {
            return;
        }
        self.edit_tailnet_path(profile_id, value);
        self.apply_tailnet_path_draft(profile_id);
    }

    pub fn apply_tailnet_path_draft(&mut self, profile_id: &str) {
        let Some(value) = self.tailnet_path_drafts.get(profile_id).cloned() else {
            return;
        };
        if self.commit_tailnet_path(profile_id, value) {
            self.tailnet_path_drafts.remove(profile_id);
        }
    }

    pub fn reset_tailnet_path(&mut self, profile_id: &str) {
        self.tailnet_path_drafts.remove(profile_id);
        let _ = self.commit_tailnet_path(profile_id, String::new());
    }

    fn commit_tailnet_path(&mut self, profile_id: &str, value: String) -> bool {
        let mut stored_path = normalize_tailnet_path(&value);
        let mut effective_path = if stored_path.is_empty() {
            automatic_tailnet_path(profile_id)
        } else {
            stored_path.clone()
        };
        let conflicts = |candidate: &str| {
            self.tailscale_profiles.iter().any(|profile| {
                profile.config.id != profile_id
                    && self
                        .tailnet_path(&profile.config.id)
                        .eq_ignore_ascii_case(candidate)
            })
        };
        if conflicts(&effective_path) {
            if !stored_path.is_empty() {
                if let Some(profile) = self.tailscale_profile_mut(profile_id) {
                    profile.ping_status =
                        format!("Tailnet path '{effective_path}' is already in use");
                }
                return false;
            }
            let mut number = 1usize;
            loop {
                let candidate = format!("tn{number}");
                number += 1;
                if !conflicts(&candidate) {
                    stored_path = candidate.clone();
                    effective_path = candidate;
                    break;
                }
            }
        }
        let Some(profile) = self.tailscale_profile_mut(profile_id) else {
            return false;
        };
        profile.config.path = stored_path;
        profile.ping_status = format!("Path: TD/{effective_path}");
        self.refresh_taildrive_address_inputs();
        self.persist_settings();
        true
    }

    pub fn edit_taildrive_device_name(&mut self, profile_id: &str, device_id: &str, value: String) {
        if self.tailscale_profile(profile_id).is_none() {
            return;
        }
        let value = value
            .chars()
            .filter(|ch| !matches!(ch, '\r' | '\n'))
            .take(64)
            .collect::<String>();
        self.taildrive_device_name_drafts
            .insert((profile_id.to_owned(), device_id.to_owned()), value);
    }

    pub fn apply_taildrive_device_name_edit(
        &mut self,
        profile_id: &str,
        device_id: &str,
        value: String,
    ) {
        let key = (profile_id.to_owned(), device_id.to_owned());
        if !self.taildrive_device_name_drafts.contains_key(&key)
            && value == self.taildrive_device_name_edit_value(profile_id, device_id)
        {
            return;
        }
        self.edit_taildrive_device_name(profile_id, device_id, value);
        self.apply_taildrive_device_name_draft(profile_id, device_id);
    }

    pub fn apply_taildrive_device_name_draft(&mut self, profile_id: &str, device_id: &str) {
        let key = (profile_id.to_owned(), device_id.to_owned());
        let Some(value) = self.taildrive_device_name_drafts.get(&key).cloned() else {
            return;
        };
        if self.commit_taildrive_device_name(profile_id, device_id, value) {
            self.taildrive_device_name_drafts.remove(&key);
        }
    }

    pub fn reset_taildrive_device_name(&mut self, profile_id: &str, device_id: &str) {
        self.taildrive_device_name_drafts
            .remove(&(profile_id.to_owned(), device_id.to_owned()));
        let _ = self.commit_taildrive_device_name(profile_id, device_id, String::new());
    }

    fn commit_taildrive_device_name(
        &mut self,
        profile_id: &str,
        device_id: &str,
        value: String,
    ) -> bool {
        let name = normalize_taildrive_device_name(&value);
        if !value.trim().is_empty() && name.is_empty() {
            if let Some(profile) = self.tailscale_profile_mut(profile_id) {
                profile.ping_status = "Device path name is invalid".to_owned();
            }
            return false;
        }
        let duplicate = if name.is_empty() {
            false
        } else if let Some(profile) = self.tailscale_profile(profile_id) {
            let named_duplicate = profile.config.device_names.iter().any(|(id, existing)| {
                id != device_id
                    && normalize_taildrive_device_name(existing).eq_ignore_ascii_case(&name)
            });
            let discovered_duplicate = profile.status.taildrive_devices.iter().any(|device| {
                device.id != device_id
                    && self
                        .taildrive_device_name(profile_id, &device.id)
                        .eq_ignore_ascii_case(&name)
            });
            let remembered_duplicate = self
                .remembered_taildrive_device_ids(profile_id)
                .into_iter()
                .any(|remembered_device_id| {
                    remembered_device_id != device_id
                        && self
                            .taildrive_device_name(profile_id, &remembered_device_id)
                            .eq_ignore_ascii_case(&name)
                });
            named_duplicate || discovered_duplicate || remembered_duplicate
        } else {
            false
        };
        if duplicate {
            if let Some(profile) = self.tailscale_profile_mut(profile_id) {
                profile.ping_status = format!("Device path name '{name}' is already in use");
            }
            return false;
        }
        let Some(profile) = self.tailscale_profile_mut(profile_id) else {
            return false;
        };
        if name.is_empty() {
            profile.config.device_names.remove(device_id);
            profile.ping_status = "Device path name reset to automatic".to_owned();
        } else {
            profile
                .config
                .device_names
                .insert(device_id.to_owned(), name.clone());
            profile.ping_status = format!("Device path name: {name}");
        }
        self.refresh_taildrive_profile_labels(profile_id);
        self.refresh_taildrive_address_inputs();
        self.persist_settings();
        true
    }

    pub fn set_tailnet_hostname(&mut self, profile_id: &str, value: String) {
        let hostname = normalize_tailscale_hostname(&value);
        if let Some(profile) = self.tailscale_profile_mut(profile_id) {
            profile.config.hostname = hostname;
            self.persist_settings();
            let effective = self.tailscale_profile_hostname(profile_id);
            let _ = crate::tailscale::save_hostname(profile_id, &effective);
        }
    }

    pub fn apply_tailnet_hostname(&mut self, profile_id: &str, value: String) {
        self.set_tailnet_hostname(profile_id, value);
        let hostname = self.tailscale_profile_hostname(profile_id);
        let enabled = self
            .tailscale_profile(profile_id)
            .is_some_and(|profile| profile.config.enabled);
        let Some(sender) = self.tailscale_sender.clone() else {
            return;
        };
        if enabled {
            let _ = sender.send(crate::tailscale::Command::Restart {
                profile_id: profile_id.to_owned(),
                hostname,
            });
        }
    }

    pub fn tailscale_offline_peers_expanded(&self, profile_id: &str) -> bool {
        self.tailscale_offline_peers_expanded.contains(profile_id)
    }

    pub fn toggle_tailscale_offline_peers(&mut self, profile_id: &str) {
        if !self.tailscale_offline_peers_expanded.remove(profile_id) {
            self.tailscale_offline_peers_expanded
                .insert(profile_id.to_owned());
        }
    }

    pub fn tailscale_offline_devices_expanded(&self, profile_id: &str) -> bool {
        self.tailscale_offline_devices_expanded.contains(profile_id)
    }

    pub fn toggle_tailscale_offline_devices(&mut self, profile_id: &str) {
        if !self.tailscale_offline_devices_expanded.remove(profile_id) {
            self.tailscale_offline_devices_expanded
                .insert(profile_id.to_owned());
        }
    }

    pub fn install_tailscale_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<crate::tailscale::Command>,
    ) {
        self.tailscale_sender = Some(sender.clone());
        for profile in &self.tailscale_profiles {
            if profile.config.enabled {
                let _ = sender.send(crate::tailscale::Command::Start {
                    profile_id: profile.config.id.clone(),
                    hostname: Self::tailscale_profile_hostname_value(profile),
                });
            }
        }
        if let Some(location) = parse_taildrive_path(&self.active_tab().current_dir) {
            self.load_taildrive_location(location);
        }
    }

    fn dispatch_taildrive_transfer(
        &self,
        command: crate::tailscale::Command,
    ) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            crate::android_transfer::submit_taildrive(command);
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        {
            self.tailscale_sender
                .as_ref()
                .ok_or_else(|| "TailDrive worker is not ready".to_owned())?
                .send(command)
                .map_err(|_| "TailDrive worker stopped unexpectedly".to_owned())
        }
    }

    pub fn add_tailnet_profile(&mut self) {
        let mut number = 1usize;
        while self
            .tailscale_profiles
            .iter()
            .any(|profile| profile.config.label == format!("TN{number}"))
        {
            number += 1;
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let short_id = ((nonce >> 32) ^ nonce) as u32;
        let id = format!("tn-{short_id:08x}");
        let mut config = TailnetProfileSettings::new(id.clone(), format!("TN{number}"));
        let mut path_number = number;
        while self.tailscale_profiles.iter().any(|profile| {
            self.tailnet_path(&profile.config.id)
                .eq_ignore_ascii_case(&format!("tn{path_number}"))
        }) {
            path_number += 1;
        }
        config.path = format!("tn{path_number}");
        config.hostname = generated_tailscale_hostname(&id);
        config.enabled = true;
        self.tailscale_profiles
            .push(TailnetProfileState::from_config(config));
        self.persist_settings();
        if let Some(sender) = &self.tailscale_sender {
            let _ = sender.send(crate::tailscale::Command::Start {
                hostname: generated_tailscale_hostname(&id),
                profile_id: id,
            });
        }
    }

    pub fn remove_tailnet_profile(&mut self, profile_id: &str) {
        if let Some(sender) = &self.tailscale_sender {
            let _ = sender.send(crate::tailscale::Command::Stop {
                profile_id: profile_id.to_owned(),
            });
        }
        self.tailscale_profiles
            .retain(|profile| profile.config.id != profile_id);
        self.tailnet_path_drafts.remove(profile_id);
        self.taildrive_device_name_drafts
            .retain(|(draft_profile_id, _), _| draft_profile_id != profile_id);
        let active_reset = self.reconcile_taildrive_profile_references();
        if active_reset {
            self.load_taildrive_location(TaildriveLocation::Root);
        }
        self.persist_settings();
        self.persist_session();
        self.persist_mobile_browsing_state();
    }

    pub fn set_tailscale_profiles(
        &mut self,
        mut profiles: Vec<TailnetProfileSettings>,
        persist: bool,
    ) {
        let mut used_paths = BTreeSet::new();
        let mut next_default = 1usize;
        for profile in &mut profiles {
            let requested = normalize_tailnet_path(&profile.path);
            let mut effective = if requested.is_empty() {
                automatic_tailnet_path(&profile.id)
            } else {
                requested.clone()
            };
            if used_paths.contains(&effective.to_lowercase()) {
                if requested.is_empty() {
                    loop {
                        let candidate = format!("tn{next_default}");
                        next_default += 1;
                        if !used_paths.contains(&candidate.to_lowercase()) {
                            profile.path = candidate.clone();
                            effective = candidate;
                            break;
                        }
                    }
                } else {
                    let base = requested;
                    let mut suffix = 2usize;
                    loop {
                        let candidate = format!("{base}-{suffix}");
                        suffix += 1;
                        if !used_paths.contains(&candidate.to_lowercase()) {
                            profile.path = candidate.clone();
                            effective = candidate;
                            break;
                        }
                    }
                }
            } else {
                profile.path = requested;
            }
            used_paths.insert(effective.to_lowercase());
            profile.device_names.retain(|_, name| {
                *name = normalize_taildrive_device_name(name);
                !name.is_empty()
            });
        }
        let previous = self.tailscale_profile_settings();
        let sender = self.tailscale_sender.clone();
        for old in &previous {
            if !profiles.iter().any(|new| new.id == old.id && new.enabled)
                && old.enabled
                && let Some(sender) = &sender
            {
                let _ = sender.send(crate::tailscale::Command::Stop {
                    profile_id: old.id.clone(),
                });
            }
        }
        let valid_profile_ids = profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect::<BTreeSet<_>>();
        self.tailnet_path_drafts
            .retain(|profile_id, _| valid_profile_ids.contains(profile_id));
        self.taildrive_device_name_drafts
            .retain(|(profile_id, _), _| valid_profile_ids.contains(profile_id));
        self.tailscale_profiles = profiles
            .into_iter()
            .map(|config| {
                if let Some(mut old) = self
                    .tailscale_profiles
                    .iter()
                    .find(|old| old.config.id == config.id)
                    .cloned()
                {
                    old.config = config;
                    old
                } else {
                    TailnetProfileState::from_config(config)
                }
            })
            .collect();
        let active_reset = self.reconcile_taildrive_profile_references();
        if active_reset {
            self.load_taildrive_location(TaildriveLocation::Root);
            self.persist_session();
            self.persist_mobile_browsing_state();
        }
        if let Some(sender) = &sender {
            for profile in &self.tailscale_profiles {
                let was_enabled = previous
                    .iter()
                    .find(|old| old.id == profile.config.id)
                    .is_some_and(|old| old.enabled);
                if profile.config.enabled && !was_enabled {
                    let _ = sender.send(crate::tailscale::Command::Start {
                        profile_id: profile.config.id.clone(),
                        hostname: Self::tailscale_profile_hostname_value(profile),
                    });
                }
            }
        }
        if persist {
            self.persist_settings();
        }
    }

    pub fn set_tailscale_enabled(&mut self, enabled: bool, persist: bool) {
        if enabled && self.tailscale_profiles.is_empty() {
            self.add_tailnet_profile();
            return;
        }
        let mut profiles = self.tailscale_profile_settings();
        for profile in &mut profiles {
            profile.enabled = enabled;
        }
        self.set_tailscale_profiles(profiles, persist);
    }

    pub fn connect_tailscale(&mut self, profile_id: &str) {
        if let Some(profile) = self.tailscale_profile_mut(profile_id) {
            profile.config.enabled = true;
            profile.status.state = "Starting".to_owned();
            profile.status.error.clear();
            profile.ping_status.clear();
        }
        self.persist_settings();
        let hostname = self.tailscale_profile_hostname(profile_id);
        if let Some(sender) = &self.tailscale_sender {
            let _ = sender.send(crate::tailscale::Command::Start {
                profile_id: profile_id.to_owned(),
                hostname,
            });
        }
    }

    pub fn refresh_tailscale(&mut self, profile_id: &str) {
        if let Some(sender) = &self.tailscale_sender {
            let _ = sender.send(crate::tailscale::Command::Refresh {
                profile_id: profile_id.to_owned(),
            });
        }
    }

    pub fn open_tailscale_login(&mut self, profile_id: &str) {
        let Some(profile) = self
            .tailscale_profiles
            .iter()
            .find(|p| p.config.id == profile_id)
        else {
            return;
        };
        let url = profile.status.auth_url.clone();
        if url.is_empty() {
            if let Some(profile) = self.tailscale_profile_mut(profile_id) {
                profile.ping_status = "Tailscale sign-in URL is not ready yet".to_owned();
            }
            return;
        }
        #[cfg(target_os = "android")]
        let result = self.android_app.as_ref().map_or_else(
            || Err("Android activity unavailable".to_owned()),
            |app| crate::android_platform::open_url(app, &url),
        );
        #[cfg(not(target_os = "android"))]
        let result = open_url_with_system(&url);
        if let Err(error) = result
            && let Some(profile) = self.tailscale_profile_mut(profile_id)
        {
            profile.ping_status = format!("Cannot open Tailscale sign-in: {error}");
        }
    }

    pub fn is_archive_current(&self) -> bool {
        crate::archive::parse_virtual_path(&self.active_tab().current_dir).is_some()
    }

    pub fn is_taildrive_current(&self) -> bool {
        parse_taildrive_path(&self.active_tab().current_dir).is_some()
    }

    pub fn can_mutate_current_location(&self) -> bool {
        if self.is_archive_current() {
            return true;
        }
        match parse_taildrive_path(&self.active_tab().current_dir) {
            None => true,
            Some(TaildriveLocation::Remote { .. }) => !self
                .remote_mutations
                .contains(&self.active_tab().current_dir),
            Some(
                TaildriveLocation::Root
                | TaildriveLocation::Profile { .. }
                | TaildriveLocation::Device { .. },
            ) => false,
        }
    }

    fn begin_remote_mutation(&mut self, location: PathBuf) -> bool {
        if self.remote_mutations.contains(&location) {
            return false;
        }
        self.remote_mutations.push(location);
        true
    }

    fn finish_remote_mutation(&mut self, location: &Path) {
        self.remote_mutations.retain(|pending| pending != location);
        #[cfg(target_os = "android")]
        if let Some(app) = self.android_app.as_ref() {
            let _ = crate::android_platform::notify_documents_changed(app);
        }
    }

    fn load_taildrive_location(&mut self, location: TaildriveLocation) {
        self.taildrive_generation = self.taildrive_generation.wrapping_add(1);
        let generation = self.taildrive_generation;
        self.active_tab_mut().selected_path = None;
        self.active_tab_mut().rename_input = None;
        self.active_tab_mut().pending_remote_folder = None;
        self.active_tab_mut().pending_remote_delete = None;
        self.active_tab_mut().search_input.clear();
        self.active_tab_mut().search_active = false;
        self.active_tab_mut().entries.clear();

        match location {
            TaildriveLocation::Root => {
                let profiles = self.tailscale_profiles.clone();
                let mut entries = profiles
                    .into_iter()
                    .map(|profile| {
                        let location = TaildriveLocation::Profile {
                            profile_id: profile.config.id.clone(),
                        };
                        FileEntry {
                            path: taildrive_path(&location),
                            name: profile.config.label,
                            kind: EntryKind::Directory,
                            size: 0,
                            modified_sort_key: 0,
                            remote: Some(location),
                            remote_modified: None,
                        }
                    })
                    .collect::<Vec<_>>();
                entries.sort_by_key(|entry| entry.name.to_lowercase());
                let count = entries.len();
                self.active_tab_mut().entries = entries;
                self.active_tab_mut().apply_sort();
                self.active_tab_mut().status = if count == 0 {
                    "No Tailnets configured".to_owned()
                } else {
                    format!("{count} Tailnet(s)")
                };
            }
            TaildriveLocation::Profile { profile_id } => {
                let Some(profile) = self.tailscale_profile(&profile_id).cloned() else {
                    self.active_tab_mut().status = "Tailnet profile not found".to_owned();
                    return;
                };
                let path_names = taildrive_device_path_names(&profile);
                let mut entries = profile
                    .status
                    .taildrive_devices
                    .into_iter()
                    .map(|device| {
                        let location = TaildriveLocation::Device {
                            profile_id: profile_id.clone(),
                            device_id: device.id.clone(),
                        };
                        let name = path_names
                            .get(&device.id)
                            .cloned()
                            .unwrap_or_else(|| default_taildrive_device_name(&device));
                        FileEntry {
                            path: taildrive_path(&location),
                            name,
                            kind: EntryKind::Directory,
                            size: 0,
                            modified_sort_key: 0,
                            remote: Some(location),
                            remote_modified: None,
                        }
                    })
                    .collect::<Vec<_>>();
                entries.sort_by_key(|entry| entry.name.to_lowercase());
                let count = entries.len();
                self.active_tab_mut().entries = entries;
                self.active_tab_mut().apply_sort();
                self.active_tab_mut().status = if count == 0 {
                    if profile.status.state != "Running" || !profile.status.service_ready {
                        "Connecting to TailDrive…".to_owned()
                    } else if profile.status.taildrive_scanning {
                        "Scanning TailDrive devices…".to_owned()
                    } else if !profile.status.taildrive_error.is_empty() {
                        format!("TailDrive: {}", profile.status.taildrive_error)
                    } else {
                        "No TailDrive devices found".to_owned()
                    }
                } else {
                    format!("{count} TailDrive device(s)")
                };
            }
            TaildriveLocation::Device {
                profile_id,
                device_id,
            } => {
                let reconnecting = self.tailscale_profile(&profile_id).is_some_and(|profile| {
                    profile.status.state != "Running"
                        || !profile.status.service_ready
                        || profile.status.taildrive_scanning
                });
                let device = self
                    .tailscale_profile(&profile_id)
                    .and_then(|profile| {
                        profile
                            .status
                            .taildrive_devices
                            .iter()
                            .find(|device| device.id == device_id)
                    })
                    .cloned();
                let Some(device) = device else {
                    self.active_tab_mut().status = if reconnecting {
                        "Connecting to TailDrive…".to_owned()
                    } else {
                        "TailDrive device is unavailable".to_owned()
                    };
                    return;
                };
                let mut entries = device
                    .shares
                    .into_iter()
                    .map(|share| {
                        let location = TaildriveLocation::Remote {
                            profile_id: profile_id.clone(),
                            device_id: device_id.clone(),
                            share: share.clone(),
                            remote_path: String::new(),
                        };
                        FileEntry {
                            path: taildrive_path(&location),
                            name: share,
                            kind: EntryKind::Directory,
                            size: 0,
                            modified_sort_key: 0,
                            remote: Some(location),
                            remote_modified: None,
                        }
                    })
                    .collect::<Vec<_>>();
                entries.sort_by_key(|entry| entry.name.to_lowercase());
                let count = entries.len();
                self.active_tab_mut().entries = entries;
                self.active_tab_mut().apply_sort();
                self.active_tab_mut().status = if count == 0 {
                    "This device has no accessible TailDrive shares".to_owned()
                } else {
                    format!("{count} TailDrive share(s)")
                };
            }
            TaildriveLocation::Remote {
                profile_id,
                device_id,
                share,
                remote_path,
            } => {
                let location = TaildriveLocation::Remote {
                    profile_id: profile_id.clone(),
                    device_id: device_id.clone(),
                    share: share.clone(),
                    remote_path: remote_path.clone(),
                };
                let cached = load_taildrive_directory_cache_entries(
                    &location,
                    self.active_tab().show_hidden,
                );
                let cached_count = cached.len();
                self.active_tab_mut().entries = cached;
                self.active_tab_mut().apply_sort();
                self.active_tab_mut().status = if cached_count == 0 {
                    "Connecting to TailDrive…".to_owned()
                } else {
                    format!("Reconnecting to TailDrive… Showing {cached_count} cached item(s).")
                };
                let Some(sender) = &self.tailscale_sender else {
                    self.active_tab_mut().status = if cached_count == 0 {
                        "TailDrive worker is starting…".to_owned()
                    } else {
                        format!(
                            "TailDrive worker is starting… Showing {cached_count} cached item(s)."
                        )
                    };
                    return;
                };
                if sender
                    .send(crate::tailscale::Command::TaildriveList {
                        profile_id,
                        device_id,
                        share,
                        path: remote_path,
                        generation,
                    })
                    .is_err()
                {
                    self.active_tab_mut().status =
                        "TailDrive worker stopped unexpectedly".to_owned();
                }
            }
        }
    }

    fn refresh_taildrive_remote(&mut self, location: TaildriveLocation) {
        let TaildriveLocation::Remote {
            profile_id,
            device_id,
            share,
            remote_path,
        } = location
        else {
            return;
        };
        self.taildrive_generation = self.taildrive_generation.wrapping_add(1);
        let generation = self.taildrive_generation;
        let Some(sender) = &self.tailscale_sender else {
            self.active_tab_mut().status = "TailDrive worker is starting…".to_owned();
            return;
        };
        if sender
            .send(crate::tailscale::Command::TaildriveList {
                profile_id,
                device_id,
                share,
                path: remote_path,
                generation,
            })
            .is_err()
        {
            self.active_tab_mut().status = "TailDrive worker stopped unexpectedly".to_owned();
        }
    }

    pub fn ping_tailscale_peer(&mut self, profile_id: &str, target: String, label: String) {
        if let Some(profile) = self.tailscale_profile_mut(profile_id) {
            profile.ping_status = format!("Testing {label}…");
        }
        if let Some(sender) = &self.tailscale_sender {
            let _ = sender.send(crate::tailscale::Command::Ping {
                profile_id: profile_id.to_owned(),
                target,
                label,
            });
        }
    }

    pub fn disconnect_tailscale(&mut self, profile_id: &str) {
        if let Some(profile) = self.tailscale_profile_mut(profile_id) {
            profile.config.enabled = false;
            profile.ping_status = "Disconnecting; identity will be preserved…".to_owned();
        }
        self.persist_settings();
        if let Some(sender) = &self.tailscale_sender {
            let _ = sender.send(crate::tailscale::Command::Stop {
                profile_id: profile_id.to_owned(),
            });
        }
    }

    pub fn sign_out_tailscale(&mut self, profile_id: &str) {
        if let Some(profile) = self.tailscale_profile_mut(profile_id) {
            profile.config.enabled = false;
            profile.ping_status = "Signing out and forgetting this Tailscale node…".to_owned();
        }
        self.persist_settings();
        if let Some(sender) = &self.tailscale_sender {
            let _ = sender.send(crate::tailscale::Command::Logout {
                profile_id: profile_id.to_owned(),
            });
        }
    }

    pub fn apply_tailscale_event(&mut self, event: crate::tailscale::Event) {
        match event {
            crate::tailscale::Event::Status { profile_id, result } => {
                let (became_running, became_service_ready, scan_finished, devices_changed) = {
                    let Some(profile) = self.tailscale_profile_mut(&profile_id) else {
                        return;
                    };
                    let old_running = profile.status.state == "Running";
                    let old_service_ready = profile.status.service_ready;
                    let old_scanning = profile.status.taildrive_scanning;
                    let old_devices = profile
                        .status
                        .taildrive_devices
                        .iter()
                        .map(|device| (device.id.clone(), device.shares.clone()))
                        .collect::<Vec<_>>();
                    match result {
                        Ok(status) => profile.status = *status,
                        Err(error) => {
                            profile.status.state = "Error".to_owned();
                            profile.status.error = error;
                        }
                    }
                    let new_devices = profile
                        .status
                        .taildrive_devices
                        .iter()
                        .map(|device| (device.id.clone(), device.shares.clone()))
                        .collect::<Vec<_>>();
                    (
                        !old_running && profile.status.state == "Running",
                        !old_service_ready && profile.status.service_ready,
                        old_scanning && !profile.status.taildrive_scanning,
                        old_devices != new_devices,
                    )
                };
                self.refresh_taildrive_profile_labels(&profile_id);
                self.refresh_taildrive_address_inputs();
                if let Some(location) = parse_taildrive_path(&self.active_tab().current_dir) {
                    let matches_profile = match &location {
                        TaildriveLocation::Root => true,
                        TaildriveLocation::Profile {
                            profile_id: current,
                        }
                        | TaildriveLocation::Device {
                            profile_id: current,
                            ..
                        }
                        | TaildriveLocation::Remote {
                            profile_id: current,
                            ..
                        } => current == &profile_id,
                    };
                    if matches_profile {
                        let remote_retry_transition = became_running
                            || became_service_ready
                            || scan_finished
                            || devices_changed;
                        if !matches!(location, TaildriveLocation::Remote { .. })
                            || remote_retry_transition
                        {
                            self.load_taildrive_location(location);
                        }
                    }
                }
            }
            crate::tailscale::Event::Ping {
                profile_id,
                label,
                result,
            } => {
                let Some(profile) = self.tailscale_profile_mut(&profile_id) else {
                    return;
                };
                profile.ping_status = match result {
                    Ok(ping) if ping.ok => {
                        let remote = ping.remote.as_ref().map_or(label.as_str(), |remote| {
                            if remote.dns_name.is_empty() {
                                remote.hostname.as_str()
                            } else {
                                remote.dns_name.as_str()
                            }
                        });
                        format!("{label}: reached {remote} in {} ms", ping.latency_ms.max(0))
                    }
                    Ok(ping) => format!("{label}: {}", ping.error),
                    Err(error) => format!("{label}: {error}"),
                };
            }
            crate::tailscale::Event::TaildriveList {
                profile_id,
                device_id,
                share,
                path,
                generation,
                result,
            } => {
                if generation != self.taildrive_generation {
                    return;
                }
                let expected = TaildriveLocation::Remote {
                    profile_id: profile_id.clone(),
                    device_id: device_id.clone(),
                    share: share.clone(),
                    remote_path: path.clone(),
                };
                if parse_taildrive_path(&self.active_tab().current_dir).as_ref() != Some(&expected)
                {
                    return;
                }
                match result {
                    Ok(items) => {
                        let cached_entries = items
                            .iter()
                            .map(|item| CachedTaildriveEntry {
                                name: item.name.clone(),
                                remote_path: item.path.clone(),
                                directory: item.directory,
                                size: item.size.parse::<u64>().unwrap_or(0),
                                modified: item.modified.clone(),
                            })
                            .collect::<Vec<_>>();
                        save_taildrive_directory_cache_entries(&expected, cached_entries);

                        let show_hidden = self.active_tab().show_hidden;
                        let mut entries = items
                            .into_iter()
                            .filter(|item| show_hidden || !item.name.starts_with('.'))
                            .map(|item| {
                                let remote = TaildriveLocation::Remote {
                                    profile_id: profile_id.clone(),
                                    device_id: device_id.clone(),
                                    share: share.clone(),
                                    remote_path: item.path.clone(),
                                };
                                let modified_sort_key = remote_modified_sort_key(&item.modified);
                                FileEntry {
                                    path: taildrive_path(&remote),
                                    name: item.name,
                                    kind: if item.directory {
                                        EntryKind::Directory
                                    } else {
                                        EntryKind::File
                                    },
                                    size: item.size.parse::<u64>().unwrap_or(0),
                                    modified_sort_key,
                                    remote: Some(remote),
                                    remote_modified: (!item.modified.is_empty())
                                        .then_some(item.modified),
                                }
                            })
                            .collect::<Vec<_>>();
                        entries.sort_by(|a, b| {
                            let a_dir = a.kind == EntryKind::Directory;
                            let b_dir = b.kind == EntryKind::Directory;
                            b_dir
                                .cmp(&a_dir)
                                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                        });
                        let count = entries.len();
                        self.active_tab_mut().entries = entries;
                        self.active_tab_mut().apply_sort();
                        self.active_tab_mut().status = format!("{count} TailDrive item(s)");
                    }
                    Err(error) => {
                        let cached_count = self.active_tab().entries.len();
                        let reconnecting =
                            self.tailscale_profile(&profile_id).is_some_and(|profile| {
                                profile.status.state != "Running"
                                    || !profile.status.service_ready
                                    || profile.status.taildrive_scanning
                            }) || error.contains("not running")
                                || error.contains("not connected")
                                || error.contains("not currently available");
                        self.active_tab_mut().status = if reconnecting {
                            if cached_count == 0 {
                                "Connecting to TailDrive…".to_owned()
                            } else {
                                format!(
                                    "Reconnecting to TailDrive… Showing {cached_count} cached item(s)."
                                )
                            }
                        } else if cached_count == 0 {
                            format!("TailDrive: {error}")
                        } else {
                            format!(
                                "TailDrive unavailable: {error}. Showing {cached_count} cached item(s)."
                            )
                        };
                    }
                }
            }
            crate::tailscale::Event::TaildriveTransferProgress {
                transfer_id,
                progress,
            } => {
                if let Some(active) = self
                    .file_transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    if !progress.phase.is_empty() && active.phase != "Preparing app install" {
                        active.phase = progress.phase;
                    }
                    let now = Instant::now();
                    let elapsed = now.duration_since(active.last_sample_at).as_secs_f64();
                    if progress.bytes_done > active.last_sample_bytes && elapsed >= 0.20 {
                        let sample =
                            (progress.bytes_done - active.last_sample_bytes) as f64 / elapsed;
                        active.bytes_per_second = if active.bytes_per_second <= 0.0 {
                            sample
                        } else {
                            active.bytes_per_second * 0.72 + sample * 0.28
                        };
                        active.last_sample_at = now;
                        active.last_sample_bytes = progress.bytes_done;
                    }
                    active.bytes_done = progress.bytes_done;
                    active.bytes_total = progress.bytes_total;
                    active.items_done = progress.items_done;
                    active.items_total = progress.items_total;
                }
            }
            crate::tailscale::Event::TaildriveDownload {
                transfer_id,
                destination,
                display_name,
                source_location,
                open_after,
                source_was_cut,
                result,
            } => {
                self.finish_file_transfer(&transfer_id, result.as_ref().err().cloned());
                let pending_cache = self.pending_remote_cache_downloads.remove(&transfer_id);
                let pending_purpose = pending_cache
                    .as_ref()
                    .map(|pending| pending.purpose.clone())
                    .unwrap_or(RemotePreparePurpose::Open);
                let mut cache_files_in_progress = self.pending_remote_cache_files();
                if let Some(pending) = pending_cache.as_ref() {
                    cache_files_in_progress.insert(pending.file_name.clone());
                }
                let source_is_current = self.active_tab().current_dir == source_location;
                match result {
                    Ok(()) if open_after => {
                        if let Some(pending) = pending_cache.as_ref()
                            && let Some(root) = destination.parent()
                            && let Some(sender) = self.cache_sender.as_ref()
                        {
                            cache_files_in_progress.insert(pending.file_name.clone());
                            if sender
                                .send(CacheCommand::Record {
                                    root: root.to_path_buf(),
                                    source_key: pending.source_key.clone(),
                                    destination: destination.clone(),
                                    display_name: pending.display_name.clone(),
                                    remote_size: pending.remote_size,
                                    remote_modified: pending.remote_modified.clone(),
                                    settings: self.remote_cache_settings,
                                    protected: cache_files_in_progress.clone(),
                                })
                                .is_ok()
                            {
                                // Record updates usage when the worker completes, but it is not
                                // a Settings usage-scan request and must not drive the spinner.
                            }
                        }
                        match pending_purpose {
                            RemotePreparePurpose::ImportArchive {
                                current,
                                name,
                                replace,
                            } => {
                                let result = self
                                    .archive_sender
                                    .as_ref()
                                    .ok_or_else(|| "Archive worker is not ready".to_owned())
                                    .and_then(|sender| {
                                        sender
                                            .send(crate::archive::Command::Import {
                                                current,
                                                source: destination.clone(),
                                                name: name.clone(),
                                                replace,
                                            })
                                            .map_err(|_| "Archive worker stopped".to_owned())
                                    });
                                if source_is_current {
                                    self.active_tab_mut().status = match result {
                                        Ok(()) => format!("Adding {name} to archive…"),
                                        Err(error) => {
                                            format!("Cannot add TailDrive file to archive: {error}")
                                        }
                                    };
                                }
                            }
                            RemotePreparePurpose::Share => {
                                let action = self.share_path(&destination);
                                if source_is_current {
                                    self.active_tab_mut().status = match action {
                                        Ok(()) => format!("Sharing TailDrive file: {display_name}"),
                                        Err(error) => {
                                            format!("Cannot share TailDrive file: {error}")
                                        }
                                    };
                                }
                            }
                            RemotePreparePurpose::Open => {
                                let action = self.open_remote_cache_path(&destination);
                                if source_is_current {
                                    self.active_tab_mut().status = match action {
                                        Ok(()) if is_aab_name(&display_name) => {
                                            format!("Preparing installer: {display_name}")
                                        }
                                        Ok(()) if is_android_install_name(&display_name) => {
                                            format!("Installer opened: {display_name}")
                                        }
                                        Ok(()) => format!("Opened TailDrive file: {display_name}"),
                                        Err(error) => {
                                            format!("Cannot open TailDrive file: {error}")
                                        }
                                    };
                                }
                            }
                        }
                    }
                    Ok(()) => {
                        if source_was_cut {
                            self.file_clipboard = None;
                        }
                        if source_is_current {
                            self.reload_after_mutation(Some(destination));
                            self.active_tab_mut().status = if source_was_cut {
                                format!(
                                    "Copied from TailDrive: {display_name}. Remote source kept for safety."
                                )
                            } else {
                                format!("Copied from TailDrive: {display_name}")
                            };
                        }
                    }
                    Err(error) => {
                        if let Some(root) = destination.parent()
                            && let Some(sender) = self.cache_sender.as_ref()
                            && sender
                                .send(CacheCommand::RemoveTemp {
                                    root: root.to_path_buf(),
                                    path: destination.clone(),
                                    settings: self.remote_cache_settings,
                                    protected: cache_files_in_progress.clone(),
                                })
                                .is_ok()
                        {
                            // Temporary-file cleanup updates usage without owning the
                            // Settings usage-loading indicator.
                        }
                        if source_is_current {
                            self.active_tab_mut().status = if open_after {
                                match pending_purpose {
                                    RemotePreparePurpose::Share => {
                                        format!(
                                            "Cannot prepare TailDrive file {display_name} for sharing: {error}"
                                        )
                                    }
                                    RemotePreparePurpose::ImportArchive { .. } => {
                                        format!(
                                            "Cannot prepare TailDrive file {display_name} for archive: {error}"
                                        )
                                    }
                                    RemotePreparePurpose::Open => {
                                        format!(
                                            "Cannot open TailDrive file {display_name}: {error}"
                                        )
                                    }
                                }
                            } else {
                                format!("Cannot copy TailDrive item {display_name}: {error}")
                            };
                        }
                    }
                }
            }
            crate::tailscale::Event::TaildriveUpload {
                transfer_id,
                source,
                source_location,
                remote_path,
                source_was_cut,
                result,
            } => {
                self.finish_file_transfer(&transfer_id, result.as_ref().err().cloned());
                let upload_info = self.pending_upload_info.remove(&transfer_id);
                let temporary_upload = self.pending_temporary_uploads.remove(&source);
                let target_is_current = self.active_tab().current_dir == source_location;
                match result {
                    Ok(()) => {
                        let (source_kind, size) = upload_info.unwrap_or((EntryKind::File, 0));
                        let source_is_dir = source_kind == EntryKind::Directory;
                        let name = remote_path
                            .rsplit('/')
                            .next()
                            .filter(|name| !name.is_empty())
                            .unwrap_or("Uploaded file")
                            .to_owned();
                        if source_was_cut
                            && self
                                .file_clipboard
                                .as_ref()
                                .is_some_and(|clipboard| clipboard.path == source)
                        {
                            self.file_clipboard = None;
                        }
                        if target_is_current
                            && let Some(TaildriveLocation::Remote {
                                profile_id,
                                device_id,
                                share,
                                ..
                            }) = parse_taildrive_path(&source_location)
                        {
                            let remote = TaildriveLocation::Remote {
                                profile_id,
                                device_id,
                                share,
                                remote_path: remote_path.clone(),
                            };
                            let virtual_path = taildrive_path(&remote);
                            if !self
                                .active_tab()
                                .entries
                                .iter()
                                .any(|entry| entry.path == virtual_path)
                            {
                                self.active_tab_mut().entries.push(FileEntry {
                                    path: virtual_path.clone(),
                                    name: name.clone(),
                                    kind: if source_is_dir {
                                        EntryKind::Directory
                                    } else {
                                        EntryKind::File
                                    },
                                    size,
                                    modified_sort_key: 0,
                                    remote: Some(remote),
                                    remote_modified: None,
                                });
                            }
                            self.active_tab_mut().select_entry(virtual_path);
                            self.active_tab_mut().status = if source_was_cut {
                                format!(
                                    "Uploaded: {name}. Local source kept because cross-filesystem Cut is not atomic."
                                )
                            } else {
                                format!("Uploaded: {name}")
                            };
                            if let Some(location) = parse_taildrive_path(&source_location) {
                                self.refresh_taildrive_remote(location);
                            }
                        }
                    }
                    Err(error) if target_is_current => {
                        self.active_tab_mut().status = format!("TailDrive upload failed: {error}");
                    }
                    Err(_) => {}
                }
                if temporary_upload {
                    // Archive-to-TailDrive exports are temporary, unindexed cache files.
                    // Cache maintenance removes them asynchronously after the upload finishes.
                    self.refresh_remote_cache_usage();
                }
            }
            crate::tailscale::Event::TaildriveRelay {
                transfer_id,
                target_location,
                display_name,
                source_was_cut,
                result,
            } => {
                self.finish_file_transfer(&transfer_id, result.as_ref().err().cloned());
                let target_is_current = self.active_tab().current_dir == target_location;
                match result {
                    Ok(()) => {
                        if source_was_cut {
                            self.file_clipboard = None;
                        }
                        if target_is_current {
                            self.active_tab_mut().status = if source_was_cut {
                                format!(
                                    "Copied in TailDrive: {display_name}. Remote source kept for safety."
                                )
                            } else {
                                format!("Copied in TailDrive: {display_name}")
                            };
                            if let Some(location) = parse_taildrive_path(&target_location) {
                                self.refresh_taildrive_remote(location);
                            }
                        }
                    }
                    Err(error) if target_is_current => {
                        self.active_tab_mut().status = format!("TailDrive copy failed: {error}");
                    }
                    Err(_) => {}
                }
            }
            crate::tailscale::Event::TaildriveMkdir {
                source_location,
                remote_path,
                result,
            } => {
                let target_is_current = self.active_tab().current_dir == source_location;
                self.finish_remote_mutation(&source_location);
                if target_is_current {
                    let virtual_path =
                        parse_taildrive_path(&source_location).and_then(|location| {
                            let TaildriveLocation::Remote {
                                profile_id,
                                device_id,
                                share,
                                ..
                            } = location
                            else {
                                return None;
                            };
                            Some(taildrive_path(&TaildriveLocation::Remote {
                                profile_id,
                                device_id,
                                share,
                                remote_path: remote_path.clone(),
                            }))
                        });
                    match result {
                        Ok(()) => {
                            self.active_tab_mut().pending_remote_folder = None;
                            self.active_tab_mut().status = "TailDrive folder created".to_owned();
                            if let Some(path) = virtual_path {
                                self.active_tab_mut().select_entry(path);
                            }
                            if let Some(location) = parse_taildrive_path(&source_location) {
                                self.refresh_taildrive_remote(location);
                            }
                        }
                        Err(error) => {
                            if let Some(path) = virtual_path {
                                self.active_tab_mut()
                                    .entries
                                    .retain(|entry| entry.path != path);
                                if self.active_tab().selected_path.as_ref() == Some(&path) {
                                    self.active_tab_mut().selected_path = None;
                                }
                            }
                            self.active_tab_mut().pending_remote_folder = None;
                            self.active_tab_mut().rename_input = None;
                            self.active_tab_mut().rename_replace_on_type = false;
                            self.active_tab_mut().rename_keyboard_suffix = None;
                            self.active_tab_mut().status =
                                format!("Cannot create TailDrive folder: {error}");
                        }
                    }
                }
            }
            crate::tailscale::Event::TaildriveDelete {
                source_location,
                remote_path,
                result,
            } => {
                let target_is_current = self.active_tab().current_dir == source_location;
                self.finish_remote_mutation(&source_location);
                match result {
                    Ok(()) if target_is_current => {
                        self.active_tab_mut().entries.retain(|entry| {
                            !matches!(
                                &entry.remote,
                                Some(TaildriveLocation::Remote { remote_path: path, .. }) if path == &remote_path
                            )
                        });
                        self.active_tab_mut().selected_path = None;
                        self.active_tab_mut().status = "Deleted from TailDrive".to_owned();
                        if let Some(location) = parse_taildrive_path(&source_location) {
                            self.refresh_taildrive_remote(location);
                        }
                    }
                    Ok(()) => {}
                    Err(error) if target_is_current => {
                        self.active_tab_mut().status = format!("TailDrive delete failed: {error}");
                    }
                    Err(_) => {}
                }
            }
            crate::tailscale::Event::TaildriveRename {
                source_location,
                remote_path,
                new_name,
                result,
            } => {
                let target_is_current = self.active_tab().current_dir == source_location;
                self.finish_remote_mutation(&source_location);
                match result {
                    Ok(()) if target_is_current => {
                        let parent = remote_path
                            .rsplit_once('/')
                            .map_or("", |(parent, _)| parent);
                        let new_remote_path = if parent.is_empty() {
                            new_name.clone()
                        } else {
                            format!("{parent}/{new_name}")
                        };
                        let mut selected = None;
                        for entry in &mut self.active_tab_mut().entries {
                            if let Some(TaildriveLocation::Remote {
                                profile_id,
                                device_id,
                                share,
                                remote_path: path,
                            }) = &mut entry.remote
                                && path == &remote_path
                            {
                                *path = new_remote_path.clone();
                                entry.name = new_name.clone();
                                entry.path = taildrive_path(&TaildriveLocation::Remote {
                                    profile_id: profile_id.clone(),
                                    device_id: device_id.clone(),
                                    share: share.clone(),
                                    remote_path: new_remote_path.clone(),
                                });
                                selected = Some(entry.path.clone());
                                break;
                            }
                        }
                        self.active_tab_mut().rename_input = None;
                        self.active_tab_mut().rename_replace_on_type = false;
                        self.active_tab_mut().rename_keyboard_suffix = None;
                        if let Some(path) = selected {
                            self.active_tab_mut().select_entry(path);
                        }
                        self.active_tab_mut().status = format!("Renamed: {new_name}");
                        if let Some(location) = parse_taildrive_path(&source_location) {
                            self.refresh_taildrive_remote(location);
                        }
                    }
                    Ok(()) => {}
                    Err(error) if target_is_current => {
                        self.active_tab_mut().status = format!("TailDrive rename failed: {error}");
                    }
                    Err(_) => {}
                }
            }
            crate::tailscale::Event::Stopped { profile_id, result } => {
                let Some(profile) = self.tailscale_profile_mut(&profile_id) else {
                    return;
                };
                match result {
                    Ok(()) => {
                        profile.status = crate::tailscale::TailscaleStatus::default();
                        profile.ping_status = "Disconnected; identity preserved".to_owned();
                    }
                    Err(error) => {
                        profile.ping_status = format!("Tailscale disconnect failed: {error}")
                    }
                }
            }
            crate::tailscale::Event::LoggedOut { profile_id, result } => {
                let Some(profile) = self.tailscale_profile_mut(&profile_id) else {
                    return;
                };
                match result {
                    Ok(()) => {
                        profile.status = crate::tailscale::TailscaleStatus::default();
                        profile.ping_status = "Signed out of Tailscale".to_owned();
                    }
                    Err(error) => {
                        profile.ping_status = format!("Tailscale sign-out failed: {error}")
                    }
                }
            }
        }
    }

    pub fn shutdown(&mut self) {
        self.persist_session();
    }

    /// Handles a platform-level Back action. Returns true when the host Activity/window
    /// should be closed because there is no in-app navigation left to consume it.
    pub fn handle_system_back(&mut self) -> bool {
        if self.pending_delete_confirmation.is_some() {
            self.cancel_delete_confirmation();
            return false;
        }
        if self.pending_paste_conflict.is_some() {
            self.cancel_paste_conflict();
            return false;
        }
        if self.rename_active() {
            self.cancel_rename();
            return false;
        }
        if self.tab_context_menu_tab_id.is_some() || self.tab_group_editor_id.is_some() {
            self.close_tab_overlays();
            return false;
        }
        if self.transfer_popup_open {
            self.close_transfer_popup();
            return false;
        }
        if self.sort_popup_open {
            self.close_sort_popup();
            return false;
        }
        if self.file_more_popup_open {
            self.close_file_more_popup();
            return false;
        }
        if self.page == AppPage::Settings {
            self.close_settings();
            return false;
        }
        if self.can_go_back() {
            self.go_back();
            return false;
        }
        self.shutdown();
        true
    }

    #[cfg(target_os = "android")]
    pub fn request_android_storage_access(&mut self) {
        let Some(app) = self.android_app.as_ref() else {
            self.active_tab_mut().status = "Android activity unavailable".to_owned();
            return;
        };
        match crate::android_platform::request_storage_access(app) {
            Ok(()) => {
                self.active_tab_mut().status =
                    "Grant file access in Android settings. FastExplorer will update when you return."
                        .to_owned();
            }
            Err(error) => {
                self.active_tab_mut().status = format!("Cannot request file access: {error}")
            }
        }
    }

    #[cfg(target_os = "android")]
    pub fn android_storage_access_granted(&self) -> bool {
        self.android_storage_access
    }

    #[cfg(target_os = "android")]
    pub fn android_insets(&self) -> crate::android_platform::SystemBarInsets {
        self.android_insets
    }

    #[cfg(target_os = "android")]
    pub fn poll_android_back(&mut self) {
        if crate::android_platform::take_back_request() && self.handle_system_back() {
            self.background_android_task();
        }
    }

    #[cfg(target_os = "android")]
    pub fn poll_android_transfers(&mut self) {
        for snapshot in crate::android_transfer::snapshots() {
            if let Some(transfer) = self
                .file_transfers
                .iter_mut()
                .find(|transfer| transfer.transfer_id == snapshot.transfer_id)
            {
                transfer.label = snapshot.label;
                transfer.phase = snapshot.phase;
                transfer.bytes_done = snapshot.bytes_done;
                transfer.bytes_total = snapshot.bytes_total;
                transfer.items_done = snapshot.items_done;
                transfer.items_total = snapshot.items_total;
                transfer.paused = snapshot.paused;
                transfer.cancelling = snapshot.cancelling;
                transfer.cancelled = snapshot.cancelled;
                transfer.done = snapshot.done;
                transfer.error = snapshot.error;
                transfer.bytes_per_second = snapshot.bytes_per_second;
                continue;
            }
            if self.file_transfers.len() >= TRANSFER_HISTORY_LIMIT
                && let Some(index) = self
                    .file_transfers
                    .iter()
                    .position(|transfer| transfer.done)
            {
                self.file_transfers.remove(index);
            }
            let now = Instant::now();
            self.file_transfers.push(FileTransferProgress {
                transfer_id: snapshot.transfer_id,
                label: snapshot.label,
                phase: snapshot.phase,
                bytes_done: snapshot.bytes_done,
                bytes_total: snapshot.bytes_total,
                items_done: snapshot.items_done,
                items_total: snapshot.items_total,
                paused: snapshot.paused,
                cancelling: snapshot.cancelling,
                cancelled: snapshot.cancelled,
                done: snapshot.done,
                error: snapshot.error,
                started_at: now,
                last_sample_at: now,
                last_sample_bytes: snapshot.bytes_done,
                bytes_per_second: snapshot.bytes_per_second,
            });
        }

        for event in crate::android_transfer::drain_ui_events() {
            match event {
                crate::android_transfer::UiEvent::Tailscale(event) => {
                    self.apply_tailscale_event(event);
                }
                crate::android_transfer::UiEvent::Local { transfer_id, event } => {
                    let error = event.result.as_ref().err().cloned();
                    self.finish_file_transfer(&transfer_id, error);
                    self.apply_local_file_event(event);
                }
            }
        }
    }

    #[cfg(target_os = "android")]
    pub fn poll_android_platform_state(&mut self) {
        if crate::android_platform::take_sync_notification_opened() {
            self.open_device_sync_settings();
            self.supabase.status = "Incoming device transfer opened from notification".to_owned();
        }
        let Some(app) = self.android_app.clone() else {
            return;
        };
        self.android_insets = crate::android_platform::system_bar_insets(&app);
        self.android_window_width_dp = crate::android_platform::window_width_dp(&app);
        if let Ok(snapshot) = crate::android_platform::network_interfaces_json(&app) {
            let _ = crate::tailscale::set_android_interfaces_json(&snapshot);
        }
        let granted = crate::android_platform::has_storage_access(&app);
        if granted && !self.android_storage_access {
            self.android_storage_access = true;
            if !android_config_migration_complete() {
                // Scoped storage may have hidden an old shared-storage config during
                // the first launch after upgrading. Re-read settings now that access
                // exists, then persist them privately; cleanup only removes the legacy
                // file when load_settings actually imported it.
                self.reload_settings();
                self.persist_settings();
            }
            if self.is_taildrive_current() {
                self.refresh();
            } else if let Ok(root) = crate::android_platform::shared_storage_root(&app) {
                self.active_tab_mut().set_current_dir(root);
                self.persist_mobile_browsing_state();
            } else {
                self.refresh();
            }
        } else {
            self.android_storage_access = granted;
        }

        // A worker can be recreated while Android is still bringing the activity and
        // storage services up. If the very first directory response is lost during
        // that transition, do not leave the file list stuck on "Loading folder…".
        // Normal directory scans finish far below this timeout, so this is only a
        // recovery path rather than periodic polling.
        let stalled_local_load = self
            .directory_request_started_at
            .is_some_and(|started| started.elapsed() >= Duration::from_secs(6))
            && self.active_tab().entries.is_empty()
            && self.active_tab().status.starts_with("Loading")
            && !self.active_tab().search_active
            && parse_taildrive_path(&self.active_tab().current_dir).is_none()
            && crate::archive::parse_virtual_path(&self.active_tab().current_dir).is_none();
        if stalled_local_load {
            self.request_directory_reload();
        }
    }

    #[cfg(target_os = "android")]
    fn ensure_android_transfer_service(&self) {
        if let Some(app) = self.android_app.as_ref()
            && let Err(error) = crate::android_platform::start_transfer_service(app)
        {
            eprintln!("FastExplorer: cannot start transfer service: {error}");
        }
    }

    #[cfg(target_os = "android")]
    pub fn background_android_task(&self) {
        if let Some(app) = self.android_app.as_ref() {
            crate::android_platform::move_task_to_back(app);
        }
    }

    fn remote_cache_root(&self) -> Option<PathBuf> {
        #[cfg(target_os = "android")]
        {
            self.android_app
                .as_ref()
                .and_then(|app| crate::android_platform::remote_open_cache_dir(app).ok())
        }
        #[cfg(not(target_os = "android"))]
        {
            Some(
                std::env::temp_dir()
                    .join("FastExplorer")
                    .join("remote-open"),
            )
        }
    }

    fn open_remote_cache_path(&self, path: &Path) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            self.android_app.as_ref().map_or_else(
                || Err("Android activity unavailable".to_owned()),
                |app| crate::android_platform::open_file(app, path),
            )
        }
        #[cfg(not(target_os = "android"))]
        {
            open_path_with_system(path)
        }
    }

    fn pending_remote_cache_files(&self) -> BTreeSet<String> {
        let files = self
            .pending_remote_cache_downloads
            .values()
            .map(|pending| pending.file_name.clone())
            .collect::<BTreeSet<_>>();
        #[cfg(target_os = "android")]
        let mut files = files;
        #[cfg(target_os = "android")]
        {
            files.extend(crate::android_transfer::protected_cache_files());
            if let Some(app) = self.android_app.as_ref()
                && let Ok(json) = crate::android_platform::remote_open_leases_json(app)
                && let Ok(leased) = serde_json::from_str::<Vec<String>>(&json)
            {
                files.extend(leased);
            }
        }
        files
    }

    fn next_remote_cache_usage_request_id(&mut self) -> u64 {
        self.remote_cache_usage_next_request_id = self
            .remote_cache_usage_next_request_id
            .wrapping_add(1)
            .max(1);
        self.remote_cache_usage_next_request_id
    }

    fn refresh_remote_cache_usage(&mut self) {
        self.request_remote_cache_usage(false);
    }

    fn force_refresh_remote_cache_usage(&mut self) {
        self.request_remote_cache_usage(true);
    }

    fn request_remote_cache_usage(&mut self, force: bool) {
        // Normal maintenance requests are coalesced, but an explicit Settings open is
        // allowed to supersede a stale request. Request ids make late responses harmless:
        // only the newest request is allowed to clear the loading state.
        if self.remote_cache_usage_pending.is_some() && !force {
            self.remote_cache_usage_refresh_queued = true;
            return;
        }
        let Some(root) = self.remote_cache_root() else {
            self.remote_cache_usage_bytes = 0;
            self.remote_cache_usage_pending = None;
            self.remote_cache_usage_refresh_queued = false;
            return;
        };
        let usage_refresh_id = self.next_remote_cache_usage_request_id();
        let command = CacheCommand::Maintain {
            root,
            settings: self.remote_cache_settings,
            protected: self.pending_remote_cache_files(),
            usage_refresh_id,
        };
        if force {
            self.remote_cache_usage_refresh_queued = false;
        }
        #[cfg(test)]
        if self.cache_sender.is_none() {
            self.remote_cache_usage_pending = Some(usage_refresh_id);
            self.apply_cache_event(CacheEvent {
                result: perform_cache_command(&command),
                usage_refresh_id: Some(usage_refresh_id),
            });
            return;
        }
        let Some(sender) = self.cache_sender.clone() else {
            if force {
                self.remote_cache_usage_pending = None;
            }
            return;
        };
        if sender.send(command).is_ok() {
            self.remote_cache_usage_pending = Some(usage_refresh_id);
        } else {
            // A dead worker must never leave the settings page permanently loading.
            self.cache_sender = None;
            self.remote_cache_usage_pending = None;
            self.remote_cache_usage_refresh_queued = false;
        }
    }

    pub fn remote_cache_limit_label(&self) -> String {
        format_size(u64::from(self.remote_cache_settings.limit_mib) * 1024 * 1024)
    }

    pub fn remote_cache_expiration_label(&self) -> String {
        let hours = self.remote_cache_settings.expiration_hours;
        if hours >= 24 && hours.is_multiple_of(24) {
            let days = hours / 24;
            if days == 1 {
                "1 day".to_owned()
            } else {
                format!("{days} days")
            }
        } else if hours == 1 {
            "1 hour".to_owned()
        } else {
            format!("{hours} hours")
        }
    }

    pub fn remote_cache_usage_label(&self) -> String {
        if self.remote_cache_usage_pending.is_some() {
            format!("Calculating… / {}", self.remote_cache_limit_label())
        } else {
            format!(
                "{} / {}",
                format_size(self.remote_cache_usage_bytes),
                self.remote_cache_limit_label()
            )
        }
    }

    pub fn remote_cache_limit_slider_value(&self) -> f64 {
        REMOTE_CACHE_LIMIT_STEPS_MIB
            .iter()
            .position(|value| *value >= self.remote_cache_settings.limit_mib)
            .unwrap_or(REMOTE_CACHE_LIMIT_STEPS_MIB.len() - 1) as f64
    }

    pub fn set_remote_cache_limit_slider_value(&mut self, value: f64) {
        let index = value
            .round()
            .clamp(0.0, (REMOTE_CACHE_LIMIT_STEPS_MIB.len() - 1) as f64)
            as usize;
        let mut settings = self.remote_cache_settings;
        settings.limit_mib = REMOTE_CACHE_LIMIT_STEPS_MIB[index];
        self.set_remote_cache_settings(settings, true);
    }

    pub fn remote_cache_expiration_slider_value(&self) -> f64 {
        REMOTE_CACHE_EXPIRATION_STEPS_HOURS
            .iter()
            .position(|value| *value >= self.remote_cache_settings.expiration_hours)
            .unwrap_or(REMOTE_CACHE_EXPIRATION_STEPS_HOURS.len() - 1) as f64
    }

    pub fn set_remote_cache_expiration_slider_value(&mut self, value: f64) {
        let index = value
            .round()
            .clamp(0.0, (REMOTE_CACHE_EXPIRATION_STEPS_HOURS.len() - 1) as f64)
            as usize;
        let mut settings = self.remote_cache_settings;
        settings.expiration_hours = REMOTE_CACHE_EXPIRATION_STEPS_HOURS[index];
        self.set_remote_cache_settings(settings, true);
    }

    pub fn clear_remote_cache(&mut self) {
        let Some(root) = self.remote_cache_root() else {
            self.remote_cache_usage_bytes = 0;
            self.remote_cache_usage_pending = None;
            self.remote_cache_usage_refresh_queued = false;
            return;
        };
        let usage_refresh_id = self.next_remote_cache_usage_request_id();
        let command = CacheCommand::Clear {
            root,
            settings: self.remote_cache_settings,
            protected: self.pending_remote_cache_files(),
            usage_refresh_id,
        };
        self.remote_cache_usage_refresh_queued = false;
        #[cfg(test)]
        if self.cache_sender.is_none() {
            self.remote_cache_usage_pending = Some(usage_refresh_id);
            self.apply_cache_event(CacheEvent {
                result: perform_cache_command(&command),
                usage_refresh_id: Some(usage_refresh_id),
            });
            return;
        }
        let Some(sender) = self.cache_sender.clone() else {
            self.remote_cache_usage_pending = None;
            return;
        };
        if sender.send(command).is_ok() {
            // Clear supersedes any older usage refresh; late older responses cannot
            // dismiss this loading state because their request id will not match.
            self.remote_cache_usage_pending = Some(usage_refresh_id);
        } else {
            self.cache_sender = None;
            self.remote_cache_usage_pending = None;
        }
    }

    pub fn palette(&self) -> ThemePalette {
        ThemePalette::generate(self.theme_settings, self.system_dark)
    }

    pub fn is_dark_theme(&self) -> bool {
        match self.theme_settings.appearance {
            AppearanceMode::System => self.system_dark,
            AppearanceMode::Light => false,
            AppearanceMode::Dark => true,
        }
    }

    pub fn theme_color(&self) -> ThemeColor {
        self.theme_settings.color
    }

    pub fn theme_intensity(&self) -> u8 {
        self.theme_settings.intensity
    }

    pub fn effective_theme_settings(&self) -> ThemeSettings {
        self.theme_settings
    }

    pub fn saved_theme_settings(&self) -> ThemeSettings {
        self.saved_theme_settings
    }

    pub const fn search_mode(&self) -> SearchMode {
        self.search_mode
    }

    pub fn everything_search_available(&self) -> bool {
        crate::search::everything_available()
    }

    pub const fn saved_search_mode(&self) -> SearchMode {
        self.saved_search_mode
    }

    pub const fn ui_font(&self) -> UiFont {
        self.ui_font
    }

    pub const fn saved_ui_font(&self) -> UiFont {
        self.saved_ui_font
    }

    pub const fn remote_cache_settings(&self) -> RemoteCacheSettings {
        self.remote_cache_settings
    }

    pub const fn saved_remote_cache_settings(&self) -> RemoteCacheSettings {
        self.saved_remote_cache_settings
    }

    pub fn set_remote_cache_settings(&mut self, settings: RemoteCacheSettings, persist: bool) {
        self.remote_cache_settings = RemoteCacheSettings {
            limit_mib: settings.limit_mib.clamp(128, 8192),
            expiration_hours: settings.expiration_hours.clamp(1, 720),
        };
        if persist {
            self.saved_remote_cache_settings = self.remote_cache_settings;
            self.persist_settings();
        }
        self.refresh_remote_cache_usage();
    }

    pub fn set_ui_font(&mut self, font: UiFont, persist: bool) {
        self.ui_font = font;
        if persist {
            self.saved_ui_font = font;
            self.persist_settings();
        }
    }

    pub fn search_mode_label(&self) -> &'static str {
        self.search_mode.label()
    }

    pub fn apply_theme_patch(&mut self, patch: ThemePatch, persist: bool) {
        if patch.is_empty() {
            return;
        }
        if persist {
            self.saved_theme_settings = patch.apply(self.saved_theme_settings);
            if patch.appearance.is_some() {
                self.theme_overrides.appearance = None;
            }
            if patch.color.is_some() {
                self.theme_overrides.color = None;
            }
            if patch.intensity.is_some() {
                self.theme_overrides.intensity = None;
            }
            self.persist_settings();
        }
        self.theme_settings = patch.apply(self.theme_settings);
        if self.theme_settings.appearance == AppearanceMode::System {
            self.system_dark = detect_system_dark();
        }
    }

    pub fn set_search_mode(&mut self, mode: SearchMode, persist: bool) {
        if mode == SearchMode::Everything && !crate::search::everything_available() {
            return;
        }
        if persist {
            self.saved_search_mode = mode;
            self.search_override = None;
            self.persist_settings();
        }
        self.search_mode = mode;
        if self.active_tab().search_active {
            self.run_active_search();
        }
    }

    pub fn reload_settings(&mut self) {
        let saved = load_settings().unwrap_or_default().migrate_legacy();
        self.saved_theme_settings = saved.theme;
        self.saved_search_mode = saved.search_mode;
        self.saved_ui_font = saved.ui_font;
        self.ui_font = saved.ui_font;
        self.remote_cache_settings = saved.remote_cache;
        self.saved_remote_cache_settings = saved.remote_cache;
        self.path_overflow_behavior = saved.path_overflow_behavior;
        self.path_reset_delay_ms = saved.path_reset_delay_ms.clamp(500, 10_000);
        self.pinned_paths = saved.pinned_paths.clone();
        self.confirm_mobile_delete = saved.confirm_mobile_delete;
        self.delete_warning_suppressed_until_ms = saved.delete_warning_suppressed_until_ms;
        self.refresh_remote_cache_usage();
        self.theme_settings = self.theme_overrides.apply(saved.theme);
        let requested_search_mode = self.search_override.unwrap_or(saved.search_mode);
        self.search_mode = if requested_search_mode == SearchMode::Everything
            && !crate::search::everything_available()
        {
            SearchMode::Default
        } else {
            requested_search_mode
        };
        self.set_tailscale_profiles(saved.tailscale_profiles, false);
        if self.theme_settings.appearance == AppearanceMode::System {
            self.system_dark = detect_system_dark();
        }
        self.supabase.settings = saved.supabase;
        if self.supabase.settings.device_id.trim().is_empty() {
            self.supabase.settings.device_id = uuid::Uuid::new_v4().to_string();
        }
        if self.supabase.settings.device_name.trim().is_empty() {
            self.supabase.settings.device_name = default_sync_device_name();
        }
        self.supabase.email_input = self.supabase.settings.email.clone();
        self.configure_supabase_worker();
        if self.active_tab().search_active {
            self.run_active_search();
        }
    }

    pub fn set_appearance_mode(&mut self, appearance: AppearanceMode) {
        self.apply_theme_patch(
            ThemePatch {
                appearance: Some(appearance),
                ..ThemePatch::default()
            },
            true,
        );
    }

    pub fn set_theme_color(&mut self, color: ThemeColor) {
        self.apply_theme_patch(
            ThemePatch {
                color: Some(color),
                ..ThemePatch::default()
            },
            true,
        );
    }

    pub fn set_theme_intensity_value(&mut self, value: f64) {
        let intensity = value.round().clamp(0.0, 100.0) as u8;
        self.apply_theme_patch(
            ThemePatch {
                intensity: Some(intensity),
                ..ThemePatch::default()
            },
            true,
        );
    }

    pub fn new_tab(&mut self) {
        self.close_tab_overlays();
        self.tab_group_scroll_anchor_id = None;
        self.tabs.push(TabState::default());
        self.active_tab = self.tabs.len() - 1;
        self.page = AppPage::Files;
        self.request_directory_reload();
        self.persist_mobile_browsing_state();
    }

    pub fn new_tab_in_group(&mut self, group_id: u64) {
        if self.tab_group(group_id).is_none() {
            return;
        }
        self.tab_group_scroll_anchor_id = None;
        let active_id = self.active_tab().id;
        let mut tab = TabState::default();
        let new_id = tab.id;
        tab.group_id = Some(group_id);
        let insert_at = self
            .tabs
            .iter()
            .rposition(|tab| tab.group_id == Some(group_id))
            .map_or(self.tabs.len(), |index| index + 1);
        self.tabs.insert(insert_at, tab);
        self.active_tab = self
            .tabs
            .iter()
            .position(|tab| tab.id == new_id)
            .unwrap_or_else(|| {
                self.tabs
                    .iter()
                    .position(|tab| tab.id == active_id)
                    .unwrap_or(0)
            });
        self.page = AppPage::Files;
        self.close_tab_overlays();
        self.request_directory_reload();
        self.persist_session();
    }

    pub(crate) fn select_tab_by_id(&mut self, tab_id: u64) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
            self.select_tab(index);
        }
    }

    #[cfg(test)]
    pub(crate) fn move_tab_to_index(&mut self, tab_id: u64, target: usize) {
        let Some(from) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let source_group = self.tabs[from].group_id;
        self.drop_tab_at(tab_id, target, source_group, target >= from);
    }

    /// Commit a tab drag as one atomic order + group-membership change.
    ///
    /// `target_group` is derived from the visual group span under the dragged
    /// tab. `None` explicitly means the tab was dropped outside every group.
    /// Keeping the membership change and reorder together prevents a frame in
    /// which one group is split into two visual islands.
    pub(crate) fn drop_tab_at(
        &mut self,
        tab_id: u64,
        target: usize,
        target_group: Option<u64>,
        moving_right: bool,
    ) {
        let Some(from) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let old_group = self.tabs[from].group_id;
        let target_group = target_group.filter(|group_id| self.tab_group(*group_id).is_some());
        let target = target.min(self.tabs.len().saturating_sub(1));
        if target == from && target_group == old_group {
            return;
        }

        let active_id = self.active_tab().id;
        let mut tab = self.tabs.remove(from);
        tab.group_id = target_group;
        let mut insert_at = target.min(self.tabs.len());

        if let Some(group_id) = target_group {
            // A group is always one contiguous range. Clamp an incoming tab to
            // that range (including the slot immediately after its last tab).
            let first = self
                .tabs
                .iter()
                .position(|tab| tab.group_id == Some(group_id));
            let last = self
                .tabs
                .iter()
                .rposition(|tab| tab.group_id == Some(group_id));
            if let (Some(first), Some(last)) = (first, last) {
                insert_at = insert_at.clamp(first, last + 1);
            }
        } else if let Some(group_id) = self.tabs.get(insert_at).and_then(|tab| tab.group_id) {
            // Dropping outside groups must never split the group currently at
            // the insertion point. Snap to the nearer directional boundary.
            let first = self
                .tabs
                .iter()
                .position(|tab| tab.group_id == Some(group_id))
                .unwrap_or(insert_at);
            let last = self
                .tabs
                .iter()
                .rposition(|tab| tab.group_id == Some(group_id))
                .unwrap_or(insert_at);
            if insert_at > first {
                insert_at = if moving_right { last + 1 } else { first };
            }
        }

        self.tabs.insert(insert_at.min(self.tabs.len()), tab);
        self.remove_empty_tab_group(old_group);
        if let Some(active_index) = self.tabs.iter().position(|tab| tab.id == active_id) {
            self.active_tab = active_index;
        }
        self.close_tab_overlays();
        self.persist_session();
    }

    pub(crate) fn move_tab_group_near(&mut self, group_id: u64, target_tab_id: u64, after: bool) {
        let Some(target_tab) = self.tabs.iter().find(|tab| tab.id == target_tab_id) else {
            return;
        };
        if target_tab.group_id == Some(group_id) {
            return;
        }
        let active_id = self.active_tab().id;
        let mut moved = Vec::new();
        let mut index = 0;
        while index < self.tabs.len() {
            if self.tabs[index].group_id == Some(group_id) {
                moved.push(self.tabs.remove(index));
            } else {
                index += 1;
            }
        }
        if moved.is_empty() {
            return;
        }

        let Some(target_index) = self.tabs.iter().position(|tab| tab.id == target_tab_id) else {
            // The target can only disappear if it belonged to the source group,
            // which was rejected above. Keep this guard for corrupted session data.
            self.tabs.extend(moved);
            return;
        };
        let target_group = self.tabs[target_index].group_id;
        let insert_at = if let Some(target_group) = target_group {
            if after {
                self.tabs
                    .iter()
                    .rposition(|tab| tab.group_id == Some(target_group))
                    .map_or(target_index + 1, |last| last + 1)
            } else {
                self.tabs
                    .iter()
                    .position(|tab| tab.group_id == Some(target_group))
                    .unwrap_or(target_index)
            }
        } else if after {
            target_index + 1
        } else {
            target_index
        };
        self.tabs.splice(insert_at..insert_at, moved);
        if let Some(active_index) = self.tabs.iter().position(|tab| tab.id == active_id) {
            self.active_tab = active_index;
        }
        self.close_tab_overlays();
        self.persist_session();
    }

    fn choose_tab_group_color(
        &self,
        ignored_group: Option<u64>,
        forbidden: [bool; TabGroupColor::ALL.len()],
    ) -> TabGroupColor {
        let mut counts = [0_usize; TabGroupColor::ALL.len()];
        for group in &self.tab_groups {
            if Some(group.id) == ignored_group {
                continue;
            }
            counts[group.color.palette_index()] += 1;
        }

        let start = self.tab_groups.len() % TabGroupColor::ALL.len();
        let mut best: Option<usize> = None;
        for offset in 0..TabGroupColor::ALL.len() {
            let index = (start + offset) % TabGroupColor::ALL.len();
            if forbidden[index] {
                continue;
            }
            if best.is_none_or(|current| counts[index] < counts[current]) {
                best = Some(index);
            }
        }
        TabGroupColor::ALL[best.unwrap_or(0)]
    }

    fn tab_group_color_for_layout(
        &self,
        group_ids: &[Option<u64>],
        new_members: &[bool],
        ignored_group: Option<u64>,
    ) -> TabGroupColor {
        let Some(start) = new_members.iter().position(|member| *member) else {
            return self.choose_tab_group_color(ignored_group, [false; TabGroupColor::ALL.len()]);
        };
        let end = new_members
            .iter()
            .rposition(|member| *member)
            .unwrap_or(start);
        let mut forbidden = [false; TabGroupColor::ALL.len()];
        for neighbor in [
            start.checked_sub(1),
            (end + 1 < group_ids.len()).then_some(end + 1),
        ] {
            let Some(index) = neighbor else {
                continue;
            };
            let Some(group_id) = group_ids.get(index).copied().flatten() else {
                continue;
            };
            if Some(group_id) == ignored_group {
                continue;
            }
            if let Some(group) = self.tab_group(group_id) {
                forbidden[group.color.palette_index()] = true;
            }
        }
        self.choose_tab_group_color(ignored_group, forbidden)
    }

    fn color_for_new_group_at_tab(&self, tab_index: usize) -> TabGroupColor {
        let old_group = self.tabs.get(tab_index).and_then(TabState::group_id);
        let disappearing_group = old_group.filter(|group_id| {
            self.tabs
                .iter()
                .filter(|tab| tab.group_id == Some(*group_id))
                .count()
                == 1
        });
        let mut group_ids = self.tabs.iter().map(TabState::group_id).collect::<Vec<_>>();
        let mut new_members = vec![false; group_ids.len()];
        if let Some(old_group_id) = old_group {
            group_ids.remove(tab_index);
            new_members.remove(tab_index);
            let insert_at = group_ids
                .iter()
                .rposition(|group| *group == Some(old_group_id))
                .map_or(tab_index.min(group_ids.len()), |index| index + 1);
            group_ids.insert(insert_at, None);
            new_members.insert(insert_at, true);
        } else if let Some(member) = new_members.get_mut(tab_index) {
            *member = true;
        }
        self.tab_group_color_for_layout(&group_ids, &new_members, disappearing_group)
    }

    pub(crate) fn tab_group_color_for_drag(
        &self,
        source_tab_id: u64,
        target_tab_id: u64,
    ) -> TabGroupColor {
        let Some(source_index) = self.tabs.iter().position(|tab| tab.id == source_tab_id) else {
            return self.choose_tab_group_color(None, [false; TabGroupColor::ALL.len()]);
        };
        let Some(target_index) = self.tabs.iter().position(|tab| tab.id == target_tab_id) else {
            return self.choose_tab_group_color(None, [false; TabGroupColor::ALL.len()]);
        };
        if source_index == target_index
            || self.tabs[source_index].group_id.is_some()
            || self.tabs[target_index].group_id.is_some()
        {
            return self.choose_tab_group_color(None, [false; TabGroupColor::ALL.len()]);
        }

        let source_was_before_target = source_index < target_index;
        let mut group_ids = self.tabs.iter().map(TabState::group_id).collect::<Vec<_>>();
        let mut new_members = vec![false; group_ids.len()];
        group_ids.remove(source_index);
        new_members.remove(source_index);
        let target_after_remove = if source_was_before_target {
            target_index - 1
        } else {
            target_index
        };
        new_members[target_after_remove] = true;
        let insert_at = if source_was_before_target {
            target_after_remove
        } else {
            target_after_remove + 1
        };
        group_ids.insert(insert_at, None);
        new_members.insert(insert_at, true);
        self.tab_group_color_for_layout(&group_ids, &new_members, None)
    }

    pub fn create_tab_group_for_tab(&mut self, tab_id: u64) {
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let old_group = self.tabs[tab_index].group_id;
        let color = self.color_for_new_group_at_tab(tab_index);
        let group_id = TAB_GROUP_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.tab_groups.push(TabGroupState {
            id: group_id,
            name: String::new(),
            color,
            collapsed: false,
        });

        if let Some(old_group_id) = old_group {
            let active_id = self.active_tab().id;
            let mut tab = self.tabs.remove(tab_index);
            tab.group_id = Some(group_id);
            let insert_at = self
                .tabs
                .iter()
                .rposition(|tab| tab.group_id == Some(old_group_id))
                .map_or(tab_index.min(self.tabs.len()), |index| index + 1);
            self.tabs.insert(insert_at, tab);
            self.active_tab = self
                .tabs
                .iter()
                .position(|tab| tab.id == active_id)
                .unwrap_or(0);
        } else {
            self.tabs[tab_index].group_id = Some(group_id);
        }
        self.remove_empty_tab_group(old_group);
        self.tab_context_menu_tab_id = None;
        self.tab_group_editor_id = Some(group_id);
        self.persist_session();
    }

    pub(crate) fn create_tab_group_from_drag(
        &mut self,
        source_tab_id: u64,
        target_tab_id: u64,
        anchor_x: f64,
    ) {
        if source_tab_id == target_tab_id {
            return;
        }
        let Some(source_index) = self.tabs.iter().position(|tab| tab.id == source_tab_id) else {
            return;
        };
        let Some(target_index) = self.tabs.iter().position(|tab| tab.id == target_tab_id) else {
            return;
        };
        if self.tabs[source_index].group_id.is_some() || self.tabs[target_index].group_id.is_some()
        {
            return;
        }

        let active_id = self.active_tab().id;
        let color = self.tab_group_color_for_drag(source_tab_id, target_tab_id);
        let group_id = TAB_GROUP_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.tab_groups.push(TabGroupState {
            id: group_id,
            name: String::new(),
            color,
            collapsed: false,
        });

        let source_was_before_target = source_index < target_index;
        let mut source = self.tabs.remove(source_index);
        source.group_id = Some(group_id);
        let Some(target_after_remove) = self.tabs.iter().position(|tab| tab.id == target_tab_id)
        else {
            self.tab_groups.retain(|group| group.id != group_id);
            return;
        };
        self.tabs[target_after_remove].group_id = Some(group_id);
        let insert_at = if source_was_before_target {
            target_after_remove
        } else {
            target_after_remove + 1
        };
        self.tabs.insert(insert_at, source);

        if let Some(active_index) = self.tabs.iter().position(|tab| tab.id == active_id) {
            self.active_tab = active_index;
        }
        self.tab_overlay_anchor_x = anchor_x.max(8.0);
        self.tab_context_menu_tab_id = None;
        self.tab_group_editor_id = Some(group_id);
        self.persist_session();
    }

    pub fn assign_tab_to_group(&mut self, tab_id: u64, group_id: u64) {
        if self.tab_group(group_id).is_none() {
            return;
        }
        let Some(from) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let old_group = self.tabs[from].group_id;
        if old_group == Some(group_id) {
            self.close_tab_overlays();
            return;
        }
        let active_id = self.active_tab().id;
        let mut tab = self.tabs.remove(from);
        tab.group_id = Some(group_id);
        let insert_at = self
            .tabs
            .iter()
            .rposition(|tab| tab.group_id == Some(group_id))
            .map_or(self.tabs.len(), |index| index + 1);
        self.tabs.insert(insert_at, tab);
        self.active_tab = self
            .tabs
            .iter()
            .position(|tab| tab.id == active_id)
            .unwrap_or(0);
        self.remove_empty_tab_group(old_group);
        self.close_tab_overlays();
        self.persist_session();
    }

    pub fn remove_tab_from_group(&mut self, tab_id: u64) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(old_group) = self.tabs[index].group_id else {
            self.close_tab_overlays();
            return;
        };
        let active_id = self.active_tab().id;
        let mut tab = self.tabs.remove(index);
        tab.group_id = None;
        let insert_at = self
            .tabs
            .iter()
            .rposition(|tab| tab.group_id == Some(old_group))
            .map_or(index.min(self.tabs.len()), |group_last| group_last + 1);
        self.tabs.insert(insert_at, tab);
        self.active_tab = self
            .tabs
            .iter()
            .position(|tab| tab.id == active_id)
            .unwrap_or(0);
        self.remove_empty_tab_group(Some(old_group));
        self.close_tab_overlays();
        self.persist_session();
    }

    pub fn ungroup_tab_group(&mut self, group_id: u64) {
        for tab in &mut self.tabs {
            if tab.group_id == Some(group_id) {
                tab.group_id = None;
            }
        }
        self.tab_groups.retain(|group| group.id != group_id);
        self.close_tab_overlays();
        self.persist_session();
    }

    /// Returns true when closing this group should close the whole application.
    pub fn close_tab_group(&mut self, group_id: u64) -> bool {
        let group_tab_count = self
            .tabs
            .iter()
            .filter(|tab| tab.group_id == Some(group_id))
            .count();
        if group_tab_count == 0 {
            self.close_tab_overlays();
            return false;
        }
        if group_tab_count == self.tabs.len() {
            self.persist_session();
            return true;
        }

        let active_id = self.active_tab().id;
        let active_was_in_group = self.active_tab().group_id == Some(group_id);
        self.tabs.retain(|tab| tab.group_id != Some(group_id));
        self.tab_groups.retain(|group| group.id != group_id);
        if !active_was_in_group {
            self.active_tab = self
                .tabs
                .iter()
                .position(|tab| tab.id == active_id)
                .unwrap_or(0);
        } else {
            self.active_tab = self.active_tab.min(self.tabs.len() - 1);
            if self.active_tab().entries.is_empty() {
                self.request_directory_reload();
            }
        }
        self.close_tab_overlays();
        self.persist_session();
        false
    }

    fn remove_empty_tab_group(&mut self, group_id: Option<u64>) {
        let Some(group_id) = group_id else {
            return;
        };
        if !self.tabs.iter().any(|tab| tab.group_id == Some(group_id)) {
            self.tab_groups.retain(|group| group.id != group_id);
            if self.tab_group_editor_id == Some(group_id) {
                self.tab_group_editor_id = None;
            }
        }
    }

    pub fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.close_tab_overlays();
            self.tab_group_scroll_anchor_id = None;
            self.active_tab = index;
            self.page = AppPage::Files;
            if let Some(location) =
                crate::archive::parse_virtual_path(&self.active_tab().current_dir)
            {
                self.load_archive_location(location);
            } else if let Some(location) = parse_taildrive_path(&self.active_tab().current_dir) {
                self.load_taildrive_location(location);
            } else if self.active_tab().entries.is_empty() {
                self.request_directory_reload();
            }
            self.persist_mobile_browsing_state();
        }
    }

    /// Returns true when closing this tab should close the whole application.
    pub fn close_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        if self.tabs.len() == 1 {
            self.persist_session();
            return true;
        }
        let active_id = self.active_tab().id;
        let removed_group = self.tabs[index].group_id;
        let removed_id = self.tabs[index].id;
        self.tabs.remove(index);
        self.active_tab = self
            .tabs
            .iter()
            .position(|tab| tab.id == active_id)
            .unwrap_or_else(|| index.min(self.tabs.len() - 1));
        if self.tab_context_menu_tab_id == Some(removed_id) {
            self.tab_context_menu_tab_id = None;
        }
        self.remove_empty_tab_group(removed_group);
        self.persist_mobile_browsing_state();
        self.persist_session();
        false
    }

    pub fn can_go_back(&self) -> bool {
        self.active_tab().can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.active_tab().can_go_forward()
    }

    pub fn can_go_up(&self) -> bool {
        if let Some(location) = crate::archive::parse_virtual_path(&self.active_tab().current_dir) {
            return crate::archive::parent_path(&location).is_some();
        }
        parse_taildrive_path(&self.active_tab().current_dir).map_or_else(
            || self.active_tab().current_dir.parent().is_some(),
            |location| taildrive_parent(&location).is_some(),
        )
    }

    pub fn can_go_home(&self) -> bool {
        home_dir().as_ref() != Some(&self.active_tab().current_dir)
    }

    pub fn set_address_input(&mut self, value: String) {
        self.active_tab_mut().set_address_input(value);
    }

    pub fn set_search_input(&mut self, value: String) {
        self.active_tab_mut().set_search_input(value);
        self.run_active_search();
    }

    pub fn expand_search_field(&mut self) {
        self.active_tab_mut().search_field_expanded = true;
    }

    pub fn clear_or_collapse_search(&mut self) {
        if self.active_tab().search_input.trim().is_empty() {
            self.active_tab_mut().search_field_expanded = false;
        } else {
            self.clear_search();
        }
    }

    pub fn submit_search(&mut self, value: String) {
        self.active_tab_mut().set_search_input(value);
        self.run_active_search();
    }

    pub fn set_supabase_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<crate::supabase_sync::Command>,
    ) {
        self.supabase.sender = Some(sender);
        self.persist_settings();
        self.configure_supabase_worker();
    }

    fn configure_supabase_worker(&mut self) {
        let Some(sender) = self.supabase.sender.as_ref() else {
            return;
        };
        let _ = sender.send(crate::supabase_sync::Command::Configure {
            settings: self.supabase.settings.clone(),
            session_path: supabase_session_path(),
        });
    }

    pub fn apply_supabase_settings(&mut self) {
        self.persist_settings();
        self.supabase.status = "Applying device sync settings…".to_owned();
        self.configure_supabase_worker();
    }

    pub fn supabase_project_url(&self) -> &str {
        &self.supabase.settings.project_url
    }

    pub fn set_supabase_project_url(&mut self, value: String) {
        self.supabase.settings.project_url = single_line_input(value);
        self.persist_settings();
    }

    pub fn supabase_publishable_key(&self) -> &str {
        &self.supabase.settings.publishable_key
    }

    pub fn set_supabase_publishable_key(&mut self, value: String) {
        self.supabase.settings.publishable_key = single_line_input(value);
        self.persist_settings();
    }

    pub fn supabase_device_name(&self) -> &str {
        &self.supabase.settings.device_name
    }

    pub fn set_supabase_device_name(&mut self, value: String) {
        self.supabase.settings.device_name = single_line_input(value);
        self.persist_settings();
    }

    pub fn supabase_email_input(&self) -> &str {
        &self.supabase.email_input
    }

    pub fn set_supabase_email_input(&mut self, value: String) {
        self.supabase.email_input = single_line_input(value);
    }

    pub fn supabase_otp_input(&self) -> &str {
        &self.supabase.otp_input
    }

    pub fn set_supabase_otp_input(&mut self, value: String) {
        self.supabase.otp_input = single_line_input(value);
    }

    pub fn supabase_status(&self) -> &str {
        &self.supabase.status
    }

    pub fn supabase_signed_in(&self) -> bool {
        self.supabase.signed_in
    }

    pub fn supabase_user_email(&self) -> &str {
        &self.supabase.user_email
    }

    pub fn supabase_devices(&self) -> &[crate::supabase_sync::Device] {
        &self.supabase.devices
    }

    pub fn supabase_inbox(&self) -> &[crate::supabase_sync::InboxItem] {
        &self.supabase.inbox
    }

    pub fn supabase_local_device_key(&self) -> &str {
        &self.supabase.settings.device_id
    }

    pub fn request_supabase_otp(&mut self) {
        self.supabase.settings.email = self.supabase.email_input.trim().to_owned();
        self.persist_settings();
        self.configure_supabase_worker();
        let Some(sender) = self.supabase.sender.as_ref() else {
            self.supabase.status = "Supabase worker is not ready".to_owned();
            return;
        };
        self.supabase.status = "Requesting email OTP…".to_owned();
        let _ = sender.send(crate::supabase_sync::Command::RequestOtp {
            email: self.supabase.email_input.clone(),
        });
    }

    pub fn verify_supabase_otp(&mut self) {
        self.configure_supabase_worker();
        let Some(sender) = self.supabase.sender.as_ref() else {
            self.supabase.status = "Supabase worker is not ready".to_owned();
            return;
        };
        self.supabase.status = "Verifying OTP…".to_owned();
        let _ = sender.send(crate::supabase_sync::Command::VerifyOtp {
            email: self.supabase.email_input.clone(),
            token: self.supabase.otp_input.clone(),
        });
    }

    pub fn sign_out_supabase(&mut self) {
        let Some(sender) = self.supabase.sender.as_ref() else {
            return;
        };
        let _ = sender.send(crate::supabase_sync::Command::SignOut);
    }

    pub fn refresh_supabase_devices(&mut self) {
        let Some(sender) = self.supabase.sender.as_ref() else {
            return;
        };
        self.supabase.status = "Refreshing devices…".to_owned();
        let _ = sender.send(crate::supabase_sync::Command::RefreshDevices);
    }

    pub fn send_clipboard_to_supabase_device(&mut self, receiver_device_id: String) {
        let text = match self.system_clipboard_text() {
            Ok(text) => text,
            Err(error) => {
                self.supabase.status = error;
                return;
            }
        };
        let Some(sender) = self.supabase.sender.as_ref() else {
            self.supabase.status = "Supabase worker is not ready".to_owned();
            return;
        };
        self.supabase.status = "Sending clipboard…".to_owned();
        let _ = sender.send(crate::supabase_sync::Command::SendClipboard {
            receiver_device_id,
            text,
        });
    }

    pub fn send_selected_file_to_supabase_device(&mut self, receiver_device_id: String) {
        let Some(path) = self.active_tab().selected_path.clone() else {
            self.supabase.status = "Select a file first".to_owned();
            return;
        };
        if parse_taildrive_path(&path).is_some()
            || crate::archive::parse_virtual_path(&path).is_some()
        {
            self.supabase.status =
                "Device send currently requires a local file; copy the remote file locally first"
                    .to_owned();
            return;
        }
        let Some(sender) = self.supabase.sender.as_ref() else {
            self.supabase.status = "Supabase worker is not ready".to_owned();
            return;
        };
        self.supabase.status = "Sending file…".to_owned();
        let _ = sender.send(crate::supabase_sync::Command::SendFile {
            receiver_device_id,
            path,
        });
    }

    pub fn receive_supabase_file(&mut self, item: crate::supabase_sync::InboxItem) {
        let Some(sender) = self.supabase.sender.as_ref() else {
            return;
        };
        self.supabase.status = format!("Receiving {}…", item.file_name);
        let _ = sender.send(crate::supabase_sync::Command::ReceiveFile { item });
    }

    pub fn copy_supabase_clipboard(&mut self, item: crate::supabase_sync::InboxItem) {
        match self.set_system_clipboard_text(item.clipboard_text.clone()) {
            Ok(()) => {
                self.supabase.status = "Received clipboard copied to this device".to_owned();
                if let Some(sender) = self.supabase.sender.as_ref() {
                    let _ = sender.send(crate::supabase_sync::Command::AckTransfer {
                        transfer_id: item.id,
                    });
                }
            }
            Err(error) => self.supabase.status = error,
        }
    }

    pub fn apply_supabase_event(&mut self, event: crate::supabase_sync::Event) {
        match event {
            crate::supabase_sync::Event::AuthState {
                signed_in,
                email,
                status,
            } => {
                self.supabase.signed_in = signed_in;
                self.supabase.user_email = email;
                self.supabase.status = status;
                #[cfg(target_os = "android")]
                if signed_in {
                    if let Some(app) = self.android_app.as_ref() {
                        let _ = crate::android_platform::ensure_notification_permission(app);
                    }
                }
                if !signed_in {
                    self.supabase.devices.clear();
                    self.supabase.inbox.clear();
                    self.supabase.push_token.clear();
                } else {
                    #[cfg(target_os = "android")]
                    self.sync_supabase_push_token();
                }
            }
            crate::supabase_sync::Event::OtpSent(status) => {
                self.supabase.status = status;
            }
            crate::supabase_sync::Event::Devices(result) => match result {
                Ok(devices) => {
                    self.supabase.devices = devices;
                    self.supabase.status = "Device list updated".to_owned();
                }
                Err(error) => self.supabase.status = error,
            },
            crate::supabase_sync::Event::Inbox(result) => match result {
                Ok(inbox) => {
                    let old_ids = self
                        .supabase
                        .inbox
                        .iter()
                        .map(|item| item.id.clone())
                        .collect::<BTreeSet<_>>();
                    let new_items = inbox
                        .iter()
                        .filter(|item| !old_ids.contains(&item.id))
                        .count();
                    self.supabase.inbox = inbox;
                    if new_items > 0 {
                        self.notify_incoming_supabase(new_items);
                    }
                    #[cfg(target_os = "android")]
                    self.sync_supabase_push_token();
                }
                Err(error) => self.supabase.status = error,
            },
            crate::supabase_sync::Event::PushTokenUpdated { token, result } => match result {
                Ok(()) => self.supabase.push_token = token,
                Err(error) => self.supabase.status = error,
            },
            crate::supabase_sync::Event::Sent(result) => {
                self.supabase.status = result.unwrap_or_else(|error| error);
            }
            crate::supabase_sync::Event::FileReceived {
                transfer_id,
                result,
            } => match result {
                Ok(path) => {
                    self.supabase.inbox.retain(|item| item.id != transfer_id);
                    self.supabase.status = format!("Received {}", path.display());
                    #[cfg(target_os = "android")]
                    if let Some(app) = self.android_app.as_ref() {
                        let _ = crate::android_platform::notify_file_changes(app, &[path]);
                    }
                }
                Err(error) => self.supabase.status = error,
            },
            crate::supabase_sync::Event::TransferAcked {
                transfer_id,
                result,
            } => match result {
                Ok(()) => self.supabase.inbox.retain(|item| item.id != transfer_id),
                Err(error) => self.supabase.status = error,
            },
        }
    }

    #[cfg(target_os = "android")]
    fn sync_supabase_push_token(&mut self) {
        if !self.supabase.signed_in {
            return;
        }
        let Some(app) = self.android_app.as_ref() else {
            return;
        };
        let Ok(token) = crate::android_platform::fcm_token(app) else {
            return;
        };
        let token = token.trim();
        if token.is_empty() || token == self.supabase.push_token {
            return;
        }
        if let Some(sender) = self.supabase.sender.as_ref() {
            let _ = sender.send(crate::supabase_sync::Command::UpdatePushToken {
                token: token.to_owned(),
            });
        }
    }

    fn notify_incoming_supabase(&self, _count: usize) {
        #[cfg(target_os = "android")]
        if let Some(app) = self.android_app.as_ref() {
            let count = _count;
            let detail = if count == 1 {
                "A file or clipboard item is ready to receive".to_owned()
            } else {
                format!("{count} items are ready to receive")
            };
            let _ = crate::android_platform::notify_incoming_sync(
                app,
                "FastExplorer device transfer".to_owned(),
                detail,
            );
        }
    }

    pub fn copy_selected_path_to_system_clipboard(&mut self) {
        let Some(path) = self.active_tab().selected_path.clone() else {
            self.active_tab_mut().status = "Select an item to copy its path".to_owned();
            return;
        };
        let text = self.display_path_for(&path);
        self.active_tab_mut().status = match self.set_system_clipboard_text(text) {
            Ok(()) => "Path copied to system clipboard".to_owned(),
            Err(error) => format!("Cannot copy path: {error}"),
        };
    }

    fn set_system_clipboard_text(&self, text: String) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            crate::system_clipboard::set_text(self.android_app.as_ref(), text)
        }
        #[cfg(not(target_os = "android"))]
        {
            crate::system_clipboard::set_text(None, text)
        }
    }

    fn system_clipboard_text(&self) -> Result<String, String> {
        #[cfg(target_os = "android")]
        {
            crate::system_clipboard::get_text(self.android_app.as_ref())
        }
        #[cfg(not(target_os = "android"))]
        {
            crate::system_clipboard::get_text(None)
        }
    }

    pub fn set_persistence_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<PersistCommand>,
    ) {
        self.persistence_sender = Some(sender);
        // Persist startup-generated device IDs/names once the async writer exists.
        self.persist_settings();
    }

    pub fn set_remote_prepare_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<RemotePrepareRequest>,
    ) {
        self.remote_prepare_sender = Some(sender);
    }

    fn prepare_remote_entry(&mut self, entry: FileEntry, purpose: RemotePreparePurpose) {
        let Some(source @ TaildriveLocation::Remote { .. }) = entry.remote.clone() else {
            return;
        };
        let source_key = remote_cache_source_key(&source);
        if self.remote_prepare_pending.contains(&source_key)
            || self
                .pending_remote_cache_downloads
                .values()
                .any(|pending| pending.source_key == source_key)
        {
            self.active_tab_mut().status = format!("Already preparing {}…", entry.name);
            self.transfer_popup_open = true;
            return;
        }
        let Some(cache_root) = self.remote_cache_root() else {
            self.active_tab_mut().status = "Cannot determine remote file cache location".to_owned();
            return;
        };
        let request = RemotePrepareRequest {
            source,
            source_location: self.active_tab().current_dir.clone(),
            display_name: entry.name.clone(),
            remote_size: entry.size,
            remote_modified: entry.remote_modified.clone().unwrap_or_default(),
            cache_root,
            cache_settings: self.remote_cache_settings,
            purpose: purpose.clone(),
        };
        #[cfg(test)]
        if self.remote_prepare_sender.is_none() {
            let result = perform_remote_prepare(&request);
            self.apply_remote_prepare_event(RemotePrepareEvent { request, result });
            return;
        }
        let Some(sender) = self.remote_prepare_sender.as_ref() else {
            self.active_tab_mut().status = "Remote file worker is starting…".to_owned();
            return;
        };
        self.remote_prepare_pending.insert(source_key);
        if sender.send(request).is_err() {
            self.remote_prepare_pending.remove(&remote_cache_source_key(
                entry.remote.as_ref().expect("remote entry"),
            ));
            self.active_tab_mut().status = "Remote file worker stopped".to_owned();
            return;
        }
        self.active_tab_mut().status = match purpose {
            RemotePreparePurpose::Share => format!("Preparing {} for Quick Share…", entry.name),
            RemotePreparePurpose::ImportArchive { .. } => {
                format!("Preparing {} for archive…", entry.name)
            }
            RemotePreparePurpose::Open => format!("Preparing {}…", entry.name),
        };
    }

    pub fn apply_remote_prepare_event(&mut self, event: RemotePrepareEvent) {
        let source_key = remote_cache_source_key(&event.request.source);
        self.remote_prepare_pending.remove(&source_key);
        let source_is_current = self.active_tab().current_dir == event.request.source_location;
        match event.result {
            Ok(RemotePrepareResult::Cached(path)) => match event.request.purpose {
                RemotePreparePurpose::ImportArchive {
                    current,
                    name,
                    replace,
                } => {
                    let Some(sender) = self.archive_sender.as_ref() else {
                        if source_is_current {
                            self.active_tab_mut().status = "Archive worker is not ready".to_owned();
                        }
                        return;
                    };
                    if sender
                        .send(crate::archive::Command::Import {
                            current,
                            source: path,
                            name: name.clone(),
                            replace,
                        })
                        .is_err()
                    {
                        if source_is_current {
                            self.active_tab_mut().status = "Archive worker stopped".to_owned();
                        }
                    } else if source_is_current {
                        self.active_tab_mut().status = format!("Adding {name} to archive…");
                    }
                }
                RemotePreparePurpose::Share => {
                    let action = self.share_path(&path);
                    if source_is_current {
                        self.active_tab_mut().status = match action {
                            Ok(()) => format!("Sharing from cache: {}", event.request.display_name),
                            Err(error) => format!("Cannot share TailDrive file: {error}"),
                        };
                    }
                }
                RemotePreparePurpose::Open => {
                    let action = self.open_remote_cache_path(&path);
                    if source_is_current {
                        self.active_tab_mut().status = match action {
                            Ok(()) if is_aab_name(&event.request.display_name) => {
                                format!("Preparing installer: {}", event.request.display_name)
                            }
                            Ok(()) if is_android_install_name(&event.request.display_name) => {
                                format!("Installer opened: {}", event.request.display_name)
                            }
                            Ok(()) => format!("Opened from cache: {}", event.request.display_name),
                            Err(error) => format!("Cannot open TailDrive file: {error}"),
                        };
                    }
                }
            },
            Ok(RemotePrepareResult::Download {
                destination,
                cache_file_name,
                source_key,
            }) => {
                let TaildriveLocation::Remote {
                    profile_id,
                    device_id,
                    share,
                    remote_path,
                } = event.request.source
                else {
                    return;
                };
                let transfer_id = next_transfer_id();
                let command = crate::tailscale::Command::TaildriveDownload {
                    profile_id,
                    device_id,
                    share,
                    path: remote_path,
                    destination,
                    display_name: event.request.display_name.clone(),
                    source_location: event.request.source_location.clone(),
                    transfer_id: transfer_id.clone(),
                    open_after: true,
                    source_was_cut: false,
                    replace: false,
                };
                if let Err(error) = self.dispatch_taildrive_transfer(command) {
                    if source_is_current {
                        self.active_tab_mut().status = error;
                    }
                    return;
                }
                self.pending_remote_cache_downloads.insert(
                    transfer_id.clone(),
                    PendingRemoteCacheDownload {
                        source_key,
                        file_name: cache_file_name,
                        display_name: event.request.display_name.clone(),
                        remote_size: event.request.remote_size,
                        remote_modified: event.request.remote_modified,
                        purpose: event.request.purpose.clone(),
                    },
                );
                let preparing_apk = matches!(event.request.purpose, RemotePreparePurpose::Open)
                    && is_android_install_name(&event.request.display_name);
                let phase = match event.request.purpose {
                    RemotePreparePurpose::Share => "Downloading to share",
                    RemotePreparePurpose::ImportArchive { .. } => "Downloading for archive",
                    RemotePreparePurpose::Open if preparing_apk => "Preparing app install",
                    RemotePreparePurpose::Open => "Downloading",
                };
                self.begin_file_transfer(transfer_id, event.request.display_name.clone(), phase);
                self.transfer_popup_open = true;
                if source_is_current {
                    self.active_tab_mut().status = match event.request.purpose {
                        RemotePreparePurpose::Share => {
                            format!(
                                "Downloading {} for Quick Share…",
                                event.request.display_name
                            )
                        }
                        RemotePreparePurpose::ImportArchive { .. } => {
                            format!("Downloading {} for archive…", event.request.display_name)
                        }
                        RemotePreparePurpose::Open if preparing_apk => {
                            format!("Preparing {} for installation…", event.request.display_name)
                        }
                        RemotePreparePurpose::Open => {
                            format!(
                                "Downloading TailDrive file: {}…",
                                event.request.display_name
                            )
                        }
                    };
                }
            }
            Err(error) => {
                if source_is_current {
                    self.active_tab_mut().status = match event.request.purpose {
                        RemotePreparePurpose::Share => {
                            format!("Cannot prepare TailDrive file for sharing: {error}")
                        }
                        RemotePreparePurpose::ImportArchive { .. } => {
                            format!("Cannot prepare TailDrive file for archive: {error}")
                        }
                        RemotePreparePurpose::Open => {
                            format!("Cannot prepare TailDrive file: {error}")
                        }
                    };
                }
            }
        }
    }

    pub fn taildrive_directory_cache_ready(&mut self) {
        let Some(location @ TaildriveLocation::Remote { .. }) =
            parse_taildrive_path(&self.active_tab().current_dir)
        else {
            return;
        };
        // Re-run the active remote listing now that the disk cache is memory-resident.
        // The live TailDrive request still follows and replaces this historical snapshot.
        self.load_taildrive_location(location);
    }

    pub fn set_cache_sender(&mut self, sender: tokio::sync::mpsc::UnboundedSender<CacheCommand>) {
        // A newly built worker cannot complete requests that belonged to an older
        // receiver. Drop any stale loading token before issuing work to this sender.
        self.cache_sender = Some(sender);
        self.remote_cache_usage_pending = None;
        self.remote_cache_usage_refresh_queued = false;
        self.refresh_remote_cache_usage();
    }

    pub fn apply_cache_event(&mut self, event: CacheEvent) {
        let completes_current_refresh = event
            .usage_refresh_id
            .is_some_and(|id| self.remote_cache_usage_pending == Some(id));
        if completes_current_refresh {
            self.remote_cache_usage_pending = None;
        }
        match event.result {
            Ok(bytes) => self.remote_cache_usage_bytes = bytes,
            Err(error) => eprintln!("FastExplorer: remote cache maintenance failed: {error}"),
        }
        if completes_current_refresh && self.remote_cache_usage_refresh_queued {
            self.remote_cache_usage_refresh_queued = false;
            self.refresh_remote_cache_usage();
        }
    }

    pub fn set_local_file_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<LocalFileCommand>,
    ) {
        self.local_file_sender = Some(sender);
    }

    #[cfg(target_os = "android")]
    fn notify_android_file_changes(&self, paths: &[PathBuf]) {
        if let Some(app) = self.android_app.as_ref() {
            let _ = crate::android_platform::notify_file_changes(app, paths);
        }
    }

    fn submit_local_file_command(&mut self, command: LocalFileCommand, status: String) -> bool {
        #[cfg(test)]
        if self.local_file_sender.is_none() {
            let result = perform_local_file_command(&command);
            let ok = result.is_ok();
            self.apply_local_file_event(LocalFileEvent { command, result });
            return ok;
        }
        let Some(sender) = self.local_file_sender.as_ref() else {
            self.active_tab_mut().status = "File operation worker is starting…".to_owned();
            return false;
        };
        #[cfg(target_os = "android")]
        let background_transfer = matches!(&command, LocalFileCommand::CopyMove { .. });
        if sender.send(command).is_err() {
            self.active_tab_mut().status = "File operation worker stopped".to_owned();
            false
        } else {
            self.active_tab_mut().status = status;
            #[cfg(target_os = "android")]
            if background_transfer {
                self.ensure_android_transfer_service();
            }
            true
        }
    }

    pub fn apply_local_file_event(&mut self, event: LocalFileEvent) {
        let current = match &event.command {
            LocalFileCommand::CreateDir { current, .. }
            | LocalFileCommand::CopyMove { current, .. }
            | LocalFileCommand::Delete { current, .. }
            | LocalFileCommand::Rename { current, .. } => current.clone(),
        };
        let active = self.active_tab().current_dir == current;
        match (event.command, event.result) {
            (LocalFileCommand::CreateDir { path, .. }, Ok(())) => {
                #[cfg(target_os = "android")]
                self.notify_android_file_changes(&[path.clone()]);
                if active {
                    let rename = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "New folder".to_owned());
                    self.active_tab_mut().selected_path = Some(path);
                    self.active_tab_mut().rename_input = Some(rename);
                    self.active_tab_mut().rename_replace_on_type = true;
                    self.active_tab_mut().rename_keyboard_suffix = Some(String::new());
                    self.request_directory_reload();
                }
            }
            (
                LocalFileCommand::CopyMove {
                    destination,
                    cut,
                    source,
                    ..
                },
                Ok(()),
            ) => {
                if cut
                    && self
                        .file_clipboard
                        .as_ref()
                        .is_some_and(|clipboard| clipboard.path == source)
                {
                    self.file_clipboard = None;
                }
                if active {
                    self.active_tab_mut().selected_path = Some(destination);
                    self.request_directory_reload();
                }
            }
            (LocalFileCommand::Delete { path, .. }, Ok(())) => {
                #[cfg(target_os = "android")]
                self.notify_android_file_changes(&[path.clone()]);
                if self
                    .file_clipboard
                    .as_ref()
                    .is_some_and(|clipboard| clipboard.path == path)
                {
                    self.file_clipboard = None;
                }
                if active {
                    self.active_tab_mut().selected_path = None;
                    self.request_directory_reload();
                }
            }
            (
                LocalFileCommand::Rename {
                    source,
                    destination,
                    ..
                },
                Ok(()),
            ) => {
                #[cfg(target_os = "android")]
                self.notify_android_file_changes(&[source.clone(), destination.clone()]);
                if let Some(clipboard) = self.file_clipboard.as_mut()
                    && clipboard.path == source
                {
                    clipboard.path = destination.clone();
                }
                if active {
                    self.active_tab_mut().rename_input = None;
                    self.active_tab_mut().rename_replace_on_type = false;
                    self.active_tab_mut().rename_keyboard_suffix = None;
                    self.active_tab_mut().selected_path = Some(destination);
                    self.request_directory_reload();
                }
            }
            (command, Err(error)) => {
                let label = match &command {
                    LocalFileCommand::CreateDir { .. } => "Create folder failed",
                    LocalFileCommand::CopyMove { .. } => "Paste failed",
                    LocalFileCommand::Delete { .. } => {
                        if cfg!(target_os = "android") {
                            "Delete failed"
                        } else {
                            "Move to Trash failed"
                        }
                    }
                    LocalFileCommand::Rename { .. } => "Rename failed",
                };
                eprintln!("FastExplorer: {label}: {error}");
                if active {
                    self.active_tab_mut().status = format!("{label}: {error}");
                }
            }
        }
    }

    pub fn set_archive_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<crate::archive::Command>,
    ) {
        self.archive_sender = Some(sender);
        if let Some(location) = crate::archive::parse_virtual_path(&self.active_tab().current_dir) {
            self.load_archive_location(location);
        }
    }

    fn load_archive_location(&mut self, location: crate::archive::ArchiveLocation) {
        self.archive_generation = self.archive_generation.wrapping_add(1);
        let generation = self.archive_generation;
        let show_hidden = self.active_tab().show_hidden;
        self.active_tab_mut().entries.clear();
        self.active_tab_mut().status = "Loading archive…".to_owned();
        let Some(sender) = self.archive_sender.as_ref() else {
            self.active_tab_mut().status = "Archive worker is starting…".to_owned();
            return;
        };
        if sender
            .send(crate::archive::Command::List {
                generation,
                location,
                show_hidden,
            })
            .is_err()
        {
            self.active_tab_mut().status = "Archive worker stopped".to_owned();
        }
    }

    fn archive_edit_destination(
        &self,
        location: &crate::archive::ArchiveLocation,
    ) -> Option<PathBuf> {
        use std::hash::{Hash, Hasher};
        let root = self.remote_cache_root()?.join("archive-edit");
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        location.hash(&mut hasher);
        let id = hasher.finish();
        let name = Path::new(&location.inner_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "archive-member".to_owned());
        Some(root.join(format!("{id:016x}-{name}")))
    }

    fn open_archive_member(
        &mut self,
        location: crate::archive::ArchiveLocation,
        share_after: bool,
    ) {
        let Some(destination) = self.archive_edit_destination(&location) else {
            self.active_tab_mut().status = "Cannot prepare archive edit cache".to_owned();
            return;
        };
        let Some(sender) = self.archive_sender.as_ref() else {
            self.active_tab_mut().status = "Archive worker is not ready".to_owned();
            return;
        };
        if sender
            .send(crate::archive::Command::OpenForEdit {
                location: location.clone(),
                destination,
                share_after,
            })
            .is_err()
        {
            self.active_tab_mut().status = "Archive worker stopped".to_owned();
            return;
        }
        let name = Path::new(&location.inner_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| location.inner_path.clone());
        self.active_tab_mut().status = if share_after {
            format!("Preparing {name} for Quick Share…")
        } else {
            format!("Opening {name} from archive…")
        };
    }

    pub fn apply_archive_event(&mut self, event: crate::archive::Event) {
        match event {
            crate::archive::Event::Listed {
                generation,
                location,
                result,
            } => {
                if generation != self.archive_generation
                    || self.active_tab().current_dir != crate::archive::virtual_path(&location)
                {
                    return;
                }
                match result {
                    Ok(entries) => {
                        let converted = entries
                            .into_iter()
                            .map(|entry| {
                                let child = crate::archive::child_location(&location, &entry.name);
                                FileEntry {
                                    path: crate::archive::virtual_path(&child),
                                    name: entry.name,
                                    kind: if entry.directory {
                                        EntryKind::Directory
                                    } else {
                                        EntryKind::File
                                    },
                                    size: entry.size,
                                    modified_sort_key: entry.modified_sort_key,
                                    remote: None,
                                    remote_modified: None,
                                }
                            })
                            .collect::<Vec<_>>();
                        let count = converted.len();
                        let tab = self.active_tab_mut();
                        tab.restore_validation_pending = false;
                        tab.entries = converted;
                        tab.apply_sort();
                        tab.status = tab
                            .restore_warning
                            .clone()
                            .map(|warning| format!("{warning} · {count} items · ZIP archive"))
                            .unwrap_or_else(|| format!("{count} items · ZIP archive"));
                        if let Some(selected) = tab.selected_path.as_ref()
                            && !tab.entries.iter().any(|entry| &entry.path == selected)
                        {
                            tab.selected_path = None;
                        }
                    }
                    Err(error) => {
                        if self.active_tab().restore_validation_pending {
                            let original = crate::archive::virtual_path(&location);
                            self.active_tab_mut()
                                .fallback_restored_location(&original, &error);
                            self.request_directory_reload();
                        } else {
                            self.active_tab_mut().entries.clear();
                            self.active_tab_mut().status = format!("Cannot read archive: {error}");
                        }
                    }
                }
            }
            crate::archive::Event::Opened {
                location,
                destination,
                share_after,
                result,
            } => {
                let name = Path::new(&location.inner_path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| location.inner_path.clone());
                let action = result.and_then(|()| {
                    if share_after {
                        self.share_path(&destination)
                    } else {
                        self.open_remote_cache_path(&destination)
                    }
                });
                self.active_tab_mut().status = match action {
                    Ok(()) if share_after => format!("Sharing from archive: {name}"),
                    Ok(()) => format!(
                        "Opened from archive: {name} · edits will be saved back automatically"
                    ),
                    Err(error) if share_after => format!("Cannot share archive file: {error}"),
                    Err(error) => format!("Cannot open archive file: {error}"),
                };
            }
            crate::archive::Event::Mutated {
                current,
                action,
                result,
            } => {
                #[cfg(target_os = "android")]
                if result.is_ok() {
                    self.notify_android_file_changes(&[current.archive_path.clone()]);
                }
                if self.active_tab().current_dir != crate::archive::virtual_path(&current) {
                    return;
                }
                match result {
                    Ok(()) => {
                        self.active_tab_mut().status = action;
                        self.load_archive_location(current);
                    }
                    Err(error) => {
                        self.active_tab_mut().status = format!("Archive update failed: {error}")
                    }
                }
            }
            crate::archive::Event::EditSynced { location, result } => {
                #[cfg(target_os = "android")]
                if result.is_ok() {
                    self.notify_android_file_changes(&[location.archive_path.clone()]);
                }
                if result.is_ok() {
                    if self.active_tab().current_dir
                        == crate::archive::virtual_path(&crate::archive::ArchiveLocation {
                            archive_path: location.archive_path.clone(),
                            inner_path: String::new(),
                        })
                        || crate::archive::parse_virtual_path(&self.active_tab().current_dir)
                            .is_some_and(|current| current.archive_path == location.archive_path)
                    {
                        self.active_tab_mut().status =
                            format!("Saved edits back into {}", location.archive_path.display());
                    }
                } else if let Err(error) = result {
                    self.active_tab_mut().status = format!("Cannot save archive edit: {error}");
                }
            }
            crate::archive::Event::Exported {
                target_location,
                destination,
                target_name,
                size,
                result,
            } => {
                #[cfg(target_os = "android")]
                if result.is_ok() && parse_taildrive_path(&target_location).is_none() {
                    self.notify_android_file_changes(&[destination.clone()]);
                }
                if self.active_tab().current_dir != target_location {
                    return;
                }
                match result {
                    Ok(()) => {
                        if let Some(TaildriveLocation::Remote {
                            profile_id,
                            device_id,
                            share,
                            remote_path,
                        }) = parse_taildrive_path(&target_location)
                        {
                            let transfer_id = next_transfer_id();
                            let command = crate::tailscale::Command::TaildriveUpload {
                                profile_id,
                                device_id,
                                share,
                                path: remote_child_path(&remote_path, &target_name),
                                source: destination.clone(),
                                source_location: target_location.clone(),
                                transfer_id: transfer_id.clone(),
                                source_was_cut: false,
                                replace: false,
                            };
                            if let Err(error) = self.dispatch_taildrive_transfer(command) {
                                self.active_tab_mut().status = error;
                                return;
                            }
                            self.pending_upload_info
                                .insert(transfer_id.clone(), (EntryKind::File, size));
                            self.pending_temporary_uploads.insert(destination);
                            self.begin_file_transfer(
                                transfer_id,
                                target_name.clone(),
                                "Uploading from archive",
                            );
                            self.transfer_popup_open = true;
                            self.active_tab_mut().status =
                                format!("Uploading {target_name} from archive…");
                        } else {
                            self.active_tab_mut().selected_path = Some(destination);
                            self.active_tab_mut().status =
                                format!("Exported from archive: {target_name}");
                            self.request_directory_reload();
                        }
                    }
                    Err(error) => {
                        self.active_tab_mut().status = format!("Archive export failed: {error}");
                    }
                }
            }
        }
    }

    pub fn set_thumbnail_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<ThumbnailRequest>,
    ) {
        self.thumbnail_sender = Some(sender);
    }

    pub fn thumbnail_for_entry(&mut self, entry: &FileEntry) -> Option<ImageData> {
        if entry.kind != EntryKind::File
            || entry.remote.is_some()
            || crate::archive::parse_virtual_path(&entry.path).is_some()
            || entry.category() != FileCategory::Image
        {
            return None;
        }
        if self.thumbnail_cache.contains_key(&entry.path) {
            return self.thumbnail_cache.get(&entry.path).cloned().flatten();
        }
        if self.thumbnail_pending.contains(&entry.path) {
            return None;
        }
        let sender = self.thumbnail_sender.clone()?;
        self.thumbnail_pending.insert(entry.path.clone());
        if sender.send(entry.path.clone()).is_err() {
            self.thumbnail_pending.remove(&entry.path);
        }
        None
    }

    pub fn apply_thumbnail_result(&mut self, path: PathBuf, result: Result<ImageData, String>) {
        self.thumbnail_pending.remove(&path);
        self.thumbnail_cache.insert(path, result.ok());
        while self.thumbnail_cache.len() > 512 {
            let Some(oldest) = self.thumbnail_cache.keys().next().cloned() else {
                break;
            };
            self.thumbnail_cache.remove(&oldest);
        }
    }

    pub fn set_directory_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<DirectoryRequest>,
    ) {
        self.directory_sender = Some(sender);
        if parse_taildrive_path(&self.active_tab().current_dir).is_none()
            && crate::archive::parse_virtual_path(&self.active_tab().current_dir).is_none()
            && !self.active_tab().search_active
        {
            self.request_directory_reload();
        }
    }

    pub fn request_directory_reload(&mut self) {
        if self.active_tab().search_active
            || parse_taildrive_path(&self.active_tab().current_dir).is_some()
            || crate::archive::parse_virtual_path(&self.active_tab().current_dir).is_some()
        {
            return;
        }
        let dir = self.active_tab().current_dir.clone();
        let show_hidden = self.active_tab().show_hidden;
        self.thumbnail_cache
            .retain(|path, _| path.parent() != Some(dir.as_path()));
        self.directory_generation = self.directory_generation.wrapping_add(1);
        let generation = self.directory_generation;
        self.active_tab_mut().status = "Loading folder…".to_owned();
        #[cfg(test)]
        if self.directory_sender.is_none() {
            let result = scan_directory(&dir, show_hidden);
            self.apply_directory_result(generation, dir, result);
            return;
        }
        if let Some(sender) = &self.directory_sender {
            if sender.send((generation, dir, show_hidden)).is_err() {
                self.directory_request_started_at = None;
                self.active_tab_mut().status = "Folder worker stopped".to_owned();
            } else {
                self.directory_request_started_at = Some(Instant::now());
            }
        } else {
            self.directory_request_started_at = None;
            self.active_tab_mut().status = "Folder worker is starting…".to_owned();
        }
    }

    pub fn accepts_directory_result(&self, generation: u64, dir: &Path) -> bool {
        self.directory_generation == generation
            && self.active_tab().current_dir == dir
            && !self.active_tab().search_active
            && parse_taildrive_path(dir).is_none()
            && crate::archive::parse_virtual_path(dir).is_none()
    }

    pub fn apply_directory_result(
        &mut self,
        generation: u64,
        dir: PathBuf,
        result: Result<Vec<FileEntry>, String>,
    ) {
        if !self.accepts_directory_result(generation, &dir) {
            return;
        }
        self.directory_request_started_at = None;
        match result {
            Ok(entries) => {
                let tab = self.active_tab_mut();
                tab.restore_validation_pending = false;
                let count = entries.len();
                tab.entries = entries;
                tab.apply_sort();
                tab.status = tab
                    .restore_warning
                    .clone()
                    .map(|warning| format!("{warning} · {count} items"))
                    .unwrap_or_else(|| format!("{count} items"));
                if let Some(selected) = tab.selected_path.as_ref()
                    && !tab.entries.iter().any(|entry| &entry.path == selected)
                {
                    tab.selected_path = None;
                }
            }
            Err(error) => {
                if self.active_tab().restore_validation_pending {
                    let original = dir.clone();
                    self.active_tab_mut()
                        .fallback_restored_location(&original, &error);
                    self.request_directory_reload();
                } else {
                    let tab = self.active_tab_mut();
                    tab.entries.clear();
                    tab.status = format!("Cannot read directory: {error}");
                }
            }
        }
    }

    pub fn set_search_sender(&mut self, sender: tokio::sync::mpsc::UnboundedSender<SearchRequest>) {
        self.search_sender = Some(sender);
        if !self.active_tab().search_input.trim().is_empty() {
            self.run_active_search();
        }
    }

    fn run_active_search(&mut self) {
        if self.is_archive_current() {
            self.active_tab_mut().search_active = false;
            self.active_tab_mut().status = "Archive search is not available yet".to_owned();
            return;
        }
        if self.is_taildrive_current() {
            self.active_tab_mut().search_active = false;
            self.active_tab_mut().status = "TailDrive search is not available yet".to_owned();
            return;
        }
        let query = self.active_tab().search_input.clone();
        if query.trim().is_empty() {
            self.clear_search();
            return;
        }
        self.active_tab_mut().search_active = true;
        self.active_tab_mut().status = "Searching…".to_owned();
        self.active_tab_mut().entries.clear();

        let dir = self.active_tab().current_dir.clone();
        let mode = self.search_mode;
        let show_hidden = self.active_tab().show_hidden;
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;

        if let Some(sender) = &self.search_sender {
            if sender
                .send((generation, dir, query, mode, show_hidden))
                .is_err()
            {
                self.active_tab_mut().status = "Search worker stopped".to_owned();
            }
        } else {
            self.active_tab_mut().status = "Search worker is starting…".to_owned();
        }
    }

    pub fn accepts_search_result(&self, generation: u64, dir: &Path, query: &str) -> bool {
        self.search_generation == generation
            && self.active_tab().current_dir == dir
            && self.active_tab().search_input.trim() == query.trim()
            && self.active_tab().search_active
    }

    pub fn clear_search(&mut self) {
        if let Some(location) = crate::archive::parse_virtual_path(&self.active_tab().current_dir) {
            self.active_tab_mut().search_input.clear();
            self.active_tab_mut().search_active = false;
            self.load_archive_location(location);
        } else if let Some(location) = parse_taildrive_path(&self.active_tab().current_dir) {
            self.active_tab_mut().search_input.clear();
            self.active_tab_mut().search_active = false;
            self.load_taildrive_location(location);
        } else {
            let tab = self.active_tab_mut();
            tab.search_input.clear();
            tab.search_active = false;
            self.request_directory_reload();
        }
    }

    pub fn submit_address(&mut self, value: String) {
        let raw = value.trim().replace('\\', "/");
        if let Some(location) = self.parse_taildrive_address(&raw) {
            self.navigate_to(taildrive_path(&location));
            return;
        }
        if let Some(location) = crate::archive::parse_display_path(&raw) {
            self.navigate_to(crate::archive::virtual_path(&location));
            return;
        }
        let base = if self.is_taildrive_current() {
            home_dir().unwrap_or_else(default_directory)
        } else {
            self.active_tab().current_dir.clone()
        };
        let path = if raw == "~" {
            home_dir().unwrap_or(base)
        } else if let Some(relative) = raw.strip_prefix("~/") {
            home_dir().map_or_else(|| base.join(relative), |home| home.join(relative))
        } else {
            let candidate = PathBuf::from(&raw);
            if candidate.is_absolute() {
                candidate
            } else {
                base.join(candidate)
            }
        };
        self.navigate_to(path);
    }

    fn set_location(&mut self, path: PathBuf) {
        let archive_location = crate::archive::parse_virtual_path(&path);
        let taildrive_location = parse_taildrive_path(&path);
        self.active_tab_mut().set_current_dir(path);
        self.refresh_active_address_input();
        if let Some(location) = archive_location {
            self.load_archive_location(location);
        } else if let Some(location) = taildrive_location {
            self.load_taildrive_location(location);
        } else {
            self.request_directory_reload();
        }
        self.persist_mobile_browsing_state();
    }

    pub fn navigate_to(&mut self, path: PathBuf) {
        // Local path validation is intentionally deferred to the directory worker.
        // This keeps address-bar navigation and session restore off the UI thread even
        // for slow removable/network-backed filesystems.
        if path == self.active_tab().current_dir {
            if let Some(location) = crate::archive::parse_virtual_path(&path) {
                self.load_archive_location(location);
            } else if let Some(location) = parse_taildrive_path(&path) {
                self.load_taildrive_location(location);
            } else {
                self.request_directory_reload();
            }
            return;
        }
        let current = self.active_tab().current_dir.clone();
        self.active_tab_mut().back_stack.push(current);
        self.active_tab_mut().forward_stack.clear();
        self.set_location(path);
    }

    pub fn go_back(&mut self) {
        let Some(path) = self.active_tab_mut().back_stack.pop() else {
            return;
        };
        let current = self.active_tab().current_dir.clone();
        self.active_tab_mut().forward_stack.push(current);
        self.set_location(path);
    }

    pub fn go_forward(&mut self) {
        let Some(path) = self.active_tab_mut().forward_stack.pop() else {
            return;
        };
        let current = self.active_tab().current_dir.clone();
        self.active_tab_mut().back_stack.push(current);
        self.set_location(path);
    }

    pub fn go_up(&mut self) {
        if let Some(location) = crate::archive::parse_virtual_path(&self.active_tab().current_dir) {
            if let Some(parent) = crate::archive::parent_path(&location) {
                self.navigate_to(parent);
            }
        } else if let Some(location) = parse_taildrive_path(&self.active_tab().current_dir) {
            if let Some(parent) = taildrive_parent(&location) {
                self.navigate_to(taildrive_path(&parent));
            }
        } else if let Some(parent) = self.active_tab().current_dir.parent() {
            self.navigate_to(parent.to_path_buf());
        }
    }

    pub fn go_home(&mut self) {
        if let Some(home) = home_dir() {
            self.navigate_to(home);
        }
    }

    #[cfg(target_os = "android")]
    pub fn open_taildrive_root(&mut self) {
        self.navigate_to(taildrive_path(&TaildriveLocation::Root));
    }

    pub fn refresh(&mut self) {
        if self.rename_active() {
            self.cancel_rename();
        }
        if let Some(location) = crate::archive::parse_virtual_path(&self.active_tab().current_dir) {
            self.load_archive_location(location);
        } else if let Some(location) = parse_taildrive_path(&self.active_tab().current_dir) {
            self.load_taildrive_location(location);
        } else if self.active_tab().search_active {
            self.run_active_search();
        } else {
            self.request_directory_reload();
        }
    }

    pub fn toggle_hidden(&mut self) {
        let search_active = self.active_tab().search_active;
        self.active_tab_mut().show_hidden = !self.active_tab().show_hidden;
        if let Some(location) = crate::archive::parse_virtual_path(&self.active_tab().current_dir) {
            self.load_archive_location(location);
        } else if let Some(location) = parse_taildrive_path(&self.active_tab().current_dir) {
            self.load_taildrive_location(location);
        } else if search_active {
            self.run_active_search();
        } else {
            self.request_directory_reload();
        }
        self.persist_mobile_browsing_state();
    }

    pub fn select_entry(&mut self, path: PathBuf) {
        if self.rename_active() {
            self.cancel_rename();
        }
        self.context_actions_visible = false;
        self.active_tab_mut().select_entry(path);
    }

    pub fn context_click_entry(&mut self, path: PathBuf) {
        if self.rename_active() {
            self.cancel_rename();
        }
        self.active_tab_mut().select_entry(path);
        self.context_actions_visible = true;
    }

    #[cfg(target_os = "windows")]
    pub fn explorer_replacement_enabled(&self) -> bool {
        self.explorer_replacement_enabled
    }

    #[cfg(target_os = "windows")]
    pub fn enable_explorer_replacement(&mut self) {
        match crate::windows_integration::enable() {
            Ok(()) => {
                self.explorer_replacement_enabled = true;
                self.active_tab_mut().status =
                    "FastExplorer is now the default file manager for folders and Win+E".to_owned();
            }
            Err(error) => {
                self.active_tab_mut().status = format!("Cannot replace File Explorer: {error}");
            }
        }
    }

    #[cfg(target_os = "windows")]
    pub fn disable_explorer_replacement(&mut self) {
        match crate::windows_integration::disable() {
            Ok(()) => {
                self.explorer_replacement_enabled = false;
                self.active_tab_mut().status =
                    "Restored the previous File Explorer registration".to_owned();
            }
            Err(error) => {
                self.active_tab_mut().status = format!("Cannot restore File Explorer: {error}");
            }
        }
    }

    pub fn pinned_paths(&self) -> &[PathBuf] {
        &self.pinned_paths
    }

    pub fn selected_is_pinned(&self) -> bool {
        let Some(path) = self.active_tab().selected_path.as_ref() else {
            return false;
        };
        self.pinned_paths.iter().any(|pinned| pinned == path)
    }

    pub fn toggle_pin_selected(&mut self) {
        let Some(path) = self.active_tab().selected_path.clone() else {
            return;
        };
        if let Some(index) = self.pinned_paths.iter().position(|pinned| pinned == &path) {
            self.pinned_paths.remove(index);
            let display = self.display_path_for(&path);
            self.active_tab_mut().status = format!("Unpinned: {display}");
        } else {
            self.pinned_paths.push(path.clone());
            let display = self.display_path_for(&path);
            self.active_tab_mut().status = format!("Pinned: {display}");
        }
        self.persist_settings();
    }

    pub fn context_actions_visible(&self) -> bool {
        self.context_actions_visible
    }

    pub fn close_context_actions(&mut self) {
        self.context_actions_visible = false;
    }

    pub fn click_entry(&mut self, path: PathBuf) {
        if self.rename_active() {
            self.cancel_rename();
        }
        let now = Instant::now();
        let was_selected = self.active_tab().selected_path.as_ref() == Some(&path);
        let elapsed = self
            .active_tab()
            .last_click
            .as_ref()
            .and_then(|(last_path, last_time)| {
                (last_path == &path).then(|| now.duration_since(*last_time))
            });
        if elapsed.is_some_and(|elapsed| elapsed <= Duration::from_millis(500)) {
            self.active_tab_mut().last_click = None;
            self.activate_entry(path);
            return;
        }
        if was_selected && elapsed.is_some_and(|elapsed| elapsed > Duration::from_millis(500)) {
            self.active_tab_mut().last_click = None;
            self.begin_rename();
            return;
        }
        self.context_actions_visible = false;
        self.active_tab_mut().select_entry(path.clone());
        self.active_tab_mut().last_click = Some((path, now));
    }

    pub fn activate_entry(&mut self, path: PathBuf) {
        if self.rename_active() {
            self.cancel_rename();
        }
        let entry = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .cloned();
        if let Some(location) = crate::archive::parse_virtual_path(&path) {
            if entry
                .as_ref()
                .is_some_and(|entry| entry.kind == EntryKind::Directory)
            {
                self.navigate_to(path);
            } else {
                self.open_archive_member(location, false);
            }
            return;
        }
        if entry.as_ref().is_some_and(|entry| {
            entry.remote.is_none()
                && entry.kind == EntryKind::File
                && crate::archive::is_supported_archive(&entry.path)
        }) {
            let location = crate::archive::ArchiveLocation {
                archive_path: path.clone(),
                inner_path: String::new(),
            };
            self.navigate_to(crate::archive::virtual_path(&location));
            return;
        }
        if let Some(entry) = entry.as_ref().cloned()
            && entry.remote.is_some()
        {
            if entry.kind == EntryKind::Directory {
                self.navigate_to(entry.path);
            } else {
                self.prepare_remote_entry(entry, RemotePreparePurpose::Open);
            }
            return;
        }
        if entry
            .as_ref()
            .is_some_and(|entry| entry.kind == EntryKind::Directory)
        {
            self.navigate_to(path);
            return;
        }
        let name = entry
            .as_ref()
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| self.display_path_for(&path));
        let result = self.open_remote_cache_path(&path);
        self.active_tab_mut().status = match result {
            Ok(()) if is_aab_name(&name) => format!("Preparing installer: {name}"),
            Ok(()) if is_android_install_name(&name) => format!("Installer opened: {name}"),
            Ok(()) => format!("Opened: {name}"),
            Err(error) => format!("Cannot open {name}: {error}"),
        };
    }

    pub fn share_selected(&mut self) {
        let Some(path) = self.active_tab().selected_path.clone() else {
            return;
        };
        let Some(entry) = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .cloned()
        else {
            return;
        };
        if entry.kind == EntryKind::Directory {
            self.active_tab_mut().status = "Quick Share currently supports files".to_owned();
            return;
        }
        if let Some(location) = crate::archive::parse_virtual_path(&entry.path) {
            self.open_archive_member(location, true);
            return;
        }
        if matches!(entry.remote, Some(TaildriveLocation::Remote { .. })) {
            self.prepare_remote_entry(entry, RemotePreparePurpose::Share);
            return;
        }
        let result = self.share_path(&path);
        self.active_tab_mut().status = match result {
            Ok(()) => format!("Sharing: {}", entry.name),
            Err(error) => format!("Cannot share {}: {error}", entry.name),
        };
    }

    fn share_path(&self, path: &Path) -> Result<(), String> {
        share_path_with_system(path, self)
    }

    pub fn touch_entry(&mut self, path: PathBuf) {
        let is_directory = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .is_some_and(|entry| entry.kind == EntryKind::Directory);
        if is_directory {
            self.activate_entry(path);
            return;
        }

        let now = Instant::now();
        let is_double_tap =
            self.active_tab()
                .last_click
                .as_ref()
                .is_some_and(|(last_path, last_time)| {
                    last_path == &path
                        && now.duration_since(*last_time) <= Duration::from_millis(500)
                });
        if is_double_tap {
            self.active_tab_mut().last_click = None;
            self.activate_entry(path);
        } else {
            self.select_entry(path.clone());
            self.active_tab_mut().last_click = Some((path, now));
        }
    }

    pub fn selected_entry_index(&self) -> Option<usize> {
        let selected = self.active_tab().selected_path.as_ref()?;
        self.active_tab()
            .entries
            .iter()
            .position(|entry| &entry.path == selected)
    }

    pub fn move_selection(&mut self, delta: isize) -> Option<usize> {
        let entry_count = self.active_tab().entries.len();
        if entry_count == 0 {
            return None;
        }
        let target = self.selected_entry_index().map_or(0, |current| {
            (current as isize + delta).clamp(0, entry_count as isize - 1) as usize
        });
        let path = self.active_tab().entries[target].path.clone();
        self.active_tab_mut().select_entry(path);
        Some(target)
    }

    pub fn select_first_entry(&mut self) -> Option<usize> {
        self.select_entry_at(0)
    }

    pub fn select_last_entry(&mut self) -> Option<usize> {
        let target = self.active_tab().entries.len().checked_sub(1)?;
        self.select_entry_at(target)
    }

    fn select_entry_at(&mut self, index: usize) -> Option<usize> {
        let path = self.active_tab().entries.get(index)?.path.clone();
        self.active_tab_mut().select_entry(path);
        Some(index)
    }

    pub fn typeahead_select(&mut self, text: String) -> Option<usize> {
        if self.rename_active() || text.is_empty() || self.active_tab().entries.is_empty() {
            return self.selected_entry_index();
        }
        let incoming = text.to_lowercase();
        if incoming.chars().any(char::is_control) {
            return self.selected_entry_index();
        }
        let now = Instant::now();
        let continuation = self
            .active_tab()
            .last_typeahead
            .is_some_and(|last| now.duration_since(last) <= Duration::from_secs(1));
        let prefix = if continuation {
            format!("{}{}", self.active_tab().typeahead_buffer, incoming)
        } else {
            incoming.clone()
        };
        self.active_tab_mut().typeahead_buffer = prefix.clone();
        self.active_tab_mut().last_typeahead = Some(now);
        let count = self.active_tab().entries.len();
        let start = self.selected_entry_index().map_or(0, |index| index);
        if let Some(index) = (0..count)
            .map(|offset| (start + offset) % count)
            .find(|&index| {
                self.active_tab().entries[index]
                    .name
                    .to_lowercase()
                    .starts_with(&prefix)
            })
        {
            return self.select_entry_at(index);
        }
        if prefix.chars().count() > incoming.chars().count() {
            self.active_tab_mut().typeahead_buffer = incoming.clone();
            let start = self
                .selected_entry_index()
                .map_or(0, |index| (index + 1) % count);
            if let Some(index) = (0..count)
                .map(|offset| (start + offset) % count)
                .find(|&index| {
                    self.active_tab().entries[index]
                        .name
                        .to_lowercase()
                        .starts_with(&incoming)
                })
            {
                return self.select_entry_at(index);
            }
        }
        self.selected_entry_index()
    }

    pub fn activate_selected(&mut self) {
        if let Some(path) = self.active_tab().selected_path.clone() {
            self.activate_entry(path);
        }
    }

    pub fn has_selection(&self) -> bool {
        self.active_tab().selected_path.is_some()
    }

    pub fn can_clipboard_selected(&self) -> bool {
        let Some(selected) = self.active_tab().selected_path.as_ref() else {
            return false;
        };
        self.active_tab()
            .entries
            .iter()
            .find(|entry| &entry.path == selected)
            .is_some_and(|entry| {
                entry.remote.is_none()
                    || matches!(entry.remote, Some(TaildriveLocation::Remote { .. }))
            })
    }

    pub fn can_paste(&self) -> bool {
        self.can_mutate_current_location() && self.file_clipboard.is_some()
    }

    pub fn sort_field(&self) -> SortField {
        self.active_tab().sort_field
    }

    pub fn sort_direction(&self) -> SortDirection {
        self.active_tab().sort_direction
    }

    pub fn sort_popup_open(&self) -> bool {
        self.sort_popup_open
    }

    pub fn open_sort_popup(&mut self) {
        self.close_tab_overlays();
        self.file_more_popup_open = false;
        self.sort_popup_open = true;
    }

    pub fn close_sort_popup(&mut self) {
        self.sort_popup_open = false;
    }

    pub fn file_more_popup_open(&self) -> bool {
        self.file_more_popup_open
    }

    #[cfg(target_os = "android")]
    pub fn mobile_overlay_width(&self, preferred: f64) -> f64 {
        let available = (self.android_window_width_dp
            - self.android_insets.left
            - self.android_insets.right
            - 16.0)
            .max(1.0);
        preferred.min(available)
    }

    #[cfg(target_os = "android")]
    pub fn mobile_transfer_compact(&self) -> bool {
        self.mobile_overlay_width(420.0) < 300.0
    }

    #[cfg(target_os = "android")]
    pub fn mobile_primary_action_capacity(&self) -> usize {
        mobile_primary_action_capacity_for_width(
            self.android_window_width_dp,
            self.android_insets.left,
            self.android_insets.right,
        )
    }

    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub fn toggle_file_more_popup(&mut self) {
        self.close_tab_overlays();
        self.file_more_popup_open = !self.file_more_popup_open;
        if self.file_more_popup_open {
            self.sort_popup_open = false;
        }
    }

    pub fn close_file_more_popup(&mut self) {
        self.file_more_popup_open = false;
    }

    pub fn set_sort_field(&mut self, field: SortField) {
        self.active_tab_mut().sort_field = field;
        self.active_tab_mut().apply_sort();
        self.sort_popup_open = false;
        self.persist_session();
    }

    pub fn set_sort_direction(&mut self, direction: SortDirection) {
        self.active_tab_mut().sort_direction = direction;
        self.active_tab_mut().apply_sort();
        self.sort_popup_open = false;
        self.persist_session();
    }

    pub fn activate_sort_field(&mut self, field: SortField) {
        let tab = self.active_tab_mut();
        if tab.sort_field == field {
            tab.sort_direction = match tab.sort_direction {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
        } else {
            tab.sort_field = field;
            tab.sort_direction = match field {
                SortField::DateModified | SortField::Size => SortDirection::Descending,
                SortField::Name | SortField::Type => SortDirection::Ascending,
            };
        }
        tab.apply_sort();
        self.sort_popup_open = false;
        self.persist_session();
    }

    pub fn file_transfers(&self) -> &[FileTransferProgress] {
        &self.file_transfers
    }

    pub fn transfer_popup_open(&self) -> bool {
        self.transfer_popup_open
    }

    pub fn toggle_transfer_popup(&mut self) {
        self.close_tab_overlays();
        self.transfer_popup_open = !self.transfer_popup_open;
    }

    pub fn close_transfer_popup(&mut self) {
        self.transfer_popup_open = false;
    }

    #[cfg(target_os = "android")]
    pub fn pause_transfer(&mut self, transfer_id: &str) {
        #[cfg(target_os = "android")]
        if let Err(error) = crate::android_transfer::pause(transfer_id) {
            self.active_tab_mut().status = format!("Cannot pause transfer: {error}");
        }
    }

    #[cfg(target_os = "android")]
    pub fn resume_transfer(&mut self, transfer_id: &str) {
        #[cfg(target_os = "android")]
        if let Err(error) = crate::android_transfer::resume(transfer_id) {
            self.active_tab_mut().status = format!("Cannot resume transfer: {error}");
        }
    }

    #[cfg(target_os = "android")]
    pub fn cancel_transfer(&mut self, transfer_id: &str) {
        #[cfg(target_os = "android")]
        if let Err(error) = crate::android_transfer::cancel(transfer_id) {
            self.active_tab_mut().status = format!("Cannot stop transfer: {error}");
        }
    }

    #[cfg(target_os = "android")]
    pub fn retry_transfer(&mut self, transfer_id: &str) {
        #[cfg(target_os = "android")]
        match crate::android_transfer::retry(transfer_id) {
            Ok(()) => self.ensure_android_transfer_service(),
            Err(error) => self.active_tab_mut().status = format!("Cannot retry transfer: {error}"),
        }
    }

    pub fn clear_finished_transfers(&mut self) {
        self.file_transfers.retain(|transfer| !transfer.done);
    }

    pub fn oldest_transfer_for_icon(&self) -> Option<&FileTransferProgress> {
        self.file_transfers
            .iter()
            .find(|transfer| !transfer.done)
            .or_else(|| self.file_transfers.last())
    }

    fn begin_file_transfer(&mut self, transfer_id: String, label: String, phase: &str) {
        if self.file_transfers.len() >= TRANSFER_HISTORY_LIMIT {
            if let Some(index) = self
                .file_transfers
                .iter()
                .position(|transfer| transfer.done)
            {
                self.file_transfers.remove(index);
            } else {
                self.file_transfers.remove(0);
            }
        }
        let now = Instant::now();
        self.file_transfers.push(FileTransferProgress {
            transfer_id,
            label,
            phase: phase.to_owned(),
            bytes_done: 0,
            bytes_total: 0,
            items_done: 0,
            items_total: 0,
            paused: false,
            cancelling: false,
            cancelled: false,
            done: false,
            error: None,
            started_at: now,
            last_sample_at: now,
            last_sample_bytes: 0,
            bytes_per_second: 0.0,
        });
        #[cfg(target_os = "android")]
        self.ensure_android_transfer_service();
    }

    fn finish_file_transfer(&mut self, transfer_id: &str, error: Option<String>) {
        let Some(transfer) = self
            .file_transfers
            .iter_mut()
            .find(|transfer| transfer.transfer_id == transfer_id)
        else {
            return;
        };
        let cancelled = error
            .as_deref()
            .is_some_and(|message| message.eq_ignore_ascii_case("transfer cancelled"));
        transfer.done = true;
        transfer.paused = false;
        transfer.cancelling = false;
        transfer.cancelled = cancelled;
        transfer.error = if cancelled { None } else { error };
        if cancelled {
            transfer.phase = "Cancelled".to_owned();
        } else if transfer.error.is_none() {
            transfer.phase = "Completed".to_owned();
            if transfer.bytes_total > 0 {
                transfer.bytes_done = transfer.bytes_total;
            }
            if transfer.items_total > 0 {
                transfer.items_done = transfer.items_total;
            }
        } else {
            transfer.phase = "Failed".to_owned();
        }
    }

    pub fn rename_active(&self) -> bool {
        self.active_tab().rename_input.is_some()
    }

    pub fn new_folder(&mut self) {
        if let Some(current) = crate::archive::parse_virtual_path(&self.active_tab().current_dir) {
            let name = unique_entry_name(&self.active_tab().entries, "New folder");
            let Some(sender) = self.archive_sender.as_ref() else {
                self.active_tab_mut().status = "Archive worker is starting…".to_owned();
                return;
            };
            if sender
                .send(crate::archive::Command::Mkdir {
                    current,
                    name: name.clone(),
                })
                .is_err()
            {
                self.active_tab_mut().status = "Archive worker stopped".to_owned();
            } else {
                self.active_tab_mut().status = format!("Creating {name} in archive…");
            }
            return;
        }
        if let Some(TaildriveLocation::Remote {
            profile_id,
            device_id,
            share,
            remote_path,
        }) = parse_taildrive_path(&self.active_tab().current_dir)
        {
            if self.rename_active() {
                self.cancel_rename();
            }
            let name = unique_entry_name(&self.active_tab().entries, "New folder");
            let remote = TaildriveLocation::Remote {
                profile_id,
                device_id,
                share,
                remote_path: remote_child_path(&remote_path, &name),
            };
            let path = taildrive_path(&remote);
            self.active_tab_mut().entries.push(FileEntry {
                path: path.clone(),
                name: name.clone(),
                kind: EntryKind::Directory,
                size: 0,
                modified_sort_key: 0,
                remote: Some(remote),
                remote_modified: None,
            });
            self.active_tab_mut().apply_sort();
            self.active_tab_mut().select_entry(path.clone());
            self.active_tab_mut().rename_input = Some(name);
            self.active_tab_mut().rename_replace_on_type = true;
            self.active_tab_mut().rename_keyboard_suffix = Some(String::new());
            self.active_tab_mut().pending_remote_folder = Some(path);
            self.active_tab_mut().status = "Enter a name for the new TailDrive folder".to_owned();
            return;
        }
        if !self.can_mutate_current_location() {
            return;
        }
        if self.active_tab().search_active {
            self.active_tab_mut().clear_search();
        }
        let dir = self.active_tab().current_dir.clone();
        let name = unique_entry_name(&self.active_tab().entries, "New folder");
        let path = dir.join(&name);
        self.submit_local_file_command(
            LocalFileCommand::CreateDir { current: dir, path },
            format!("Creating {name}…"),
        );
    }

    pub fn copy_selected(&mut self) {
        self.set_clipboard_from_selection(ClipboardMode::Copy);
    }

    pub fn cut_selected(&mut self) {
        self.set_clipboard_from_selection(ClipboardMode::Cut);
    }

    fn set_clipboard_from_selection(&mut self, mode: ClipboardMode) {
        let Some(path) = self.active_tab().selected_path.clone() else {
            return;
        };
        let Some(entry) = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .cloned()
        else {
            return;
        };
        if entry.remote.is_some() && !matches!(entry.remote, Some(TaildriveLocation::Remote { .. }))
        {
            return;
        }
        if mode == ClipboardMode::Cut && !self.can_mutate_current_location() {
            return;
        }
        let name = entry.name.clone();
        self.file_clipboard = Some(FileClipboard {
            path,
            name: entry.name,
            kind: entry.kind,
            size: entry.size,
            remote_modified: entry.remote_modified,
            mode,
        });
        self.active_tab_mut().status = match mode {
            ClipboardMode::Copy => format!("Copied: {name}"),
            ClipboardMode::Cut => format!("Cut: {name}"),
        };
    }

    fn paste_target_name(
        &mut self,
        clipboard: &FileClipboard,
        target_location: &Path,
        resolution: Option<PasteConflictResolution>,
    ) -> Option<(String, bool)> {
        let same_folder = clipboard.path.parent() == Some(target_location);
        let exact_exists = self
            .active_tab()
            .entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(&clipboard.name));
        if !exact_exists {
            return Some((clipboard.name.clone(), false));
        }
        if same_folder && clipboard.mode == ClipboardMode::Copy {
            return Some((
                unique_entry_copy_name(&self.active_tab().entries, &clipboard.name),
                false,
            ));
        }
        match resolution {
            Some(PasteConflictResolution::Replace) => Some((clipboard.name.clone(), true)),
            Some(PasteConflictResolution::KeepBoth) => Some((
                unique_entry_copy_name(&self.active_tab().entries, &clipboard.name),
                false,
            )),
            None => {
                self.pending_paste_conflict = Some(PendingPasteConflict {
                    clipboard: clipboard.clone(),
                    target_location: target_location.to_path_buf(),
                });
                self.active_tab_mut().status = format!(
                    "{} already exists here. Choose Replace, Keep both, or Skip.",
                    clipboard.name
                );
                None
            }
        }
    }

    pub fn paste(&mut self) {
        if !self.can_mutate_current_location() {
            return;
        }
        let Some(clipboard) = self.file_clipboard.clone() else {
            return;
        };
        let conflict_resolution = self.paste_conflict_resolution.take();
        let target_location = self.active_tab().current_dir.clone();
        if clipboard.mode == ClipboardMode::Cut
            && clipboard.path.parent() == Some(target_location.as_path())
        {
            self.active_tab_mut().status = "Already in this folder".to_owned();
            return;
        }
        if let Some(current) = crate::archive::parse_virtual_path(&target_location) {
            if clipboard.kind != EntryKind::File {
                self.active_tab_mut().status =
                    "Only files can be pasted into ZIP archives".to_owned();
                return;
            }
            let Some((name, replace)) =
                self.paste_target_name(&clipboard, &target_location, conflict_resolution)
            else {
                return;
            };
            if let Some(source @ TaildriveLocation::Remote { .. }) =
                parse_taildrive_path(&clipboard.path)
            {
                let entry = FileEntry {
                    path: clipboard.path.clone(),
                    name: clipboard.name.clone(),
                    kind: clipboard.kind,
                    size: clipboard.size,
                    modified_sort_key: clipboard
                        .remote_modified
                        .as_deref()
                        .map(remote_modified_sort_key)
                        .unwrap_or(0),
                    remote: Some(source),
                    remote_modified: clipboard.remote_modified.clone(),
                };
                self.prepare_remote_entry(
                    entry,
                    RemotePreparePurpose::ImportArchive {
                        current,
                        name,
                        replace,
                    },
                );
                return;
            }
            let Some(sender) = self.archive_sender.as_ref() else {
                self.active_tab_mut().status = "Archive worker is starting…".to_owned();
                return;
            };
            let command = if let Some(source) = crate::archive::parse_virtual_path(&clipboard.path)
            {
                crate::archive::Command::CopyMember {
                    current,
                    source,
                    name: name.clone(),
                    replace,
                }
            } else {
                crate::archive::Command::Import {
                    current,
                    source: clipboard.path.clone(),
                    name: name.clone(),
                    replace,
                }
            };
            if sender.send(command).is_err() {
                self.active_tab_mut().status = "Archive worker stopped".to_owned();
            } else {
                if clipboard.mode == ClipboardMode::Cut {
                    self.active_tab_mut().status =
                        format!("Adding {name} to archive… Source will be kept for safety.");
                } else {
                    self.active_tab_mut().status = format!("Adding {name} to archive…");
                }
            }
            return;
        }
        if let Some(source) = crate::archive::parse_virtual_path(&clipboard.path) {
            if clipboard.kind != EntryKind::File {
                self.active_tab_mut().status =
                    "Exporting archive directories is not available yet".to_owned();
                return;
            }
            let target_is_remote = matches!(
                parse_taildrive_path(&target_location),
                Some(TaildriveLocation::Remote { .. })
            );
            let name = if target_is_remote {
                unique_entry_copy_name(&self.active_tab().entries, &clipboard.name)
            } else {
                let Some((name, _replace)) =
                    self.paste_target_name(&clipboard, &target_location, conflict_resolution)
                else {
                    return;
                };
                name
            };
            let destination = if target_is_remote {
                let Some(root) = self.remote_cache_root() else {
                    self.active_tab_mut().status =
                        "Cannot prepare archive transfer cache".to_owned();
                    return;
                };
                root.join(format!(
                    "archive-upload-{}-{}",
                    next_transfer_id(),
                    remote_cache_filename(&name)
                ))
            } else {
                target_location.join(&name)
            };
            let Some(sender) = self.archive_sender.as_ref() else {
                self.active_tab_mut().status = "Archive worker is starting…".to_owned();
                return;
            };
            if sender
                .send(crate::archive::Command::Export {
                    source,
                    destination,
                    target_location: target_location.clone(),
                    target_name: name.clone(),
                    size: clipboard.size,
                })
                .is_err()
            {
                self.active_tab_mut().status = "Archive worker stopped".to_owned();
            } else {
                self.active_tab_mut().status = if target_is_remote {
                    format!("Preparing {name} for TailDrive upload…")
                } else if clipboard.mode == ClipboardMode::Cut {
                    format!("Exporting {name}… Archive source will be kept for safety.")
                } else {
                    format!("Exporting {name}…")
                };
            }
            return;
        }
        let source_remote = parse_taildrive_path(&clipboard.path);
        let source_is_remote = source_remote.is_some();
        let target_remote = parse_taildrive_path(&target_location);

        if let Some(TaildriveLocation::Remote {
            profile_id: target_profile_id,
            device_id: target_device_id,
            share: target_share,
            remote_path: target_parent,
        }) = target_remote
        {
            let Some((target_name, replace)) =
                self.paste_target_name(&clipboard, &target_location, conflict_resolution)
            else {
                return;
            };
            let target_path = remote_child_path(&target_parent, &target_name);
            let transfer_id = next_transfer_id();
            let upload_info = (!source_is_remote).then_some((clipboard.kind, clipboard.size));

            let command = match source_remote {
                Some(TaildriveLocation::Remote {
                    profile_id: source_profile_id,
                    device_id: source_device_id,
                    share: source_share,
                    remote_path: source_path,
                }) => crate::tailscale::Command::TaildriveRelay {
                    transfer_id: transfer_id.clone(),
                    source_profile_id,
                    source_device_id,
                    source_share,
                    source_path,
                    target_profile_id,
                    target_device_id,
                    target_share,
                    target_path,
                    display_name: target_name.clone(),
                    target_location: target_location.clone(),
                    source_was_cut: clipboard.mode == ClipboardMode::Cut,
                    replace,
                },
                Some(_) => {
                    self.active_tab_mut().status =
                        "This TailDrive item cannot be copied".to_owned();
                    return;
                }
                None => crate::tailscale::Command::TaildriveUpload {
                    profile_id: target_profile_id,
                    device_id: target_device_id,
                    share: target_share,
                    path: target_path,
                    source: clipboard.path,
                    source_location: target_location.clone(),
                    source_was_cut: clipboard.mode == ClipboardMode::Cut,
                    replace,
                    transfer_id: transfer_id.clone(),
                },
            };

            if let Err(error) = self.dispatch_taildrive_transfer(command) {
                self.active_tab_mut().status = error;
            } else {
                if let Some(info) = upload_info {
                    self.pending_upload_info.insert(transfer_id.clone(), info);
                }
                let phase = if source_is_remote {
                    "Copying"
                } else {
                    "Uploading"
                };
                self.begin_file_transfer(transfer_id, target_name.clone(), phase);
                self.active_tab_mut().status = if clipboard.mode == ClipboardMode::Cut {
                    format!("{phase} {target_name}… Source will be kept for safety.")
                } else {
                    format!("{phase} {target_name}…")
                };
            }
            return;
        }

        if let Some(TaildriveLocation::Remote {
            profile_id,
            device_id,
            share,
            remote_path,
        }) = source_remote
        {
            let Some((target_name, replace)) =
                self.paste_target_name(&clipboard, &target_location, conflict_resolution)
            else {
                return;
            };
            let destination = target_location.join(&target_name);
            let transfer_id = next_transfer_id();
            let command = crate::tailscale::Command::TaildriveDownload {
                profile_id,
                device_id,
                share,
                path: remote_path,
                destination,
                display_name: clipboard.name.clone(),
                source_location: target_location,
                transfer_id: transfer_id.clone(),
                open_after: false,
                source_was_cut: clipboard.mode == ClipboardMode::Cut,
                replace,
            };
            if let Err(error) = self.dispatch_taildrive_transfer(command) {
                self.active_tab_mut().status = error;
            } else {
                self.begin_file_transfer(transfer_id, clipboard.name.clone(), "Downloading");
                self.active_tab_mut().status = if clipboard.mode == ClipboardMode::Cut {
                    format!(
                        "Copying {} from TailDrive… Source will be kept for safety.",
                        clipboard.name
                    )
                } else {
                    format!("Copying {} from TailDrive…", clipboard.name)
                };
            }
            return;
        }

        if clipboard.kind == EntryKind::Directory && target_location.starts_with(&clipboard.path) {
            self.active_tab_mut().status = "Cannot paste a folder into itself".to_owned();
            return;
        }
        let Some((target_name, replace)) =
            self.paste_target_name(&clipboard, &target_location, conflict_resolution)
        else {
            return;
        };
        let destination = target_location.join(&target_name);
        self.submit_local_file_command(
            LocalFileCommand::CopyMove {
                current: target_location,
                source: clipboard.path,
                destination,
                cut: clipboard.mode == ClipboardMode::Cut,
                replace,
            },
            if clipboard.mode == ClipboardMode::Cut {
                format!("Moving {target_name}…")
            } else {
                format!("Copying {target_name}…")
            },
        );
    }

    pub fn delete_selected(&mut self) {
        if !self.can_mutate_current_location() {
            return;
        }
        let Some(path) = self.active_tab().selected_path.clone() else {
            return;
        };
        #[cfg(target_os = "android")]
        let name = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| self.display_path_for(&path));

        #[cfg(target_os = "android")]
        if self.confirm_mobile_delete && unix_ms_now() >= self.delete_warning_suppressed_until_ms {
            self.pending_delete_confirmation = Some(PendingDeleteConfirmation { path, name });
            return;
        }

        self.delete_path(path);
    }

    fn delete_path(&mut self, path: PathBuf) {
        if let Some(target) = crate::archive::parse_virtual_path(&path) {
            let Some(current) = crate::archive::parse_virtual_path(&self.active_tab().current_dir)
            else {
                return;
            };
            let Some(sender) = self.archive_sender.as_ref() else {
                self.active_tab_mut().status = "Archive worker is starting…".to_owned();
                return;
            };
            if sender
                .send(crate::archive::Command::Delete { current, target })
                .is_err()
            {
                self.active_tab_mut().status = "Archive worker stopped".to_owned();
            } else {
                self.active_tab_mut().selected_path = None;
                self.active_tab_mut().status = "Deleting from archive…".to_owned();
            }
            return;
        }
        if let Some((
            name,
            TaildriveLocation::Remote {
                profile_id,
                device_id,
                share,
                remote_path,
            },
        )) = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .and_then(|entry| {
                entry
                    .remote
                    .clone()
                    .map(|remote| (entry.name.clone(), remote))
            })
        {
            #[cfg(not(target_os = "android"))]
            {
                let now = Instant::now();
                let confirmed = self
                    .active_tab()
                    .pending_remote_delete
                    .as_ref()
                    .is_some_and(|(pending, when)| {
                        pending == &path && now.duration_since(*when) <= Duration::from_secs(4)
                    });
                if !confirmed {
                    self.active_tab_mut().pending_remote_delete = Some((path, now));
                    self.active_tab_mut().status = format!(
                        "TailDrive deletes permanently. Press Delete again within 4 seconds to delete {name}."
                    );
                    return;
                }
            }
            self.active_tab_mut().pending_remote_delete = None;
            let Some(sender) = self.tailscale_sender.clone() else {
                self.active_tab_mut().status = "TailDrive worker is starting…".to_owned();
                return;
            };
            let source_location = self.active_tab().current_dir.clone();
            if !self.begin_remote_mutation(source_location.clone()) {
                self.active_tab_mut().status =
                    "Another TailDrive change is still running".to_owned();
                return;
            }
            if sender
                .send(crate::tailscale::Command::TaildriveDelete {
                    profile_id,
                    device_id,
                    share,
                    path: remote_path,
                    source_location: source_location.clone(),
                })
                .is_err()
            {
                self.finish_remote_mutation(&source_location);
                self.active_tab_mut().status = "TailDrive worker stopped unexpectedly".to_owned();
            } else {
                self.active_tab_mut().status = format!("Deleting {name} from TailDrive…");
            }
            return;
        }
        let current = self.active_tab().current_dir.clone();
        let name = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| self.display_path_for(&path));
        let status = if cfg!(target_os = "android") {
            format!("Deleting {name}…")
        } else {
            format!("Moving {name} to Trash…")
        };
        self.submit_local_file_command(LocalFileCommand::Delete { current, path }, status);
    }

    pub fn begin_rename(&mut self) {
        if !self.can_mutate_current_location() {
            return;
        }
        let Some(path) = self.active_tab().selected_path.clone() else {
            return;
        };
        if crate::archive::parse_virtual_path(&path).is_some() {
            if let Some(entry) = self
                .active_tab()
                .entries
                .iter()
                .find(|entry| entry.path == path)
                .cloned()
            {
                let suffix = keyboard_rename_suffix_for(entry.kind, &entry.name);
                self.active_tab_mut().rename_input = Some(entry.name);
                self.active_tab_mut().rename_replace_on_type = true;
                self.active_tab_mut().rename_keyboard_suffix = Some(suffix);
            }
            return;
        }
        if let Some(entry) = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == path && entry.remote.is_some())
            .cloned()
        {
            let suffix = keyboard_rename_suffix_for(entry.kind, &entry.name);
            self.active_tab_mut().rename_input = Some(entry.name);
            self.active_tab_mut().rename_replace_on_type = true;
            self.active_tab_mut().rename_keyboard_suffix = Some(suffix);
            return;
        }
        let Some(entry) = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .cloned()
        else {
            return;
        };
        let suffix = keyboard_rename_suffix_for(entry.kind, &entry.name);
        self.active_tab_mut().rename_input = Some(entry.name);
        self.active_tab_mut().rename_replace_on_type = true;
        self.active_tab_mut().rename_keyboard_suffix = Some(suffix);
    }

    pub fn set_rename_input(&mut self, value: String) {
        if self.active_tab().rename_input.is_some() {
            self.active_tab_mut().rename_input = Some(single_line_input(value));
            self.active_tab_mut().rename_replace_on_type = false;
            self.active_tab_mut().rename_keyboard_suffix = None;
        }
    }

    pub fn submit_rename(&mut self, value: String) {
        if self.active_tab().rename_input.is_some() {
            self.active_tab_mut().rename_input = Some(single_line_input(value));
            self.active_tab_mut().rename_replace_on_type = false;
            self.active_tab_mut().rename_keyboard_suffix = None;
            self.apply_rename();
        }
    }

    pub fn type_rename_text(&mut self, text: String) {
        let tab = self.active_tab_mut();
        let Some(input) = tab.rename_input.as_mut() else {
            return;
        };
        let Some(suffix) = tab.rename_keyboard_suffix.as_deref() else {
            input.push_str(&text);
            return;
        };
        if tab.rename_replace_on_type {
            *input = format!("{text}{suffix}");
            tab.rename_replace_on_type = false;
        } else if suffix.is_empty() {
            input.push_str(&text);
        } else {
            let insert_at = input.len().saturating_sub(suffix.len());
            input.insert_str(insert_at, &text);
        }
    }

    pub fn backspace_rename(&mut self) {
        let tab = self.active_tab_mut();
        let Some(input) = tab.rename_input.as_mut() else {
            return;
        };
        let Some(suffix) = tab.rename_keyboard_suffix.as_deref() else {
            input.pop();
            return;
        };
        if tab.rename_replace_on_type {
            *input = suffix.to_owned();
            tab.rename_replace_on_type = false;
            return;
        }
        let editable_end = input.len().saturating_sub(suffix.len());
        if let Some((start, _)) = input[..editable_end].char_indices().next_back() {
            input.drain(start..editable_end);
        }
    }

    pub fn apply_rename(&mut self) {
        if !self.can_mutate_current_location() {
            self.cancel_rename();
            return;
        }
        let Some(source) = self.active_tab().selected_path.clone() else {
            self.active_tab_mut().rename_input = None;
            self.active_tab_mut().rename_replace_on_type = false;
            self.active_tab_mut().rename_keyboard_suffix = None;
            return;
        };
        let Some(name) = self.active_tab().rename_input.clone() else {
            return;
        };
        let name = name.trim();
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
        {
            self.active_tab_mut().status = "Invalid file name".to_owned();
            return;
        }
        if let Some(target) = crate::archive::parse_virtual_path(&source) {
            let conflict = self.active_tab().entries.iter().any(|candidate| {
                candidate.path != source && candidate.name.eq_ignore_ascii_case(name)
            });
            if conflict {
                self.active_tab_mut().status = "An item with that name already exists".to_owned();
                return;
            }
            let Some(current) = crate::archive::parse_virtual_path(&self.active_tab().current_dir)
            else {
                return;
            };
            let Some(sender) = self.archive_sender.as_ref() else {
                self.active_tab_mut().status = "Archive worker is starting…".to_owned();
                return;
            };
            if sender
                .send(crate::archive::Command::Rename {
                    current,
                    target,
                    new_name: name.to_owned(),
                })
                .is_err()
            {
                self.active_tab_mut().status = "Archive worker stopped".to_owned();
            } else {
                self.active_tab_mut().rename_input = None;
                self.active_tab_mut().rename_replace_on_type = false;
                self.active_tab_mut().rename_keyboard_suffix = None;
                self.active_tab_mut().status = format!("Renaming to {name} in archive…");
            }
            return;
        }
        if let Some(entry) = self
            .active_tab()
            .entries
            .iter()
            .find(|entry| entry.path == source)
            .cloned()
            && let Some(TaildriveLocation::Remote {
                profile_id,
                device_id,
                share,
                remote_path,
            }) = entry.remote
        {
            let conflict = self.active_tab().entries.iter().any(|candidate| {
                candidate.path != source && candidate.name.eq_ignore_ascii_case(name)
            });
            if conflict {
                self.active_tab_mut().status = "An item with that name already exists".to_owned();
                return;
            }
            let source_location = self.active_tab().current_dir.clone();
            let pending_create = self.active_tab().pending_remote_folder.as_ref() == Some(&source);
            let Some(sender) = self.tailscale_sender.clone() else {
                self.active_tab_mut().status = "TailDrive worker is starting…".to_owned();
                return;
            };
            if pending_create {
                let parent_remote_path = match parse_taildrive_path(&source_location) {
                    Some(TaildriveLocation::Remote { remote_path, .. }) => remote_path,
                    _ => return,
                };
                let final_remote_path = remote_child_path(&parent_remote_path, name);
                if !self.begin_remote_mutation(source_location.clone()) {
                    self.active_tab_mut().status =
                        "Another TailDrive change is still running".to_owned();
                    return;
                }
                let final_remote = TaildriveLocation::Remote {
                    profile_id: profile_id.clone(),
                    device_id: device_id.clone(),
                    share: share.clone(),
                    remote_path: final_remote_path.clone(),
                };
                let final_path = taildrive_path(&final_remote);
                if let Some(item) = self
                    .active_tab_mut()
                    .entries
                    .iter_mut()
                    .find(|item| item.path == source)
                {
                    item.path = final_path.clone();
                    item.name = name.to_owned();
                    item.remote = Some(final_remote);
                }
                self.active_tab_mut().selected_path = Some(final_path.clone());
                self.active_tab_mut().pending_remote_folder = Some(final_path);
                if sender
                    .send(crate::tailscale::Command::TaildriveMkdir {
                        profile_id,
                        device_id,
                        share,
                        path: final_remote_path,
                        source_location: source_location.clone(),
                    })
                    .is_err()
                {
                    self.finish_remote_mutation(&source_location);
                    self.active_tab_mut().status =
                        "TailDrive worker stopped unexpectedly".to_owned();
                    return;
                }
                self.active_tab_mut().rename_input = None;
                self.active_tab_mut().rename_replace_on_type = false;
                self.active_tab_mut().rename_keyboard_suffix = None;
                self.active_tab_mut().status = format!("Creating TailDrive folder {name}…");
                return;
            }
            if entry.name == name {
                self.cancel_rename();
                return;
            }
            if !self.begin_remote_mutation(source_location.clone()) {
                self.active_tab_mut().status =
                    "Another TailDrive change is still running".to_owned();
                return;
            }
            if sender
                .send(crate::tailscale::Command::TaildriveRename {
                    profile_id,
                    device_id,
                    share,
                    path: remote_path,
                    new_name: name.to_owned(),
                    source_location: source_location.clone(),
                })
                .is_err()
            {
                self.finish_remote_mutation(&source_location);
                self.active_tab_mut().status = "TailDrive worker stopped unexpectedly".to_owned();
                return;
            }
            self.active_tab_mut().rename_input = None;
            self.active_tab_mut().rename_replace_on_type = false;
            self.active_tab_mut().rename_keyboard_suffix = None;
            self.active_tab_mut().status = format!("Renaming to {name}…");
            return;
        }
        let destination = source.with_file_name(name);
        if destination == source {
            self.active_tab_mut().rename_input = None;
            self.active_tab_mut().rename_replace_on_type = false;
            self.active_tab_mut().rename_keyboard_suffix = None;
            return;
        }
        let current = self.active_tab().current_dir.clone();
        self.active_tab_mut().rename_input = None;
        self.active_tab_mut().rename_replace_on_type = false;
        self.active_tab_mut().rename_keyboard_suffix = None;
        self.submit_local_file_command(
            LocalFileCommand::Rename {
                current,
                source,
                destination,
            },
            format!("Renaming to {name}…"),
        );
    }

    fn reload_after_mutation(&mut self, preferred_selection: Option<PathBuf>) {
        if self.active_tab().search_active {
            self.run_active_search();
            return;
        }
        self.request_directory_reload();
        if let Some(path) = preferred_selection
            && self
                .active_tab()
                .entries
                .iter()
                .any(|entry| entry.path == path)
        {
            self.active_tab_mut().select_entry(path);
        }
    }

    pub fn cancel_rename(&mut self) {
        let pending = if self.active_tab().rename_input.is_some() {
            self.active_tab().pending_remote_folder.clone()
        } else {
            None
        };
        if let Some(path) = pending {
            self.active_tab_mut()
                .entries
                .retain(|entry| entry.path != path);
            if self.active_tab().selected_path.as_ref() == Some(&path) {
                self.active_tab_mut().selected_path = None;
            }
            self.active_tab_mut().pending_remote_folder = None;
            self.active_tab_mut().status = "New folder cancelled".to_owned();
        }
        self.active_tab_mut().rename_input = None;
        self.active_tab_mut().rename_replace_on_type = false;
        self.active_tab_mut().rename_keyboard_suffix = None;
    }
}

pub fn perform_persist_command(command: &PersistCommand) -> Result<(), String> {
    write_bytes_atomic(&command.path, &command.bytes)?;
    #[cfg(target_os = "android")]
    cleanup_legacy_android_config_after_persist(&command.path);
    Ok(())
}

#[cfg(target_os = "android")]
fn cleanup_legacy_android_config_after_persist(persisted_path: &Path) {
    let Some(current) = config_path() else {
        return;
    };
    if current != persisted_path
        || android_config_migration_complete()
        || !ANDROID_LEGACY_CONFIG_LOADED.load(Ordering::Acquire)
    {
        return;
    }
    let Some(legacy) = legacy_android_config_path() else {
        return;
    };
    if legacy == current {
        return;
    }

    let legacy_removed = match fs::remove_file(&legacy) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(error) => {
            eprintln!(
                "FastExplorer: failed to remove legacy Android config {}: {error}",
                legacy.display()
            );
            false
        }
    };
    if !legacy_removed {
        return;
    }

    match mark_android_config_migration_complete() {
        Ok(()) => {
            ANDROID_LEGACY_CONFIG_LOADED.store(false, Ordering::Release);
            eprintln!(
                "FastExplorer: migrated Android config from {} to app-private storage",
                legacy.display()
            );
        }
        Err(error) => eprintln!(
            "FastExplorer: migrated legacy Android config but could not record completion: {error}"
        ),
    }
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "persistence path has no parent".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    if fs::rename(&temp, path).is_err() {
        let _ = fs::remove_file(path);
        if let Err(error) = fs::rename(&temp, path) {
            let _ = fs::remove_file(&temp);
            return Err(error.to_string());
        }
    }
    Ok(())
}

pub fn perform_remote_prepare(
    request: &RemotePrepareRequest,
) -> Result<RemotePrepareResult, String> {
    fs::create_dir_all(&request.cache_root).map_err(|error| error.to_string())?;
    let source_key = remote_cache_source_key(&request.source);
    if let Some(cached) = remote_cache_try_acquire(
        &request.cache_root,
        &source_key,
        request.remote_size,
        &request.remote_modified,
        request.cache_settings,
    )? {
        return Ok(RemotePrepareResult::Cached(cached));
    }
    let cache_file_name = remote_cache_filename(&request.display_name);
    Ok(RemotePrepareResult::Download {
        destination: request.cache_root.join(&cache_file_name),
        cache_file_name,
        source_key,
    })
}

pub fn perform_cache_command(command: &CacheCommand) -> Result<u64, String> {
    match command {
        CacheCommand::Maintain {
            root,
            settings,
            protected,
            ..
        } => remote_cache_cleanup_with_protected(root, *settings, protected),
        CacheCommand::Clear {
            root,
            settings,
            protected,
            ..
        } => {
            remote_cache_clear(root, protected)?;
            remote_cache_cleanup_with_protected(root, *settings, protected)
        }
        CacheCommand::Record {
            root,
            source_key,
            destination,
            display_name,
            remote_size,
            remote_modified,
            settings,
            protected,
        } => remote_cache_record(
            root,
            source_key.clone(),
            destination,
            display_name.clone(),
            *remote_size,
            remote_modified.clone(),
            *settings,
            protected,
        ),
        CacheCommand::RemoveTemp {
            root,
            path,
            settings,
            protected,
        } => {
            let _ = fs::remove_file(path);
            remote_cache_cleanup_with_protected(root, *settings, protected)
        }
    }
}

pub fn perform_local_file_command(command: &LocalFileCommand) -> Result<(), String> {
    match command {
        LocalFileCommand::CreateDir { path, .. } => {
            fs::create_dir(path).map_err(|error| error.to_string())
        }
        LocalFileCommand::CopyMove {
            source,
            destination,
            cut,
            replace,
            ..
        } => {
            if destination.exists() && !replace {
                return Err(format!(
                    "Destination already exists: {}",
                    display_path(destination)
                ));
            }
            if *replace {
                copy_path_recursive_replacing(source, destination)?;
                if *cut {
                    remove_path_permanently(source)?;
                }
                Ok(())
            } else if *cut {
                move_path(source, destination)
            } else {
                copy_path_recursive(source, destination)
            }
        }
        LocalFileCommand::Delete { path, .. } => move_to_trash(path),
        LocalFileCommand::Rename {
            source,
            destination,
            ..
        } => {
            if rename_destination_conflicts(source, destination) {
                return Err("A file with that name already exists".to_owned());
            }
            fs::rename(source, destination).map_err(|error| error.to_string())
        }
    }
}

fn keyboard_rename_suffix_for(kind: EntryKind, name: &str) -> String {
    if kind == EntryKind::Directory {
        return String::new();
    }
    Path::new(name)
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default()
}

fn rename_destination_conflicts(source: &Path, destination: &Path) -> bool {
    if !destination.exists() {
        return false;
    }
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        let case_only_name =
            source
                .file_name()
                .zip(destination.file_name())
                .is_some_and(|(source, destination)| {
                    source
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&destination.to_string_lossy())
                });
        if case_only_name && fs::canonicalize(source).ok() == fs::canonicalize(destination).ok() {
            return false;
        }
    }
    true
}

#[cfg(not(target_os = "android"))]
fn move_to_trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|error| error.to_string())
}

#[cfg(target_os = "android")]
fn move_to_trash(path: &Path) -> Result<(), String> {
    // Android's shared-storage APIs do not expose a filesystem Trash operation to
    // an all-files-access app. FastExplorer therefore performs a permanent delete,
    // guarded by the mobile confirmation UI unless the user disables it.
    remove_path_permanently(path)
}

#[cfg(not(target_os = "android"))]
fn open_url_with_system(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) URLs can be opened".to_owned());
    }
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(target_os = "android")]
fn share_path_with_system(path: &Path, state: &AppState) -> Result<(), String> {
    state
        .android_app
        .as_ref()
        .ok_or_else(|| "Android activity is not attached".to_owned())
        .and_then(|app| crate::android_platform::share_file(app, path))
}

#[cfg(target_os = "windows")]
fn share_path_with_system(path: &Path, _state: &AppState) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb = "share\0".encode_utf16().collect::<Vec<_>>();
    let file = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut info = SHELLEXECUTEINFOW::default();
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_INVOKEIDLIST;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    // SAFETY: all pointers reference live, NUL-terminated buffers for the duration of ShellExecuteExW.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(not(any(target_os = "android", target_os = "windows")))]
fn share_path_with_system(_path: &Path, _state: &AppState) -> Result<(), String> {
    Err("Quick Share is currently available on Windows and Android".to_owned())
}

#[cfg(target_os = "android")]
fn open_path_with_system(_path: &Path) -> Result<(), String> {
    Err("opening local files with Android apps is not available yet".to_owned())
}

#[cfg(not(target_os = "android"))]
fn open_path_with_system(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]).arg(path);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };

    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    return Err("opening files is not supported on this platform".to_owned());

    #[cfg(any(target_os = "windows", target_os = "macos", unix))]
    {
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

fn is_android_install_name(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("apk")
            || extension.eq_ignore_ascii_case("apks")
            || extension.eq_ignore_ascii_case("aab")
    })
}

fn is_aab_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("aab"))
}

fn unique_entry_name(entries: &[FileEntry], base_name: &str) -> String {
    let exists = |candidate: &str| {
        entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(candidate))
    };
    if !exists(base_name) {
        return base_name.to_owned();
    }
    for index in 2.. {
        let candidate = format!("{base_name} ({index})");
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn unique_entry_copy_name(entries: &[FileEntry], source_name: &str) -> String {
    let exists = |candidate: &str| {
        entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(candidate))
    };
    if !exists(source_name) {
        return source_name.to_owned();
    }
    let source = Path::new(source_name);
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| source_name.to_owned());
    let extension = source
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned());
    for index in 1.. {
        let suffix = if index == 1 {
            " - Copy".to_owned()
        } else {
            format!(" - Copy ({index})")
        };
        let candidate = extension.as_ref().map_or_else(
            || format!("{stem}{suffix}"),
            |extension| format!("{stem}{suffix}.{extension}"),
        );
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn remote_child_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{}/{name}", parent.trim_end_matches('/'))
    }
}

fn next_transfer_id() -> String {
    let counter = TRANSFER_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{stamp}-{counter}", std::process::id())
}

fn system_time_sort_key(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn remote_modified_sort_key(value: &str) -> u64 {
    httpdate::parse_http_date(value)
        .ok()
        .map(system_time_sort_key)
        .unwrap_or(0)
}

pub fn scan_directory(path: &Path, show_hidden: bool) -> Result<Vec<FileEntry>, String> {
    let read_dir = fs::read_dir(path).map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        if let Some(item) = file_entry(entry.path(), show_hidden) {
            entries.push(item);
        }
    }
    entries.sort_by(|a, b| {
        let a_dir = a.kind == EntryKind::Directory;
        let b_dir = b.kind == EntryKind::Directory;
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

fn file_entry(path: PathBuf, show_hidden: bool) -> Option<FileEntry> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    if !show_hidden && name.starts_with('.') {
        return None;
    }
    let metadata = fs::symlink_metadata(&path).ok()?;
    let file_type = metadata.file_type();
    let kind = if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else if file_type.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    };
    let size = if kind == EntryKind::File {
        metadata.len()
    } else {
        0
    };
    Some(FileEntry {
        path,
        name,
        kind,
        size,
        modified_sort_key: metadata
            .modified()
            .ok()
            .map(system_time_sort_key)
            .unwrap_or(0),
        remote: None,
        remote_modified: None,
    })
}

fn copy_path_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if metadata.is_dir() {
        fs::create_dir(destination).map_err(|error| error.to_string())?;
        for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            copy_path_recursive(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(source, destination)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn remove_path_permanently(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(path).map_err(|error| error.to_string())
    }
}

fn copy_path_recursive_replacing(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        if destination.exists() {
            let destination_metadata =
                fs::symlink_metadata(destination).map_err(|error| error.to_string())?;
            if !destination_metadata.is_dir() || destination_metadata.file_type().is_symlink() {
                remove_path_permanently(destination)?;
                fs::create_dir(destination).map_err(|error| error.to_string())?;
            }
        } else {
            fs::create_dir(destination).map_err(|error| error.to_string())?;
        }
        for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            copy_path_recursive_replacing(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if destination.exists()
            && fs::symlink_metadata(destination)
                .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                .unwrap_or(false)
        {
            remove_path_permanently(destination)?;
        }
        fs::copy(source, destination)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn move_path(source: &Path, destination: &Path) -> Result<(), String> {
    if fs::rename(source, destination).is_ok() {
        return Ok(());
    }
    copy_path_recursive(source, destination)?;
    remove_path_permanently(source)
}

fn remote_cache_filename(display_name: &str) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let extension = Path::new(display_name)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 24
                && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    let mut name = format!("{}-{stamp}", std::process::id());
    if let Some(extension) = extension {
        name.push('.');
        name.push_str(extension);
    }
    name
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(target_os = "android")]
fn next_utc_day_boundary_ms() -> u64 {
    const DAY_MS: u64 = 24 * 60 * 60 * 1000;
    let now = unix_ms_now();
    now.saturating_div(DAY_MS)
        .saturating_add(1)
        .saturating_mul(DAY_MS)
}

fn remote_cache_now_ms() -> u64 {
    unix_ms_now()
}

fn remote_cache_index_path(root: &Path) -> PathBuf {
    root.join("index.json")
}

fn load_remote_cache_index(root: &Path) -> RemoteOpenCacheIndex {
    fs::read_to_string(remote_cache_index_path(root))
        .ok()
        .and_then(|text| serde_json::from_str::<RemoteOpenCacheIndex>(&text).ok())
        .filter(|index| index.version == REMOTE_OPEN_CACHE_VERSION)
        .unwrap_or_else(|| RemoteOpenCacheIndex {
            version: REMOTE_OPEN_CACHE_VERSION,
            entries: BTreeMap::new(),
        })
}

fn save_remote_cache_index(root: &Path, index: &RemoteOpenCacheIndex) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    save_json_atomic(&remote_cache_index_path(root), index)
}

fn remote_cache_source_key(location: &TaildriveLocation) -> String {
    taildrive_path(location)
        .iter()
        .map(|component| component.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
fn remote_cache_cleanup(root: &Path, settings: RemoteCacheSettings) -> Result<u64, String> {
    remote_cache_cleanup_with_protected(root, settings, &BTreeSet::new())
}

fn remote_cache_cleanup_with_protected(
    root: &Path,
    settings: RemoteCacheSettings,
    protected_files: &BTreeSet<String>,
) -> Result<u64, String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let mut index = load_remote_cache_index(root);
    let now = remote_cache_now_ms();
    let expiry_ms = u64::from(settings.expiration_hours).saturating_mul(60 * 60 * 1000);

    index.entries.retain(|_, entry| {
        let path = root.join(&entry.file_name);
        let exists = path.is_file();
        #[cfg(target_os = "android")]
        if let Ok(modified) = fs::metadata(&path).and_then(|metadata| metadata.modified())
            && let Ok(duration) = modified.duration_since(UNIX_EPOCH)
        {
            entry.last_accessed_unix_ms =
                entry.last_accessed_unix_ms.max(duration.as_millis() as u64);
        }
        let in_use = protected_files.contains(&entry.file_name);
        let expired = expiry_ms > 0 && now.saturating_sub(entry.last_accessed_unix_ms) > expiry_ms;
        if exists && (in_use || !expired) {
            true
        } else {
            if exists {
                let _ = fs::remove_file(path);
            }
            false
        }
    });

    let known = index
        .entries
        .values()
        .map(|entry| entry.file_name.clone())
        .collect::<BTreeSet<_>>();
    if let Ok(files) = fs::read_dir(root) {
        for file in files.flatten() {
            let path = file.path();
            let name = file.file_name().to_string_lossy().into_owned();
            if path.is_file()
                && name != "index.json"
                && !known.contains(&name)
                && !protected_files.contains(&name)
            {
                let _ = fs::remove_file(path);
            }
        }
    }

    let limit_bytes = u64::from(settings.limit_mib).saturating_mul(1024 * 1024);
    let mut usage = index
        .entries
        .values()
        .filter_map(|entry| fs::metadata(root.join(&entry.file_name)).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
    if limit_bytes > 0 && usage > limit_bytes {
        let mut oldest = index
            .entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.last_accessed_unix_ms))
            .collect::<Vec<_>>();
        oldest.sort_by_key(|(_, accessed)| *accessed);
        for (key, _) in oldest {
            if usage <= limit_bytes || index.entries.len() <= 1 {
                break;
            }
            if index
                .entries
                .get(&key)
                .is_some_and(|entry| protected_files.contains(&entry.file_name))
            {
                continue;
            }
            if let Some(entry) = index.entries.remove(&key) {
                let path = root.join(entry.file_name);
                let size = fs::metadata(&path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                let _ = fs::remove_file(path);
                usage = usage.saturating_sub(size);
            }
        }
    }
    save_remote_cache_index(root, &index)?;
    Ok(usage)
}

fn remote_cache_try_acquire(
    root: &Path,
    source_key: &str,
    remote_size: u64,
    remote_modified: &str,
    settings: RemoteCacheSettings,
) -> Result<Option<PathBuf>, String> {
    let mut index = load_remote_cache_index(root);
    if remote_modified.is_empty() {
        return Ok(None);
    }
    let Some(entry) = index.entries.get_mut(source_key) else {
        return Ok(None);
    };
    let path = root.join(&entry.file_name);
    #[cfg(target_os = "android")]
    if let Ok(modified) = fs::metadata(&path).and_then(|metadata| metadata.modified())
        && let Ok(duration) = modified.duration_since(UNIX_EPOCH)
    {
        entry.last_accessed_unix_ms = entry.last_accessed_unix_ms.max(duration.as_millis() as u64);
    }
    let expiry_ms = u64::from(settings.expiration_hours).saturating_mul(60 * 60 * 1000);
    let now = remote_cache_now_ms();
    let expired = expiry_ms > 0 && now.saturating_sub(entry.last_accessed_unix_ms) > expiry_ms;
    let version_matches = !remote_modified.is_empty()
        && !entry.remote_modified.is_empty()
        && entry.remote_size == remote_size
        && entry.remote_modified == remote_modified;
    if !path.is_file() || expired || !version_matches {
        index.entries.remove(source_key);
        save_remote_cache_index(root, &index)?;
        return Ok(None);
    }
    entry.last_accessed_unix_ms = now;
    save_remote_cache_index(root, &index)?;
    Ok(Some(path))
}

#[allow(clippy::too_many_arguments)]
fn remote_cache_record(
    root: &Path,
    source_key: String,
    file_path: &Path,
    display_name: String,
    remote_size: u64,
    remote_modified: String,
    settings: RemoteCacheSettings,
    protected_files: &BTreeSet<String>,
) -> Result<u64, String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let file_name = file_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| "remote cache path has no file name".to_owned())?;
    let now = remote_cache_now_ms();
    let mut index = load_remote_cache_index(root);
    index.entries.insert(
        source_key,
        RemoteOpenCacheEntry {
            file_name: file_name.clone(),
            display_name,
            remote_size,
            remote_modified,
            cached_unix_ms: now,
            last_accessed_unix_ms: now,
        },
    );
    save_remote_cache_index(root, &index)?;
    remote_cache_cleanup_with_protected(root, settings, protected_files)
}

fn remote_cache_clear(root: &Path, protected_files: &BTreeSet<String>) -> Result<(), String> {
    if let Ok(files) = fs::read_dir(root) {
        for file in files.flatten() {
            let path = file.path();
            let name = file.file_name().to_string_lossy().into_owned();
            if path.is_file() && !protected_files.contains(&name) {
                let _ = fs::remove_file(path);
            }
        }
    }
    save_remote_cache_index(
        root,
        &RemoteOpenCacheIndex {
            version: REMOTE_OPEN_CACHE_VERSION,
            entries: BTreeMap::new(),
        },
    )
}

fn taildrive_directory_cache_path() -> Option<PathBuf> {
    session_path().map(|path| path.with_file_name("taildrive-directory-cache.json"))
}

fn taildrive_directory_cache_key(location: &TaildriveLocation) -> String {
    taildrive_path(location)
        .iter()
        .map(|component| component.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn empty_taildrive_directory_cache() -> TaildriveDirectoryCache {
    TaildriveDirectoryCache {
        version: TAILDRIVE_DIRECTORY_CACHE_VERSION,
        directories: BTreeMap::new(),
    }
}

pub(crate) fn preload_taildrive_directory_cache() {
    if TAILDRIVE_DIRECTORY_CACHE_MEMORY.get().is_some() {
        return;
    }
    let cache = taildrive_directory_cache_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<TaildriveDirectoryCache>(&text).ok())
        .filter(|cache| cache.version == TAILDRIVE_DIRECTORY_CACHE_VERSION)
        .unwrap_or_else(empty_taildrive_directory_cache);
    let _ = TAILDRIVE_DIRECTORY_CACHE_MEMORY.set(Mutex::new(cache));
}

fn taildrive_directory_cache_memory() -> &'static Mutex<TaildriveDirectoryCache> {
    TAILDRIVE_DIRECTORY_CACHE_MEMORY.get_or_init(|| Mutex::new(empty_taildrive_directory_cache()))
}

fn taildrive_directory_cache_writer()
-> &'static std::sync::mpsc::Sender<(PathBuf, TaildriveDirectoryCache)> {
    TAILDRIVE_DIRECTORY_CACHE_WRITER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<(PathBuf, TaildriveDirectoryCache)>();
        std::thread::Builder::new()
            .name("taildrive-cache-writer".to_owned())
            .spawn(move || {
                while let Ok((mut path, mut cache)) = receiver.recv() {
                    while let Ok((new_path, newer_cache)) = receiver.try_recv() {
                        path = new_path;
                        cache = newer_cache;
                    }
                    if let Err(error) = save_json_atomic(&path, &cache) {
                        eprintln!(
                            "FastExplorer: failed to save TailDrive directory cache: {error}"
                        );
                    }
                }
            })
            .expect("failed to start TailDrive cache writer");
        sender
    })
}

fn load_taildrive_directory_cache_entries(
    location: &TaildriveLocation,
    show_hidden: bool,
) -> Vec<FileEntry> {
    let TaildriveLocation::Remote {
        profile_id,
        device_id,
        share,
        ..
    } = location
    else {
        return Vec::new();
    };
    let Some(memory) = TAILDRIVE_DIRECTORY_CACHE_MEMORY.get() else {
        return Vec::new();
    };
    let cache = memory
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = taildrive_directory_cache_key(location);
    let Some(directory) = cache.directories.get(&key) else {
        return Vec::new();
    };
    let mut entries = directory
        .entries
        .iter()
        .filter(|entry| show_hidden || !entry.name.starts_with('.'))
        .map(|entry| {
            let remote = TaildriveLocation::Remote {
                profile_id: profile_id.clone(),
                device_id: device_id.clone(),
                share: share.clone(),
                remote_path: entry.remote_path.clone(),
            };
            FileEntry {
                path: taildrive_path(&remote),
                name: entry.name.clone(),
                kind: if entry.directory {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: entry.size,
                modified_sort_key: remote_modified_sort_key(&entry.modified),
                remote: Some(remote),
                // Directory-list cache metadata is historical, not a fresh remote
                // validation. Only a live TailDrive list may authorize a file-cache hit.
                remote_modified: None,
            }
        })
        .collect::<Vec<_>>();
    drop(cache);
    entries.sort_by(|a, b| {
        let a_dir = a.kind == EntryKind::Directory;
        let b_dir = b.kind == EntryKind::Directory;
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

fn save_taildrive_directory_cache_entries(
    location: &TaildriveLocation,
    entries: Vec<CachedTaildriveEntry>,
) {
    let Some(path) = taildrive_directory_cache_path() else {
        return;
    };
    let memory = taildrive_directory_cache_memory();
    let key = taildrive_directory_cache_key(location);
    let updated_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let snapshot = {
        let mut cache = memory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.directories.insert(
            key,
            CachedTaildriveDirectory {
                updated_unix_ms,
                entries,
            },
        );
        while cache.directories.len() > TAILDRIVE_DIRECTORY_CACHE_LIMIT {
            let Some(oldest) = cache
                .directories
                .iter()
                .min_by_key(|(_, directory)| directory.updated_unix_ms)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            cache.directories.remove(&oldest);
        }
        cache.clone()
    };
    if taildrive_directory_cache_writer()
        .send((path, snapshot))
        .is_err()
    {
        eprintln!("FastExplorer: TailDrive directory cache writer stopped unexpectedly");
    }
}

fn default_directory() -> PathBuf {
    home_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn session_path() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    if let Some(state_dir) = ANDROID_STATE_DIR.get() {
        return Some(state_dir.join("session.json"));
    }
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(local).join("FastExplorer/session.json"));
    }
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(state_home).join("fast-explorer/session.json"));
    }
    home_dir().map(|home| home.join(".local/state/fast-explorer/session.json"))
}

#[cfg(target_os = "android")]
fn legacy_android_session_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".local/state/fast-explorer/session.json"))
}

#[cfg(target_os = "android")]
fn remove_legacy_android_session() {
    let Some(current) = session_path() else {
        return;
    };
    let Some(legacy) = legacy_android_session_path() else {
        return;
    };
    if legacy == current || !legacy.is_file() {
        return;
    }
    if let Err(error) = fs::remove_file(&legacy) {
        eprintln!(
            "FastExplorer: failed to remove migrated Android session {}: {error}",
            legacy.display()
        );
    }
}

fn supabase_session_path() -> Option<PathBuf> {
    session_path().and_then(|path| {
        path.parent()
            .map(|parent| parent.join("supabase-session.json"))
    })
}

#[cfg(target_os = "android")]
fn legacy_android_config_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".config/fast-explorer/config.json"))
}

#[cfg(target_os = "android")]
fn android_config_migration_marker_path() -> Option<PathBuf> {
    ANDROID_STATE_DIR
        .get()
        .map(|state_dir| state_dir.join("legacy-config-migrated"))
}

#[cfg(target_os = "android")]
fn android_config_migration_complete() -> bool {
    android_config_migration_marker_path().is_some_and(|path| path.is_file())
}

#[cfg(target_os = "android")]
fn backup_private_config_before_legacy_import(private_path: &Path) {
    if !private_path.is_file() {
        return;
    }
    let Some(state_dir) = ANDROID_STATE_DIR.get() else {
        return;
    };
    let backup = state_dir.join("config.pre-legacy-migration.json");
    if backup.exists() {
        return;
    }
    if let Err(error) = fs::copy(private_path, &backup) {
        eprintln!(
            "FastExplorer: failed to preserve pre-migration Android config {}: {error}",
            private_path.display()
        );
    }
}

#[cfg(target_os = "android")]
fn mark_android_config_migration_complete() -> Result<(), String> {
    let marker = android_config_migration_marker_path()
        .ok_or_else(|| "Android state directory is unavailable".to_owned())?;
    write_bytes_atomic(&marker, b"migrated\n")
}

fn config_path() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    if let Some(state_dir) = ANDROID_STATE_DIR.get() {
        // App configuration must remain writable before all-files access is granted.
        // Keep it in app-private storage alongside the Android session state.
        return Some(state_dir.join("config.json"));
    }
    #[cfg(target_os = "windows")]
    if let Some(config_home) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(config_home).join("FastExplorer/config.json"));
    }
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("fast-explorer/config.json"));
    }
    home_dir().map(|home| home.join(".config/fast-explorer/config.json"))
}

fn parse_settings_text(text: &str) -> Option<AppSettings> {
    let mut settings = serde_json::from_str::<AppSettings>(text).ok()?;
    settings.theme.intensity = settings.theme.intensity.min(100);
    Some(settings)
}

fn load_settings() -> Option<AppSettings> {
    let path = config_path()?;

    #[cfg(target_os = "android")]
    if !android_config_migration_complete()
        && let Some(legacy) = legacy_android_config_path()
        && legacy != path
    {
        match fs::read_to_string(&legacy) {
            Ok(text) => {
                if let Some(settings) = parse_settings_text(&text) {
                    // A previous launch may have synthesized a private config while
                    // scoped-storage permission hid the legacy file. Preserve that
                    // private copy before making the now-readable legacy settings
                    // authoritative for the one-time migration.
                    backup_private_config_before_legacy_import(&path);
                    ANDROID_LEGACY_CONFIG_LOADED.store(true, Ordering::Release);
                    return Some(settings);
                }
                eprintln!(
                    "FastExplorer: legacy Android config {} is invalid; keeping private config",
                    legacy.display()
                );
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) => {}
            Err(error) => eprintln!(
                "FastExplorer: cannot read legacy Android config {}: {error}",
                legacy.display()
            ),
        }
    }

    let text = fs::read_to_string(path).ok()?;
    parse_settings_text(&text)
}

fn save_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "config path has no parent".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    if fs::rename(&temp, path).is_err() {
        let _ = fs::remove_file(path);
        fs::rename(&temp, path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn detect_system_dark() -> bool {
    if std::env::var("GTK_THEME")
        .ok()
        .is_some_and(|value| value.to_ascii_lowercase().contains("dark"))
    {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        for (program, args) in [
            (
                "xfconf-query",
                vec!["-c", "xsettings", "-p", "/Net/ThemeName"],
            ),
            (
                "gsettings",
                vec!["get", "org.gnome.desktop.interface", "color-scheme"],
            ),
        ] {
            if let Ok(output) = Command::new(program).args(args).output() {
                let value = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
                if value.contains("dark") {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(target_os = "android")]
pub(crate) fn home_dir() -> Option<PathBuf> {
    Some(
        ANDROID_HOME
            .get()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("/data/data/dev.oligami.fastexplorer/files")),
    )
}

#[cfg(target_os = "windows")]
pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

#[cfg(not(any(target_os = "android", target_os = "windows")))]
pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn default_sync_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "android") {
                "Android device".to_owned()
            } else {
                "FastExplorer device".to_owned()
            }
        })
}

fn single_line_input(value: String) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    normalized
        .split('\n')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_eta(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    if total < 60 {
        return format!("{total}s");
    }
    let minutes = total / 60;
    let seconds = total % 60;
    if minutes < 60 {
        return format!("{minutes}m {seconds:02}s");
    }
    let hours = minutes / 60;
    let minutes = minutes % 60;
    format!("{hours}h {minutes:02}m")
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sandbox() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("fast_explorer_{}_{}", std::process::id(), stamp));
        fs::create_dir_all(&path).expect("create sandbox");
        path
    }

    fn tab_at(path: PathBuf) -> TabState {
        TabState::from_path(path)
    }

    fn app_with_full_palette_around_ungrouped(root: PathBuf, ungrouped: usize) -> AppState {
        let mut app = app_at(root.clone());
        app.tabs.clear();
        app.tab_groups.clear();
        let left_colors = [
            TabGroupColor::Yellow,
            TabGroupColor::Green,
            TabGroupColor::Pink,
            TabGroupColor::Purple,
            TabGroupColor::Cyan,
            TabGroupColor::Orange,
            TabGroupColor::Blue,
        ];
        for (index, color) in left_colors.into_iter().enumerate() {
            let group_id = 10_000 + index as u64;
            let mut tab = tab_at(root.clone());
            tab.group_id = Some(group_id);
            app.tabs.push(tab);
            app.tab_groups.push(TabGroupState {
                id: group_id,
                name: String::new(),
                color,
                collapsed: false,
            });
        }
        for _ in 0..ungrouped {
            app.tabs.push(tab_at(root.clone()));
        }
        let red_group_id = 20_000;
        let mut red_tab = tab_at(root);
        red_tab.group_id = Some(red_group_id);
        app.tabs.push(red_tab);
        app.tab_groups.push(TabGroupState {
            id: red_group_id,
            name: String::new(),
            color: TabGroupColor::Red,
            collapsed: false,
        });
        app.active_tab = 0;
        app
    }

    #[test]
    fn legacy_grey_tab_group_color_migrates_to_blue() {
        let color: TabGroupColor = serde_json::from_str("\"grey\"").expect("legacy grey color");
        assert_eq!(color, TabGroupColor::Blue);
        assert_eq!(TabGroupColor::default(), TabGroupColor::Blue);
        assert_eq!(
            serde_json::to_string(&color).expect("serialize blue"),
            "\"blue\""
        );
        assert_eq!(TabGroupColor::ALL.len(), 8);
    }

    #[test]
    fn new_tab_groups_use_unused_colors_before_repeating() {
        let root = sandbox();
        let mut app = app_at(root.clone());
        for index in 1..TabGroupColor::ALL.len() {
            app.tabs.push(tab_at(root.join(format!("tab-{index}"))));
        }

        let tab_ids = app.tabs.iter().map(TabState::id).collect::<Vec<_>>();
        let mut colors = Vec::new();
        for tab_id in tab_ids {
            app.create_tab_group_for_tab(tab_id);
            let group_id = app
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(|tab| tab.group_id)
                .expect("new group id");
            let color = app.tab_group(group_id).expect("new group").color();
            assert!(!colors.contains(&color), "group color repeated too early");
            colors.push(color);
        }
        assert_eq!(colors.len(), TabGroupColor::ALL.len());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn automatic_single_tab_group_avoids_adjacent_group_colors() {
        let root = sandbox();
        let mut app = app_with_full_palette_around_ungrouped(root.clone(), 1);
        let target_index = app.tabs.len() - 2;
        let target_id = app.tabs[target_index].id();

        app.create_tab_group_for_tab(target_id);
        let group_id = app.tabs[target_index].group_id.expect("generated group");
        let color = app
            .tab_group(group_id)
            .expect("generated group state")
            .color();
        assert_ne!(color, TabGroupColor::Blue);
        assert_ne!(color, TabGroupColor::Red);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn drag_group_preview_and_commit_avoid_adjacent_group_colors() {
        let root = sandbox();
        let mut app = app_with_full_palette_around_ungrouped(root.clone(), 2);
        let source_index = app.tabs.len() - 3;
        let target_index = app.tabs.len() - 2;
        let source_id = app.tabs[source_index].id();
        let target_id = app.tabs[target_index].id();
        let preview_color = app.tab_group_color_for_drag(source_id, target_id);
        assert_ne!(preview_color, TabGroupColor::Blue);
        assert_ne!(preview_color, TabGroupColor::Red);

        app.create_tab_group_from_drag(source_id, target_id, 32.0);
        let group_id = app
            .tabs
            .iter()
            .find(|tab| tab.id == source_id)
            .and_then(TabState::group_id)
            .expect("drag-created group");
        assert_eq!(
            app.tab_group(group_id)
                .expect("drag-created group state")
                .color(),
            preview_color
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn group_toggle_scroll_anchor_rearms_and_explicit_tab_selection_releases_it() {
        let root = sandbox();
        let mut app = app_at(root.clone());
        app.tabs.push(tab_at(root.join("second")));
        let first_id = app.tabs[0].id();
        app.create_tab_group_for_tab(first_id);
        let group_id = app.tabs[0].group_id().expect("group id");

        app.toggle_tab_group_collapsed(group_id);
        let first_anchor = app.tab_group_scroll_anchor().expect("first anchor");
        assert_eq!(first_anchor.0, group_id);
        app.toggle_tab_group_collapsed(group_id);
        let second_anchor = app.tab_group_scroll_anchor().expect("second anchor");
        assert_eq!(second_anchor.0, group_id);
        assert_ne!(first_anchor.1, second_anchor.1);

        app.select_tab(1);
        assert_eq!(app.tab_group_scroll_anchor(), None);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn mobile_action_capacity_never_forces_overflow() {
        assert_eq!(mobile_primary_action_capacity_for_width(400.0, 0.0, 0.0), 7);
        assert_eq!(mobile_primary_action_capacity_for_width(360.0, 0.0, 0.0), 6);
        assert_eq!(mobile_primary_action_capacity_for_width(240.0, 0.0, 0.0), 3);
        assert_eq!(mobile_primary_action_capacity_for_width(206.0, 0.0, 0.0), 3);
        assert_eq!(mobile_primary_action_capacity_for_width(172.0, 0.0, 0.0), 2);
        assert_eq!(mobile_primary_action_capacity_for_width(70.0, 0.0, 0.0), 0);
        assert_eq!(
            mobile_primary_action_capacity_for_width(240.0, 20.0, 20.0),
            3
        );
    }

    fn app_at(path: PathBuf) -> AppState {
        AppState {
            tabs: vec![TabState::from_path(path)],
            active_tab: 0,
            tab_groups: Vec::new(),
            tab_context_menu_tab_id: None,
            tab_group_editor_id: None,
            tab_group_scroll_anchor_id: None,
            tab_group_scroll_anchor_epoch: 0,
            tab_overlay_anchor_x: 8.0,
            tab_strip_drag_preview: TabStripDragPreviewHandle::default(),
            tab_strip_drag_preview_active: false,
            page: AppPage::Files,
            settings_entry_point: SettingsEntryPoint::General,
            persistence_enabled: false,
            persistence_sender: None,
            theme_settings: ThemeSettings::default(),
            saved_theme_settings: ThemeSettings::default(),
            theme_overrides: ThemePatch::default(),
            search_mode: SearchMode::Default,
            saved_search_mode: SearchMode::Default,
            search_override: None,
            ui_font: UiFont::System,
            saved_ui_font: UiFont::System,
            remote_cache_settings: RemoteCacheSettings::default(),
            saved_remote_cache_settings: RemoteCacheSettings::default(),
            path_overflow_behavior: PathOverflowBehavior::default(),
            path_reset_delay_ms: 3000,
            remote_cache_usage_bytes: 0,
            pending_remote_cache_downloads: BTreeMap::new(),
            pending_upload_info: BTreeMap::new(),
            pending_temporary_uploads: BTreeSet::new(),
            file_clipboard: None,
            pending_delete_confirmation: None,
            pending_paste_conflict: None,
            paste_conflict_resolution: None,
            confirm_mobile_delete: true,
            delete_warning_suppressed_until_ms: 0,
            file_transfers: Vec::new(),
            transfer_popup_open: false,
            sort_popup_open: false,
            file_more_popup_open: false,
            system_dark: false,
            context_actions_visible: false,
            pinned_paths: Vec::new(),
            tailscale_profiles: Vec::new(),
            tailnet_path_drafts: BTreeMap::new(),
            taildrive_device_name_drafts: BTreeMap::new(),
            supabase: crate::supabase_sync::UiState::default(),
            tailscale_offline_peers_expanded: BTreeSet::new(),
            tailscale_offline_devices_expanded: BTreeSet::new(),
            tailscale_sender: None,
            directory_sender: None,
            directory_generation: 0,
            directory_request_started_at: None,
            archive_sender: None,
            archive_generation: 0,
            local_file_sender: None,
            cache_sender: None,
            remote_prepare_sender: None,
            remote_prepare_pending: BTreeSet::new(),
            remote_cache_usage_pending: None,
            remote_cache_usage_next_request_id: 0,
            remote_cache_usage_refresh_queued: false,
            thumbnail_sender: None,
            thumbnail_cache: BTreeMap::new(),
            thumbnail_pending: BTreeSet::new(),
            search_sender: None,
            search_generation: 0,
            taildrive_generation: 0,
            remote_mutations: Vec::new(),
            #[cfg(target_os = "android")]
            android_app: None,
            #[cfg(target_os = "android")]
            android_storage_access: false,
            #[cfg(target_os = "android")]
            android_insets: crate::android_platform::SystemBarInsets::default(),
            #[cfg(target_os = "android")]
            android_window_width_dp: 360.0,
        }
    }

    fn taildrive_test_location(remote_path: &str) -> TaildriveLocation {
        TaildriveLocation::Remote {
            profile_id: "profile-1".to_owned(),
            device_id: "device-1".to_owned(),
            share: "share".to_owned(),
            remote_path: remote_path.to_owned(),
        }
    }

    fn taildrive_app(
        remote_path: &str,
    ) -> (
        AppState,
        tokio::sync::mpsc::UnboundedReceiver<crate::tailscale::Command>,
    ) {
        let mut app = app_at(default_directory());
        app.tabs = vec![TabState::from_path(taildrive_path(
            &taildrive_test_location(remote_path),
        ))];
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        app.tailscale_sender = Some(sender);
        (app, receiver)
    }

    fn taildrive_entry(parent: &str, name: &str, kind: EntryKind) -> FileEntry {
        let location = taildrive_test_location(&remote_child_path(parent, name));
        FileEntry {
            path: taildrive_path(&location),
            name: name.to_owned(),
            kind,
            size: if kind == EntryKind::File { 7 } else { 0 },
            modified_sort_key: 0,
            remote: Some(location),
            remote_modified: Some("test-version".to_owned()),
        }
    }

    #[test]
    fn remote_cache_access_renews_sliding_expiration() {
        let root = sandbox();
        let path = root.join("cached.pdf");
        fs::write(&path, b"pdf").expect("cache file");
        let now = remote_cache_now_ms();
        let old_access = now.saturating_sub(60_000);
        let mut index = RemoteOpenCacheIndex {
            version: REMOTE_OPEN_CACHE_VERSION,
            entries: BTreeMap::new(),
        };
        index.entries.insert(
            "source".to_owned(),
            RemoteOpenCacheEntry {
                file_name: "cached.pdf".to_owned(),
                display_name: "cached.pdf".to_owned(),
                remote_size: 3,
                remote_modified: "v1".to_owned(),
                cached_unix_ms: old_access,
                last_accessed_unix_ms: old_access,
            },
        );
        save_remote_cache_index(&root, &index).expect("save index");

        let hit =
            remote_cache_try_acquire(&root, "source", 3, "v1", RemoteCacheSettings::default())
                .expect("cache lookup");
        assert_eq!(hit, Some(path));
        let renewed = load_remote_cache_index(&root);
        assert!(renewed.entries["source"].last_accessed_unix_ms > old_access);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn remote_cache_rejects_changed_remote_version() {
        let root = sandbox();
        let path = root.join("cached.pdf");
        fs::write(&path, b"pdf").expect("cache file");
        let now = remote_cache_now_ms();
        let mut index = RemoteOpenCacheIndex {
            version: REMOTE_OPEN_CACHE_VERSION,
            entries: BTreeMap::new(),
        };
        index.entries.insert(
            "source".to_owned(),
            RemoteOpenCacheEntry {
                file_name: "cached.pdf".to_owned(),
                display_name: "cached.pdf".to_owned(),
                remote_size: 3,
                remote_modified: "v1".to_owned(),
                cached_unix_ms: now,
                last_accessed_unix_ms: now,
            },
        );
        save_remote_cache_index(&root, &index).expect("save index");

        let hit =
            remote_cache_try_acquire(&root, "source", 3, "v2", RemoteCacheSettings::default())
                .expect("cache lookup");
        assert!(hit.is_none());
        assert!(path.exists());
        assert!(
            !load_remote_cache_index(&root)
                .entries
                .contains_key("source")
        );
        remote_cache_cleanup(&root, RemoteCacheSettings::default()).expect("orphan cleanup");
        assert!(!path.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn remote_cache_does_not_reuse_entry_without_remote_modified_time() {
        let root = sandbox();
        let path = root.join("cached.pdf");
        fs::write(&path, b"pdf").expect("cache file");
        let now = remote_cache_now_ms();
        let mut index = RemoteOpenCacheIndex {
            version: REMOTE_OPEN_CACHE_VERSION,
            entries: BTreeMap::new(),
        };
        index.entries.insert(
            "source".to_owned(),
            RemoteOpenCacheEntry {
                file_name: "cached.pdf".to_owned(),
                display_name: "cached.pdf".to_owned(),
                remote_size: 3,
                remote_modified: String::new(),
                cached_unix_ms: now,
                last_accessed_unix_ms: now,
            },
        );
        save_remote_cache_index(&root, &index).expect("save index");

        let hit = remote_cache_try_acquire(&root, "source", 3, "", RemoteCacheSettings::default())
            .expect("cache lookup");
        assert!(hit.is_none());
        assert!(path.exists());
        assert!(
            load_remote_cache_index(&root)
                .entries
                .contains_key("source")
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn remote_cache_limit_evicts_least_recently_used_file() {
        let root = sandbox();
        let old_path = root.join("old.bin");
        let new_path = root.join("new.bin");
        fs::write(&old_path, vec![0_u8; 700 * 1024]).expect("old cache");
        fs::write(&new_path, vec![0_u8; 700 * 1024]).expect("new cache");
        let now_time = SystemTime::now();
        fs::OpenOptions::new()
            .write(true)
            .open(&old_path)
            .expect("open old cache")
            .set_times(fs::FileTimes::new().set_modified(now_time - Duration::from_secs(20)))
            .expect("set old cache time");
        fs::OpenOptions::new()
            .write(true)
            .open(&new_path)
            .expect("open new cache")
            .set_times(fs::FileTimes::new().set_modified(now_time - Duration::from_secs(10)))
            .expect("set new cache time");
        let now = remote_cache_now_ms();
        let mut index = RemoteOpenCacheIndex {
            version: REMOTE_OPEN_CACHE_VERSION,
            entries: BTreeMap::new(),
        };
        for (key, file_name, accessed) in [
            ("old", "old.bin", now.saturating_sub(2_000)),
            ("new", "new.bin", now.saturating_sub(1_000)),
        ] {
            index.entries.insert(
                key.to_owned(),
                RemoteOpenCacheEntry {
                    file_name: file_name.to_owned(),
                    display_name: file_name.to_owned(),
                    remote_size: 700 * 1024,
                    remote_modified: "v1".to_owned(),
                    cached_unix_ms: accessed,
                    last_accessed_unix_ms: accessed,
                },
            );
        }
        save_remote_cache_index(&root, &index).expect("save index");

        let usage = remote_cache_cleanup(
            &root,
            RemoteCacheSettings {
                limit_mib: 1,
                expiration_hours: 720,
            },
        )
        .expect("cache cleanup");
        assert!(usage <= 1024 * 1024);
        assert!(!old_path.exists());
        assert!(new_path.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn cache_usage_loading_ignores_unrelated_and_stale_cache_events() {
        let root = sandbox();
        let mut app = app_at(root.clone());
        app.remote_cache_usage_pending = Some(2);

        app.apply_cache_event(CacheEvent {
            result: Ok(42),
            usage_refresh_id: None,
        });
        assert_eq!(app.remote_cache_usage_pending, Some(2));
        assert!(app.remote_cache_usage_label().starts_with("Calculating"));

        app.apply_cache_event(CacheEvent {
            result: Ok(63),
            usage_refresh_id: Some(1),
        });
        assert_eq!(app.remote_cache_usage_pending, Some(2));
        assert!(app.remote_cache_usage_label().starts_with("Calculating"));

        app.apply_cache_event(CacheEvent {
            result: Ok(84),
            usage_refresh_id: Some(2),
        });
        assert_eq!(app.remote_cache_usage_pending, None);
        assert_eq!(app.remote_cache_usage_bytes, 84);
        assert!(!app.remote_cache_usage_label().starts_with("Calculating"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn opening_settings_supersedes_a_stuck_cache_usage_request() {
        let root = sandbox();
        let mut app = app_at(root.clone());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        app.cache_sender = Some(sender);
        app.remote_cache_usage_pending = Some(999);

        app.open_settings();

        let command = receiver.try_recv().expect("forced usage refresh");
        let request_id = match command {
            CacheCommand::Maintain {
                usage_refresh_id, ..
            } => usage_refresh_id,
            other => panic!("expected maintenance request, got {other:?}"),
        };
        assert_ne!(request_id, 999);
        assert_eq!(app.remote_cache_usage_pending, Some(request_id));
        assert!(app.remote_cache_usage_label().starts_with("Calculating"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn settings_entry_points_keep_context_for_help_and_device_sync() {
        let root = sandbox();
        let mut app = app_at(root.clone());

        app.open_settings();
        assert_eq!(app.settings_entry_point(), SettingsEntryPoint::General);
        app.open_action_guide();
        assert_eq!(app.settings_entry_point(), SettingsEntryPoint::ActionGuide);
        app.open_device_sync_settings();
        assert_eq!(app.settings_entry_point(), SettingsEntryPoint::DeviceSync);
        assert_eq!(app.page(), AppPage::Settings);

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn remote_cache_cleanup_preserves_in_progress_file() {
        let root = sandbox();
        let pending = root.join("pending.pdf");
        fs::write(&pending, b"partial").expect("pending cache file");
        let protected = BTreeSet::from(["pending.pdf".to_owned()]);

        remote_cache_cleanup_with_protected(&root, RemoteCacheSettings::default(), &protected)
            .expect("protected cleanup");
        assert!(pending.exists());

        remote_cache_clear(&root, &protected).expect("protected clear");
        assert!(pending.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn remote_cache_protected_indexed_file_survives_expiration_lru_and_clear() {
        let root = sandbox();
        let protected_path = root.join("viewer.pdf");
        let other_path = root.join("other.bin");
        fs::write(&protected_path, vec![0_u8; 700 * 1024]).expect("protected file");
        fs::write(&other_path, vec![0_u8; 700 * 1024]).expect("other file");
        let now = remote_cache_now_ms();
        let old = now.saturating_sub(3 * 60 * 60 * 1000);
        let mut index = RemoteOpenCacheIndex {
            version: REMOTE_OPEN_CACHE_VERSION,
            entries: BTreeMap::new(),
        };
        for (key, file_name) in [("viewer", "viewer.pdf"), ("other", "other.bin")] {
            index.entries.insert(
                key.to_owned(),
                RemoteOpenCacheEntry {
                    file_name: file_name.to_owned(),
                    display_name: file_name.to_owned(),
                    remote_size: 700 * 1024,
                    remote_modified: "v1".to_owned(),
                    cached_unix_ms: old,
                    last_accessed_unix_ms: old,
                },
            );
        }
        save_remote_cache_index(&root, &index).expect("save index");
        let protected = BTreeSet::from(["viewer.pdf".to_owned()]);

        remote_cache_cleanup_with_protected(
            &root,
            RemoteCacheSettings {
                limit_mib: 1,
                expiration_hours: 1,
            },
            &protected,
        )
        .expect("protected cleanup");
        assert!(protected_path.exists());
        assert!(
            load_remote_cache_index(&root)
                .entries
                .contains_key("viewer")
        );

        let replacement = root.join("viewer-new.pdf");
        fs::write(&replacement, vec![1_u8; 700 * 1024]).expect("replacement file");
        remote_cache_record(
            &root,
            "viewer".to_owned(),
            &replacement,
            "viewer.pdf".to_owned(),
            700 * 1024,
            "v2".to_owned(),
            RemoteCacheSettings {
                limit_mib: 1,
                expiration_hours: 1,
            },
            &protected,
        )
        .expect("protected replacement");
        assert!(protected_path.exists());
        assert!(replacement.exists());

        remote_cache_clear(&root, &protected).expect("protected clear");
        assert!(protected_path.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn transient_remote_cache_setting_does_not_change_saved_setting() {
        let root = sandbox();
        let mut app = app_at(root.clone());
        let saved = app.saved_remote_cache_settings();
        app.set_remote_cache_settings(
            RemoteCacheSettings {
                limit_mib: 2048,
                expiration_hours: 72,
            },
            false,
        );
        assert_eq!(app.remote_cache_settings().limit_mib, 2048);
        assert_eq!(app.remote_cache_settings().expiration_hours, 72);
        assert_eq!(app.saved_remote_cache_settings(), saved);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn tailnet_profiles_are_independent_and_removed_ids_are_not_reused() {
        let root = sandbox();
        let mut app = app_at(root.clone());
        app.add_tailnet_profile();
        app.add_tailnet_profile();
        assert_eq!(app.tailscale_profiles.len(), 2);
        assert_ne!(
            app.tailscale_profiles[0].config.id,
            app.tailscale_profiles[1].config.id
        );
        assert_eq!(app.tailscale_profiles[0].config.label, "TN1");
        assert_eq!(app.tailscale_profiles[1].config.label, "TN2");
        let removed = app.tailscale_profiles[0].config.id.clone();
        app.remove_tailnet_profile(&removed);
        app.add_tailnet_profile();
        assert!(
            !app.tailscale_profiles
                .iter()
                .any(|profile| profile.config.id == removed)
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn listing_sorts_directories_and_hides_dotfiles() {
        let root = sandbox();
        fs::create_dir(root.join("alpha_dir")).expect("dir");
        fs::write(root.join("zeta.txt"), b"hello").expect("file");
        fs::write(root.join(".hidden"), b"secret").expect("hidden");
        let mut tab = tab_at(root.clone());
        let names = tab
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["alpha_dir", "zeta.txt"]);

        tab.show_hidden = true;
        tab.reload_current();
        assert!(tab.entries.iter().any(|entry| entry.name == ".hidden"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn entry_click_selects_then_double_click_opens() {
        let root = sandbox();
        let child = root.join("child");
        fs::create_dir(&child).expect("child dir");
        let mut tab = tab_at(root.clone());

        tab.click_entry(child.clone());
        assert_eq!(tab.current_dir, root);
        assert_eq!(tab.selected_path.as_ref(), Some(&child));

        tab.click_entry(child.clone());
        assert_eq!(tab.current_dir, child);
        assert!(tab.selected_path.is_none());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn stale_same_query_search_result_is_rejected_after_refresh() {
        let root = sandbox();
        let mut app = app_at(root.clone());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        app.set_search_sender(sender);

        app.submit_search("needle".to_owned());
        let first = receiver.try_recv().expect("first search request");
        app.refresh();
        let second = receiver.try_recv().expect("refreshed search request");

        assert_ne!(first.0, second.0);
        assert!(!app.accepts_search_result(first.0, &first.1, &first.2));
        assert!(app.accepts_search_result(second.0, &second.1, &second.2));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn touch_entry_opens_directories_but_only_selects_files() {
        let root = sandbox();
        let child = root.join("child");
        let file = root.join("note.txt");
        fs::create_dir(&child).expect("child dir");
        fs::write(&file, b"note").expect("file");
        let mut app = app_at(root.clone());

        app.touch_entry(file.clone());
        assert_eq!(app.active_tab().current_dir, root);
        assert_eq!(app.active_tab().selected_path.as_ref(), Some(&file));

        app.touch_entry(child.clone());
        assert_eq!(app.active_tab().current_dir, child);
        assert!(app.active_tab().selected_path.is_none());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn keyboard_selection_moves_and_clamps() {
        let root = sandbox();
        fs::write(root.join("a.txt"), b"a").expect("a");
        fs::write(root.join("b.txt"), b"b").expect("b");
        fs::write(root.join("c.txt"), b"c").expect("c");
        let mut app = app_at(root.clone());

        assert_eq!(app.move_selection(1), Some(0));
        assert_eq!(app.selected_entry_index(), Some(0));
        assert_eq!(app.move_selection(1), Some(1));
        assert_eq!(app.move_selection(100), Some(2));
        assert_eq!(app.move_selection(-100), Some(0));
        assert_eq!(app.select_last_entry(), Some(2));
        assert_eq!(app.select_first_entry(), Some(0));

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn keyboard_activation_opens_selected_directory() {
        let root = sandbox();
        let child = root.join("child");
        fs::create_dir(&child).expect("child dir");
        fs::write(root.join("file.txt"), b"file").expect("file");
        let mut app = app_at(root.clone());

        assert_eq!(app.select_first_entry(), Some(0));
        assert_eq!(app.active_tab().selected_path.as_ref(), Some(&child));
        app.activate_selected();
        assert_eq!(app.active_tab().current_dir, child);
        assert!(app.active_tab().selected_path.is_none());

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn mobile_ime_submit_finishes_new_folder_name() {
        let temp = sandbox();
        let mut app = app_at(temp.clone());
        app.new_folder();
        assert!(app.rename_active());

        app.submit_rename("Phone folder\n".to_owned());

        assert!(!app.rename_active());
        assert!(temp.join("Phone folder").is_dir());
        assert!(!temp.join("Phone folder\n").exists());
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn multiline_rename_change_is_single_line_without_auto_submit() {
        let temp = sandbox();
        let mut app = app_at(temp.clone());
        app.new_folder();
        assert!(app.rename_active());

        app.set_rename_input("Pasted\nname".to_owned());

        assert!(app.rename_active());
        assert_eq!(
            app.active_tab().rename_input.as_deref(),
            Some("Pasted name")
        );
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn keyboard_rename_replaces_then_appends_text() {
        let root = sandbox();
        let source = root.join("source.txt");
        fs::write(&source, b"file").expect("file");
        let mut app = app_at(root.clone());

        app.active_tab_mut().select_entry(source.clone());
        app.begin_rename();
        app.type_rename_text("renamed".to_owned());
        assert_eq!(
            app.active_tab().rename_input.as_deref(),
            Some("renamed.txt")
        );
        app.type_rename_text("2".to_owned());
        assert_eq!(
            app.active_tab().rename_input.as_deref(),
            Some("renamed2.txt")
        );
        app.apply_rename();

        let renamed = root.join("renamed2.txt");
        assert!(renamed.exists());
        assert!(!source.exists());
        assert!(!app.rename_active());

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn common_file_actions_create_copy_move_and_rename() {
        let root = sandbox();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source");
        fs::create_dir_all(&target_dir).expect("target");
        let original = source_dir.join("item.txt");
        fs::write(&original, b"hello").expect("source file");

        let mut app = app_at(source_dir.clone());

        app.active_tab_mut().select_entry(original.clone());
        app.copy_selected();
        app.navigate_to(target_dir.clone());
        app.paste();
        let copied = target_dir.join("item.txt");
        assert_eq!(fs::read(&copied).expect("copied bytes"), b"hello");

        app.active_tab_mut().select_entry(copied.clone());
        app.begin_rename();
        app.submit_rename("renamed.txt".to_owned());
        let renamed = target_dir.join("renamed.txt");
        assert!(renamed.exists());

        app.active_tab_mut().select_entry(renamed.clone());
        app.cut_selected();
        app.navigate_to(source_dir.clone());
        app.paste();
        assert!(source_dir.join("renamed.txt").exists());
        assert!(!renamed.exists());

        app.new_folder();
        assert!(source_dir.join("New folder").is_dir());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn paste_conflict_offers_replace_or_keep_both_like_explorer() {
        let root = sandbox();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source");
        fs::create_dir_all(&target_dir).expect("target");
        let source = source_dir.join("item.txt");
        let existing = target_dir.join("item.txt");
        fs::write(&source, b"source").expect("source file");
        fs::write(&existing, b"existing").expect("existing file");

        let mut app = app_at(source_dir.clone());
        app.active_tab_mut().select_entry(source.clone());
        app.copy_selected();
        app.navigate_to(target_dir.clone());
        app.paste();
        assert_eq!(app.paste_conflict_name(), Some("item.txt"));
        assert_eq!(fs::read(&existing).expect("unchanged"), b"existing");

        app.replace_paste_conflict();
        assert_eq!(fs::read(&existing).expect("replaced"), b"source");

        fs::write(&existing, b"existing-again").expect("reset existing");
        app.navigate_to(source_dir.clone());
        app.active_tab_mut().select_entry(source.clone());
        app.copy_selected();
        app.navigate_to(target_dir.clone());
        app.paste();
        assert_eq!(app.paste_conflict_name(), Some("item.txt"));
        app.keep_both_paste_conflict();
        assert_eq!(
            fs::read(&existing).expect("kept original"),
            b"existing-again"
        );
        assert_eq!(
            fs::read(target_dir.join("item - Copy.txt")).expect("kept copy"),
            b"source"
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn file_mutations_keep_search_consistent() {
        let root = sandbox();
        let report = root.join("report.txt");
        fs::write(&report, b"report").expect("report");
        fs::write(root.join("other.txt"), b"other").expect("other");
        let mut app = app_at(root.clone());

        app.submit_search("report".to_owned());
        assert!(app.active_tab().search_active);
        app.active_tab_mut().select_entry(report.clone());
        app.copy_selected();
        app.paste();
        assert!(app.active_tab().search_active);
        assert!(
            app.active_tab()
                .entries
                .iter()
                .all(|entry| entry.name.contains("report"))
        );

        app.new_folder();
        assert!(!app.active_tab().search_active);
        assert!(app.rename_active());
        assert!(root.join("New folder").is_dir());
        app.cancel_rename();

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn cut_paste_in_same_folder_is_noop() {
        let root = sandbox();
        let source = root.join("item.txt");
        fs::write(&source, b"item").expect("item");
        let mut app = app_at(root.clone());

        app.active_tab_mut().select_entry(source.clone());
        app.cut_selected();
        app.paste();

        assert!(source.exists());
        assert!(!root.join("item - Copy.txt").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn tilde_slash_address_expands_to_home() {
        let root = sandbox();
        let mut tab = tab_at(root.clone());
        let home = home_dir().expect("home directory");

        tab.submit_address("~/".to_owned());
        assert_eq!(tab.current_dir, home);

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn navigation_history_round_trips() {
        let root = sandbox();
        let first = root.join("first");
        let second = first.join("second");
        fs::create_dir_all(&second).expect("nested dirs");

        let mut tab = tab_at(root.clone());
        tab.navigate_to(first.clone());
        tab.navigate_to(second.clone());
        tab.go_back();
        assert_eq!(tab.current_dir, first);
        tab.go_forward();
        assert_eq!(tab.current_dir, second);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn system_back_consumes_settings_and_history_before_exit() {
        let root = sandbox();
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("nested");
        let mut app = app_at(root.clone());

        app.pending_delete_confirmation = Some(PendingDeleteConfirmation {
            path: root.join("delete.txt"),
            name: "delete.txt".to_owned(),
        });
        assert!(!app.handle_system_back());
        assert!(app.pending_delete_confirmation.is_none());

        app.pending_paste_conflict = Some(PendingPasteConflict {
            clipboard: FileClipboard {
                path: root.join("copy.txt"),
                name: "copy.txt".to_owned(),
                kind: EntryKind::File,
                size: 0,
                remote_modified: None,
                mode: ClipboardMode::Copy,
            },
            target_location: root.clone(),
        });
        assert!(!app.handle_system_back());
        assert!(app.pending_paste_conflict.is_none());

        app.begin_file_transfer("back-test".to_owned(), "file.bin".to_owned(), "Copying");
        app.toggle_transfer_popup();
        assert!(app.transfer_popup_open());
        assert!(!app.handle_system_back());
        assert!(!app.transfer_popup_open());

        app.toggle_file_more_popup();
        assert!(app.file_more_popup_open());
        app.open_sort_popup();
        assert!(app.sort_popup_open());
        assert!(!app.file_more_popup_open());
        assert!(!app.handle_system_back());
        assert!(!app.sort_popup_open());

        app.toggle_file_more_popup();
        assert!(app.file_more_popup_open());
        assert!(!app.handle_system_back());
        assert!(!app.file_more_popup_open());

        let tab_id = app.active_tab().id();
        app.open_tab_context_menu(tab_id);
        assert_eq!(app.tab_context_menu_tab_id(), Some(tab_id));
        assert!(!app.handle_system_back());
        assert!(app.tab_context_menu_tab_id().is_none());

        app.open_settings();
        assert!(!app.handle_system_back());
        assert_eq!(app.page(), AppPage::Files);
        app.navigate_to(nested);
        assert!(!app.handle_system_back());
        assert_eq!(app.active_tab().current_dir, root);
        assert!(app.handle_system_back());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn new_tab_requests_async_directory_load() {
        let root = sandbox();
        let first = root.join("first");
        fs::create_dir_all(&first).expect("first");

        let mut app = app_at(first);
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        app.set_directory_sender(sender);
        receiver.try_recv().expect("initial directory request");

        app.new_tab();

        let (_, requested_dir, _) = receiver.try_recv().expect("new tab directory request");
        assert_eq!(requested_dir, app.active_tab().current_dir);
        assert_eq!(app.active_tab().status, "Loading folder…");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn tab_reorder_by_id_preserves_active_tab_identity() {
        let root = sandbox();
        let first = root.join("first");
        let second = root.join("second");
        let third = root.join("third");
        fs::create_dir_all(&first).expect("first");
        fs::create_dir_all(&second).expect("second");
        fs::create_dir_all(&third).expect("third");

        let mut app = app_at(first.clone());
        app.tabs.push(tab_at(second.clone()));
        app.tabs.push(tab_at(third.clone()));
        app.active_tab = 1;
        let first_id = app.tabs[0].id();

        app.move_tab_to_index(first_id, 1);
        assert_eq!(app.tabs[1].current_dir, first);
        assert_eq!(app.active_tab().current_dir, second);
        assert_eq!(app.active_tab_index(), 0);

        app.move_tab_to_index(first_id, 2);
        assert_eq!(app.tabs[2].current_dir, first);
        assert_eq!(app.active_tab().current_dir, second);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn tab_drag_preview_preserves_first_targets_until_preview_ends() {
        let root = sandbox();
        fs::create_dir_all(root.join("preview")).expect("preview fixture");
        let mut app = app_at(root.join("preview"));
        let preview = app.tab_strip_drag_preview_handle();

        preview.set_targets(vec![24.0, -80.0], vec![true, false]);
        preview.set_group_candidate(Some(1), Some(Color::BLACK));
        app.begin_tab_strip_drag_preview();
        assert!(app.tab_strip_drag_preview_active());
        assert_eq!(preview.target_for_slot(0), (24.0, true));
        assert_eq!(preview.target_for_slot(1), (-80.0, false));
        assert_eq!(preview.group_candidate_slot(), Some(1));
        assert_eq!(preview.group_candidate_border(), Some(Color::BLACK));

        app.end_tab_strip_drag_preview();
        assert!(!app.tab_strip_drag_preview_active());
        assert_eq!(preview.target_for_slot(0), (0.0, false));
        assert_eq!(preview.group_candidate_slot(), None);
        assert_eq!(preview.group_candidate_border(), None);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tab_group_membership_stays_contiguous_and_empty_groups_are_removed() {
        let root = sandbox();
        let first = root.join("first");
        let second = root.join("second");
        let third = root.join("third");
        let fourth = root.join("fourth");
        fs::create_dir_all(&first).expect("first");
        fs::create_dir_all(&second).expect("second");
        fs::create_dir_all(&third).expect("third");
        fs::create_dir_all(&fourth).expect("fourth");

        let mut app = app_at(first);
        app.tabs.push(tab_at(second));
        app.tabs.push(tab_at(third));
        app.tabs.push(tab_at(fourth));
        let first_id = app.tabs[0].id();
        let second_id = app.tabs[1].id();
        let third_id = app.tabs[2].id();

        app.create_tab_group_for_tab(first_id);
        let group_id = app.tabs[0].group_id().expect("group id");
        app.assign_tab_to_group(second_id, group_id);
        app.assign_tab_to_group(third_id, group_id);
        assert!(
            app.tabs[..3]
                .iter()
                .all(|tab| tab.group_id() == Some(group_id))
        );

        // Removing a middle member must move it outside the group instead of
        // splitting the remaining group into two visual islands.
        app.remove_tab_from_group(second_id);
        assert_eq!(app.tabs[0].group_id(), Some(group_id));
        assert_eq!(app.tabs[1].group_id(), Some(group_id));
        assert_eq!(app.tabs[2].id(), second_id);
        assert_eq!(app.tabs[2].group_id(), None);

        app.move_tab_to_index(first_id, 3);
        assert_eq!(app.tabs[0].group_id(), Some(group_id));
        assert_eq!(app.tabs[1].group_id(), Some(group_id));
        assert!(app.tabs[2..].iter().all(|tab| tab.group_id().is_none()));

        app.remove_tab_from_group(first_id);
        app.remove_tab_from_group(third_id);
        assert!(app.tab_group(group_id).is_none());
        assert!(app.tabs().iter().all(|tab| tab.group_id().is_none()));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn drag_overlap_can_create_a_two_tab_group_without_reordering_other_tabs() {
        let root = sandbox();
        for name in ["one", "two", "three", "four"] {
            fs::create_dir_all(root.join(name)).expect("tab fixture");
        }
        let mut app = app_at(root.join("one"));
        app.tabs.push(tab_at(root.join("two")));
        app.tabs.push(tab_at(root.join("three")));
        app.tabs.push(tab_at(root.join("four")));
        let ids = app.tabs.iter().map(TabState::id).collect::<Vec<_>>();

        app.create_tab_group_from_drag(ids[0], ids[2], 240.0);

        let source_pos = app.tabs.iter().position(|tab| tab.id == ids[0]).unwrap();
        let target_pos = app.tabs.iter().position(|tab| tab.id == ids[2]).unwrap();
        let group_id = app.tabs[source_pos].group_id.expect("new drag group");
        assert_eq!(app.tabs[target_pos].group_id, Some(group_id));
        assert_eq!(target_pos, source_pos + 1);
        assert_eq!(
            app.tabs
                .iter()
                .find(|tab| tab.id == ids[1])
                .unwrap()
                .group_id,
            None
        );
        assert_eq!(
            app.tabs
                .iter()
                .find(|tab| tab.id == ids[3])
                .unwrap()
                .group_id,
            None
        );
        assert_eq!(app.tab_group(group_id).unwrap().name(), "");
        assert_eq!(app.tab_group_editor_id(), Some(group_id));
        assert_eq!(app.tab_overlay_anchor_x(), 240.0);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tab_drag_can_enter_leave_and_cross_groups_without_splitting_them() {
        let root = sandbox();
        for name in ["one", "two", "three", "four", "five"] {
            fs::create_dir_all(root.join(name)).expect("tab fixture");
        }
        let mut app = app_at(root.join("one"));
        app.tabs.push(tab_at(root.join("two")));
        app.tabs.push(tab_at(root.join("three")));
        app.tabs.push(tab_at(root.join("four")));
        app.tabs.push(tab_at(root.join("five")));
        let ids = app.tabs.iter().map(TabState::id).collect::<Vec<_>>();

        app.create_tab_group_for_tab(ids[1]);
        let group_a = app
            .tabs
            .iter()
            .find(|tab| tab.id == ids[1])
            .unwrap()
            .group_id
            .unwrap();
        app.assign_tab_to_group(ids[2], group_a);
        app.create_tab_group_for_tab(ids[3]);
        let group_b = app
            .tabs
            .iter()
            .find(|tab| tab.id == ids[3])
            .unwrap()
            .group_id
            .unwrap();

        // Ungrouped -> group A.
        app.drop_tab_at(ids[0], 1, Some(group_a), true);
        assert!(
            app.tabs
                .iter()
                .filter(|tab| tab.group_id == Some(group_a))
                .count()
                == 3
        );

        // Group A -> group B.
        app.drop_tab_at(ids[2], 3, Some(group_b), true);
        assert_eq!(
            app.tabs
                .iter()
                .find(|tab| tab.id == ids[2])
                .unwrap()
                .group_id,
            Some(group_b)
        );

        // Pull a member out again. Remaining members of both groups must stay contiguous.
        let from = app.tabs.iter().position(|tab| tab.id == ids[1]).unwrap();
        app.drop_tab_at(ids[1], from, None, false);
        for group_id in [group_a, group_b] {
            let positions = app
                .tabs
                .iter()
                .enumerate()
                .filter_map(|(index, tab)| (tab.group_id == Some(group_id)).then_some(index))
                .collect::<Vec<_>>();
            if let (Some(first), Some(last)) = (positions.first(), positions.last()) {
                assert_eq!(positions.len(), last - first + 1);
            }
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn moving_a_tab_group_keeps_both_groups_contiguous_and_internal_order_stable() {
        let root = sandbox();
        for name in ["one", "two", "three", "four", "five"] {
            fs::create_dir_all(root.join(name)).expect("tab fixture");
        }
        let mut app = app_at(root.join("one"));
        app.tabs.push(tab_at(root.join("two")));
        app.tabs.push(tab_at(root.join("three")));
        app.tabs.push(tab_at(root.join("four")));
        app.tabs.push(tab_at(root.join("five")));
        let ids = app.tabs.iter().map(TabState::id).collect::<Vec<_>>();

        app.create_tab_group_for_tab(ids[1]);
        let group_a = app
            .tabs
            .iter()
            .find(|tab| tab.id == ids[1])
            .unwrap()
            .group_id
            .unwrap();
        app.assign_tab_to_group(ids[2], group_a);
        app.create_tab_group_for_tab(ids[3]);
        let group_b = app
            .tabs
            .iter()
            .find(|tab| tab.id == ids[3])
            .unwrap()
            .group_id
            .unwrap();
        app.assign_tab_to_group(ids[4], group_b);

        let group_a_before = app
            .tabs
            .iter()
            .filter(|tab| tab.group_id == Some(group_a))
            .map(TabState::id)
            .collect::<Vec<_>>();
        let group_b_before = app
            .tabs
            .iter()
            .filter(|tab| tab.group_id == Some(group_b))
            .map(TabState::id)
            .collect::<Vec<_>>();

        app.move_tab_group_near(group_a, ids[3], true);

        let group_a_after = app
            .tabs
            .iter()
            .filter(|tab| tab.group_id == Some(group_a))
            .map(TabState::id)
            .collect::<Vec<_>>();
        let group_b_after = app
            .tabs
            .iter()
            .filter(|tab| tab.group_id == Some(group_b))
            .map(TabState::id)
            .collect::<Vec<_>>();
        assert_eq!(group_a_after, group_a_before);
        assert_eq!(group_b_after, group_b_before);

        let a_first = app
            .tabs
            .iter()
            .position(|tab| tab.group_id == Some(group_a))
            .unwrap();
        let b_last = app
            .tabs
            .iter()
            .rposition(|tab| tab.group_id == Some(group_b))
            .unwrap();
        assert_eq!(a_first, b_last + 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tab_add_switch_and_close_preserves_state() {
        let root = sandbox();
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).expect("first");
        fs::create_dir_all(&second).expect("second");

        let mut app = app_at(first.clone());
        app.new_tab();
        assert_eq!(app.tab_count(), 2);
        assert_eq!(app.active_tab_index(), 1);
        app.navigate_to(second.clone());
        assert_eq!(app.active_tab().current_dir, second);
        app.select_tab(0);
        assert_eq!(app.active_tab().current_dir, first);
        app.select_tab(1);

        assert!(!app.close_tab(0));
        assert_eq!(app.tab_count(), 1);
        assert_eq!(app.active_tab().current_dir, second);
        assert!(app.close_tab(0));
        assert_eq!(app.tab_count(), 1);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn closing_group_with_all_tabs_requests_app_exit() {
        let root = sandbox();
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).expect("first");
        fs::create_dir_all(&second).expect("second");

        let mut app = app_at(first);
        app.tabs.push(tab_at(second));
        let first_id = app.tabs[0].id();
        let second_id = app.tabs[1].id();
        app.create_tab_group_for_tab(first_id);
        let group_id = app.tabs[0].group_id().expect("group id");
        app.assign_tab_to_group(second_id, group_id);

        assert!(app.close_tab_group(group_id));
        assert_eq!(app.tab_count(), 2);
        assert!(
            app.tabs()
                .iter()
                .all(|tab| tab.group_id() == Some(group_id))
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn session_round_trip_restores_tabs_history_and_active_tab() {
        let root = sandbox();
        let first = root.join("first");
        let second = root.join("second");
        let nested = second.join("nested");
        fs::create_dir_all(&first).expect("first");
        fs::create_dir_all(&nested).expect("nested");

        let session_file = root.join("state/session.json");
        let mut first_tab = TabState::from_path(first.clone());
        first_tab.show_hidden = true;
        first_tab.sort_field = SortField::Size;
        first_tab.sort_direction = SortDirection::Descending;
        let mut second_tab = TabState::from_path(second.clone());
        second_tab.sort_field = SortField::DateModified;
        second_tab.sort_direction = SortDirection::Ascending;
        second_tab.navigate_to(nested.clone());
        let mut app = app_at(first.clone());
        app.tabs = vec![first_tab, second_tab];
        app.active_tab = 1;
        let first_id = app.tabs[0].id();
        let second_id = app.tabs[1].id();
        app.create_tab_group_for_tab(first_id);
        let group_id = app.tabs[0].group_id().expect("group id");
        app.assign_tab_to_group(second_id, group_id);
        app.set_tab_group_name(group_id, "Research".to_owned());
        app.set_tab_group_color(group_id, TabGroupColor::Purple);
        app.toggle_tab_group_collapsed(group_id);
        app.save_session_to(&session_file).expect("save session");

        let restored = AppState::load_session_from(&session_file).expect("restore session");
        assert_eq!(restored.tab_count(), 2);
        assert_eq!(restored.active_tab_index(), 1);
        assert!(restored.tabs()[0].show_hidden);
        assert_eq!(restored.tabs()[0].sort_field, SortField::Size);
        assert_eq!(restored.tabs()[0].sort_direction, SortDirection::Descending);
        assert_eq!(restored.tabs()[0].current_dir, first);
        assert_eq!(restored.active_tab().sort_field, SortField::DateModified);
        assert_eq!(
            restored.active_tab().sort_direction,
            SortDirection::Ascending
        );
        assert_eq!(restored.active_tab().current_dir, nested);
        let restored_group = restored.tab_group(group_id).expect("restored group");
        assert_eq!(restored_group.name(), "Research");
        assert_eq!(restored_group.color(), TabGroupColor::Purple);
        assert!(restored_group.collapsed());
        assert!(
            restored
                .tabs()
                .iter()
                .all(|tab| tab.group_id() == Some(group_id))
        );
        assert!(restored.can_go_back());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn settings_page_is_transient_and_tab_actions_return_to_files() {
        let root = sandbox();
        let first = root.join("first");
        fs::create_dir_all(&first).expect("first");
        let session_file = root.join("session.json");

        let mut app = app_at(first);

        app.open_settings();
        assert_eq!(app.page(), AppPage::Settings);
        app.save_session_to(&session_file).expect("save session");
        let restored = AppState::load_session_from(&session_file).expect("restore session");
        assert_eq!(restored.page(), AppPage::Files);

        app.new_tab();
        assert_eq!(app.page(), AppPage::Files);
        app.open_settings();
        app.select_tab(0);
        assert_eq!(app.page(), AppPage::Files);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn corrupt_session_is_ignored() {
        let root = sandbox();
        let session_file = root.join("session.json");
        fs::write(&session_file, b"{not valid json").expect("write corrupt session");
        assert!(AppState::load_session_from(&session_file).is_none());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn missing_saved_directory_falls_back_safely() {
        let root = sandbox();
        let missing = root.join("gone");
        let session_file = root.join("session.json");
        let session = SessionState {
            version: SESSION_VERSION,
            active_tab: 0,
            tabs: vec![SavedTab {
                group_id: None,
                current_dir: missing,
                show_hidden: true,
                sort_field: SortField::Name,
                sort_direction: SortDirection::Ascending,
                back_stack: Vec::new(),
                forward_stack: Vec::new(),
            }],
            tab_groups: Vec::new(),
        };
        fs::write(
            &session_file,
            serde_json::to_vec(&session).expect("serialize"),
        )
        .expect("write session");

        let restored = AppState::load_session_from(&session_file).expect("restore session");
        assert!(restored.active_tab().current_dir.is_dir());
        assert!(restored.active_tab().show_hidden);
        let warning = restored.restore_warning().expect("restore warning");
        assert!(warning.contains("gone"));
        assert!(warning.contains("opened"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn tailscale_hostnames_are_short_stable_and_normalized() {
        let generated = generated_tailscale_hostname("tailnet-very-long-internal-profile-id");
        assert!(generated.starts_with("fe-"));
        assert_eq!(generated.len(), 9);
        assert_eq!(
            generated,
            generated_tailscale_hostname("tailnet-very-long-internal-profile-id")
        );
        assert_eq!(
            normalize_tailscale_hostname("  My Fast Explorer!!  "),
            "my-fast-explorer"
        );
        assert_eq!(
            normalize_tailscale_hostname("---WORK__PHONE---"),
            "work-phone"
        );
    }

    #[test]
    fn directory_change_changes_virtual_scroll_reset_key() {
        let root = sandbox();
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("nested");
        let mut tab = TabState::from_path(root.clone());
        let before = tab.scroll_reset_key();
        tab.set_current_dir(nested);
        assert_ne!(before, tab.scroll_reset_key());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn display_path_always_uses_forward_slashes() {
        assert_eq!(
            display_path(Path::new(r"C:\Users\Ada\Documents\report.txt")),
            "C:/Users/Ada/Documents/report.txt"
        );
    }

    #[test]
    fn taildrive_virtual_path_round_trips_and_has_human_title() {
        let location = TaildriveLocation::Remote {
            profile_id: "work".to_owned(),
            device_id: "desktop-1".to_owned(),
            share: "My Docs".to_owned(),
            remote_path: "projects/日本語/report.pdf".to_owned(),
        };
        let path = taildrive_path(&location);
        assert_eq!(parse_taildrive_path(&path), Some(location.clone()));
        let display = display_path(&path);
        assert_eq!(
            display,
            "TD/work/desktop-1/My Docs/projects/日本語/report.pdf"
        );
        assert_eq!(parse_taildrive_display_path(&display), Some(location));
        let tab = TabState::from_path(path);
        assert_eq!(tab.title(), "report.pdf");
    }

    #[test]
    fn taildrive_short_aliases_round_trip_without_changing_internal_ids() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("profile-internal-long-id", "Work");
        config.path = "w".to_owned();
        config
            .device_names
            .insert("device-internal-long-id".to_owned(), "pc".to_owned());
        let mut profile = TailnetProfileState::from_config(config);
        profile.status.taildrive_devices = vec![crate::tailscale::TaildriveDevice {
            id: "device-internal-long-id".to_owned(),
            hostname: "actual-long-device-name".to_owned(),
            dns_name: "actual-long-device-name.example.ts.net".to_owned(),
            os: "linux".to_owned(),
            ips: vec!["100.64.0.1".to_owned()],
            online: true,
            target: "actual-long-device-name".to_owned(),
            shares: vec!["Docs".to_owned()],
        }];
        app.tailscale_profiles = vec![profile];

        let location = TaildriveLocation::Remote {
            profile_id: "profile-internal-long-id".to_owned(),
            device_id: "device-internal-long-id".to_owned(),
            share: "Docs".to_owned(),
            remote_path: "project/report.txt".to_owned(),
        };
        let internal = taildrive_path(&location);
        assert_eq!(
            app.display_path_for(&internal),
            "TD/w/pc/Docs/project/report.txt"
        );
        assert_eq!(
            app.parse_taildrive_address("TD/w/pc/Docs/project/report.txt"),
            Some(location.clone())
        );
        assert_eq!(
            app.parse_taildrive_address(
                "TailDrive/profile-internal-long-id/device-internal-long-id/Docs/project/report.txt"
            ),
            Some(location.clone())
        );

        assert!(app.commit_tailnet_path("profile-internal-long-id", "x".to_owned()));
        assert!(app.commit_taildrive_device_name(
            "profile-internal-long-id",
            "device-internal-long-id",
            "d".to_owned(),
        ));
        assert_eq!(
            app.display_path_for(&internal),
            "TD/x/d/Docs/project/report.txt"
        );
        assert_eq!(parse_taildrive_path(&internal), Some(location));
    }

    #[test]
    fn cleared_tailnet_path_uses_short_automatic_alias_and_remains_editable() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("profile-with-a-long-stable-id", "Work");
        config.path = "work".to_owned();
        app.tailscale_profiles = vec![TailnetProfileState::from_config(config)];

        assert!(app.commit_tailnet_path("profile-with-a-long-stable-id", String::new()));
        let automatic = app.tailnet_path("profile-with-a-long-stable-id");
        assert!(automatic.starts_with("tn-"));
        assert!(automatic.chars().count() <= 9);
        assert!(app.tailscale_profiles[0].config.path.is_empty());

        assert!(app.commit_tailnet_path("profile-with-a-long-stable-id", "home".to_owned()));
        assert_eq!(app.tailnet_path("profile-with-a-long-stable-id"), "home");
    }

    #[test]
    fn tailnet_path_draft_does_not_mutate_until_apply() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("work", "Work");
        config.path = "old".to_owned();
        app.tailscale_profiles = vec![TailnetProfileState::from_config(config)];

        app.edit_tailnet_path("work", String::new());
        assert_eq!(app.tailnet_path_edit_value("work"), "");
        assert_eq!(app.tailnet_path("work"), "old");
        assert_eq!(app.tailscale_profile_settings()[0].path, "old");

        app.edit_tailnet_path("work", "new path".to_owned());
        assert_eq!(app.tailnet_path_edit_value("work"), "new path");
        assert_eq!(app.tailnet_path("work"), "old");
        app.apply_tailnet_path_draft("work");
        assert_eq!(app.tailnet_path("work"), "new-path");
        assert_eq!(app.tailnet_path_edit_value("work"), "new-path");

        app.reset_tailnet_path("work");
        assert!(app.tailscale_profile_settings()[0].path.is_empty());
        assert!(app.tailnet_path("work").starts_with("tn-"));
    }

    #[test]
    fn taildrive_device_name_draft_is_atomic_and_updates_visible_paths_only_on_apply() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("work", "Work");
        config.path = "w".to_owned();
        config
            .device_names
            .insert("device-1".to_owned(), "old".to_owned());
        let mut profile = TailnetProfileState::from_config(config);
        profile.status.taildrive_devices = vec![crate::tailscale::TaildriveDevice {
            id: "device-1".to_owned(),
            hostname: "desktop-long-name".to_owned(),
            dns_name: "desktop-long-name.example.ts.net".to_owned(),
            os: "linux".to_owned(),
            ips: vec![],
            online: true,
            target: "desktop-long-name".to_owned(),
            shares: vec!["Docs".to_owned()],
        }];
        app.tailscale_profiles = vec![profile];
        let location = TaildriveLocation::Remote {
            profile_id: "work".to_owned(),
            device_id: "device-1".to_owned(),
            share: "Docs".to_owned(),
            remote_path: "folder/report.txt".to_owned(),
        };
        let internal = taildrive_path(&location);
        app.active_tab_mut().set_current_dir(internal.clone());
        app.refresh_taildrive_address_inputs();

        app.edit_taildrive_device_name("work", "device-1", String::new());
        assert_eq!(app.taildrive_device_name_edit_value("work", "device-1"), "");
        assert_eq!(app.taildrive_device_name("work", "device-1"), "old");
        assert_eq!(
            app.active_tab().address_input,
            "TD/w/old/Docs/folder/report.txt"
        );
        assert_eq!(app.active_tab().current_dir, internal);

        app.edit_taildrive_device_name("work", "device-1", "desk / one".to_owned());
        assert_eq!(
            app.taildrive_device_name_edit_value("work", "device-1"),
            "desk / one"
        );
        assert_eq!(app.taildrive_device_name("work", "device-1"), "old");
        app.apply_taildrive_device_name_draft("work", "device-1");

        assert_eq!(app.taildrive_device_name("work", "device-1"), "desk - one");
        assert_eq!(
            app.active_tab().address_input,
            "TD/w/desk - one/Docs/folder/report.txt"
        );
        assert_eq!(
            parse_taildrive_path(&app.active_tab().current_dir),
            Some(location)
        );
    }

    #[test]
    fn rejected_device_name_keeps_draft_and_committed_alias() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("work", "Work");
        config
            .device_names
            .insert("device-a".to_owned(), "alpha".to_owned());
        config
            .device_names
            .insert("device-b".to_owned(), "beta".to_owned());
        app.tailscale_profiles = vec![TailnetProfileState::from_config(config)];

        app.edit_taildrive_device_name("work", "device-b", "alpha".to_owned());
        app.apply_taildrive_device_name_draft("work", "device-b");
        assert_eq!(app.taildrive_device_name("work", "device-b"), "beta");
        assert_eq!(
            app.taildrive_device_name_edit_value("work", "device-b"),
            "alpha"
        );
        assert!(
            app.tailscale_profiles[0]
                .ping_status
                .contains("already in use")
        );

        app.edit_taildrive_device_name("work", "device-b", "..".to_owned());
        app.apply_taildrive_device_name_draft("work", "device-b");
        assert_eq!(app.taildrive_device_name("work", "device-b"), "beta");
        assert_eq!(
            app.taildrive_device_name_edit_value("work", "device-b"),
            ".."
        );
        assert!(app.tailscale_profiles[0].ping_status.contains("invalid"));
    }

    #[test]
    fn applying_an_untouched_automatic_alias_does_not_turn_it_into_an_override() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut profile =
            TailnetProfileState::from_config(TailnetProfileSettings::new("work", "Work"));
        profile.status.taildrive_devices = vec![crate::tailscale::TaildriveDevice {
            id: "device-a".to_owned(),
            hostname: "desktop-a".to_owned(),
            dns_name: String::new(),
            os: "linux".to_owned(),
            ips: vec![],
            online: true,
            target: "desktop-a".to_owned(),
            shares: vec![],
        }];
        app.tailscale_profiles = vec![profile];

        let automatic_tailnet = app.tailnet_path_edit_value("work");
        let automatic_device = app.taildrive_device_name_edit_value("work", "device-a");
        app.apply_tailnet_path_draft("work");
        app.apply_taildrive_device_name_draft("work", "device-a");
        app.apply_tailnet_path_edit("work", automatic_tailnet);
        app.apply_taildrive_device_name_edit("work", "device-a", automatic_device);

        let config = &app.tailscale_profile_settings()[0];
        assert!(config.path.is_empty());
        assert!(!config.device_names.contains_key("device-a"));
    }

    #[test]
    fn device_alias_cannot_steal_a_name_used_by_remembered_offline_device() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("work", "Work");
        config.path = "w".to_owned();
        let mut profile = TailnetProfileState::from_config(config);
        profile.status.taildrive_devices = vec![crate::tailscale::TaildriveDevice {
            id: "device-a".to_owned(),
            hostname: "desktop-a".to_owned(),
            dns_name: String::new(),
            os: "linux".to_owned(),
            ips: vec![],
            online: true,
            target: "desktop-a".to_owned(),
            shares: vec![],
        }];
        app.tailscale_profiles = vec![profile];

        let remembered_id = "device-b-offline";
        let remembered_alias = app.taildrive_device_name("work", remembered_id);
        app.active_tab_mut()
            .back_stack
            .push(taildrive_path(&TaildriveLocation::Device {
                profile_id: "work".to_owned(),
                device_id: remembered_id.to_owned(),
            }));

        app.edit_taildrive_device_name("work", "device-a", remembered_alias.clone());
        app.apply_taildrive_device_name_draft("work", "device-a");

        assert_ne!(
            app.taildrive_device_name("work", "device-a"),
            remembered_alias
        );
        assert_eq!(
            app.taildrive_device_name_edit_value("work", "device-a"),
            remembered_alias
        );
        assert!(
            app.tailscale_profiles[0]
                .ping_status
                .contains("already in use")
        );
    }

    #[test]
    fn profile_refresh_preserves_drafts_for_profiles_that_still_exist() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("work", "Work");
        config.path = "w".to_owned();
        config
            .device_names
            .insert("device-a".to_owned(), "old".to_owned());
        app.tailscale_profiles = vec![TailnetProfileState::from_config(config)];
        app.edit_tailnet_path("work", "draft-tailnet".to_owned());
        app.edit_taildrive_device_name("work", "device-a", "draft-device".to_owned());

        let mut refreshed = app.tailscale_profile_settings();
        refreshed[0].enabled = !refreshed[0].enabled;
        app.set_tailscale_profiles(refreshed, false);

        assert_eq!(app.tailnet_path_edit_value("work"), "draft-tailnet");
        assert_eq!(
            app.taildrive_device_name_edit_value("work", "device-a"),
            "draft-device"
        );
    }

    #[test]
    fn taildrive_device_names_stay_short_unique_and_parseable_after_truncation() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("work", "Work");
        config.path = "w".to_owned();
        let mut profile = TailnetProfileState::from_config(config);
        let long_name = "this-device-name-is-definitely-too-long";
        let device_a = crate::tailscale::TaildriveDevice {
            id: "device-a".to_owned(),
            hostname: long_name.to_owned(),
            dns_name: String::new(),
            os: "linux".to_owned(),
            ips: vec![],
            online: true,
            target: long_name.to_owned(),
            shares: vec!["Docs".to_owned()],
        };
        let device_b = crate::tailscale::TaildriveDevice {
            id: "device-b".to_owned(),
            hostname: long_name.to_owned(),
            dns_name: String::new(),
            os: "linux".to_owned(),
            ips: vec![],
            online: true,
            target: long_name.to_owned(),
            shares: vec!["Docs".to_owned()],
        };
        profile.status.taildrive_devices = vec![device_b];
        app.tailscale_profiles = vec![profile];

        let second_before = app.taildrive_device_name("work", "device-b");
        app.tailscale_profiles[0]
            .status
            .taildrive_devices
            .push(device_a);
        let first = app.taildrive_device_name("work", "device-a");
        let second = app.taildrive_device_name("work", "device-b");
        assert!(first.chars().count() <= 24);
        assert!(second.chars().count() <= 24);
        assert_ne!(first.to_lowercase(), second.to_lowercase());
        assert_eq!(second, second_before);
        assert_eq!(
            app.parse_taildrive_address(&format!("TD/w/{first}")),
            Some(TaildriveLocation::Device {
                profile_id: "work".to_owned(),
                device_id: "device-a".to_owned(),
            })
        );
        assert_eq!(
            app.parse_taildrive_address(&format!("TD/w/{second}")),
            Some(TaildriveLocation::Device {
                profile_id: "work".to_owned(),
                device_id: "device-b".to_owned(),
            })
        );
    }

    #[test]
    fn remembered_taildrive_device_short_name_parses_before_status_arrives() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("work", "Work");
        config.path = "w".to_owned();
        app.tailscale_profiles = vec![TailnetProfileState::from_config(config)];
        let location = TaildriveLocation::Device {
            profile_id: "work".to_owned(),
            device_id: "very-long-internal-device-id".to_owned(),
        };
        app.active_tab_mut().current_dir = taildrive_path(&location);
        let short_name = app.taildrive_device_name("work", "very-long-internal-device-id");
        assert!(short_name.starts_with("d-"));
        assert!(short_name.chars().count() <= 24);
        assert_eq!(
            app.parse_taildrive_address(&format!("TD/w/{short_name}")),
            Some(location)
        );
    }

    #[test]
    fn removing_tailnet_prunes_stale_taildrive_tabs_history_and_pins() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;
        let mut config = TailnetProfileSettings::new("work", "Work");
        config.path = "w".to_owned();
        app.tailscale_profiles = vec![TailnetProfileState::from_config(config)];

        let remote = taildrive_path(&TaildriveLocation::Remote {
            profile_id: "work".to_owned(),
            device_id: "pc".to_owned(),
            share: "Docs".to_owned(),
            remote_path: "report.txt".to_owned(),
        });
        app.active_tab_mut().set_current_dir(remote.clone());
        app.active_tab_mut().back_stack.push(remote.clone());
        app.active_tab_mut().forward_stack.push(remote.clone());
        app.pinned_paths.push(remote);

        app.remove_tailnet_profile("work");

        assert_eq!(
            parse_taildrive_path(&app.active_tab().current_dir),
            Some(TaildriveLocation::Root)
        );
        assert!(app.active_tab().back_stack.is_empty());
        assert!(app.active_tab().forward_stack.is_empty());
        assert!(app.pinned_paths().is_empty());
        assert_eq!(app.active_tab().address_input, "TD");
    }

    #[test]
    fn legacy_taildrive_ids_win_over_conflicting_short_aliases() {
        let mut app = AppState::fresh();
        app.persistence_enabled = false;

        let mut first = TailnetProfileSettings::new("profile-a", "A");
        first.path = "profile-b".to_owned();
        first
            .device_names
            .insert("device-a".to_owned(), "device-b".to_owned());
        let mut first = TailnetProfileState::from_config(first);
        first.status.taildrive_devices = vec![
            crate::tailscale::TaildriveDevice {
                id: "device-a".to_owned(),
                hostname: "alpha".to_owned(),
                dns_name: "alpha.example.ts.net".to_owned(),
                os: "linux".to_owned(),
                ips: vec![],
                online: true,
                target: "alpha".to_owned(),
                shares: vec!["Docs".to_owned()],
            },
            crate::tailscale::TaildriveDevice {
                id: "device-b".to_owned(),
                hostname: "beta".to_owned(),
                dns_name: "beta.example.ts.net".to_owned(),
                os: "linux".to_owned(),
                ips: vec![],
                online: true,
                target: "beta".to_owned(),
                shares: vec!["Docs".to_owned()],
            },
        ];
        let mut second = TailnetProfileSettings::new("profile-b", "B");
        second.path = "b".to_owned();
        let second = TailnetProfileState::from_config(second);
        app.tailscale_profiles = vec![first, second];

        assert_eq!(
            app.parse_taildrive_address("TD/profile-b/device-b"),
            Some(TaildriveLocation::Device {
                profile_id: "profile-a".to_owned(),
                device_id: "device-a".to_owned(),
            })
        );
        assert_eq!(
            app.parse_taildrive_address("TailDrive/profile-b/device-b"),
            Some(TaildriveLocation::Device {
                profile_id: "profile-b".to_owned(),
                device_id: "device-b".to_owned(),
            })
        );
    }

    #[test]
    fn taildrive_display_and_cache_keys_escape_separator_like_names() {
        let first = TaildriveLocation::Remote {
            profile_id: "work/team".to_owned(),
            device_id: "desktop%1".to_owned(),
            share: "a/b".to_owned(),
            remote_path: "c/100%/notes\\old.txt".to_owned(),
        };
        let second = TaildriveLocation::Remote {
            profile_id: "work/team".to_owned(),
            device_id: "desktop%1".to_owned(),
            share: "a".to_owned(),
            remote_path: "b/c/100%/notes\\old.txt".to_owned(),
        };
        let display = taildrive_display_path(&first);
        assert_eq!(parse_taildrive_display_path(&display), Some(first.clone()));
        assert_ne!(
            taildrive_directory_cache_key(&first),
            taildrive_directory_cache_key(&second)
        );
    }

    #[test]
    fn taildrive_new_folder_is_virtual_until_confirmed() {
        let (mut app, mut receiver) = taildrive_app("parent");
        app.new_folder();
        assert!(app.rename_active());
        assert!(app.active_tab().pending_remote_folder.is_some());
        assert!(receiver.try_recv().is_err());

        app.set_rename_input("資料".to_owned());
        app.apply_rename();
        assert!(!app.rename_active());
        match receiver.try_recv().expect("mkdir command") {
            crate::tailscale::Command::TaildriveMkdir { path, .. } => {
                assert_eq!(path, "parent/資料");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn cancelling_virtual_taildrive_folder_sends_no_network_request() {
        let (mut app, mut receiver) = taildrive_app("parent");
        app.new_folder();
        assert_eq!(app.active_tab().entries.len(), 1);
        app.cancel_rename();
        assert!(app.active_tab().entries.is_empty());
        assert!(app.active_tab().pending_remote_folder.is_none());
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn taildrive_delete_requires_second_confirmation() {
        let (mut app, mut receiver) = taildrive_app("parent");
        let entry = taildrive_entry("parent", "old.txt", EntryKind::File);
        let path = entry.path.clone();
        app.active_tab_mut().entries = vec![entry];
        app.active_tab_mut().select_entry(path);

        app.delete_selected();
        assert!(receiver.try_recv().is_err());
        assert!(app.active_tab().status.contains("Press Delete again"));
        app.delete_selected();
        match receiver.try_recv().expect("delete command") {
            crate::tailscale::Command::TaildriveDelete { path, .. } => {
                assert_eq!(path, "parent/old.txt");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn local_directory_paste_to_taildrive_is_one_worker_command() {
        let source_root = sandbox();
        let source = source_root.join("bundle");
        fs::create_dir(&source).expect("bundle dir");
        fs::write(source.join("a.txt"), b"a").expect("child file");
        let (mut app, mut receiver) = taildrive_app("parent");
        app.file_clipboard = Some(FileClipboard {
            path: source.clone(),
            name: "bundle".to_owned(),
            kind: EntryKind::Directory,
            size: 0,
            remote_modified: None,
            mode: ClipboardMode::Copy,
        });

        app.paste();
        match receiver.try_recv().expect("upload command") {
            crate::tailscale::Command::TaildriveUpload {
                path,
                source: command_source,
                source_was_cut,
                ..
            } => {
                assert_eq!(path, "parent/bundle");
                assert_eq!(command_source, source);
                assert!(!source_was_cut);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(receiver.try_recv().is_err());
        fs::remove_dir_all(source_root).expect("cleanup");
    }

    #[test]
    fn cut_paste_to_taildrive_keeps_local_source_after_success() {
        let source_root = sandbox();
        let source = source_root.join("keep.txt");
        fs::write(&source, b"keep me").expect("source file");
        let (mut app, mut receiver) = taildrive_app("parent");
        app.file_clipboard = Some(FileClipboard {
            path: source.clone(),
            name: "keep.txt".to_owned(),
            kind: EntryKind::File,
            size: 0,
            remote_modified: None,
            mode: ClipboardMode::Cut,
        });

        app.paste();
        let (remote_path, source_location, source_was_cut, transfer_id) =
            match receiver.try_recv().expect("upload command") {
                crate::tailscale::Command::TaildriveUpload {
                    path,
                    source_location,
                    source_was_cut,
                    transfer_id,
                    ..
                } => (path, source_location, source_was_cut, transfer_id),
                other => panic!("unexpected command: {other:?}"),
            };
        assert!(source_was_cut);
        app.apply_tailscale_event(crate::tailscale::Event::TaildriveUpload {
            transfer_id,
            source: source.clone(),
            source_location,
            remote_path,
            source_was_cut,
            result: Ok(()),
        });
        assert!(source.exists());
        assert!(app.file_clipboard.is_none());
        assert!(app.active_tab().status.contains("Local source kept"));
        fs::remove_dir_all(source_root).expect("cleanup");
    }

    #[test]
    fn repeated_taildrive_open_reuses_in_progress_download() {
        let parent = format!("dedupe-{}", remote_cache_now_ms());
        let (mut app, mut receiver) = taildrive_app(&parent);
        let entry = taildrive_entry(&parent, "report.pdf", EntryKind::File);
        let path = entry.path.clone();
        app.active_tab_mut().entries = vec![entry];

        app.activate_entry(path.clone());
        assert!(matches!(
            receiver.try_recv().expect("first download"),
            crate::tailscale::Command::TaildriveDownload {
                open_after: true,
                ..
            }
        ));
        app.activate_entry(path);
        assert!(receiver.try_recv().is_err());
        assert_eq!(app.pending_remote_cache_downloads.len(), 1);
    }

    #[test]
    fn taildrive_selection_can_be_copied_to_clipboard() {
        let (mut app, _receiver) = taildrive_app("source");
        let entry = taildrive_entry("source", "report.pdf", EntryKind::File);
        let path = entry.path.clone();
        app.active_tab_mut().entries = vec![entry];
        app.active_tab_mut().select_entry(path.clone());

        assert!(app.can_clipboard_selected());
        app.copy_selected();
        let clipboard = app.file_clipboard.as_ref().expect("TailDrive clipboard");
        assert_eq!(clipboard.path, path);
        assert_eq!(clipboard.name, "report.pdf");
        assert_eq!(clipboard.mode, ClipboardMode::Copy);
    }

    #[test]
    fn taildrive_paste_to_local_dispatches_download_with_progress() {
        let target = sandbox();
        let mut app = app_at(target.clone());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        app.tailscale_sender = Some(sender);
        let source = taildrive_test_location("source/report.pdf");
        app.file_clipboard = Some(FileClipboard {
            path: taildrive_path(&source),
            name: "report.pdf".to_owned(),
            kind: EntryKind::File,
            size: 0,
            remote_modified: None,
            mode: ClipboardMode::Copy,
        });

        app.paste();
        match receiver.try_recv().expect("download command") {
            crate::tailscale::Command::TaildriveDownload {
                path,
                destination,
                open_after,
                transfer_id,
                ..
            } => {
                assert_eq!(path, "source/report.pdf");
                assert_eq!(destination, target.join("report.pdf"));
                assert!(!open_after);
                assert!(!transfer_id.is_empty());
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert_eq!(app.file_transfers.len(), 1);
        fs::remove_dir_all(target).expect("cleanup");
    }

    #[test]
    fn taildrive_paste_conflict_replace_is_explicit() {
        let (mut app, mut receiver) = taildrive_app("target");
        app.active_tab_mut().entries =
            vec![taildrive_entry("target", "report.pdf", EntryKind::File)];
        let source_root = sandbox();
        let source = source_root.join("report.pdf");
        fs::write(&source, b"replacement").expect("source");
        app.file_clipboard = Some(FileClipboard {
            path: source.clone(),
            name: "report.pdf".to_owned(),
            kind: EntryKind::File,
            size: 11,
            remote_modified: None,
            mode: ClipboardMode::Copy,
        });

        app.paste();
        assert_eq!(app.paste_conflict_name(), Some("report.pdf"));
        assert!(receiver.try_recv().is_err());
        app.replace_paste_conflict();
        match receiver.try_recv().expect("replace upload") {
            crate::tailscale::Command::TaildriveUpload { path, replace, .. } => {
                assert_eq!(path, "target/report.pdf");
                assert!(replace);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        fs::remove_dir_all(source_root).expect("cleanup");
    }

    #[test]
    fn taildrive_paste_to_taildrive_dispatches_relay() {
        let (mut app, mut receiver) = taildrive_app("target");
        let source = taildrive_test_location("source/report.pdf");
        app.file_clipboard = Some(FileClipboard {
            path: taildrive_path(&source),
            name: "report.pdf".to_owned(),
            kind: EntryKind::File,
            size: 0,
            remote_modified: None,
            mode: ClipboardMode::Copy,
        });

        app.paste();
        match receiver.try_recv().expect("relay command") {
            crate::tailscale::Command::TaildriveRelay {
                source_path,
                target_path,
                display_name,
                transfer_id,
                ..
            } => {
                assert_eq!(source_path, "source/report.pdf");
                assert_eq!(target_path, "target/report.pdf");
                assert_eq!(display_name, "report.pdf");
                assert!(!transfer_id.is_empty());
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert_eq!(app.file_transfers.len(), 1);
    }

    #[test]
    fn taildrive_transfer_progress_formats_percentage() {
        let (mut app, _receiver) = taildrive_app("target");
        app.begin_file_transfer("transfer-1".to_owned(), "report.pdf".to_owned(), "Copying");
        app.apply_tailscale_event(crate::tailscale::Event::TaildriveTransferProgress {
            transfer_id: "transfer-1".to_owned(),
            progress: crate::tailscale::TaildriveTransferProgress {
                phase: "Uploading".to_owned(),
                bytes_done: 5 * 1024 * 1024,
                bytes_total: 10 * 1024 * 1024,
                items_done: 1,
                paused: false,
                cancelled: false,
                items_total: 2,
                done: false,
                error: String::new(),
            },
        });
        let transfer = app.oldest_transfer_for_icon().expect("progress item");
        assert_eq!(transfer.phase, "Uploading");
        assert_eq!(transfer.label, "report.pdf");
        assert!((transfer.fraction() - 0.5).abs() < f64::EPSILON);
        let text = transfer.detail_text();
        assert!(text.contains("50%"));
        assert!(text.contains("5.0 MB / 10.0 MB"));
    }

    #[test]
    fn apk_install_transfer_keeps_user_facing_phase() {
        let (mut app, _receiver) = taildrive_app("target");
        app.begin_file_transfer(
            "apk-transfer".to_owned(),
            "sample.apk".to_owned(),
            "Preparing app install",
        );
        app.apply_tailscale_event(crate::tailscale::Event::TaildriveTransferProgress {
            transfer_id: "apk-transfer".to_owned(),
            progress: crate::tailscale::TaildriveTransferProgress {
                phase: "Downloading".to_owned(),
                bytes_done: 50,
                bytes_total: 100,
                items_done: 0,
                paused: false,
                cancelled: false,
                items_total: 1,
                done: false,
                error: String::new(),
            },
        });
        let transfer = app.oldest_transfer_for_icon().expect("apk progress");
        assert_eq!(transfer.phase, "Preparing app install");
        assert!((transfer.fraction() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn aab_is_treated_as_android_install_package() {
        assert!(is_android_install_name("sample.apk"));
        assert!(is_android_install_name("sample.APKS"));
        assert!(is_android_install_name("sample.AAB"));
        assert!(is_aab_name("sample.aab"));
        assert!(!is_android_install_name("sample.pdf"));
    }

    #[test]
    fn transfer_icon_tracks_oldest_unfinished_transfer() {
        let (mut app, _receiver) = taildrive_app("target");
        app.begin_file_transfer("first".to_owned(), "first.bin".to_owned(), "Uploading");
        app.begin_file_transfer("second".to_owned(), "second.bin".to_owned(), "Downloading");
        app.file_transfers[0].bytes_done = 25;
        app.file_transfers[0].bytes_total = 100;
        app.file_transfers[1].bytes_done = 80;
        app.file_transfers[1].bytes_total = 100;

        let oldest = app.oldest_transfer_for_icon().expect("oldest transfer");
        assert_eq!(oldest.transfer_id, "first");
        assert!((oldest.fraction() - 0.25).abs() < f64::EPSILON);

        app.finish_file_transfer("first", None);
        let oldest = app.oldest_transfer_for_icon().expect("next transfer");
        assert_eq!(oldest.transfer_id, "second");
        assert!((oldest.fraction() - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn transfer_popup_keeps_finished_history_until_cleared() {
        let (mut app, _receiver) = taildrive_app("target");
        app.begin_file_transfer("done".to_owned(), "done.bin".to_owned(), "Uploading");
        app.file_transfers[0].bytes_total = 1024;
        app.finish_file_transfer("done", None);
        let completed = app.file_transfers[0].display_detail_text();
        assert_eq!(completed.matches("Completed").count(), 1);
        app.toggle_transfer_popup();
        assert!(app.transfer_popup_open());
        assert_eq!(app.file_transfers().len(), 1);

        app.clear_finished_transfers();
        assert!(app.file_transfers().is_empty());
        assert!(app.transfer_popup_open());
    }

    #[test]
    fn transfer_popup_can_open_with_empty_history() {
        let (mut app, _receiver) = taildrive_app("target");
        assert!(app.file_transfers().is_empty());

        app.toggle_transfer_popup();
        assert!(app.transfer_popup_open());

        app.toggle_transfer_popup();
        assert!(!app.transfer_popup_open());
    }

    #[test]
    fn taildrive_mutation_lock_survives_navigation() {
        let (mut app, _receiver) = taildrive_app("parent");
        let source_location = app.active_tab().current_dir.clone();
        assert!(app.begin_remote_mutation(source_location.clone()));
        app.navigate_to(taildrive_path(&taildrive_test_location("other")));
        app.navigate_to(source_location.clone());
        assert!(!app.can_mutate_current_location());
        app.finish_remote_mutation(&source_location);
        assert!(app.can_mutate_current_location());
    }

    #[test]
    fn taildrive_existing_item_rename_uses_move_command() {
        let (mut app, mut receiver) = taildrive_app("parent");
        let entry = taildrive_entry("parent", "old.txt", EntryKind::File);
        let path = entry.path.clone();
        app.active_tab_mut().entries = vec![entry];
        app.active_tab_mut().select_entry(path);
        app.begin_rename();
        app.set_rename_input("new.txt".to_owned());
        app.apply_rename();
        match receiver.try_recv().expect("rename command") {
            crate::tailscale::Command::TaildriveRename { path, new_name, .. } => {
                assert_eq!(path, "parent/old.txt");
                assert_eq!(new_name, "new.txt");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn taildrive_mutation_lock_survives_navigation_and_clears_on_completion() {
        let (mut app, mut receiver) = taildrive_app("parent");
        let source_location = app.active_tab().current_dir.clone();
        let entry = taildrive_entry("parent", "old.txt", EntryKind::File);
        let path = entry.path.clone();
        app.active_tab_mut().entries = vec![entry];
        app.active_tab_mut().select_entry(path);
        app.begin_rename();
        app.set_rename_input("new.txt".to_owned());
        app.apply_rename();
        let _ = receiver.try_recv().expect("rename command");
        assert!(app.remote_mutations.contains(&source_location));

        app.navigate_to(taildrive_path(&TaildriveLocation::Root));
        assert!(app.remote_mutations.contains(&source_location));
        app.apply_tailscale_event(crate::tailscale::Event::TaildriveRename {
            source_location: source_location.clone(),
            remote_path: "parent/old.txt".to_owned(),
            new_name: "new.txt".to_owned(),
            result: Ok(()),
        });
        assert!(!app.remote_mutations.contains(&source_location));
    }

    #[test]
    fn failed_taildrive_command_send_releases_mutation_lock() {
        let (mut app, receiver) = taildrive_app("parent");
        drop(receiver);
        let source_location = app.active_tab().current_dir.clone();
        let entry = taildrive_entry("parent", "old.txt", EntryKind::File);
        let path = entry.path.clone();
        app.active_tab_mut().entries = vec![entry];
        app.active_tab_mut().select_entry(path);
        app.begin_rename();
        app.set_rename_input("new.txt".to_owned());
        app.apply_rename();
        assert!(!app.remote_mutations.contains(&source_location));
        assert!(app.active_tab().status.contains("worker stopped"));
    }

    #[test]
    fn common_sorting_keeps_folders_first_and_orders_size_and_date() {
        let mut tab = TabState::from_path(default_directory());
        let entry = |name: &str, kind: EntryKind, size: u64, modified_sort_key: u64| FileEntry {
            path: PathBuf::from(name),
            name: name.to_owned(),
            kind,
            size,
            modified_sort_key,
            remote: None,
            remote_modified: None,
        };
        tab.entries = vec![
            entry("tiny.txt", EntryKind::File, 1, 30),
            entry("folder", EntryKind::Directory, 0, 5),
            entry("large.bin", EntryKind::File, 100, 20),
        ];
        tab.sort_field = SortField::Size;
        tab.sort_direction = SortDirection::Descending;
        tab.apply_sort();
        assert_eq!(
            tab.entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["folder", "large.bin", "tiny.txt"]
        );
        tab.sort_field = SortField::DateModified;
        tab.sort_direction = SortDirection::Ascending;
        tab.apply_sort();
        assert_eq!(
            tab.entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["folder", "large.bin", "tiny.txt"]
        );
    }

    #[test]
    fn column_sort_switches_to_common_default_order_and_toggles() {
        let root = sandbox();
        fs::write(root.join("small.txt"), b"x").expect("small fixture");
        fs::write(root.join("large.txt"), b"larger payload").expect("large fixture");
        let mut app = app_at(root.clone());

        assert_eq!(app.sort_field(), SortField::Name);
        assert_eq!(app.sort_direction(), SortDirection::Ascending);

        app.activate_sort_field(SortField::Size);
        assert_eq!(app.sort_field(), SortField::Size);
        assert_eq!(app.sort_direction(), SortDirection::Descending);
        assert_eq!(app.active_tab().entries[0].name, "large.txt");

        app.activate_sort_field(SortField::Size);
        assert_eq!(app.sort_direction(), SortDirection::Ascending);
        assert_eq!(app.active_tab().entries[0].name, "small.txt");

        app.activate_sort_field(SortField::Type);
        assert_eq!(app.sort_field(), SortField::Type);
        assert_eq!(app.sort_direction(), SortDirection::Ascending);

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn semantic_file_categories_follow_filename_extension() {
        let entry = |name: &str| FileEntry {
            path: PathBuf::from(name),
            name: name.to_owned(),
            kind: EntryKind::File,
            size: 0,
            modified_sort_key: 0,
            remote: None,
            remote_modified: None,
        };
        assert_eq!(entry("photo.png").category(), FileCategory::Image);
        assert_eq!(entry("main.rs").category(), FileCategory::Code);
        assert_eq!(entry("sheet.xlsx").category(), FileCategory::Spreadsheet);
        assert_eq!(entry("data.json").category(), FileCategory::Json);
    }

    #[test]
    fn typeahead_selects_matching_initial_and_repeated_key_cycles() {
        let root = sandbox();
        for name in ["alpha.txt", "apricot.txt", "beta.txt"] {
            fs::write(root.join(name), name.as_bytes()).expect("fixture");
        }
        let mut app = app_at(root.clone());
        assert_eq!(app.typeahead_select("a".to_owned()), Some(0));
        assert_eq!(app.active_tab().entries[0].name, "alpha.txt");
        assert_eq!(app.typeahead_select("a".to_owned()), Some(1));
        assert_eq!(app.active_tab().entries[1].name, "apricot.txt");
        assert_eq!(app.typeahead_select("b".to_owned()), Some(2));
        assert_eq!(app.active_tab().entries[2].name, "beta.txt");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn slow_second_click_on_selected_file_enters_inline_rename() {
        let root = sandbox();
        let file = root.join("report.txt");
        fs::write(&file, b"report").expect("file");
        let mut app = app_at(root.clone());
        app.click_entry(file.clone());
        app.active_tab_mut().last_click =
            Some((file.clone(), Instant::now() - Duration::from_millis(700)));
        app.click_entry(file);
        assert!(app.rename_active());
        assert_eq!(app.active_tab().rename_input.as_deref(), Some("report.txt"));
        fs::remove_dir_all(root).expect("cleanup");
    }
}
