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
    private bool _reloadInProgress;
    private int  _reloadCount;
    private readonly List<nint> _eventListeners = [];
    private readonly Dictionary<string, DeviceSettingsModel> _settingsCache = [];
    private readonly DeviceCache _deviceCache = new();
    private readonly DispatcherQueue _uiQueue;

    // Timers and watchers
    private DispatcherTimer? _eventPollTimer;
    private DispatcherTimer? _eventReloadTimer;
    private DispatcherTimer? _pairingTimer;
    private DispatcherTimer? _settingsDebounceTimer;
    private DispatcherTimer? _usbConnectTimer;
    private DeviceWatcher? _deviceWatcher;
    private bool _isEventPolling;
    private bool _isPairingPollRunning;
    private bool _pendingEventReload;
    private long _lastReloadCompletedTick;  // Environment.TickCount64 at last Reload end
    private long _rawInputProbeTick;        // last time raw-input triggered a probe
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
    }

    public void Dispose()
    {
        // Stop the timer so no new polls start. Then close handles directly without
        // waiting for _isEventPolling -- at shutdown the 0ms-timeout poll completes
        // in microseconds and the OS reclaims any still-open handles on process exit.
        _eventPollTimer?.Stop();
        _eventPollTimer = null;
        unsafe
        {
            foreach (var listener in _eventListeners)
                NativeInterop.pulsaar_close_event_listener(listener);
        }
        _eventListeners.Clear();

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
        _deviceWatcher.Added += (_, _) =>
        {
            // Suppress the spurious Added from the brief re-enumeration set_notification_flags
            // causes on the Bolt receiver. long reads are atomic on 64-bit Windows.
            if (Environment.TickCount64 - _lastReloadCompletedTick < 3000) return;
            ScheduleUsbConnectReload();
        };
        _deviceWatcher.Removed += (_, _) =>
        {
            // Same: suppress the Removed that follows set_notification_flags re-enumeration.
            if (Environment.TickCount64 - _lastReloadCompletedTick < 3000) return;
            _uiQueue.TryEnqueue(() => _ = Reload(false));
        };
        _deviceWatcher.Start();
        Debug.WriteLine("[PULSAAR][USB] DeviceWatcher started");
    }

    private void ScheduleUsbConnectReload()
    {
        // 3s debounce -- absorbs DeviceWatcher.Added callbacks for already-connected devices
        // at startup. We capture the reload count at schedule time: if any reload has completed
        // since then (e.g. the startup Reload), we skip -- the device state is already fresh.
        _uiQueue.TryEnqueue(() =>
        {
            _usbConnectTimer?.Stop();
            var countAtSchedule = _reloadCount;
            _usbConnectTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(3) };
            _usbConnectTimer.Tick += (s, _) =>
            {
                ((DispatcherTimer)s!).Stop();
                if (!IsPrefetching && _reloadCount == countAtSchedule)
                {
                    Debug.WriteLine("[PULSAAR][USB] debounced connect reload");
                    _ = Reload(false);
                }
            };
            _usbConnectTimer.Start();
        });
    }

    // Called from the WM_INPUT window proc (UI thread) when raw mouse/keyboard
    // input arrives from a Logitech receiver. Triggers a re-enumerate so any
    // device that woke from inactivity gets flipped online.
    public void OnRawInputActivity()
    {
        // Fast exit: all devices already online, nothing to update.
        if (!Receivers.Any(r => r.Devices.Any(d => !d.IsOnline))) return;

        // Cooldown: once we've decided to probe, suppress further triggers for
        // 3 seconds. Short enough that a second device waking up shortly after
        // the first is still detected promptly; long enough to prevent repeated
        // reloads while the user is actively using the newly-woken device.
        if (Environment.TickCount64 - _rawInputProbeTick < 3_000) return;
        _rawInputProbeTick = Environment.TickCount64;

        // 1-second delay lets the waking device finish its power-on sequence
        // before we attempt HID++ feature reads against its slots.
        var t = new DispatcherTimer { Interval = TimeSpan.FromSeconds(1) };
        t.Tick += (s, _) =>
        {
            ((DispatcherTimer)s!).Stop();
            if (Receivers.Any(r => r.Devices.Any(d => !d.IsOnline)))
                ScheduleEventReload();
        };
        t.Start();
    }

    private void ScheduleEventReload()
    {
        // Matches macOS scheduleEventReload: 750ms debounce so rapid online/offline bursts
        // (e.g. several devices reconnecting at once) collapse into a single Reload.
        //
        // If a Reload is already in progress we cannot start another one immediately.
        // Set _pendingEventReload so the in-progress Reload triggers a follow-up when done.
        if (_reloadInProgress)
        {
            _pendingEventReload = true;
            return;
        }

        _eventReloadTimer?.Stop();
        _eventReloadTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(750) };
        _eventReloadTimer.Tick += (s, _) =>
        {
            ((DispatcherTimer)s!).Stop();
            _ = Reload(false);
        };
        _eventReloadTimer.Start();
    }

    // ---------------------------------------------------------------------------
    // Reload (enumerate receivers + devices)
    // ---------------------------------------------------------------------------

    public async Task Reload(bool showIndicator)
    {
        if (_reloadInProgress)
        {
            Debug.WriteLine("[PULSAAR][STORE] Reload skipped (already in progress)");
            return;
        }
        _reloadInProgress = true;
        Debug.WriteLine("[PULSAAR][STORE] Reload");
        if (showIndicator) IsLoading = true;

        await StopEventListeners();

        var newReceivers     = new List<ReceiverModel>();
        var newDirectDevices = new List<DirectDeviceModel>();
        var rctxList         = new List<nint>();
        var kindList         = new List<ReceiverKind>();
        var riList           = new List<int>();
        string? error        = null;

        var swReload = System.Diagnostics.Stopwatch.StartNew();

        // Phase 1: Open all receivers (fast, ~30ms per receiver).
        // Builds ReceiverModel with an empty device list so the sidebar shows immediately.
        await Task.Run(() =>
        {
            unsafe
            {
                var sw = System.Diagnostics.Stopwatch.StartNew();
                var status = NativeInterop.pulsaar_refresh_receivers(_ctx);
                Debug.WriteLine($"[PULSAAR][TIMING] pulsaar_refresh_receivers: {sw.ElapsedMilliseconds}ms -> {status}");
                if (status != PulsaarStatus.Ok)
                {
                    error = $"Receiver refresh failed: {status}";
                    return;
                }

                nuint receiverCount = NativeInterop.pulsaar_get_receiver_count(_ctx);
                Debug.WriteLine($"[PULSAAR][TIMING] receiver count: {receiverCount}");

                for (nuint ri = 0; ri < receiverCount; ri++)
                {
                    PulsaarStatus openStatus;
                    sw.Restart();
                    var rctx = NativeInterop.pulsaar_open_receiver(_ctx, ri, &openStatus);
                    Debug.WriteLine($"[PULSAAR][TIMING] open_receiver[{ri}]: {sw.ElapsedMilliseconds}ms -> {openStatus}");
                    if (rctx == nint.Zero)
                    {
                        Debug.WriteLine($"[PULSAAR][STORE] receiver {ri} open failed: {openStatus}");
                        continue;
                    }

                    COpenedReceiverInfo openedInfo;
                    NativeInterop.pulsaar_get_opened_receiver_info(rctx, &openedInfo);

                    var receiver = ReceiverModel.FromOpened((int)ri, &openedInfo, []);
                    newReceivers.Add(receiver);
                    rctxList.Add(rctx);
                    kindList.Add((ReceiverKind)openedInfo.kind);
                    riList.Add((int)ri);
                    Debug.WriteLine($"[PULSAAR][STORE] opened receiver {ri}: {receiver.Name}");
                }
            }
        });

        // On the first load the sidebar is blank -- show the receiver name immediately so
        // there is something visible while Phase 2 enumerates devices. On subsequent reloads
        // the sidebar already has devices; keep showing them until the new data is ready.
        if (error == null && _reloadCount == 0)
            Receivers = newReceivers;

        // Phase 2: Enumerate devices per receiver, one at a time.
        // After each device is found the sidebar updates so devices trickle in as discovered.
        for (int i = 0; i < rctxList.Count; i++)
        {
            var rctx          = rctxList[i];
            var receiverModel = newReceivers[i];
            var myKind        = kindList[i];
            var myRi          = riList[i];

            await Task.Run(() => { unsafe { NativeInterop.pulsaar_start_enumerate(rctx); } });

            while (true)
            {
                DeviceModel?  device    = null;
                PulsaarStatus devStatus = PulsaarStatus.Unknown;

                await Task.Run(() =>
                {
                    unsafe
                    {
                        CDeviceInfo local = default;
                        devStatus = NativeInterop.pulsaar_enumerate_next_device(rctx, &local);
                        if (devStatus == PulsaarStatus.Ok)
                            device = DeviceModel.FromC(&local, myRi, myKind);
                    }
                });

                if (devStatus != PulsaarStatus.Ok || device == null) break;

                // Offline hysteresis: if discover_features timed out but the same
                // device was confirmed online within the last 60 seconds, keep it
                // showing online. This prevents a device that briefly entered its
                // inactivity sleep just before a reload from flashing offline.
                if (!device.IsOnline)
                {
                    var prev = Receivers.SelectMany(r => r.Devices)
                                        .FirstOrDefault(d => d.Id == device.Id);
                    if (prev != null &&
                        Environment.TickCount64 - prev.LastSeenOnlineTick < 180_000)
                    {
                        device.IsOnline = true;
                        if (prev.Name.Length > device.Name.Length)
                            device.Name = prev.Name;
                    }
                }

                if (!device.IsOnline)
                {
                    var cachedName = _deviceCache.Name(device.Serial);
                    if (cachedName != null) device.Name = cachedName;
                }
                else
                    _deviceCache.UpdateName(device.Serial, device.Name);

                if (!device.IsOnline)
                    device.Battery = _deviceCache.Battery(device.Serial);
                else if (device.Battery != null)
                    _deviceCache.UpdateBattery(device.Serial, device.Battery);

                receiverModel.Devices.Add(device);
                // On the first load, stream devices into the sidebar as they are found.
                // On subsequent reloads, accumulate silently and do one atomic update below.
                if (_reloadCount == 0)
                    Receivers = new List<ReceiverModel>(newReceivers);
            }

            Debug.WriteLine($"[PULSAAR][STORE] receiver {myRi}: {receiverModel.Name} ({receiverModel.Devices.Count} device(s))");
            NativeInterop.pulsaar_close_receiver(rctx);
        }

        // Direct (Bluetooth) devices
        await Task.Run(() =>
        {
            unsafe
            {
                var sw = System.Diagnostics.Stopwatch.StartNew();
                NativeInterop.pulsaar_refresh_direct_devices(_ctx);
                nuint directCount = NativeInterop.pulsaar_get_direct_device_count(_ctx);
                Debug.WriteLine($"[PULSAAR][TIMING] refresh_direct_devices: {sw.ElapsedMilliseconds}ms -> {directCount} device(s)");
                for (nuint di = 0; di < directCount; di++)
                {
                    CDirectDeviceInfo info;
                    if (NativeInterop.pulsaar_get_direct_device_info(_ctx, di, &info) != PulsaarStatus.Ok) continue;
                    newDirectDevices.Add(DirectDeviceModel.FromC(&info));
                }
            }
        });

        Debug.WriteLine($"[PULSAAR][TIMING] Reload total: {swReload.ElapsedMilliseconds}ms");

        // Snapshot which devices are currently online before we overwrite Receivers.
        // Used below to detect devices that just woke up so we can refresh their settings.
        var prevOnlineIds = _reloadCount > 0
            ? Receivers.SelectMany(r => r.Devices).Where(d => d.IsOnline).Select(d => d.Id).ToHashSet()
            : null;

        // Subsequent reloads skipped the per-device Receivers update to avoid flicker.
        // Do one atomic swap now that all device states are ready.
        if (error == null && _reloadCount > 0)
            Receivers = new List<ReceiverModel>(newReceivers);

        // Record completion time so the DeviceWatcher.Removed handler can suppress the
        // spurious removal event that set_notification_flags causes on the Bolt receiver.
        _lastReloadCompletedTick = Environment.TickCount64;

        DirectDevices     = newDirectDevices;
        ErrorMessage      = error;
        IsLoading         = false;
        _reloadCount++;
        _reloadInProgress = false;

        await RestartEventListeners();

        if (_reloadCount == 1)
            _ = PrefetchSettings();

        // Schedule a settings refresh for any device that just came online in this reload
        // (e.g. woke from inactivity sleep). PrefetchSettings handles the first-load case.
        if (prevOnlineIds != null)
        {
            foreach (var r in newReceivers)
            foreach (var d in r.Devices)
            {
                if (d.IsOnline && !prevOnlineIds.Contains(d.Id))
                {
                    Debug.WriteLine($"[PULSAAR][STORE] {d.Name} woke up, scheduling settings refresh");
                    ScheduleSettingsRefresh(d.Slot, d.ReceiverIndex);
                }
            }
        }

        // An online/offline event that arrived while this Reload was in progress was deferred.
        // Fire the follow-up now that we're no longer in progress.
        if (_pendingEventReload) { _pendingEventReload = false; ScheduleEventReload(); }
    }

    // ---------------------------------------------------------------------------
    // Event listeners
    // ---------------------------------------------------------------------------

    private async Task StopEventListeners()
    {
        _eventPollTimer?.Stop();
        _eventPollTimer = null;

        // Polls use a 0ms timeout so PollEventListeners returns in microseconds.
        // Yield the UI thread in short bursts until the in-flight poll's finally block
        // has had a chance to run and set _isEventPolling = false, ensuring no handle
        // is closed while it is still being referenced by the background thread.
        while (_isEventPolling)
            await Task.Delay(5);

        unsafe
        {
            foreach (var listener in _eventListeners)
                NativeInterop.pulsaar_close_event_listener(listener);
        }
        _eventListeners.Clear();
        Debug.WriteLine("[PULSAAR][EVENTS] listeners stopped");
    }

    private async Task RestartEventListeners()
    {
        await StopEventListeners();
        // After the await we're back on the UI thread; call the sync opener directly.
        OpenAndStartListeners();
    }

    // Sync helper: opens HID event handles and starts the poll timer.
    // Must not be async so that the `unsafe { &local }` pattern is valid.
    private void OpenAndStartListeners()
    {
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
        catch (Exception ex) { Debug.WriteLine($"[PULSAAR][EVENTS] poll error: {ex.Message}"); }
        finally { _isEventPolling = false; }
    }

    private void PollEventListeners()
    {
        var snapshot = _eventListeners.ToList(); // local copy for thread safety
        for (int i = 0; i < snapshot.Count; i++)
        {
            if (snapshot[i] == nint.Zero) continue;

            CDeviceConnectionEvent evRaw;
            unsafe { NativeInterop.pulsaar_poll_device_event(snapshot[i], 0, &evRaw); }

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
            ScheduleEventReload();
            return;
        }

        if (ev.event_type == PulsaarConnectionEvent.SettingsChanged)
        {
            var receiver = Receivers.ElementAtOrDefault(receiverIndex);
            var device   = receiver?.Devices.FirstOrDefault(d => d.Slot == ev.slot);

            // Bolt receivers don't send 0x41 link-change notifications when a device
            // reconnects -- it just starts sending unsolicited feature reports. Flip
            // IsOnline in-place immediately instead of doing a full reload.
            if (device != null && !device.IsOnline)
            {
                Debug.WriteLine($"[PULSAAR][EVENTS] slot={ev.slot} offline -> online (in-place)");
                device.IsOnline = true;
                Receivers = new List<ReceiverModel>(Receivers);
                ScheduleSettingsRefresh(ev.slot, receiverIndex);
                return;
            }

            // Discard persistent REPROG_CONTROLS_V4 (0x1B04) button events.
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
        if (device == null || !device.IsOnline) return;

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
        await StopEventListeners();

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
                        foreach (var device in snapshot[ri].Devices.ToList())
                        {
                            if (!device.IsOnline) continue;

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
        await RestartEventListeners();
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

        // Device is offline and not in cache -- no point sending HID requests that will time out.
        if (!device.IsOnline) return null;

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
        await StopEventListeners();
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
            await RestartEventListeners();
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
