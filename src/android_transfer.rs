use std::collections::{BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Instant;

use jni::objects::{JClass, JString};

const TRANSFER_HISTORY_LIMIT: usize = 32;
static LOCAL_TRANSFER_COUNTER: AtomicU64 = AtomicU64::new(1);
static UI_REVISION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub(crate) enum UiEvent {
    Tailscale(crate::tailscale::Event),
    Local {
        transfer_id: String,
        event: crate::app::LocalFileEvent,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct TransferSnapshot {
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
    pub bytes_per_second: f64,
}

#[derive(Debug, Clone)]
enum TransferJob {
    Tailscale(crate::tailscale::Command),
    Local(crate::app::LocalFileCommand),
}

#[derive(Debug)]
struct TransferRecord {
    snapshot: TransferSnapshot,
    job: TransferJob,
    protected_files: BTreeSet<String>,
    resume_phase: String,
    pause_override: Option<bool>,
    cancel_requested: bool,
    last_sample_at: Instant,
    last_sample_bytes: u64,
}

#[derive(Debug, Default)]
struct TransferState {
    records: Vec<TransferRecord>,
    events: VecDeque<UiEvent>,
    system_file_changes: VecDeque<PathBuf>,
    batch_failed_count: usize,
    batch_last_finished: Option<String>,
}

static STATE: OnceLock<Mutex<TransferState>> = OnceLock::new();
static CONTROL_CV: OnceLock<Condvar> = OnceLock::new();

fn control_cv() -> &'static Condvar {
    CONTROL_CV.get_or_init(Condvar::new)
}

fn state() -> &'static Mutex<TransferState> {
    STATE.get_or_init(|| Mutex::new(TransferState::default()))
}

fn lock_state() -> std::sync::MutexGuard<'static, TransferState> {
    state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn mark_ui_changed() {
    UI_REVISION.fetch_add(1, Ordering::Release);
}

pub(crate) fn ui_revision() -> u64 {
    UI_REVISION.load(Ordering::Acquire)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| crate::app::display_path(path))
}
fn register_transfer(
    transfer_id: String,
    label: String,
    phase: String,
    protected_files: BTreeSet<String>,
    job: TransferJob,
) {
    let mut state = lock_state();
    if state
        .records
        .iter()
        .any(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
    {
        return;
    }
    state
        .records
        .retain(|record| record.snapshot.transfer_id != transfer_id);
    if !state.records.iter().any(|record| !record.snapshot.done) {
        state.batch_failed_count = 0;
        state.batch_last_finished = None;
    }
    if state.records.len() >= TRANSFER_HISTORY_LIMIT
        && let Some(index) = state.records.iter().position(|record| record.snapshot.done)
    {
        state.records.remove(index);
    }
    let now = Instant::now();
    state.records.push(TransferRecord {
        snapshot: TransferSnapshot {
            transfer_id,
            label,
            phase: phase.clone(),
            bytes_done: 0,
            bytes_total: 0,
            items_done: 0,
            items_total: 0,
            paused: false,
            cancelling: false,
            cancelled: false,
            done: false,
            error: None,
            bytes_per_second: 0.0,
        },
        job,
        protected_files,
        resume_phase: phase,
        pause_override: None,
        cancel_requested: false,
        last_sample_at: now,
        last_sample_bytes: 0,
    });
    mark_ui_changed();
}

fn reconcile_pause_progress(pause_override: &mut Option<bool>, reported_paused: bool) -> bool {
    if let Some(desired_paused) = *pause_override {
        if reported_paused == desired_paused {
            *pause_override = None;
        }
        desired_paused
    } else {
        reported_paused
    }
}

fn update_taildrive_progress(
    transfer_id: &str,
    progress: crate::tailscale::TaildriveTransferProgress,
) {
    let mut state = lock_state();
    let Some(record) = state
        .records
        .iter_mut()
        .find(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
    else {
        return;
    };
    // pause()/resume() complete before they update the UI snapshot. A progress
    // callback already in flight can therefore describe the previous control state.
    // Keep the explicit user command authoritative until the transfer reports that
    // same state, then resume following progress normally.
    record.snapshot.paused = reconcile_pause_progress(&mut record.pause_override, progress.paused);
    if progress.cancelled {
        record.cancel_requested = true;
        record.snapshot.cancelling = true;
    }
    if !progress.phase.is_empty()
        && !record.snapshot.paused
        && !record.snapshot.cancelling
        && record.snapshot.phase != "Preparing app install"
    {
        record.resume_phase = progress.phase.clone();
        record.snapshot.phase = progress.phase;
    }
    let now = Instant::now();
    let elapsed = now.duration_since(record.last_sample_at).as_secs_f64();
    if progress.bytes_done > record.last_sample_bytes && elapsed >= 0.20 {
        let sample = (progress.bytes_done - record.last_sample_bytes) as f64 / elapsed;
        record.snapshot.bytes_per_second = if record.snapshot.bytes_per_second <= 0.0 {
            sample
        } else {
            record.snapshot.bytes_per_second * 0.72 + sample * 0.28
        };
        record.last_sample_at = now;
        record.last_sample_bytes = progress.bytes_done;
    }
    record.snapshot.bytes_done = progress.bytes_done;
    record.snapshot.bytes_total = progress.bytes_total;
    record.snapshot.items_done = progress.items_done;
    record.snapshot.items_total = progress.items_total;
    mark_ui_changed();
}

fn update_local_progress(
    transfer_id: &str,
    bytes_done: u64,
    bytes_total: u64,
    items_done: u64,
    items_total: u64,
) {
    let mut state = lock_state();
    let Some(record) = state
        .records
        .iter_mut()
        .find(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
    else {
        return;
    };
    let now = Instant::now();
    let elapsed = now.duration_since(record.last_sample_at).as_secs_f64();
    if bytes_done > record.last_sample_bytes && elapsed >= 0.20 {
        let sample = (bytes_done - record.last_sample_bytes) as f64 / elapsed;
        record.snapshot.bytes_per_second = if record.snapshot.bytes_per_second <= 0.0 {
            sample
        } else {
            record.snapshot.bytes_per_second * 0.72 + sample * 0.28
        };
        record.last_sample_at = now;
        record.last_sample_bytes = bytes_done;
    }
    record.snapshot.bytes_done = bytes_done;
    record.snapshot.bytes_total = bytes_total;
    record.snapshot.items_done = items_done;
    record.snapshot.items_total = items_total;
    mark_ui_changed();
}

fn finish_transfer(transfer_id: &str, error: Option<String>) {
    let mut state = lock_state();
    let Some(record) = state
        .records
        .iter_mut()
        .find(|record| record.snapshot.transfer_id == transfer_id)
    else {
        return;
    };
    let cancelled = record.cancel_requested
        || error
            .as_deref()
            .is_some_and(|message| message.eq_ignore_ascii_case("transfer cancelled"));
    record.snapshot.done = true;
    record.pause_override = None;
    record.snapshot.paused = false;
    record.snapshot.cancelling = false;
    record.snapshot.cancelled = cancelled;
    record.snapshot.error = if cancelled { None } else { error.clone() };
    if cancelled {
        record.snapshot.phase = "Cancelled".to_owned();
    } else if error.is_some() {
        record.snapshot.phase = "Failed".to_owned();
    } else {
        record.snapshot.phase = "Completed".to_owned();
        if record.snapshot.bytes_total > 0 {
            record.snapshot.bytes_done = record.snapshot.bytes_total;
        }
        if record.snapshot.items_total > 0 {
            record.snapshot.items_done = record.snapshot.items_total;
        }
    }
    let finished_id = record.snapshot.transfer_id.clone();
    if error.is_some() && !cancelled {
        state.batch_failed_count = state.batch_failed_count.saturating_add(1);
    }
    state.batch_last_finished = Some(finished_id);
    mark_ui_changed();
}

fn push_event(event: UiEvent) {
    lock_state().events.push_back(event);
    mark_ui_changed();
}

fn queue_system_file_changes(paths: impl IntoIterator<Item = PathBuf>) {
    let mut state = lock_state();
    for path in paths {
        if !state.system_file_changes.contains(&path) {
            state.system_file_changes.push_back(path);
        }
    }
}

fn drain_system_file_changes_json() -> String {
    let mut state = lock_state();
    let paths = state
        .system_file_changes
        .drain(..)
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    serde_json::to_string(&paths).unwrap_or_else(|_| "[]".to_owned())
}

pub(crate) fn snapshots() -> Vec<TransferSnapshot> {
    lock_state()
        .records
        .iter()
        .map(|record| record.snapshot.clone())
        .collect()
}

pub(crate) fn drain_ui_events() -> Vec<UiEvent> {
    lock_state().events.drain(..).collect()
}

pub(crate) fn has_active_transfers() -> bool {
    lock_state()
        .records
        .iter()
        .any(|record| !record.snapshot.done)
}
pub(crate) fn protected_cache_files() -> BTreeSet<String> {
    let state = lock_state();
    state
        .records
        .iter()
        .filter(|record| !record.snapshot.done)
        .flat_map(|record| record.protected_files.iter().cloned())
        .collect()
}

fn taildrive_event_error(event: &crate::tailscale::Event) -> Option<String> {
    match event {
        crate::tailscale::Event::TaildriveDownload { result, .. }
        | crate::tailscale::Event::TaildriveUpload { result, .. }
        | crate::tailscale::Event::TaildriveRelay { result, .. } => result.as_ref().err().cloned(),
        _ => Some("background transfer returned an unexpected event".to_owned()),
    }
}

fn is_android_install_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("apk")
                || extension.eq_ignore_ascii_case("apks")
                || extension.eq_ignore_ascii_case("aab")
        })
}

