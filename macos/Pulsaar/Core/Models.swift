// Swift model types that mirror the C FFI structs from the Rust core.
// These are the types that flow through the SwiftUI view hierarchy.

import Foundation

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// Read a null-terminated C byte array (stored as a Swift tuple) into a String.
func cBufToString<T>(_ tuple: T) -> String {
    withUnsafeBytes(of: tuple) { rawPtr in
        let chars = rawPtr.bindMemory(to: CChar.self)
        return String(cString: chars.baseAddress!)
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

enum ReceiverKind {
    case unifying, bolt, nano, lightSpeed

    init(byte: UInt8) {
        switch byte {
        case 1: self = .bolt
        case 2: self = .nano
        case 3: self = .lightSpeed
        default: self = .unifying
        }
    }

    var label: String {
        switch self {
        case .unifying: return "Unifying"
        case .bolt: return "Bolt"
        case .nano: return "Nano"
        case .lightSpeed: return "LightSpeed"
        }
    }
}

enum DeviceKind {
    case unknown, keyboard, mouse, numpad, presenter, remote
    case trackball, touchpad, tablet, gamepad, joystick
    case headset, remoteControl, receiver

    init(byte: UInt8) {
        switch byte {
        case 1: self = .keyboard
        case 2: self = .mouse
        case 3: self = .numpad
        case 4: self = .presenter
        case 5: self = .remote
        case 6: self = .trackball
        case 7: self = .touchpad
        case 8: self = .tablet
        case 9: self = .gamepad
        case 10: self = .joystick
        case 11: self = .headset
        case 12: self = .remoteControl
        case 13: self = .receiver
        default: self = .unknown
        }
    }

    var label: String {
        switch self {
        case .unknown: return "Unknown"
        case .keyboard: return "Keyboard"
        case .mouse: return "Mouse"
        case .numpad: return "Numpad"
        case .presenter: return "Presenter"
        case .remote: return "Remote"
        case .trackball: return "Trackball"
        case .touchpad: return "Touchpad"
        case .tablet: return "Tablet"
        case .gamepad: return "Gamepad"
        case .joystick: return "Joystick"
        case .headset: return "Headset"
        case .remoteControl: return "Remote Control"
        case .receiver: return "Receiver"
        }
    }

    var systemImage: String {
        switch self {
        case .keyboard, .numpad: return "keyboard"
        case .mouse, .trackball: return "computermouse.fill"
        case .headset: return "headphones"
        case .gamepad, .joystick: return "gamecontroller.fill"
        case .touchpad: return "hand.point.up.left.fill"
        case .presenter, .remote, .remoteControl: return "tv.remote.fill"
        case .tablet: return "pencil.and.scribble"
        case .receiver: return "antenna.radiowaves.left.and.right"
        default: return "questionmark.circle"
        }
    }
}

enum BatteryStatus: Equatable {
    case discharging, recharging, almostFull, full, slowRecharge, invalidBattery, thermalError

    init(byte: UInt8) {
        switch byte {
        case 1: self = .recharging
        case 2: self = .almostFull
        case 3: self = .full
        case 4: self = .slowRecharge
        case 5: self = .invalidBattery
        case 6: self = .thermalError
        default: self = .discharging
        }
    }

    var label: String {
        switch self {
        case .discharging: return "Not charging"
        case .recharging: return "Charging"
        case .almostFull: return "Charging (almost full)"
        case .full: return "Fully charged"
        case .slowRecharge: return "Charging slowly"
        case .invalidBattery: return "Invalid battery"
        case .thermalError: return "Thermal error"
        }
    }

    var isCharging: Bool {
        self == .recharging || self == .almostFull || self == .full || self == .slowRecharge
    }
}

// ---------------------------------------------------------------------------
// Model types
// ---------------------------------------------------------------------------

struct BatteryModel {
    let level: Int?           // nil when 0xFF (unavailable)
    let status: BatteryStatus?  // nil when 0xFF (unavailable)
    let voltage: UInt16?       // nil when 0 (unavailable)

    init(c: CBattery) {
        level  = c.level  == 0xFF ? nil : Int(c.level)
        status = c.status == 0xFF ? nil : BatteryStatus(byte: c.status)
        voltage = c.voltage == 0 ? nil : c.voltage
    }

    var levelText: String {
        if let l = level { return "\(l)%" }
        return "?"
    }

    var batterySystemImage: String {
        // Only battery.100percent.bolt exists in SF Symbols; use it for all charging states.
        // The adjacent percentage text conveys the actual level when charging.
        if status?.isCharging == true { return "battery.100percent.bolt" }
        guard let l = level else { return "battery.0percent" }
        switch l {
        case 75...: return "battery.100percent"
        case 50..<75: return "battery.75percent"
        case 25..<50: return "battery.50percent"
        case 1..<25: return "battery.25percent"
        default: return "battery.0percent"
        }
    }
}

struct DeviceModel: Identifiable {
    let id: String          // "receiverIndex-slot", stable across reloads
    let receiverIndex: Int
    let slot: UInt8
    let kind: DeviceKind
    let name: String
    let serial: String
    let battery: BatteryModel?

    // A device is considered online when battery info was successfully read.
    // For wireless devices, HID++ feature calls only succeed when connected,
    // so battery presence is a reliable proxy for connection state.
    var isOnline: Bool { battery != nil }

    init(c: CDeviceInfo, receiverIndex: Int) {
        id            = "\(receiverIndex)-\(c.slot)"
        self.receiverIndex = receiverIndex
        slot          = c.slot
        kind          = DeviceKind(byte: c.kind)
        name          = cBufToString(c.name)
        serial        = cBufToString(c.serial)
        battery       = c.has_battery != 0 ? BatteryModel(c: c.battery) : nil
    }
}

struct ReceiverModel: Identifiable {
    let id: Int             // index within the session's receiver list
    let productId: UInt16
    let kind: ReceiverKind
    let name: String
    let serial: String
    let maxDevices: UInt8
    let devices: [DeviceModel]

    init(index: Int, openedInfo: COpenedReceiverInfo, devices: [DeviceModel]) {
        id         = index
        productId  = openedInfo.product_id
        kind       = ReceiverKind(byte: openedInfo.kind)
        name       = cBufToString(openedInfo.name)
        serial     = cBufToString(openedInfo.serial)
        maxDevices = openedInfo.max_devices
        self.devices = devices
    }
}
