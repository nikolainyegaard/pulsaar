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
use crate::direct::{DirectDeviceInfo, enumerate_direct_devices};
use crate::receiver::{ReceiverHandle, ReceiverKind, Receiver, PairingSession, PairingEvent, enumerate_receivers};

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

/// Info about a directly-connected (Bluetooth) Logitech device.
///
/// kind:        same encoding as CDeviceInfo.kind
/// has_battery: 0 if no battery info, 1 if the battery field is populated.
#[repr(C)]
pub struct CDirectDeviceInfo {
    pub product_id:  u16,
    pub kind:        u8,
    /// Null-terminated device name.
    pub name:        [u8; 64],
    /// Null-terminated serial (from HID descriptor; may be empty).
    pub serial:      [u8; 64],
    pub has_battery: u8,
    pub battery:     CBattery,
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
// Pairing types
// ---------------------------------------------------------------------------

/// State of an in-progress pairing operation.
#[repr(C)]
pub enum PulsaarPairingState {
    /// Lock open, waiting for a device to initiate pairing.
    Waiting        = 0,
    /// Bolt: a compatible device was found and pairing has been initiated.
    DeviceFound    = 1,
    /// Bolt: the device requires the user to type a numeric passkey on the keyboard, then press Enter.
    PasskeyNumeric = 2,
    /// Bolt: the device requires the user to press buttons in the sequence encoded in passkey.
    PasskeyButton  = 3,
    /// Pairing completed successfully.
    Paired         = 4,
    /// Pairing failed; see error field for a description.
    Failed         = 5,
    /// No pairing is currently in progress.
    Idle           = 6,
}

/// Result of one pulsaar_poll_pairing call.
#[repr(C)]
pub struct CPairingStatus {
    pub state:       PulsaarPairingState,
    /// Name of the newly found or paired device (valid for DeviceFound, Paired).
    pub device_name: [u8; 64],
    /// Passkey string (valid for PasskeyNumeric, PasskeyButton).
    /// PasskeyNumeric: ASCII digit string to type on the keyboard.
    /// PasskeyButton:  10-character L/R sequence; press corresponding buttons, then both simultaneously.
    pub passkey:     [u8; 16],
    /// Human-readable error description (valid for Failed).
    pub error:       [u8; 64],
}

// ---------------------------------------------------------------------------
// Device connection-event types
// ---------------------------------------------------------------------------

/// Kind of device connection event returned by pulsaar_poll_device_event.
#[repr(C)]
pub enum PulsaarConnectionEvent {
    /// No event received within the timeout.
    None    = 0,
    /// A paired device in slot X came online.
    Online  = 1,
    /// A paired device in slot X went offline.
    Offline = 2,
}

/// Result of one pulsaar_poll_device_event call.
#[repr(C)]
pub struct CDeviceConnectionEvent {
    pub event: PulsaarConnectionEvent,
    /// 1-based slot of the device that changed state. 0 when event is None.
    pub slot:  u8,
}

// ---------------------------------------------------------------------------
// Opaque context types (heap-allocated; exposed as raw pointers to callers)
// ---------------------------------------------------------------------------

/// Owns the HID API instance, receiver handle list, and direct device list.
/// Lives for the lifetime of the session.
pub struct PulsaarContext {
    api:                  hidapi::HidApi,
    receivers:            Vec<ReceiverHandle>,
    direct_devices:       Vec<DirectDeviceInfo>,
    /// Paths where Transport::open previously failed (e.g. Privileged=Yes BT LE
    /// devices). Skipped in future enumerate_direct_devices calls so the OS TCC
    /// deny is triggered at most once per path per session.
    unprobeable_bt_paths: std::collections::HashSet<String>,
}

/// Owns one opened receiver and the last-enumerated device list.
/// Lives between pulsaar_open_receiver / pulsaar_close_receiver.
pub struct PulsaarReceiverContext {
    receiver: Receiver,
    devices:  Vec<DeviceInfo>,
    pairing:  Option<PairingSession>,
}

/// Owns a receiver opened exclusively for monitoring device connection-state events.
/// Lives between pulsaar_open_event_listener / pulsaar_close_event_listener.
pub struct PulsaarEventListenerContext {
    receiver: Receiver,
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

fn direct_device_info_to_c(d: &DirectDeviceInfo) -> CDirectDeviceInfo {
    let (has_battery, battery) = battery_to_c(d.battery.as_ref());
    CDirectDeviceInfo {
        product_id: d.product_id,
        kind:       device_kind_to_u8(d.kind),
        name:       str_to_buf(&d.name),
        serial:     str_to_buf(&d.serial),
        has_battery,
        battery,
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
            let receivers                    = enumerate_receivers(&api);
            let mut unprobeable_bt_paths     = std::collections::HashSet::new();
            let direct_devices               = enumerate_direct_devices(&api, &mut unprobeable_bt_paths);
            PulsaarContext { api, receivers, direct_devices, unprobeable_bt_paths }
        })
    });
    match result {
        Ok(Ok(ctx)) => Box::into_raw(Box::new(ctx)),
        _            => std::ptr::null_mut(),
    }
}

