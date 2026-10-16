//! Strings in the current user's registry, which needs no admin

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn get(key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut size = 0u32;
    // SAFETY: both names are null-terminated; the first call only asks for the size
    let found = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
    };
    if found != ERROR_SUCCESS {
        return None;
    }
    let mut data = vec![0u16; size as usize / 2 + 1];
    let mut size = (data.len() * 2) as u32;
    // SAFETY: the buffer is as big as `size` says
    let read = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), data.as_mut_ptr().cast(), &mut size)
    };
    if read != ERROR_SUCCESS {
        return None;
    }
    let len = data.iter().position(|&c| c == 0).unwrap_or(data.len());
    Some(String::from_utf16_lossy(&data[..len]))
}

pub fn set(key: &str, value: &str, data: &str) -> Result<(), String> {
    let (key, value, data) = (wide(key), wide(value), wide(data));
    // SAFETY: the data is null-terminated, and sized in bytes
    let status = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
    };
    check(status)
}

/// Fine if it's already gone
pub fn delete(key: &str, value: &str) -> Result<(), String> {
    let (key, value) = (wide(key), wide(value));
    // SAFETY: both names are null-terminated
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
    check(if status == ERROR_FILE_NOT_FOUND { ERROR_SUCCESS } else { status })
}

fn check(status: u32) -> Result<(), String> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("the registry said {status}"))
    }
}
