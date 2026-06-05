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
#[derive(Copy, Clone, Debug)]
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

/// DPI capabilities and current state for a device with the Adjustable DPI feature (0x2201).
///
/// dpi_count:   Number of entries populated in dpi_list. 0 means the feature is absent.
/// dpi_list:    Supported DPI values (native u16, sorted ascending), up to 200 entries.
/// current_dpi: Currently active DPI. 0 if not reported by the device.
/// default_dpi: Default DPI reported by the device. 0 if not reported.
#[repr(C)]
pub struct CDpiSettings {
    pub dpi_count:   u8,
    pub dpi_list:    [u16; 200],
    pub current_dpi: u16,
    pub default_dpi: u16,
}

/// Scroll wheel capabilities and current state for a device with the HiRes Wheel feature (0x2121).
///
/// has_invert:    1 if the device supports scroll inversion, 0 if not.
/// has_hires:     1 if the device supports hi-res scroll mode, 0 if not.
/// inverted:      Current inversion state (1=inverted). Meaningful only if has_invert=1.
/// hires_enabled: Current hi-res mode state (1=enabled). Meaningful only if has_hires=1.
#[repr(C)]
pub struct CScrollSettings {
    pub has_invert:    u8,
    pub has_hires:     u8,
    pub inverted:      u8,
    pub hires_enabled: u8,
}

/// SmartShift ratchet state for a device with FEAT_SMART_SHIFT_ENHANCED (0x2111).
///
/// wheel_mode: 1=always freespin, 2=smart-shift (auto based on speed).
/// has_torque: 1 if the device supports tunable ratchet torque, 0 if not.
/// torque:     Ratchet engagement force, 1-100. Meaningful only if has_torque=1.
#[repr(C)]
pub struct CSmartShiftSettings {
    pub wheel_mode: u8,
    pub has_torque: u8,
    pub torque:     u8,
}

/// Info about one host slot as returned by pulsaar_get_hosts.
///
/// slot:      0-based slot index (use this value with pulsaar_set_active_host).
/// name:      Null-terminated host name (up to 63 chars).
/// is_active: 1 if this is the currently active host, 0 otherwise.
#[repr(C)]
pub struct CHostInfo {
    pub slot:      u8,
    pub name:      [u8; 64],
    pub is_active: u8,
}

/// List of hosts returned by pulsaar_get_hosts.
///
/// count: number of valid entries in hosts (0 if the feature is absent).
#[repr(C)]
pub struct CHostList {
    pub count: u8,
    pub hosts: [CHostInfo; 8],
}

/// FN key inversion state for a keyboard with FEAT_FN_INVERSION (0x40A0 / 0x40A2 / 0x40A3).
///
/// has_feature: 1 if the device has any FN inversion feature, 0 if absent.
/// fn_swapped:  1 if F1-F12 keys act as multimedia keys by default (swap is active).
#[repr(C)]
pub struct CFnSettings {
    pub has_feature: u8,
    pub fn_swapped:  u8,
}

/// Multiplatform OS layout state for a keyboard with FEAT_MULTIPLATFORM (0x4531).
///
/// count:            number of valid entries in platform_names/platform_indices (0 if absent).
/// current:          index into platform_names/platform_indices of the active platform.
/// platform_names:   null-terminated OS name strings, up to 8 platforms.
/// platform_indices: platform index values to pass to pulsaar_set_multiplatform.
#[repr(C)]
pub struct CMultiplatformSettings {
    pub count:             u8,
    pub current:           u8,
    pub platform_names:    [[u8; 32]; 8],
    pub platform_indices:  [u8; 8],
}

/// Backlight state for a keyboard with FEAT_BACKLIGHT2 (0x1982).
///
/// has_feature:      1 if the device has FEAT_BACKLIGHT2, 0 if absent.
/// mode:             0=disabled, 1=automatic, 3=manual (permanent on).
/// auto_supported:   1 if automatic mode is available, 0 if not.
/// manual_supported: 1 if manual mode is available, 0 if not.
/// brightness:       0-100 brightness level (relevant when mode=3).
#[repr(C)]
pub struct CBacklightSettings {
    pub has_feature:      u8,
    pub mode:             u8,
    pub auto_supported:   u8,
    pub manual_supported: u8,
    pub brightness:       u8,
}

