using System.Text;
using Pulsaar.Core;

namespace Pulsaar.Core;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

public enum ReceiverKind
{
    Unifying  = 0,
    Bolt      = 1,
    Nano      = 2,
    LightSpeed = 3,
}

public static class ReceiverKindExtensions
{
    public static string Label(this ReceiverKind k) => k switch
    {
        ReceiverKind.Bolt      => "Bolt",
        ReceiverKind.Nano      => "Nano",
        ReceiverKind.LightSpeed => "LightSpeed",
        _                      => "Unifying",
    };

    // Custom image asset name (bolt.svg, unifying.svg), or null for SF-only kinds.
    public static string? CustomImageName(this ReceiverKind k) => k switch
    {
        ReceiverKind.Bolt     => "bolt",
        ReceiverKind.Unifying => "unifying",
        _                     => null,
    };
}

public enum DeviceKind
{
    Unknown       = 0,
    Keyboard      = 1,
    Mouse         = 2,
    Numpad        = 3,
    Presenter     = 4,
    Remote        = 5,
    Trackball     = 6,
    Touchpad      = 7,
    Tablet        = 8,
    Gamepad       = 9,
    Joystick      = 10,
    Headset       = 11,
    RemoteControl = 12,
    Receiver      = 13,
}

public static class DeviceKindExtensions
{
    public static string Label(this DeviceKind k) => k switch
    {
        DeviceKind.Keyboard      => "Keyboard",
        DeviceKind.Mouse         => "Mouse",
        DeviceKind.Numpad        => "Numpad",
        DeviceKind.Presenter     => "Presenter",
        DeviceKind.Remote        => "Remote",
        DeviceKind.Trackball     => "Trackball",
        DeviceKind.Touchpad      => "Touchpad",
        DeviceKind.Tablet        => "Tablet",
        DeviceKind.Gamepad       => "Gamepad",
        DeviceKind.Joystick      => "Joystick",
        DeviceKind.Headset       => "Headset",
        DeviceKind.RemoteControl => "Remote Control",
        DeviceKind.Receiver      => "Receiver",
        _                        => "Unknown",
    };

    // Segoe Fluent Icons glyph for device type icon.
    public static string Glyph(this DeviceKind k) => k switch
    {
        DeviceKind.Keyboard or DeviceKind.Numpad   => "", // Keyboard
        DeviceKind.Mouse or DeviceKind.Trackball   => "", // Mouse
        DeviceKind.Headset                         => "", // Headset
        DeviceKind.Gamepad or DeviceKind.Joystick  => "", // Game
        DeviceKind.Touchpad                        => "", // Mouse (closest)
        DeviceKind.Presenter or DeviceKind.Remote
            or DeviceKind.RemoteControl            => "", // Remote
        DeviceKind.Tablet                          => "", // Tablet
        DeviceKind.Receiver                        => "", // Antenna
        _                                          => "", // Unknown
    };
}

public enum BatteryStatus
{
    Discharging    = 0,
    Recharging     = 1,
    AlmostFull     = 2,
    Full           = 3,
    SlowRecharge   = 4,
    InvalidBattery = 5,
    ThermalError   = 6,
}

public static class BatteryStatusExtensions
{
    public static bool IsCharging(this BatteryStatus s) =>
        s is BatteryStatus.Recharging or BatteryStatus.AlmostFull
          or BatteryStatus.Full or BatteryStatus.SlowRecharge;

    public static string Label(this BatteryStatus s) => s switch
    {
        BatteryStatus.Recharging     => "Charging",
        BatteryStatus.AlmostFull     => "Charging (almost full)",
        BatteryStatus.Full           => "Fully charged",
        BatteryStatus.SlowRecharge   => "Charging slowly",
        BatteryStatus.InvalidBattery => "Invalid battery",
        BatteryStatus.ThermalError   => "Thermal error",
        _                            => "Not charging",
    };
}

public enum WheelMode : byte
{
    Freespin  = 1,
    SmartShift = 2,
}

public static class WheelModeExtensions
{
    public static string Label(this WheelMode m) => m switch
    {
        WheelMode.Freespin => "Freespin",
        _                  => "Ratchet",
    };
}

