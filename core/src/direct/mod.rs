// Direct (Bluetooth) device support.
//
// Logitech devices connected to the host via Bluetooth rather than through a
// USB receiver show up as standalone HID devices. They speak HID++ 2.0 using
// device index 0xFF (the "self" index for direct connections, same as the value
// used to address a receiver itself on a receiver transport).
//
// Enumeration strategy:
//   1. List all hidapi devices with Logitech vendor ID (0x046D).
//   2. Keep only usage_page=0xFF00 interfaces with usage != 0x0001.
//      (Receivers use usage=0x0001; BT HID++ typically uses usage=0x0002.)
//   3. Skip any interface whose product ID matches a known receiver PID
//      -- some receivers expose additional HID interfaces and we must not
//      misidentify them as direct devices.
//   4. Open each candidate, probe HID++ 2.0 ROOT feature (single short
//      request). If the device responds with a non-empty feature map,
//      read name and battery. Discard non-HID++ interfaces silently.
//
// Reference: reference/lib/logitech_receiver/base_usb.py (BT device detection)

use hidapi::HidApi;

use crate::devices::types::{Battery, DeviceKind};
use crate::hidpp::{hidpp20, message::RECEIVER_DEVICE};
use crate::receiver::RECEIVER_PIDS;
use crate::transport::Transport;

pub const LOGITECH_VID: u16 = 0x046D;

/// All information read about a directly-connected (Bluetooth) Logitech device.
#[derive(Debug, Clone)]
pub struct DirectDeviceInfo {
    pub product_id: u16,
    pub name:       String,
    pub serial:     String,
    pub kind:       DeviceKind,
    pub battery:    Option<Battery>,
}

/// Enumerate all directly-connected Logitech HID++ 2.0 devices (Bluetooth).
///
/// Opens each candidate interface briefly to probe for HID++ 2.0 support,
/// then closes it. Returns only devices that successfully respond to the probe.
pub fn enumerate_direct_devices(api: &HidApi) -> Vec<DirectDeviceInfo> {
    // Collect candidate (path, product_id, serial) tuples up front to avoid
    // holding the device_list iterator while also calling api methods below.
    let candidates: Vec<(String, u16, String)> = api
        .device_list()
        .filter(|d| d.vendor_id() == LOGITECH_VID)
        .filter(|d| d.usage_page() == 0xFF00 && d.usage() != 0x0001)
        .filter(|d| !RECEIVER_PIDS.contains(&d.product_id()))
        .map(|d| {
            let path   = d.path().to_string_lossy().into_owned();
            let pid    = d.product_id();
            let serial = d.serial_number().unwrap_or("").to_owned();
            (path, pid, serial)
        })
        .collect();

    let mut result = Vec::new();

    for (path, product_id, serial) in candidates {
        let transport = match Transport::open(api, &path) {
            Ok(t)  => t,
            Err(_) => continue,
        };

        // Probe HID++ 2.0 at device index 0xFF (direct self-index).
        // discover_features returns an empty map for HID++ 1.0 and non-HID++ devices.
        let features = match hidpp20::discover_features(&transport, RECEIVER_DEVICE) {
            Ok(f) if !f.is_empty() => f,
            _                       => continue,
        };

        // Device name: feature 0x0005 if present, else a placeholder.
        let name = hidpp20::get_device_name(&transport, RECEIVER_DEVICE, &features)
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("Logitech Device {:04X}", product_id));

        // Battery: try unified (0x1004), then status (0x1000), then voltage (0x1001).
        let battery = hidpp20::get_unified_battery(&transport, RECEIVER_DEVICE, &features)
            .ok()
            .flatten()
            .or_else(|| {
                hidpp20::get_battery_status(&transport, RECEIVER_DEVICE, &features)
                    .ok()
                    .flatten()
            })
            .or_else(|| {
                hidpp20::get_battery_voltage(&transport, RECEIVER_DEVICE, &features)
                    .ok()
                    .flatten()
            });

        // Infer device kind from a sibling generic-HID interface for the same PID.
        // The 0xFF00 interface does not carry usage semantics for the device type;
        // the 0x0001 (Generic Desktop) interface does via its usage value.
        let kind = api
            .device_list()
            .filter(|d| d.vendor_id() == LOGITECH_VID && d.product_id() == product_id)
            .filter(|d| d.usage_page() == 0x0001)
            .find_map(|d| kind_from_generic_desktop_usage(d.usage()))
            .unwrap_or(DeviceKind::Unknown);

        result.push(DirectDeviceInfo { product_id, name, serial, kind, battery });
    }

    result
}

/// Map a Generic Desktop (usage_page=0x0001) usage value to a DeviceKind.
fn kind_from_generic_desktop_usage(usage: u16) -> Option<DeviceKind> {
    match usage {
        0x02 => Some(DeviceKind::Mouse),
        0x04 => Some(DeviceKind::Joystick),
        0x05 => Some(DeviceKind::Gamepad),
        0x06 => Some(DeviceKind::Keyboard),
        _    => None,
    }
}
