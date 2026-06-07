using System.Runtime.InteropServices;
using System.Text;

namespace Pulsaar.Core;

// ---------------------------------------------------------------------------
// Status codes
// ---------------------------------------------------------------------------

public enum PulsaarStatus : int
{
    Ok         = 0,
    HidError   = 1,
    Timeout    = 2,
    NoReceiver = 3,
    EmptySlot  = 4,
    InvalidArg = 5,
    Unknown    = 99,
}

// ---------------------------------------------------------------------------
// Pairing state
// ---------------------------------------------------------------------------

public enum PulsaarPairingState : int
{
    Waiting        = 0,
    DeviceFound    = 1,
    PasskeyNumeric = 2,
    PasskeyButton  = 3,
    Paired         = 4,
    Failed         = 5,
    Idle           = 6,
}

// ---------------------------------------------------------------------------
// Connection event type
// ---------------------------------------------------------------------------

public enum PulsaarConnectionEvent : int
{
    None            = 0,
    Online          = 1,
    Offline         = 2,
    SettingsChanged = 3,
}

// ---------------------------------------------------------------------------
// C-compatible structs (mirror #[repr(C)] structs in core/src/ffi.rs)
// Field layout must exactly match the C definitions in PulsaarCore-Bridging-Header.h.
// ---------------------------------------------------------------------------

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CBattery
{
    public byte level;     // 0-100, or 0xFF = unavailable
    public byte status;    // BatteryStatus byte, or 0xFF = unavailable
    public ushort voltage; // millivolts, or 0 = unavailable
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CReceiverInfo
{
    public ushort product_id;
    public byte kind;            // 0=Unifying 1=Bolt 2=Nano 3=LightSpeed
    public fixed byte name[64];  // null-terminated display name
    public fixed byte path[256]; // null-terminated OS HID path
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct COpenedReceiverInfo
{
    public ushort product_id;
    public byte kind;
    public byte max_devices;
    public fixed byte name[64];   // null-terminated display name
    public fixed byte serial[33]; // null-terminated serial
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CDeviceInfo
{
    public byte slot;
    public byte kind;
    public fixed byte wpid[2];
    public fixed byte name[64];   // null-terminated device name
    public fixed byte serial[32]; // null-terminated serial
    public byte has_battery;
    public CBattery battery;
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CDpiSettings
{
    public byte dpi_count;
    public fixed ushort dpi_list[200];
    public ushort current_dpi;
    public ushort default_dpi;
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CScrollSettings
{
    public byte has_invert;
    public byte has_hires;
    public byte inverted;
    public byte hires_enabled;
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CSmartShiftSettings
{
    public byte wheel_mode; // 0=absent, 1=freespin, 2=smart-shift
    public byte has_torque;
    public byte torque;     // 1-100
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CHostInfo
{
    public byte slot;
    public fixed byte name[64]; // null-terminated
    public byte is_active;
}

// C#'s `fixed` keyword only works with primitive element types, so CHostInfo hosts[8]
// is declared as 8 named fields -- the sequential layout is identical to C.
[StructLayout(LayoutKind.Sequential)]
public unsafe struct CHostList
{
    public byte count;
    public CHostInfo Host0, Host1, Host2, Host3, Host4, Host5, Host6, Host7;

    public CHostInfo GetHost(int index)
    {
        fixed (CHostList* self = &this)
            return (&self->Host0)[index];
    }
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CFnSettings
{
    public byte has_feature;
    public byte fn_swapped;
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CMultiplatformSettings
{
    public byte count;
    public byte current;
    public fixed byte platform_names[256]; // uint8_t platform_names[8][32]
    public fixed byte platform_indices[8];
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CBacklightSettings
{
    public byte has_feature;
    public byte mode;             // 0=disabled 1=automatic 3=manual
    public byte auto_supported;
    public byte manual_supported;
    public byte brightness;       // 0-100
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CAllDeviceSettings
{
    public CDpiSettings dpi;
    public CScrollSettings scroll;
    public CSmartShiftSettings ss;
    public CHostList hosts;
    public CFnSettings fn_s;
    public CMultiplatformSettings mp;
    public CBacklightSettings backlight;
    public byte reprog_controls_idx; // 0x1B04 feature index, or 0 if absent
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CDirectDeviceInfo
{
    public ushort product_id;
    public byte kind;
    public fixed byte name[64];   // null-terminated
    public fixed byte serial[64]; // null-terminated (may be empty)
    public byte has_battery;
    public CBattery battery;
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CPairingStatus
{
    public PulsaarPairingState state;
    public fixed byte device_name[64]; // valid for DeviceFound and Paired
    public fixed byte passkey[16];     // valid for PasskeyNumeric and PasskeyButton
    public fixed byte error[64];       // valid for Failed
}

[StructLayout(LayoutKind.Sequential)]
public unsafe struct CDeviceConnectionEvent
{
    public PulsaarConnectionEvent event_type; // renamed: 'event' is a C# keyword
    public byte slot;          // 1-based slot; 0 when event_type is None
    public byte feature_index; // for SettingsChanged: HID++ 2.0 feature index
}

// ---------------------------------------------------------------------------
// P/Invoke declarations (mirror all 24+ exports from core/src/ffi.rs)
// ---------------------------------------------------------------------------

public static unsafe class NativeInterop
{
    private const string Dll = "pulsaar_core.dll";
    private const CallingConvention CC = CallingConvention.Cdecl;

    // Session
    [DllImport(Dll, CallingConvention = CC)] public static extern nint pulsaar_init();
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_refresh_receivers(nint ctx);
    [DllImport(Dll, CallingConvention = CC)] public static extern void pulsaar_destroy(nint ctx);

    // Receiver enumeration (pre-open)
    [DllImport(Dll, CallingConvention = CC)] public static extern nuint pulsaar_get_receiver_count(nint ctx);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_receiver_info(nint ctx, nuint index, CReceiverInfo* out_info);

    // Receiver open / close
    [DllImport(Dll, CallingConvention = CC)] public static extern nint pulsaar_open_receiver(nint ctx, nuint index, PulsaarStatus* status_out);
    [DllImport(Dll, CallingConvention = CC)] public static extern void pulsaar_close_receiver(nint rctx);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_opened_receiver_info(nint rctx, COpenedReceiverInfo* out_info);

    // Device enumeration
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_enumerate_devices(nint rctx);
    [DllImport(Dll, CallingConvention = CC)] public static extern nuint pulsaar_get_device_count(nint rctx);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_device_info(nint rctx, nuint index, CDeviceInfo* out_info);

    // Unpair
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_unpair_device(nint rctx, byte slot);

    // Pairing
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_start_pairing(nint rctx, byte timeout_secs);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_poll_pairing(nint rctx, uint timeout_ms, CPairingStatus* out_status);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_cancel_pairing(nint rctx);

    // Event listeners
    [DllImport(Dll, CallingConvention = CC)] public static extern nint pulsaar_open_event_listener(nint ctx, nuint index, PulsaarStatus* status_out);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_poll_device_event(nint listener, uint timeout_ms, CDeviceConnectionEvent* out_event);
    [DllImport(Dll, CallingConvention = CC)] public static extern void pulsaar_close_event_listener(nint listener);

    // Direct (Bluetooth) devices
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_refresh_direct_devices(nint ctx);
    [DllImport(Dll, CallingConvention = CC)] public static extern nuint pulsaar_get_direct_device_count(nint ctx);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_direct_device_info(nint ctx, nuint index, CDirectDeviceInfo* out_info);

    // DPI (FEAT_ADJUSTABLE_DPI 0x2201)
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_dpi_settings(nint rctx, byte slot, CDpiSettings* out_settings);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_set_dpi(nint rctx, byte slot, ushort dpi);

    // Scroll wheel (FEAT_HIRES_WHEEL 0x2121)
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_scroll_settings(nint rctx, byte slot, CScrollSettings* out_settings);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_set_scroll_settings(nint rctx, byte slot, byte inverted, byte hires_enabled);

    // SmartShift (FEAT_SMART_SHIFT_ENHANCED 0x2111)
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_smartshift(nint rctx, byte slot, CSmartShiftSettings* out_settings);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_set_smartshift(nint rctx, byte slot, byte wheel_mode, byte torque);

    // Hosts (FEAT_CHANGE_HOST 0x1814 / FEAT_HOSTS_INFO 0x1815)
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_hosts(nint rctx, byte slot, CHostList* out_hosts);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_set_active_host(nint rctx, byte slot, byte host_slot);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_set_host_name(nint rctx, byte slot, byte host_slot, [MarshalAs(UnmanagedType.LPUTF8Str)] string name);

    // FN swap (FEAT_FN_INVERSION family)
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_fn_settings(nint rctx, byte slot, CFnSettings* out_settings);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_set_fn_swap(nint rctx, byte slot, byte swapped);

    // Multiplatform (FEAT_MULTIPLATFORM 0x4531)
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_multiplatform(nint rctx, byte slot, CMultiplatformSettings* out_settings);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_set_multiplatform(nint rctx, byte slot, byte platform_index);

    // Backlight (FEAT_BACKLIGHT2 0x1982)
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_backlight(nint rctx, byte slot, CBacklightSettings* out_settings);
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_set_backlight(nint rctx, byte slot, byte mode, byte brightness);

    // Batch settings read
    [DllImport(Dll, CallingConvention = CC)] public static extern PulsaarStatus pulsaar_get_all_settings(nint rctx, byte slot, CAllDeviceSettings* out_settings);

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    public static unsafe string CStringToString(byte* ptr, int maxLen)
    {
        var span = new ReadOnlySpan<byte>(ptr, maxLen);
        int len = span.IndexOf((byte)0);
        if (len < 0) len = maxLen;
        return Encoding.UTF8.GetString(span[..len]);
    }
}
