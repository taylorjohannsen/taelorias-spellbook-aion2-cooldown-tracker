//! Spellbook was called tjcd until 0.2.0. tjcd's updater installs Spellbook
//! beside it, as a new program and without shortcuts, so Spellbook takes over:
//! the settings, the shortcuts, starting with Windows, then uninstalling tjcd.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use crate::{autostart, registry};

const OLD_NAME: &str = "tjcd";
const OLD_UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\tjcd";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// tjcd's settings folder, next to ours, becomes ours if we don't have one yet
pub fn move_data(dir: &Path) {
    let Some(old) = dir.parent().map(|p| p.join(OLD_NAME)) else { return };
    if dir.exists() || !old.exists() {
        return;
    }
    if std::fs::rename(&old, dir).is_ok() {
        // tjcd's log is only of use to tjcd
        let _ = std::fs::remove_file(dir.join("tjcd.log"));
        return;
    }
    // something still has a file open in it, so copy instead
    let _ = std::fs::create_dir_all(dir);
    for entry in std::fs::read_dir(&old).into_iter().flatten().flatten() {
        if entry.path().is_file() {
            let _ = std::fs::copy(entry.path(), dir.join(entry.file_name()));
        }
    }
}

/// For an installed copy, once, after updating from tjcd. Says what it did.
pub fn take_over() -> Vec<String> {
    let mut done = Vec::new();
    match autostart::take_over() {
        Ok(true) => done.push("now starting with Windows instead of tjcd".to_string()),
        Ok(false) => {}
        Err(e) => done.push(format!("couldn't move starting with Windows from tjcd: {e}")),
    }
    let Some(uninstaller) = registry::get(OLD_UNINSTALL_KEY, "UninstallString") else { return done };
    let uninstaller = uninstaller.trim_matches('"').to_string();
    let Some(old_dir) = Path::new(&uninstaller).parent().map(Path::to_path_buf) else { return done };
    let Ok(exe) = std::env::current_exe() else { return done };

    // before tjcd's go, so we know whether there was one on the desktop
    match shortcuts(&exe) {
        Ok(()) => done.push("made the shortcuts".to_string()),
        Err(e) => done.push(format!("couldn't make the shortcuts: {e}")),
    }

    // `_?=` runs the uninstaller where it is and waits for it. It must come
    // last and unquoted, spaces and all.
    let status = Command::new(&uninstaller)
        .arg("/S")
        .raw_arg(format!("_?={}", old_dir.display()))
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    match status {
        Ok(s) if s.success() => done.push("uninstalled tjcd".to_string()),
        Ok(s) => done.push(format!("tjcd's uninstaller said {s}")),
        Err(e) => done.push(format!("couldn't run tjcd's uninstaller: {e}")),
    }
    // which leaves itself behind, and tjcd's browser cache
    if !old_dir.join("tjcd.exe").exists() && old_dir.ends_with(OLD_NAME) {
        let _ = std::fs::remove_dir_all(&old_dir);
    }
    done
}

/// A Start menu shortcut, and a desktop one if tjcd had one there
fn shortcuts(exe: &Path) -> Result<(), String> {
    let exe = exe.display().to_string().replace('\'', "''");
    let script = format!(
        r#"$shell = New-Object -ComObject WScript.Shell
function Make($path) {{ $s = $shell.CreateShortcut($path); $s.TargetPath = '{exe}'; $s.WorkingDirectory = Split-Path '{exe}'; $s.Save() }}
$programs = [Environment]::GetFolderPath('Programs')
$desktop = [Environment]::GetFolderPath('Desktop')
if (-not (Test-Path "$programs\Spellbook.lnk")) {{ Make "$programs\Spellbook.lnk" }}
if ((Test-Path "$desktop\tjcd.lnk") -and -not (Test-Path "$desktop\Spellbook.lnk")) {{ Make "$desktop\Spellbook.lnk" }}"#
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("PowerShell said {status}"))
    }
}
