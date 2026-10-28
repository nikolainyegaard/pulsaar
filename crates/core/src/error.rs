/// HID++ 1.0 error codes, as returned in error response sub-ID 0x8F.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hidpp10Error {
    /// Also used as a ping response from HID++ 1.0 devices (not a real error).
    InvalidSubId,
    InvalidAddress,
    InvalidValue,
    ConnectionFailed,
    TooManyDevices,
    AlreadyExists,
    Busy,
    /// Slot is empty or device not paired.
    UnknownDevice,
    ResourceError,
    RequestUnavailable,
    UnsupportedParam,
    WrongPinCode,
    Other(u8),
}

impl Hidpp10Error {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x01 => Self::InvalidSubId,
            0x02 => Self::InvalidAddress,
            0x03 => Self::InvalidValue,
            0x04 => Self::ConnectionFailed,
            0x05 => Self::TooManyDevices,
            0x06 => Self::AlreadyExists,
            0x07 => Self::Busy,
            0x08 => Self::UnknownDevice,
            0x09 => Self::ResourceError,
            0x0A => Self::RequestUnavailable,
            0x0B => Self::UnsupportedParam,
            0x0C => Self::WrongPinCode,
            other => Self::Other(other),
        }
    }
}

/// HID++ 2.0 feature call error codes, as returned in error response sub-ID 0xFF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hidpp20Error {
    Unknown,
    InvalidArgument,
    OutOfRange,
    HardwareError,
    LogitechError,
    InvalidFeature,
    InvalidFunction,
    Busy,
    Unsupported,
    Other(u8),
}

impl Hidpp20Error {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x01 => Self::Unknown,
            0x02 => Self::InvalidArgument,
            0x03 => Self::OutOfRange,
            0x04 => Self::HardwareError,
            0x05 => Self::LogitechError,
            0x06 => Self::InvalidFeature,
            0x07 => Self::InvalidFunction,
            0x08 => Self::Busy,
            0x09 => Self::Unsupported,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Hid(String),
    Hidpp10(Hidpp10Error),
    Hidpp20(Hidpp20Error),
    Timeout,
    InvalidResponse,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Hid(s) => write!(f, "HID error: {}", s),
            Error::Hidpp10(e) => write!(f, "HID++ 1.0 error: {:?}", e),
            Error::Hidpp20(e) => write!(f, "HID++ 2.0 error: {:?}", e),
            Error::Timeout => write!(f, "request timed out"),
            Error::InvalidResponse => write!(f, "invalid response"),
        }
    }
}

impl std::error::Error for Error {}

impl From<hidapi::HidError> for Error {
    fn from(e: hidapi::HidError) -> Self {
        Error::Hid(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
