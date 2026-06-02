/// What kind of device this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Unknown,
    Keyboard,
    Mouse,
    Numpad,
    Presenter,
    Remote,
    Trackball,
    Touchpad,
    Tablet,
    Gamepad,
    Joystick,
    Headset,
    RemoteControl,
    Receiver,
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

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Keyboard => "keyboard",
            Self::Mouse => "mouse",
            Self::Numpad => "numpad",
            Self::Presenter => "presenter",
            Self::Remote => "remote",
            Self::Trackball => "trackball",
            Self::Touchpad => "touchpad",
            Self::Tablet => "tablet",
            Self::Gamepad => "gamepad",
            Self::Joystick => "joystick",
            Self::Headset => "headset",
            Self::RemoteControl => "remote_control",
            Self::Receiver => "receiver",
        }
    }
}

impl std::fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Approximate battery level, used when the device reports a category rather than a percentage.
#[repr(u8)]
pub enum BatteryLevel {
    Empty    = 0,
    Critical = 5,
    Low      = 20,
    Good     = 50,
    Full     = 90,
}

/// Battery charging/discharging status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryStatus {
    Discharging,
    Recharging,
    AlmostFull,
    Full,
    SlowRecharge,
    InvalidBattery,
    ThermalError,
}

impl BatteryStatus {
    pub fn is_charging(&self) -> bool {
        matches!(self, Self::Recharging | Self::AlmostFull | Self::Full | Self::SlowRecharge)
    }
}

/// Battery state for a device.
#[derive(Debug, Clone)]
pub struct Battery {
    /// Charge level 0-100, or an approximation from BatteryLevel.
    pub level: Option<u8>,
    pub status: Option<BatteryStatus>,
    /// Voltage in mV, when available (HID++ 2.0 battery voltage feature).
    pub voltage: Option<u16>,
}

/// Firmware component type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareKind {
    Firmware,
    Bootloader,
    Other,
}

/// One firmware component on a device.
#[derive(Debug, Clone)]
pub struct FirmwareInfo {
    pub kind: FirmwareKind,
    pub version: String,
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
    pub firmware: Vec<FirmwareInfo>,
}
