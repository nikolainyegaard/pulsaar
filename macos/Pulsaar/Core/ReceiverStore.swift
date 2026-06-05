// Observable store that owns the Rust HID session and exposes receiver/device
// data to SwiftUI views. All mutations happen on the MainActor (default isolation).
//
// PulsaarContext and PulsaarReceiverContext are opaque C types (incomplete structs),
// so Swift imports pointers to them as OpaquePointer rather than UnsafeMutablePointer<T>.

import Foundation
import IOKit.hid

// Tracks which stage the pairing sheet is in.
enum PairingStage: Equatable {
    case idle
    case waiting
    case deviceFound
    case passkey
    case paired
    case failed
}

@Observable
final class ReceiverStore {
    var receivers: [ReceiverModel] = []
    var directDevices: [DirectDeviceModel] = []
    var settingsCache: [String: DeviceSettingsModel] = [:]
    var isLoading = false
    var errorMessage: String? = nil

    // OpaquePointer because PulsaarContext is a forward-declared (incomplete) C struct.
    // @ObservationIgnored because this pointer never needs to trigger SwiftUI updates.
    @ObservationIgnored private var ctx: OpaquePointer? = nil

    // ---------------------------------------------------------------------------
    // Pairing state (drives PairingSheetView)
    // ---------------------------------------------------------------------------

    var pairingStage: PairingStage = .idle
    var pairingDeviceName: String = ""
    var pairingPasskey: String = ""
    var pairingPasskeyIsNumeric: Bool = true
    var pairingNewSlot: UInt8 = 0
    var pairingError: String = ""

    @ObservationIgnored private var deviceCache = DeviceCache()
    @ObservationIgnored private var pairingRctx: OpaquePointer? = nil
    @ObservationIgnored private var pairingTimer: Timer? = nil
    @ObservationIgnored private var hidMonitor: IOHIDManager? = nil
    @ObservationIgnored private var eventListeners: [OpaquePointer] = []
    @ObservationIgnored private var eventTimer: Timer? = nil
    @ObservationIgnored private var pendingEventReload: DispatchWorkItem? = nil

    var isPairing: Bool { pairingStage != .idle }

