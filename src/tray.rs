//! Windows system-tray front end.
//!
//! A worker thread scans devices on an interval; the main thread owns the tray
//! icon and pumps Win32 messages (tray-icon needs a message loop on its thread).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT,
    WM_QUIT,
};

use crate::alert::{self, Alert, Beeper};
use crate::icon::{self, IconState};
use crate::model::{DeviceStatus, Reading};
use crate::scan::Scanner;
use crate::{autostart, settings};

type ScanResult = Result<Vec<DeviceStatus>, String>;

/// A scan result, flagged when it was produced in answer to "Refresh now".
struct Update {
    result: ScanResult,
    requested: bool,
}

fn worker(interval: Duration, results: Sender<Update>, refresh: Receiver<()>) {
    let mut scanner: Option<Scanner> = None;
    let mut requested = false;
    loop {
        // Keep retrying HID initialisation instead of giving up for good.
        if scanner.is_none() {
            match Scanner::new() {
                Ok(s) => scanner = Some(s),
                Err(e) => {
                    if results.send(Update { result: Err(e), requested }).is_err() {
                        return;
                    }
                }
            }
        }
        if let Some(s) = scanner.as_mut()
            && results.send(Update { result: Ok(s.scan()), requested }).is_err()
        {
            return;
        }
        requested = false;
        match refresh.recv_timeout(interval) {
            Ok(()) => {
                while refresh.try_recv().is_ok() {}
                requested = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Returns false if another copy is already running.
fn single_instance() -> bool {
    let name: Vec<u16> = "Local\\BatteryTraySingleInstance\0".encode_utf16().collect();
    // The handle is intentionally leaked: it must live as long as the process.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    !(handle.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS)
}

/// Plays a low-battery beep without blocking the message loop.
fn beep(alert: Alert) {
    let wav = alert::tone(alert.volume());
    std::thread::spawn(move || {
        // SND_MEMORY: the pointer is a WAV image, not a file name. Synchronous, so `wav` outlives playback.
        unsafe { PlaySoundW(wav.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_NODEFAULT | SND_SYNC) };
    });
}

fn make_icon(state: &IconState) -> Icon {
    Icon::from_rgba(icon::render(state), icon::SIZE as u32, icon::SIZE as u32).expect("valid icon size")
}

/// The icon shows the lowest live level; if every device sleeps, the lowest last-known one in grey.
pub fn icon_state(list: &[DeviceStatus]) -> IconState {
    let pick = |live: bool| {
        list.iter()
            .filter_map(|s| s.display_level())
            .filter(|(_, _, l)| *l == live)
            .min_by_key(|(p, _, _)| *p)
    };
    match pick(true).or_else(|| pick(false)) {
        Some((p, c, live)) => IconState { level: Some(p), charging: c && live, live, unanswered: false },
        // Devices present but none has ever answered -> "?", nothing present -> "-".
        None => IconState { level: None, charging: false, live: true, unanswered: !list.is_empty() },
    }
}

fn tooltip(list: &[DeviceStatus]) -> String {
    if list.is_empty() {
        return "Battery Tray: no supported devices".into();
    }
    let lines: Vec<String> = list
        .iter()
        .map(|s| {
            let v = match (&s.reading, s.last_known) {
                (Reading::Battery { percent, charging: true }, _) => format!("{percent}% charging"),
                (Reading::Battery { percent, .. }, _) => format!("{percent}%"),
                (Reading::Offline, Some((p, _))) => format!("asleep ({p}%)"),
                (Reading::Error(_), Some((p, _))) => format!("no answer ({p}%)"),
                (Reading::Offline, None) => "asleep/off".into(),
                (Reading::Error(_), None) => "no answer".into(),
            };
            format!("{}: {v}", s.short_name())
        })
        .collect();
    // Windows tooltips hold 127 characters.
    lines.join("\n").chars().take(127).collect()
}

struct Ui {
    tray: TrayIcon,
    menu: Menu,
    device_items: Vec<MenuItem>,
    refresh: MenuItem,
    shown: Option<IconState>,
    /// "Refresh now" was clicked and its scan hasn't come back yet.
    refresh_pending: bool,
    /// Low-battery alert from the latest scan.
    alert: Option<Alert>,
}

impl Ui {
    fn request_refresh(&mut self, tx: &Sender<()>) {
        self.refresh.set_text("Refreshing…");
        self.refresh.set_enabled(false);
        self.refresh_pending = true;
        let _ = tx.send(());
    }

    fn show(&mut self, update: Update) {
        let result = update.result;
        for item in self.device_items.drain(..) {
            let _ = self.menu.remove(&item);
        }
        let (texts, tip, state, alert) = match result {
            Ok(list) => {
                let texts = if list.is_empty() {
                    vec!["No supported devices found".to_string()]
                } else {
                    list.iter().map(|s| format!("{}  —  {}", s.name, s.describe())).collect()
                };
                (texts, tooltip(&list), icon_state(&list), Alert::for_devices(&list))
            }
            Err(e) => (
                vec![format!("Error: {e}")],
                format!("Battery Tray: {e}"),
                IconState { level: None, charging: false, live: true, unanswered: true },
                None,
            ),
        };
        self.alert = alert;
        for (i, t) in texts.iter().enumerate() {
            let item = MenuItem::new(t, false, None);
            let _ = self.menu.insert(&item, i);
            self.device_items.push(item);
        }
        let _ = self.tray.set_tooltip(Some(tip));
        if self.shown != Some(state) {
            let _ = self.tray.set_icon(Some(make_icon(&state)));
            self.shown = Some(state);
        }
        // Only the scan that answers the click re-enables the button.
        if !self.refresh_pending || update.requested {
            self.refresh_pending = false;
            self.refresh.set_text("Refresh now");
            self.refresh.set_enabled(true);
        }
    }
}

pub fn run(interval: Duration) {
    if !single_instance() {
        return;
    }

    let (res_tx, res_rx) = mpsc::channel::<Update>();
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || worker(interval, res_tx, refresh_rx));

    let menu = Menu::new();
    let scanning = MenuItem::new("Scanning…", false, None);
    let refresh = MenuItem::new("Refresh now", true, None);
    let startup = CheckMenuItem::new("Start with Windows", true, autostart::is_enabled(), None);
    let mut beeps_on = settings::beeps_enabled();
    let beeps = CheckMenuItem::new("Beep on low battery", true, beeps_on, None);
    let quit = MenuItem::new("Quit", true, None);
    let _ = menu.append_items(&[
        &scanning,
        &PredefinedMenuItem::separator(),
        &refresh,
        &startup,
        &beeps,
        &PredefinedMenuItem::separator(),
        &quit,
    ]);

    let initial = IconState { level: None, charging: false, live: true, unanswered: false };
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_menu_on_left_click(false)
        .with_tooltip("Battery Tray: scanning…")
        .with_icon(make_icon(&initial))
        .build()
        .expect("create tray icon");

    let mut ui = Ui {
        tray,
        menu,
        device_items: vec![scanning],
        refresh,
        shown: Some(initial),
        refresh_pending: false,
        alert: None,
    };
    let mut beeper = Beeper::default();

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    'outer: loop {
        // Sleep until a window message arrives or 250 ms pass (to pick up scan results).
        unsafe { MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 250, QS_ALLINPUT) };
        while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_QUIT {
                break 'outer;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if ev.id == *quit.id() {
                break 'outer;
            } else if ev.id == *ui.refresh.id() {
                ui.request_refresh(&refresh_tx);
            } else if ev.id == *startup.id() {
                // muda already toggled the check mark; make the registry follow it.
                let want = startup.is_checked();
                if autostart::set_enabled(want).is_err() {
                    startup.set_checked(!want);
                }
            } else if ev.id == *beeps.id() {
                beeps_on = beeps.is_checked();
                // Still applies for this session if it can't be saved.
                let _ = settings::set_beeps_enabled(beeps_on);
            }
        }

        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                ui.request_refresh(&refresh_tx);
            }
        }

        while let Ok(update) = res_rx.try_recv() {
            ui.show(update);
        }

        if beeps_on && let Some(a) = beeper.due(ui.alert, Instant::now()) {
            beep(a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;

    fn dev(name: &str, reading: Reading, last: Option<(u8, bool)>) -> DeviceStatus {
        DeviceStatus { kind: Kind::Mouse, name: name.into(), vid: 0, pid: 0, reading, last_known: last }
    }

    #[test]
    fn icon_prefers_lowest_live_value() {
        let list = vec![
            dev("a", Reading::Battery { percent: 80, charging: false }, Some((80, false))),
            dev("b", Reading::Battery { percent: 30, charging: true }, Some((30, true))),
            dev("c", Reading::Offline, Some((5, false))),
        ];
        assert_eq!(icon_state(&list), IconState { level: Some(30), charging: true, live: true, unanswered: false });
        let asleep = vec![dev("c", Reading::Offline, Some((5, false)))];
        assert_eq!(icon_state(&asleep), IconState { level: Some(5), charging: false, live: false, unanswered: false });
        let silent = vec![dev("d", Reading::Error("timeout".into()), None)];
        assert_eq!(icon_state(&silent), IconState { level: None, charging: false, live: true, unanswered: true });
        assert_eq!(icon_state(&[]), IconState { level: None, charging: false, live: true, unanswered: false });
    }
}
