// Observable store that owns the Rust HID session and exposes receiver/device
// data to SwiftUI views. All mutations happen on the MainActor (default isolation).
//
// PulsaarContext and PulsaarReceiverContext are opaque C types (incomplete structs),
// so Swift imports pointers to them as OpaquePointer rather than UnsafeMutablePointer<T>.

import Foundation

@Observable
final class ReceiverStore {
    var receivers: [ReceiverModel] = []
    var isLoading = false
    var errorMessage: String? = nil

    // OpaquePointer because PulsaarContext is a forward-declared (incomplete) C struct.
    // @ObservationIgnored because this pointer never needs to trigger SwiftUI updates.
    @ObservationIgnored private var ctx: OpaquePointer? = nil

    init() {
        ctx = pulsaar_init()
        guard ctx != nil else {
            errorMessage = "Could not initialize HID. Is a receiver plugged in?"
            return
        }
        reload()
    }

    deinit {
        if let ctx {
            pulsaar_destroy(ctx)
        }
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
    }
}
