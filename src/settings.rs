//! Preferences kept in HKCU\Software\BatteryTray.

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const KEY: &str = r"Software\BatteryTray";
const BEEPS: &str = "LowBatteryBeeps";

/// Low-battery beeps are on unless they were switched off in the menu.
pub fn beeps_enabled() -> bool {
    let stored = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).and_then(|k| k.get_value::<u32, _>(BEEPS));
    !matches!(stored, Ok(0))
}

pub fn set_beeps_enabled(enable: bool) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY)?;
    key.set_value(BEEPS, &u32::from(enable))
}
