// Observable store that owns the Rust HID session and exposes receiver/device
// data to SwiftUI views. All mutations happen on the MainActor (default isolation).
//
// PulsaarContext and PulsaarReceiverContext are opaque C types (incomplete structs),
// so Swift imports pointers to them as OpaquePointer rather than UnsafeMutablePointer<T>.

import Foundation
import IOKit.hid

// ---------------------------------------------------------------------------
// Debug logging
// ---------------------------------------------------------------------------

private let _pLogDateFmt: DateFormatter = {
    let f = DateFormatter()
    f.dateFormat = "HH:mm:ss.SSS"
    return f
}()

/// Set false to silence all Pulsaar debug output.
let pulsaarLoggingEnabled = true

func pLog(_ tag: String, _ msg: String) {
    guard pulsaarLoggingEnabled else { return }
    print("[PULSAAR][\(tag)] \(_pLogDateFmt.string(from: Date())) \(msg)")
}

func statusStr(_ s: PulsaarStatus) -> String {
    switch s {
    case PulsaarStatusOk:         return "ok"
    case PulsaarStatusHidError:   return "hid_error"
    case PulsaarStatusTimeout:    return "timeout"
    case PulsaarStatusNoReceiver: return "no_receiver"
    case PulsaarStatusEmptySlot:  return "empty_slot"
    case PulsaarStatusInvalidArg: return "invalid_arg"
    default:                      return "unknown"
    }
}

