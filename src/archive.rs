use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const ARCHIVE_ROOT_COMPONENT: &str = "__fast_explorer_archive__";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchiveLocation {
    pub archive_path: PathBuf,
    pub inner_path: String,
}

#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub name: String,
    pub directory: bool,
    pub size: u64,
    pub modified_sort_key: u64,
}

fn zip_datetime_sort_key(value: zip::DateTime) -> u64 {
    u64::from(value.year()) * 10_000_000_000
        + u64::from(value.month()) * 100_000_000
        + u64::from(value.day()) * 1_000_000
        + u64::from(value.hour()) * 10_000
        + u64::from(value.minute()) * 100
        + u64::from(value.second())
}

fn hex_encode(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex_decode(value: &str) -> Option<String> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(pair).ok()?;
        bytes.push(u8::from_str_radix(text, 16).ok()?);
    }
    String::from_utf8(bytes).ok()
}

fn normalize_inner(value: &str) -> String {
    value
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .collect::<Vec<_>>()
        .join("/")
}

pub fn virtual_path(location: &ArchiveLocation) -> PathBuf {
    let mut path = PathBuf::from(ARCHIVE_ROOT_COMPONENT);
    path.push(hex_encode(&location.archive_path.to_string_lossy()));
    for component in normalize_inner(&location.inner_path).split('/') {
        if !component.is_empty() {
            path.push(hex_encode(component));
        }
    }
    path
}

pub fn parse_virtual_path(path: &Path) -> Option<ArchiveLocation> {
    let mut components = path.components();
    let root = components.next()?.as_os_str().to_string_lossy();
    if root != ARCHIVE_ROOT_COMPONENT {
        return None;
    }
    let archive_path = PathBuf::from(hex_decode(
        &components.next()?.as_os_str().to_string_lossy(),
    )?);
    let mut inner = Vec::new();
    for component in components {
        inner.push(hex_decode(&component.as_os_str().to_string_lossy())?);
    }
    Some(ArchiveLocation {
        archive_path,
        inner_path: normalize_inner(&inner.join("/")),
    })
}

pub fn display_path(location: &ArchiveLocation) -> String {
    let archive = location.archive_path.to_string_lossy().replace('\\', "/");
    let inner = normalize_inner(&location.inner_path);
    if inner.is_empty() {
        format!("{archive}!/")
    } else {
        format!("{archive}!/{inner}")
    }
}

pub fn parse_display_path(value: &str) -> Option<ArchiveLocation> {
    let normalized = value.trim().replace('\\', "/");
    let (archive, inner) = normalized.split_once("!/")?;
    let archive_path = PathBuf::from(archive);
    is_supported_archive(&archive_path).then_some(ArchiveLocation {
        archive_path,
        inner_path: normalize_inner(inner),
    })
}

pub fn parent_path(location: &ArchiveLocation) -> Option<PathBuf> {
    let inner = normalize_inner(&location.inner_path);
    if inner.is_empty() {
        return location.archive_path.parent().map(Path::to_path_buf);
    }
    let mut parts = inner.split('/').collect::<Vec<_>>();
    parts.pop();
    Some(virtual_path(&ArchiveLocation {
        archive_path: location.archive_path.clone(),
        inner_path: parts.join("/"),
    }))
}

pub fn child_location(parent: &ArchiveLocation, name: &str) -> ArchiveLocation {
    let inner = normalize_inner(&parent.inner_path);
    ArchiveLocation {
        archive_path: parent.archive_path.clone(),
        inner_path: if inner.is_empty() {
            normalize_inner(name)
        } else {
            format!("{inner}/{}", normalize_inner(name))
        },
    }
}

pub fn is_supported_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

pub fn title(location: &ArchiveLocation) -> String {
    normalize_inner(&location.inner_path)
        .rsplit('/')
        .find(|part| !part.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            location
                .archive_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "Archive".to_owned())
}