public enum BacklightMode : byte
{
    Disabled  = 0,
    Automatic = 1,
    Manual    = 3,
}

public static class BacklightModeExtensions
{
    public static string Label(this BacklightMode m) => m switch
    {
        BacklightMode.Automatic => "Automatic",
        BacklightMode.Manual    => "Always on",
        _                       => "Off",
    };
}

// ---------------------------------------------------------------------------
// BatteryModel
// ---------------------------------------------------------------------------

public class BatteryModel
{
    public int? Level { get; }
    public BatteryStatus? Status { get; }
    public ushort? Voltage { get; }
    public bool IsCached { get; }

    private BatteryModel(int? level, BatteryStatus? status, ushort? voltage, bool isCached)
    {
        Level = level; Status = status; Voltage = voltage; IsCached = isCached;
    }

    public static unsafe BatteryModel FromLive(CBattery c) => new(
        c.level  == 0xFF ? null : (int?)c.level,
        c.status == 0xFF ? null : (BatteryStatus?)c.status,
        c.voltage == 0   ? null : (ushort?)c.voltage,
        false
    );

    public static BatteryModel FromCache(CachedBattery c) => new(
        c.Level,
        c.StatusByte.HasValue ? (BatteryStatus?)c.StatusByte.Value : null,
        c.Voltage,
        true
    );

    public string LevelText => Level.HasValue ? $"{Level}%" : "?";

    public bool IsCharging => Status?.IsCharging() ?? false;

    // Segoe Fluent Icons glyph codes for battery state.
    // Battery0=, Battery2=, Battery4=, Battery6=,
    // Battery10= (full), BatteryCharging=.
    public string BatteryGlyph
    {
        get
        {
            if (IsCharging) return "";
            return Level switch
            {
                >= 75 => "",
                >= 50 => "",
                >= 25 => "",
                >= 1  => "",
                _     => "",
            };
        }
    }
}

// ---------------------------------------------------------------------------
// DeviceModel
// ---------------------------------------------------------------------------

public class DeviceModel
{
    public string Id { get; }           // "receiverIndex-slot", stable across reloads
    public int ReceiverIndex { get; }
    public byte Slot { get; }
    public DeviceKind Kind { get; }
    public string Name { get; set; }
    public string Serial { get; }
    public string ProductId { get; }    // 0xXXXX from wpid
    public ReceiverKind ReceiverKind { get; }
    public BatteryModel? Battery { get; set; }

    // Tracks when the device was last confirmed online so the 60-second offline
    // hysteresis in Reload() can suppress false-offline states caused by a brief
    // inactivity sleep occurring just before or during a reload.
    public long LastSeenOnlineTick { get; private set; }

    private bool _isOnline;
    public bool IsOnline
    {
        get => _isOnline;
        set
        {
            _isOnline = value;
            if (value) LastSeenOnlineTick = Environment.TickCount64;
        }
    }

    public string ConnectionLabel => ReceiverKind switch
    {
        ReceiverKind.Bolt       => "Bolt (Encrypted)",
        ReceiverKind.LightSpeed => "LightSpeed",
        ReceiverKind.Nano       => "Nano",
        _                       => "Unifying",
    };

    public static unsafe DeviceModel FromC(CDeviceInfo* c, int receiverIndex, ReceiverKind receiverKind)
    {
        string name   = NativeInterop.CStringToString(c->name, 64);
        string serial = NativeInterop.CStringToString(c->serial, 32);
        string pid    = $"0x{c->wpid[0]:X2}{c->wpid[1]:X2}";
        BatteryModel? battery = c->has_battery != 0 ? BatteryModel.FromLive(c->battery) : null;

        return new DeviceModel(
            id:           $"{receiverIndex}-{c->slot}",
            receiverIndex: receiverIndex,
            slot:          c->slot,
            kind:          (DeviceKind)c->kind,
            name:          name,
            serial:        serial,
            productId:     pid,
            receiverKind:  receiverKind,
            battery:       battery,
            isOnline:      battery != null
        );
    }

    private DeviceModel(string id, int receiverIndex, byte slot, DeviceKind kind,
        string name, string serial, string productId, ReceiverKind receiverKind,
        BatteryModel? battery, bool isOnline)
    {
        Id = id; ReceiverIndex = receiverIndex; Slot = slot; Kind = kind;
        Name = name; Serial = serial; ProductId = productId; ReceiverKind = receiverKind;
        Battery = battery;
        IsOnline = isOnline;  // setter seeds LastSeenOnlineTick when true
    }
}

