// HID report IDs and sizes per the HID++ protocol.
pub const SHORT_REPORT_ID: u8 = 0x10;
pub const LONG_REPORT_ID: u8 = 0x11;
pub const SHORT_MESSAGE_SIZE: usize = 7;
pub const LONG_MESSAGE_SIZE: usize = 20;
pub const MAX_READ_SIZE: usize = 32;

/// The HID++ device number used when addressing the receiver itself.
pub const RECEIVER_DEVICE: u8 = 0xFF;

/// Software ID embedded in the low 4 bits of the HID++ 2.0 function/address byte.
/// Pulsaar uses 0x0A (Solaar uses 0x0B, so we don't collide if both run simultaneously).
pub const SOFTWARE_ID: u8 = 0x0A;

// Sub-IDs for HID++ 1.0 register operations. The high nibble encodes read/write
// and whether the register is short (0x00-0xFF) or long (0x200-0x2FF).
pub const HIDPP10_SHORT_WRITE: u8 = 0x80;
pub const HIDPP10_SHORT_READ: u8 = 0x81;
pub const HIDPP10_LONG_WRITE: u8 = 0x82;
pub const HIDPP10_LONG_READ: u8 = 0x83;

/// Sub-ID that marks a HID++ 1.0 error response.
pub const HIDPP10_ERROR: u8 = 0x8F;

/// Sub-ID that marks a HID++ 2.0 error response.
pub const HIDPP20_ERROR: u8 = 0xFF;

/// A HID++ short (7-byte) or long (20-byte) message.
///
/// Raw layout:
///   buf[0] = report_id  (0x10 short, 0x11 long)
///   buf[1] = device     (0xFF for receiver, 1-6 for paired devices)
///   buf[2] = sub_id     (register operation or feature index)
///   buf[3] = address    (register address or function | software_id)
///   buf[4..] = params   (up to 3 bytes short, up to 16 bytes long)
#[derive(Debug, Clone)]
pub struct Message {
    buf: [u8; LONG_MESSAGE_SIZE],
    len: usize,
}

impl Message {
    /// Construct a short (7-byte) message.
    pub fn short(device: u8, sub_id: u8, address: u8, p0: u8, p1: u8, p2: u8) -> Self {
        let mut buf = [0u8; LONG_MESSAGE_SIZE];
        buf[0] = SHORT_REPORT_ID;
        buf[1] = device;
        buf[2] = sub_id;
        buf[3] = address;
        buf[4] = p0;
        buf[5] = p1;
        buf[6] = p2;
        Self { buf, len: SHORT_MESSAGE_SIZE }
    }

    /// Construct a long (20-byte) message.
    pub fn long(device: u8, sub_id: u8, address: u8, params: &[u8]) -> Self {
        let mut buf = [0u8; LONG_MESSAGE_SIZE];
        buf[0] = LONG_REPORT_ID;
        buf[1] = device;
        buf[2] = sub_id;
        buf[3] = address;
        let n = params.len().min(16);
        buf[4..4 + n].copy_from_slice(&params[..n]);
        Self { buf, len: LONG_MESSAGE_SIZE }
    }

    /// Parse a raw HID read buffer into a Message. Returns None if the
    /// buffer is too short or starts with an unrecognised report ID.
    pub fn from_bytes(raw: &[u8]) -> Option<Self> {
        let len = match raw.first()? {
            &SHORT_REPORT_ID if raw.len() >= SHORT_MESSAGE_SIZE => SHORT_MESSAGE_SIZE,
            &LONG_REPORT_ID if raw.len() >= LONG_MESSAGE_SIZE => LONG_MESSAGE_SIZE,
            _ => return None,
        };
        let mut buf = [0u8; LONG_MESSAGE_SIZE];
        buf[..len].copy_from_slice(&raw[..len]);
        Some(Self { buf, len })
    }

    pub fn as_bytes(&self) -> &[u8] { &self.buf[..self.len] }

    pub fn report_id(&self) -> u8 { self.buf[0] }
    pub fn device(&self) -> u8    { self.buf[1] }
    pub fn sub_id(&self) -> u8    { self.buf[2] }
    pub fn address(&self) -> u8   { self.buf[3] }

    /// The parameter bytes after the header (buf[4..len]).
    pub fn params(&self) -> &[u8] { &self.buf[4..self.len] }

    /// True if this is a HID++ 1.0 error response (sub_id 0x8F).
    pub fn is_hidpp10_error(&self) -> bool { self.buf[2] == HIDPP10_ERROR }

    /// True if this is a HID++ 2.0 error response (sub_id 0xFF).
    pub fn is_hidpp20_error(&self) -> bool { self.buf[2] == HIDPP20_ERROR }
}
