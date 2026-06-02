// C-compatible FFI exports for platform frontends (Swift, C#, Python).
//
// All three platform frontends call into this layer via their native FFI
// mechanisms (Swift bridging header, C# P/Invoke, Python ctypes).
//
// Rules:
//   - All types crossing this boundary must be #[repr(C)].
//   - Strings are embedded as null-terminated byte arrays inside structs.
//     The caller copies them out; there are no heap-allocated strings to free.
//   - Arrays cross via the count-then-index pattern (enumerate, then query by index).
//   - Errors are communicated via PulsaarStatus return codes or null pointer returns.
//   - No Rust panics must reach the FFI boundary.
//
// Typical call sequence:
//   1.  pulsaar_init()                              -> *mut PulsaarContext (null on failure)
//   2.  pulsaar_get_receiver_count(ctx)             -> usize
//   3.  pulsaar_get_receiver_info(ctx, i, &out)     -> PulsaarStatus
//   4.  pulsaar_open_receiver(ctx, i, &status)      -> *mut PulsaarReceiverContext (null on failure)
//   5.  pulsaar_get_opened_receiver_info(rctx, &out)-> PulsaarStatus
//   6.  pulsaar_enumerate_devices(rctx)             -> PulsaarStatus
//   7.  pulsaar_get_device_count(rctx)              -> usize
//   8.  pulsaar_get_device_info(rctx, i, &out)      -> PulsaarStatus
//   9.  pulsaar_close_receiver(rctx)
//   10. pulsaar_destroy(ctx)

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::devices::types::{Battery, BatteryStatus, DeviceInfo, DeviceKind};
use crate::receiver::{ReceiverHandle, ReceiverKind, Receiver, enumerate_receivers};

// ---------------------------------------------------------------------------
// Status codes (keep in sync with the From<Error> impl below)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// C-compatible structs
// ---------------------------------------------------------------------------

/// Info about a receiver as enumerated from the HID device list (pre-open).
/// Includes the OS device path so the frontend can display or log it.
#[repr(C)]
pub struct CReceiverInfo {
    pub product_id: u16,
    /// 0=Unifying, 1=Bolt, 2=Nano, 3=LightSpeed
    pub kind:       u8,
    /// Null-terminated display name, e.g. "Bolt Receiver".
    pub name:       [u8; 64],
    /// Null-terminated OS HID path (opaque; useful for debugging).
    pub path:       [u8; 256],
}

/// Info about a receiver after it has been successfully opened.
#[repr(C)]
pub struct COpenedReceiverInfo {
    pub product_id:  u16,
    /// 0=Unifying, 1=Bolt, 2=Nano, 3=LightSpeed
    pub kind:        u8,
    pub max_devices: u8,
    /// Null-terminated display name.
    pub name:        [u8; 64],
    /// Null-terminated serial number (hex string). 33 bytes to accommodate the
    /// Bolt receiver's 32-char unique ID (16 raw bytes hex-encoded) plus null.
    pub serial:      [u8; 33],
}

/// Battery state for a device.
///
/// level:   0-100 if reported, 0xFF if not available.
/// status:  0=Discharging, 1=Recharging, 2=AlmostFull, 3=Full, 4=SlowRecharge,
///          5=InvalidBattery, 6=ThermalError, 0xFF if not available.
/// voltage: millivolts if reported, 0 if not available.
#[repr(C)]
pub struct CBattery {
    pub level:   u8,
    pub status:  u8,
    pub voltage: u16,
}

/// Info about a device paired to a receiver.
///
/// kind:        0=Unknown, 1=Keyboard, 2=Mouse, 3=Numpad, 4=Presenter, 5=Remote,
///              6=Trackball, 7=Touchpad, 8=Tablet, 9=Gamepad, 10=Joystick,
///              11=Headset, 12=RemoteControl, 13=Receiver
/// has_battery: 0 if no battery info, 1 if the battery field is populated.
#[repr(C)]
pub struct CDeviceInfo {
    pub slot:        u8,
    pub kind:        u8,
    pub wpid:        [u8; 2],
    /// Null-terminated device name.
    pub name:        [u8; 64],
    /// Null-terminated serial (hex string).
    pub serial:      [u8; 32],
    pub has_battery: u8,
    pub battery:     CBattery,
}

// ---------------------------------------------------------------------------
// Opaque context types (heap-allocated; exposed as raw pointers to callers)
// ---------------------------------------------------------------------------