// ---------------------------------------------------------------------------
// DirectDeviceModel
// ---------------------------------------------------------------------------

public class DirectDeviceModel
{
    public string Id { get; }
    public ushort ProductId { get; }
    public DeviceKind Kind { get; }
    public string Name { get; }
    public string Serial { get; }
    public BatteryModel? Battery { get; }
    public bool IsOnline => true;
    public string ConnectionLabel => "Bluetooth";

    public static unsafe DirectDeviceModel FromC(CDirectDeviceInfo* c)
    {
        string serial = NativeInterop.CStringToString(c->serial, 64);
        string name   = NativeInterop.CStringToString(c->name, 64);
        string id     = serial.Length > 0 ? serial : $"direct-{c->product_id}";
        BatteryModel? battery = c->has_battery != 0 ? BatteryModel.FromLive(c->battery) : null;

        return new DirectDeviceModel(id, c->product_id, (DeviceKind)c->kind, name, serial, battery);
    }

    private DirectDeviceModel(string id, ushort productId, DeviceKind kind,
        string name, string serial, BatteryModel? battery)
    {
        Id = id; ProductId = productId; Kind = kind;
        Name = name; Serial = serial; Battery = battery;
    }
}

// ---------------------------------------------------------------------------
// Settings sub-types
// ---------------------------------------------------------------------------

public class HostInfo
{
    public byte Slot { get; }
    public string Name { get; }
    public bool IsActive { get; }

    public HostInfo(byte slot, string name, bool isActive)
    { Slot = slot; Name = name; IsActive = isActive; }
}

public class OSPlatform
{
    public byte Id { get; }   // raw platform_index from device
    public string Name { get; }

    public OSPlatform(byte id, string name) { Id = id; Name = name; }
}

// ---------------------------------------------------------------------------
// DeviceSettingsModel
// ---------------------------------------------------------------------------

public class DeviceSettingsModel
{
    // DPI (FEAT_ADJUSTABLE_DPI 0x2201)
    public List<int> DpiList { get; private set; } = [];
    public int CurrentDpi { get; set; }
    public int DefaultDpi { get; private set; }

    // Scroll wheel (FEAT_HIRES_WHEEL 0x2121)
    public bool HasInvert { get; private set; }
    public bool HasHires { get; private set; }
    public bool ScrollInverted { get; set; }
    public bool HiresEnabled { get; set; }

    // SmartShift (FEAT_SMART_SHIFT_ENHANCED 0x2111)
    public WheelMode? WheelMode { get; set; }
    public bool HasTorque { get; private set; }
    public int SmartShiftTorque { get; set; }

    // Change Host (FEAT_CHANGE_HOST 0x1814)
    public List<HostInfo>? Hosts { get; set; }

    // FN swap (FEAT_FN_INVERSION family)
    public bool? FnSwapped { get; set; }

    // Multiplatform (FEAT_MULTIPLATFORM 0x4531)
    public List<OSPlatform>? Platforms { get; set; }
    public int CurrentOsIdx { get; set; }

    // Backlight (FEAT_BACKLIGHT2 0x1982)
    public BacklightMode? BacklightMode { get; set; }
    public bool BacklightAutoSupported { get; private set; }
    public bool BacklightManualSupported { get; private set; }
    public int BacklightBrightness { get; set; }

    // Feature index of REPROG_CONTROLS_V4 (0x1B04), or 0 if absent.
    public byte ReprogControlsIdx { get; private set; }

    public bool HasDpi           => DpiList.Count > 0;
    public bool HasScrollSettings => HasInvert || HasHires;
    public bool HasSmartShift    => WheelMode.HasValue;
    public bool HasHosts         => Hosts?.Count > 0;
    public bool HasFnSwap        => FnSwapped.HasValue;
    public bool HasMultiplatform => Platforms?.Count > 0;
    public bool HasBacklight     => BacklightMode.HasValue;
    public bool HasAnySettings   => HasDpi || HasScrollSettings || HasSmartShift
                                 || HasHosts || HasFnSwap || HasMultiplatform || HasBacklight;

