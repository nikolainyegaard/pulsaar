/// What kind of device this is.
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

impl BatteryStatus {
    /// Decode the charging-status byte shared by FEAT_BATTERY_STATUS (0x1000)
    /// and FEAT_UNIFIED_BATTERY (0x1004); the discriminants equal it.
    pub fn from_byte(b: u8) -> Self {
        match b {
            1 => Self::Recharging,
            2 => Self::AlmostFull,
            3 => Self::Full,
            4 => Self::SlowRecharge,
            5 => Self::InvalidBattery,
            6 => Self::ThermalError,
            _ => Self::Discharging,
        }
    }
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
    /// True when the device answered HID++ 2.0 during enumeration.
    pub online: bool,
}
