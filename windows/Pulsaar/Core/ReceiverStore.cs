using System.Diagnostics;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Windows.Devices.Enumeration;
using Pulsaar.Core;

namespace Pulsaar.Core;

// ---------------------------------------------------------------------------
// Pairing stage (app-level enum, separate from the FFI PulsaarPairingState)
// ---------------------------------------------------------------------------

public enum PairingStage
{
    None,
    Waiting,
    DeviceFound,
    PasskeyNumeric,
    PasskeyButton,
    Paired,
    Failed,
}

// ---------------------------------------------------------------------------
// ReceiverStore -- ObservableObject that owns the Rust HID session
// Mirrors ReceiverStore.swift exactly in behavior.
// ---------------------------------------------------------------------------

public partial class ReceiverStore : ObservableObject
{
    // Observable properties (CommunityToolkit.Mvvm generates the public Property + PropertyChanged)
    [ObservableProperty] private List<ReceiverModel> _receivers = [];
    [ObservableProperty] private List<DirectDeviceModel> _directDevices = [];
    [ObservableProperty] private bool _isLoading;
    [ObservableProperty] private bool _isPrefetching;
    [ObservableProperty] private string? _errorMessage;
    [ObservableProperty] private string? _toastMessage;
    [ObservableProperty] private int _settingsCacheVersion;

    // Pairing state
    [ObservableProperty] private PairingStage _pairingStage;
    [ObservableProperty] private string _pairingDeviceName = "";
    [ObservableProperty] private string _pairingPasskey = "";
    [ObservableProperty] private bool _pairingPasskeyIsNumeric;
    [ObservableProperty] private string _pairingError = "";

    // Non-observable HID state
    private nint _ctx;
    private nint _pairingRctx;
    private readonly List<nint> _eventListeners = [];
    private readonly Dictionary<string, DeviceSettingsModel> _settingsCache = [];
    private readonly DeviceCache _deviceCache = new();
    private readonly DispatcherQueue _uiQueue;

    // Timers and watchers
    private DispatcherTimer? _eventPollTimer;
    private DispatcherTimer? _pairingTimer;
    private DispatcherTimer? _settingsDebounceTimer;
    private DispatcherTimer? _usbConnectTimer;
    private DeviceWatcher? _deviceWatcher;
    private bool _isEventPolling;
    private bool _isPairingPollRunning;
    private (byte Slot, int ReceiverIndex) _pendingSettingsRefresh;

    // ---------------------------------------------------------------------------
    // Init / destroy
    // ---------------------------------------------------------------------------

    public ReceiverStore(DispatcherQueue uiQueue)
    {
        _uiQueue = uiQueue;
        _ctx = NativeInterop.pulsaar_init();
        Debug.WriteLine($"[PULSAAR][STORE] pulsaar_init -> 0x{_ctx:X}");

        if (_ctx == nint.Zero)
        {
            ErrorMessage = "Failed to initialize HID session.";
            return;
        }

        StartUsbMonitoring();
        _ = Reload(false);

        // Prefetch settings 1s after init (same delay as macOS)
        _uiQueue.TryEnqueue(async () =>
        {
            await Task.Delay(1000);
            await PrefetchSettings();
        });
    }

    public void Dispose()
    {
        StopEventListeners();
        CancelPairing();
        _deviceWatcher?.Stop();
        if (_ctx != nint.Zero)
        {
            NativeInterop.pulsaar_destroy(_ctx);
            _ctx = nint.Zero;
        }
        Debug.WriteLine("[PULSAAR][STORE] destroyed");
    }

    // ---------------------------------------------------------------------------
    // USB monitoring (mirrors IOHIDManager in macOS)
    // ---------------------------------------------------------------------------

    private void StartUsbMonitoring()
    {
        // HID interface class GUID -- fires on any HID connect/disconnect.
        // The 3s debounce on connect and actual pulsaar_refresh_receivers handle false positives.
        const string selector =
            "System.Devices.InterfaceClassGuid:=\"{4D1E55B2-F16F-11CF-88CB-001111000030}\"";

        _deviceWatcher = DeviceInformation.CreateWatcher(selector);
        _deviceWatcher.Added   += (_, _) => ScheduleUsbConnectReload();
        _deviceWatcher.Removed += (_, _) =>
        {
            _uiQueue.TryEnqueue(() => _ = Reload(false));
        };
        _deviceWatcher.Start();
        Debug.WriteLine("[PULSAAR][USB] DeviceWatcher started");
    }

