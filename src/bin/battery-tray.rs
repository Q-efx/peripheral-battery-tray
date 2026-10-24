// No console window for the tray app.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::time::Duration;

fn interval_from_args() -> Duration {
    let args: Vec<String> = std::env::args().collect();
    let secs = args
        .iter()
        .position(|a| a == "--interval")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60)
        .max(5);
    Duration::from_secs(secs)
}

#[cfg(windows)]
fn main() {
    battery_tray::tray::run(interval_from_args());
}

#[cfg(not(windows))]
fn main() {
    let _ = interval_from_args();
    eprintln!("The tray app is Windows-only; use battery-cli on this system.");
    std::process::exit(1);
}
