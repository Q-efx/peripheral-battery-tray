//! Razer mice: the 90-byte HID feature-report protocol documented by OpenRazer.
//!
//! Battery level is command class 0x07, command 0x80; charging state is 0x07/0x84.
//! The answer lives in `arguments[1]` (0..=255 for the level, 0/1 for charging).

use std::thread::sleep;
use std::time::Duration;

use hidapi::HidDevice;

use crate::model::Reading;

pub const VID: u16 = 0x1532;
pub const REPORT_LEN: usize = 90;

/// Per-model facts, generated from OpenRazer (see `razer_devices.rs`).
#[derive(Clone, Copy, Debug)]
pub struct RazerModel {
    pub pid: u16,
    pub name: &'static str,
    /// Transaction id the device expects (0x1F for most current wireless mice).
    pub transaction_id: u8,
    /// How long to wait between sending a request and reading the answer.
    pub wait_us: u32,
    /// USB interface number the feature report is sent to.
    pub interface: i32,
    /// False for AA-battery mice that cannot report charging.
    pub has_charging: bool,
}

pub fn find_model(pid: u16) -> Option<&'static RazerModel> {
    crate::razer_devices::RAZER_MODELS.iter().find(|m| m.pid == pid)
}

// Response status codes (byte 0 of a response).
pub const STATUS_NEW: u8 = 0x00;
pub const STATUS_BUSY: u8 = 0x01;
pub const STATUS_OK: u8 = 0x02;
pub const STATUS_FAIL: u8 = 0x03;
pub const STATUS_TIMEOUT: u8 = 0x04;
pub const STATUS_NOT_SUPPORTED: u8 = 0x05;

/// XOR of bytes 2..88, stored at byte 88.
pub fn crc(report: &[u8; REPORT_LEN]) -> u8 {
    report[2..88].iter().fold(0u8, |acc, b| acc ^ b)
}

pub fn build_request(transaction_id: u8, class: u8, id: u8, data_size: u8) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[1] = transaction_id;
    // r[2..4] remaining packets = 0, r[4] protocol type = 0
    r[5] = data_size;
    r[6] = class;
    r[7] = id;
    r[88] = crc(&r);
    r
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResponseError {
    /// The mouse is asleep/off: the receiver answers "timeout" or nothing yet.
    Asleep,
    NotSupported,
    Failed(u8),
    Mismatch,
}

/// Check a response against its request and return the 80 argument bytes.
pub fn parse_response(
    request: &[u8; REPORT_LEN],
    response: &[u8; REPORT_LEN],
) -> Result<[u8; 80], ResponseError> {
    match response[0] {
        STATUS_OK | STATUS_BUSY => {}
        // The receiver answered but the mouse didn't (asleep / off).
        STATUS_TIMEOUT => return Err(ResponseError::Asleep),
        // STATUS_NEW (no answer filled in) and failures are retried, like OpenRazer.
        STATUS_NOT_SUPPORTED => return Err(ResponseError::NotSupported),
        other => return Err(ResponseError::Failed(other)),
    }
    if response[2..4] != request[2..4] || response[6] != request[6] || response[7] != request[7] {
        return Err(ResponseError::Mismatch);
    }
    let mut args = [0u8; 80];
    args.copy_from_slice(&response[8..88]);
    Ok(args)
}

/// 0..=255 -> 0..=100, rounded (0xAB -> 67%).
pub fn raw_to_percent(raw: u8) -> u8 {
    ((raw as u32 * 100 + 127) / 255) as u8
}

#[derive(Debug)]
pub enum TransactError {
    Hid(String),
    Response(ResponseError),
}

/// Send one request as a feature report (report id 0) and read the answer.
fn transact_once(
    dev: &HidDevice,
    request: &[u8; REPORT_LEN],
    wait: Duration,
) -> Result<[u8; REPORT_LEN], TransactError> {
    let mut out = [0u8; REPORT_LEN + 1];
    out[1..].copy_from_slice(request);
    dev.send_feature_report(&out)
        .map_err(|e| TransactError::Hid(format!("send: {e}")))?;
    sleep(wait);
    let mut inp = [0u8; REPORT_LEN + 1];
    let n = dev
        .get_feature_report(&mut inp)
        .map_err(|e| TransactError::Hid(format!("receive: {e}")))?;
    if n < REPORT_LEN + 1 {
        return Err(TransactError::Hid(format!("short report ({n} bytes)")));
    }
    let mut resp = [0u8; REPORT_LEN];
    resp.copy_from_slice(&inp[1..]);
    Ok(resp)
}

