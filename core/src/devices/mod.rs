// Device types and descriptors.
// Reference: reference/lib/logitech_receiver/descriptors.py, hidpp20_devices.py

pub mod types;

pub use types::{Battery, BatteryLevel, BatteryStatus, DeviceInfo, DeviceKind, FirmwareInfo, FirmwareKind};
