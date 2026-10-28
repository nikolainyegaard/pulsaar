/// What kind of device this is.
/// Discriminants are the FFI encoding (CDeviceInfo.kind); keep them stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DeviceKind {
    Unknown = 0,
    Keyboard = 1,
    Mouse = 2,
    Numpad = 3,
    Presenter = 4,
    Remote = 5,
    Trackball = 6,
    Touchpad = 7,
    Tablet = 8,
    Gamepad = 9,
    Joystick = 10,
    Headset = 11,
    RemoteControl = 12,
    Receiver = 13,
}

impl DeviceKind {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x01 => Self::Keyboard,
            0x02 => Self::Mouse,
            0x03 => Self::Numpad,
            0x04 => Self::Presenter,
            0x07 => Self::Remote,
            0x08 => Self::Trackball,
            0x09 => Self::Touchpad,
            0x0A => Self::Tablet,
            0x0B => Self::Gamepad,
            0x0C => Self::Joystick,
            0x0D => Self::Headset,
            0x0E => Self::RemoteControl,
            0x0F => Self::Receiver,
            _ => Self::Unknown,
        }
    }
}

/// Battery charging/discharging status.
/// Discriminants are the FFI encoding (CBattery.status); keep them stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BatteryStatus {
    Discharging = 0,
    Recharging = 1,
    AlmostFull = 2,
    Full = 3,
    SlowRecharge = 4,
    InvalidBattery = 5,
    ThermalError = 6,
}

/// Battery state for a device.
#[derive(Debug, Clone)]
pub struct Battery {
    /// Charge level 0-100.
    pub level: Option<u8>,
    pub status: Option<BatteryStatus>,
    /// Voltage in mV, when available (HID++ 2.0 battery voltage feature).
    pub voltage: Option<u16>,
}

/// All information Pulsaar has read about a paired device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Slot number on the receiver (1-based).
    pub slot: u8,
    /// Human-readable name, e.g. "MX Master 3S".
    pub name: String,
    pub kind: DeviceKind,
    /// Serial number as a hex string.
    pub serial: String,
    /// Wireless product ID (2 bytes).
    pub wpid: [u8; 2],
    pub battery: Option<Battery>,
}