fn protect_path(path: &Path, protected: &mut BTreeSet<String>) {
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        protected.insert(name.to_owned());
    }
}

fn wait_local_control(transfer_id: &str) -> Result<(), String> {
    let mut guard = lock_state();
    loop {
        let Some(record) = guard
            .records
            .iter()
            .find(|record| record.snapshot.transfer_id == transfer_id)
        else {
            return Err("transfer state disappeared".to_owned());
        };
        if record.cancel_requested {
            return Err("transfer cancelled".to_owned());
        }
        if !record.snapshot.paused {
            return Ok(());
        }
        guard = control_cv()
            .wait(guard)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

fn set_transfer_phase(transfer_id: &str, phase: &str) {
    let mut state = lock_state();
    if let Some(record) = state
        .records
        .iter_mut()
        .find(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
    {
        record.resume_phase = phase.to_owned();
        if !record.snapshot.paused && !record.snapshot.cancelling {
            record.snapshot.phase = phase.to_owned();
        }
        mark_ui_changed();
    }
}

fn measure_local_path(path: &Path) -> Result<(u64, u64), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        let mut bytes = 0u64;
        let mut items = 1u64;
        for entry in std::fs::read_dir(path).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let (entry_bytes, entry_items) = measure_local_path(&entry.path())?;
            bytes = bytes.saturating_add(entry_bytes);
            items = items.saturating_add(entry_items);
        }
        Ok((bytes, items))
    } else {
        Ok((metadata.len(), 1))
    }
}

fn apply_local_metadata(metadata: &std::fs::Metadata, destination: &Path) {
    // Apply timestamps while the freshly-created destination still has writable/readable
    // creator permissions. A source may intentionally be 0222/0111; applying those mode
    // bits first can make reopening the destination impossible before timestamps are set.
    let mut times = std::fs::FileTimes::new();
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    if let Ok(modified) = metadata.modified() {
        times = times.set_modified(modified);
    }
    match std::fs::File::open(destination) {
        Ok(file) => {
            if let Err(error) = file.set_times(times) {
                eprintln!(
                    "FastExplorer: cannot preserve timestamps for {}: {error}",
                    destination.display()
                );
            }
        }
        Err(error) => eprintln!(
            "FastExplorer: cannot reopen {} to preserve timestamps: {error}",
            destination.display()
        ),
    }
    if let Err(error) = std::fs::set_permissions(destination, metadata.permissions()) {
        // Shared-storage/FUSE volumes may not implement POSIX mode changes. The copy
        // itself is still valid, so metadata preservation remains best-effort there.
        eprintln!(
            "FastExplorer: cannot preserve permissions for {}: {error}",
            destination.display()
        );
    }
}

