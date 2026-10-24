//! SteelSeries Arctis headsets. Request bytes, response offsets and HID
//! collection choices follow HeadsetControl's device implementations.

use std::time::{Duration, Instant};

use hidapi::HidDevice;

use crate::model::{map_range, pct, Reading};
use crate::razer::hex;

pub const VID: u16 = 0x1038;

/// How long to wait for the headset's answer.
const READ_TIMEOUT: Duration = Duration::from_millis(2000);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    /// Arctis 1 / 7X / 7P (original): 06 12, battery at [3], [2]==01 offline.
    Arctis1,
    /// Arctis 7 / Pro 2019 / Pro GameDAC: 06 18, battery at [2].
    Arctis7,
    /// Arctis 9: 00 20, battery [3] in 0x64..0x9A, [4]==01 charging.
    Arctis9,
    /// Arctis Pro Wireless: 41 AA (status) then 40 AA (level 0..4).
    ProWireless,
    /// Arctis 7+: status 00 B0, [1]==01 offline, level [2] 0..4, [3]==01 charging.
    Arctis7Plus,
    /// Nova 7 family, original firmware: status 00 B0, level [2] 0..4, [3] 0=off 1/2=charging.
    Nova7Discrete,
    /// Nova 7 family, Gen 2 / Jan 2026 firmware: like above but [2] is already a percentage.
    Nova7Percent,
    /// Nova 5 / 5X / 3P / 3X: status 00 B0, [1]==02 offline, level [3], [4]==01 charging.
    Nova5,
    /// Arctis GameBuds: status 00 B0, per-bud status [3]/[4], level [5]/[6].
    GameBuds,
    /// Nova Pro Wireless base station: 06 B0 (31 bytes), level [6] 0..8, status [15].
    NovaPro,
}

#[derive(Clone, Copy, Debug)]
pub struct ArctisModel {
    pub pid: u16,
    pub name: &'static str,
    pub proto: Proto,
}

/// Which HID collection to talk to. Zero means "don't care".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub usage_page: u16,
    pub usage: u16,
    pub interface: i32,
}

impl Proto {
    pub fn target(self) -> Target {
        let t = |usage_page, usage, interface| Target { usage_page, usage, interface };
        match self {
            Proto::Arctis1 => t(0xff43, 0x0202, 3),
            Proto::Arctis7 => t(0, 0, 5),
            Proto::Arctis9 | Proto::ProWireless => t(0, 0, 0),
            Proto::NovaPro => t(0, 0, 4),
            Proto::Arctis7Plus
            | Proto::Nova7Discrete
            | Proto::Nova7Percent
            | Proto::Nova5
            | Proto::GameBuds => t(0xffc0, 0x0001, 3),
        }
    }
}

macro_rules! models {
    ($($pid:literal $name:literal $proto:ident),* $(,)?) => {
        pub const ARCTIS_MODELS: &[ArctisModel] = &[
            $(ArctisModel { pid: $pid, name: $name, proto: Proto::$proto },)*
        ];
    };
}

