// HID++ 1.0 protocol implementation.
// Reference: reference/lib/logitech_receiver/hidpp10.py, hidpp10_constants.py

use crate::error::{Error, Hidpp10Error, Result};
use crate::hidpp::message::{Message, RECEIVER_DEVICE};
use crate::transport::Transport;
use crate::devices::types::{Battery, BatteryLevel, BatteryStatus, DeviceKind, FirmwareInfo, FirmwareKind};

/// HID++ 1.0 register addresses.
/// Registers >= 0x200 are "long" registers; they use sub_id 0x83 and return LONG messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Register {
    Notifications       = 0x00,
    MouseButtonFlags    = 0x01,
    ReceiverConnection  = 0x02,
    DevicesConfig       = 0x03,
    BatteryStatus       = 0x07,
    BatteryCharge       = 0x0D,
    ThreeLeds           = 0x51,
    ReceiverPairing     = 0xB2,
    Firmware            = 0xF1,
    DevicesActivity     = 0x2B3,
    ReceiverInfo        = 0x2B5,
    BoltDeviceDiscovery = 0xC0,
    BoltPairing         = 0x2C1,
    BoltUniqueId        = 0x2FB,
}

/// Sub-registers for Register::ReceiverInfo (0x2B5).
/// For per-slot registers, add the 0-based slot index: e.g. PairingInfo + slot.
#[repr(u8)]
pub enum InfoSubReg {
    SerialNumber           = 0x01,
    FirmwareVersion        = 0x02,
    ReceiverInformation    = 0x03,
    PairingInformation     = 0x20, // + slot (0-based)
    ExtendedPairingInfo    = 0x30, // + slot (0-based)
    DeviceName             = 0x40, // + slot (0-based)
    BoltPairingInformation = 0x50, // + slot (0-based)
    BoltDeviceName         = 0x60, // + slot (0-based)
}

/// Compute the sub_id and address bytes for a register read.
/// request_id = 0x8100 | (register & 0x2FF)
fn read_ids(register: u16) -> (u8, u8) {
    let id = 0x8100u16 | (register & 0x2FF);
    ((id >> 8) as u8, (id & 0xFF) as u8)
}

/// Compute the sub_id and address bytes for a register write.
/// request_id = 0x8000 | (register & 0x2FF)
fn write_ids(register: u16) -> (u8, u8) {
    let id = 0x8000u16 | (register & 0x2FF);
    ((id >> 8) as u8, (id & 0xFF) as u8)
}

/// Read a short register (0x00-0xFF). Returns a Message whose params() contain
/// the 3-byte reply payload.
pub fn read_short(transport: &Transport, device: u8, reg: Register, p0: u8) -> Result<Message> {
    let (sub_id, address) = read_ids(reg as u16);
    let req = Message::short(device, sub_id, address, p0, 0, 0);
    transport.request(&req)
}

/// Read a long register (0x200-0x2FF) with a sub-register selector.
/// The transport will verify that the reply echoes the sub_reg byte.
pub fn read_long(transport: &Transport, device: u8, reg: Register, sub_reg: u8) -> Result<Message> {
    let (sub_id, address) = read_ids(reg as u16);
    let req = Message::short(device, sub_id, address, sub_reg, 0, 0);
    transport.request(&req)
}

/// Write a short register.
pub fn write_short(transport: &Transport, device: u8, reg: Register, p0: u8, p1: u8, p2: u8) -> Result<Message> {
    let (sub_id, address) = write_ids(reg as u16);
    let req = Message::short(device, sub_id, address, p0, p1, p2);
    transport.request(&req)
}

// -- Receiver queries ---------------------------------------------------------

/// Read receiver serial and max supported devices from RECEIVER_INFO sub-reg 0x03.
///
/// Reply params layout (0-indexed):
///   [0]    = 0x03 (sub-register echo)
///   [1..4] = serial (4 bytes)
///   [6]    = max_devices
pub fn get_receiver_info(transport: &Transport) -> Result<(String, u8)> {
    let msg = read_long(transport, RECEIVER_DEVICE, Register::ReceiverInfo, InfoSubReg::ReceiverInformation as u8)?;
    let p = msg.params();
    if p.len() < 7 {
        return Err(Error::InvalidResponse);
    }
    let serial = bytes_to_hex(&p[1..5]);
    let max_devices = p[6];
    Ok((serial, max_devices))
}

/// Read basic pairing info for a given slot (1-based).
///
/// Reply params layout:
///   [0]    = sub-register echo
///   [2]    = polling rate in ms
///   [3..5] = WPID (2 bytes, big-endian)
///   [7]    = device kind (low nibble)
pub fn get_pairing_info(transport: &Transport, slot: u8) -> Result<Option<PairingInfo>> {
    let sub_reg = InfoSubReg::PairingInformation as u8 + slot - 1;
    match read_long(transport, RECEIVER_DEVICE, Register::ReceiverInfo, sub_reg) {
        Err(Error::Hidpp10(Hidpp10Error::UnknownDevice | Hidpp10Error::InvalidValue)) => return Ok(None),
        Err(e) => return Err(e),
        Ok(msg) => {
            let p = msg.params();
            if p.len() < 8 {
                return Err(Error::InvalidResponse);
            }
            let wpid = [p[3], p[4]];
            let kind = DeviceKind::from_byte(p[7] & 0x0F);
            let polling_rate_ms = p[2];
            Ok(Some(PairingInfo { wpid, kind, polling_rate_ms }))
        }
    }
}