/// Owns the HID API instance and the receiver handle list.
/// Lives for the lifetime of the session.
pub struct PulsaarContext {
    api:       hidapi::HidApi,
    receivers: Vec<ReceiverHandle>,
}

/// Owns one opened receiver and the last-enumerated device list.
/// Lives between pulsaar_open_receiver / pulsaar_close_receiver.
pub struct PulsaarReceiverContext {
    receiver: Receiver,
    devices:  Vec<DeviceInfo>,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn str_to_buf<const N: usize>(s: &str) -> [u8; N] {
    let mut buf = [0u8; N];
    let bytes = s.as_bytes();
    let len = bytes.len().min(N.saturating_sub(1));
    buf[..len].copy_from_slice(&bytes[..len]);
    buf
}

fn receiver_kind_to_u8(k: ReceiverKind) -> u8 {
    match k {
        ReceiverKind::Unifying  => 0,
        ReceiverKind::Bolt      => 1,
        ReceiverKind::Nano      => 2,
        ReceiverKind::LightSpeed => 3,
    }
}

fn device_kind_to_u8(k: DeviceKind) -> u8 {
    match k {
        DeviceKind::Unknown       => 0,
        DeviceKind::Keyboard      => 1,
        DeviceKind::Mouse         => 2,
        DeviceKind::Numpad        => 3,
        DeviceKind::Presenter     => 4,
        DeviceKind::Remote        => 5,
        DeviceKind::Trackball     => 6,
        DeviceKind::Touchpad      => 7,
        DeviceKind::Tablet        => 8,
        DeviceKind::Gamepad       => 9,
        DeviceKind::Joystick      => 10,
        DeviceKind::Headset       => 11,
        DeviceKind::RemoteControl => 12,
        DeviceKind::Receiver      => 13,
    }
}

fn battery_status_to_u8(s: BatteryStatus) -> u8 {
    match s {
        BatteryStatus::Discharging    => 0,
        BatteryStatus::Recharging     => 1,
        BatteryStatus::AlmostFull     => 2,
        BatteryStatus::Full           => 3,
        BatteryStatus::SlowRecharge   => 4,
        BatteryStatus::InvalidBattery => 5,
        BatteryStatus::ThermalError   => 6,
    }
}

fn battery_to_c(b: Option<&Battery>) -> (u8, CBattery) {
    match b {
        None => (0, CBattery { level: 0xFF, status: 0xFF, voltage: 0 }),
        Some(bat) => (
            1,
            CBattery {
                level:   bat.level.unwrap_or(0xFF),
                status:  bat.status.map(battery_status_to_u8).unwrap_or(0xFF),
                voltage: bat.voltage.unwrap_or(0),
            },
        ),
    }
}

fn receiver_handle_to_c(h: &ReceiverHandle) -> CReceiverInfo {
    CReceiverInfo {
        product_id: h.product_id,
        kind:       receiver_kind_to_u8(h.kind),
        name:       str_to_buf(h.name),
        path:       str_to_buf(&h.path),
    }
}

fn opened_receiver_to_c(r: &Receiver) -> COpenedReceiverInfo {
    COpenedReceiverInfo {
        product_id:  r.product_id,
        kind:        receiver_kind_to_u8(r.kind),
        max_devices: r.max_devices,
        name:        str_to_buf(r.name),
        serial:      str_to_buf(&r.serial),
    }
}

fn device_info_to_c(d: &DeviceInfo) -> CDeviceInfo {
    let (has_battery, battery) = battery_to_c(d.battery.as_ref());
    CDeviceInfo {
        slot:        d.slot,
        kind:        device_kind_to_u8(d.kind),
        wpid:        d.wpid,
        name:        str_to_buf(&d.name),
        serial:      str_to_buf(&d.serial),
        has_battery,
        battery,
    }
}

// ---------------------------------------------------------------------------
// Exported functions
// ---------------------------------------------------------------------------

/// Initialize HID and enumerate receivers.
///
/// Returns a pointer to the session context, or null on failure.
/// The caller must eventually call pulsaar_destroy.
#[no_mangle]
pub extern "C" fn pulsaar_init() -> *mut PulsaarContext {
    let result = catch_unwind(|| {
        crate::init().map(|api| {
            let receivers = enumerate_receivers(&api);
            PulsaarContext { api, receivers }
        })
    });
    match result {
        Ok(Ok(ctx)) => Box::into_raw(Box::new(ctx)),
        _            => std::ptr::null_mut(),
    }
}

/// Free the session context. Safe to call with null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_destroy(ctx: *mut PulsaarContext) {
    if !ctx.is_null() {
        drop(Box::from_raw(ctx));
    }
}

