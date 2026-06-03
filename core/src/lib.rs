pub mod error;
pub mod transport;
pub mod hidpp;
pub mod devices;
pub mod receiver;
pub mod direct;
pub mod ffi;

pub use error::{Error, Result};
pub use devices::types::{Battery, BatteryStatus, DeviceInfo, DeviceKind};
pub use receiver::{ReceiverHandle, ReceiverKind, Receiver, enumerate_receivers};
pub use direct::DirectDeviceInfo;

/// Initialize the library and return a HID API instance.
///
/// Must be called before any other pulsaar_core functions. On macOS, this sets
/// the HID exclusive-access flag before hidapi initializes -- without it, device
/// opens will fail because macOS claims exclusive access by default.
pub fn init() -> Result<hidapi::HidApi> {
    #[cfg(target_os = "macos")]
    unsafe {
        extern "C" {
            fn hid_darwin_set_open_exclusive(exclusive: std::os::raw::c_int);
        }
        hid_darwin_set_open_exclusive(0);
    }
    Ok(hidapi::HidApi::new()?)
}
