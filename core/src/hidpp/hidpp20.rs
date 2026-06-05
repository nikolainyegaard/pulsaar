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
pub const FEAT_ADJUSTABLE_DPI: u16      = 0x2201;
pub const FEAT_HIRES_WHEEL: u16         = 0x2121;
pub const FEAT_SMART_SHIFT_ENHANCED: u16 = 0x2111;
pub const FEAT_CHANGE_HOST: u16         = 0x1814;
pub const FEAT_HOSTS_INFO: u16          = 0x1815;
pub const FEAT_FN_INVERSION: u16        = 0x40A0;
pub const FEAT_NEW_FN_INVERSION: u16    = 0x40A2;
pub const FEAT_K375S_FN_INVERSION: u16  = 0x40A3;
pub const FEAT_MULTIPLATFORM: u16       = 0x4531;
pub const FEAT_BACKLIGHT2: u16          = 0x1982;

/// DPI capabilities and current state for a device with FEAT_ADJUSTABLE_DPI.
pub struct DpiInfo {
    /// Sorted list of discrete DPI steps the sensor supports.
    pub dpi_list:    Vec<u16>,
    /// Currently active DPI. 0 if the device did not report one.
    pub current_dpi: u16,
    /// Default DPI reported by the device. 0 if not reported.
    pub default_dpi: u16,
}

/// Scroll wheel capabilities and current state for a device with FEAT_HIRES_WHEEL.
pub struct ScrollInfo {
    pub has_invert:    bool,
    pub has_hires:     bool,
    pub inverted:      bool,
    pub hires_enabled: bool,
}

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

