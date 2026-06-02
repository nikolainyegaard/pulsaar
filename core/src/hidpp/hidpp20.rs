// HID++ 2.0 protocol implementation.
// Reference: reference/lib/logitech_receiver/hidpp20.py, hidpp20_constants.py

use std::collections::HashMap;

use crate::error::{Error, Hidpp10Error, Result};
use crate::hidpp::message::{Message, SOFTWARE_ID};
use crate::transport::Transport;
use crate::devices::types::{Battery, BatteryStatus, FirmwareInfo, FirmwareKind};

// Key HID++ 2.0 feature IDs.
pub const FEAT_ROOT: u16            = 0x0000;
pub const FEAT_FEATURE_SET: u16     = 0x0001;
pub const FEAT_FW_VERSION: u16      = 0x0003;
pub const FEAT_DEVICE_NAME: u16     = 0x0005;
pub const FEAT_BATTERY_STATUS: u16  = 0x1000;
pub const FEAT_BATTERY_VOLTAGE: u16 = 0x1001;
pub const FEAT_UNIFIED_BATTERY: u16 = 0x1004;

/// Build the HID++ 2.0 address byte: (function << 4) | SOFTWARE_ID.
fn fn_addr(function: u8) -> u8 {
    (function << 4) | SOFTWARE_ID
}

/// Query ROOT (feature index 0) for the device index of a given feature_id.
/// Returns None if the feature is not present (device returns index 0).
pub fn get_feature_index(transport: &Transport, device: u8, feature_id: u16) -> Result<Option<u8>> {
    // ROOT.GetFeature (function 0): params = [feature_id_hi, feature_id_lo, 0]
    let req = Message::short(
        device,
        0x00,         // ROOT is always at index 0
        fn_addr(0),   // function 0
        (feature_id >> 8) as u8,
        (feature_id & 0xFF) as u8,
        0,
    );
    let reply = transport.request(&req)?;
    let idx = reply.params().first().copied().unwrap_or(0);
    Ok(if idx == 0 { None } else { Some(idx) })
}

/// Discover all features on a HID++ 2.0 device by scanning the feature table.
/// Returns a map of feature_id -> feature_index. Returns an empty map if the
/// device does not support HID++ 2.0 (no FEATURE_SET).
pub fn discover_features(transport: &Transport, device: u8) -> Result<HashMap<u16, u8>> {
    let mut map = HashMap::new();

    let fs_idx = match get_feature_index(transport, device, FEAT_FEATURE_SET)? {
        Some(i) => i,
        None => return Ok(map), // HID++ 1.0 device
    };

    map.insert(FEAT_ROOT, 0);
    map.insert(FEAT_FEATURE_SET, fs_idx);

    // FEATURE_SET.GetCount (function 0): returns feature count not including ROOT.
    let req = Message::short(device, fs_idx, fn_addr(0), 0, 0, 0);
    let reply = transport.request(&req)?;
    let count = reply.params().first().copied().unwrap_or(0) as usize + 1; // +1 for ROOT

    // FEATURE_SET.GetFeature (function 1): for each index, returns [feat_id_hi, feat_id_lo, flags, version].
    for i in 0..count {
        if map.values().any(|&v| v == i as u8) {
            continue; // already know ROOT and FEATURE_SET
        }
        let req = Message::short(device, fs_idx, fn_addr(1), i as u8, 0, 0);
        let reply = match transport.request(&req) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let p = reply.params();
        if p.len() >= 2 {
            let feat_id = ((p[0] as u16) << 8) | (p[1] as u16);
            if feat_id != 0 {
                map.insert(feat_id, i as u8);
            }
        }
    }

    Ok(map)
}

/// Get the protocol version from a device using ROOT.GetProtocolVersion.
/// Returns (major, minor). Returns None if the device is HID++ 1.0.
pub fn get_protocol_version(transport: &Transport, device: u8) -> Result<Option<(u8, u8)>> {
    // ROOT.GetProtocolVersion (function 1): no params.
    let req = Message::short(device, 0x00, fn_addr(1), 0, 0, 0);
    match transport.request(&req) {
        Ok(reply) => {
            let p = reply.params();
            Ok(Some((p.first().copied().unwrap_or(1), p.get(1).copied().unwrap_or(0))))
        }
        Err(Error::Hidpp10(Hidpp10Error::InvalidSubId)) => Ok(None), // HID++ 1.0 device
        Err(e) => Err(e),
    }
}

// -- Feature calls ------------------------------------------------------------