    private void ScheduleUsbConnectReload()
    {
        // 3s debounce -- same as macOS to absorb startup matching callbacks
        _uiQueue.TryEnqueue(() =>
        {
            _usbConnectTimer?.Stop();
            _usbConnectTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(3) };
            _usbConnectTimer.Tick += (s, _) =>
            {
                ((DispatcherTimer)s!).Stop();
                if (!IsPrefetching)
                {
                    Debug.WriteLine("[PULSAAR][USB] debounced connect reload");
                    _ = Reload(false);
                }
            };
            _usbConnectTimer.Start();
        });
    }

    // ---------------------------------------------------------------------------
    // Reload (enumerate receivers + devices)
    // ---------------------------------------------------------------------------

    public async Task Reload(bool showIndicator)
    {
        Debug.WriteLine("[PULSAAR][STORE] Reload");
        if (showIndicator) IsLoading = true;

        StopEventListeners();

        var newReceivers     = new List<ReceiverModel>();
        var newDirectDevices = new List<DirectDeviceModel>();
        string? error        = null;

        await Task.Run(() =>
        {
            unsafe
            {
                var status = NativeInterop.pulsaar_refresh_receivers(_ctx);
                if (status != PulsaarStatus.Ok)
                {
                    error = $"Receiver refresh failed: {status}";
                    return;
                }

                nuint receiverCount = NativeInterop.pulsaar_get_receiver_count(_ctx);
                for (nuint ri = 0; ri < receiverCount; ri++)
                {
                    var rctx = default(nint);
                    PulsaarStatus openStatus;
                    rctx = NativeInterop.pulsaar_open_receiver(_ctx, ri, &openStatus);
                    if (rctx == nint.Zero)
                    {
                        Debug.WriteLine($"[PULSAAR][STORE] receiver {ri} open failed: {openStatus}");
                        continue;
                    }

                    COpenedReceiverInfo openedInfo;
                    NativeInterop.pulsaar_get_opened_receiver_info(rctx, &openedInfo);

                    NativeInterop.pulsaar_enumerate_devices(rctx);
                    nuint deviceCount = NativeInterop.pulsaar_get_device_count(rctx);

                    var devices = new List<DeviceModel>((int)deviceCount);
                    for (nuint di = 0; di < deviceCount; di++)
                    {
                        CDeviceInfo info;
                        if (NativeInterop.pulsaar_get_device_info(rctx, di, &info) != PulsaarStatus.Ok)
                            continue;

                        var device = DeviceModel.FromC(&info, (int)ri, (ReceiverKind)openedInfo.kind);

                        // Inject cached name for offline devices
                        if (!device.IsOnline)
                        {
                            var cachedName = _deviceCache.Name(device.Serial);
                            if (cachedName != null) device.Name = cachedName;
                        }
                        else
                        {
                            _deviceCache.UpdateName(device.Serial, device.Name);
                        }

                        // Inject cached battery for offline devices
                        if (!device.IsOnline)
                        {
                            device.Battery = _deviceCache.Battery(device.Serial);
                        }
                        else if (device.Battery != null)
                        {
                            _deviceCache.UpdateBattery(device.Serial, device.Battery);
                        }

                        devices.Add(device);
                    }

                    NativeInterop.pulsaar_close_receiver(rctx);

                    var receiver = ReceiverModel.FromOpened((int)ri, &openedInfo, devices);
                    newReceivers.Add(receiver);

                    Debug.WriteLine($"[PULSAAR][STORE] receiver {ri}: {receiver.Name} ({deviceCount} devices)");
                }

                // Direct (Bluetooth) devices
                NativeInterop.pulsaar_refresh_direct_devices(_ctx);
                nuint directCount = NativeInterop.pulsaar_get_direct_device_count(_ctx);
                for (nuint di = 0; di < directCount; di++)
                {
                    CDirectDeviceInfo info;
                    if (NativeInterop.pulsaar_get_direct_device_info(_ctx, di, &info) != PulsaarStatus.Ok)
                        continue;
                    newDirectDevices.Add(DirectDeviceModel.FromC(&info));
                }
            }
        });

        Receivers     = newReceivers;
        DirectDevices = newDirectDevices;
        ErrorMessage  = error;
        IsLoading     = false;

        RestartEventListeners();
    }

    // ---------------------------------------------------------------------------
    // Event listeners
    // ---------------------------------------------------------------------------

    private void StopEventListeners()
    {
        _eventPollTimer?.Stop();
        _eventPollTimer = null;

        unsafe
        {
            foreach (var listener in _eventListeners)
                NativeInterop.pulsaar_close_event_listener(listener);
        }
        _eventListeners.Clear();
        Debug.WriteLine("[PULSAAR][EVENTS] listeners stopped");
    }

    private void RestartEventListeners()
    {
        StopEventListeners();

        unsafe
        {
            for (nuint i = 0; i < (nuint)Receivers.Count; i++)
            {
                PulsaarStatus status;
                var listener = NativeInterop.pulsaar_open_event_listener(_ctx, i, &status);
                if (listener == nint.Zero)
                {
                    Debug.WriteLine($"[PULSAAR][EVENTS] open listener {i} failed: {status}");
                    _eventListeners.Add(nint.Zero);
                }
                else
                {
                    _eventListeners.Add(listener);
                }
            }
        }

        if (_eventListeners.Count == 0) return;

        _eventPollTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(250) };
        _eventPollTimer.Tick += OnEventPollTick;
        _eventPollTimer.Start();
        Debug.WriteLine($"[PULSAAR][EVENTS] {_eventListeners.Count} listener(s) started");
    }

    private void PauseEventPolling() => _eventPollTimer?.Stop();
    private void ResumeEventPolling() => _eventPollTimer?.Start();

    private async void OnEventPollTick(object? sender, object e)
    {
        if (_isEventPolling) return;
        _isEventPolling = true;
        try { await Task.Run(PollEventListeners); }
        finally { _isEventPolling = false; }
    }

    private void PollEventListeners()
    {
        var snapshot = _eventListeners.ToList(); // local copy for thread safety
        for (int i = 0; i < snapshot.Count; i++)
        {
            if (snapshot[i] == nint.Zero) continue;

            CDeviceConnectionEvent evRaw;
            unsafe { NativeInterop.pulsaar_poll_device_event(snapshot[i], 200, &evRaw); }

            if (evRaw.event_type == PulsaarConnectionEvent.None) continue;

            // evRaw had its address taken; copy to a fresh local so the lambda can capture it (CS1686).
            var ev = evRaw;
            int receiverIndex = i;
            _uiQueue.TryEnqueue(() => HandleDeviceEvent(ev, receiverIndex));
        }
    }

    private void HandleDeviceEvent(CDeviceConnectionEvent ev, int receiverIndex)
    {
        Debug.WriteLine($"[PULSAAR][EVENTS] slot={ev.slot} event={ev.event_type} feature={ev.feature_index}");

        if (ev.event_type is PulsaarConnectionEvent.Online or PulsaarConnectionEvent.Offline)
        {
            _ = Reload(false);
            return;
        }

        if (ev.event_type == PulsaarConnectionEvent.SettingsChanged)
        {
            // Discard persistent REPROG_CONTROLS_V4 (0x1B04) button events.
            var receiver = Receivers.ElementAtOrDefault(receiverIndex);
            var device   = receiver?.Devices.FirstOrDefault(d => d.Slot == ev.slot);
            if (device != null &&
                _settingsCache.TryGetValue(device.Id, out var cached) &&
                cached.ReprogControlsIdx != 0 &&
                ev.feature_index == cached.ReprogControlsIdx)
            {
                Debug.WriteLine("[PULSAAR][EVENTS] discarding button-CID event");
                return;
            }

            ScheduleSettingsRefresh(ev.slot, receiverIndex);
        }
    }

    // ---------------------------------------------------------------------------
    // Settings refresh (750ms debounce, mirrors macOS scheduleSettingsRefresh)
    // ---------------------------------------------------------------------------

    private void ScheduleSettingsRefresh(byte slot, int receiverIndex)
    {
        _pendingSettingsRefresh = (slot, receiverIndex);
        _settingsDebounceTimer?.Stop();
        _settingsDebounceTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(0.75) };
        _settingsDebounceTimer.Tick += (s, _) =>
        {
            ((DispatcherTimer)s!).Stop();
            var (sl, ri) = _pendingSettingsRefresh;
            _ = Task.Run(() => DoSettingsRefresh(sl, ri));
        };
        _settingsDebounceTimer.Start();
    }

    private void DoSettingsRefresh(byte slot, int receiverIndex)
    {
        // Called on a background thread from Task.Run in the debounce timer tick.
        // On Windows there is no exclusive-access restriction, so we do not need to
        // stop event listeners here -- the Rust core can hold multiple handles simultaneously.
        var receiver = Receivers.ElementAtOrDefault(receiverIndex);
        var device   = receiver?.Devices.FirstOrDefault(d => d.Slot == slot);
        if (device == null) return;

        Debug.WriteLine($"[PULSAAR][SETTINGS] refresh slot {slot} receiver {receiverIndex}");

        DeviceSettingsModel? settings = null;
        unsafe
        {
            PulsaarStatus openStatus;
            var rctx = NativeInterop.pulsaar_open_receiver(_ctx, (nuint)receiverIndex, &openStatus);
            if (rctx == nint.Zero) return;
            try
            {
                CAllDeviceSettings raw;
                if (NativeInterop.pulsaar_get_all_settings(rctx, slot, &raw) == PulsaarStatus.Ok)
                    settings = DeviceSettingsModel.FromAllSettings(&raw);
            }
            finally { NativeInterop.pulsaar_close_receiver(rctx); }
        }

        _uiQueue.TryEnqueue(() =>
        {
            if (settings != null) _settingsCache[device.Id] = settings;
            SettingsCacheVersion++;
        });
    }

    // ---------------------------------------------------------------------------
    // Prefetch settings (batch read at startup and after pairing)
    // ---------------------------------------------------------------------------

    public async Task PrefetchSettings()
    {
        if (IsPrefetching) return;
        IsPrefetching = true;

        // Stop listeners on UI thread before handing off to background.
        // On Windows there is no exclusive-access restriction (unlike macOS),
        // but we still stop to avoid a double-open on the same receiver handle.
        StopEventListeners();

        Debug.WriteLine("[PULSAAR][SETTINGS] prefetch start");

        // Snapshot receiver list so the background thread doesn't race with Reload.
        var snapshot = Receivers.ToList();

        await Task.Run(() =>
        {
            unsafe
            {
                for (int ri = 0; ri < snapshot.Count; ri++)
                {
                    PulsaarStatus openStatus;
                    var rctx = NativeInterop.pulsaar_open_receiver(_ctx, (nuint)ri, &openStatus);
                    if (rctx == nint.Zero)
                    {
                        Debug.WriteLine($"[PULSAAR][SETTINGS] prefetch receiver {ri} open failed: {openStatus}");
                        continue;
                    }

                    try
                    {
                        foreach (var device in snapshot[ri].Devices)
                        {
                            CAllDeviceSettings raw;
                            if (NativeInterop.pulsaar_get_all_settings(rctx, device.Slot, &raw) != PulsaarStatus.Ok)
                                continue;

                            var settings = DeviceSettingsModel.FromAllSettings(&raw);
                            if (settings != null)
                            {
                                _settingsCache[device.Id] = settings;
                                Debug.WriteLine($"[PULSAAR][SETTINGS] cached {device.Name}");
                            }

                            // Write machine hostname to the active host slot.
                            if (raw.hosts.count > 0)
                            {
                                string name = Environment.MachineName;
                                for (int hi = 0; hi < raw.hosts.count; hi++)
                                {
                                    CHostInfo h = raw.hosts.GetHost(hi);
                                    if (h.is_active != 0)
                                    {
                                        NativeInterop.pulsaar_set_host_name(rctx, device.Slot, h.slot, name);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    finally
                    {
                        NativeInterop.pulsaar_close_receiver(rctx);
                    }
                }
            }
        });

        // Back on UI thread (synchronization context restores it after Task.Run await).
        IsPrefetching = false;
        SettingsCacheVersion++;
        RestartEventListeners();
        Debug.WriteLine("[PULSAAR][SETTINGS] prefetch complete");
    }

    // ---------------------------------------------------------------------------
    // Load settings for a single device (called from the detail view task)
    // ---------------------------------------------------------------------------

    public async Task<DeviceSettingsModel?> LoadSettings(DeviceModel device)
    {
        // Serve from cache if prefetch already populated it
        if (_settingsCache.TryGetValue(device.Id, out var cached)) return cached;

        // Wait up to 6s for prefetch to finish (matches macOS isPrefetching guard)
        for (int i = 0; i < 12 && IsPrefetching; i++)
            await Task.Delay(500);

        if (_settingsCache.TryGetValue(device.Id, out cached)) return cached;

        DeviceSettingsModel? settings = null;
        await Task.Run(() =>
        {
            unsafe
            {
                PulsaarStatus openStatus;
                var rctx = NativeInterop.pulsaar_open_receiver(_ctx, (nuint)device.ReceiverIndex, &openStatus);
                if (rctx == nint.Zero) return;
                try
                {
                    CAllDeviceSettings raw;
                    if (NativeInterop.pulsaar_get_all_settings(rctx, device.Slot, &raw) == PulsaarStatus.Ok)
                        settings = DeviceSettingsModel.FromAllSettings(&raw);
                }
                finally { NativeInterop.pulsaar_close_receiver(rctx); }
            }
        });

        if (settings != null) _settingsCache[device.Id] = settings;
        return settings;
    }

    public Dictionary<string, DeviceSettingsModel> SettingsCache => _settingsCache;

    // ---------------------------------------------------------------------------
    // Settings writes (write + patch cache + toast)
    // ---------------------------------------------------------------------------

    public async Task SetDpi(DeviceModel device, ushort dpi)
    {
        bool ok = await WriteToDevice(device, (rctx) =>
        {
            unsafe { return NativeInterop.pulsaar_set_dpi(rctx, device.Slot, dpi); }
        }, "set DPI");

        if (ok && _settingsCache.TryGetValue(device.Id, out var s))
        {
            s.CurrentDpi = dpi;
            ShowToast($"DPI set to {dpi}");
        }
    }

    public async Task SetScrollSettings(DeviceModel device, bool inverted, bool hiresEnabled)
    {
        bool ok = await WriteToDevice(device, (rctx) =>
        {
            unsafe { return NativeInterop.pulsaar_set_scroll_settings(rctx, device.Slot, inverted ? (byte)1 : (byte)0, hiresEnabled ? (byte)1 : (byte)0); }
        }, "set scroll");

        if (ok && _settingsCache.TryGetValue(device.Id, out var s))
        {
            s.ScrollInverted = inverted;
            s.HiresEnabled   = hiresEnabled;
            ShowToast("Scroll settings updated");
        }
    }

    public async Task SetSmartShift(DeviceModel device, WheelMode wheelMode, int torque)
    {
        bool ok = await WriteToDevice(device, (rctx) =>
        {
            unsafe { return NativeInterop.pulsaar_set_smartshift(rctx, device.Slot, (byte)wheelMode, (byte)torque); }
        }, "set smartshift");

        if (ok && _settingsCache.TryGetValue(device.Id, out var s))
        {
            s.WheelMode        = wheelMode;
            s.SmartShiftTorque = torque;
            ShowToast("SmartShift updated");
        }
    }

    public async Task SetActiveHost(DeviceModel device, byte hostSlot)
    {
        bool ok = await WriteToDevice(device, (rctx) =>
        {
            unsafe { return NativeInterop.pulsaar_set_active_host(rctx, device.Slot, hostSlot); }
        }, "set active host");

        if (ok && _settingsCache.TryGetValue(device.Id, out var s) && s.Hosts != null)
        {
            s.Hosts = s.Hosts.Select(h => new HostInfo(h.Slot, h.Name, h.Slot == hostSlot)).ToList();
            ShowToast("Host changed");
        }
    }

    public async Task SetFnSwap(DeviceModel device, bool swapped)
    {
        bool ok = await WriteToDevice(device, (rctx) =>
        {
            unsafe { return NativeInterop.pulsaar_set_fn_swap(rctx, device.Slot, swapped ? (byte)1 : (byte)0); }
        }, "set FN swap");

        if (ok && _settingsCache.TryGetValue(device.Id, out var s))
        {
            s.FnSwapped = swapped;
            ShowToast(swapped ? "FN keys swapped" : "FN keys restored");
        }
    }

    public async Task SetMultiplatform(DeviceModel device, byte platformIndex)
    {
        bool ok = await WriteToDevice(device, (rctx) =>
        {
            unsafe { return NativeInterop.pulsaar_set_multiplatform(rctx, device.Slot, platformIndex); }
        }, "set OS");

        if (ok && _settingsCache.TryGetValue(device.Id, out var s) && s.Platforms != null)
        {
            s.CurrentOsIdx = s.Platforms.FindIndex(p => p.Id == platformIndex);
            ShowToast("OS layout updated");
        }
    }

    public async Task SetBacklight(DeviceModel device, BacklightMode mode, int brightness)
    {
        bool ok = await WriteToDevice(device, (rctx) =>
        {
            unsafe { return NativeInterop.pulsaar_set_backlight(rctx, device.Slot, (byte)mode, (byte)brightness); }
        }, "set backlight");

        if (ok && _settingsCache.TryGetValue(device.Id, out var s))
        {
            s.BacklightMode       = mode;
            s.BacklightBrightness = brightness;
            ShowToast("Backlight updated");
        }
    }

    // ---------------------------------------------------------------------------
    // Unpair
    // ---------------------------------------------------------------------------

    public async Task Unpair(DeviceModel device)
    {
        StopEventListeners();
        bool ok = await WriteToDevice(device, (rctx) =>
        {
            unsafe { return NativeInterop.pulsaar_unpair_device(rctx, device.Slot); }
        }, "unpair");

        if (ok)
        {
            Debug.WriteLine($"[PULSAAR][STORE] unpaired {device.Name}");
            await Reload(false);
        }
        else
        {
            RestartEventListeners();
        }
    }

    // ---------------------------------------------------------------------------
    // Pairing
    // ---------------------------------------------------------------------------

    public void StartPairing(int receiverIndex, byte timeoutSecs = 60)
    {
        PauseEventPolling();
        PairingStage  = PairingStage.Waiting;
        PairingDeviceName = "";
        PairingPasskey    = "";
        PairingError      = "";

        _ = Task.Run(() =>
        {
            unsafe
            {
                PulsaarStatus openStatus;
                var rctx = NativeInterop.pulsaar_open_receiver(_ctx, (nuint)receiverIndex, &openStatus);
                if (rctx == nint.Zero)
                {
                    // openStatus had its address taken; copy before the lambda captures it (CS1686).
                    var capturedStatus = openStatus;
                    _uiQueue.TryEnqueue(() =>
                    {
                        PairingStage = PairingStage.Failed;
                        PairingError = $"Could not open receiver ({capturedStatus})";
                        ResumeEventPolling();
                    });
                    return;
                }

                var status = NativeInterop.pulsaar_start_pairing(rctx, timeoutSecs);
                _uiQueue.TryEnqueue(() =>
                {
                    if (status != PulsaarStatus.Ok)
                    {
                        NativeInterop.pulsaar_close_receiver(rctx);
                        PairingStage = PairingStage.Failed;
                        PairingError = $"Start pairing failed ({status})";
                        ResumeEventPolling();
                        return;
                    }
                    _pairingRctx = rctx;
                    StartPairingPollTimer();
                });
            }
        });
    }

    private void StartPairingPollTimer()
    {
        _pairingTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(250) };
        _pairingTimer.Tick += OnPairingPollTick;
        _pairingTimer.Start();
    }

    private async void OnPairingPollTick(object? sender, object e)
    {
        if (_isPairingPollRunning) return;
        _isPairingPollRunning = true;
        try { await Task.Run(DoPollPairing); }
        finally { _isPairingPollRunning = false; }
    }

    private void DoPollPairing()
    {
        if (_pairingRctx == nint.Zero) return;

        // Extract all strings while the raw struct is on this stack frame.
        // Never capture the CPairingStatus struct itself in a lambda -- by the time
        // the UI thread runs the lambda, this stack frame may be gone.
        PulsaarPairingState state;
        string deviceName, passkey, error;

        unsafe
        {
            CPairingStatus raw;
            NativeInterop.pulsaar_poll_pairing(_pairingRctx, 200, &raw);
            CPairingStatus* p = &raw;
            state      = p->state;
            deviceName = NativeInterop.CStringToString(p->device_name, 64);
            passkey    = NativeInterop.CStringToString(p->passkey, 16);
            error      = NativeInterop.CStringToString(p->error, 64);
        }

        _uiQueue.TryEnqueue(() => HandlePairingStatus(state, deviceName, passkey, error));
    }

    private void HandlePairingStatus(PulsaarPairingState state, string deviceName, string passkey, string error)
    {
        switch (state)
        {
            case PulsaarPairingState.Waiting:
                PairingStage = PairingStage.Waiting;
                break;

            case PulsaarPairingState.DeviceFound:
                PairingStage      = PairingStage.DeviceFound;
                PairingDeviceName = deviceName;
                break;

            case PulsaarPairingState.PasskeyNumeric:
                PairingStage            = PairingStage.PasskeyNumeric;
                PairingPasskey          = passkey;
                PairingPasskeyIsNumeric = true;
                break;

            case PulsaarPairingState.PasskeyButton:
                PairingStage            = PairingStage.PasskeyButton;
                PairingPasskey          = passkey;
                PairingPasskeyIsNumeric = false;
                break;

            case PulsaarPairingState.Paired:
                PairingStage      = PairingStage.Paired;
                PairingDeviceName = deviceName;
                FinalizePairing();
                break;

            case PulsaarPairingState.Failed:
                PairingStage = PairingStage.Failed;
                PairingError = error;
                AbortPairing();
                break;
        }
    }

    private void FinalizePairing()
    {
        _pairingTimer?.Stop();
        _pairingTimer = null;

        var rctx = _pairingRctx;
        _pairingRctx = nint.Zero;

        _ = Task.Run(async () =>
        {
            unsafe { NativeInterop.pulsaar_close_receiver(rctx); }
            await Task.Delay(750);
            _uiQueue.TryEnqueue(async () =>
            {
                await Reload(false);
                await PrefetchSettings();
            });
        });
    }

    private void AbortPairing()
    {
        _pairingTimer?.Stop();
        _pairingTimer = null;

        var rctx = _pairingRctx;
        _pairingRctx = nint.Zero;

        if (rctx != nint.Zero)
            _ = Task.Run(() => { unsafe { NativeInterop.pulsaar_close_receiver(rctx); } });

        ResumeEventPolling();
    }

    public void CancelPairing()
    {
        if (_pairingRctx == nint.Zero) return;
        _ = Task.Run(() =>
        {
            unsafe { NativeInterop.pulsaar_cancel_pairing(_pairingRctx); }
        });
        AbortPairing();
        PairingStage = PairingStage.None;
    }

    // ---------------------------------------------------------------------------
    // Toast
    // ---------------------------------------------------------------------------

    private DispatcherTimer? _toastTimer;

    private void ShowToast(string message)
    {
        ToastMessage = message;
        _toastTimer?.Stop();
        _toastTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2) };
        _toastTimer.Tick += (s, _) =>
        {
            ((DispatcherTimer)s!).Stop();
            ToastMessage = null;
        };
        _toastTimer.Start();
    }

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    private async Task<bool> WriteToDevice(DeviceModel device, Func<nint, PulsaarStatus> write, string label)
    {
        bool success = false;
        await Task.Run(() =>
        {
            unsafe
            {
                PulsaarStatus openStatus;
                var rctx = NativeInterop.pulsaar_open_receiver(_ctx, (nuint)device.ReceiverIndex, &openStatus);
                if (rctx == nint.Zero)
                {
                    Debug.WriteLine($"[PULSAAR][WRITE] {label}: open failed {openStatus}");
                    return;
                }
                try
                {
                    var status = write(rctx);
                    success = status == PulsaarStatus.Ok;
                    Debug.WriteLine($"[PULSAAR][WRITE] {label}: {status}");
                }
                finally { NativeInterop.pulsaar_close_receiver(rctx); }
            }
        });
        return success;
    }
}
