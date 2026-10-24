//! Shared types for every supported device family.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Mouse,
    Headset,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Mouse => "mouse",
            Kind::Headset => "headset",
        }
    }
}

/// What a single query returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reading {
    /// A live battery value.
    Battery { percent: u8, charging: bool },
    /// The receiver/dongle answered, but the device itself is off, asleep or docked.
    Offline,
    /// Talking to the device failed.
    Error(String),
}

/// One device as shown to the user.
#[derive(Clone, Debug)]
pub struct DeviceStatus {
    pub kind: Kind,
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub reading: Reading,
    /// The last live value seen for this device (kept while it is asleep or off).
    pub last_known: Option<(u8, bool)>,
}

impl DeviceStatus {
    /// Stable key used to remember the last value across scans.
    pub fn key(&self) -> (u16, u16) {
        (self.vid, self.pid)
    }

    /// The level to display: live if available, otherwise the last known one.
    pub fn display_level(&self) -> Option<(u8, bool, bool)> {
        match &self.reading {
            Reading::Battery { percent, charging } => Some((*percent, *charging, true)),
            _ => self.last_known.map(|(p, c)| (p, c, false)),
        }
    }

    /// Name without the vendor prefix, for the space-limited tooltip.
    pub fn short_name(&self) -> &str {
        self.name
            .strip_prefix("Razer ")
            .or_else(|| self.name.strip_prefix("SteelSeries "))
            .unwrap_or(&self.name)
    }

    pub fn describe(&self) -> String {
        match (&self.reading, self.last_known) {
            (Reading::Battery { percent, charging: true }, _) => format!("{percent}% (charging)"),
            (Reading::Battery { percent, .. }, _) => format!("{percent}%"),
            (Reading::Offline, Some((p, _))) => format!("asleep/off (last {p}%)"),
            (Reading::Offline, None) => "asleep/off".to_string(),
            (Reading::Error(e), Some((p, _))) => format!("no answer (last {p}%): {e}"),
            (Reading::Error(e), None) => format!("no answer: {e}"),
        }
    }
}

/// Integer range mapping with clamping, same semantics HeadsetControl uses.
pub fn map_range(x: i32, in_min: i32, in_max: i32, out_min: i32, out_max: i32) -> i32 {
    let (lo, hi) = if in_min <= in_max { (in_min, in_max) } else { (in_max, in_min) };
    let x = x.clamp(lo, hi);
    if in_max == in_min {
        return out_min;
    }
    out_min + (x - in_min) * (out_max - out_min) / (in_max - in_min)
}

/// Clamp any value into 0..=100.
pub fn pct(v: i32) -> u8 {
    v.clamp(0, 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_range_matches_expectations() {
        assert_eq!(map_range(0, 0, 4, 0, 100), 0);
        assert_eq!(map_range(2, 0, 4, 0, 100), 50);
        assert_eq!(map_range(4, 0, 4, 0, 100), 100);
        assert_eq!(map_range(9, 0, 4, 0, 100), 100); // clamped
        assert_eq!(map_range(0x64, 0x64, 0x9A, 0, 100), 0);
        assert_eq!(map_range(0x9A, 0x64, 0x9A, 0, 100), 100);
        assert_eq!(map_range(4, 0, 8, 0, 100), 50);
    }
}
