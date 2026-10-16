//! Starting with Windows, through the current user's Run key, which needs no
//! admin. The key itself says whether it's on.

use crate::registry;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Spellbook";
/// From before Spellbook was renamed
const OLD_VALUE: &str = "tjcd";

pub fn enabled() -> bool {
    registry::get(RUN_KEY, VALUE).is_some()
}

/// Points Windows at this exe, wherever it is now
pub fn set(on: bool) -> Result<(), String> {
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        registry::set(RUN_KEY, VALUE, &format!("\"{}\"", exe.display()))
    } else {
        registry::delete(RUN_KEY, VALUE)
    }
}

/// tjcd started with Windows, so Spellbook does now instead
pub fn take_over() -> Result<bool, String> {
    if registry::get(RUN_KEY, OLD_VALUE).is_none() {
        return Ok(false);
    }
    registry::delete(RUN_KEY, OLD_VALUE)?;
    set(true)?;
    Ok(true)
}
