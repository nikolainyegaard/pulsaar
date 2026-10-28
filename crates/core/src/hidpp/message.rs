// HID report IDs and sizes per the HID++ protocol.
pub const SHORT_REPORT_ID: u8 = 0x10;
pub const LONG_REPORT_ID: u8 = 0x11;
pub const SHORT_MESSAGE_SIZE: usize = 7;
pub const LONG_MESSAGE_SIZE: usize = 20;
/// DJ report: what a receiver sends in DJ mode, which the Linux kernel driver
/// hid-logitech-dj switches it to. Connection changes arrive this way there.
pub const DJ_REPORT_ID: u8 = 0x20;
pub const DJ_MESSAGE_SIZE: usize = 15;
pub const MAX_READ_SIZE: usize = 32;

/// The HID++ device number used when addressing the receiver itself.
pub const RECEIVER_DEVICE: u8 = 0xFF;

/// Software ID embedded in the low 4 bits of the HID++ 2.0 function/address byte.
/// Pulsaar uses 0x0A (Solaar uses 0x0B, so we don't collide if both run simultaneously).
pub const SOFTWARE_ID: u8 = 0x0A;

/// Sub-ID that marks a HID++ 1.0 error response.
pub const HIDPP10_ERROR: u8 = 0x8F;

/// Sub-ID that marks a HID++ 2.0 error response.
pub const HIDPP20_ERROR: u8 = 0xFF;

/// Reading reply bytes without panicking. Devices send short or malformed
/// replies now and then; a missing byte reads as 0 and a missing range as
/// the part that is there, like the absent fields the parsers already
/// default to 0.
pub trait ReplyBytes {
    /// The byte at `i`, or 0 past the end.
    fn at(&self, i: usize) -> u8;
    /// `self[start..end]` clamped to the length; empty when `start` is past it.
    fn span(&self, start: usize, end: usize) -> &[u8];
}

impl ReplyBytes for [u8] {
    fn at(&self, i: usize) -> u8 {
        self.get(i).copied().unwrap_or(0)
    }

    fn span(&self, start: usize, end: usize) -> &[u8] {
        self.get(start..end.min(self.len())).unwrap_or_default()
    }
}

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
        Self {
            buf,
            len: SHORT_MESSAGE_SIZE,
        }
    }

    /// Construct a long (20-byte) message.
    pub fn long(device: u8, sub_id: u8, address: u8, params: &[u8]) -> Self {
        let mut buf = [0u8; LONG_MESSAGE_SIZE];
        buf[0] = LONG_REPORT_ID;
        buf[1] = device;
        buf[2] = sub_id;
        buf[3] = address;
        for (dst, src) in buf.iter_mut().skip(4).zip(params) {
            *dst = *src;
        }
        Self {
            buf,
            len: LONG_MESSAGE_SIZE,
        }
    }

    /// Parse a raw HID read buffer into a Message. Returns None if the
    /// buffer is too short or starts with an unrecognised report ID.
    pub fn from_bytes(raw: &[u8]) -> Option<Self> {
        let len = match *raw.first()? {
            SHORT_REPORT_ID if raw.len() >= SHORT_MESSAGE_SIZE => SHORT_MESSAGE_SIZE,
            LONG_REPORT_ID if raw.len() >= LONG_MESSAGE_SIZE => LONG_MESSAGE_SIZE,
            DJ_REPORT_ID if raw.len() >= DJ_MESSAGE_SIZE => DJ_MESSAGE_SIZE,
            _ => return None,
        };
        let mut buf = [0u8; LONG_MESSAGE_SIZE];
        for (dst, src) in buf.iter_mut().zip(raw.iter().take(len)) {
            *dst = *src;
        }
        Some(Self { buf, len })
    }

    /// A DJ report rather than HID++. Its sub_id is the DJ report type and
    /// never answers a request.
    pub fn is_dj(&self) -> bool {
        self.buf[0] == DJ_REPORT_ID
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf.span(0, self.len)
    }

    pub fn device(&self) -> u8 {
        self.buf[1]
    }
    pub fn sub_id(&self) -> u8 {
        self.buf[2]
    }
    pub fn address(&self) -> u8 {
        self.buf[3]
    }

    /// The parameter bytes after the header (buf[4..len]).
    pub fn params(&self) -> &[u8] {
        self.buf.span(4, self.len)
    }

    /// True if this is a HID++ 1.0 error response (sub_id 0x8F).
    pub fn is_hidpp10_error(&self) -> bool {
        self.buf[2] == HIDPP10_ERROR
    }

    /// True if this is a HID++ 2.0 error response (sub_id 0xFF).
    pub fn is_hidpp20_error(&self) -> bool {
        self.buf[2] == HIDPP20_ERROR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_bytes_takes_short_long_and_dj_reports_and_rejects_the_rest() {
        let short = Message::from_bytes(&[0x10, 0x01, 0x41, 0x10, 0x41, 0x6E, 0xB3]);
        assert_eq!(
            short.map(|m| m.params().to_vec()),
            Some(vec![0x41, 0x6E, 0xB3])
        );

        let long = Message::from_bytes(&[0x11, 0xFF, 0x83, 0xB5, 0x51, 0x41]);
        assert!(long.is_none(), "a long report needs 20 bytes");

        let mut raw = [0u8; 20];
        raw[0] = LONG_REPORT_ID;
        raw[4] = 0x37;
        let long = Message::from_bytes(&raw);
        assert_eq!(long.as_ref().map(|m| m.params().len()), Some(16));
        assert_eq!(long.map(|m| m.params().at(0)), Some(0x37));

        let dj = Message::from_bytes(&[0x20, 0x02, 0x42, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(dj.as_ref().is_some_and(Message::is_dj));

        assert!(Message::from_bytes(&[]).is_none());
        assert!(Message::from_bytes(&[0x02, 0, 0, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn reply_bytes_read_past_the_end_as_zero_and_empty() {
        let p: &[u8] = &[1, 2, 3];
        assert_eq!(p.at(2), 3);
        assert_eq!(p.at(3), 0);
        assert_eq!(p.span(1, 10), &[2, 3]);
        assert!(p.span(5, 10).is_empty());
        assert!(p.span(2, 1).is_empty());
    }

    #[test]
    fn long_truncates_params_to_sixteen_bytes() {
        let m = Message::long(1, 2, 3, &[9; 20]);
        assert_eq!(m.as_bytes().len(), LONG_MESSAGE_SIZE);
        assert_eq!(m.params(), &[9; 16]);
    }
}
