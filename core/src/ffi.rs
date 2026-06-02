// C-compatible FFI exports for platform frontends (Swift, C#, Python).
//
// All three platform frontends call into this layer via their native FFI
// mechanisms (Swift bridging header, C# P/Invoke, Python ctypes).
//
// Rules:
//   - All types crossing this boundary must be #[repr(C)].
//   - Strings cross as null-terminated *const c_char (caller does not own them;
//     use the provided free functions to release memory allocated here).
//   - Arrays cross as pointer + length.
//   - Errors are communicated via a PulsaarStatus return code.
//   - No Rust panics must reach the FFI boundary -- use catch_unwind where needed.
//
// This layer is intentionally minimal for now. Expand as the core is fleshed out.


#[repr(C)]
pub enum PulsaarStatus {
    Ok          = 0,
    HidError    = 1,
    Timeout     = 2,
    NoReceiver  = 3,
    EmptySlot   = 4,
    InvalidArg  = 5,
    Unknown     = 99,
}

impl From<crate::error::Error> for PulsaarStatus {
    fn from(e: crate::error::Error) -> Self {
        use crate::error::Error;
        match e {
            Error::Hid(_)              => Self::HidError,
            Error::Timeout             => Self::Timeout,
            Error::NoReceiver          => Self::NoReceiver,
            Error::EmptySlot           => Self::EmptySlot,
            _                          => Self::Unknown,
        }
    }
}

// TODO: implement the following exports as the core stabilises:
//
//   pulsaar_init()                    -> PulsaarStatus
//   pulsaar_enumerate_receivers(...)  -> receiver count
//   pulsaar_open_receiver(index, ...) -> PulsaarStatus
//   pulsaar_enumerate_devices(...)    -> device count
//   pulsaar_get_device_name(...)      -> *const c_char
//   pulsaar_get_battery(...)          -> level, status
//   pulsaar_close_receiver(...)
//   pulsaar_free_string(ptr)