/// Call a feature function and return the reply.
pub fn feature_call(
    transport: &Transport,
    device: u8,
    feature_index: u8,
    function: u8,
    params: &[u8],
) -> Result<Message> {
    let req = if params.len() <= 3 {
        let p = [
            params.first().copied().unwrap_or(0),
            params.get(1).copied().unwrap_or(0),
            params.get(2).copied().unwrap_or(0),
        ];
        Message::short(device, feature_index, fn_addr(function), p[0], p[1], p[2])
    } else {
        Message::long(device, feature_index, fn_addr(function), params)
    };
    transport.request(&req)
}

// -- High-level accessors -----------------------------------------------------

/// Get the device name. Requires FEAT_DEVICE_NAME to be present in features.
/// The HID++ 2.0 device name feature (0x0005) returns the name across possibly
/// multiple chunks; we read only the first chunk here (sufficient for most devices).
pub fn get_device_name(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<String>> {
    let idx = match features.get(&FEAT_DEVICE_NAME) {
        Some(&i) => i,
        None => return Ok(None),
    };

    // Function 0: GetCount -> returns [name_byte_count].
    let count_reply = feature_call(transport, device, idx, 0, &[])?;
    let total_chars = count_reply.params().first().copied().unwrap_or(0) as usize;
    if total_chars == 0 {
        return Ok(None);
    }

    // Function 1: GetDeviceName(char_index) -> returns up to 14 chars starting at char_index.
    let mut name_bytes = Vec::with_capacity(total_chars);
    let mut offset = 0usize;
    while name_bytes.len() < total_chars {
        let reply = feature_call(transport, device, idx, 1, &[offset as u8])?;
        let p = reply.params();
        let remaining = total_chars - name_bytes.len();
        let chunk = &p[..remaining.min(p.len())];
        name_bytes.extend_from_slice(chunk);
        offset += chunk.len();
        if chunk.is_empty() {
            break;
        }
    }

    Ok(Some(String::from_utf8_lossy(&name_bytes[..total_chars.min(name_bytes.len())]).into_owned()))
}

/// Get battery status using FEAT_BATTERY_STATUS (0x1000).
/// Function 0: GetBatteryLevelStatus -> [level_0-100, next_level, charging_status, ...].
pub fn get_battery_status(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<Battery>> {
    let idx = match features.get(&FEAT_BATTERY_STATUS) {
        Some(&i) => i,
        None => return Ok(None),
    };

    let reply = feature_call(transport, device, idx, 0, &[])?;
    let p = reply.params();
    let level = p.first().copied();
    let status = p.get(2).map(|&b| match b {
        0 => BatteryStatus::Discharging,
        1 => BatteryStatus::Recharging,
        2 => BatteryStatus::AlmostFull,
        3 => BatteryStatus::Full,
        4 => BatteryStatus::SlowRecharge,
        5 => BatteryStatus::InvalidBattery,
        6 => BatteryStatus::ThermalError,
        _ => BatteryStatus::Discharging,
    });

    Ok(Some(Battery { level, status, voltage: None }))
}

/// Get battery voltage using FEAT_BATTERY_VOLTAGE (0x1001).
/// Function 0: GetBatteryVoltage -> [voltage_hi, voltage_lo, ...] in mV.
pub fn get_battery_voltage(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<Battery>> {
    let idx = match features.get(&FEAT_BATTERY_VOLTAGE) {
        Some(&i) => i,
        None => return Ok(None),
    };

    let reply = feature_call(transport, device, idx, 0, &[])?;
    let p = reply.params();
    let voltage = if p.len() >= 2 {
        Some(((p[0] as u16) << 8) | (p[1] as u16))
    } else {
        None
    };

    Ok(Some(Battery { level: None, status: None, voltage }))
}

/// Get firmware version using FEAT_FW_VERSION (0x0003).
/// Function 0: GetFwInfo -> [entity_count].
/// Function 1: GetFwInfo(entity_idx) -> [entity_type, prefix_1, prefix_2, num_hi, num_lo, rev, build_hi, build_lo, ...].
pub fn get_firmware(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Vec<FirmwareInfo>> {
    let idx = match features.get(&FEAT_FW_VERSION) {
        Some(&i) => i,
        None => return Ok(vec![]),
    };

    // Function 0: GetEntityCount
    let count_reply = feature_call(transport, device, idx, 0, &[])?;
    let count = count_reply.params().first().copied().unwrap_or(0) as usize;

    let mut result = Vec::new();
    for i in 0..count {
        let reply = match feature_call(transport, device, idx, 1, &[i as u8]) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let p = reply.params();
        if p.len() < 5 {
            continue;
        }
        let kind = match p[0] {
            0 => FirmwareKind::Firmware,
            1 => FirmwareKind::Bootloader,
            _ => FirmwareKind::Other,
        };
        // Version format: XX.YY.BBBB (major, minor, build)
        let version = format!("{:02X}.{:02X}.B{:02X}{:02X}", p[3], p[4], p[6], p[7]);
        result.push(FirmwareInfo { kind, version });
    }

    Ok(result)
}
