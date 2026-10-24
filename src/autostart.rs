//! "Start with Windows" via HKCU\Software\Microsoft\Windows\CurrentVersion\Run.

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "BatteryTray";

fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

pub fn is_enabled() -> bool {
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ) else {
        return false;
    };
    match (key.get_value::<String, _>(VALUE), command()) {
        (Ok(v), Some(cmd)) => v.eq_ignore_ascii_case(&cmd),
        _ => false,
    }
}

pub fn set_enabled(enable: bool) -> std::io::Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE)?;
    if enable {
        let cmd = command().ok_or_else(|| std::io::Error::other("cannot find own path"))?;
        key.set_value(VALUE, &cmd)
    } else {
        match key.delete_value(VALUE) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}