/// All configurable device settings returned in a single batch call.
/// Fields for absent features are zero-initialized, identical to the individual pulsaar_get_* calls.
#[repr(C)]
pub struct CAllDeviceSettings {
    pub dpi:               CDpiSettings,
    pub scroll:            CScrollSettings,
    pub ss:                CSmartShiftSettings,
    pub hosts:             CHostList,
    pub fn_s:              CFnSettings,
    pub mp:                CMultiplatformSettings,
    pub backlight:         CBacklightSettings,
    /// Feature index of REPROG_CONTROLS_V4 (0x1B04), or 0 if absent.
    /// Used by the event listener to distinguish button-CID notifications
    /// from actual settings-change notifications.
    pub reprog_controls_idx: u8,
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
    /// The device in slot X sent an unsolicited HID++ 2.0 feature notification,
    /// indicating a local settings change (FN mode, scroll mode, SmartShift, etc.).
    /// The caller should re-read settings for this slot and refresh the UI.
    SettingsChanged = 3,
}

/// Result of one pulsaar_poll_device_event call.
#[repr(C)]
pub struct CDeviceConnectionEvent {
    pub event: PulsaarConnectionEvent,
    /// 1-based slot of the device that changed state. 0 when event is None.
    pub slot:  u8,
    /// For SettingsChanged: the sub_id (= HID++ 2.0 feature index) that triggered
    /// the notification. 0 for LinkChange and None events.
    /// Compare against reprog_controls_idx from CAllDeviceSettings to distinguish
    /// button-CID events (0x1B04) from actual settings changes.
    pub feature_index: u8,
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
            eprintln!("[PULSAAR][FFI] pulsaar_open_receiver[{}] '{}' ok", index, handle.name);
            if !status_out.is_null() { *status_out = PulsaarStatus::Ok; }
            Box::into_raw(Box::new(PulsaarReceiverContext {
                receiver,
                devices: Vec::new(),
                pairing: None,
            }))
        }
        Err(e) => {
            eprintln!("[PULSAAR][FFI] pulsaar_open_receiver[{}] FAIL: {:?}", index, e);
            fail!(PulsaarStatus::from(e))
        }
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

    *out = CDeviceConnectionEvent { event: PulsaarConnectionEvent::None, slot: 0, feature_index: 0 };

    let result = catch_unwind(AssertUnwindSafe(|| {
        listener.receiver.poll_device_event(timeout_ms as i32)
    }));

    use crate::receiver::DeviceEvent;
    match result {
        Ok(Ok(Some(DeviceEvent::LinkChange { slot, online: true })))  => {
            (*out).event = PulsaarConnectionEvent::Online;
            (*out).slot  = slot;
            PulsaarStatus::Ok
        }
        Ok(Ok(Some(DeviceEvent::LinkChange { slot, online: false }))) => {
            (*out).event = PulsaarConnectionEvent::Offline;
            (*out).slot  = slot;
            PulsaarStatus::Ok
        }
        Ok(Ok(Some(DeviceEvent::SettingsChanged { slot, feature_index }))) => {
            (*out).event         = PulsaarConnectionEvent::SettingsChanged;
            (*out).slot          = slot;
            (*out).feature_index = feature_index;
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

// ---------------------------------------------------------------------------
// Device settings: DPI and scroll wheel
// ---------------------------------------------------------------------------

/// Read DPI capabilities and current DPI for the device in the given slot.
///
/// Discovers HID++ 2.0 features first, then reads FEAT_ADJUSTABLE_DPI (0x2201).
/// On success, out->dpi_count > 0 and out->dpi_list is populated.
/// If the feature is absent on this device, out->dpi_count == 0 and Ok is returned.
/// Returns InvalidArg if rctx or out is null, or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_dpi_settings(
    rctx: *const PulsaarReceiverContext,
    slot: u8,
    out:  *mut CDpiSettings,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    *out = CDpiSettings { dpi_count: 0, dpi_list: [0u16; 200], current_dpi: 0, default_dpi: 0 };
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.get_dpi_info(slot)));
    match result {
        Ok(Ok(Some(info))) => {
            let count = info.dpi_list.len().min(200);
            (*out).dpi_count   = count as u8;
            (*out).current_dpi = info.current_dpi;
            (*out).default_dpi = info.default_dpi;
            for (i, &val) in info.dpi_list.iter().take(200).enumerate() {
                (*out).dpi_list[i] = val;
            }
            PulsaarStatus::Ok
        }
        Ok(Ok(None)) => PulsaarStatus::Ok,
        Ok(Err(e))   => PulsaarStatus::from(e),
        Err(_)       => PulsaarStatus::Unknown,
    }
}