fn copy_local_symlink_controlled(
    transfer_id: &str,
    source: &Path,
    destination: &Path,
    bytes_done: &mut u64,
    items_done: &mut u64,
    bytes_total: u64,
    items_total: u64,
) -> Result<(), String> {
    wait_local_control(transfer_id)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let target = std::fs::read_link(source).map_err(|error| error.to_string())?;
    std::os::unix::fs::symlink(target, destination).map_err(|error| error.to_string())?;
    let size = std::fs::symlink_metadata(source)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    *bytes_done = bytes_done.saturating_add(size);
    *items_done = items_done.saturating_add(1);
    update_local_progress(
        transfer_id,
        *bytes_done,
        bytes_total,
        *items_done,
        items_total,
    );
    Ok(())
}

fn copy_local_file_controlled(
    transfer_id: &str,
    source: &Path,
    destination: &Path,
    bytes_done: &mut u64,
    items_done: &mut u64,
    bytes_total: u64,
    items_total: u64,
) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let metadata = std::fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    let mut input = std::fs::File::open(source).map_err(|error| error.to_string())?;
    let mut output = std::fs::File::create(destination).map_err(|error| error.to_string())?;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        wait_local_control(transfer_id)?;
        let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
        *bytes_done = bytes_done.saturating_add(read as u64);
        update_local_progress(
            transfer_id,
            *bytes_done,
            bytes_total,
            *items_done,
            items_total,
        );
    }
    output.flush().map_err(|error| error.to_string())?;
    drop(output);
    apply_local_metadata(&metadata, destination);
    *items_done = items_done.saturating_add(1);
    update_local_progress(
        transfer_id,
        *bytes_done,
        bytes_total,
        *items_done,
        items_total,
    );
    Ok(())
}

fn copy_local_path_controlled(
    transfer_id: &str,
    source: &Path,
    destination: &Path,
    bytes_done: &mut u64,
    items_done: &mut u64,
    bytes_total: u64,
    items_total: u64,
) -> Result<(), String> {
    wait_local_control(transfer_id)?;
    let metadata = std::fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return copy_local_symlink_controlled(
            transfer_id,
            source,
            destination,
            bytes_done,
            items_done,
            bytes_total,
            items_total,
        );
    }
    if metadata.is_dir() {
        std::fs::create_dir(destination).map_err(|error| error.to_string())?;
        *items_done = items_done.saturating_add(1);
        update_local_progress(
            transfer_id,
            *bytes_done,
            bytes_total,
            *items_done,
            items_total,
        );
        for entry in std::fs::read_dir(source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            copy_local_path_controlled(
                transfer_id,
                &entry.path(),
                &destination.join(entry.file_name()),
                bytes_done,
                items_done,
                bytes_total,
                items_total,
            )?;
        }
        apply_local_metadata(&metadata, destination);
        Ok(())
    } else {
        copy_local_file_controlled(
            transfer_id,
            source,
            destination,
            bytes_done,
            items_done,
            bytes_total,
            items_total,
        )
    }
}

fn remove_local_path(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        std::fs::remove_file(path).map_err(|error| error.to_string())
    }
}

fn remove_local_path_controlled(transfer_id: &str, path: &Path) -> Result<(), String> {
    wait_local_control(transfer_id)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        loop {
            wait_local_control(transfer_id)?;
            let entries = std::fs::read_dir(path)
                .map_err(|error| error.to_string())?
                .take(128)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            if entries.is_empty() {
                break;
            }
            for entry in entries {
                remove_local_path_controlled(transfer_id, &entry.path())?;
            }
        }
        std::fs::remove_dir(path).map_err(|error| error.to_string())
    } else {
        std::fs::remove_file(path).map_err(|error| error.to_string())
    }
}

fn remove_local_path_in_background(path: PathBuf) {
    let display = crate::app::display_path(&path);
    let cleanup_path = path.clone();
    let spawn = std::thread::Builder::new()
        .name("fast-explorer-replace-cleanup".to_owned())
        .spawn(move || {
            for attempt in 0..3 {
                match remove_local_path(&cleanup_path) {
                    Ok(()) => return,
                    Err(_) if !cleanup_path.symlink_metadata().is_ok() => return,
                    Err(error) if attempt < 2 => {
                        eprintln!(
                            "FastExplorer: replace backup cleanup retry {} for {}: {error}",
                            attempt + 1,
                            cleanup_path.display()
                        );
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                    Err(error) => {
                        eprintln!(
                            "FastExplorer: replace backup cleanup failed for {}: {error}",
                            cleanup_path.display()
                        );
                    }
                }
            }
        });
    if let Err(error) = spawn {
        eprintln!("FastExplorer: cannot schedule replace backup cleanup for {display}: {error}");
        if let Err(cleanup_error) = remove_local_path(&path) {
            eprintln!(
                "FastExplorer: synchronous replace backup cleanup also failed for {display}: {cleanup_error}"
            );
        }
    }
}

fn local_staging_path(destination: &Path, transfer_id: &str) -> PathBuf {
    destination
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".fastexplorer-transfer-{transfer_id}"))
}

fn local_replace_backup_path(destination: &Path, transfer_id: &str) -> Result<PathBuf, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "replace destination has no parent directory".to_owned())?;
    for suffix in 0..10_000u32 {
        let candidate = parent.join(format!(
            ".fastexplorer-replace-backup-{transfer_id}-{suffix}"
        ));
        if !candidate.symlink_metadata().is_ok() {
            return Ok(candidate);
        }
    }
    Err("cannot allocate a temporary replace backup path".to_owned())
}