    public static unsafe DeviceSettingsModel? FromAllSettings(CAllDeviceSettings* s)
    {
        var m = new DeviceSettingsModel();

        // DPI
        if (s->dpi.dpi_count > 0)
        {
            var list = new List<int>(s->dpi.dpi_count);
            for (int i = 0; i < s->dpi.dpi_count; i++)
                list.Add(s->dpi.dpi_list[i]);
            m.DpiList    = list;
            m.CurrentDpi = s->dpi.current_dpi;
            m.DefaultDpi = s->dpi.default_dpi;
        }

        // Scroll wheel
        m.HasInvert      = s->scroll.has_invert    != 0;
        m.HasHires       = s->scroll.has_hires     != 0;
        m.ScrollInverted = s->scroll.inverted      != 0;
        m.HiresEnabled   = s->scroll.hires_enabled != 0;

        // SmartShift
        if (s->ss.wheel_mode != 0)
        {
            m.WheelMode        = (WheelMode)s->ss.wheel_mode;
            m.HasTorque        = s->ss.has_torque != 0;
            m.SmartShiftTorque = s->ss.torque;
        }
        else
        {
            m.SmartShiftTorque = 50;
        }

        // Hosts -- use pointer arithmetic through s->hosts to avoid value-copy of fixed arrays.
        if (s->hosts.count > 0)
        {
            var hosts = new List<HostInfo>(s->hosts.count);
            CHostInfo* hostArr = &s->hosts.Host0;
            for (int i = 0; i < s->hosts.count; i++)
            {
                CHostInfo* h = hostArr + i;
                string name = NativeInterop.CStringToString(h->name, 64);
                hosts.Add(new HostInfo(h->slot, name, h->is_active != 0));
            }
            m.Hosts = hosts;
        }

        // FN swap: null = feature absent
        m.FnSwapped = s->fn_s.has_feature != 0 ? (bool?)(s->fn_s.fn_swapped != 0) : null;

        // Multiplatform: only expose if 2+ platforms (consistent with macOS behavior)
        if (s->mp.count >= 2)
        {
            var platforms = new List<OSPlatform>(s->mp.count);
            for (int i = 0; i < s->mp.count; i++)
            {
                byte* namePtr = s->mp.platform_names + i * 32;
                string name = NativeInterop.CStringToString(namePtr, 32);
                platforms.Add(new OSPlatform(s->mp.platform_indices[i], name));
            }
            m.Platforms   = platforms;
            m.CurrentOsIdx = s->mp.current;
        }

        // Backlight: null = feature absent
        if (s->backlight.has_feature != 0)
        {
            m.BacklightMode           = (BacklightMode)s->backlight.mode;
            m.BacklightAutoSupported   = s->backlight.auto_supported != 0;
            m.BacklightManualSupported = s->backlight.manual_supported != 0;
            m.BacklightBrightness      = s->backlight.brightness;
        }

        m.ReprogControlsIdx = s->reprog_controls_idx;

        return m.HasAnySettings ? m : null;
    }
}

// ---------------------------------------------------------------------------
// ReceiverModel
// ---------------------------------------------------------------------------

public class ReceiverModel
{
    public int Id { get; }          // index within the session's receiver list
    public ushort ProductId { get; }
    public ReceiverKind Kind { get; }
    public string Name { get; }
    public string Serial { get; }
    public byte MaxDevices { get; }
    public List<DeviceModel> Devices { get; }

    public static unsafe ReceiverModel FromOpened(int index, COpenedReceiverInfo* opened, List<DeviceModel> devices)
    {
        string name   = NativeInterop.CStringToString(opened->name, 64);
        string serial = NativeInterop.CStringToString(opened->serial, 33);
        return new ReceiverModel(index, opened->product_id, (ReceiverKind)opened->kind,
            name, serial, opened->max_devices, devices);
    }

    private ReceiverModel(int id, ushort productId, ReceiverKind kind,
        string name, string serial, byte maxDevices, List<DeviceModel> devices)
    {
        Id = id; ProductId = productId; Kind = kind;
        Name = name; Serial = serial; MaxDevices = maxDevices; Devices = devices;
    }
}
