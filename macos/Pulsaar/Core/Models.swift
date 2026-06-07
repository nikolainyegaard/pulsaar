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

    var systemImage: String {
        switch self {
        case .unifying: return "sun.max.fill"
        case .bolt: return "bolt.circle.fill"
        case .lightSpeed: return "wifi.circle.fill"
        default: return "antenna.radiowaves.left.and.right"
        }
    }

    var customImageName: String? {
        switch self {
        case .bolt: return "bolt"
        case .unifying: return "unifying"
        default: return nil
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

    var byte: UInt8 {
        switch self {
        case .discharging:    return 0
        case .recharging:     return 1
        case .almostFull:     return 2
        case .full:           return 3
        case .slowRecharge:   return 4
        case .invalidBattery: return 5
        case .thermalError:   return 6
        }
    }
}

// ---------------------------------------------------------------------------
// Model types
// ---------------------------------------------------------------------------

struct BatteryModel {
    let level: Int?            // nil when 0xFF (unavailable)
    let status: BatteryStatus? // nil when 0xFF (unavailable)
    let voltage: UInt16?       // nil when 0 (unavailable)
    let isCached: Bool         // true when sourced from DeviceCache, not a live read

    init(c: CBattery) {
        level    = c.level   == 0xFF ? nil : Int(c.level)
        status   = c.status  == 0xFF ? nil : BatteryStatus(byte: c.status)
        voltage  = c.voltage == 0    ? nil : c.voltage
        isCached = false
    }

    init(cached: CachedBattery) {
        level    = cached.level
        status   = cached.statusByte.map { BatteryStatus(byte: $0) }
        voltage  = cached.voltage
        isCached = true
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
    var name: String
    let serial: String
    let productId: String   // wpid from receiver pairing data, formatted as 0xXXXX
    let receiverKind: ReceiverKind
    var battery: BatteryModel? // var so DeviceCache can inject a cached value post-init
    let isOnline: Bool         // true only when a live battery read succeeded at this reload

    var connectionLabel: String {
        switch receiverKind {
        case .bolt:       return "Bolt (Encrypted)"
        case .unifying:   return "Unifying"
        case .lightSpeed: return "LightSpeed"
        case .nano:       return "Nano"
        }
    }

    init(c: CDeviceInfo, receiverIndex: Int, receiverKind: ReceiverKind) {
        id            = "\(receiverIndex)-\(c.slot)"
        self.receiverIndex = receiverIndex
        slot          = c.slot
        kind          = DeviceKind(byte: c.kind)
        name          = cBufToString(c.name)
        serial        = cBufToString(c.serial)
        productId     = String(format: "0x%02X%02X", c.wpid.0, c.wpid.1)
        self.receiverKind = receiverKind
        let liveBattery = c.has_battery != 0 ? BatteryModel(c: c.battery) : nil
        battery       = liveBattery
        isOnline      = liveBattery != nil
    }
}

struct DirectDeviceModel: Identifiable {
    let id: String          // serial if non-empty, else "direct-<productId>"
    let productId: UInt16
    let kind: DeviceKind
    let name: String
    let serial: String
    let battery: BatteryModel?
    let isOnline: Bool      // always true: visible in device list means the device is connected
    let connectionLabel: String = "Bluetooth"

    init(c: CDirectDeviceInfo) {
        let serialStr = cBufToString(c.serial)
        id        = serialStr.isEmpty ? "direct-\(c.product_id)" : serialStr
        productId = c.product_id
        kind      = DeviceKind(byte: c.kind)
        name      = cBufToString(c.name)
        serial    = serialStr
        battery   = c.has_battery != 0 ? BatteryModel(c: c.battery) : nil
        isOnline  = true
    }
}

// ---------------------------------------------------------------------------
// Settings model enums and helpers
// ---------------------------------------------------------------------------

enum WheelMode: UInt8, CaseIterable {
    case freespin  = 1
    case smartShift = 2

    var label: String {
        switch self {
        case .freespin:   return "Freespin"
        case .smartShift: return "Ratchet"
        }
    }
}

struct HostInfo: Identifiable {
    let id: UInt8  // slot index
    let name: String
    let isActive: Bool
}

struct OSPlatform: Identifiable {
    let id: UInt8       // raw platform_index from device
    let name: String
}

enum BacklightMode: UInt8, CaseIterable {
    case disabled  = 0
    case automatic = 1
    case manual    = 3

    var label: String {
        switch self {
        case .disabled:  return "Off"
        case .automatic: return "Automatic"
        case .manual:    return "Always on"
        }
    }
}

// ---------------------------------------------------------------------------
// Settings model
// ---------------------------------------------------------------------------

struct DeviceSettingsModel {
    // DPI (FEAT_ADJUSTABLE_DPI 0x2201)
    let dpiList:    [Int]
    var currentDpi: Int
    let defaultDpi: Int

    // Scroll wheel (FEAT_HIRES_WHEEL 0x2121)
    let hasInvert:      Bool
    let hasHires:       Bool
    var scrollInverted: Bool
    var hiresEnabled:   Bool

    // SmartShift (FEAT_SMART_SHIFT_ENHANCED 0x2111)
    var wheelMode:     WheelMode?
    let hasTorque:     Bool
    var smartShiftTorque: Int

    // Change Host (FEAT_CHANGE_HOST 0x1814)
    var hosts: [HostInfo]?

    // FN key swap (FEAT_FN_INVERSION family)
    var fnSwapped: Bool?

    // Multiplatform / Set OS (FEAT_MULTIPLATFORM 0x4531)
    var platforms:     [OSPlatform]?
    var currentOsIdx:  Int          // index into platforms array

    // Backlight (FEAT_BACKLIGHT2 0x1982)
    var backlightMode:       BacklightMode?
    let backlightAutoSupported:   Bool
    let backlightManualSupported: Bool
    var backlightBrightness: Int

    // Feature index of REPROG_CONTROLS_V4 (0x1B04), or 0 if absent.
    // Used to filter spurious SettingsChanged events triggered by persistent
    // button diversions (mouse clicks) vs. actual settings changes.
    let reprogControlsIdx: UInt8

    var hasDpi: Bool            { !dpiList.isEmpty }
    var hasScrollSettings: Bool { hasInvert || hasHires }
    var hasSmartShift: Bool     { wheelMode != nil }
    var hasHosts: Bool          { !(hosts?.isEmpty ?? true) }
    var hasFnSwap: Bool         { fnSwapped != nil }
    var hasMultiplatform: Bool  { !(platforms?.isEmpty ?? true) }
    var hasBacklight: Bool      { backlightMode != nil }
    var hasAnySettings: Bool    { hasDpi || hasScrollSettings || hasSmartShift || hasHosts || hasFnSwap || hasMultiplatform || hasBacklight }

    init?(dpi: CDpiSettings, scroll: CScrollSettings, smartShift: CSmartShiftSettings, hosts hostList: CHostList, fn fnSettings: CFnSettings, mp: CMultiplatformSettings, backlight bl: CBacklightSettings, reprogControlsIdx: UInt8 = 0) {
        // DPI
        if dpi.dpi_count > 0 {
            dpiList = withUnsafeBytes(of: dpi.dpi_list) { raw in
                let words = raw.bindMemory(to: UInt16.self)
                return (0..<Int(dpi.dpi_count)).map { Int(words[$0]) }
            }
            currentDpi = Int(dpi.current_dpi)
            defaultDpi = Int(dpi.default_dpi)
        } else {
            dpiList    = []
            currentDpi = 0
            defaultDpi = 0
        }

        // Scroll wheel
        hasInvert      = scroll.has_invert    != 0
        hasHires       = scroll.has_hires     != 0
        scrollInverted = scroll.inverted      != 0
        hiresEnabled   = scroll.hires_enabled != 0

        // SmartShift
        if smartShift.wheel_mode != 0 {
            wheelMode        = WheelMode(rawValue: smartShift.wheel_mode) ?? .smartShift
            hasTorque        = smartShift.has_torque != 0
            smartShiftTorque = Int(smartShift.torque)
        } else {
            wheelMode        = nil
            hasTorque        = false
            smartShiftTorque = 50
        }

        // Hosts
        if hostList.count > 0 {
            hosts = withUnsafeBytes(of: hostList.hosts) { raw in
                let arr = raw.bindMemory(to: CHostInfo.self)
                return (0..<Int(hostList.count)).map { i in
                    HostInfo(id: arr[i].slot, name: cBufToString(arr[i].name), isActive: arr[i].is_active != 0)
                }
            }
        } else {
            hosts = nil
        }

        // FN swap: nil means feature absent (has_feature=0); non-nil Bool is the actual state.
        fnSwapped = fnSettings.has_feature != 0 ? (fnSettings.fn_swapped != 0) : nil

        // Multiplatform
        if mp.count >= 2 {
            let count = Int(mp.count)
            var platformList: [OSPlatform] = []
            withUnsafeBytes(of: mp.platform_names) { rawNames in
                withUnsafeBytes(of: mp.platform_indices) { rawIdx in
                    let idxBytes = rawIdx.bindMemory(to: UInt8.self)
                    for i in 0..<count {
                        let nameStart = i * 32
                        let nameSlice = rawNames[nameStart..<(nameStart + 32)]
                        let nameStr = String(bytes: nameSlice.prefix(while: { $0 != 0 }), encoding: .utf8) ?? "Platform \(i+1)"
                        platformList.append(OSPlatform(id: idxBytes[i], name: nameStr))
                    }
                }
            }
            platforms    = platformList
            currentOsIdx = Int(mp.current)
        } else {
            platforms    = nil
            currentOsIdx = 0
        }

        // Backlight: nil means feature absent (has_feature=0).
        if bl.has_feature != 0 {
            backlightMode            = BacklightMode(rawValue: bl.mode) ?? .disabled
            backlightAutoSupported   = bl.auto_supported != 0
            backlightManualSupported = bl.manual_supported != 0
            backlightBrightness      = Int(bl.brightness)
        } else {
            backlightMode            = nil
            backlightAutoSupported   = false
            backlightManualSupported = false
            backlightBrightness      = 0
        }

        self.reprogControlsIdx = reprogControlsIdx

        // Nil when no settings at all.
        if !hasAnySettings { return nil }
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
