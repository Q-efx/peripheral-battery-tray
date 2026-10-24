//! Finds supported devices and reads their batteries.

use std::collections::HashMap;
use std::ffi::CString;

use hidapi::{DeviceInfo, HidApi};

use crate::model::{DeviceStatus, Kind, Reading};
use crate::{razer, steelseries};

pub struct Scanner {
    api: HidApi,
    /// HID collection that produced a real battery value, per (vid, pid).
    good_path: HashMap<(u16, u16), CString>,
    /// Last live value per device, kept while it sleeps.
    last_known: HashMap<(u16, u16), (u8, bool)>,
    /// Print enumeration details and raw traffic to stderr.
    pub debug: bool,
}

/// A HID collection as enumerated (owned copy so we can drop the borrow on `api`).
#[derive(Clone, Debug)]
struct Collection {
    path: CString,
    interface: i32,
    usage_page: u16,
    usage: u16,
}

impl From<&DeviceInfo> for Collection {
    fn from(d: &DeviceInfo) -> Self {
        Collection {
            path: d.path().to_owned(),
            interface: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
        }
    }
}

/// HeadsetControl's `get_hid_path`: the first collection with the wanted usage
/// page/usage, otherwise the first with the wanted interface (0 = any).
fn headsetcontrol_choice(cols: &[Collection], t: steelseries::Target) -> Option<&Collection> {
    if t.usage_page != 0
        && t.usage != 0
        && let Some(c) = cols.iter().find(|c| c.usage_page == t.usage_page && c.usage == t.usage)
    {
        return Some(c);
    }
    cols.iter().find(|c| t.interface == 0 || c.interface == t.interface)
}

impl Scanner {
    pub fn new() -> Result<Self, String> {
        let api = HidApi::new().map_err(|e| format!("cannot initialise HID: {e}"))?;
        Ok(Scanner { api, good_path: HashMap::new(), last_known: HashMap::new(), debug: false })
    }

    fn log(&self, msg: &str) {
        if self.debug {
            eprintln!("{msg}");
        }
    }

    /// Enumerate and query every supported device once.
    pub fn scan(&mut self) -> Vec<DeviceStatus> {
        if let Err(e) = self.api.refresh_devices() {
            self.log(&format!("HID refresh failed: {e}"));
        }

        // Group collections by (vid, pid), keeping enumeration order.
        let mut groups: Vec<((u16, u16), Vec<Collection>)> = Vec::new();
        for d in self.api.device_list() {
            let vid = d.vendor_id();
            if vid != razer::VID && vid != steelseries::VID {
                continue;
            }
            let key = (vid, d.product_id());
            let c = Collection::from(d);
            if self.debug {
                eprintln!(
                    "found {:04x}:{:04x} iface={} usage={:04x}:{:04x} {}",
                    key.0,
                    key.1,
                    c.interface,
                    c.usage_page,
                    c.usage,
                    c.path.to_string_lossy()
                );
            }
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(c),
                None => groups.push((key, vec![c])),
            }
        }

        let mut out = Vec::new();
        for ((vid, pid), cols) in groups {
            let status = if vid == razer::VID {
                let Some(model) = razer::find_model(pid) else { continue };
                let reading = self.query_razer(model, &cols);
                DeviceStatus { kind: Kind::Mouse, name: model.name.into(), vid, pid, reading, last_known: None }
            } else {
                let Some(model) = steelseries::find_model(pid) else { continue };
                let reading = self.query_arctis(model, &cols);
                DeviceStatus { kind: Kind::Headset, name: model.name.into(), vid, pid, reading, last_known: None }
            };
            out.push(status);
        }

        for s in &mut out {
            if let Reading::Battery { percent, charging } = s.reading {
                self.last_known.insert(s.key(), (percent, charging));
            }
            s.last_known = self.last_known.get(&s.key()).copied();
        }
        dedupe_twins(out)
    }

    /// Remember a path only once it produced a real value.
    fn remember(&mut self, key: (u16, u16), path: &CString, reading: &Reading) {
        if matches!(reading, Reading::Battery { .. }) {
            self.good_path.insert(key, path.clone());
        }
    }

    fn query_razer(&mut self, model: &razer::RazerModel, cols: &[Collection]) -> Reading {
        let key = (razer::VID, model.pid);
        let good = self.good_path.get(&key).cloned();
        // The collection that worked before, then the documented interface,
        // then the rest (Bluetooth devices report interface -1).
        let mut candidates: Vec<&Collection> = cols.iter().collect();
        candidates.sort_by_key(|c| {
            if Some(&c.path) == good.as_ref() {
                0
            } else if c.interface == model.interface {
                1
            } else {
                2
            }
        });
        let debug = self.debug;
        let mut last_err = String::from("no usable HID collection");
        for c in candidates {
            let known_good = Some(&c.path) == good.as_ref();
            self.log(&format!(
                "razer {:04x}: trying iface={} usage={:04x}:{:04x}",
                model.pid, c.interface, c.usage_page, c.usage
            ));
            let dev = match self.api.open_path(&c.path) {
                Ok(d) => d,
                Err(e) => {
                    last_err = format!("open: {e}");
                    continue;
                }
            };
            let result = razer::read_battery(&dev, model, known_good, |m| {
                if debug {
                    eprintln!("{m}")
                }
            });
            match result {
                // Battery or a genuine "asleep" answer (status/class/command all matched).
                Ok(reading) => {
                    self.remember(key, &c.path, &reading);
                    return reading;
                }
                Err(e) => last_err = format!("{e:?}"),
            }
        }
        self.good_path.remove(&key);
        Reading::Error(last_err)
    }

    fn query_arctis(&mut self, model: &steelseries::ArctisModel, cols: &[Collection]) -> Reading {
        let key = (steelseries::VID, model.pid);
        let good = self.good_path.get(&key).cloned();
        // Write only where HeadsetControl would (plus a path that answered before),
        // so commands never go to unrelated collections.
        let chosen = good
            .as_ref()
            .and_then(|g| cols.iter().find(|c| &c.path == g))
            .or_else(|| headsetcontrol_choice(cols, model.proto.target()));
        let Some(c) = chosen else {
            return Reading::Error("expected HID interface not present".into());
        };
        self.log(&format!(
            "arctis {:04x}: using iface={} usage={:04x}:{:04x}",
            model.pid, c.interface, c.usage_page, c.usage
        ));
        let dev = match self.api.open_path(&c.path) {
            Ok(d) => d,
            Err(e) => return Reading::Error(format!("open: {e}")),
        };
        let debug = self.debug;
        let path = c.path.clone();
        match steelseries::read_battery(&dev, model, |m| {
            if debug {
                eprintln!("{m}")
            }
        }) {
            Ok(reading) => {
                self.remember(key, &path, &reading);
                reading
            }
            Err(e) => {
                self.good_path.remove(&key);
                Reading::Error(e)
            }
        }
    }
}