/// Re-scan the HID device tree and update the receiver list in place.
///
/// Call this after plugging or unplugging a receiver, before calling
/// pulsaar_get_receiver_count / pulsaar_get_receiver_info again.
/// Any previously opened PulsaarReceiverContext pointers remain valid.
/// Returns InvalidArg if ctx is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_refresh_receivers(ctx: *mut PulsaarContext) -> PulsaarStatus {
    if ctx.is_null() { return PulsaarStatus::InvalidArg; }
    let ctx = &mut *ctx;
    let result = catch_unwind(AssertUnwindSafe(|| {
        ctx.api.refresh_devices().map_err(crate::error::Error::from)?;
        ctx.receivers      = enumerate_receivers(&ctx.api);
        ctx.direct_devices = enumerate_direct_devices(&ctx.api, &mut ctx.unprobeable_bt_paths);
        Ok::<_, crate::error::Error>(())
    }));
    match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => PulsaarStatus::from(e),
        Err(_)     => PulsaarStatus::Unknown,
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
                pairing: None,
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

/// Unpair the device in slot from the opened receiver.
/// slot: 1-based device slot number as returned in CDeviceInfo.slot.
/// Returns InvalidArg if rctx is null or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_unpair_device(
    rctx: *mut PulsaarReceiverContext,
    slot: u8,
) -> PulsaarStatus {
    if rctx.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    let rctx = &mut *rctx;
    let result = catch_unwind(AssertUnwindSafe(|| rctx.receiver.unpair_device(slot)));
    match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => PulsaarStatus::from(e),
        Err(_)     => PulsaarStatus::Unknown,
    }
}

/// Open the pairing lock (Unifying/Nano/LightSpeed) or start device discovery (Bolt).
/// timeout_secs: how long the receiver will wait for a device (1-255, receiver default if 0).
/// After this returns Ok, call pulsaar_poll_pairing in a loop until Paired or Failed.
/// Returns InvalidArg if rctx is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_start_pairing(
    rctx:         *mut PulsaarReceiverContext,
    timeout_secs: u8,
) -> PulsaarStatus {
    if rctx.is_null() { return PulsaarStatus::InvalidArg; }
    let rctx = &mut *rctx;
    let result = catch_unwind(AssertUnwindSafe(|| rctx.receiver.start_pairing(timeout_secs)));
    match result {
        Ok(Ok(session)) => { rctx.pairing = Some(session); PulsaarStatus::Ok }
        Ok(Err(e))      => PulsaarStatus::from(e),
        Err(_)          => PulsaarStatus::Unknown,
    }
}