/// Set the active DPI for the device in the given slot.
///
/// Returns Ok if the device does not support FEAT_ADJUSTABLE_DPI (no-op).
/// Returns InvalidArg if rctx is null or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_set_dpi(
    rctx: *const PulsaarReceiverContext,
    slot: u8,
    dpi:  u16,
) -> PulsaarStatus {
    eprintln!("[PULSAAR][FFI] pulsaar_set_dpi slot={} dpi={}", slot, dpi);
    if rctx.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.set_dpi(slot, dpi)));
    let status = match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => { eprintln!("[PULSAAR][FFI]   set_dpi err: {:?}", e); PulsaarStatus::from(e) }
        Err(_)     => PulsaarStatus::Unknown,
    };
    eprintln!("[PULSAAR][FFI]   set_dpi -> {:?}", status as u32);
    status
}

/// Read scroll wheel capabilities and current mode for the device in the given slot.
///
/// Discovers HID++ 2.0 features first, then reads FEAT_HIRES_WHEEL (0x2121).
/// On success, out->has_invert and out->has_hires reflect device capabilities.
/// If the feature is absent, both capability flags are 0 and Ok is returned.
/// Returns InvalidArg if rctx or out is null, or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_scroll_settings(
    rctx: *const PulsaarReceiverContext,
    slot: u8,
    out:  *mut CScrollSettings,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    *out = CScrollSettings { has_invert: 0, has_hires: 0, inverted: 0, hires_enabled: 0 };
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.get_scroll_info(slot)));
    match result {
        Ok(Ok(Some(info))) => {
            (*out).has_invert    = if info.has_invert { 1 } else { 0 };
            (*out).has_hires     = if info.has_hires  { 1 } else { 0 };
            (*out).inverted      = if info.inverted      { 1 } else { 0 };
            (*out).hires_enabled = if info.hires_enabled { 1 } else { 0 };
            PulsaarStatus::Ok
        }
        Ok(Ok(None)) => PulsaarStatus::Ok, // feature absent; flags stay 0
        Ok(Err(e))   => PulsaarStatus::from(e),
        Err(_)       => PulsaarStatus::Unknown,
    }
}

/// Set scroll wheel inversion and hi-res mode for the device in the given slot.
///
/// inverted:      1 to enable scroll inversion, 0 to disable.
/// hires_enabled: 1 to enable hi-res scroll mode, 0 to disable.
/// Returns Ok if the device does not support FEAT_HIRES_WHEEL (no-op).
/// Returns InvalidArg if rctx is null or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_set_scroll_settings(
    rctx:         *const PulsaarReceiverContext,
    slot:         u8,
    inverted:     u8,
    hires_enabled: u8,
) -> PulsaarStatus {
    eprintln!("[PULSAAR][FFI] pulsaar_set_scroll_settings slot={} inverted={} hires={}", slot, inverted, hires_enabled);
    if rctx.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    let result = catch_unwind(AssertUnwindSafe(|| {
        (*rctx).receiver.set_scroll_settings(slot, inverted != 0, hires_enabled != 0)
    }));
    let status = match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => { eprintln!("[PULSAAR][FFI]   set_scroll err: {:?}", e); PulsaarStatus::from(e) }
        Err(_)     => PulsaarStatus::Unknown,
    };
    eprintln!("[PULSAAR][FFI]   set_scroll -> {:?}", status as u32);
    status
}

