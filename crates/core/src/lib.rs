// ponytail: reply parsing indexes into `params()` without length checks (60
// sites). A short or malformed reply panics the HID worker thread; the release
// profile unwinds so the UI survives it. Replace with `get()` and drop this
// allow when the parser is hardened.
#![allow(clippy::indexing_slicing)]

pub mod devices;
pub mod direct;
pub mod error;
pub mod hidpp;
pub mod receiver;
pub mod transport;

pub use devices::types::{Battery, BatteryStatus, DeviceInfo, DeviceKind};
pub use direct::DirectDeviceInfo;
pub use error::{Error, Result};
pub use hidapi::HidApi;
pub use receiver::{enumerate_receivers, Receiver, ReceiverHandle, ReceiverKind};

use std::sync::OnceLock;
use std::time::Instant;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Milliseconds since the first call (approximately process start). Used for log timestamps.
pub fn t_ms() -> u64 {
    PROCESS_START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// Initialize the library and return a HID API instance.
///
/// Must be called before any other pulsaar_core functions. On macOS, this sets
/// the HID exclusive-access flag before hidapi initializes -- without it, device
/// opens will fail because macOS claims exclusive access by default.
#[allow(unsafe_code)]
pub fn init() -> Result<hidapi::HidApi> {
    // SAFETY: a plain setter on a global flag in the bundled hidapi C library,
    // called before any device is opened.
    #[cfg(target_os = "macos")]
    unsafe {
        extern "C" {
            fn hid_darwin_set_open_exclusive(exclusive: std::os::raw::c_int);
        }
        hid_darwin_set_open_exclusive(0);
    }
    Ok(hidapi::HidApi::new()?)
}