/// Like OpenRazer: up to 5 tries, 10 ms apart. Returns the argument bytes.
///
/// `retry_hid_errors` is set for a collection that answered before (a transient
/// failure, e.g. while Synapse talks to the mouse). For an untried collection a
/// HID error means it can't carry the report, so we fail fast and move on.
pub fn transact(
    dev: &HidDevice,
    request: &[u8; REPORT_LEN],
    wait: Duration,
    retry_hid_errors: bool,
    mut trace: impl FnMut(&str),
) -> Result<[u8; 80], TransactError> {
    let mut last = TransactError::Response(ResponseError::Asleep);
    for attempt in 1..=5 {
        match transact_once(dev, request, wait) {
            Ok(resp) => {
                trace(&format!("  try {attempt}: response {}", hex(&resp[..12])));
                match parse_response(request, &resp) {
                    Ok(args) => return Ok(args),
                    // "Not supported" will not change on retry.
                    Err(ResponseError::NotSupported) => {
                        return Err(TransactError::Response(ResponseError::NotSupported));
                    }
                    Err(e) => last = TransactError::Response(e),
                }
            }
            Err(e @ TransactError::Hid(_)) => {
                trace(&format!("  try {attempt}: {e:?}"));
                if !retry_hid_errors {
                    return Err(e);
                }
                last = e;
            }
            Err(e) => last = e,
        }
        sleep(Duration::from_millis(10));
    }
    Err(last)
}

/// Read battery level (and charging state when the model supports it).
pub fn read_battery(
    dev: &HidDevice,
    model: &RazerModel,
    known_good: bool,
    mut trace: impl FnMut(&str),
) -> Result<Reading, TransactError> {
    let wait = Duration::from_micros(model.wait_us as u64);
    let req = build_request(model.transaction_id, 0x07, 0x80, 0x02);
    let args = match transact(dev, &req, wait, known_good, &mut trace) {
        Ok(args) => args,
        Err(TransactError::Response(ResponseError::Asleep)) => return Ok(Reading::Offline),
        Err(e) => return Err(e),
    };
    let raw = args[1];
    trace(&format!("  battery raw = {raw:#04x}"));
    // A sleeping mouse behind a receiver answers with zeros.
    if raw == 0 {
        return Ok(Reading::Offline);
    }
    let charging = if model.has_charging {
        let req = build_request(model.transaction_id, 0x07, 0x84, 0x02);
        match transact(dev, &req, wait, true, &mut trace) {
            Ok(args) => args[1] == 1,
            Err(_) => false,
        }
    } else {
        false
    };
    Ok(Reading::Battery { percent: raw_to_percent(raw), charging })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_request_layout() {
        let r = build_request(0x1F, 0x07, 0x80, 0x02);
        assert_eq!(&r[..8], &[0x00, 0x1F, 0x00, 0x00, 0x00, 0x02, 0x07, 0x80]);
        // crc = 0x02 ^ 0x07 ^ 0x80
        assert_eq!(r[88], 0x85);
        assert_eq!(r[89], 0);
    }

    #[test]
    fn parses_known_deathadder_v4_answer() {
        // "02 1f 00 00 00 02 07 80 00 ab" -> 67 %
        let req = build_request(0x1F, 0x07, 0x80, 0x02);
        let mut resp = [0u8; REPORT_LEN];
        resp[..10].copy_from_slice(&[0x02, 0x1f, 0, 0, 0, 0x02, 0x07, 0x80, 0x00, 0xab]);
        let args = parse_response(&req, &resp).unwrap();
        assert_eq!(raw_to_percent(args[1]), 67);
    }

    #[test]
    fn sleeping_and_mismatched_responses() {
        let req = build_request(0x1F, 0x07, 0x80, 0x02);
        let mut resp = [0u8; REPORT_LEN];
        resp[0] = STATUS_TIMEOUT;
        assert_eq!(parse_response(&req, &resp), Err(ResponseError::Asleep));
        // "New" means no answer was filled in: a failure to retry, not "asleep".
        resp[0] = STATUS_NEW;
        assert_eq!(parse_response(&req, &resp), Err(ResponseError::Failed(STATUS_NEW)));
        resp[0] = STATUS_OK;
        resp[6] = 0x07;
        resp[7] = 0x84;
        assert_eq!(parse_response(&req, &resp), Err(ResponseError::Mismatch));
    }

    #[test]
    fn percent_scale() {
        assert_eq!(raw_to_percent(0), 0);
        assert_eq!(raw_to_percent(255), 100);
        assert_eq!(raw_to_percent(128), 50);
    }

    #[test]
    fn device_table_is_sane() {
        let models = crate::razer_devices::RAZER_MODELS;
        assert!(models.len() > 50);
        let dav3 = find_model(0x00B7).unwrap();
        assert_eq!(dav3.transaction_id, 0x1F);
        let mut pids: Vec<u16> = models.iter().map(|m| m.pid).collect();
        pids.sort();
        pids.dedup();
        assert_eq!(pids.len(), models.len());
    }
}
