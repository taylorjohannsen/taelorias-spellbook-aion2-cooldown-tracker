//! Starting with Windows, through the current user's Run key, which needs no
//! admin. The key itself says whether it's on.

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "tjcd";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn enabled() -> bool {
    let (key, value) = (wide(RUN_KEY), wide(VALUE));
    // SAFETY: both names are null-terminated, and no data is asked for
    let found = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
    };
    found == ERROR_SUCCESS
}

/// Points Windows at this exe, wherever it is now
pub fn set(on: bool) -> Result<(), String> {
    let (key, value) = (wide(RUN_KEY), wide(VALUE));
    let status = if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = wide(&format!("\"{}\"", exe.display()));
        // SAFETY: the data is the null-terminated command, sized in bytes
        unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), REG_SZ, command.as_ptr().cast(), (command.len() * 2) as u32)
        }
    } else {
        // SAFETY: both names are null-terminated
        let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        if status == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND {
            ERROR_SUCCESS
        } else {
            status
        }
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("the registry said {status}"))
    }
}