pub fn list(location: &ArchiveLocation, show_hidden: bool) -> Result<Vec<ArchiveEntry>, String> {
    let file = File::open(&location.archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|error| error.to_string())?;
    let inner = normalize_inner(&location.inner_path);
    let prefix = if inner.is_empty() {
        String::new()
    } else {
        format!("{inner}/")
    };
    let mut found: BTreeMap<String, ArchiveEntry> = BTreeMap::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().replace('\\', "/");
        let Some(remainder) = name.strip_prefix(&prefix) else {
            continue;
        };
        let remainder = remainder.trim_matches('/');
        if remainder.is_empty() {
            continue;
        }
        let mut parts = remainder.split('/');
        let first = parts.next().unwrap_or_default();
        if first.is_empty() || (!show_hidden && first.starts_with('.')) {
            continue;
        }
        let nested = parts.next().is_some();
        let directory = nested || entry.is_dir();
        let size = if directory { 0 } else { entry.size() };
        let modified_sort_key = entry
            .last_modified()
            .map(zip_datetime_sort_key)
            .unwrap_or(0);
        found
            .entry(first.to_owned())
            .and_modify(|current| {
                current.directory |= directory;
                current.modified_sort_key = current.modified_sort_key.max(modified_sort_key);
            })
            .or_insert_with(|| ArchiveEntry {
                name: first.to_owned(),
                directory,
                size,
                modified_sort_key,
            });
    }
    let mut entries = found.into_values().collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

pub fn extract_member(location: &ArchiveLocation, destination: &Path) -> Result<(), String> {
    let file = File::open(&location.archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|error| error.to_string())?;
    let inner = normalize_inner(&location.inner_path);
    if inner.is_empty() {
        return Err("archive root is not a file".to_owned());
    }
    let mut member = archive.by_name(&inner).map_err(|error| error.to_string())?;
    if member.is_dir() {
        return Err("archive member is a directory".to_owned());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut output = File::create(destination).map_err(|error| error.to_string())?;
    std::io::copy(&mut member, &mut output).map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())?;
    Ok(())
}

fn unique_sibling(path: &Path, label: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    parent.join(format!(".{name}.fastexplorer-{label}-{stamp}-{id}"))
}

fn replace_archive_file(original: &Path, replacement: &Path) -> Result<(), String> {
    let backup = unique_sibling(original, "backup");
    fs::rename(original, &backup).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(replacement, original) {
        let _ = fs::rename(&backup, original);
        return Err(error.to_string());
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

pub fn replace_member_from_file(location: &ArchiveLocation, source: &Path) -> Result<(), String> {
    let member_name = normalize_inner(&location.inner_path);
    if member_name.is_empty() {
        return Err("cannot replace the archive root".to_owned());
    }
    let input = File::open(&location.archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(input).map_err(|error| error.to_string())?;
    let temporary = unique_sibling(&location.archive_path, "rewrite");
    let output = File::create(&temporary).map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new(output);
    let mut found = false;
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|error| error.to_string())?;
        if normalize_inner(file.name()) == member_name {
            found = true;
            continue;
        }
        writer
            .raw_copy_file(file)
            .map_err(|error| error.to_string())?;
    }
    if !found {
        let _ = fs::remove_file(&temporary);
        return Err(format!("archive member no longer exists: {member_name}"));
    }
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer
        .start_file(&member_name, options)
        .map_err(|error| error.to_string())?;
    let mut edited = File::open(source).map_err(|error| error.to_string())?;
    std::io::copy(&mut edited, &mut writer).map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    replace_archive_file(&location.archive_path, &temporary)
}

pub fn delete_member(location: &ArchiveLocation) -> Result<(), String> {
    let target = normalize_inner(&location.inner_path);
    if target.is_empty() {
        return Err("cannot delete the archive root".to_owned());
    }
    rewrite_filtered(&location.archive_path, |name| {
        let normalized = normalize_inner(name);
        normalized != target && !normalized.starts_with(&format!("{target}/"))
    })
}

fn rewrite_filtered(archive_path: &Path, mut keep: impl FnMut(&str) -> bool) -> Result<(), String> {
    let input = File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(input).map_err(|error| error.to_string())?;
    let temporary = unique_sibling(archive_path, "rewrite");
    let output = File::create(&temporary).map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new(output);
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|error| error.to_string())?;
        if keep(file.name()) {
            writer
                .raw_copy_file(file)
                .map_err(|error| error.to_string())?;
        }
    }
    writer.finish().map_err(|error| error.to_string())?;
    replace_archive_file(archive_path, &temporary)
}