// ---------------------------------------------------------------------------
// Pairing state
// ---------------------------------------------------------------------------

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
    /// True while prefetchSettings is running on a background thread.
    /// The settings task checks this to avoid racing to open the receiver concurrently.
    var isPrefetching = false

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
    @ObservationIgnored private var pendingUSBConnectReload: DispatchWorkItem? = nil

    var isPairing: Bool { pairingStage != .idle }

    init() {
        pLog("STORE", "init: initializing HID context")
        ctx = pulsaar_init()
        guard ctx != nil else {
            pLog("STORE", "init: FAILED - could not initialize HID context")
            errorMessage = "Could not initialize HID. Is a receiver plugged in?"
            return
        }
        pLog("STORE", "init: HID context ready")
        reload()
        startUSBMonitoring()
        pLog("STORE", "init: prefetchSettings scheduled in 1.0s")
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
        pendingUSBConnectReload?.cancel()
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
            DispatchQueue.main.async {
                pLog("USB", "receiver connected -> scheduling reload (3s debounce)")
                store.scheduleUSBConnectReload()
            }
        }, ptr)

        IOHIDManagerRegisterDeviceRemovalCallback(manager, { context, _, _, _ in
            guard let ctx = context else { return }
            let store = Unmanaged<ReceiverStore>.fromOpaque(ctx).takeUnretainedValue()
            DispatchQueue.main.async {
                pLog("USB", "receiver disconnected -> reload")
                store.reload()
            }
        }, ptr)

        IOHIDManagerScheduleWithRunLoop(manager, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
        IOHIDManagerOpen(manager, IOOptionBits(kIOHIDOptionsTypeNone))
        hidMonitor = manager
        pLog("USB", "IOKit HID monitoring started (vendor=0x046D usage_page=0xFF00 usage=0x0001)")
    }

    // Debounced reload for device-matching callbacks. IOHIDManagerOpen fires matching
    // callbacks for already-connected devices at startup, causing spurious rapid-fire
    // reloads. A 3s debounce collapses the burst into a single reload.
    private func scheduleUSBConnectReload() {
        pendingUSBConnectReload?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            // If prefetch is still running it holds the receiver open; opening it again
            // here would fail with exclusive-access and wipe the receiver list. Skip --
            // prefetch will call restartEventListeners when it finishes.
            guard !self.isPrefetching else {
                pLog("USB", "USB connect reload firing -- skipped, prefetch running")
                return
            }
            pLog("USB", "USB connect reload firing")
            self.reload()
        }
        pendingUSBConnectReload = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.0, execute: work)
    }

    // ---------------------------------------------------------------------------
    // Device connection-state event listeners (one per receiver)
    // ---------------------------------------------------------------------------

    private func stopEventListeners() {
        pLog("EVENTS", "stopEventListeners: closing \(eventListeners.count) listener(s)")
        pendingEventReload?.cancel()
        pendingEventReload = nil
        stopEventTimer()
        for listener in eventListeners {
            pulsaar_close_event_listener(listener)
        }
        eventListeners.removeAll()
    }

    private func restartEventListeners() {
        stopEventListeners()
        guard let ctx else { return }
        pLog("EVENTS", "restartEventListeners: opening listeners for \(receivers.count) receiver(s)")
        for i in 0..<receivers.count {
            var status = PulsaarStatusUnknown
            if let listener = pulsaar_open_event_listener(ctx, i, &status) {
                eventListeners.append(listener)
            } else {
                pLog("EVENTS", "  receiver[\(i)] listener FAILED: \(statusStr(status))")
            }
        }
        pLog("EVENTS", "  \(eventListeners.count)/\(receivers.count) listener(s) active")
        startEventTimer()
    }

    private func pollEventListeners() {
        guard !isPairing else { return }
        for (i, listener) in eventListeners.enumerated() {
            var event = CDeviceConnectionEvent()
            pulsaar_poll_device_event(listener, 0, &event)
            if event.event != PulsaarConnectionEventNone {
                let kind = event.event == PulsaarConnectionEventOnline ? "online" : "offline"
                pLog("EVENTS", "connection event: listener[\(i)] slot=\(event.slot) \(kind) -> scheduling reload")
                scheduleEventReload()
                return
            }
        }
    }

    private func scheduleEventReload() {
        pendingEventReload?.cancel()
        pLog("EVENTS", "scheduleEventReload: reload in 0.75s")
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

    func pauseEventPolling() {
        pLog("EVENTS", "pauseEventPolling")
        stopEventListeners()
    }

    func resumeEventPolling() {
        pLog("EVENTS", "resumeEventPolling")
        restartEventListeners()
    }

    private func withReceiverContext(for receiverIndex: Int, _ body: (OpaquePointer) -> Bool) -> Bool {
        guard let ctx else { return false }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, receiverIndex, &openStatus) else {
            pLog("STORE", "withReceiverContext: FAIL receiver[\(receiverIndex)] \(statusStr(openStatus))")
            return false
        }
        defer { pulsaar_close_receiver(rctx) }
        return body(rctx)
    }

    // ---------------------------------------------------------------------------
    // Pairing
    // ---------------------------------------------------------------------------

    func startPairing(receiverIndex: Int, timeoutSecs: UInt8 = 30) {
        pLog("PAIRING", "startPairing: receiver[\(receiverIndex)] timeout=\(timeoutSecs)s")
        guard let ctx else { return }
        stopEventListeners()
        cleanupPairingResources()

        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, receiverIndex, &openStatus) else {
            pLog("PAIRING", "startPairing: FAIL open receiver: \(statusStr(openStatus))")
            pairingError = "Could not open receiver"
            pairingStage = .failed
            return
        }
        pairingRctx = rctx

        guard pulsaar_start_pairing(rctx, timeoutSecs) == PulsaarStatusOk else {
            pLog("PAIRING", "startPairing: FAIL pulsaar_start_pairing")
            pairingError = "Could not start pairing"
            pairingStage = .failed
            pulsaar_close_receiver(rctx)
            pairingRctx = nil
            return
        }

        pLog("PAIRING", "startPairing: waiting for device")
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
            break

        case PulsaarPairingStateDeviceFound:
            pairingDeviceName = cBufToString(status.device_name)
            pLog("PAIRING", "device found: '\(pairingDeviceName)'")
            pairingStage = .deviceFound

        case PulsaarPairingStatePasskeyNumeric:
            pairingPasskey = cBufToString(status.passkey)
            pairingPasskeyIsNumeric = true
            pLog("PAIRING", "passkey (numeric): '\(pairingPasskey)'")
            pairingStage = .passkey

        case PulsaarPairingStatePasskeyButton:
            pairingPasskey = cBufToString(status.passkey)
            pairingPasskeyIsNumeric = false
            pLog("PAIRING", "passkey (button): '\(pairingPasskey)'")
            pairingStage = .passkey

        case PulsaarPairingStatePaired:
            pairingNewSlot = status.device_name.0
            pLog("PAIRING", "paired: slot=\(pairingNewSlot)")
            pairingStage = .paired
            stopPairingTimer()
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
                self?.finalizePairing()
            }

        case PulsaarPairingStateFailed:
            pairingError = cBufToString(status.error)
            pLog("PAIRING", "failed: '\(pairingError)'")
            pairingStage = .failed
            stopPairingTimer()

        default:
            break
        }
    }

    func cancelPairing() {
        pLog("PAIRING", "cancelPairing")
        if let rctx = pairingRctx {
            pulsaar_cancel_pairing(rctx)
        }
        cleanupPairing()
        restartEventListeners()
    }

    func resetPairing() {
        pLog("PAIRING", "resetPairing")
        cleanupPairing()
    }

    private func finalizePairing() {
        pLog("PAIRING", "finalizePairing: closing rctx, reloading")
        closePairingRctx()
        reload()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.75) { [weak self] in
            self?.prefetchSettings()
        }
    }

    private func cleanupPairing() {
        stopPairingTimer()
        closePairingRctx()
        pairingStage = .idle
        pairingDeviceName = ""
        pairingPasskey = ""
        pairingError = ""
        pairingNewSlot = 0
    }

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

    func unpair(device: DeviceModel) -> Bool {
        pLog("STORE", "unpair '\(device.name)' slot=\(device.slot) receiver=\(device.receiverIndex)")
        stopEventListeners()
        let ok = withReceiverContext(for: device.receiverIndex) { rctx in
            let s = pulsaar_unpair_device(rctx, device.slot)
            pLog("STORE", "  pulsaar_unpair_device -> \(statusStr(s))")
            return s == PulsaarStatusOk
        }
        pLog("STORE", "  unpair result: \(ok ? "ok" : "failed")")
        if ok { reload() } else { restartEventListeners() }
        return ok
    }

    // ---------------------------------------------------------------------------
    // Device settings
    // ---------------------------------------------------------------------------

    func loadSettings(for device: DeviceModel) -> DeviceSettingsModel? {
        pLog("SETTINGS", "loadSettings '\(device.name)' slot=\(device.slot) receiver=\(device.receiverIndex)")
        guard let ctx else { return nil }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else {
            pLog("SETTINGS", "  FAIL open receiver: \(statusStr(openStatus))")
            return nil
        }
        defer { pulsaar_close_receiver(rctx) }

        pLog("SETTINGS", "  receiver opened, reading all features (single discover_features call)...")
        var allOut = CAllDeviceSettings()
        pulsaar_get_all_settings(rctx, device.slot, &allOut)
        pLog("SETTINGS", "  DPI: count=\(allOut.dpi.dpi_count) current=\(allOut.dpi.current_dpi)")
        pLog("SETTINGS", "  scroll: hasInvert=\(allOut.scroll.has_invert) hasHires=\(allOut.scroll.has_hires)")
        pLog("SETTINGS", "  smartShift: wheelMode=\(allOut.ss.wheel_mode) hasTorque=\(allOut.ss.has_torque)")
        pLog("SETTINGS", "  hosts: count=\(allOut.hosts.count)")
        pLog("SETTINGS", "  fn: hasFeature=\(allOut.fn_s.has_feature) swapped=\(allOut.fn_s.fn_swapped)")
        pLog("SETTINGS", "  backlight: hasFeature=\(allOut.backlight.has_feature) mode=\(allOut.backlight.mode)")

        let result = DeviceSettingsModel(dpi: allOut.dpi, scroll: allOut.scroll, smartShift: allOut.ss, hosts: allOut.hosts, fn: allOut.fn_s, mp: allOut.mp, backlight: allOut.backlight)
        pLog("SETTINGS", "  loadSettings result: \(result != nil ? "has settings" : "nil (no configurable settings)")")
        return result
    }

    func setDpi(for device: DeviceModel, dpi: Int) {
        pLog("WRITE", "setDpi '\(device.name)' slot=\(device.slot) dpi=\(dpi)")
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else {
            pLog("WRITE", "  FAIL open receiver: \(statusStr(openStatus))")
            return
        }
        defer { pulsaar_close_receiver(rctx) }
        let s = pulsaar_set_dpi(rctx, device.slot, UInt16(dpi))
        pLog("WRITE", "  pulsaar_set_dpi -> \(statusStr(s))")
    }

    func setScrollSettings(for device: DeviceModel, inverted: Bool, hires: Bool) {
        pLog("WRITE", "setScrollSettings '\(device.name)' slot=\(device.slot) inverted=\(inverted) hires=\(hires)")
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else {
            pLog("WRITE", "  FAIL open receiver: \(statusStr(openStatus))")
            return
        }
        defer { pulsaar_close_receiver(rctx) }
        let s = pulsaar_set_scroll_settings(rctx, device.slot, inverted ? 1 : 0, hires ? 1 : 0)
        pLog("WRITE", "  pulsaar_set_scroll_settings -> \(statusStr(s))")
    }

    func setSmartShift(for device: DeviceModel, wheelMode: UInt8, torque: UInt8) {
        pLog("WRITE", "setSmartShift '\(device.name)' slot=\(device.slot) wheelMode=\(wheelMode) torque=\(torque)")
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else {
            pLog("WRITE", "  FAIL open receiver: \(statusStr(openStatus))")
            return
        }
        defer { pulsaar_close_receiver(rctx) }
        let s = pulsaar_set_smartshift(rctx, device.slot, wheelMode, torque)
        pLog("WRITE", "  pulsaar_set_smartshift -> \(statusStr(s))")
    }

    func setActiveHost(for device: DeviceModel, hostSlot: UInt8) {
        pLog("WRITE", "setActiveHost '\(device.name)' slot=\(device.slot) hostSlot=\(hostSlot)")
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else {
            pLog("WRITE", "  FAIL open receiver: \(statusStr(openStatus))")
            return
        }
        defer { pulsaar_close_receiver(rctx) }
        let s = pulsaar_set_active_host(rctx, device.slot, hostSlot)
        pLog("WRITE", "  pulsaar_set_active_host -> \(statusStr(s))")
    }

    func setFnSwap(for device: DeviceModel, swapped: Bool) {
        pLog("WRITE", "setFnSwap '\(device.name)' slot=\(device.slot) swapped=\(swapped)")
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else {
            pLog("WRITE", "  FAIL open receiver: \(statusStr(openStatus))")
            return
        }
        defer { pulsaar_close_receiver(rctx) }
        let s = pulsaar_set_fn_swap(rctx, device.slot, swapped ? 1 : 0)
        pLog("WRITE", "  pulsaar_set_fn_swap -> \(statusStr(s))")
    }

    func setMultiplatform(for device: DeviceModel, platformIndex: UInt8) {
        pLog("WRITE", "setMultiplatform '\(device.name)' slot=\(device.slot) platformIndex=\(platformIndex)")
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else {
            pLog("WRITE", "  FAIL open receiver: \(statusStr(openStatus))")
            return
        }
        defer { pulsaar_close_receiver(rctx) }
        let s = pulsaar_set_multiplatform(rctx, device.slot, platformIndex)
        pLog("WRITE", "  pulsaar_set_multiplatform -> \(statusStr(s))")
    }

    func setBacklight(for device: DeviceModel, mode: UInt8, brightness: UInt8) {
        pLog("WRITE", "setBacklight '\(device.name)' slot=\(device.slot) mode=\(mode) brightness=\(brightness)")
        guard let ctx else { return }
        var openStatus = PulsaarStatusUnknown
        guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, device.receiverIndex, &openStatus) else {
            pLog("WRITE", "  FAIL open receiver: \(statusStr(openStatus))")
            return
        }
        defer { pulsaar_close_receiver(rctx) }
        let s = pulsaar_set_backlight(rctx, device.slot, mode, brightness)
        pLog("WRITE", "  pulsaar_set_backlight -> \(statusStr(s))")
    }

    // ---------------------------------------------------------------------------
    // Prefetch settings
    // ---------------------------------------------------------------------------

    func prefetchSettings() {
        let snapshot = receivers
        guard !snapshot.isEmpty else {
            pLog("SETTINGS", "prefetchSettings: no receivers, skipping")
            return
        }
        pLog("SETTINGS", "prefetchSettings: \(snapshot.count) receiver(s), \(snapshot.flatMap(\.devices).count) device(s) total")
        isPrefetching = true

        DispatchQueue.global(qos: .utility).async { [weak self] in
            guard let self else { return }

            let sem = DispatchSemaphore(value: 0)
            DispatchQueue.main.async { self.stopEventListeners(); sem.signal() }
            sem.wait()

            var batch: [String: DeviceSettingsModel] = [:]

            if let ctx = self.ctx {
                for receiver in snapshot {
                    guard !receiver.devices.isEmpty else { continue }
                    pLog("SETTINGS", "  receiver[\(receiver.id)] '\(receiver.name)' \(receiver.devices.count) device(s)")
                    var openStatus = PulsaarStatusUnknown
                    if let rctx: OpaquePointer = pulsaar_open_receiver(ctx, receiver.id, &openStatus) {
                        for device in receiver.devices {
                            pLog("SETTINGS", "    '\(device.name)' slot=\(device.slot) reading...")
                            var allOut = CAllDeviceSettings()
                            pulsaar_get_all_settings(rctx, device.slot, &allOut)
                            if allOut.scroll.has_hires != 0 || allOut.scroll.has_invert != 0 {
                                pLog("SETTINGS", "    writing scroll mode to clear HID++ target bit (inverted=\(allOut.scroll.inverted) hires=\(allOut.scroll.hires_enabled))")
                                let sw = pulsaar_set_scroll_settings(rctx, device.slot, allOut.scroll.inverted, allOut.scroll.hires_enabled)
                                pLog("SETTINGS", "    scroll write -> \(statusStr(sw))")
                            }
                            if let model = DeviceSettingsModel(dpi: allOut.dpi, scroll: allOut.scroll, smartShift: allOut.ss, hosts: allOut.hosts, fn: allOut.fn_s, mp: allOut.mp, backlight: allOut.backlight) {
                                batch[device.id] = model
                                pLog("SETTINGS", "    cached: dpi=\(model.hasDpi) scroll=\(model.hasScrollSettings) fn=\(model.hasFnSwap) backlight=\(model.hasBacklight)")
                            } else {
                                pLog("SETTINGS", "    no configurable settings")
                            }
                        }
                        pulsaar_close_receiver(rctx)
                    } else {
                        pLog("SETTINGS", "  receiver[\(receiver.id)] FAIL open: \(statusStr(openStatus))")
                    }
                }
            }

            DispatchQueue.main.async {
                pLog("SETTINGS", "prefetchSettings done: \(batch.count) device(s) cached")
                for (id, model) in batch { self.settingsCache[id] = model }
                self.isPrefetching = false
                self.restartEventListeners()
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Reload
    // ---------------------------------------------------------------------------

    func reload(showIndicator: Bool = false) {
        pLog("STORE", "reload start (showIndicator=\(showIndicator))")
        guard let ctx else { return }
        stopEventListeners()
        if showIndicator { isLoading = true }
        errorMessage = nil

        pulsaar_refresh_receivers(ctx)

        var result: [ReceiverModel] = []
        let count = pulsaar_get_receiver_count(ctx)
        pLog("STORE", "  \(count) receiver(s) from HID scan")

        for i in 0..<count {
            var preInfo = CReceiverInfo()
            guard pulsaar_get_receiver_info(ctx, i, &preInfo) == PulsaarStatusOk else { continue }

            var openStatus = PulsaarStatusUnknown
            guard let rctx: OpaquePointer = pulsaar_open_receiver(ctx, i, &openStatus) else {
                pLog("STORE", "  receiver[\(i)] FAIL open: \(statusStr(openStatus))")
                continue
            }
            defer { pulsaar_close_receiver(rctx) }

            var rinfo = COpenedReceiverInfo()
            guard pulsaar_get_opened_receiver_info(rctx, &rinfo) == PulsaarStatusOk else { continue }
            pLog("STORE", "  receiver[\(i)] '\(cBufToString(rinfo.name))' pid=0x\(String(format: "%04X", rinfo.product_id)) kind=\(rinfo.kind) maxDev=\(rinfo.max_devices)")

            var devices: [DeviceModel] = []
            if pulsaar_enumerate_devices(rctx) == PulsaarStatusOk {
                let dcount = pulsaar_get_device_count(rctx)
                for j in 0..<dcount {
                    var dev = CDeviceInfo()
                    if pulsaar_get_device_info(rctx, j, &dev) == PulsaarStatusOk {
                        let rKind = ReceiverKind(byte: rinfo.kind)
                        var device = DeviceModel(c: dev, receiverIndex: i, receiverKind: rKind)
                        pLog("STORE", "    device slot=\(dev.slot) '\(cBufToString(dev.name))' online=\(device.isOnline) battery=\(device.battery?.levelText ?? "none")")
                        if device.isOnline {
                            if let battery = device.battery {
                                deviceCache.update(serial: device.serial, battery: battery)
                            }
                        } else if let cached = deviceCache.battery(for: device.serial) {
                            device.battery = BatteryModel(cached: cached)
                            pLog("STORE", "      injected cached battery: \(device.battery?.levelText ?? "?")")
                        }
                        devices.append(device)
                    }
                }
            }

            result.append(ReceiverModel(index: i, openedInfo: rinfo, devices: devices))
        }

        receivers = result

        let dcount = pulsaar_get_direct_device_count(ctx)
        var directResult: [DirectDeviceModel] = []
        pLog("STORE", "  \(dcount) direct (Bluetooth) device(s)")
        for i in 0..<dcount {
            var info = CDirectDeviceInfo()
            if pulsaar_get_direct_device_info(ctx, i, &info) == PulsaarStatusOk {
                let d = DirectDeviceModel(c: info)
                pLog("STORE", "    direct '\(d.name)' pid=0x\(String(format: "%04X", d.productId)) battery=\(d.battery?.levelText ?? "none")")
                directResult.append(d)
            }
        }
        directDevices = directResult

        isLoading = false
        let deviceCount = result.flatMap(\.devices).count
        pLog("STORE", "reload done: \(result.count) receiver(s), \(deviceCount) paired device(s), \(directResult.count) direct device(s)")
        restartEventListeners()
    }
}