models! {
    0x12b3 "SteelSeries Arctis 1 Wireless" Arctis1,
    0x12b6 "SteelSeries Arctis 1 Wireless (Xbox)" Arctis1,
    0x12d7 "SteelSeries Arctis 7X" Arctis1,
    0x12d5 "SteelSeries Arctis 7P" Arctis1,

    0x1260 "SteelSeries Arctis 7" Arctis7,
    0x12ad "SteelSeries Arctis 7 (2019)" Arctis7,
    0x1252 "SteelSeries Arctis Pro (2019)" Arctis7,
    0x1280 "SteelSeries Arctis Pro GameDAC" Arctis7,

    0x12c2 "SteelSeries Arctis 9" Arctis9,
    0x1290 "SteelSeries Arctis Pro Wireless" ProWireless,

    0x220e "SteelSeries Arctis 7+" Arctis7Plus,
    0x2212 "SteelSeries Arctis 7P+" Arctis7Plus,
    0x2216 "SteelSeries Arctis 7X+" Arctis7Plus,
    0x2236 "SteelSeries Arctis 7+ Destiny" Arctis7Plus,

    0x2202 "SteelSeries Arctis Nova 7" Nova7Discrete,
    0x2206 "SteelSeries Arctis Nova 7X" Nova7Discrete,
    0x220a "SteelSeries Arctis Nova 7P" Nova7Discrete,
    0x223a "SteelSeries Arctis Nova 7 Diablo IV" Nova7Discrete,
    0x227a "SteelSeries Arctis Nova 7 WoW Edition" Nova7Discrete,
    0x22a4 "SteelSeries Arctis Nova 7X" Nova7Discrete,

    0x22a1 "SteelSeries Arctis Nova 7" Nova7Percent,
    0x227e "SteelSeries Arctis Nova 7 Gen 2" Nova7Percent,
    0x2258 "SteelSeries Arctis Nova 7X v2" Nova7Percent,
    0x229e "SteelSeries Arctis Nova 7X v2" Nova7Percent,
    0x22ad "SteelSeries Arctis Nova 7X v2" Nova7Percent,
    0x22a9 "SteelSeries Arctis Nova 7 Diablo IV" Nova7Percent,
    0x22a5 "SteelSeries Arctis Nova 7X" Nova7Percent,
    0x22a7 "SteelSeries Arctis Nova 7P v2" Nova7Percent,
    0x2298 "SteelSeries Arctis Nova 7P v2" Nova7Percent,

    0x2232 "SteelSeries Arctis Nova 5" Nova5,
    0x2253 "SteelSeries Arctis Nova 5X" Nova5,
    0x2269 "SteelSeries Arctis Nova 3P Wireless" Nova5,
    0x226d "SteelSeries Arctis Nova 3X Wireless" Nova5,

    0x230a "SteelSeries Arctis GameBuds" GameBuds,

    0x12e0 "SteelSeries Arctis Nova Pro Wireless" NovaPro,
    0x12e5 "SteelSeries Arctis Nova Pro Wireless X" NovaPro,
}

pub fn find_model(pid: u16) -> Option<&'static ArctisModel> {
    ARCTIS_MODELS.iter().find(|m| m.pid == pid)
}

/// The bytes written to ask for battery status.
pub fn request(proto: Proto) -> Vec<u8> {
    match proto {
        Proto::Arctis1 => vec![0x06, 0x12],
        Proto::Arctis7 => vec![0x06, 0x18],
        Proto::Arctis9 => vec![0x00, 0x20],
        Proto::ProWireless => padded(&[0x41, 0xAA], 31),
        Proto::NovaPro => padded(&[0x06, 0xB0], 31),
        Proto::Arctis7Plus
        | Proto::Nova7Discrete
        | Proto::Nova7Percent
        | Proto::Nova5
        | Proto::GameBuds => vec![0x00, 0xB0],
    }
}

fn padded(bytes: &[u8], len: usize) -> Vec<u8> {
    let mut v = bytes.to_vec();
    v.resize(len, 0);
    v
}

fn response_len(proto: Proto) -> usize {
    match proto {
        Proto::Arctis1 | Proto::Arctis7 => 8,
        Proto::Arctis9 => 12,
        Proto::ProWireless => 2,
        _ => 128,
    }
}