/// Poll for one pairing event. Blocks for at most timeout_ms milliseconds.
/// out is filled with the current pairing state.
/// Call in a loop after pulsaar_start_pairing until out.state is Paired or Failed.
/// Returns InvalidArg if rctx or out is null, or if pairing has not been started.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_poll_pairing(
    rctx:       *mut PulsaarReceiverContext,
    timeout_ms: u32,
    out:        *mut CPairingStatus,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() { return PulsaarStatus::InvalidArg; }
    let rctx = &mut *rctx;
    let session = match &mut rctx.pairing {
        Some(s) => s,
        None    => {
            (*out).state = PulsaarPairingState::Idle;
            return PulsaarStatus::Ok;
        }
    };

    let result = catch_unwind(AssertUnwindSafe(|| {
        rctx.receiver.poll_pairing(session, timeout_ms as i32)
    }));

    let event = match result {
        Ok(Ok(e))  => e,
        Ok(Err(e)) => { return PulsaarStatus::from(e); }
        Err(_)     => { return PulsaarStatus::Unknown; }
    };

    // Safety: out is non-null, checked above.
    let status = &mut *out;
    // Zero the output struct.
    *status = CPairingStatus {
        state:       PulsaarPairingState::Waiting,
        device_name: [0u8; 64],
        passkey:     [0u8; 16],
        error:       [0u8; 64],
    };

    match event {
        PairingEvent::Waiting => {
            status.state = PulsaarPairingState::Waiting;
        }
        PairingEvent::BoltDeviceFound(name) => {
            status.state = PulsaarPairingState::DeviceFound;
            let b = name.as_bytes();
            let n = b.len().min(63);
            status.device_name[..n].copy_from_slice(&b[..n]);
        }
        PairingEvent::PasskeyNumeric(pk) => {
            status.state = PulsaarPairingState::PasskeyNumeric;
            let b = pk.as_bytes();
            let n = b.len().min(15);
            status.passkey[..n].copy_from_slice(&b[..n]);
        }
        PairingEvent::PasskeyButton(pk) => {
            status.state = PulsaarPairingState::PasskeyButton;
            let b = pk.as_bytes();
            let n = b.len().min(15);
            status.passkey[..n].copy_from_slice(&b[..n]);
        }
        PairingEvent::Paired(slot) => {
            status.state = PulsaarPairingState::Paired;
            // Embed slot in device_name[0] for easy retrieval.
            status.device_name[0] = slot;
            rctx.pairing = None; // session is done
        }
        PairingEvent::Failed(msg) => {
            status.state = PulsaarPairingState::Failed;
            let b = msg.as_bytes();
            let n = b.len().min(63);
            status.error[..n].copy_from_slice(&b[..n]);
            rctx.pairing = None;
        }
    }

    PulsaarStatus::Ok
}

/// Cancel an in-progress pairing. Closes the lock / stops discovery.
/// Safe to call even if pairing is not in progress.
/// Returns InvalidArg if rctx is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_cancel_pairing(
    rctx: *mut PulsaarReceiverContext,
) -> PulsaarStatus {
    if rctx.is_null() { return PulsaarStatus::InvalidArg; }
    let rctx = &mut *rctx;
    if rctx.pairing.is_some() {
        let result = catch_unwind(AssertUnwindSafe(|| rctx.receiver.cancel_pairing()));
        rctx.pairing = None;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return PulsaarStatus::from(e),
            Err(_)     => return PulsaarStatus::Unknown,
        }
    }
    PulsaarStatus::Ok
}

// ---------------------------------------------------------------------------
// Event listener: device connection-state monitoring
// ---------------------------------------------------------------------------

/// Open a receiver for connection-state event monitoring.
///
/// Opens an independent HID handle for the receiver at `index` and enables
/// wireless notifications, so the receiver sends 0x41 notifications when a
/// paired device comes online or goes offline.
///
/// Returns a pointer to the listener context, or null on failure.
/// The caller must eventually call pulsaar_close_event_listener.
/// Returns InvalidArg if ctx is null or index is out of range.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_open_event_listener(
    ctx:        *mut PulsaarContext,
    index:      usize,
    status_out: *mut PulsaarStatus,
) -> *mut PulsaarEventListenerContext {
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

    match Receiver::open_for_events(&(*ctx).api, handle) {
        Ok(receiver) => {
            if !status_out.is_null() { *status_out = PulsaarStatus::Ok; }
            Box::into_raw(Box::new(PulsaarEventListenerContext { receiver }))
        }
        Err(e) => fail!(PulsaarStatus::from(e)),
    }
}

