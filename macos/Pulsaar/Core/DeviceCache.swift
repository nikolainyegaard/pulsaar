// Persists last-known battery state per device across reloads and app launches.
// Stored as JSON in ~/Library/Application Support/Pulsaar/device-cache.json,
// keyed by device serial number.

import Foundation

struct CachedBattery: Codable {
    let level: Int?
    let statusByte: UInt8?
    let voltage: UInt16?
    let seenAt: Date
}

struct DeviceCache {
    private(set) var entries: [String: CachedBattery] = [:]
    // Full DEVICE_NAME (0x0005) keyed by serial; persisted separately so it
    // survives across app launches and is injected when the device is offline.
    private(set) var names: [String: String] = [:]
    private let url: URL
    private let namesUrl: URL

    init() {
        let appSupport = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let dir = appSupport.appendingPathComponent("Pulsaar")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        url = dir.appendingPathComponent("device-cache.json")
        namesUrl = dir.appendingPathComponent("device-names.json")
        if let data = try? Data(contentsOf: url),
           let decoded = try? JSONDecoder().decode([String: CachedBattery].self, from: data) {
            entries = decoded
        }
        if let data = try? Data(contentsOf: namesUrl),
           let decoded = try? JSONDecoder().decode([String: String].self, from: data) {
            names = decoded
        }
    }

    // Call after a successful live battery read.
    mutating func update(serial: String, battery: BatteryModel) {
        guard !serial.isEmpty else { return }
        entries[serial] = CachedBattery(
            level: battery.level,
            statusByte: battery.status?.byte,
            voltage: battery.voltage,
            seenAt: Date()
        )
        save()
    }

    func battery(for serial: String) -> CachedBattery? {
        guard !serial.isEmpty else { return nil }
        return entries[serial]
    }

    // Call when the device is online and we have the full DEVICE_NAME.
    mutating func updateName(serial: String, name: String) {
        guard !serial.isEmpty, !name.isEmpty else { return }
        guard names[serial] != name else { return }
        names[serial] = name
        saveNames()
    }

    func name(for serial: String) -> String? {
        guard !serial.isEmpty else { return nil }
        return names[serial]
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(entries) else { return }
        try? data.write(to: url, options: .atomic)
    }

    private func saveNames() {
        guard let data = try? JSONEncoder().encode(names) else { return }
        try? data.write(to: namesUrl, options: .atomic)
    }
}
