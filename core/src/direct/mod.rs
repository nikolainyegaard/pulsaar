// Direct (Bluetooth) device support.
//
// Logitech devices connected to the host via Bluetooth rather than through a
// USB receiver show up as standalone HID devices.
//
// Enumeration strategy:
//   1. List all hidapi devices with Logitech vendor ID (0x046D).
//   2. Keep only usage_page=0xFF00 or 0xFF43 interfaces (Logitech HID++ vendor pages).
//      Skip any interface whose product ID matches a known receiver PID.
//   3. Deduplicate by IOKit path -- BT devices expose multiple interfaces at the
//      same path; deduplication avoids probing the same physical device twice.
//   4. For each candidate, read name and kind from device_list metadata (no open
//      needed -- these come from the HID descriptor).
//   5. Attempt HID++ 2.0 probe to read name and battery. On macOS, BT LE HID
//      devices are marked Privileged=Yes in IOKit and cannot be opened by
//      third-party apps -- open_bt will fail with kIOReturnNotPermitted. Battery
//      for BT LE devices would require CoreBluetooth GATT (Battery Service 0x180F),
//      which is a separate implementation path. Devices are emitted with the
//      descriptor name and kind even when the probe fails.
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

/// Enumerate all directly-connected Logitech devices (Bluetooth).
///
/// Returns one entry per physical device. Name and kind always come from HID
/// descriptor metadata. Battery is populated only if the HID++ 2.0 probe
/// succeeds (possible for Classic BT devices; blocked for BT LE on macOS).
///
/// `unprobeable`: paths where open previously failed (e.g. Privileged=Yes BT LE
/// devices on macOS). Paths are skipped on input and added on new failures, so
/// the OS TCC deny is triggered at most once per path per session.
pub fn enumerate_direct_devices(
    api:         &HidApi,
    unprobeable: &mut std::collections::HashSet<String>,
) -> Vec<DirectDeviceInfo> {
    // Logitech uses two vendor usage pages for HID++:
    //   0xFF00 -- USB receivers and some older BT devices
    //   0xFF43 -- Bluetooth HID++ (e.g. MX Anywhere3SB, MX Keys via BT)
    // BT devices expose multiple interfaces at the same IOKit path; deduplicate
    // so we probe each physical device once.
    let mut seen_paths = std::collections::HashSet::new();
    let candidates: Vec<(String, u16, String)> = api
        .device_list()
        .filter(|d| d.vendor_id() == LOGITECH_VID)
        .filter(|d| d.usage_page() == 0xFF00 || d.usage_page() == 0xFF43)
        .filter(|d| !RECEIVER_PIDS.contains(&d.product_id()))
        .map(|d| {
            let path   = d.path().to_string_lossy().into_owned();
            let pid    = d.product_id();
            let serial = d.serial_number().unwrap_or("").to_owned();
            (path, pid, serial)
        })
        .filter(|(path, _, _)| seen_paths.insert(path.clone()))
        .collect();

    let mut result = Vec::new();

    for (path, product_id, serial) in candidates {
        // Name and kind from descriptor metadata -- available without opening the device.
        let name_from_descriptor: String = api
            .device_list()
            .find(|d| d.vendor_id() == LOGITECH_VID && d.product_id() == product_id)
            .and_then(|d| d.product_string())
            .map(|s| s.to_owned())
            .unwrap_or_else(|| format!("Logitech Device {:04X}", product_id));

        let kind = api
            .device_list()
            .filter(|d| d.vendor_id() == LOGITECH_VID && d.product_id() == product_id)
            .filter(|d| d.usage_page() == 0x0001)
            .find_map(|d| kind_from_generic_desktop_usage(d.usage()))
            .unwrap_or(DeviceKind::Unknown);

        // Attempt HID++ 2.0 probe for name and battery. On macOS, BT LE devices are
        // marked Privileged=Yes and the open will fail -- emit with descriptor info only.
        // Skip the open entirely for paths that already failed (avoids repeated TCC denies).
        if unprobeable.contains(&path) {
            result.push(DirectDeviceInfo {
                product_id, name: name_from_descriptor, serial, kind, battery: None,
            });
            continue;
        }

        let transport = match Transport::open(api, &path, &path) {
            Ok(t)  => t,
            Err(_) => {
                unprobeable.insert(path);
                result.push(DirectDeviceInfo {
                    product_id, name: name_from_descriptor, serial, kind, battery: None,
                });
                continue;
            }
        };

        let features = match hidpp20::discover_features(&transport, RECEIVER_DEVICE) {
            Ok(f) if !f.is_empty() => f,
            _ => {
                result.push(DirectDeviceInfo {
                    product_id, name: name_from_descriptor, serial, kind, battery: None,
                });
                continue;
            }
        };

        let name = hidpp20::get_device_name(&transport, RECEIVER_DEVICE, &features)
            .ok()
            .flatten()
            .or_else(|| {
                hidpp20::get_friendly_name(&transport, RECEIVER_DEVICE, &features)
                    .ok()
                    .flatten()
            })
            .unwrap_or(name_from_descriptor);

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
