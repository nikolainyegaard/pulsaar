// C declarations for the Pulsaar Rust core FFI.
// Referenced as the Swift Objective-C bridging header.
// Re-declares the types and functions from core/src/ffi.rs in C so Swift
// can call them directly via the bridging mechanism.

#pragma once
#include <stdint.h>
#include <stddef.h>

// ---------------------------------------------------------------------------
// Status codes (mirror PulsaarStatus in core/src/ffi.rs)
// ---------------------------------------------------------------------------

typedef enum {
    PulsaarStatusOk         = 0,
    PulsaarStatusHidError   = 1,
    PulsaarStatusTimeout    = 2,
    PulsaarStatusNoReceiver = 3,
    PulsaarStatusEmptySlot  = 4,
    PulsaarStatusInvalidArg = 5,
    PulsaarStatusUnknown    = 99,
} PulsaarStatus;

// ---------------------------------------------------------------------------
// C structs (mirror the #[repr(C)] structs in core/src/ffi.rs)
// ---------------------------------------------------------------------------

// Info about a receiver as enumerated from the HID device list (pre-open).
typedef struct {
    uint16_t product_id;
    uint8_t  kind;        // 0=Unifying, 1=Bolt, 2=Nano, 3=LightSpeed
    uint8_t  name[64];    // null-terminated display name
    uint8_t  path[256];   // null-terminated OS HID path
} CReceiverInfo;

// Info about a receiver after it has been successfully opened.
typedef struct {
    uint16_t product_id;
    uint8_t  kind;        // 0=Unifying, 1=Bolt, 2=Nano, 3=LightSpeed
    uint8_t  max_devices;
    uint8_t  name[64];    // null-terminated display name
    uint8_t  serial[33];  // null-terminated serial; 33 bytes holds Bolt's 32-char hex serial
} COpenedReceiverInfo;

// Battery state for a device.
// level: 0-100 if reported, 0xFF if not available.
// status: 0=Discharging, 1=Recharging, 2=AlmostFull, 3=Full, 4=SlowRecharge,
//         5=InvalidBattery, 6=ThermalError, 0xFF if not available.
// voltage: millivolts if reported, 0 if not available.
typedef struct {
    uint8_t  level;
    uint8_t  status;
    uint16_t voltage;
} CBattery;

// Info about a device paired to a receiver.
// kind: 0=Unknown, 1=Keyboard, 2=Mouse, 3=Numpad, 4=Presenter, 5=Remote,
//       6=Trackball, 7=Touchpad, 8=Tablet, 9=Gamepad, 10=Joystick,
//       11=Headset, 12=RemoteControl, 13=Receiver
// has_battery: 0 if no battery info, 1 if battery field is populated.
typedef struct {
    uint8_t  slot;
    uint8_t  kind;
    uint8_t  wpid[2];
    uint8_t  name[64];    // null-terminated device name
    uint8_t  serial[32];  // null-terminated serial (hex string)
    uint8_t  has_battery;
    CBattery battery;
} CDeviceInfo;

// ---------------------------------------------------------------------------
// Pairing types
// ---------------------------------------------------------------------------

typedef enum {
    PulsaarPairingStateWaiting        = 0, // lock open, waiting for a device
    PulsaarPairingStateDeviceFound    = 1, // Bolt: device found, pairing initiated
    PulsaarPairingStatePasskeyNumeric = 2, // Bolt: type passkey digits on keyboard then press Enter
    PulsaarPairingStatePasskeyButton  = 3, // Bolt: press L/R buttons per passkey string, then both
    PulsaarPairingStatePaired         = 4, // pairing complete; device_name[0] = slot (1-based)
    PulsaarPairingStateFailed         = 5, // pairing failed; see error field
    PulsaarPairingStateIdle           = 6, // no pairing in progress
} PulsaarPairingState;

