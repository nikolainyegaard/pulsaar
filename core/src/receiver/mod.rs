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