pub fn rename_member(location: &ArchiveLocation, new_name: &str) -> Result<(), String> {
    let old = normalize_inner(&location.inner_path);
    if old.is_empty() {
        return Err("cannot rename the archive root".to_owned());
    }
    let parent = old.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("");
    let clean_name = normalize_inner(new_name);
    if clean_name.is_empty() || clean_name.contains('/') {
        return Err("invalid archive member name".to_owned());
    }
    let renamed = if parent.is_empty() {
        clean_name
    } else {
        format!("{parent}/{clean_name}")
    };
    let input = File::open(&location.archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(input).map_err(|error| error.to_string())?;
    let temporary = unique_sibling(&location.archive_path, "rename");
    let output = File::create(&temporary).map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new(output);
    let prefix = format!("{old}/");
    let mut found = false;
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|error| error.to_string())?;
        let normalized = normalize_inner(file.name());
        let target_name = if normalized == old {
            found = true;
            renamed.clone()
        } else if let Some(suffix) = normalized.strip_prefix(&prefix) {
            found = true;
            format!("{renamed}/{suffix}")
        } else {
            normalized
        };
        writer
            .raw_copy_file_rename(file, target_name)
            .map_err(|error| error.to_string())?;
    }
    if !found {
        let _ = fs::remove_file(&temporary);
        return Err(format!("archive member no longer exists: {old}"));
    }
    writer.finish().map_err(|error| error.to_string())?;
    replace_archive_file(&location.archive_path, &temporary)
}

pub fn create_directory(parent: &ArchiveLocation, name: &str) -> Result<(), String> {
    let location = child_location(parent, name);
    let mut member = normalize_inner(&location.inner_path);
    if member.is_empty() {
        return Err("invalid archive directory name".to_owned());
    }
    member.push('/');
    let input = File::options()
        .read(true)
        .write(true)
        .open(&parent.archive_path)
        .map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new_append(input).map_err(|error| error.to_string())?;
    writer
        .add_directory(member, SimpleFileOptions::default())
        .map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(())
}

pub fn import_file(parent: &ArchiveLocation, source: &Path, name: &str) -> Result<(), String> {
    let location = child_location(parent, name);
    let member = normalize_inner(&location.inner_path);
    if member.is_empty() {
        return Err("invalid archive member name".to_owned());
    }
    let input = File::options()
        .read(true)
        .write(true)
        .open(&parent.archive_path)
        .map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new_append(input).map_err(|error| error.to_string())?;
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer
        .start_file(&member, options)
        .map_err(|error| error.to_string())?;
    let mut input = File::open(source).map_err(|error| error.to_string())?;
    std::io::copy(&mut input, &mut writer).map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(())
}

#[derive(Debug, Clone)]
pub enum Command {
    List {
        generation: u64,
        location: ArchiveLocation,
        show_hidden: bool,
    },
    OpenForEdit {
        location: ArchiveLocation,
        destination: PathBuf,
        share_after: bool,
    },
    Delete {
        current: ArchiveLocation,
        target: ArchiveLocation,
    },
    Rename {
        current: ArchiveLocation,
        target: ArchiveLocation,
        new_name: String,
    },
    Mkdir {
        current: ArchiveLocation,
        name: String,
    },
    Import {
        current: ArchiveLocation,
        source: PathBuf,
        name: String,
        replace: bool,
    },
    Export {
        source: ArchiveLocation,
        destination: PathBuf,
        target_location: PathBuf,
        target_name: String,
        size: u64,
    },
    CopyMember {
        current: ArchiveLocation,
        source: ArchiveLocation,
        name: String,
        replace: bool,
    },
}