fn merge_staging_entry_controlled(
    transfer_id: &str,
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    wait_local_control(transfer_id)?;
    if !destination.symlink_metadata().is_ok() {
        return std::fs::rename(source, destination).map_err(|error| error.to_string());
    }

    let source_metadata = std::fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    let destination_metadata =
        std::fs::symlink_metadata(destination).map_err(|error| error.to_string())?;
    let source_dir = source_metadata.is_dir() && !source_metadata.file_type().is_symlink();
    let destination_dir =
        destination_metadata.is_dir() && !destination_metadata.file_type().is_symlink();

    if source_dir && destination_dir {
        loop {
            wait_local_control(transfer_id)?;
            let entries = std::fs::read_dir(source)
                .map_err(|error| error.to_string())?
                .take(128)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            if entries.is_empty() {
                break;
            }
            for entry in entries {
                merge_staging_entry_controlled(
                    transfer_id,
                    &entry.path(),
                    &destination.join(entry.file_name()),
                )?;
            }
        }
        return std::fs::remove_dir(source).map_err(|error| error.to_string());
    }

    if !source_dir && !destination_dir && std::fs::rename(source, destination).is_ok() {
        // Android/Linux rename atomically replaces an existing non-directory entry.
        return Ok(());
    }

    // Type conflicts (file <-> directory), or an OEM/filesystem that refused the
    // direct replace above, use a same-filesystem backup so the old destination can
    // be restored if publishing the staged entry fails. Deleting a large replaced
    // directory remains pause/cancel-aware instead of blocking in a final fs::copy.
    let backup = local_replace_backup_path(destination, transfer_id)?;
    std::fs::rename(destination, &backup).map_err(|error| error.to_string())?;
    if let Err(error) = std::fs::rename(source, destination) {
        let rollback = std::fs::rename(&backup, destination);
        return Err(match rollback {
            Ok(()) => error.to_string(),
            Err(rollback_error) => format!(
                "cannot publish replacement ({error}); rollback also failed: {rollback_error}"
            ),
        });
    }
    // Once the staged entry has been published, this backup is no longer part of the
    // user's transfer result. Delete it off the transfer worker so a large old tree or
    // a transient filesystem lock cannot hold progress at 99% or turn a successful
    // replacement into a reported failure.
    remove_local_path_in_background(backup);
    Ok(())
}

fn publish_local_staging(
    transfer_id: &str,
    staging: &Path,
    destination: &Path,
    replace: bool,
) -> Result<(), String> {
    wait_local_control(transfer_id)?;
    if !destination.exists() {
        set_transfer_phase(transfer_id, "Finishing");
        return std::fs::rename(staging, destination).map_err(|error| error.to_string());
    }
    if !replace {
        return Err(format!(
            "Destination already exists: {}",
            crate::app::display_path(destination)
        ));
    }

    // The staging entry lives beside the destination on the same filesystem.
    // Merge by rename instead of copying the full payload a second time. Keep the
    // phase controllable while walking conflicting directories and deleting old
    // replacements; only the final trivial state transition is marked Finishing.
    set_transfer_phase(transfer_id, "Merging");
    merge_staging_entry_controlled(transfer_id, staging, destination)?;
    set_transfer_phase(transfer_id, "Finishing");
    Ok(())
}

fn perform_local_transfer(
    transfer_id: &str,
    command: &crate::app::LocalFileCommand,
) -> Result<(), String> {
    let crate::app::LocalFileCommand::CopyMove {
        source,
        destination,
        cut,
        replace,
        ..
    } = command
    else {
        return crate::app::perform_local_file_command(command);
    };
    if *cut && !*replace && std::fs::rename(source, destination).is_ok() {
        return Ok(());
    }
    let (bytes_total, items_total) = measure_local_path(source)?;
    update_local_progress(transfer_id, 0, bytes_total, 0, items_total);
    let staging = local_staging_path(destination, transfer_id);
    if staging.exists() {
        remove_local_path(&staging)?;
    }
    let mut bytes_done = 0;
    let mut items_done = 0;
    let copied = copy_local_path_controlled(
        transfer_id,
        source,
        &staging,
        &mut bytes_done,
        &mut items_done,
        bytes_total,
        items_total,
    );
    if let Err(error) = copied {
        let _ = remove_local_path(&staging);
        return Err(error);
    }
    if let Err(error) = publish_local_staging(transfer_id, &staging, destination, *replace) {
        let _ = remove_local_path(&staging);
        return Err(error);
    }
    if *cut {
        set_transfer_phase(transfer_id, "Removing source");
        remove_local_path_controlled(transfer_id, source)?;
        set_transfer_phase(transfer_id, "Finishing");
    }
    Ok(())
}

fn taildrive_control_ids(command: &crate::tailscale::Command, transfer_id: &str) -> Vec<String> {
    if matches!(command, crate::tailscale::Command::TaildriveRelay { .. }) {
        vec![
            format!("{transfer_id}-download"),
            format!("{transfer_id}-upload"),
        ]
    } else {
        vec![transfer_id.to_owned()]
    }
}

fn control_taildrive_job(
    command: &crate::tailscale::Command,
    transfer_id: &str,
    action: &str,
) -> Result<(), String> {
    let ids = taildrive_control_ids(command, transfer_id);
    let mut controlled = false;
    for id in ids {
        match crate::tailscale::taildrive_transfer_control(&id, action) {
            Ok(()) => controlled = true,
            Err(error) if action == "prepare" => return Err(error),
            Err(_) => {}
        }
    }
    if controlled {
        Ok(())
    } else {
        Err(format!("cannot {action} transfer"))
    }
}

fn spawn_taildrive_job(transfer_id: String, command: crate::tailscale::Command) {
    if let Err(error) = control_taildrive_job(&command, &transfer_id, "prepare") {
        finish_transfer(&transfer_id, Some(error));
        return;
    }
    let thread_id = transfer_id.clone();
    let release_ids = taildrive_control_ids(&command, &transfer_id);
    let spawn = std::thread::Builder::new()
        .name(format!("fast-explorer-transfer-{transfer_id}"))
        .spawn(move || {
            let event = crate::tailscale::execute_background_transfer(command, |id, progress| {
                update_taildrive_progress(id, progress);
            });
            for id in release_ids {
                let _ = crate::tailscale::taildrive_transfer_control(&id, "release");
            }
            match event {
                Some(event) => {
                    let error = taildrive_event_error(&event);
                    if let crate::tailscale::Event::TaildriveDownload {
                        destination,
                        open_after: false,
                        result: Ok(()),
                        ..
                    } = &event
                    {
                        queue_system_file_changes([destination.clone()]);
                    }
                    finish_transfer(&thread_id, error);
                    push_event(UiEvent::Tailscale(event));
                }
                None => {
                    finish_transfer(
                        &thread_id,
                        Some("unsupported background TailDrive transfer".to_owned()),
                    );
                }
            }
        });
    if let Err(error) = spawn {
        finish_transfer(
            &transfer_id,
            Some(format!("cannot start background transfer thread: {error}")),
        );
    }
}