/// Turn a status response into a reading. Pure, so it is unit tested.
pub fn parse(proto: Proto, r: &[u8]) -> Result<Reading, String> {
    let need = |n: usize| {
        if r.len() < n {
            Err(format!("response too short ({} bytes)", r.len()))
        } else {
            Ok(())
        }
    };
    let battery = |percent: i32, charging: bool| Reading::Battery { percent: pct(percent), charging };
    Ok(match proto {
        Proto::Arctis1 => {
            need(4)?;
            if r[2] == 0x01 {
                Reading::Offline
            } else {
                battery(r[3] as i32, false)
            }
        }
        Proto::Arctis7 => {
            need(3)?;
            battery(r[2] as i32, false)
        }
        Proto::Arctis9 => {
            need(5)?;
            if r[4] == 0xFF {
                Reading::Offline
            } else {
                battery(map_range(r[3] as i32, 0x64, 0x9A, 0, 100), r[4] == 0x01)
            }
        }
        Proto::ProWireless => unreachable!("Pro Wireless is parsed in two steps"),
        Proto::Arctis7Plus => {
            need(4)?;
            if r[1] == 0x01 {
                Reading::Offline
            } else {
                battery(map_range(r[2] as i32, 0, 4, 0, 100), r[3] == 0x01)
            }
        }
        Proto::Nova7Discrete | Proto::Nova7Percent => {
            need(4)?;
            if r[3] == 0x00 {
                Reading::Offline
            } else {
                let level = if proto == Proto::Nova7Discrete {
                    map_range(r[2] as i32, 0, 4, 0, 100)
                } else {
                    r[2] as i32
                };
                battery(level, r[3] == 0x01 || r[3] == 0x02)
            }
        }
        Proto::Nova5 => {
            need(5)?;
            if r[1] == 0x02 {
                Reading::Offline
            } else {
                battery(r[3] as i32, r[4] == 0x01)
            }
        }
        Proto::GameBuds => {
            need(7)?;
            const ACTIVE: u8 = 0x03; // out of the case
            let mut level: Option<i32> = None;
            if r[3] == ACTIVE {
                level = Some(r[5] as i32);
            }
            if r[4] == ACTIVE {
                let right = r[6] as i32;
                level = Some(level.map_or(right, |l| l.min(right)));
            }
            match level {
                Some(l) => battery(l, false),
                None => Reading::Offline, // both buds docked
            }
        }
        Proto::NovaPro => {
            need(16)?;
            match r[15] {
                0x01 => Reading::Offline,
                status => battery(map_range(r[6] as i32, 0, 8, 0, 100), status == 0x02),
            }
        }
    })
}

/// Throw away input reports that queued up since the last poll (e.g. volume or
/// chat-mix events), so the next read is the answer to our request.
fn drain(dev: &HidDevice) {
    let mut buf = [0u8; 128];
    for _ in 0..64 {
        match dev.read_timeout(&mut buf, 0) {
            Ok(n) if n > 0 => continue,
            _ => break,
        }
    }
}

/// First byte of a valid answer, where it is known. The Nova-family status
/// interface also carries unrelated reports (chat-mix, volume, sidetone
/// replies); status reports start with 0xB0, which HeadsetControl relies on too.
pub fn expected_header(proto: Proto) -> Option<u8> {
    match proto {
        Proto::Arctis7Plus | Proto::Nova7Discrete | Proto::Nova7Percent | Proto::Nova5 | Proto::GameBuds => {
            Some(0xB0)
        }
        _ => None,
    }
}

fn exchange(
    dev: &HidDevice,
    req: &[u8],
    len: usize,
    header: Option<u8>,
    trace: &mut impl FnMut(&str),
) -> Result<Vec<u8>, String> {
    drain(dev);
    dev.write(req).map_err(|e| format!("write: {e}"))?;
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("no answer (timeout)".into());
        }
        let mut buf = vec![0u8; len];
        let n = dev
            .read_timeout(&mut buf, left.as_millis().max(1) as i32)
            .map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return Err("no answer (timeout)".into());
        }
        buf.truncate(n);
        trace(&format!("  sent {} -> got {}", hex(&req[..req.len().min(4)]), hex(&buf[..buf.len().min(20)])));
        match header {
            Some(h) if buf[0] != h => continue, // unrelated report, keep waiting
            _ => return Ok(buf),
        }
    }
}

