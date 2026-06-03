// Receiver enumeration and device management.
// Reference: reference/lib/logitech_receiver/receiver.py, base_usb.py

use hidapi::HidApi;

use crate::devices::types::{Battery, DeviceInfo, DeviceKind};
use crate::error::Result;
use crate::hidpp::{hidpp10, hidpp20};
use crate::transport::Transport;

const LOGITECH_VID: u16 = 0x046D;

/// All known receiver product IDs with their kind and display name.
const RECEIVERS: &[(u16, ReceiverKind, &str)] = &[
    // Bolt
    (0xC548, ReceiverKind::Bolt,       "Bolt Receiver"),
    // Unifying
    (0xC52B, ReceiverKind::Unifying,   "Unifying Receiver"),
    (0xC532, ReceiverKind::Unifying,   "Unifying Receiver"),
    // Nano
    (0xC52F, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC521, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC525, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC526, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC52E, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC531, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC534, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC535, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC537, ReceiverKind::Nano,       "Nano Receiver"),
    // LightSpeed
    (0xC539, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC53A, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC53D, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC53F, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC541, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC545, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC547, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC54D, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    Unifying,
    Bolt,
    Nano,
    LightSpeed,
}

/// Lightweight descriptor returned from enumerate_receivers before opening.
#[derive(Debug, Clone)]
pub struct ReceiverHandle {
    pub path: String,
    pub product_id: u16,
    pub kind: ReceiverKind,
    pub name: &'static str,
}

/// An opened receiver. Provides access to paired device info.
pub struct Receiver {
    transport: Transport,
    pub kind: ReceiverKind,
    pub name: &'static str,
    pub product_id: u16,
    pub serial: String,
    pub max_devices: u8,
}

/// Enumerate all Logitech receivers attached to the machine.
///
/// Filters to usage_page=0xFF00, usage=0x0001 (the HID++ vendor interface) to
/// get exactly one entry per physical receiver and avoid the HID interfaces
/// that require input-monitoring permissions on macOS.
pub fn enumerate_receivers(api: &HidApi) -> Vec<ReceiverHandle> {
    api.device_list()
        .filter(|d| d.vendor_id() == LOGITECH_VID)
        .filter(|d| d.usage_page() == 0xFF00 && d.usage() == 0x0001)
        .filter_map(|d| {
            let pid = d.product_id();
            RECEIVERS.iter().find(|&&(p, _, _)| p == pid).map(|&(_, kind, name)| {
                let path = d.path().to_string_lossy().into_owned();
                ReceiverHandle { path, product_id: pid, kind, name }
            })
        })
        .collect()
}

impl Receiver {
    /// Open a receiver by its handle descriptor.
    pub fn open(api: &HidApi, handle: &ReceiverHandle) -> Result<Self> {
        let transport = Transport::open(api, &handle.path)?;

        // Bolt uses a different register for serial and does not support the standard
        // RECEIVER_INFO sub-register 0x03 that Unifying/Nano/LightSpeed use.
        let (serial, max_devices) = if handle.kind == ReceiverKind::Bolt {
            let serial = hidpp10::get_bolt_serial(&transport)
                .unwrap_or_else(|_| String::from("unknown"));
            (serial, 6u8) // Bolt supports up to 6 paired devices
        } else {
            hidpp10::get_receiver_info(&transport).unwrap_or_else(|_| {
                (String::from("unknown"), 1)
            })
        };

        Ok(Self {
            transport,
            kind: handle.kind,
            name: handle.name,
            product_id: handle.product_id,
            serial,
            max_devices,
        })
    }

    /// Enumerate all devices paired to this receiver.
    ///
    /// For each occupied slot, reads pairing info (HID++ 1.0) to determine WPID
    /// and kind, then tries to read battery and name.
    pub fn enumerate_devices(&self) -> Result<Vec<DeviceInfo>> {
        let mut devices = Vec::new();

        for slot in 1..=self.max_devices {
            // Bolt uses different pairing sub-registers and embeds serial in the pairing response.
            let (wpid, kind, serial, name) = if self.kind == ReceiverKind::Bolt {
                let bolt = match hidpp10::get_bolt_pairing_info(&self.transport, slot)? {
                    Some(p) => p,
                    None => continue,
                };
                let name = hidpp10::get_bolt_device_codename(&self.transport, slot)?
                    .unwrap_or_else(|| format!("Device {}", slot));
                (bolt.wpid, bolt.kind, bytes_to_hex(&bolt.serial), name)
            } else {
                let pairing = match hidpp10::get_pairing_info(&self.transport, slot)? {
                    Some(p) => p,
                    None => continue,
                };
                let serial = hidpp10::get_extended_pairing_info(&self.transport, slot)?
                    .map(|s| bytes_to_hex(&s))
                    .unwrap_or_default();
                let name = hidpp10::get_device_codename(&self.transport, slot)?
                    .unwrap_or_else(|| format!("Device {}", slot));
                (pairing.wpid, pairing.kind, serial, name)
            };

            // Try HID++ 2.0 battery and name first; fall back to HID++ 1.0.
            let (battery, firmware) = self.read_device_info(slot, &kind);

            devices.push(DeviceInfo {
                slot,
                name,
                kind,
                serial,
                wpid,
                battery,
                firmware,
            });
        }

        Ok(devices)
    }

    /// Attempt to read battery and firmware from a device, trying HID++ 2.0 first.
    fn read_device_info(&self, slot: u8, _kind: &DeviceKind) -> (Option<Battery>, Vec<crate::devices::types::FirmwareInfo>) {
        // Probe for HID++ 2.0 support.
        match hidpp20::discover_features(&self.transport, slot) {
            Ok(features) if !features.is_empty() => {
                let battery = hidpp20::get_unified_battery(&self.transport, slot, &features)
                    .ok()
                    .flatten()
                    .or_else(|| {
                        hidpp20::get_battery_status(&self.transport, slot, &features)
                            .ok()
                            .flatten()
                    })
                    .or_else(|| {
                        hidpp20::get_battery_voltage(&self.transport, slot, &features)
                            .ok()
                            .flatten()
                    });
                let firmware = hidpp20::get_firmware(&self.transport, slot, &features)
                    .unwrap_or_default();
                (battery, firmware)
            }
            _ => {
                // HID++ 1.0 fallback.
                let battery = hidpp10::get_battery(&self.transport, slot).ok().flatten();
                let firmware = hidpp10::get_firmware(&self.transport, slot).unwrap_or_default();
                (battery, firmware)
            }
        }
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect()
}

// ---------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------

// Sub-IDs for receiver pairing notifications.
const NOTIF_PAIRING_LOCK:         u8 = 0x4A; // Unifying: lock opened/closed
const NOTIF_DJ_PAIRING:           u8 = 0x41; // Unifying: new device connection
const NOTIF_PASSKEY_REQUEST:      u8 = 0x4D; // Bolt: passkey request
const NOTIF_DEVICE_DISCOVERY:     u8 = 0x4F; // Bolt: device found during discovery
const NOTIF_DISCOVERY_STATUS:     u8 = 0x53; // Bolt: discovery lock open/closed
const NOTIF_PAIRING_STATUS:       u8 = 0x54; // Bolt: pairing lock open/closed

/// Device discovered by a Bolt receiver during the discovery phase.
/// Collected across up to two 0x4F notification chunks before pairing is initiated.
pub struct BoltDiscoveredDevice {
    pub address:        [u8; 6],
    pub authentication: u8,
    pub kind:           u8,
    pub name:           String,
}

/// State kept across calls to Receiver::poll_pairing.
pub struct PairingSession {
    // Unifying: slot of a newly connected device (set on 0x41 notification).
    pub unifying_new_slot:  Option<u8>,
    // Bolt: data collected from 0x4F discovery notifications.
    pub bolt_discovered:    Option<BoltDiscoveredDevice>,
    // Bolt: passkey string (numeric digits to type, or button sequence).
    pub passkey:            Option<String>,
}

impl PairingSession {
    pub fn new() -> Self {
        Self { unifying_new_slot: None, bolt_discovered: None, passkey: None }
    }
}

/// Events returned by Receiver::poll_pairing.
pub enum PairingEvent {
    /// Still waiting; no relevant notification received within the timeout.
    Waiting,
    /// Bolt: a compatible device was found and pairing has been initiated.
    BoltDeviceFound(String),
    /// Bolt: numeric passkey -- user must type these digits on the keyboard, then press Enter.
    PasskeyNumeric(String),
    /// Bolt: button passkey -- encoded as a string of 'L'/'R' characters; press simultaneously when done.
    PasskeyButton(String),
    /// Pairing complete. slot is the 1-based slot of the new device.
    Paired(u8),
    /// Pairing failed.
    Failed(String),
}

impl Receiver {
    /// Unpair the device in the given 1-based slot.
    pub fn unpair_device(&self, slot: u8) -> Result<()> {
        if self.kind == ReceiverKind::Bolt {
            hidpp10::bolt_unpair_device(&self.transport, slot)
        } else {
            hidpp10::unpair_device(&self.transport, slot)
        }
    }

    /// Open the pairing lock (Unifying) or start device discovery (Bolt).
    /// Returns a PairingSession that must be passed to poll_pairing.
    /// timeout_secs: maximum time the receiver will wait for a device (0-255).
    pub fn start_pairing(&self, timeout_secs: u8) -> Result<PairingSession> {
        // Enable wireless connection notifications so we receive pairing events.
        if let Ok(flags) = hidpp10::get_notification_flags(&self.transport) {
            let needed = hidpp10::NOTIF_WIRELESS | hidpp10::NOTIF_SOFTWARE_PRESENT;
            if flags & needed != needed {
                let _ = hidpp10::set_notification_flags(&self.transport, flags | needed);
            }
        }

        if self.kind == ReceiverKind::Bolt {
            hidpp10::bolt_start_discovery(&self.transport, false, timeout_secs)?;
        } else {
            hidpp10::set_pairing_lock(&self.transport, true, timeout_secs)?;
        }
        Ok(PairingSession::new())
    }

    /// Poll for one pairing event. Blocks for at most timeout_ms milliseconds.
    /// Call in a loop (e.g. every 200 ms) until Paired or Failed is returned.
    pub fn poll_pairing(&self, session: &mut PairingSession, timeout_ms: i32) -> Result<PairingEvent> {
        let msg = match self.transport.read_notification(timeout_ms)? {
            Some(m) => m,
            None    => return Ok(PairingEvent::Waiting),
        };

        // Ignore HID++ reply messages (sub_id >= 0x80); we only care about notifications.
        let sub_id = msg.sub_id();
        if sub_id >= 0x80 {
            return Ok(PairingEvent::Waiting);
        }

        let address = msg.address();
        let params  = msg.params().to_vec(); // clone to avoid borrow issues

        if self.kind == ReceiverKind::Bolt {
            match sub_id {
                NOTIF_DISCOVERY_STATUS => {
                    // address 0x00: discovering; anything else: stopped.
                    let error = params.first().copied().unwrap_or(0);
                    if address != 0x00 && error != 0 {
                        return Ok(PairingEvent::Failed(format!("discovery error 0x{:02X}", error)));
                    }
                    Ok(PairingEvent::Waiting)
                }
                NOTIF_DEVICE_DISCOVERY => {
                    // Two chunks arrive for each discovered device.
                    // Chunk type is at params[1]: 0 = address/auth/kind, 1 = name.
                    let chunk = params.get(1).copied().unwrap_or(0xFF);
                    if chunk == 0 && params.len() >= 15 {
                        let mut addr = [0u8; 6];
                        addr.copy_from_slice(&params[6..12]);
                        session.bolt_discovered = Some(BoltDiscoveredDevice {
                            address:        addr,
                            authentication: params[14],
                            kind:           params[3],
                            name:           String::new(),
                        });
                    } else if chunk == 1 {
                        if let Some(dev) = &mut session.bolt_discovered {
                            let name_len = params.get(2).copied().unwrap_or(0) as usize;
                            let end = (3 + name_len).min(params.len());
                            dev.name = String::from_utf8_lossy(&params[3..end]).into_owned();
                        }
                        // Kick off pairing once we have both chunks.
                        if let Some(dev) = &session.bolt_discovered {
                            if !dev.name.is_empty() {
                                // Keyboards get 20 bits of entropy (numeric passkey);
                                // other devices get 10 (button passkey).
                                let entropy = if dev.kind == 1 { 20u8 } else { 10u8 };
                                hidpp10::bolt_pair_device(
                                    &self.transport, 0, &dev.address, dev.authentication, entropy,
                                )?;
                                return Ok(PairingEvent::BoltDeviceFound(dev.name.clone()));
                            }
                        }
                    }
                    Ok(PairingEvent::Waiting)
                }
                NOTIF_PASSKEY_REQUEST => {
                    // params[0..6] = passkey as ASCII digits or a button bitmask string.
                    let raw = &params[..6.min(params.len())];
                    let passkey = String::from_utf8_lossy(raw).trim_end_matches('\0').to_owned();
                    session.passkey = Some(passkey.clone());
                    // Determine keyboard vs button passkey by authentication flags.
                    // authentication bit 0 = requires numeric passkey.
                    let auth = session.bolt_discovered.as_ref().map(|d| d.authentication).unwrap_or(0);
                    if auth & 0x01 != 0 {
                        Ok(PairingEvent::PasskeyNumeric(passkey))
                    } else {
                        // Encode button bitmask as L/R string: each bit 1=right, 0=left.
                        let buttons = passkey_to_buttons(raw);
                        Ok(PairingEvent::PasskeyButton(buttons))
                    }
                }
                NOTIF_PAIRING_STATUS => {
                    // address 0x00: lock open; 0x01: lock closed no device; 0x02: paired.
                    let error = params.first().copied().unwrap_or(0);
                    if error != 0 {
                        return Ok(PairingEvent::Failed(format!("pairing error 0x{:02X}", error)));
                    }
                    if address == 0x02 {
                        let slot = params.get(7).copied().unwrap_or(0);
                        return Ok(PairingEvent::Paired(slot));
                    }
                    Ok(PairingEvent::Waiting)
                }
                _ => Ok(PairingEvent::Waiting),
            }
        } else {
            // Unifying / Nano / LightSpeed
            match sub_id {
                NOTIF_DJ_PAIRING if msg.device() != 0xFF => {
                    // New device connected; device() holds its 1-based slot.
                    session.unifying_new_slot = Some(msg.device());
                    Ok(PairingEvent::Waiting)
                }
                NOTIF_PAIRING_LOCK => {
                    let lock_open = address & 0x01 != 0;
                    let error     = params.first().copied().unwrap_or(0);
                    if lock_open {
                        return Ok(PairingEvent::Waiting);
                    }
                    // Lock closed.
                    if error != 0 {
                        return Ok(PairingEvent::Failed(format!("pairing error 0x{:02X}", error)));
                    }
                    match session.unifying_new_slot {
                        Some(slot) => Ok(PairingEvent::Paired(slot)),
                        None       => Ok(PairingEvent::Failed("no device connected before lock closed".to_owned())),
                    }
                }
                _ => Ok(PairingEvent::Waiting),
            }
        }
    }

    /// Cancel an in-progress pairing by closing the lock / stopping discovery.
    pub fn cancel_pairing(&self) -> Result<()> {
        if self.kind == ReceiverKind::Bolt {
            hidpp10::bolt_start_discovery(&self.transport, true, 0)?;
        } else {
            hidpp10::set_pairing_lock(&self.transport, false, 0)?;
        }
        Ok(())
    }
}

/// Convert a Bolt button-passkey byte slice to an L/R string.
/// Each bit: 1=right, 0=left. First bit is MSB of first byte.
/// Solaar uses 10 bits, so we produce 10 characters.
fn passkey_to_buttons(raw: &[u8]) -> String {
    let mut bits = Vec::with_capacity(10);
    'outer: for byte in raw {
        for shift in (0..8u8).rev() {
            bits.push(if byte & (1 << shift) != 0 { 'R' } else { 'L' });
            if bits.len() == 10 { break 'outer; }
        }
    }
    bits.iter().collect()
}