/// Get battery status using FEAT_UNIFIED_BATTERY (0x1004).
/// Function 1: GetStatus -> [soc_0-100, level, charging_status, ...].
/// level: 0=empty, 1=critical, 2=low, 4=good, 8=full (bitmask approximation).
/// Prefers the SOC percentage; falls back to the level approximation when SOC is 0.
pub fn get_unified_battery(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<Battery>> {
    let idx = match features.get(&FEAT_UNIFIED_BATTERY) {
        Some(&i) => i,
        None => return Ok(None),
    };

    let reply = feature_call(transport, device, idx, 1, &[])?;
    let p = reply.params();
    if p.is_empty() {
        return Ok(None);
    }

    let level = if p[0] > 0 {
        Some(p[0]) // SOC percentage
    } else {
        // No SOC reported; derive approximate level from the level nibble.
        Some(match p.get(1).copied().unwrap_or(0) {
            8 => 90u8, // full
            4 => 50u8, // good
            2 => 20u8, // low
            1 => 5u8,  // critical
            _ => 0u8,  // empty
        })
    };

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

// -- Settings -----------------------------------------------------------------

/// Read DPI capabilities and current DPI using FEAT_ADJUSTABLE_DPI (0x2201).
///
/// GetSensorDpiList (fn 1): params = [sensor_idx=0, page_offset].
/// Each response page holds up to 8 u16 DPI values (big-endian, 0x0000 terminates).
/// A value with bits[15:13] == 0b111 is a range-step marker: the following value is
/// the range end; all steps from the previous DPI to the end are expanded.
///
/// GetSensorDpi (fn 2): returns [sensor_echo, _, current_hi, current_lo, default_hi, default_lo].
pub fn get_dpi_info(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<DpiInfo>> {
    let idx = match features.get(&FEAT_ADJUSTABLE_DPI) {
        Some(&i) => i,
        None => return Ok(None),
    };

    // Read the DPI list, paginating through pages until a 0x0000 terminator.
    // Each response starts with a sensor-index echo byte; skip it (ignore=1 per Solaar).
    // GetSensorDpiList params: [sensor_idx=0, direction=0, page_idx].
    let mut dpi_list: Vec<u16> = Vec::new();
    let mut page: u8 = 0;
    'pages: loop {
        let reply = feature_call(transport, device, idx, 1, &[0x00, 0x00, page])?;
        let p = reply.params();
        // Skip byte 0 (sensor echo); DPI values are 2-byte big-endian words starting at byte 1.
        let mut i = 1;
        while i + 1 < p.len() {
            let val = ((p[i] as u16) << 8) | (p[i + 1] as u16);
            i += 2;
            if val == 0 {
                break 'pages;
            }
            if val >> 13 == 0b111 {
                // Range step marker: the next value is the end of the range.
                let step = val & 0x1FFF;
                if i + 1 < p.len() {
                    let end = ((p[i] as u16) << 8) | (p[i + 1] as u16);
                    i += 2;
                    if end == 0 { break 'pages; }
                    if let Some(&last) = dpi_list.last() {
                        let mut cur = last.saturating_add(step);
                        while cur <= end {
                            dpi_list.push(cur);
                            cur = cur.saturating_add(step);
                        }
                    }
                }
            } else {
                dpi_list.push(val);
            }
        }
        // No terminator in this page; advance to the next.
        page = match page.checked_add(8) { Some(n) => n, None => break };
        if dpi_list.len() >= 200 { break; }
    }

    // GetSensorDpi (fn 2): [sensor_echo, current_hi, current_lo, default_hi, default_lo, ...].
    // Byte 0 is the sensor echo; current DPI is at bytes 1-2, default at bytes 3-4.
    // If current is 0 the device reports only the default (use default as current).
    let (current_dpi, default_dpi) = match feature_call(transport, device, idx, 2, &[0x00]) {
        Ok(reply) => {
            let p = reply.params();
            let current = if p.len() >= 3 { ((p[1] as u16) << 8) | (p[2] as u16) } else { 0 };
            let default = if p.len() >= 5 { ((p[3] as u16) << 8) | (p[4] as u16) } else { 0 };
            let resolved = if current == 0 { default } else { current };
            (resolved, default)
        }
        Err(_) => (0, 0),
    };

    Ok(Some(DpiInfo { dpi_list, current_dpi, default_dpi }))
}

/// Set the active DPI for sensor 0 using FEAT_ADJUSTABLE_DPI (0x2201).
///
/// SetSensorDpi (fn 3): params = [sensor_idx=0, dpi_hi, dpi_lo].
/// Silently succeeds if the feature is not present on this device.
pub fn set_dpi(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
    dpi: u16,
) -> Result<()> {
    let idx = match features.get(&FEAT_ADJUSTABLE_DPI) {
        Some(&i) => i,
        None => return Ok(()),
    };
    feature_call(transport, device, idx, 3, &[0x00, (dpi >> 8) as u8, (dpi & 0xFF) as u8])?;
    Ok(())
}

/// Read scroll wheel capabilities and current mode using FEAT_HIRES_WHEEL (0x2121).
///
/// GetCapabilities (fn 0): response byte 1 = capability flags (0x08=has_invert, 0x02=has_hires).
/// GetMode (fn 1): response byte 0 = mode flags (0x04=inverted, 0x02=hires_enabled).
pub fn get_scroll_info(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<ScrollInfo>> {
    let idx = match features.get(&FEAT_HIRES_WHEEL) {
        Some(&i) => i,
        None => return Ok(None),
    };

    // GetCapabilities (fn 0): response[0]=multiplier, response[1]=flags.
    // Flags: bit 3 (0x08) = supports inversion, bit 2 (0x04) = has ratchet.
    // Hi-res mode (bit 1 in mode byte) is always settable when the feature is present.
    let caps_reply = feature_call(transport, device, idx, 0, &[])?;
    let cap_byte   = caps_reply.params().get(1).copied().unwrap_or(0);
    let has_invert = (cap_byte & 0x08) != 0;
    let has_hires  = true; // hi-res mode is always available when HIRES_WHEEL is present

    let mode_reply    = feature_call(transport, device, idx, 1, &[])?;
    let mode_byte     = mode_reply.params().first().copied().unwrap_or(0);
    let inverted      = (mode_byte & 0x04) != 0;
    let hires_enabled = (mode_byte & 0x02) != 0;

    Ok(Some(ScrollInfo { has_invert, has_hires, inverted, hires_enabled }))
}

/// Set scroll wheel mode using FEAT_HIRES_WHEEL (0x2121).
///
/// SetMode (fn 2): params = [mode_byte] where bit 2 = invert, bit 1 = hi-res.
/// Silently succeeds if the feature is not present on this device.
pub fn set_scroll_settings(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
    inverted: bool,
    hires_enabled: bool,
) -> Result<()> {
    let idx = match features.get(&FEAT_HIRES_WHEEL) {
        Some(&i) => i,
        None => return Ok(()),
    };
    let mode_byte = (if inverted { 0x04u8 } else { 0 }) | (if hires_enabled { 0x02 } else { 0 });
    feature_call(transport, device, idx, 2, &[mode_byte])?;
    Ok(())
}

// -- SmartShift Enhanced (0x2111) ---------------------------------------------

/// Smart shift / scroll wheel ratchet state for a device with FEAT_SMART_SHIFT_ENHANCED.
///
/// wheel_mode:  1=always freespin, 2=smart-shift (auto switches based on speed).
/// has_torque:  true when the device supports tunable ratchet torque.
/// torque:      ratchet engagement torque, 1-100 (only valid when has_torque is true).
pub struct SmartShiftInfo {
    pub wheel_mode: u8,
    pub has_torque: bool,
    pub torque:     u8,
}

/// Read smart-shift ratchet mode and torque using FEAT_SMART_SHIFT_ENHANCED (0x2111).
///
/// GetCapabilities (fn 0): reply[0] bit 0 = supports tunable torque.
/// GetRatchetControlMode (fn 1): [wheel_mode, auto_disengage, torque, ...].
pub fn get_smart_shift(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<SmartShiftInfo>> {
    let idx = match features.get(&FEAT_SMART_SHIFT_ENHANCED) {
        Some(&i) => i,
        None => return Ok(None),
    };

    let caps = feature_call(transport, device, idx, 0, &[])?;
    let has_torque = (caps.params().first().copied().unwrap_or(0) & 0x01) != 0;

    let reply = feature_call(transport, device, idx, 1, &[])?;
    let p = reply.params();
    let wheel_mode = p.first().copied().unwrap_or(2);
    let torque = p.get(2).copied().unwrap_or(50);

    Ok(Some(SmartShiftInfo { wheel_mode, has_torque, torque }))
}

/// Set smart-shift wheel mode and torque using FEAT_SMART_SHIFT_ENHANCED (0x2111).
///
/// SetRatchetControlMode (fn 2): [wheel_mode, auto_disengage, torque].
/// auto_disengage is read from the device first so it is preserved.
pub fn set_smart_shift(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
    wheel_mode: u8,
    torque: u8,
) -> Result<()> {
    let idx = match features.get(&FEAT_SMART_SHIFT_ENHANCED) {
        Some(&i) => i,
        None => return Ok(()),
    };
    let current = feature_call(transport, device, idx, 1, &[])?;
    let auto_disengage = current.params().get(1).copied().unwrap_or(0);
    feature_call(transport, device, idx, 2, &[wheel_mode, auto_disengage, torque])?;
    Ok(())
}

// -- Change Host (0x1814 + 0x1815) --------------------------------------------

/// Info about one host slot as reported by FEAT_HOSTS_INFO (0x1815).
pub struct HostInfo {
    pub slot:      u8,
    pub name:      String,
    pub is_active: bool,
}

/// Read the list of host slots using FEAT_CHANGE_HOST (0x1814) for count and
/// FEAT_HOSTS_INFO (0x1815) for names. Falls back to generic names if 0x1815 is absent.
///
/// GetCount (fn 0) on CHANGE_HOST: [numHosts, currentHost, ...].
/// GetHostsInfo (fn 0) on HOSTS_INFO: [cap_flags, _, numHosts, currentHost, ...].
///   cap_flags bit 0 = can read host names.
/// GetHostInfo (fn 1) on HOSTS_INFO: params=[slot]; reply[4] = nameLen.
/// GetHostName (fn 3) on HOSTS_INFO: params=[slot, offset]; reply[2..] = name bytes.
pub fn get_hosts(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<Vec<HostInfo>>> {
    let ch_idx = match features.get(&FEAT_CHANGE_HOST) {
        Some(&i) => i,
        None => return Ok(None),
    };

    let reply = feature_call(transport, device, ch_idx, 0, &[])?;
    let p = reply.params();
    let num_hosts = p.first().copied().unwrap_or(0) as usize;
    let current_host = p.get(1).copied().unwrap_or(0);

    if num_hosts == 0 { return Ok(None); }

    let hi_idx = features.get(&FEAT_HOSTS_INFO).copied();

    let mut hosts = Vec::with_capacity(num_hosts);
    for slot in 0..num_hosts as u8 {
        let is_active = slot == current_host;

        let name = if let Some(hi) = hi_idx {
            read_host_name(transport, device, hi, slot)
                .unwrap_or_else(|_| format!("Host {}", slot + 1))
        } else {
            format!("Host {}", slot + 1)
        };

        hosts.push(HostInfo { slot, name, is_active });
    }

    Ok(Some(hosts))
}

fn read_host_name(transport: &Transport, device: u8, hi_idx: u8, slot: u8) -> Result<String> {
    // GetHostsInfo (fn 0): [cap_flags, _, numHosts, currentHost, ...]
    let caps = feature_call(transport, device, hi_idx, 0, &[])?;
    if (caps.params().first().copied().unwrap_or(0) & 0x01) == 0 {
        return Ok(format!("Host {}", slot + 1));
    }

    // GetHostInfo (fn 1): params=[slot] -> [_, status, _, _, nameLen, _, ...]
    let info = feature_call(transport, device, hi_idx, 1, &[slot])?;
    let name_len = info.params().get(4).copied().unwrap_or(0) as usize;
    if name_len == 0 {
        return Ok(format!("Host {}", slot + 1));
    }

    // GetHostName (fn 3): params=[slot, 0] -> [_, _, name_bytes...]
    let name_reply = feature_call(transport, device, hi_idx, 3, &[slot, 0])?;
    let np = name_reply.params();
    let end = 2 + name_len.min(np.len().saturating_sub(2));
    let name_bytes = if end > 2 { &np[2..end] } else { &np[2..] };
    Ok(String::from_utf8_lossy(name_bytes).trim_end_matches('\0').to_owned())
}

/// Switch the active host using FEAT_CHANGE_HOST (0x1814).
///
/// SetCurrentHost (fn 1): params=[host_slot].
/// The device switches immediately and will not reply; we ignore the result.
pub fn set_active_host(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
    host_slot: u8,
) -> Result<()> {
    let idx = match features.get(&FEAT_CHANGE_HOST) {
        Some(&i) => i,
        None => return Ok(()),
    };
    let _ = feature_call(transport, device, idx, 1, &[host_slot]);
    Ok(())
}

// -- FN key inversion (0x40A0 / 0x40A2 / 0x40A3) -----------------------------

/// FN key inversion state for keyboards.
///
/// fn_swapped: true when F1-F12 behave as multimedia keys by default;
///             false when they behave as standard function keys.
pub struct FnInfo {
    pub fn_swapped:         bool,
    pub default_fn_swapped: bool,
}

fn find_fn_feature(features: &HashMap<u16, u8>) -> Option<u8> {
    features.get(&FEAT_NEW_FN_INVERSION)
        .or_else(|| features.get(&FEAT_FN_INVERSION))
        .or_else(|| features.get(&FEAT_K375S_FN_INVERSION))
        .copied()
}

/// Read FN key inversion state using whichever FN inversion feature the device has.
///
/// GetFnInversionState (fn 0): [inverted_byte, default_byte, ...].
/// Bit 0 of inverted_byte = currently swapped.
pub fn get_fn_settings(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<FnInfo>> {
    let idx = match find_fn_feature(features) {
        Some(i) => i,
        None => return Ok(None),
    };
    let reply = feature_call(transport, device, idx, 0, &[])?;
    let p = reply.params();
    let fn_swapped         = (p.first().copied().unwrap_or(0) & 0x01) != 0;
    let default_fn_swapped = (p.get(1).copied().unwrap_or(0) & 0x01) != 0;
    Ok(Some(FnInfo { fn_swapped, default_fn_swapped }))
}

/// Set FN key inversion state.
///
/// SetFnInversionState (fn 1): params=[inverted_byte] where bit 0 = swapped.
pub fn set_fn_swap(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
    swapped: bool,
) -> Result<()> {
    let idx = match find_fn_feature(features) {
        Some(i) => i,
        None => return Ok(()),
    };
    feature_call(transport, device, idx, 1, &[if swapped { 0x01 } else { 0x00 }])?;
    Ok(())
}

// -- Multiplatform / Set OS (0x4531) ------------------------------------------

/// One platform descriptor returned by FEAT_MULTIPLATFORM.
pub struct PlatformDescriptor {
    pub index: u8,
    /// Human-readable OS name derived from the os_flags bitmask.
    pub name:  String,
}

/// Multiplatform / OS layout selection state.
///
/// can_set:   false when the device does not allow changing the platform.
/// current:   index of the currently active platform descriptor.
/// platforms: list of available platform descriptors in descriptor order.
pub struct MultiplatformInfo {
    pub can_set:   bool,
    pub current:   u8,
    pub platforms: Vec<PlatformDescriptor>,
}

/// Read multiplatform state using FEAT_MULTIPLATFORM (0x4531).
///
/// GetMultiplatformInfo (fn 0): [flags(1), _(1), num_descriptors(1), ..., current_platform(1) at byte 6].
///   flags bit 1 = can set platform.
/// GetPlatformDescriptor (fn 1): params=[index]; [platform_index(1), _(1), os_flags(2 BE), ...].
///   os_flags: 0x2000=macOS, 0x0100=Windows, 0x0400=Linux, 0x4000=iOS, 0x1000=Android.
pub fn get_multiplatform(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<MultiplatformInfo>> {
    let idx = match features.get(&FEAT_MULTIPLATFORM) {
        Some(&i) => i,
        None => return Ok(None),
    };

    let reply = feature_call(transport, device, idx, 0, &[])?;
    let p = reply.params();
    let flags = p.first().copied().unwrap_or(0);
    let can_set = (flags & 0x02) != 0;
    if !can_set { return Ok(None); }

    let num_descriptors = p.get(2).copied().unwrap_or(0) as usize;
    // current platform is at byte 6 of the GetMultiplatformInfo response params.
    let current = p.get(6).copied().unwrap_or(0);

    let mut platforms = Vec::new();
    for i in 0..num_descriptors {
        let desc = feature_call(transport, device, idx, 1, &[i as u8])?;
        let dp = desc.params();
        let platform_index = dp.first().copied().unwrap_or(i as u8);
        let os_flags = if dp.len() >= 4 {
            ((dp[2] as u16) << 8) | (dp[3] as u16)
        } else { 0 };

        let name = os_name_from_flags(os_flags);
        platforms.push(PlatformDescriptor { index: platform_index, name });
    }

    if platforms.len() < 2 { return Ok(None); } // no point showing a picker with <2 options

    Ok(Some(MultiplatformInfo { can_set, current, platforms }))
}

fn os_name_from_flags(flags: u16) -> String {
    if (flags & 0x2000) != 0 { "macOS".to_owned() }
    else if (flags & 0x0100) != 0 { "Windows".to_owned() }
    else if (flags & 0x0400) != 0 { "Linux".to_owned() }
    else if (flags & 0x4000) != 0 { "iOS".to_owned() }
    else if (flags & 0x1000) != 0 { "Android".to_owned() }
    else { "Other".to_owned() }
}

/// Set the active platform using FEAT_MULTIPLATFORM (0x4531).
///
/// SetPlatform (fn 3): params=[0xFF, platform_index].
pub fn set_multiplatform(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
    platform_index: u8,
) -> Result<()> {
    let idx = match features.get(&FEAT_MULTIPLATFORM) {
        Some(&i) => i,
        None => return Ok(()),
    };
    feature_call(transport, device, idx, 3, &[0xFF, platform_index])?;
    Ok(())
}

// -- Backlight2 (0x1982) ------------------------------------------------------

/// Backlight state for a keyboard with FEAT_BACKLIGHT2.
///
/// mode:             0=disabled, 1=automatic, 3=manual.
/// auto_supported:   true if automatic mode is available.
/// manual_supported: true if manual (permanent-on) mode is available.
/// level:            brightness 0-100 (relevant when mode=3).
pub struct BacklightInfo {
    pub mode:             u8,
    pub auto_supported:   bool,
    pub manual_supported: bool,
    pub level:            u8,
}

/// Read backlight state using FEAT_BACKLIGHT2 (0x1982).
///
/// GetBacklightState (fn 0): little-endian [enabled(1), options(1), supported(1), effects(2), level(1), ...].
///   mode = (options >> 3) & 0x03.
///   supported bit 3=auto, bit 5=manual.
/// If enabled==0 the backlight is off (mode reported as 0).
pub fn get_backlight(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
) -> Result<Option<BacklightInfo>> {
    let idx = match features.get(&FEAT_BACKLIGHT2) {
        Some(&i) => i,
        None => return Ok(None),
    };
    let reply = feature_call(transport, device, idx, 0, &[])?;
    let p = reply.params();
    if p.len() < 3 { return Ok(None); }

    let enabled   = p[0] != 0;
    let options   = p[1];
    let supported = p[2];
    // effects field at bytes 3-4; level at byte 5.
    let level     = p.get(5).copied().unwrap_or(0);
    let mode = if enabled { (options >> 3) & 0x03 } else { 0 };
    let auto_supported   = (supported & 0x08) != 0;
    let manual_supported = (supported & 0x20) != 0;

    Ok(Some(BacklightInfo { mode, auto_supported, manual_supported, level }))
}

/// Set backlight mode and brightness using FEAT_BACKLIGHT2 (0x1982).
///
/// Reads current state first to preserve dho/dhi/dpow fields.
/// SetBacklightState (fn 1): little-endian [enabled(1), options(1), 0xFF, level(1), dho(2LE), dhi(2LE), dpow(2LE)].
pub fn set_backlight(
    transport: &Transport,
    device: u8,
    features: &HashMap<u16, u8>,
    mode: u8,
    level: u8,
) -> Result<()> {
    let idx = match features.get(&FEAT_BACKLIGHT2) {
        Some(&i) => i,
        None => return Ok(()),
    };

    // Read current state to preserve options base bits and timing fields.
    let current = feature_call(transport, device, idx, 0, &[])?;
    let cp = current.params();
    if cp.len() < 6 { return Ok(()); }

    let options_raw = cp[1];
    let new_enabled: u8 = if mode == 0 { 0 } else { 1 };
    let new_options = (options_raw & 0x07) | (mode << 3);
    let new_level   = if mode == 3 { level } else { 0 };

    // Preserve dho/dhi/dpow (little-endian u16 at bytes 6-7, 8-9, 10-11).
    let dho_lo = cp.get(6).copied().unwrap_or(0);
    let dho_hi = cp.get(7).copied().unwrap_or(0);
    let dhi_lo = cp.get(8).copied().unwrap_or(0);
    let dhi_hi = cp.get(9).copied().unwrap_or(0);
    let dpow_lo = cp.get(10).copied().unwrap_or(0);
    let dpow_hi = cp.get(11).copied().unwrap_or(0);

    let params = [new_enabled, new_options, 0xFF, new_level, dho_lo, dho_hi, dhi_lo, dhi_hi, dpow_lo, dpow_hi];
    feature_call(transport, device, idx, 1, &params)?;
    Ok(())
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
