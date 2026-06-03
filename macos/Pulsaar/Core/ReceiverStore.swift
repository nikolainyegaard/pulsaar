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

    @ObservationIgnored private var pairingRctx: OpaquePointer? = nil
    @ObservationIgnored private var pairingTimer: Timer? = nil
    @ObservationIgnored private var hidMonitor: IOHIDManager? = nil
    @ObservationIgnored private var eventListeners: [OpaquePointer] = []
    @ObservationIgnored private var eventTimer: Timer? = nil

    var isPairing: Bool { pairingStage != .idle }

    init() {
        ctx = pulsaar_init()
        guard ctx != nil else {
            errorMessage = "Could not initialize HID. Is a receiver plugged in?"
            return
        }
        reload()
        startUSBMonitoring()
    }

    deinit {
        // Direct cleanup to avoid touching @Observable properties in deinit.
        if let monitor = hidMonitor {
            IOHIDManagerUnscheduleFromRunLoop(monitor, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
            IOHIDManagerClose(monitor, IOOptionBits(kIOHIDOptionsTypeNone))
        }
        eventTimer?.invalidate()
        for listener in eventListeners {
            pulsaar_close_event_listener(listener)
        }
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

    // Called at the end of every reload(). Closes old listeners, opens new ones
    // for all currently known receivers, and (re)starts the polling timer.
    private func restartEventListeners() {
        stopEventTimer()
        for listener in eventListeners {
            pulsaar_close_event_listener(listener)
        }
        eventListeners.removeAll()

        guard let ctx else { return }

        for i in 0..<receivers.count {
            var status = PulsaarStatusUnknown
            if let listener = pulsaar_open_event_listener(ctx, i, &status) {
                eventListeners.append(listener)
            }
        }

        guard !eventListeners.isEmpty else { return }

        eventTimer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            self?.pollEventListeners()
        }
    }

    private func pollEventListeners() {
        guard !isPairing else { return } // pairing uses the same notification channel; let it run
        for listener in eventListeners {
            var event = CDeviceConnectionEvent()
            pulsaar_poll_device_event(listener, 0, &event)
            if event.event != PulsaarConnectionEventNone {
                reload()
                return
            }
        }
    }

    private func stopEventTimer() {
        eventTimer?.invalidate()
        eventTimer = nil
    }

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
    }

    // Reset pairing state to idle without cancelling (call after successful pair + sheet dismiss).
    func resetPairing() {
        cleanupPairing()
    }

    private func finalizePairing() {
        closePairingRctx()
        reload()
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
        let ok = withReceiverContext(for: device.receiverIndex) { rctx in
            pulsaar_unpair_device(rctx, device.slot) == PulsaarStatusOk
        }
        if ok { reload() }
        return ok
    }

    func reload() {
        guard let ctx else { return }
        isLoading = true
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
                        devices.append(DeviceModel(c: dev, receiverIndex: i))
                    }
                }
            }

            result.append(ReceiverModel(index: i, openedInfo: rinfo, devices: devices))
        }

        receivers = result
        isLoading = false
        restartEventListeners()
    }
}
