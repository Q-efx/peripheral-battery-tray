//! Low-battery beeps: which level warrants one, when it repeats, and the tone.

use std::f32::consts::PI;
use std::time::{Duration, Instant};

use crate::model::{DeviceStatus, Reading};

/// How urgent the lowest battery is, from mild to urgent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Alert {
    /// Under 20 %: a very quiet beep every five minutes.
    Low,
    /// Under 10 %: a bit louder.
    VeryLow,
    /// Under 5 %: as loud as `VeryLow`, but every minute.
    Critical,
}

impl Alert {
    fn for_level(percent: u8) -> Option<Alert> {
        match percent {
            0..5 => Some(Alert::Critical),
            5..10 => Some(Alert::VeryLow),
            10..20 => Some(Alert::Low),
            _ => None,
        }
    }

    /// The most urgent alert across live, discharging devices. Asleep devices
    /// only have a stale value and charging ones are already being looked after.
    pub fn for_devices(list: &[DeviceStatus]) -> Option<Alert> {
        list.iter()
            .filter_map(|s| match s.reading {
                Reading::Battery { percent, charging: false } => Alert::for_level(percent),
                _ => None,
            })
            .max()
    }

    pub fn interval(self) -> Duration {
        match self {
            Alert::Low | Alert::VeryLow => Duration::from_secs(5 * 60),
            Alert::Critical => Duration::from_secs(60),
        }
    }

    /// Peak amplitude of the beep, as a fraction of full scale.
    pub fn volume(self) -> f32 {
        match self {
            Alert::Low => 0.06,
            Alert::VeryLow | Alert::Critical => 0.12,
        }
    }
}

/// Decides when the next beep is due.
#[derive(Default)]
pub struct Beeper {
    /// The alert last beeped for, and when.
    last: Option<(Alert, Instant)>,
}

impl Beeper {
    /// The alert to beep for now, if any. A more urgent alert than the last one
    /// beeps at once; otherwise beeps are at least the alert's interval apart,
    /// so a level wobbling around a threshold doesn't add extra beeps.
    pub fn due(&mut self, alert: Option<Alert>, now: Instant) -> Option<Alert> {
        let alert = alert?;
        let due = match self.last {
            Some((prev, at)) => alert > prev || now.duration_since(at) >= alert.interval(),
            None => true,
        };
        if due {
            self.last = Some((alert, now));
        }
        due.then_some(alert)
    }
}

const SAMPLE_RATE: u32 = 22_050;

/// A short 880 Hz beep as a 16-bit mono WAV file; `volume` is the peak amplitude (0..=1).
pub fn tone(volume: f32) -> Vec<u8> {
    const FREQ: f32 = 880.0;
    let samples = SAMPLE_RATE * 150 / 1000;
    let fade = SAMPLE_RATE * 15 / 1000;
    let data_len = samples * 2;

    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2u16.to_le_bytes()); // bytes per sample frame
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());

    let volume = volume.clamp(0.0, 1.0);
    for i in 0..samples {
        // Fade in and out so the tone starts and stops without a click.
        let envelope = (i.min(samples - 1 - i) as f32 / fade as f32).min(1.0);
        let t = i as f32 / SAMPLE_RATE as f32;
        let s = (2.0 * PI * FREQ * t).sin() * volume * envelope;
        wav.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;

    fn dev(reading: Reading) -> DeviceStatus {
        DeviceStatus { kind: Kind::Mouse, name: "m".into(), vid: 0, pid: 0, reading, last_known: Some((3, false)) }
    }

    fn battery(percent: u8, charging: bool) -> DeviceStatus {
        dev(Reading::Battery { percent, charging })
    }

    #[test]
    fn thresholds() {
        let at = |p| Alert::for_devices(&[battery(p, false)]);
        assert_eq!(at(20), None);
        assert_eq!(at(19), Some(Alert::Low));
        assert_eq!(at(10), Some(Alert::Low));
        assert_eq!(at(9), Some(Alert::VeryLow));
        assert_eq!(at(5), Some(Alert::VeryLow));
        assert_eq!(at(4), Some(Alert::Critical));
        assert_eq!(at(0), Some(Alert::Critical));
    }

    #[test]
    fn picks_most_urgent_live_discharging_device() {
        let list = [battery(80, false), battery(15, false), battery(7, false), battery(2, true), dev(Reading::Offline)];
        assert_eq!(Alert::for_devices(&list), Some(Alert::VeryLow));
        assert_eq!(Alert::for_devices(&[battery(2, true), dev(Reading::Error("timeout".into()))]), None);
        assert_eq!(Alert::for_devices(&[]), None);
    }

    #[test]
    fn beeps_at_once_then_every_interval() {
        let t0 = Instant::now();
        let min = |m: u64| t0 + Duration::from_secs(m * 60);
        let mut b = Beeper::default();
        assert_eq!(b.due(Some(Alert::Low), t0), Some(Alert::Low));
        assert_eq!(b.due(Some(Alert::Low), min(4)), None);
        assert_eq!(b.due(Some(Alert::Low), min(5)), Some(Alert::Low));
        // Dropping under 10 % beeps louder straight away, then every five minutes.
        assert_eq!(b.due(Some(Alert::VeryLow), min(6)), Some(Alert::VeryLow));
        assert_eq!(b.due(Some(Alert::VeryLow), min(10)), None);
        assert_eq!(b.due(Some(Alert::VeryLow), min(11)), Some(Alert::VeryLow));
        // Under 5 %: every minute.
        assert_eq!(b.due(Some(Alert::Critical), min(11) + Duration::from_secs(10)), Some(Alert::Critical));
        assert_eq!(b.due(Some(Alert::Critical), min(12)), None);
        assert_eq!(b.due(Some(Alert::Critical), min(12) + Duration::from_secs(10)), Some(Alert::Critical));
        assert_eq!(b.due(None, min(30)), None);
    }

    #[test]
    fn wobbling_level_adds_no_beeps() {
        let t0 = Instant::now();
        let min = |m: u64| t0 + Duration::from_secs(m * 60);
        let mut b = Beeper::default();
        assert_eq!(b.due(Some(Alert::VeryLow), t0), Some(Alert::VeryLow));
        assert_eq!(b.due(Some(Alert::Low), min(1)), None);
        assert_eq!(b.due(Some(Alert::VeryLow), min(2)), None);
        assert_eq!(b.due(None, min(3)), None);
        assert_eq!(b.due(Some(Alert::VeryLow), min(4)), None);
        assert_eq!(b.due(Some(Alert::VeryLow), min(5)), Some(Alert::VeryLow));
    }

    #[test]
    fn tone_is_a_valid_quiet_wav() {
        let wav = tone(Alert::Low.volume());
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize, wav.len() - 8);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize, wav.len() - 44);
        let peak = wav[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        let limit = (Alert::Low.volume() * i16::MAX as f32) as u16;
        assert!(peak > limit / 2 && peak <= limit, "peak {peak}, limit {limit}");
    }
}