// ---------------------------------------------------------------------------
// Device settings: SmartShift, Change Host, FN swap, Multiplatform, Backlight
// ---------------------------------------------------------------------------

/// Read smart-shift ratchet mode and torque for the device in the given slot.
///
/// If FEAT_SMART_SHIFT_ENHANCED is absent, out->wheel_mode is set to 0 and Ok is returned.
/// Returns InvalidArg if rctx or out is null, or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_smartshift(
    rctx: *const PulsaarReceiverContext,
    slot: u8,
    out:  *mut CSmartShiftSettings,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    *out = CSmartShiftSettings { wheel_mode: 0, has_torque: 0, torque: 0 };
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.get_smart_shift(slot)));
    match result {
        Ok(Ok(Some(info))) => {
            (*out).wheel_mode = info.wheel_mode;
            (*out).has_torque = if info.has_torque { 1 } else { 0 };
            (*out).torque     = info.torque;
            PulsaarStatus::Ok
        }
        Ok(Ok(None)) => PulsaarStatus::Ok,
        Ok(Err(e))   => PulsaarStatus::from(e),
        Err(_)       => PulsaarStatus::Unknown,
    }
}

/// Set smart-shift wheel mode and torque for the device in the given slot.
///
/// wheel_mode: 1=freespin, 2=smart-shift.
/// torque:     ratchet engagement force 1-100 (ignored if device has_torque=0).
/// Returns Ok if the device does not support FEAT_SMART_SHIFT_ENHANCED (no-op).
/// Returns InvalidArg if rctx is null or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_set_smartshift(
    rctx:       *const PulsaarReceiverContext,
    slot:       u8,
    wheel_mode: u8,
    torque:     u8,
) -> PulsaarStatus {
    eprintln!("[PULSAAR][FFI] pulsaar_set_smartshift slot={} wheel_mode={} torque={}", slot, wheel_mode, torque);
    if rctx.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.set_smart_shift(slot, wheel_mode, torque)));
    let status = match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => { eprintln!("[PULSAAR][FFI]   set_smartshift err: {:?}", e); PulsaarStatus::from(e) }
        Err(_)     => PulsaarStatus::Unknown,
    };
    eprintln!("[PULSAAR][FFI]   set_smartshift -> {:?}", status as u32);
    status
}

/// Read the host slot list for the device in the given slot.
///
/// If FEAT_CHANGE_HOST is absent, out->count is set to 0 and Ok is returned.
/// Returns InvalidArg if rctx or out is null, or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_hosts(
    rctx: *const PulsaarReceiverContext,
    slot: u8,
    out:  *mut CHostList,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    // Zero the struct; CHostInfo arrays are large so zeroing explicitly is cleaner.
    let out_ref = &mut *out;
    out_ref.count = 0;
    for i in 0..8 {
        out_ref.hosts[i] = CHostInfo { slot: 0, name: [0u8; 64], is_active: 0 };
    }

    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.get_hosts(slot)));
    match result {
        Ok(Ok(Some(hosts))) => {
            let count = hosts.len().min(8);
            out_ref.count = count as u8;
            for (i, h) in hosts.iter().take(8).enumerate() {
                out_ref.hosts[i].slot      = h.slot;
                out_ref.hosts[i].name      = str_to_buf(&h.name);
                out_ref.hosts[i].is_active = if h.is_active { 1 } else { 0 };
            }
            PulsaarStatus::Ok
        }
        Ok(Ok(None)) => PulsaarStatus::Ok,
        Ok(Err(e))   => PulsaarStatus::from(e),
        Err(_)       => PulsaarStatus::Unknown,
    }
}