// Result of one pulsaar_poll_pairing call.
typedef struct {
    PulsaarPairingState state;
    uint8_t device_name[64]; // null-terminated; valid for DeviceFound and Paired
                             // Paired: device_name[0] = 1-based slot of new device
    uint8_t passkey[16];     // null-terminated; valid for PasskeyNumeric and PasskeyButton
    uint8_t error[64];       // null-terminated; valid for Failed
} CPairingStatus;

// ---------------------------------------------------------------------------
// Opaque context types (heap-allocated Rust structs; never inspected in Swift)
// ---------------------------------------------------------------------------

struct PulsaarContext;
struct PulsaarReceiverContext;

// ---------------------------------------------------------------------------
// Exported functions
// ---------------------------------------------------------------------------

// Initialize HID and enumerate receivers. Returns null on failure.
// The caller must eventually call pulsaar_destroy.
struct PulsaarContext *pulsaar_init(void);

// Re-scan the HID device tree and update the receiver list in place.
// Call this after plugging or unplugging a receiver, before querying receiver count/info.
// Any previously opened PulsaarReceiverContext pointers remain valid.
PulsaarStatus pulsaar_refresh_receivers(struct PulsaarContext *ctx);

// Free the session context. Safe to call with null.
void pulsaar_destroy(struct PulsaarContext *ctx);

// Number of receivers found at init time. Returns 0 if ctx is null.
size_t pulsaar_get_receiver_count(const struct PulsaarContext *ctx);

// Fill out with info for the receiver at index (0-based).
PulsaarStatus pulsaar_get_receiver_info(const struct PulsaarContext *ctx, size_t index, CReceiverInfo *out);

// Open the receiver at index. Returns null on failure; status_out receives the error code.
// The caller must eventually call pulsaar_close_receiver.
struct PulsaarReceiverContext *pulsaar_open_receiver(struct PulsaarContext *ctx, size_t index, PulsaarStatus *status_out);

// Close an opened receiver and free its context. Safe to call with null.
void pulsaar_close_receiver(struct PulsaarReceiverContext *rctx);

// Fill out with properties of the opened receiver (serial, max_devices, etc.).
PulsaarStatus pulsaar_get_opened_receiver_info(const struct PulsaarReceiverContext *rctx, COpenedReceiverInfo *out);

// Enumerate devices paired to the receiver. Must be called before get_device_count/info.
PulsaarStatus pulsaar_enumerate_devices(struct PulsaarReceiverContext *rctx);

// Number of devices in the last pulsaar_enumerate_devices result. Returns 0 if rctx is null.
size_t pulsaar_get_device_count(const struct PulsaarReceiverContext *rctx);

// Fill out with info for the device at index in the cached device list.
PulsaarStatus pulsaar_get_device_info(const struct PulsaarReceiverContext *rctx, size_t index, CDeviceInfo *out);

// Unpair the device in slot from the opened receiver.
// slot: 1-based device slot number (from CDeviceInfo.slot).
PulsaarStatus pulsaar_unpair_device(struct PulsaarReceiverContext *rctx, uint8_t slot);

// Open the pairing lock (Unifying) or start device discovery (Bolt).
// timeout_secs: how long the receiver waits for a device (1-255).
// Call pulsaar_poll_pairing in a loop after this returns Ok.
PulsaarStatus pulsaar_start_pairing(struct PulsaarReceiverContext *rctx, uint8_t timeout_secs);

// Poll for one pairing event. Blocks for at most timeout_ms milliseconds.
// Call in a loop after pulsaar_start_pairing until out.state is Paired or Failed.
PulsaarStatus pulsaar_poll_pairing(struct PulsaarReceiverContext *rctx, uint32_t timeout_ms, CPairingStatus *out);

// Cancel an in-progress pairing. Closes the lock / stops discovery.
// Safe to call even if pairing is not in progress.
PulsaarStatus pulsaar_cancel_pairing(struct PulsaarReceiverContext *rctx);
