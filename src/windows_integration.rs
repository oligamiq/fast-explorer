#![cfg(target_os = "windows")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DIRECTORY_SHELL: &str = r"HKCU\Software\Classes\Directory\shell";
const DRIVE_SHELL: &str = r"HKCU\Software\Classes\Drive\shell";
const FOLDER_SHELL: &str = r"HKCU\Software\Classes\Folder\shell";
const WIN_E_CLSID: &str = r"HKCU\Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";
const MARKER: &str = "enabled-v1";

fn integration_dir() -> Result<PathBuf, String> {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|p| p.join("AppData/Local"))
        })
        .ok_or_else(|| "LOCALAPPDATA and USERPROFILE are unavailable".to_owned())?;
    Ok(root.join("FastExplorer").join("explorer-integration"))
}

fn run_reg(args: &[&str]) -> Result<(), String> {
    let output = Command::new("reg")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run reg.exe: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Err(if !stderr.is_empty() { stderr } else { stdout })
}

fn run_reg_ignore(args: &[&str]) {
    let _ = run_reg(args);
}

fn backup_key(root: &Path, name: &str, key: &str) -> Result<(), String> {
    let reg_file = root.join(format!("{name}.reg"));
    let absent = root.join(format!("{name}.absent"));
    let _ = fs::remove_file(&reg_file);
    let _ = fs::remove_file(&absent);
    let file = reg_file.to_string_lossy().into_owned();
    match run_reg(&["export", key, &file, "/y"]) {
        Ok(()) => Ok(()),
        Err(_) => fs::write(absent, b"absent").map_err(|error| error.to_string()),
    }
}

fn restore_key(root: &Path, name: &str, key: &str) -> Result<(), String> {
    let reg_file = root.join(format!("{name}.reg"));
    let absent = root.join(format!("{name}.absent"));
    run_reg_ignore(&["delete", key, "/f"]);
    if reg_file.is_file() {
        let file = reg_file.to_string_lossy().into_owned();
        run_reg(&["import", &file])?;
    } else if !absent.is_file() {
        return Err(format!("missing registry backup for {name}"));
    }
    Ok(())
}

fn set_open_command(shell_key: &str, exe: &Path) -> Result<(), String> {
    let command_key = format!(r"{shell_key}\open\command");
    let command = format!("\"{}\" \"%1\"", exe.display());
    run_reg(&["add", shell_key, "/ve", "/d", "open", "/f"])?;
    run_reg(&["add", &command_key, "/ve", "/d", &command, "/f"])?;
    run_reg_ignore(&["delete", &command_key, "/v", "DelegateExecute", "/f"]);
    Ok(())
}

fn set_win_e_command(exe: &Path) -> Result<(), String> {
    let shell = format!(r"{WIN_E_CLSID}\shell");
    let verb = format!(r"{shell}\opennewwindow");
    let command_key = format!(r"{verb}\command");
    let command = format!("\"{}\"", exe.display());
    run_reg(&["add", WIN_E_CLSID, "/ve", "/d", "", "/f"])?;
    run_reg(&["add", &shell, "/f"])?;
    run_reg(&["add", &verb, "/f"])?;
    run_reg(&["add", &command_key, "/ve", "/d", &command, "/f"])?;
    run_reg(&["add", &command_key, "/v", "DelegateExecute", "/d", "", "/f"])?;
    Ok(())
}

pub fn is_registered() -> bool {
    integration_dir().is_ok_and(|root| root.join(MARKER).is_file())
}

pub fn enable() -> Result<(), String> {
    if is_registered() {
        return Ok(());
    }
    let root = integration_dir()?;
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    for (name, key) in [
        ("directory", DIRECTORY_SHELL),
        ("drive", DRIVE_SHELL),
        ("folder", FOLDER_SHELL),
        ("win-e", WIN_E_CLSID),
    ] {
        backup_key(&root, name, key)?;
    }
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let apply = (|| {
        set_open_command(DIRECTORY_SHELL, &exe)?;
        set_open_command(DRIVE_SHELL, &exe)?;
        set_open_command(FOLDER_SHELL, &exe)?;
        set_win_e_command(&exe)?;
        Ok::<(), String>(())
    })();
    if let Err(error) = apply {
        let _ = restore_all(&root);
        return Err(error);
    }
    fs::write(root.join(MARKER), exe.to_string_lossy().as_bytes())
        .map_err(|error| error.to_string())
}

fn restore_all(root: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    for (name, key) in [
        ("directory", DIRECTORY_SHELL),
        ("drive", DRIVE_SHELL),
        ("folder", FOLDER_SHELL),
        ("win-e", WIN_E_CLSID),
    ] {
        if let Err(error) = restore_key(root, name, key) {
            errors.push(format!("{name}: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub fn disable() -> Result<(), String> {
    let root = integration_dir()?;
    if !root.join(MARKER).is_file() {
        return Ok(());
    }
    restore_all(&root)?;
    fs::remove_file(root.join(MARKER)).map_err(|error| error.to_string())?;
    Ok(())
}