/// Switch the active host for the device in the given slot.
///
/// The device will disconnect immediately after receiving this command.
/// Returns InvalidArg if rctx is null or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_set_active_host(
    rctx:      *const PulsaarReceiverContext,
    slot:      u8,
    host_slot: u8,
) -> PulsaarStatus {
    eprintln!("[PULSAAR][FFI] pulsaar_set_active_host slot={} host_slot={}", slot, host_slot);
    if rctx.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.set_active_host(slot, host_slot)));
    let status = match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => { eprintln!("[PULSAAR][FFI]   set_active_host err: {:?}", e); PulsaarStatus::from(e) }
        Err(_)     => PulsaarStatus::Unknown,
    };
    eprintln!("[PULSAAR][FFI]   set_active_host -> {:?}", status as u32);
    status
}

/// Read FN key inversion state for the device in the given slot.
///
/// If no FN inversion feature is present, out->fn_swapped is set to 0 and Ok is returned.
/// Returns InvalidArg if rctx or out is null, or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_fn_settings(
    rctx: *const PulsaarReceiverContext,
    slot: u8,
    out:  *mut CFnSettings,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    *out = CFnSettings { has_feature: 0, fn_swapped: 0 };
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.get_fn_settings(slot)));
    match result {
        Ok(Ok(Some(info))) => {
            (*out).has_feature = 1;
            (*out).fn_swapped  = if info.fn_swapped { 1 } else { 0 };
            PulsaarStatus::Ok
        }
        Ok(Ok(None)) => PulsaarStatus::Ok,
        Ok(Err(e))   => PulsaarStatus::from(e),
        Err(_)       => PulsaarStatus::Unknown,
    }
}

/// Set FN key inversion for the device in the given slot.
///
/// swapped: 1 to make F1-F12 act as multimedia keys by default, 0 for standard function keys.
/// Returns Ok if the device does not support any FN inversion feature (no-op).
/// Returns InvalidArg if rctx is null or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_set_fn_swap(
    rctx:    *const PulsaarReceiverContext,
    slot:    u8,
    swapped: u8,
) -> PulsaarStatus {
    eprintln!("[PULSAAR][FFI] pulsaar_set_fn_swap slot={} swapped={}", slot, swapped);
    if rctx.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.set_fn_swap(slot, swapped != 0)));
    let status = match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => { eprintln!("[PULSAAR][FFI]   set_fn_swap err: {:?}", e); PulsaarStatus::from(e) }
        Err(_)     => PulsaarStatus::Unknown,
    };
    eprintln!("[PULSAAR][FFI]   set_fn_swap -> {:?}", status as u32);
    status
}

/// Read multiplatform OS layout state for the device in the given slot.
///
/// If the feature is absent or the device cannot change OS, out->count is 0 and Ok is returned.
/// Returns InvalidArg if rctx or out is null, or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_multiplatform(
    rctx: *const PulsaarReceiverContext,
    slot: u8,
    out:  *mut CMultiplatformSettings,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    let out_ref = &mut *out;
    out_ref.count   = 0;
    out_ref.current = 0;
    for i in 0..8 {
        out_ref.platform_names[i]   = [0u8; 32];
        out_ref.platform_indices[i] = 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.get_multiplatform(slot)));
    match result {
        Ok(Ok(Some(info))) => {
            let count = info.platforms.len().min(8);
            out_ref.count   = count as u8;
            // Find which platforms entry has index == info.current.
            let current_pos = info.platforms.iter().position(|p| p.index == info.current).unwrap_or(0);
            out_ref.current = current_pos as u8;
            for (i, p) in info.platforms.iter().take(8).enumerate() {
                out_ref.platform_names[i]   = str_to_buf(&p.name);
                out_ref.platform_indices[i] = p.index;
            }
            PulsaarStatus::Ok
        }
        Ok(Ok(None)) => PulsaarStatus::Ok,
        Ok(Err(e))   => PulsaarStatus::from(e),
        Err(_)       => PulsaarStatus::Unknown,
    }
}