fn spawn_local_job(transfer_id: String, command: crate::app::LocalFileCommand) {
    let thread_id = transfer_id.clone();
    let worker_command = command.clone();
    let spawn = std::thread::Builder::new()
        .name(format!("fast-explorer-transfer-{transfer_id}"))
        .spawn(move || {
            let result = perform_local_transfer(&thread_id, &worker_command);
            if result.is_ok()
                && let crate::app::LocalFileCommand::CopyMove {
                    source,
                    destination,
                    cut,
                    ..
                } = &worker_command
            {
                if *cut {
                    queue_system_file_changes([source.clone(), destination.clone()]);
                } else {
                    queue_system_file_changes([destination.clone()]);
                }
            }
            let error = result.as_ref().err().cloned();
            finish_transfer(&thread_id, error);
            push_event(UiEvent::Local {
                transfer_id: thread_id,
                event: crate::app::LocalFileEvent {
                    command: worker_command,
                    result,
                },
            });
        });
    if let Err(error) = spawn {
        finish_transfer(
            &transfer_id,
            Some(format!("cannot start background file operation: {error}")),
        );
    }
}

pub(crate) fn submit_taildrive(command: crate::tailscale::Command) {
    let mut protected = BTreeSet::new();
    let (transfer_id, label, phase) = match &command {
        crate::tailscale::Command::TaildriveDownload {
            transfer_id,
            destination,
            display_name,
            open_after,
            ..
        } => {
            protect_path(destination, &mut protected);
            let phase = if *open_after && is_android_install_name(display_name) {
                "Preparing app install"
            } else {
                "Downloading"
            };
            (transfer_id.clone(), display_name.clone(), phase.to_owned())
        }
        crate::tailscale::Command::TaildriveUpload {
            transfer_id,
            source,
            path,
            ..
        } => {
            protect_path(source, &mut protected);
            let label = source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| path.rsplit('/').next().unwrap_or("Upload").to_owned());
            (transfer_id.clone(), label, "Uploading".to_owned())
        }
        crate::tailscale::Command::TaildriveRelay {
            transfer_id,
            display_name,
            ..
        } => (
            transfer_id.clone(),
            display_name.clone(),
            "Copying".to_owned(),
        ),
        _ => return,
    };
    register_transfer(
        transfer_id.clone(),
        label,
        phase,
        protected,
        TransferJob::Tailscale(command.clone()),
    );
    spawn_taildrive_job(transfer_id, command);
}

