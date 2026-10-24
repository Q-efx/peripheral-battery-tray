//! Console read-out, useful for checking support and for debugging.
//!
//!   battery-cli            one line per device
//!   battery-cli --debug    also list HID collections and raw responses
//!   battery-cli --json     machine-readable output
//!   battery-cli --models   list every supported model

use battery_tray::model::Reading;
use battery_tray::scan::Scanner;
use battery_tray::steelseries;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);

    if has("--help") || has("-h") {
        println!("battery-cli [--debug] [--json] [--models]");
        return;
    }
    if has("--models") {
        println!("Razer mice ({}):", razer_models().len());
        for (pid, name) in razer_models() {
            println!("  1532:{pid:04x}  {name}");
        }
        println!("SteelSeries headsets ({}):", steelseries::ARCTIS_MODELS.len());
        for m in steelseries::ARCTIS_MODELS {
            println!("  1038:{:04x}  {}", m.pid, m.name);
        }
        return;
    }

    let mut scanner = match Scanner::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    scanner.debug = has("--debug");
    let list = scanner.scan();

    if has("--json") {
        let items: Vec<String> = list
            .iter()
            .map(|s| {
                let (level, charging, state) = match &s.reading {
                    Reading::Battery { percent, charging } => (percent.to_string(), charging.to_string(), "ok".to_string()),
                    Reading::Offline => ("null".into(), "null".into(), "offline".into()),
                    Reading::Error(e) => ("null".into(), "null".into(), format!("error: {e}")),
                };
                format!(
                    "{{\"name\":{:?},\"kind\":\"{}\",\"usb_id\":\"{:04x}:{:04x}\",\"level\":{level},\"charging\":{charging},\"state\":{:?}}}",
                    s.name,
                    s.kind.label(),
                    s.vid,
                    s.pid,
                    state
                )
            })
            .collect();
        println!("[{}]", items.join(","));
        return;
    }

    if list.is_empty() {
        println!("No supported Razer mouse or SteelSeries Arctis headset found.");
        println!("Run with --debug to see which Razer/SteelSeries USB devices are present.");
        return;
    }
    let width = list.iter().map(|s| s.name.len()).max().unwrap_or(0);
    for s in &list {
        println!("{:<width$}  {:<7}  {}", s.name, s.kind.label(), s.describe());
    }
}

fn razer_models() -> Vec<(u16, &'static str)> {
    battery_tray::razer_devices::RAZER_MODELS.iter().map(|m| (m.pid, m.name)).collect()
}