#[derive(Debug)]
pub enum Event {
    Listed {
        generation: u64,
        location: ArchiveLocation,
        result: Result<Vec<ArchiveEntry>, String>,
    },
    Opened {
        location: ArchiveLocation,
        destination: PathBuf,
        share_after: bool,
        result: Result<(), String>,
    },
    Mutated {
        current: ArchiveLocation,
        action: String,
        result: Result<(), String>,
    },
    EditSynced {
        location: ArchiveLocation,
        result: Result<(), String>,
    },
    Exported {
        target_location: PathBuf,
        destination: PathBuf,
        target_name: String,
        size: u64,
        result: Result<(), String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSignature {
    modified: Option<SystemTime>,
    len: u64,
}

fn signature(path: &Path) -> Option<FileSignature> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileSignature {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

#[derive(Debug, Clone)]
struct EditWatch {
    location: ArchiveLocation,
    path: PathBuf,
    observed: FileSignature,
    pending: Option<FileSignature>,
}

pub async fn run_worker(
    proxy: xilem::core::MessageProxy<Event>,
    mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<Command>,
) {
    let mut watches: BTreeMap<PathBuf, EditWatch> = BTreeMap::new();
    let mut interval = xilem::tokio::time::interval(std::time::Duration::from_secs(2));
    interval.set_missed_tick_behavior(xilem::tokio::time::MissedTickBehavior::Skip);
    loop {
        xilem::tokio::select! {
            command = rx.recv() => {
                let Some(command) = command else { break; };
                handle_command(&proxy, &mut watches, command).await;
            }
            _ = interval.tick() => {
                poll_edit_watches(&proxy, &mut watches).await;
            }
        }
    }
}

async fn handle_command(
    proxy: &xilem::core::MessageProxy<Event>,
    watches: &mut BTreeMap<PathBuf, EditWatch>,
    command: Command,
) {
    match command {
        Command::List {
            generation,
            location,
            show_hidden,
        } => {
            let worker_location = location.clone();
            let result =
                xilem::tokio::task::spawn_blocking(move || list(&worker_location, show_hidden))
                    .await
                    .unwrap_or_else(|error| Err(format!("archive worker failed: {error}")));
            let _ = proxy.message(Event::Listed {
                generation,
                location,
                result,
            });
        }

        Command::OpenForEdit {
            location,
            destination,
            share_after,
        } => {
            let worker_location = location.clone();
            let worker_destination = destination.clone();
            let result = xilem::tokio::task::spawn_blocking(move || {
                extract_member(&worker_location, &worker_destination)
            })
            .await
            .unwrap_or_else(|error| Err(format!("archive worker failed: {error}")));
            if result.is_ok()
                && !share_after
                && let Some(observed) = signature(&destination)
            {
                watches.insert(
                    destination.clone(),
                    EditWatch {
                        location: location.clone(),
                        path: destination.clone(),
                        observed,
                        pending: None,
                    },
                );
            }
            let _ = proxy.message(Event::Opened {
                location,
                destination,
                share_after,
                result,
            });
        }
        Command::Delete { current, target } => {
            let label = target.inner_path.clone();
            let result = xilem::tokio::task::spawn_blocking(move || delete_member(&target))
                .await
                .unwrap_or_else(|error| Err(format!("archive worker failed: {error}")));
            let _ = proxy.message(Event::Mutated {
                current,
                action: format!("Deleted {label}"),
                result,
            });
        }
        Command::Rename {
            current,
            target,
            new_name,
        } => {
            let action = format!("Renamed {}", target.inner_path);
            let result =
                xilem::tokio::task::spawn_blocking(move || rename_member(&target, &new_name))
                    .await
                    .unwrap_or_else(|error| Err(format!("archive worker failed: {error}")));
            let _ = proxy.message(Event::Mutated {
                current,
                action,
                result,
            });
        }
        Command::Mkdir { current, name } => {
            let worker_current = current.clone();
            let action = format!("Created {name}");
            let result = xilem::tokio::task::spawn_blocking(move || {
                create_directory(&worker_current, &name)
            })
            .await
            .unwrap_or_else(|error| Err(format!("archive worker failed: {error}")));
            let _ = proxy.message(Event::Mutated {
                current,
                action,
                result,
            });
        }
        Command::Import {
            current,
            source,
            name,
            replace,
        } => {
            let worker_current = current.clone();
            let action = if replace {
                format!("Replaced {name}")
            } else {
                format!("Added {name}")
            };
            let result = xilem::tokio::task::spawn_blocking(move || {
                if replace {
                    let target = child_location(&worker_current, &name);
                    replace_member_from_file(&target, &source)
                } else {
                    import_file(&worker_current, &source, &name)
                }
            })
            .await
            .unwrap_or_else(|error| Err(format!("archive worker failed: {error}")));
            let _ = proxy.message(Event::Mutated {
                current,
                action,
                result,
            });
        }
        Command::Export {
            source,
            destination,
            target_location,
            target_name,
            size,
        } => {
            let worker_source = source.clone();
            let worker_destination = destination.clone();
            let result = xilem::tokio::task::spawn_blocking(move || {
                extract_member(&worker_source, &worker_destination)
            })
            .await
            .unwrap_or_else(|error| Err(format!("archive worker failed: {error}")));
            let _ = proxy.message(Event::Exported {
                target_location,
                destination,
                target_name,
                size,
                result,
            });
        }
        Command::CopyMember {
            current,
            source,
            name,
            replace,
        } => {
            let worker_current = current.clone();
            let action = if replace {
                format!("Replaced {name}")
            } else {
                format!("Copied {name}")
            };
            let result = xilem::tokio::task::spawn_blocking(move || {
                if replace {
                    let target = child_location(&worker_current, &name);
                    let temporary = unique_sibling(&worker_current.archive_path, "copy-replace");
                    if let Err(error) = extract_member(&source, &temporary) {
                        let _ = fs::remove_file(&temporary);
                        return Err(error);
                    }
                    let result = replace_member_from_file(&target, &temporary);
                    let _ = fs::remove_file(&temporary);
                    result
                } else {
                    copy_member_into_archive(&source, &worker_current, &name)
                }
            })
            .await
            .unwrap_or_else(|error| Err(format!("archive worker failed: {error}")));
            let _ = proxy.message(Event::Mutated {
                current,
                action,
                result,
            });
        }
    }
}

async fn poll_edit_watches(
    proxy: &xilem::core::MessageProxy<Event>,
    watches: &mut BTreeMap<PathBuf, EditWatch>,
) {
    let paths = watches.keys().cloned().collect::<Vec<_>>();
    for path in paths {
        let Some(mut watch) = watches.get(&path).cloned() else {
            continue;
        };
        let check_path = path.clone();
        let current = xilem::tokio::task::spawn_blocking(move || signature(&check_path))
            .await
            .ok()
            .flatten();
        let Some(current) = current else {
            continue;
        };
        if current == watch.observed {
            watch.pending = None;
            watches.insert(path, watch);
            continue;
        }
        if watch.pending.as_ref() != Some(&current) {
            watch.pending = Some(current);
            watches.insert(path, watch);
            continue;
        }
        let location = watch.location.clone();
        let worker_location = location.clone();
        let source = watch.path.clone();
        let result = xilem::tokio::task::spawn_blocking(move || {
            replace_member_from_file(&worker_location, &source)
        })
        .await
        .unwrap_or_else(|error| Err(format!("archive worker failed: {error}")));
        if result.is_ok() {
            watch.observed = current;
            watch.pending = None;
            watches.insert(path, watch);
        }
        let _ = proxy.message(Event::EditSynced { location, result });
    }
}

pub fn copy_member_into_archive(
    source: &ArchiveLocation,
    target_parent: &ArchiveLocation,
    name: &str,
) -> Result<(), String> {
    let source_name = normalize_inner(&source.inner_path);
    if source_name.is_empty() {
        return Err("cannot copy the archive root".to_owned());
    }
    let mut target_name = normalize_inner(&target_parent.inner_path);
    if !target_name.is_empty() {
        target_name.push('/');
    }
    target_name.push_str(&normalize_inner(name));
    if target_name.ends_with('/') || target_name.is_empty() {
        return Err("invalid target archive member name".to_owned());
    }
    if source.archive_path == target_parent.archive_path {
        let input = File::open(&source.archive_path).map_err(|error| error.to_string())?;
        let mut archive = ZipArchive::new(input).map_err(|error| error.to_string())?;
        let temporary = unique_sibling(&source.archive_path, "copy-member");
        let output = File::create(&temporary).map_err(|error| error.to_string())?;
        let mut writer = ZipWriter::new(output);
        for index in 0..archive.len() {
            let file = archive.by_index(index).map_err(|error| error.to_string())?;
            writer
                .raw_copy_file(file)
                .map_err(|error| error.to_string())?;
        }
        let file = archive
            .by_name(&source_name)
            .map_err(|error| error.to_string())?;
        if file.is_dir() {
            let _ = fs::remove_file(&temporary);
            return Err("copying archive directories is not supported yet".to_owned());
        }
        writer
            .raw_copy_file_rename(file, target_name)
            .map_err(|error| error.to_string())?;
        writer.finish().map_err(|error| error.to_string())?;
        return replace_archive_file(&source.archive_path, &temporary);
    }

    let input = File::open(&source.archive_path).map_err(|error| error.to_string())?;
    let mut source_archive = ZipArchive::new(input).map_err(|error| error.to_string())?;
    let source_file = source_archive
        .by_name(&source_name)
        .map_err(|error| error.to_string())?;
    if source_file.is_dir() {
        return Err("copying archive directories is not supported yet".to_owned());
    }
    let target = File::options()
        .read(true)
        .write(true)
        .open(&target_parent.archive_path)
        .map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new_append(target).map_err(|error| error.to_string())?;
    writer
        .raw_copy_file_rename(source_file, target_name)
        .map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    fn sandbox() -> PathBuf {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("fast-explorer-archive-test-{id}"));
        fs::create_dir_all(&root).expect("sandbox");
        root
    }

    fn fixture(root: &Path) -> PathBuf {
        let path = root.join("fixture.zip");
        let output = File::create(&path).expect("zip");
        let mut writer = ZipWriter::new(output);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer.add_directory("docs/", options).expect("dir");
        writer.start_file("docs/a.txt", options).expect("a");
        writer.write_all(b"alpha").expect("write a");
        writer.start_file("root.txt", options).expect("root");
        writer.write_all(b"root").expect("write root");
        writer.finish().expect("finish");
        path
    }

    fn read_member(path: &Path, name: &str) -> Vec<u8> {
        let file = File::open(path).expect("zip");
        let mut archive = ZipArchive::new(file).expect("archive");
        let mut member = archive.by_name(name).expect("member");
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).expect("read");
        bytes
    }

    #[test]
    fn list_and_edit_member_preserves_other_entries() {
        let root = sandbox();
        let archive_path = fixture(&root);
        let archive_root = ArchiveLocation {
            archive_path: archive_path.clone(),
            inner_path: String::new(),
        };
        let entries = list(&archive_root, false).expect("list");
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "docs" && entry.directory)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "root.txt" && !entry.directory)
        );

        let member = ArchiveLocation {
            archive_path: archive_path.clone(),
            inner_path: "docs/a.txt".to_owned(),
        };
        let edited = root.join("edited.txt");
        fs::write(&edited, b"edited alpha").expect("edit");
        replace_member_from_file(&member, &edited).expect("replace");
        assert_eq!(read_member(&archive_path, "docs/a.txt"), b"edited alpha");
        assert_eq!(read_member(&archive_path, "root.txt"), b"root");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn archive_rename_copy_delete_and_import_work_without_extracting_whole_archive() {
        let root = sandbox();
        let archive_path = fixture(&root);
        let docs = ArchiveLocation {
            archive_path: archive_path.clone(),
            inner_path: "docs".to_owned(),
        };
        let source = ArchiveLocation {
            archive_path: archive_path.clone(),
            inner_path: "docs/a.txt".to_owned(),
        };
        copy_member_into_archive(&source, &docs, "copy.txt").expect("copy");
        assert_eq!(read_member(&archive_path, "docs/copy.txt"), b"alpha");
        rename_member(&source, "renamed.txt").expect("rename");
        assert_eq!(read_member(&archive_path, "docs/renamed.txt"), b"alpha");
        let copied = ArchiveLocation {
            archive_path: archive_path.clone(),
            inner_path: "docs/copy.txt".to_owned(),
        };
        delete_member(&copied).expect("delete");
        let file = File::open(&archive_path).expect("zip");
        let mut zip = ZipArchive::new(file).expect("archive");
        assert!(zip.by_name("docs/copy.txt").is_err());
        drop(zip);

        let imported = root.join("new.txt");
        fs::write(&imported, b"new").expect("import fixture");
        import_file(&docs, &imported, "new.txt").expect("import");
        assert_eq!(read_member(&archive_path, "docs/new.txt"), b"new");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn virtual_path_round_trips() {
        let location = ArchiveLocation {
            archive_path: PathBuf::from("/tmp/example archive.zip"),
            inner_path: "日本語/with space.txt".to_owned(),
        };
        assert_eq!(parse_virtual_path(&virtual_path(&location)), Some(location));
    }
}