pub(crate) fn submit_local(command: crate::app::LocalFileCommand) {
    let (label, phase) = match &command {
        crate::app::LocalFileCommand::CopyMove {
            destination, cut, ..
        } => (
            file_name(destination),
            if *cut { "Moving" } else { "Copying" }.to_owned(),
        ),
        _ => return,
    };
    let transfer_id = format!(
        "android-local-{}",
        LOCAL_TRANSFER_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    register_transfer(
        transfer_id.clone(),
        label,
        phase,
        BTreeSet::new(),
        TransferJob::Local(command.clone()),
    );
    spawn_local_job(transfer_id, command);
}

pub(crate) fn pause(transfer_id: &str) -> Result<(), String> {
    let job = {
        let state = lock_state();
        let record = state
            .records
            .iter()
            .find(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
            .ok_or_else(|| "transfer is not active".to_owned())?;
        if record.snapshot.phase.starts_with("Finishing") || record.snapshot.cancelling {
            return Err("transfer is finishing and cannot be paused safely".to_owned());
        }
        record.job.clone()
    };
    if let TransferJob::Tailscale(command) = &job
        && control_taildrive_job(command, transfer_id, "pause").is_err()
    {
        return Err("cannot pause transfer right now".to_owned());
    }
    let mut state = lock_state();
    if let Some(record) = state
        .records
        .iter_mut()
        .find(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
    {
        record.pause_override = Some(true);
        record.snapshot.paused = true;
        record.snapshot.phase = "Paused".to_owned();
        record.snapshot.bytes_per_second = 0.0;
        mark_ui_changed();
        Ok(())
    } else {
        Err("transfer is not active".to_owned())
    }
}

pub(crate) fn resume(transfer_id: &str) -> Result<(), String> {
    let job = {
        let state = lock_state();
        state
            .records
            .iter()
            .find(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
            .map(|record| record.job.clone())
    };
    let Some(job) = job else {
        return Err("transfer is not active".to_owned());
    };
    if let TransferJob::Tailscale(command) = &job
        && control_taildrive_job(command, transfer_id, "resume").is_err()
    {
        return Err("cannot resume transfer right now".to_owned());
    }
    let mut state = lock_state();
    if let Some(record) = state
        .records
        .iter_mut()
        .find(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
    {
        record.pause_override = Some(false);
        record.snapshot.paused = false;
        record.snapshot.phase = record.resume_phase.clone();
        record.last_sample_at = Instant::now();
        record.last_sample_bytes = record.snapshot.bytes_done;
        control_cv().notify_all();
        mark_ui_changed();
        Ok(())
    } else {
        Err("transfer is not active".to_owned())
    }
}

pub(crate) fn cancel(transfer_id: &str) -> Result<(), String> {
    let job = {
        let state = lock_state();
        let record = state
            .records
            .iter()
            .find(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
            .ok_or_else(|| "transfer is not active".to_owned())?;
        if record.snapshot.cancelling {
            return Ok(());
        }
        if record.snapshot.phase.starts_with("Finishing") {
            return Err("transfer is already being committed".to_owned());
        }
        record.job.clone()
    };

    if let TransferJob::Tailscale(command) = &job {
        control_taildrive_job(command, transfer_id, "cancel")
            .map_err(|_| "cannot cancel transfer right now".to_owned())?;
    }

    let mut state = lock_state();
    let Some(record) = state
        .records
        .iter_mut()
        .find(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
    else {
        return Err("transfer is no longer active".to_owned());
    };
    if record.snapshot.phase.starts_with("Finishing") {
        return Err("transfer is already being committed".to_owned());
    }
    record.cancel_requested = true;
    record.pause_override = None;
    record.snapshot.paused = false;
    record.snapshot.cancelling = true;
    record.snapshot.phase = "Cancelling".to_owned();
    mark_ui_changed();
    drop(state);
    control_cv().notify_all();
    Ok(())
}

fn transfer_is_active(transfer_id: &str) -> bool {
    lock_state()
        .records
        .iter()
        .any(|record| record.snapshot.transfer_id == transfer_id && !record.snapshot.done)
}

fn quiesce_all_for_service_timeout() -> Result<(), String> {
    let transfer_ids = {
        let state = lock_state();
        state
            .records
            .iter()
            .filter(|record| !record.snapshot.done)
            .map(|record| record.snapshot.transfer_id.clone())
            .collect::<Vec<_>>()
    };
    let mut failures = Vec::new();
    for transfer_id in transfer_ids {
        if pause(&transfer_id).is_ok() {
            continue;
        }
        if cancel(&transfer_id).is_ok() || !transfer_is_active(&transfer_id) {
            continue;
        }
        failures.push(transfer_id);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("could not safely suspend: {}", failures.join(", ")))
    }
}

fn initial_phase(job: &TransferJob) -> String {
    match job {
        TransferJob::Tailscale(crate::tailscale::Command::TaildriveDownload {
            display_name,
            open_after,
            ..
        }) if *open_after && is_android_install_name(display_name) => {
            "Preparing app install".to_owned()
        }
        TransferJob::Tailscale(crate::tailscale::Command::TaildriveDownload { .. }) => {
            "Downloading".to_owned()
        }
        TransferJob::Tailscale(crate::tailscale::Command::TaildriveUpload { .. }) => {
            "Uploading".to_owned()
        }
        TransferJob::Tailscale(crate::tailscale::Command::TaildriveRelay { .. }) => {
            "Copying".to_owned()
        }
        TransferJob::Tailscale(_) => "Transferring".to_owned(),
        TransferJob::Local(crate::app::LocalFileCommand::CopyMove { cut: true, .. }) => {
            "Moving".to_owned()
        }
        TransferJob::Local(_) => "Copying".to_owned(),
    }
}

pub(crate) fn retry(transfer_id: &str) -> Result<(), String> {
    let (job, phase) = {
        let mut state = lock_state();
        let Some(record) = state
            .records
            .iter_mut()
            .find(|record| record.snapshot.transfer_id == transfer_id && record.snapshot.done)
        else {
            return Err("transfer cannot be retried".to_owned());
        };
        if record.snapshot.error.is_none() && !record.snapshot.cancelled {
            return Err("only failed or cancelled transfers can be retried".to_owned());
        }
        let phase = initial_phase(&record.job);
        record.resume_phase = phase.clone();
        record.snapshot.phase = phase.clone();
        record.snapshot.bytes_done = 0;
        record.snapshot.bytes_total = 0;
        record.snapshot.items_done = 0;
        record.snapshot.items_total = 0;
        record.snapshot.paused = false;
        record.snapshot.cancelling = false;
        record.snapshot.cancelled = false;
        record.snapshot.done = false;
        record.snapshot.error = None;
        record.snapshot.bytes_per_second = 0.0;
        record.pause_override = None;
        record.cancel_requested = false;
        record.last_sample_at = Instant::now();
        record.last_sample_bytes = 0;
        mark_ui_changed();
        (record.job.clone(), phase)
    };
    set_transfer_phase(transfer_id, &phase);
    match job {
        TransferJob::Tailscale(command) => spawn_taildrive_job(transfer_id.to_owned(), command),
        TransferJob::Local(command) => spawn_local_job(transfer_id.to_owned(), command),
    }
    Ok(())
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
fn format_eta(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

fn detail(snapshot: &TransferSnapshot) -> String {
    if let Some(error) = snapshot.error.as_ref() {
        return format!("Failed — {error}");
    }
    if snapshot.cancelled {
        return "Cancelled".to_owned();
    }
    if snapshot.cancelling {
        return "Stopping safely…".to_owned();
    }
    if snapshot.done {
        return if snapshot.bytes_total > 0 {
            format!("Completed — {}", format_size(snapshot.bytes_total))
        } else {
            "Completed".to_owned()
        };
    }
    if snapshot.paused {
        if snapshot.bytes_total > 0 {
            let percent = ((snapshot.bytes_done as f64 / snapshot.bytes_total as f64) * 100.0)
                .clamp(0.0, 100.0)
                .round() as u64;
            return format!(
                "Paused at {percent}% · {} transferred",
                format_size(snapshot.bytes_done)
            );
        }
        return "Paused".to_owned();
    }
    if snapshot.bytes_total > 0 {
        let percent = ((snapshot.bytes_done as f64 / snapshot.bytes_total as f64) * 100.0)
            .clamp(0.0, 100.0)
            .round() as u64;
        let mut text = format!(
            "{percent}% · {} / {}",
            format_size(snapshot.bytes_done),
            format_size(snapshot.bytes_total)
        );
        if snapshot.bytes_per_second > 1.0 && snapshot.bytes_done < snapshot.bytes_total {
            let remaining =
                (snapshot.bytes_total - snapshot.bytes_done) as f64 / snapshot.bytes_per_second;
            text.push_str(&format!(" · {} left", format_eta(remaining)));
        }
        return text;
    }
    if snapshot.items_total > 0 {
        return format!("{} / {} items", snapshot.items_done, snapshot.items_total);
    }
    "Running in background".to_owned()
}

fn notification_snapshot_json() -> String {
    let state = lock_state();
    let active_count = state
        .records
        .iter()
        .filter(|record| !record.snapshot.done)
        .count();
    let primary = state
        .records
        .iter()
        .find(|record| !record.snapshot.done)
        .or_else(|| {
            state.batch_last_finished.as_ref().and_then(|id| {
                state
                    .records
                    .iter()
                    .find(|record| &record.snapshot.transfer_id == id)
            })
        });
    let primary = primary.map(|record| {
        let snapshot = &record.snapshot;
        let percent = if snapshot.bytes_total > 0 {
            ((snapshot.bytes_done as f64 / snapshot.bytes_total as f64) * 100.0)
                .clamp(0.0, 100.0)
                .round() as i64
        } else if snapshot.items_total > 0 {
            ((snapshot.items_done as f64 / snapshot.items_total as f64) * 100.0)
                .clamp(0.0, 100.0)
                .round() as i64
        } else {
            -1
        };
        let finishing = snapshot.phase == "Finishing"
            || snapshot.phase == "Finishing upload"
            || snapshot.phase == "Finishing download";
        serde_json::json!({
            "transfer_id": snapshot.transfer_id,
            "label": snapshot.label,
            "phase": snapshot.phase,
            "detail": detail(snapshot),
            "percent": percent,
            "paused": snapshot.paused,
            "cancelling": snapshot.cancelling,
            "cancelled": snapshot.cancelled,
            "can_pause": !snapshot.done && !snapshot.cancelling && !finishing,
            "can_cancel": !snapshot.done && !snapshot.cancelling && !finishing,
            "can_retry": snapshot.done && (snapshot.error.is_some() || snapshot.cancelled),
            "error": snapshot.error.as_deref().unwrap_or_default(),
        })
    });
    serde_json::json!({
        "active_count": active_count,
        "failed_count": if active_count == 0 { state.batch_failed_count } else { 0 },
        "primary": primary,
    })
    .to_string()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_oligami_fastexplorer_FastExplorerTransferService_nativeTransferSnapshotJson<
    'local,
>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let json = notification_snapshot_json();
    let outcome = unowned_env.with_env(|env| -> Result<JString<'local>, jni::errors::Error> {
        JString::from_str(env, json)
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_oligami_fastexplorer_FastExplorerTransferService_nativeDrainFileChangesJson<
    'local,
>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let json = drain_system_file_changes_json();
    let outcome = unowned_env.with_env(|env| -> Result<JString<'local>, jni::errors::Error> {
        JString::from_str(env, json)
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_oligami_fastexplorer_FastExplorerTransferService_nativeUpdateNetworkInterfaces<
    'local,
>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: JClass<'local>,
    value: JString<'local>,
) {
    let outcome = unowned_env.with_env(|env| -> Result<(), jni::errors::Error> {
        let value = value.try_to_string(env)?;
        if let Err(error) = crate::tailscale::set_android_interfaces_json(&value) {
            eprintln!("FastExplorer: transfer service network update failed: {error}");
        }
        Ok(())
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_oligami_fastexplorer_FastExplorerTransferService_nativeControlTransfer<
    'local,
>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: JClass<'local>,
    transfer_id: JString<'local>,
    action: JString<'local>,
) -> JString<'local> {
    let outcome = unowned_env.with_env(|env| -> Result<JString<'local>, jni::errors::Error> {
        let transfer_id = transfer_id.try_to_string(env)?;
        let action = action.try_to_string(env)?;
        let result = match action.as_str() {
            "pause" => pause(&transfer_id),
            "resume" => resume(&transfer_id),
            "cancel" => cancel(&transfer_id),
            "retry" => retry(&transfer_id),
            "quiesce_all" => quiesce_all_for_service_timeout(),
            _ => Err(format!("unsupported transfer action: {action}")),
        };
        JString::from_str(env, result.err().unwrap_or_default())
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, UNIX_EPOCH};

    use super::{
        TransferJob, copy_local_path_controlled, finish_transfer, merge_staging_entry_controlled,
        reconcile_pause_progress, register_transfer,
    };

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn stale_paused_progress_cannot_undo_resume() {
        let mut pause_override = Some(false);
        assert!(!reconcile_pause_progress(&mut pause_override, true));
        assert_eq!(pause_override, Some(false));
        assert!(!reconcile_pause_progress(&mut pause_override, false));
        assert_eq!(pause_override, None);
    }

    #[test]
    fn stale_running_progress_cannot_undo_pause() {
        let mut pause_override = Some(true);
        assert!(reconcile_pause_progress(&mut pause_override, false));
        assert_eq!(pause_override, Some(true));
        assert!(reconcile_pause_progress(&mut pause_override, true));
        assert_eq!(pause_override, None);
    }

    #[test]
    fn existing_directory_merge_moves_staging_entries_without_second_copy() {
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "fast-explorer-android-merge-test-{}-{sequence}",
            std::process::id()
        ));
        let staging = root.join("staging");
        let destination = root.join("destination");
        fs::create_dir_all(staging.join("nested")).expect("create staging");
        fs::create_dir_all(destination.join("nested")).expect("create destination");
        fs::write(staging.join("new.txt"), b"new").expect("new file");
        fs::write(staging.join("replace.txt"), b"replacement").expect("replacement file");
        fs::write(staging.join("nested/replace.txt"), b"nested replacement")
            .expect("nested replacement");
        fs::write(destination.join("keep.txt"), b"keep").expect("keep file");
        fs::write(destination.join("replace.txt"), b"old").expect("old file");
        fs::write(destination.join("nested/keep.txt"), b"nested keep").expect("nested keep");
        fs::write(destination.join("nested/replace.txt"), b"nested old").expect("nested old");

        let moved_inode = fs::metadata(staging.join("new.txt"))
            .expect("staging metadata")
            .ino();
        let transfer_id = format!("android-merge-test-{sequence}");
        register_transfer(
            transfer_id.clone(),
            "merge test".to_owned(),
            "Merging".to_owned(),
            BTreeSet::new(),
            TransferJob::Local(crate::app::LocalFileCommand::CopyMove {
                current: root.clone(),
                source: root.join("source"),
                destination: destination.clone(),
                cut: false,
                replace: true,
            }),
        );

        let result = merge_staging_entry_controlled(&transfer_id, &staging, &destination);
        finish_transfer(&transfer_id, result.as_ref().err().cloned());
        result.expect("merge staging directory");

        assert!(!staging.exists());
        assert_eq!(fs::read(destination.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(
            fs::read(destination.join("replace.txt")).unwrap(),
            b"replacement"
        );
        assert_eq!(
            fs::read(destination.join("nested/keep.txt")).unwrap(),
            b"nested keep"
        );
        assert_eq!(
            fs::read(destination.join("nested/replace.txt")).unwrap(),
            b"nested replacement"
        );
        assert_eq!(
            fs::metadata(destination.join("new.txt")).unwrap().ino(),
            moved_inode,
            "publishing should preserve the staged inode instead of copying bytes again"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn controlled_copy_preserves_directory_symlinks() {
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "fast-explorer-android-symlink-test-{}-{sequence}",
            std::process::id()
        ));
        let source_root = root.join("source");
        let target_dir = source_root.join("target");
        let link = source_root.join("link");
        let destination = root.join("copied-link");
        fs::create_dir_all(&target_dir).expect("create symlink target");
        fs::write(target_dir.join("child.txt"), b"child").expect("target child");
        symlink("target", &link).expect("create directory symlink");

        let transfer_id = format!("android-symlink-test-{sequence}");
        register_transfer(
            transfer_id.clone(),
            "symlink test".to_owned(),
            "Copying".to_owned(),
            BTreeSet::new(),
            TransferJob::Local(crate::app::LocalFileCommand::CopyMove {
                current: root.clone(),
                source: link.clone(),
                destination: destination.clone(),
                cut: false,
                replace: false,
            }),
        );
        let (bytes_total, items_total) = super::measure_local_path(&link).expect("measure link");
        let mut bytes_done = 0;
        let mut items_done = 0;
        let result = copy_local_path_controlled(
            &transfer_id,
            &link,
            &destination,
            &mut bytes_done,
            &mut items_done,
            bytes_total,
            items_total,
        );
        finish_transfer(&transfer_id, result.as_ref().err().cloned());
        result.expect("copy directory symlink");

        assert!(
            fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(&destination).unwrap(),
            std::path::PathBuf::from("target")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn controlled_copy_preserves_supported_file_metadata() {
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "fast-explorer-android-metadata-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create metadata test root");
        let source = root.join("source.txt");
        let destination = root.join("destination.txt");
        fs::write(&source, b"metadata").expect("write metadata source");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).expect("set source mode");
        let expected_modified = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        fs::File::open(&source)
            .expect("open metadata source")
            .set_times(std::fs::FileTimes::new().set_modified(expected_modified))
            .expect("set source mtime");

        let transfer_id = format!("android-metadata-test-{sequence}");
        register_transfer(
            transfer_id.clone(),
            "metadata test".to_owned(),
            "Copying".to_owned(),
            BTreeSet::new(),
            TransferJob::Local(crate::app::LocalFileCommand::CopyMove {
                current: root.clone(),
                source: source.clone(),
                destination: destination.clone(),
                cut: false,
                replace: false,
            }),
        );
        let (bytes_total, items_total) =
            super::measure_local_path(&source).expect("measure source");
        let mut bytes_done = 0;
        let mut items_done = 0;
        let result = copy_local_path_controlled(
            &transfer_id,
            &source,
            &destination,
            &mut bytes_done,
            &mut items_done,
            bytes_total,
            items_total,
        );
        finish_transfer(&transfer_id, result.as_ref().err().cloned());
        result.expect("copy metadata source");

        let source_metadata = fs::metadata(&source).unwrap();
        let destination_metadata = fs::metadata(&destination).unwrap();
        assert_eq!(
            destination_metadata.permissions().mode() & 0o777,
            source_metadata.permissions().mode() & 0o777
        );
        assert_eq!(
            destination_metadata.modified().unwrap(),
            source_metadata.modified().unwrap()
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn metadata_timestamps_survive_restrictive_source_permissions() {
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "fast-explorer-android-restrictive-metadata-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create restrictive metadata test root");
        let source = root.join("source.txt");
        let destination = root.join("destination.txt");
        fs::write(&source, b"source").expect("write metadata source");
        fs::write(&destination, b"destination").expect("write metadata destination");
        let expected_modified = UNIX_EPOCH + Duration::from_secs(1_650_000_000);
        fs::File::open(&source)
            .expect("open metadata source")
            .set_times(std::fs::FileTimes::new().set_modified(expected_modified))
            .expect("set source mtime");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o200))
            .expect("set restrictive source mode");
        let source_metadata = fs::metadata(&source).expect("source metadata");

        super::apply_local_metadata(&source_metadata, &destination);

        let destination_metadata = fs::metadata(&destination).expect("destination metadata");
        assert_eq!(destination_metadata.permissions().mode() & 0o777, 0o200);
        assert_eq!(destination_metadata.modified().unwrap(), expected_modified);
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).ok();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o600)).ok();
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn type_conflict_replace_removes_internal_backup() {
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "fast-explorer-android-backup-test-{}-{sequence}",
            std::process::id()
        ));
        let staging = root.join("staging-file");
        let destination = root.join("destination");
        fs::create_dir_all(destination.join("old-tree")).expect("create old destination");
        fs::write(destination.join("old-tree/data.txt"), b"old").expect("old data");
        fs::write(&staging, b"replacement").expect("staged replacement");

        let transfer_id = format!("android-backup-test-{sequence}");
        register_transfer(
            transfer_id.clone(),
            "backup test".to_owned(),
            "Merging".to_owned(),
            BTreeSet::new(),
            TransferJob::Local(crate::app::LocalFileCommand::CopyMove {
                current: root.clone(),
                source: staging.clone(),
                destination: destination.clone(),
                cut: false,
                replace: true,
            }),
        );
        let result = merge_staging_entry_controlled(&transfer_id, &staging, &destination);
        finish_transfer(&transfer_id, result.as_ref().err().cloned());
        result.expect("replace directory with file");
        assert_eq!(fs::read(&destination).unwrap(), b"replacement");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let leftovers = loop {
            let leftovers = fs::read_dir(&root)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(".fastexplorer-replace-backup-"))
                .collect::<Vec<_>>();
            if leftovers.is_empty() || std::time::Instant::now() >= deadline {
                break leftovers;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            leftovers.is_empty(),
            "unexpected replace backups: {leftovers:?}"
        );
        fs::remove_dir_all(root).ok();
    }
}