    init() {
        ctx = pulsaar_init()
        guard ctx != nil else {
            errorMessage = "Could not initialize HID. Is a receiver plugged in?"
            return
        }
        reload()
        startUSBMonitoring()
        // Prefetch settings after a short delay so the IOKit matching callbacks
        // (which fire during IOHIDManagerOpen and trigger their own reload) have
        // time to complete before we stop listeners and open receivers for settings.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [weak self] in
            self?.prefetchSettings()
        }
    }

    deinit {
        // Direct cleanup to avoid touching @Observable properties in deinit.
        if let monitor = hidMonitor {
            IOHIDManagerUnscheduleFromRunLoop(monitor, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
            IOHIDManagerClose(monitor, IOOptionBits(kIOHIDOptionsTypeNone))
        }
        pendingEventReload?.cancel()
        eventTimer?.invalidate()
        for listener in eventListeners { pulsaar_close_event_listener(listener) }
        pairingTimer?.invalidate()
        if let rctx = pairingRctx {
            pulsaar_cancel_pairing(rctx)
            pulsaar_close_receiver(rctx)
        }
        if let ctx {
            pulsaar_destroy(ctx)
        }
    }

    // ---------------------------------------------------------------------------
    // USB monitoring
    // ---------------------------------------------------------------------------

    // Watch for Logitech receiver interfaces being added or removed. Calls reload()
    // automatically so the sidebar updates without a manual refresh.
    private func startUSBMonitoring() {
        let manager = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))

        IOHIDManagerSetDeviceMatching(manager, [
            kIOHIDVendorIDKey: 0x046D,
            "UsagePage": 0xFF00,
            "Usage": 0x0001,
        ] as CFDictionary)

        let ptr = Unmanaged.passUnretained(self).toOpaque()

        IOHIDManagerRegisterDeviceMatchingCallback(manager, { context, _, _, _ in
            guard let ctx = context else { return }
            let store = Unmanaged<ReceiverStore>.fromOpaque(ctx).takeUnretainedValue()
            DispatchQueue.main.async { store.reload() }
        }, ptr)

        IOHIDManagerRegisterDeviceRemovalCallback(manager, { context, _, _, _ in
            guard let ctx = context else { return }
            let store = Unmanaged<ReceiverStore>.fromOpaque(ctx).takeUnretainedValue()
            DispatchQueue.main.async { store.reload() }
        }, ptr)

        IOHIDManagerScheduleWithRunLoop(manager, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
        IOHIDManagerOpen(manager, IOOptionBits(kIOHIDOptionsTypeNone))
        hidMonitor = manager
    }

    // ---------------------------------------------------------------------------
    // Device connection-state event listeners (one per receiver)
    // ---------------------------------------------------------------------------

    // Tear down all event listeners and cancel any pending delayed reload.
    // Called at the start of reload() so the receiver handle is free for enumeration,
    // and by restartEventListeners() before opening new ones.
    private func stopEventListeners() {
        pendingEventReload?.cancel()
        pendingEventReload = nil
        stopEventTimer()
        for listener in eventListeners {
            pulsaar_close_event_listener(listener)
        }
        eventListeners.removeAll()
    }

    // Called at the end of every reload(). Opens one listener per receiver and
    // starts the polling timer.
    private func restartEventListeners() {
        stopEventListeners()

        guard let ctx else { return }

        for i in 0..<receivers.count {
            var status = PulsaarStatusUnknown
            if let listener = pulsaar_open_event_listener(ctx, i, &status) {
                eventListeners.append(listener)
            }
        }

        startEventTimer()
    }

    private func pollEventListeners() {
        guard !isPairing else { return } // pairing uses the same notification channel; let it run
        for listener in eventListeners {
            var event = CDeviceConnectionEvent()
            pulsaar_poll_device_event(listener, 0, &event)
            if event.event != PulsaarConnectionEventNone {
                scheduleEventReload()
                return
            }
        }
    }

    // Debounced reload for device state events. Waits 750 ms after the last event
    // before reloading, giving the device and receiver time to finish the transition.
    // If another event arrives within that window the timer resets.
    private func scheduleEventReload() {
        pendingEventReload?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.reload() }
        pendingEventReload = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.75, execute: work)
    }

    private func stopEventTimer() {
        eventTimer?.invalidate()
        eventTimer = nil
    }

    private func startEventTimer() {
        guard !eventListeners.isEmpty else { return }
        eventTimer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            self?.pollEventListeners()
        }
    }

    // Close listener handles so the receiver can be opened for settings reads/writes.
    // Same pattern as pairing (stopEventListeners before open, restartEventListeners after).
    func pauseEventPolling() { stopEventListeners() }

    // Reopen listener handles after a settings read/write completes.
    func resumeEventPolling() { restartEventListeners() }

    // Open the receiver for a device, run a closure with the context, then close it.
    // Returns false if the receiver cannot be opened.
    private func withReceiverContext(for receiverIndex: Int, _ body: (OpaquePointer) -> Bool) -> Bool {
        guard let ctx else { return false }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, receiverIndex, &openStatus) else { return false }
        defer { pulsaar_close_receiver(rctx) }
        return body(rctx)
    }

    // ---------------------------------------------------------------------------
    // Pairing
    // ---------------------------------------------------------------------------

    // Open the receiver and begin the pairing lock/discovery sequence.
    // The pairing sheet should call this in its onAppear.
    func startPairing(receiverIndex: Int, timeoutSecs: UInt8 = 30) {
        guard let ctx else { return }
        stopEventListeners()
        cleanupPairingResources()

        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, receiverIndex, &openStatus) else {
            pairingError = "Could not open receiver"
            pairingStage = .failed
            return
        }
        pairingRctx = rctx

        guard pulsaar_start_pairing(rctx, timeoutSecs) == PulsaarStatusOk else {
            pairingError = "Could not start pairing"
            pairingStage = .failed
            pulsaar_close_receiver(rctx)
            pairingRctx = nil
            return
        }

        pairingStage = .waiting
        pairingTimer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            self?.doPollPairing()
        }
    }

    private func doPollPairing() {
        guard let rctx = pairingRctx else { return }
        var status = CPairingStatus()
        guard pulsaar_poll_pairing(rctx, 0, &status) == PulsaarStatusOk else { return }

        switch status.state {
        case PulsaarPairingStateWaiting, PulsaarPairingStateIdle:
            break  // no event yet; keep waiting

        case PulsaarPairingStateDeviceFound:
            pairingDeviceName = cBufToString(status.device_name)
            pairingStage = .deviceFound

        case PulsaarPairingStatePasskeyNumeric:
            pairingPasskey = cBufToString(status.passkey)
            pairingPasskeyIsNumeric = true
            pairingStage = .passkey

        case PulsaarPairingStatePasskeyButton:
            pairingPasskey = cBufToString(status.passkey)
            pairingPasskeyIsNumeric = false
            pairingStage = .passkey

        case PulsaarPairingStatePaired:
            // device_name[0] carries the 1-based slot; actual name is in pairingDeviceName
            // from the earlier DeviceFound event (Bolt only; Unifying skips that step).
            pairingNewSlot = status.device_name.0
            pairingStage = .paired
            stopPairingTimer()
            // Give the sheet 1.5 s to show the success state, then close the rctx and reload.
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
                self?.finalizePairing()
            }

        case PulsaarPairingStateFailed:
            pairingError = cBufToString(status.error)
            pairingStage = .failed
            stopPairingTimer()

        default:
            break
        }
    }

    // Cancel an in-progress pairing (from the Cancel button or sheet onDismiss).
    // Safe to call in any state; no-op when not pairing.
    func cancelPairing() {
        if let rctx = pairingRctx {
            pulsaar_cancel_pairing(rctx)
        }
        cleanupPairing()
        restartEventListeners()
    }

    // Reset pairing state to idle without cancelling (call after successful pair + sheet dismiss).
    func resetPairing() {
        cleanupPairing()
    }

    private func finalizePairing() {
        closePairingRctx()
        reload()
        // Prefetch settings for the newly paired device after the reload settles.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.75) { [weak self] in
            self?.prefetchSettings()
        }
    }

    // Full teardown: stop timer, close rctx, reset all state vars.
    private func cleanupPairing() {
        stopPairingTimer()
        closePairingRctx()
        pairingStage = .idle
        pairingDeviceName = ""
        pairingPasskey = ""
        pairingError = ""
        pairingNewSlot = 0
    }

    // Teardown without resetting state vars (used in deinit and cleanupPairing).
    private func cleanupPairingResources() {
        stopPairingTimer()
        closePairingRctx()
    }

    private func stopPairingTimer() {
        pairingTimer?.invalidate()
        pairingTimer = nil
    }

    private func closePairingRctx() {
        if let rctx = pairingRctx {
            pulsaar_close_receiver(rctx)
            pairingRctx = nil
        }
    }

    // ---------------------------------------------------------------------------
    // Unpair
    // ---------------------------------------------------------------------------

    // Unpair a device from its receiver, then reload.
    // Returns true on success.
    func unpair(device: DeviceModel) -> Bool {
        stopEventListeners()
        let ok = withReceiverContext(for: device.receiverIndex) { rctx in
            pulsaar_unpair_device(rctx, device.slot) == PulsaarStatusOk
        }
        if ok { reload() } else { restartEventListeners() }
        return ok
    }

    // ---------------------------------------------------------------------------
    // Device settings
    // ---------------------------------------------------------------------------

    // Read all Phase 1+2 settings for a receiver-paired device.
    // Opens the receiver, reads all features, and closes it.
    // Returns nil if no settings are present or the receiver cannot be opened.
    // This is a blocking call; run it on a background thread.
    func loadSettings(for device: DeviceModel) -> DeviceSettingsModel? {
        guard let ctx else { return nil }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else { return nil }
        defer { pulsaar_close_receiver(rctx) }

        var dpiOut       = CDpiSettings()
        var scrollOut    = CScrollSettings()
        var ssOut        = CSmartShiftSettings()
        var hostsOut     = CHostList()
        var fnOut        = CFnSettings()
        var mpOut        = CMultiplatformSettings()
        var blOut        = CBacklightSettings()

        pulsaar_get_dpi_settings(rctx, device.slot, &dpiOut)
        pulsaar_get_scroll_settings(rctx, device.slot, &scrollOut)
        pulsaar_get_smartshift(rctx, device.slot, &ssOut)
        pulsaar_get_hosts(rctx, device.slot, &hostsOut)
        pulsaar_get_fn_settings(rctx, device.slot, &fnOut)
        pulsaar_get_multiplatform(rctx, device.slot, &mpOut)
        pulsaar_get_backlight(rctx, device.slot, &blOut)

        return DeviceSettingsModel(dpi: dpiOut, scroll: scrollOut, smartShift: ssOut, hosts: hostsOut, fn: fnOut, mp: mpOut, backlight: blOut)
    }

    // Set the active DPI for a device. Blocking; run on a background thread.
    func setDpi(for device: DeviceModel, dpi: Int) {
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else { return }
        defer { pulsaar_close_receiver(rctx) }
        pulsaar_set_dpi(rctx, device.slot, UInt16(dpi))
    }

    // Set scroll inversion and hi-res mode for a device. Blocking; run on a background thread.
    func setScrollSettings(for device: DeviceModel, inverted: Bool, hires: Bool) {
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else { return }
        defer { pulsaar_close_receiver(rctx) }
        pulsaar_set_scroll_settings(rctx, device.slot, inverted ? 1 : 0, hires ? 1 : 0)
    }

    // Set smart-shift wheel mode (1=freespin, 2=smart-shift) and torque (1-100). Blocking.
    func setSmartShift(for device: DeviceModel, wheelMode: UInt8, torque: UInt8) {
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else { return }
        defer { pulsaar_close_receiver(rctx) }
        pulsaar_set_smartshift(rctx, device.slot, wheelMode, torque)
    }

    // Switch the active host for a device. The device disconnects immediately. Blocking.
    func setActiveHost(for device: DeviceModel, hostSlot: UInt8) {
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else { return }
        defer { pulsaar_close_receiver(rctx) }
        pulsaar_set_active_host(rctx, device.slot, hostSlot)
    }

    // Set FN key swap state (true = multimedia keys by default). Blocking.
    func setFnSwap(for device: DeviceModel, swapped: Bool) {
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else { return }
        defer { pulsaar_close_receiver(rctx) }
        pulsaar_set_fn_swap(rctx, device.slot, swapped ? 1 : 0)
    }

    // Set the active OS platform for a device. Blocking.
    func setMultiplatform(for device: DeviceModel, platformIndex: UInt8) {
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else { return }
        defer { pulsaar_close_receiver(rctx) }
        pulsaar_set_multiplatform(rctx, device.slot, platformIndex)
    }

    // Set backlight mode (0=off, 1=auto, 3=manual) and brightness. Blocking.
    func setBacklight(for device: DeviceModel, mode: UInt8, brightness: UInt8) {
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else { return }
        defer { pulsaar_close_receiver(rctx) }
        pulsaar_set_backlight(rctx, device.slot, mode, brightness)
    }

    // Prefetch settings for all paired devices across all receivers in the background.
    // Stops event listeners once for the entire batch, reads all receivers, then restarts.
    // Called once at launch (after IOKit callbacks settle) and after pairing -- NOT from reload(),
    // so it never races with an IOKit- or event-triggered reload opening the same receiver.
    func prefetchSettings() {
        let snapshot = receivers
        guard !snapshot.isEmpty else { return }

        DispatchQueue.global(qos: .utility).async { [weak self] in
            guard let self else { return }

            // Stop all listeners once before opening any receiver.
            let sem = DispatchSemaphore(value: 0)
            DispatchQueue.main.async { self.stopEventListeners(); sem.signal() }
            sem.wait()

            var batch: [String: DeviceSettingsModel] = [:]

            if let ctx = self.ctx {
                for receiver in snapshot {
                    guard !receiver.devices.isEmpty else { continue }
                    var openStatus = PulsaarStatusUnknown
                    if let rctx: OpaquePointer = pulsaar_open_receiver(ctx, receiver.id, &openStatus) {
                        for device in receiver.devices {
                            var dpiOut    = CDpiSettings()
                            var scrollOut = CScrollSettings()
                            var ssOut     = CSmartShiftSettings()
                            var hostsOut  = CHostList()
                            var fnOut     = CFnSettings()
                            var mpOut     = CMultiplatformSettings()
                            var blOut     = CBacklightSettings()
                            pulsaar_get_dpi_settings(rctx, device.slot, &dpiOut)
                            pulsaar_get_scroll_settings(rctx, device.slot, &scrollOut)
                            pulsaar_get_smartshift(rctx, device.slot, &ssOut)
                            pulsaar_get_hosts(rctx, device.slot, &hostsOut)
                            pulsaar_get_fn_settings(rctx, device.slot, &fnOut)
                            pulsaar_get_multiplatform(rctx, device.slot, &mpOut)
                            pulsaar_get_backlight(rctx, device.slot, &blOut)
                            // Write scroll mode back after reading to clear the HIRES_WHEEL
                            // "target" bit (0x10). That bit, when set by other software, routes
                            // scroll events through the HID++ channel instead of standard HID,
                            // causing the OS scroll input to stop working while Pulsaar is open.
                            if scrollOut.has_hires != 0 || scrollOut.has_invert != 0 {
                                pulsaar_set_scroll_settings(rctx, device.slot, scrollOut.inverted, scrollOut.hires_enabled)
                            }
                            if let model = DeviceSettingsModel(dpi: dpiOut, scroll: scrollOut, smartShift: ssOut, hosts: hostsOut, fn: fnOut, mp: mpOut, backlight: blOut) {
                                batch[device.id] = model
                            }
                        }
                        pulsaar_close_receiver(rctx)
                    }
                }
            }

            // Update cache and restart listeners in one main-thread block.
            DispatchQueue.main.async {
                for (id, model) in batch { self.settingsCache[id] = model }
                self.restartEventListeners()
            }
        }
    }

    // showIndicator: true for user-initiated reloads (shows "Scanning..." in sidebar).
    // false (default) for automatic reloads; the sidebar stays as-is until the
    // new state is ready, then updates atomically with no intermediate blank state.
    func reload(showIndicator: Bool = false) {
        guard let ctx else { return }
        // Close event listeners before opening receivers for enumeration so there
        // is no competing HID handle on the same device during the reload.
        stopEventListeners()
        if showIndicator { isLoading = true }
        errorMessage = nil

        // Refresh the HID device tree so plug/unplug events are reflected.
        pulsaar_refresh_receivers(ctx)

        var result: [ReceiverModel] = []
        let count = pulsaar_get_receiver_count(ctx)

        for i in 0..<count {
            // Pre-open info (verify the slot is valid before opening).
            var preInfo = CReceiverInfo()
            guard pulsaar_get_receiver_info(ctx, i, &preInfo) == PulsaarStatusOk else { continue }

            // Open the receiver. rctx is also an OpaquePointer (PulsaarReceiverContext).
            var openStatus = PulsaarStatusUnknown
            guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, i, &openStatus) else { continue }
            defer { pulsaar_close_receiver(rctx) }

            // Opened receiver info (serial, max_devices, etc.).
            var rinfo = COpenedReceiverInfo()
            guard pulsaar_get_opened_receiver_info(rctx, &rinfo) == PulsaarStatusOk else { continue }

            // Device enumeration.
            var devices: [DeviceModel] = []
            if pulsaar_enumerate_devices(rctx) == PulsaarStatusOk {
                let dcount = pulsaar_get_device_count(rctx)
                for j in 0..<dcount {
                    var dev = CDeviceInfo()
                    if pulsaar_get_device_info(rctx, j, &dev) == PulsaarStatusOk {
                        let rKind = ReceiverKind(byte: rinfo.kind)
                        var device = DeviceModel(c: dev, receiverIndex: i, receiverKind: rKind)
                        if device.isOnline {
                            // Persist the live battery reading for future offline display.
                            if let battery = device.battery {
                                deviceCache.update(serial: device.serial, battery: battery)
                            }
                        } else if let cached = deviceCache.battery(for: device.serial) {
                            // Inject last-known battery so the UI can show it while offline.
                            device.battery = BatteryModel(cached: cached)
                        }
                        devices.append(device)
                    }
                }
            }

            result.append(ReceiverModel(index: i, openedInfo: rinfo, devices: devices))
        }

        receivers = result

        // Read directly-connected (Bluetooth) devices. pulsaar_refresh_receivers already
        // re-ran enumerate_direct_devices inside the Rust context, so the count is fresh.
        var directResult: [DirectDeviceModel] = []
        let dcount = pulsaar_get_direct_device_count(ctx)
        for i in 0..<dcount {
            var info = CDirectDeviceInfo()
            if pulsaar_get_direct_device_info(ctx, i, &info) == PulsaarStatusOk {
                directResult.append(DirectDeviceModel(c: info))
            }
        }
        directDevices = directResult

        isLoading = false
        restartEventListeners()
    }
}