/// Poll for one device connection-state event. Blocks for at most timeout_ms milliseconds.
///
/// `out` is filled with the event kind and the slot of the affected device.
/// Returns Ok with event=None on timeout or non-connection messages.
/// Returns InvalidArg if listener or out is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_poll_device_event(
    listener:   *mut PulsaarEventListenerContext,
    timeout_ms: u32,
    out:        *mut CDeviceConnectionEvent,
) -> PulsaarStatus {
    if listener.is_null() || out.is_null() { return PulsaarStatus::InvalidArg; }
    let listener = &*listener;

    *out = CDeviceConnectionEvent { event: PulsaarConnectionEvent::None, slot: 0 };

    let result = catch_unwind(AssertUnwindSafe(|| {
        listener.receiver.poll_device_event(timeout_ms as i32)
    }));

    match result {
        Ok(Ok(Some((slot, true))))  => {
            (*out).event = PulsaarConnectionEvent::Online;
            (*out).slot  = slot;
            PulsaarStatus::Ok
        }
        Ok(Ok(Some((slot, false)))) => {
            (*out).event = PulsaarConnectionEvent::Offline;
            (*out).slot  = slot;
            PulsaarStatus::Ok
        }
        Ok(Ok(None))   => PulsaarStatus::Ok,
        Ok(Err(e))     => PulsaarStatus::from(e),
        Err(_)         => PulsaarStatus::Unknown,
    }
}

/// Close an event listener and free its context. Safe to call with null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_close_event_listener(listener: *mut PulsaarEventListenerContext) {
    if !listener.is_null() {
        drop(Box::from_raw(listener));
    }
}

// ---------------------------------------------------------------------------
// Direct (Bluetooth) device enumeration
// ---------------------------------------------------------------------------

/// Re-enumerate directly-connected (Bluetooth) Logitech devices.
///
/// Call this after a Bluetooth device connects or disconnects. The result is
/// cached in the context and read via pulsaar_get_direct_device_count and
/// pulsaar_get_direct_device_info.
///
/// Note: enumeration opens each candidate HID interface briefly to probe for
/// HID++ 2.0 support, then closes it. Devices that do not respond are skipped.
///
/// Returns InvalidArg if ctx is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_refresh_direct_devices(ctx: *mut PulsaarContext) -> PulsaarStatus {
    if ctx.is_null() { return PulsaarStatus::InvalidArg; }
    let ctx = &mut *ctx;
    let result = catch_unwind(AssertUnwindSafe(|| {
        ctx.api.refresh_devices().map_err(crate::error::Error::from)?;
        ctx.direct_devices = enumerate_direct_devices(&ctx.api, &mut ctx.unprobeable_bt_paths);
        Ok::<_, crate::error::Error>(())
    }));
    match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => PulsaarStatus::from(e),
        Err(_)     => PulsaarStatus::Unknown,
    }
}

/// Number of directly-connected devices found at last enumeration. Returns 0 if ctx is null.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_direct_device_count(ctx: *const PulsaarContext) -> usize {
    if ctx.is_null() { return 0; }
    (*ctx).direct_devices.len()
}

/// Fill `out` with info for the direct device at `index` (0-based).
///
/// Returns InvalidArg if ctx or out is null, or index is out of range.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_direct_device_info(
    ctx:   *const PulsaarContext,
    index: usize,
    out:   *mut CDirectDeviceInfo,
) -> PulsaarStatus {
    if ctx.is_null() || out.is_null() { return PulsaarStatus::InvalidArg; }
    let device = match (&(*ctx).direct_devices).get(index) {
        Some(d) => d,
        None    => return PulsaarStatus::InvalidArg,
    };
    *out = direct_device_info_to_c(device);
    PulsaarStatus::Ok
}
