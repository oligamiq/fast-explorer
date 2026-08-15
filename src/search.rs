use std::fs;
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::process::Command;

use crate::app::{EntryKind, FileEntry};
use crate::settings::SearchMode;

pub const SEARCH_RESULT_LIMIT: usize = 500;

#[cfg(target_os = "windows")]
pub fn everything_available() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|directory| directory.join("es.exe").is_file())
        })
    })
}

#[cfg(not(target_os = "windows"))]
pub const fn everything_available() -> bool {
    false
}

pub fn search(
    mode: SearchMode,
    root: &Path,
    query: &str,
    show_hidden: bool,
) -> Result<Vec<FileEntry>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    match mode {
        SearchMode::Default => default_search(root, query, show_hidden),
        SearchMode::Everything => everything_search(root, query, show_hidden),
    }
}

fn default_search(root: &Path, query: &str, show_hidden: bool) -> Result<Vec<FileEntry>, String> {
    let needle = query.to_lowercase();
    let mut pending = vec![root.to_path_buf()];
    let mut results = Vec::new();
    while let Some(dir) = pending.pop() {
        let read_dir = match fs::read_dir(&dir) {
            Ok(read_dir) => read_dir,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
            else {
                continue;
            };
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() && !file_type.is_symlink() {
                pending.push(path.clone());
            }
            if name.to_lowercase().contains(&needle)
                && let Some(item) = result_entry(root, path, show_hidden)
            {
                results.push(item);
                if results.len() >= SEARCH_RESULT_LIMIT {
                    return Ok(results);
                }
            }
        }
    }
    results.sort_by_key(|entry| entry.name.to_lowercase());
    Ok(results)
}

#[cfg(any(target_os = "windows", test))]
fn everything_cli_search_args(query: &str) -> [String; 2] {
    // ES supports `--` to disable switch parsing for all following arguments.
    // Keep the user's Everything search expression byte-for-byte unchanged.
    ["--".to_owned(), query.to_owned()]
}

#[cfg(target_os = "windows")]
fn everything_search(
    root: &Path,
    query: &str,
    show_hidden: bool,
) -> Result<Vec<FileEntry>, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let output_file = std::env::temp_dir().join(format!(
        "fast-explorer-everything-{}-{stamp}.txt",
        std::process::id()
    ));
    #[cfg(target_os = "windows")]
    use std::os::windows::process::CommandExt;
    let mut command = Command::new("es.exe");
    command.creation_flags(0x08000000);
    let output = command
        .arg("-argv")
        .arg("-path")
        .arg(root)
        .arg("-n")
        .arg(SEARCH_RESULT_LIMIT.to_string())
        .arg("-export-txt")
        .arg(&output_file)
        .args(everything_cli_search_args(query))
        .output()
        .map_err(|error| format!("Everything ES is unavailable: {error}"))?;
    if !output.status.success() {
        let _ = fs::remove_file(&output_file);
        return Err(format!(
            "Everything search failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = fs::read_to_string(&output_file)
        .map_err(|error| format!("Cannot read Everything results: {error}"));
    let _ = fs::remove_file(&output_file);
    let text = text?;
    let mut results = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter_map(|path| result_entry(root, path, show_hidden))
        .collect::<Vec<_>>();
    results.truncate(SEARCH_RESULT_LIMIT);
    Ok(results)
}

#[cfg(not(target_os = "windows"))]
fn everything_search(
    _root: &Path,
    _query: &str,
    _show_hidden: bool,
) -> Result<Vec<FileEntry>, String> {
    Err("Everything search is available on Windows only".to_owned())
}

fn result_entry(root: &Path, path: PathBuf, show_hidden: bool) -> Option<FileEntry> {
    let base_name = path.file_name()?.to_string_lossy().into_owned();
    if !show_hidden && base_name.starts_with('.') {
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
    let name = path
        .strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or(base_name);
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
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0),
        remote: None,
        remote_modified: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn everything_is_disabled_when_the_backend_cannot_exist() {
        assert!(!everything_available());
    }

    #[test]
    fn everything_cli_query_disables_switch_parsing_without_rewriting_search() {
        let query = "-export-txt C:\\tmp\\owned.txt | ext:rs";
        let args = everything_cli_search_args(query);
        assert_eq!(args[0], "--");
        assert_eq!(args[1], query);
    }

    #[test]
    fn default_search_is_recursive_and_bounded_to_root() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("fast-explorer-search-{stamp}"));
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/needle.txt"), b"x").unwrap();
        fs::write(root.join("other.txt"), b"x").unwrap();
        let results = default_search(&root, "needle", false).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "nested/needle.txt");
        fs::remove_dir_all(root).unwrap();
    }
}