/// Razer mice show up twice when the cable is plugged in while the receiver is
/// also connected ("X (Wired)" and "X (Wireless)"). Keep one entry per mouse.
fn base_name(name: &str) -> &str {
    for suffix in [" (Wired)", " (Wireless)", " (Receiver)", " Receiver"] {
        if let Some(b) = name.strip_suffix(suffix) {
            return b;
        }
    }
    name
}

pub fn dedupe_twins(list: Vec<DeviceStatus>) -> Vec<DeviceStatus> {
    // Preference: live value first; among live ones the wired copy (it knows it's charging);
    // otherwise the receiver copy, which is the one that wakes up first.
    let score = |s: &DeviceStatus| match s.reading {
        Reading::Battery { .. } if s.name.ends_with("(Wired)") => 4,
        Reading::Battery { .. } => 3,
        Reading::Offline => 2,
        Reading::Error(_) => 1,
    };
    let mut out: Vec<DeviceStatus> = Vec::new();
    for s in list {
        match out.iter_mut().find(|o| o.kind == s.kind && base_name(&o.name) == base_name(&s.name)) {
            Some(existing) => {
                if score(&s) > score(existing) {
                    // Keep the last known value from whichever twin had one.
                    let last = s.last_known.or(existing.last_known);
                    *existing = DeviceStatus { last_known: last, ..s };
                } else if existing.last_known.is_none() {
                    existing.last_known = s.last_known;
                }
            }
            None => out.push(s),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(name: &str, pid: u16, reading: Reading) -> DeviceStatus {
        DeviceStatus { kind: Kind::Mouse, name: name.into(), vid: 0x1532, pid, reading, last_known: None }
    }

    fn names(list: &[DeviceStatus]) -> Vec<&str> {
        list.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn wired_twin_hides_sleeping_receiver() {
        let list = vec![
            st("Razer DeathAdder V3 Pro (Wired)", 0xb6, Reading::Battery { percent: 80, charging: true }),
            st("Razer DeathAdder V3 Pro (Wireless)", 0xb7, Reading::Offline),
            st("Razer Viper V3 Pro (Wireless)", 0xc1, Reading::Offline),
        ];
        let out = dedupe_twins(list);
        assert_eq!(names(&out), ["Razer DeathAdder V3 Pro (Wired)", "Razer Viper V3 Pro (Wireless)"]);
    }

    #[test]
    fn both_twins_live_or_both_asleep_show_once() {
        let live = vec![
            st("Razer DeathAdder V3 Pro (Wireless)", 0xb7, Reading::Battery { percent: 80, charging: false }),
            st("Razer DeathAdder V3 Pro (Wired)", 0xb6, Reading::Battery { percent: 80, charging: true }),
        ];
        assert_eq!(names(&dedupe_twins(live)), ["Razer DeathAdder V3 Pro (Wired)"]);
        let mut asleep_wired = st("Razer DeathAdder V3 Pro (Wired)", 0xb6, Reading::Error("x".into()));
        asleep_wired.last_known = Some((70, false));
        let asleep = vec![asleep_wired, st("Razer DeathAdder V3 Pro (Wireless)", 0xb7, Reading::Offline)];
        let out = dedupe_twins(asleep);
        assert_eq!(names(&out), ["Razer DeathAdder V3 Pro (Wireless)"]);
        assert_eq!(out[0].last_known, Some((70, false)));
    }

    #[test]
    fn collection_choice_matches_headsetcontrol() {
        let col = |i: i32, up: u16, u: u16, p: &str| Collection {
            path: CString::new(p).unwrap(),
            interface: i,
            usage_page: up,
            usage: u,
        };
        let cols = vec![col(0, 0x000c, 1, "a"), col(3, 0xff00, 1, "b"), col(3, 0xffc0, 1, "c"), col(5, 0xff00, 1, "d")];
        let pick = |p: steelseries::Proto| headsetcontrol_choice(&cols, p.target()).map(|c| c.path.to_str().unwrap());
        assert_eq!(pick(steelseries::Proto::Nova7Percent), Some("c")); // usage match
        assert_eq!(pick(steelseries::Proto::Arctis7), Some("d")); // interface 5
        assert_eq!(pick(steelseries::Proto::Arctis9), Some("a")); // first collection
        assert_eq!(pick(steelseries::Proto::NovaPro), None); // no interface 4
    }
}
