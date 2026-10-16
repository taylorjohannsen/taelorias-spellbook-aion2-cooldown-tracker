//! Whether AION 2, or one of our own windows, is the window in front, so the
//! overlay can keep out of the way of a browser. Only the window's process id
//! and the process list are read, which works with the game running elevated.

use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::overlay::Overlay;

const GAME_EXE: &str = "aion2.exe";
const POLL: Duration = Duration::from_millis(200);

pub fn watch(overlay: Overlay) {
    std::thread::spawn(move || {
        let own = std::process::id();
        let mut last_pid = None;
        let mut focused = false;
        loop {
            let pid = foreground_pid();
            // the process list is only read when the window in front changes
            if pid != last_pid {
                last_pid = pid;
                let now = pid.is_some_and(|pid| pid == own || exe_name(pid).is_some_and(|exe| exe.eq_ignore_ascii_case(GAME_EXE)));
                if now != focused {
                    focused = now;
                    overlay.update(|v| v.focused = now);
                }
            }
            std::thread::sleep(POLL);
        }
    });
}

fn foreground_pid() -> Option<u32> {
    let mut pid = 0;
    // SAFETY: plain calls; a null window just gives no process
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return None;
        }
        GetWindowThreadProcessId(window, &mut pid);
    }
    (pid != 0).then_some(pid)
}

fn exe_name(pid: u32) -> Option<String> {
    // SAFETY: the snapshot is closed before returning, and the entry is sized as
    // the API asks
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut name = None;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            if entry.th32ProcessID == pid {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                name = Some(String::from_utf16_lossy(&entry.szExeFile[..len]));
                break;
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        name
    }
}
