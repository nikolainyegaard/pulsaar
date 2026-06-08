use std::ffi::CString;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};

use crate::error::{Error, Hidpp10Error, Hidpp20Error, Result};
use crate::hidpp::message::{Message, MAX_READ_SIZE, RECEIVER_DEVICE};

// Receiver responds quickly (short USB round-trip). 300ms is generous for USB.
const RECEIVER_TIMEOUT: Duration = Duration::from_millis(300);

// Wireless devices can be slow to respond.
const DEVICE_TIMEOUT: Duration = Duration::from_millis(4000);

pub struct Transport {
    write_dev: HidDevice,
    // None means use write_dev for reads (single-path devices: macOS, Linux, Bluetooth).
    // Some means a separate handle for the input collection (Windows HID++ receivers).
    read_dev: Option<HidDevice>,
}

fn open_hid(api: &HidApi, path: &str) -> Result<HidDevice> {
    let c_path = CString::new(path).map_err(|_| Error::InvalidResponse)?;
    Ok(api.open_path(c_path.as_c_str())?)
}

impl Transport {
    /// Open a transport. On Windows HID++ receivers the write and read paths differ
    /// (usage=0x0001 for commands, usage=0x0002 for replies). Pass the same string
    /// for both on platforms/devices that use a single bidirectional path.
    pub fn open(api: &HidApi, write_path: &str, read_path: &str) -> Result<Self> {
        let write_dev = open_hid(api, write_path)?;
        let read_dev = if read_path != write_path {
            Some(open_hid(api, read_path)?)
        } else {
            None
        };
        Ok(Self { write_dev, read_dev })
    }

    fn read_dev(&self) -> &HidDevice {
        self.read_dev.as_ref().unwrap_or(&self.write_dev)
    }

    fn read_one(&self, timeout_ms: i32) -> Result<Option<Message>> {
        let mut buf = [0u8; MAX_READ_SIZE];
        let n = self.read_dev().read_timeout(&mut buf, timeout_ms)?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Message::from_bytes(&buf[..n]))
    }

    pub fn write(&self, msg: &Message) -> Result<()> {
        self.write_dev.write(msg.as_bytes())?;
        Ok(())
    }

    /// Read one unsolicited notification with a timeout. Returns None on timeout.
    /// Used during pairing to receive device discovery and connection events from
    /// the receiver without an associated request.
    pub fn read_notification(&self, timeout_ms: i32) -> Result<Option<Message>> {
        self.read_one(timeout_ms)
    }

    /// Send a request and wait for a matching reply.
    ///
    /// HID++ 1.0 error responses have sub_id 0x8F, then [req_sub_id, req_address, error_code].
    /// HID++ 2.0 error responses have sub_id 0xFF, then [req_sub_id, req_address, error_code].
    /// Normal replies match on (device, sub_id, address).
    ///
    /// Receiver register reads 0x83B5 (RECEIVER_INFO) and 0x81F1 (FIRMWARE) additionally
    /// require the first reply param to match the first request param (the sub-register).
    pub fn request(&self, req: &Message) -> Result<Message> {
        self.request_timeout(req, None)
    }

    /// Like request() but with an explicit timeout override.
    /// When Some, the override applies unconditionally to both receiver and device
    /// requests (useful for short-timeout probes where the caller knows the device
    /// responds quickly). Pass None to use the standard RECEIVER_TIMEOUT / DEVICE_TIMEOUT.
    pub fn request_timeout(&self, req: &Message, timeout_override: Option<Duration>) -> Result<Message> {
        // Determine timeout; long register reads (sub_id 0x83) get extra time.
        let base = match timeout_override {
            Some(t) => t,
            None => if req.device() == RECEIVER_DEVICE { RECEIVER_TIMEOUT } else { DEVICE_TIMEOUT },
        };
        // Only apply the 2x multiplier for long register reads (0x83) sent to wireless devices,
        // not to the receiver itself (which is USB and always responds quickly).
        let timeout = if req.sub_id() == 0x83 && req.device() != RECEIVER_DEVICE { base * 2 } else { base };

        // For receiver registers 0x83B5 (RECEIVER_INFO) and 0x81F1 (FIRMWARE) we must also
        // match the first reply param against the first request param (the sub-register byte).
        let needs_sub_reg_match = req.device() == RECEIVER_DEVICE
            && ((req.sub_id() == 0x83 && req.address() == 0xB5)
                || (req.sub_id() == 0x81 && req.address() == 0xF1));
        let expected_sub_reg = if needs_sub_reg_match {
            req.params().first().copied()
        } else {
            None
        };

        self.write(req)?;

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }

            let msg = match self.read_one(remaining.as_millis() as i32)? {
                Some(m) => m,
                None => continue,
            };

            // Skip messages for other devices.
            // Bluetooth devices sometimes return 0x00 instead of 0xFF for the receiver.
            let rep_dev = msg.device();
            let req_dev = req.device();
            if rep_dev != req_dev && !(req_dev == RECEIVER_DEVICE && rep_dev == 0x00) {
                continue;
            }

            // HID++ 1.0 error: sub_id=0x8F, address=failed_sub_id, params=[failed_address, ..., error_code].
            if msg.is_hidpp10_error() {
                let p = msg.params();
                if msg.address() == req.sub_id() && p.first().copied() == Some(req.address()) {
                    // error_code is at params[1] (after the failed address byte).
                    return Err(Error::Hidpp10(Hidpp10Error::from_byte(p.get(1).copied().unwrap_or(0))));
                }
                continue;
            }

            // HID++ 2.0 error: sub_id=0xFF, address=failed_feature_index, params=[failed_fn_byte, error_code, ...].
            if msg.is_hidpp20_error() {
                let p = msg.params();
                if msg.address() == req.sub_id() && p.first().copied() == Some(req.address()) {
                    return Err(Error::Hidpp20(Hidpp20Error::from_byte(p.get(1).copied().unwrap_or(0))));
                }
                continue;
            }

            // Normal reply: sub_id and address must match the request.
            if msg.sub_id() != req.sub_id() || msg.address() != req.address() {
                continue;
            }

            // For RECEIVER_INFO and FIRMWARE register reads, also match the sub-register.
            if let Some(expected) = expected_sub_reg {
                if msg.params().first().copied() != Some(expected) {
                    continue;
                }
            }

            return Ok(msg);
        }
    }
}