pub fn read_battery(dev: &HidDevice, model: &ArctisModel, mut trace: impl FnMut(&str)) -> Result<Reading, String> {
    if model.proto == Proto::ProWireless {
        let status = exchange(dev, &request(Proto::ProWireless), 2, None, &mut trace)?;
        if status.first() == Some(&0x02) {
            return Ok(Reading::Offline);
        }
        let level = exchange(dev, &padded(&[0x40, 0xAA], 31), 1, None, &mut trace)?;
        let percent = map_range(level[0] as i32, 0, 4, 0, 100);
        return Ok(Reading::Battery { percent: pct(percent), charging: false });
    }
    let resp = exchange(
        dev,
        &request(model.proto),
        response_len(model.proto),
        expected_header(model.proto),
        &mut trace,
    )?;
    parse(model.proto, &resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bat(percent: u8, charging: bool) -> Reading {
        Reading::Battery { percent, charging }
    }

    #[test]
    fn nova7_both_firmware_generations() {
        let r = [0xb0, 0x00, 3, 0x03, 0, 0];
        assert_eq!(parse(Proto::Nova7Discrete, &r).unwrap(), bat(75, false));
        let r = [0xb0, 0x00, 64, 0x01, 0, 0];
        assert_eq!(parse(Proto::Nova7Percent, &r).unwrap(), bat(64, true));
        let r = [0xb0, 0x00, 64, 0x00, 0, 0];
        assert_eq!(parse(Proto::Nova7Percent, &r).unwrap(), Reading::Offline);
    }

    #[test]
    fn nova5_and_7plus() {
        assert_eq!(parse(Proto::Nova5, &[0, 0x03, 0, 88, 0x01]).unwrap(), bat(88, true));
        assert_eq!(parse(Proto::Nova5, &[0, 0x02, 0, 88, 0x00]).unwrap(), Reading::Offline);
        assert_eq!(parse(Proto::Arctis7Plus, &[0, 0x03, 2, 0]).unwrap(), bat(50, false));
        assert_eq!(parse(Proto::Arctis7Plus, &[0, 0x01, 2, 0]).unwrap(), Reading::Offline);
    }

    #[test]
    fn legacy_models() {
        assert_eq!(parse(Proto::Arctis1, &[0x06, 0x12, 0x03, 77, 0, 0, 0, 0]).unwrap(), bat(77, false));
        assert_eq!(parse(Proto::Arctis1, &[0x06, 0x12, 0x01, 77, 0, 0, 0, 0]).unwrap(), Reading::Offline);
        assert_eq!(parse(Proto::Arctis7, &[0x06, 0x18, 42, 0, 0, 0, 0, 0]).unwrap(), bat(42, false));
        assert_eq!(parse(Proto::Arctis9, &[0, 0, 0, 0x9A, 0x01]).unwrap(), bat(100, true));
    }

    #[test]
    fn nova_pro_and_gamebuds() {
        let mut r = [0u8; 16];
        r[6] = 6;
        r[15] = 0x08;
        assert_eq!(parse(Proto::NovaPro, &r).unwrap(), bat(75, false));
        r[15] = 0x02;
        assert_eq!(parse(Proto::NovaPro, &r).unwrap(), bat(75, true));
        r[15] = 0x01;
        assert_eq!(parse(Proto::NovaPro, &r).unwrap(), Reading::Offline);

        assert_eq!(parse(Proto::GameBuds, &[0, 0, 0, 0x03, 0x03, 80, 60]).unwrap(), bat(60, false));
        assert_eq!(parse(Proto::GameBuds, &[0, 0, 0, 0x03, 0x02, 80, 0]).unwrap(), bat(80, false));
        assert_eq!(parse(Proto::GameBuds, &[0, 0, 0, 0x02, 0x02, 0, 0]).unwrap(), Reading::Offline);
    }

    #[test]
    fn short_response_is_an_error() {
        assert!(parse(Proto::Nova5, &[0, 1]).is_err());
    }

    #[test]
    fn model_table_has_unique_pids() {
        let mut pids: Vec<u16> = ARCTIS_MODELS.iter().map(|m| m.pid).collect();
        pids.sort();
        pids.dedup();
        assert_eq!(pids.len(), ARCTIS_MODELS.len());
    }
}