/// Read extended pairing info for a given slot (serial number).
///
/// Reply params layout:
///   [0]    = sub-register echo
///   [1..5] = serial (4 bytes)
pub fn get_extended_pairing_info(transport: &Transport, slot: u8) -> Result<Option<[u8; 4]>> {
    let sub_reg = InfoSubReg::ExtendedPairingInfo as u8 + slot - 1;
    match read_long(transport, RECEIVER_DEVICE, Register::ReceiverInfo, sub_reg) {
        Err(Error::Hidpp10(Hidpp10Error::UnknownDevice | Hidpp10Error::InvalidValue)) => return Ok(None),
        Err(e) => return Err(e),
        Ok(msg) => {
            let p = msg.params();
            if p.len() < 5 {
                return Err(Error::InvalidResponse);
            }
            Ok(Some([p[1], p[2], p[3], p[4]]))
        }
    }
}

/// Read the device name (codename) for a given slot.
///
/// Reply params layout:
///   [0]        = sub-register echo
///   [1]        = name length
///   [2..2+len] = ASCII name
pub fn get_device_codename(transport: &Transport, slot: u8) -> Result<Option<String>> {
    let sub_reg = InfoSubReg::DeviceName as u8 + slot - 1;
    match read_long(transport, RECEIVER_DEVICE, Register::ReceiverInfo, sub_reg) {
        Err(Error::Hidpp10(Hidpp10Error::UnknownDevice | Hidpp10Error::InvalidValue)) => return Ok(None),
        Err(e) => return Err(e),
        Ok(msg) => {
            let p = msg.params();
            if p.len() < 2 {
                return Ok(None);
            }
            let name_len = p[1] as usize;
            let end = (2 + name_len).min(p.len());
            let name = String::from_utf8_lossy(&p[2..end]).into_owned();
            Ok(Some(name))
        }
    }
}

// -- Device queries -----------------------------------------------------------