/// Set the active OS platform for the device in the given slot.
///
/// platform_index: the raw platform index from CMultiplatformSettings.platform_indices[n].
/// Returns Ok if the device does not support FEAT_MULTIPLATFORM (no-op).
/// Returns InvalidArg if rctx is null or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_set_multiplatform(
    rctx:           *const PulsaarReceiverContext,
    slot:           u8,
    platform_index: u8,
) -> PulsaarStatus {
    eprintln!("[PULSAAR][FFI] pulsaar_set_multiplatform slot={} platform_index={}", slot, platform_index);
    if rctx.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.set_multiplatform(slot, platform_index)));
    let status = match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => { eprintln!("[PULSAAR][FFI]   set_multiplatform err: {:?}", e); PulsaarStatus::from(e) }
        Err(_)     => PulsaarStatus::Unknown,
    };
    eprintln!("[PULSAAR][FFI]   set_multiplatform -> {:?}", status as u32);
    status
}

/// Read backlight state for the device in the given slot.
///
/// If FEAT_BACKLIGHT2 is absent, out->mode is set to 0 and Ok is returned.
/// Returns InvalidArg if rctx or out is null, or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_backlight(
    rctx: *const PulsaarReceiverContext,
    slot: u8,
    out:  *mut CBacklightSettings,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    *out = CBacklightSettings { has_feature: 0, mode: 0, auto_supported: 0, manual_supported: 0, brightness: 0 };
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.get_backlight(slot)));
    match result {
        Ok(Ok(Some(info))) => {
            (*out).has_feature      = 1;
            (*out).mode             = info.mode;
            (*out).auto_supported   = if info.auto_supported   { 1 } else { 0 };
            (*out).manual_supported = if info.manual_supported { 1 } else { 0 };
            (*out).brightness       = info.level;
            PulsaarStatus::Ok
        }
        Ok(Ok(None)) => PulsaarStatus::Ok,
        Ok(Err(e))   => PulsaarStatus::from(e),
        Err(_)       => PulsaarStatus::Unknown,
    }
}