/// Number of receivers found at init time. Returns 0 if ctx is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_receiver_count(ctx: *const PulsaarContext) -> usize {
    if ctx.is_null() { return 0; }
    (*ctx).receivers.len()
}

/// Fill `out` with info for the receiver at `index` (0-based).
///
/// Returns InvalidArg if ctx or out is null, or index is out of range.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_receiver_info(
    ctx:   *const PulsaarContext,
    index: usize,
    out:   *mut CReceiverInfo,
) -> PulsaarStatus {
    if ctx.is_null() || out.is_null() { return PulsaarStatus::InvalidArg; }
    let handle = match (&(*ctx).receivers).get(index) {
        Some(h) => h,
        None    => return PulsaarStatus::InvalidArg,
    };
    *out = receiver_handle_to_c(handle);
    PulsaarStatus::Ok
}

/// Open the receiver at `index` in the context's receiver list.
///
/// Returns a pointer to the opened receiver context, or null on failure.
/// On failure, `status_out` (if non-null) receives the error code.
/// The caller must eventually call pulsaar_close_receiver.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_open_receiver(
    ctx:        *mut PulsaarContext,
    index:      usize,
    status_out: *mut PulsaarStatus,
) -> *mut PulsaarReceiverContext {
    macro_rules! fail {
        ($s:expr) => {{
            if !status_out.is_null() { *status_out = $s; }
            return std::ptr::null_mut();
        }};
    }

    if ctx.is_null() { fail!(PulsaarStatus::InvalidArg); }

    let handle = match (&(*ctx).receivers).get(index) {
        Some(h) => h,
        None    => fail!(PulsaarStatus::InvalidArg),
    };

    match Receiver::open(&(*ctx).api, handle) {
        Ok(receiver) => {
            if !status_out.is_null() { *status_out = PulsaarStatus::Ok; }
            Box::into_raw(Box::new(PulsaarReceiverContext {
                receiver,
                devices: Vec::new(),
            }))
        }
        Err(e) => fail!(PulsaarStatus::from(e)),
    }
}

/// Close an opened receiver and free its context. Safe to call with null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_close_receiver(rctx: *mut PulsaarReceiverContext) {
    if !rctx.is_null() {
        drop(Box::from_raw(rctx));
    }
}

/// Fill `out` with properties of the opened receiver (serial, max_devices, etc.).
///
/// Returns InvalidArg if rctx or out is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_opened_receiver_info(
    rctx: *const PulsaarReceiverContext,
    out:  *mut COpenedReceiverInfo,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() { return PulsaarStatus::InvalidArg; }
    *out = opened_receiver_to_c(&(*rctx).receiver);
    PulsaarStatus::Ok
}

/// Enumerate devices paired to the receiver. Replaces any previously cached list.
///
/// Must be called before pulsaar_get_device_count / pulsaar_get_device_info.
/// Returns InvalidArg if rctx is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_enumerate_devices(
    rctx: *mut PulsaarReceiverContext,
) -> PulsaarStatus {
    if rctx.is_null() { return PulsaarStatus::InvalidArg; }
    let rctx = &mut *rctx;
    let result = catch_unwind(AssertUnwindSafe(|| rctx.receiver.enumerate_devices()));
    match result {
        Ok(Ok(devices)) => { rctx.devices = devices; PulsaarStatus::Ok }
        Ok(Err(e))      => PulsaarStatus::from(e),
        Err(_)          => PulsaarStatus::Unknown,
    }
}

/// Number of devices in the last pulsaar_enumerate_devices result.
/// Returns 0 if rctx is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_device_count(rctx: *const PulsaarReceiverContext) -> usize {
    if rctx.is_null() { return 0; }
    (*rctx).devices.len()
}

/// Fill `out` with info for the device at `index` in the cached device list.
///
/// Returns InvalidArg if rctx or out is null, or index is out of range.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_device_info(
    rctx:  *const PulsaarReceiverContext,
    index: usize,
    out:   *mut CDeviceInfo,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() { return PulsaarStatus::InvalidArg; }
    let device = match (&(*rctx).devices).get(index) {
        Some(d) => d,
        None    => return PulsaarStatus::InvalidArg,
    };
    *out = device_info_to_c(device);
    PulsaarStatus::Ok
}