/// Read battery from a HID++ 1.0 device. Tries BATTERY_CHARGE (0x0D) first,
/// then falls back to BATTERY_STATUS (0x07).
pub fn get_battery(transport: &Transport, device: u8) -> Result<Option<Battery>> {
    // Try BATTERY_CHARGE register first.
    match read_short(transport, device, Register::BatteryCharge, 0) {
        Ok(msg) => {
            let p = msg.params();
            return Ok(Some(parse_battery_charge(p)));
        }
        Err(Error::Hidpp10(Hidpp10Error::InvalidSubId | Hidpp10Error::RequestUnavailable)) => {}
        Err(e) => return Err(e),
    }

    // Fall back to BATTERY_STATUS register.
    match read_short(transport, device, Register::BatteryStatus, 0) {
        Ok(msg) => {
            let p = msg.params();
            Ok(Some(parse_battery_status(p)))
        }
        Err(Error::Hidpp10(Hidpp10Error::InvalidSubId | Hidpp10Error::RequestUnavailable)) => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Parse a BATTERY_CHARGE (0x0D) response.
/// params[0] = charge 0-100, params[2] high nibble = status
fn parse_battery_charge(p: &[u8]) -> Battery {
    let level = p.first().copied();
    let status = p.get(2).map(|&b| match b & 0xF0 {
        0x30 => BatteryStatus::Discharging,
        0x50 => BatteryStatus::Recharging,
        0x90 => BatteryStatus::Full,
        _ => BatteryStatus::Discharging,
    });
    Battery { level, status, voltage: None }
}

/// Parse a BATTERY_STATUS (0x07) response.
/// params[0] = level code (1=critical, 3=low, 5=good, 7=full)
/// params[1] = charging byte
fn parse_battery_status(p: &[u8]) -> Battery {
    let level_code = p.first().copied().unwrap_or(0);
    let charging_byte = p.get(1).copied().unwrap_or(0);

    let level = match level_code {
        7 => Some(BatteryLevel::Full as u8),
        5 => Some(BatteryLevel::Good as u8),
        3 => Some(BatteryLevel::Low as u8),
        1 => Some(BatteryLevel::Critical as u8),
        _ => None,
    };

    let status = if charging_byte == 0x00 {
        Some(BatteryStatus::Discharging)
    } else if charging_byte & 0x21 == 0x21 {
        Some(BatteryStatus::Recharging)
    } else if charging_byte & 0x22 == 0x22 {
        Some(BatteryStatus::Full)
    } else {
        None
    };

    Battery { level, status, voltage: None }
}

/// Read firmware version from a HID++ 1.0 device.
pub fn get_firmware(transport: &Transport, device: u8) -> Result<Vec<FirmwareInfo>> {
    let mut result = Vec::new();

    // Firmware version (sub-register 0x01).
    if let Ok(msg) = read_short(transport, device, Register::Firmware, 0x01) {
        let p = msg.params();
        if p.len() >= 3 {
            let v = format!("{:02X}.{:02X}", p[1], p[2]);
            let version = if let Ok(msg2) = read_short(transport, device, Register::Firmware, 0x02) {
                let p2 = msg2.params();
                if p2.len() >= 3 { format!("{}.B{:02X}{:02X}", v, p2[1], p2[2]) } else { v }
            } else {
                v
            };
            result.push(FirmwareInfo { kind: FirmwareKind::Firmware, version });
        }
    }

    // Bootloader version (sub-register 0x04).
    if let Ok(msg) = read_short(transport, device, Register::Firmware, 0x04) {
        let p = msg.params();
        if p.len() >= 3 {
            result.push(FirmwareInfo {
                kind: FirmwareKind::Bootloader,
                version: format!("{:02X}.{:02X}", p[1], p[2]),
            });
        }
    }

    Ok(result)
}

// -- Bolt-specific receiver queries -------------------------------------------

/// Read the Bolt receiver unique ID from Register::BoltUniqueId (0x2FB).
/// Returns the raw params as a hex string (used as the serial number).
pub fn get_bolt_serial(transport: &Transport) -> Result<String> {
    let msg = read_long(transport, RECEIVER_DEVICE, Register::BoltUniqueId, 0)?;
    Ok(bytes_to_hex(msg.params()))
}

/// Read Bolt pairing info for a given slot (1-based).
///
/// Bolt uses sub-register 0x50+slot (1-based index) instead of the Unifying
/// 0x20+slot-1. The serial number is embedded in the pairing response.
///
/// Reply params layout:
///   [0]    = sub-register echo
///   [1]    = device kind (low nibble)
///   [2]    = WPID low byte
///   [3]    = WPID high byte
///   [4..8] = serial (4 bytes)
pub fn get_bolt_pairing_info(transport: &Transport, slot: u8) -> Result<Option<BoltPairingInfo>> {
    let sub_reg = InfoSubReg::BoltPairingInformation as u8 + slot;
    match read_long(transport, RECEIVER_DEVICE, Register::ReceiverInfo, sub_reg) {
        Err(Error::Hidpp10(Hidpp10Error::UnknownDevice | Hidpp10Error::UnsupportedParam)) => Ok(None),
        Err(e) => Err(e),
        Ok(msg) => {
            let p = msg.params();
            // Treat short/unexpected replies as empty slot rather than hard errors.
            if p.len() < 8 {
                return Ok(None);
            }
            // Bolt stores WPID bytes reversed relative to Unifying: high at [3], low at [2].
            let wpid = [p[3], p[2]];
            let kind = DeviceKind::from_byte(p[1] & 0x0F);
            let serial = [p[4], p[5], p[6], p[7]];
            Ok(Some(BoltPairingInfo { wpid, kind, serial }))
        }
    }
}

/// Read the device name for a Bolt paired device (1-based slot).
///
/// Bolt device name uses sub-register 0x60+slot with an extra param 0x01.
///
/// Reply params layout:
///   [0]        = sub-register echo
///   [1]        = unused
///   [2]        = name length
///   [3..3+len] = ASCII name (up to 14 chars)
pub fn get_bolt_device_codename(transport: &Transport, slot: u8) -> Result<Option<String>> {
    let sub_reg = InfoSubReg::BoltDeviceName as u8 + slot;
    let (sub_id, address) = read_ids(Register::ReceiverInfo as u16);
    let req = Message::short(RECEIVER_DEVICE, sub_id, address, sub_reg, 0x01, 0);
    match transport.request(&req) {
        Err(Error::Hidpp10(Hidpp10Error::UnknownDevice | Hidpp10Error::UnsupportedParam)) => Ok(None),
        Err(e) => Err(e),
        Ok(msg) => {
            let p = msg.params();
            if p.len() < 3 {
                return Ok(None);
            }
            let name_len = (p[2] as usize).min(14);
            let end = (3 + name_len).min(p.len());
            let name = String::from_utf8_lossy(&p[3..end]).into_owned();
            Ok(Some(name))
        }
    }
}

// -- Helpers ------------------------------------------------------------------

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect()
}

/// Basic pairing info read from the receiver for a paired device slot.
#[derive(Debug, Clone)]
pub struct PairingInfo {
    pub wpid: [u8; 2],
    pub kind: DeviceKind,
    pub polling_rate_ms: u8,
}

/// Pairing info for a Bolt device slot (serial embedded in the pairing response).
#[derive(Debug, Clone)]
pub struct BoltPairingInfo {
    pub wpid: [u8; 2],
    pub kind: DeviceKind,
    pub serial: [u8; 4],
}