/// Read all configurable settings for the device in the given slot in one call.
///
/// Performs a single feature discovery and reads all supported features with the same map.
/// More efficient than calling the seven individual pulsaar_get_* functions, each of which
/// performs its own feature discovery. Individual feature errors are treated as absent.
///
/// Returns InvalidArg if rctx or out is null, or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_get_all_settings(
    rctx: *const PulsaarReceiverContext,
    slot: u8,
    out:  *mut CAllDeviceSettings,
) -> PulsaarStatus {
    if rctx.is_null() || out.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }

    let out_ref = &mut *out;
    out_ref.dpi      = CDpiSettings { dpi_count: 0, dpi_list: [0u16; 200], current_dpi: 0, default_dpi: 0 };
    out_ref.scroll   = CScrollSettings { has_invert: 0, has_hires: 0, inverted: 0, hires_enabled: 0 };
    out_ref.ss       = CSmartShiftSettings { wheel_mode: 0, has_torque: 0, torque: 0 };
    out_ref.hosts.count = 0;
    for i in 0..8 { out_ref.hosts.hosts[i] = CHostInfo { slot: 0, name: [0u8; 64], is_active: 0 }; }
    out_ref.fn_s     = CFnSettings { has_feature: 0, fn_swapped: 0 };
    out_ref.mp.count   = 0;
    out_ref.mp.current = 0;
    for i in 0..8 { out_ref.mp.platform_names[i] = [0u8; 32]; out_ref.mp.platform_indices[i] = 0; }
    out_ref.backlight           = CBacklightSettings { has_feature: 0, mode: 0, auto_supported: 0, manual_supported: 0, brightness: 0 };
    out_ref.reprog_controls_idx = 0;

    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.get_all_settings(slot)));
    match result {
        Ok(Ok(all)) => {
            if let Some(dpi) = all.dpi {
                let count = dpi.dpi_list.len().min(200);
                out_ref.dpi.dpi_count   = count as u8;
                out_ref.dpi.current_dpi = dpi.current_dpi;
                out_ref.dpi.default_dpi = dpi.default_dpi;
                for (i, &val) in dpi.dpi_list.iter().take(200).enumerate() {
                    out_ref.dpi.dpi_list[i] = val;
                }
            }
            if let Some(scroll) = all.scroll {
                out_ref.scroll.has_invert    = if scroll.has_invert    { 1 } else { 0 };
                out_ref.scroll.has_hires     = if scroll.has_hires     { 1 } else { 0 };
                out_ref.scroll.inverted      = if scroll.inverted      { 1 } else { 0 };
                out_ref.scroll.hires_enabled = if scroll.hires_enabled { 1 } else { 0 };
            }
            if let Some(ss) = all.smart_shift {
                out_ref.ss.wheel_mode = ss.wheel_mode;
                out_ref.ss.has_torque = if ss.has_torque { 1 } else { 0 };
                out_ref.ss.torque     = ss.torque;
            }
            if let Some(hosts) = all.hosts {
                let count = hosts.len().min(8);
                out_ref.hosts.count = count as u8;
                for (i, h) in hosts.iter().take(8).enumerate() {
                    out_ref.hosts.hosts[i].slot      = h.slot;
                    out_ref.hosts.hosts[i].name      = str_to_buf(&h.name);
                    out_ref.hosts.hosts[i].is_active = if h.is_active { 1 } else { 0 };
                }
            }
            if let Some(fn_s) = all.fn_settings {
                out_ref.fn_s.has_feature = 1;
                out_ref.fn_s.fn_swapped  = if fn_s.fn_swapped { 1 } else { 0 };
            }
            if let Some(mp) = all.multiplatform {
                let count = mp.platforms.len().min(8);
                out_ref.mp.count   = count as u8;
                let cur_pos = mp.platforms.iter().position(|p| p.index == mp.current).unwrap_or(0);
                out_ref.mp.current = cur_pos as u8;
                for (i, p) in mp.platforms.iter().take(8).enumerate() {
                    out_ref.mp.platform_names[i]   = str_to_buf(&p.name);
                    out_ref.mp.platform_indices[i] = p.index;
                }
            }
            if let Some(bl) = all.backlight {
                out_ref.backlight.has_feature      = 1;
                out_ref.backlight.mode             = bl.mode;
                out_ref.backlight.auto_supported   = if bl.auto_supported   { 1 } else { 0 };
                out_ref.backlight.manual_supported = if bl.manual_supported { 1 } else { 0 };
                out_ref.backlight.brightness       = bl.level;
            }
            out_ref.reprog_controls_idx = all.reprog_controls_idx;
            PulsaarStatus::Ok
        }
        Ok(Err(e)) => PulsaarStatus::from(e),
        Err(_)     => PulsaarStatus::Unknown,
    }
}

/// Set backlight mode and brightness for the device in the given slot.
///
/// mode:       0=disabled, 1=automatic, 3=manual.
/// brightness: 0-100 (used only when mode=3).
/// Returns Ok if the device does not support FEAT_BACKLIGHT2 (no-op).
/// Returns InvalidArg if rctx is null or slot is 0.
#[no_mangle]
pub unsafe extern "C" fn pulsaar_set_backlight(
    rctx:       *const PulsaarReceiverContext,
    slot:       u8,
    mode:       u8,
    brightness: u8,
) -> PulsaarStatus {
    eprintln!("[PULSAAR][FFI] pulsaar_set_backlight slot={} mode={} brightness={}", slot, mode, brightness);
    if rctx.is_null() || slot == 0 { return PulsaarStatus::InvalidArg; }
    let result = catch_unwind(AssertUnwindSafe(|| (*rctx).receiver.set_backlight(slot, mode, brightness)));
    let status = match result {
        Ok(Ok(())) => PulsaarStatus::Ok,
        Ok(Err(e)) => { eprintln!("[PULSAAR][FFI]   set_backlight err: {:?}", e); PulsaarStatus::from(e) }
        Err(_)     => PulsaarStatus::Unknown,
    };
    eprintln!("[PULSAAR][FFI]   set_backlight -> {:?}", status as u32);
    status
}
