import SwiftUI
import AppKit

// Sidebar selection: a receiver row, a receiver-hosted device row, or a direct BT device row.
enum SidebarItem: Hashable {
    case bluetooth
    case receiver(Int)
    case device(String)        // DeviceModel.id ("receiverIndex-slot")
    case directDevice(String)  // DirectDeviceModel.id (serial or "direct-<pid>")
}

struct ContentView: View {
    @Environment(ReceiverStore.self) private var store
    @State private var selection: SidebarItem? = nil

    var body: some View {
        NavigationSplitView {
            sidebar
                .navigationSplitViewColumnWidth(min: 240, ideal: 300)
        } detail: {
            detailPane
                .navigationSplitViewColumnWidth(min: 320, ideal: 480)
                .overlay(alignment: .bottom) {
                    if let msg = store.toastMessage {
                        ToastView(message: msg)
                            .padding(.bottom, 24)
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    }
                }
                .animation(.easeInOut(duration: 0.2), value: store.toastMessage)
        }
    }

    // MARK: - Sidebar

    @ViewBuilder
    private var sidebar: some View {
        List(selection: $selection) {
            if store.isLoading {
                Label("Scanning...", systemImage: "arrow.clockwise")
                    .foregroundStyle(.secondary)
            } else if let error = store.errorMessage {
                Label(error, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.secondary)
            } else if store.receivers.isEmpty && store.directDevices.isEmpty {
                Label("No devices found", systemImage: "antenna.radiowaves.left.and.right")
                    .foregroundStyle(.secondary)
            } else {
                let btName = Host.current().localizedName ?? "Bluetooth"
                let hasBluetooth = !store.directDevices.isEmpty

                // Build a sorted list of parent items: bluetooth row + receivers, sorted by display name.
                // Each entry is either a receiver index or nil (= bluetooth).
                let sortedParents: [Int?] = {
                    var items: [(name: String, receiverIndex: Int?)] = store.receivers.map { ($0.name, $0.id) }
                    if hasBluetooth { items.append((btName, nil)) }
                    return items.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }.map { $0.receiverIndex }
                }()

                ForEach(sortedParents, id: \.self) { receiverIndex in
                    if let receiverIndex {
                        if let receiver = store.receivers.first(where: { $0.id == receiverIndex }) {
                            ReceiverSidebarRow(receiver: receiver)
                                .tag(SidebarItem.receiver(receiver.id))
                            ForEach(Array(receiver.devices.enumerated()), id: \.element.id) { index, device in
                                DeviceSidebarRow(device: device, isLast: index == receiver.devices.count - 1)
                                    .tag(SidebarItem.device(device.id))
                                    .listRowInsets(EdgeInsets(top: 3, leading: 16, bottom: 3, trailing: 8))
                            }
                        }
                    } else {
                        BluetoothSidebarRow()
                            .tag(SidebarItem.bluetooth)
                        ForEach(Array(store.directDevices.enumerated()), id: \.element.id) { index, device in
                            DirectDeviceSidebarRow(device: device, isLast: index == store.directDevices.count - 1)
                                .tag(SidebarItem.directDevice(device.id))
                                .listRowInsets(EdgeInsets(top: 3, leading: 16, bottom: 3, trailing: 8))
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .navigationTitle("Pulsaar")
    }

    // MARK: - Detail pane

    @ViewBuilder
    private var detailPane: some View {
        switch selection {
        case .bluetooth:
            BluetoothDetailView(devices: store.directDevices, selection: $selection)
        case .device(let id):
            if let device = findDevice(id: id) {
                DeviceDetailView(device: device)
            } else {
                emptyState
            }
        case .receiver(let id):
            if let receiver = store.receivers.first(where: { $0.id == id }) {
                ReceiverDetailView(receiver: receiver, selection: $selection)
            } else {
                emptyState
            }
        case .directDevice(let id):
            if let device = store.directDevices.first(where: { $0.id == id }) {
                DirectDeviceDetailView(device: device)
            } else {
                emptyState
            }
        case nil:
            emptyState
        }
    }

    private var emptyState: some View {
        ContentUnavailableView(
            "Select a device",
            systemImage: "dot.radiowaves.left.and.right",
            description: Text("Choose a device from the list to see its details.")
        )
    }

    private func findDevice(id: String) -> DeviceModel? {
        store.receivers.flatMap(\.devices).first { $0.id == id }
    }
}

// MARK: - Toast

private struct ToastView: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "checkmark.circle.fill")
            .font(.subheadline)
            .symbolRenderingMode(.multicolor)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .background(.regularMaterial, in: Capsule())
            .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
    }
}

// MARK: - Receiver sidebar row

struct ReceiverSidebarRow: View {
    let receiver: ReceiverModel

    var body: some View {
        Label {
            Text(receiver.name)
                .fontWeight(.medium)
        } icon: {
            if let name = receiver.kind.customImageName {
                Image(name)
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 16, height: 16)
            } else {
                Image(systemName: receiver.kind.systemImage)
            }
        }
    }
}

// MARK: - Bluetooth sidebar row

struct BluetoothSidebarRow: View {
    private var hostName: String {
        Host.current().localizedName ?? "Bluetooth"
    }

    var body: some View {
        Label {
            Text(hostName)
                .fontWeight(.medium)
        } icon: {
            Image("bluetooth")
                .renderingMode(.template)
                .resizable()
                .scaledToFit()
                .frame(width: 16, height: 16)
        }
    }
}

// MARK: - Device sidebar row

struct TreeConnector: View {
    let isLast: Bool

    var body: some View {
        Canvas { context, size in
            let midX = size.width / 2
            let midY = size.height / 2
            let style = StrokeStyle(lineWidth: 1, lineCap: .round, dash: [2, 3])
            let shading = GraphicsContext.Shading.color(.gray.opacity(0.5))

            var vert = Path()
            vert.move(to: CGPoint(x: midX, y: 0))
            vert.addLine(to: CGPoint(x: midX, y: isLast ? midY : size.height))
            context.stroke(vert, with: shading, style: style)

            var horiz = Path()
            horiz.move(to: CGPoint(x: midX, y: midY))
            horiz.addLine(to: CGPoint(x: size.width, y: midY))
            context.stroke(horiz, with: shading, style: style)
        }
    }
}

struct DeviceSidebarRow: View {
    let device: DeviceModel
    let isLast: Bool

    var body: some View {
        HStack(spacing: 0) {
            TreeConnector(isLast: isLast)
                .frame(width: 28)
            Label {
                Text(device.name)
            } icon: {
                Image(systemName: device.kind.systemImage)
                    .foregroundStyle(device.isOnline ? .primary : .secondary)
            }

            Spacer()

            if let battery = device.battery {
                HStack(spacing: 3) {
                    if battery.isCached {
                        Image(systemName: "clock")
                            .font(.caption2)
                    }
                    Text(battery.levelText)
                        .font(.caption2)
                    Image(systemName: battery.batterySystemImage)
                        .font(.caption2)
                }
                .foregroundStyle(sidebarBatteryColor(battery))
            }
        }
        .opacity(device.isOnline ? 1.0 : 0.45)
    }
}

// MARK: - Shared battery color helpers

private func sidebarBatteryColor(_ battery: BatteryModel) -> Color {
    if battery.isCached { return .secondary }
    if battery.status?.isCharging == true { return .green }
    guard let level = battery.level else { return .secondary }
    if level <= 10 { return .red }
    if level <= 25 { return .orange }
    return .secondary
}

private func batteryColor(_ battery: BatteryModel) -> Color {
    if battery.isCached { return .secondary }
    if battery.status?.isCharging == true { return .green }
    guard let level = battery.level else { return .secondary }
    if level <= 10 { return .red }
    if level <= 25 { return .orange }
    return .primary
}

// MARK: - Shared device detail components

private struct DeviceHeader: View {
    let name: String
    let kindImage: String
    let isOnline: Bool
    let battery: BatteryModel?

    var body: some View {
        HStack(spacing: 20) {
            Image(systemName: kindImage)
                .font(.system(size: 44))
                .foregroundStyle(isOnline ? .primary : .secondary)
                .frame(width: 56, height: 56)

            VStack(alignment: .leading, spacing: 6) {
                Text(name)
                    .font(.title2)
                    .fontWeight(.semibold)
                    .lineLimit(1)
                    .truncationMode(.tail)

                HStack(spacing: 6) {
                    Circle()
                        .fill(isOnline ? Color.green : Color.secondary)
                        .frame(width: 8, height: 8)
                    Text(isOnline ? "Online" : "Offline")
                        .font(.subheadline)
                        .foregroundStyle(isOnline ? .primary : .secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if let battery {
                VStack(alignment: .trailing, spacing: 4) {
                    Image(systemName: battery.batterySystemImage)
                        .font(.system(size: 28))
                        .foregroundStyle(batteryColor(battery))
                        .frame(height: 28)
                    Text(battery.levelText)
                        .font(.headline)
                        .foregroundStyle(batteryColor(battery))
                }
                .frame(minWidth: 56, alignment: .trailing)
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary)
    }
}

// MARK: - Device detail

struct DeviceDetailView: View {
    let device: DeviceModel
    @Environment(ReceiverStore.self) private var store
    @State private var showingInfo = false
    @State private var showingUnpairConfirm = false
    @State private var unpairFailed = false
    @State private var containerHeight: CGFloat = 480

    var body: some View {
        VStack(spacing: 0) {
            DeviceHeader(
                name: device.name,
                kindImage: device.kind.systemImage,
                isOnline: device.isOnline,
                battery: device.battery
            )
            Divider()
            DeviceSettingsPanel(device: device)
            Divider()
            Button(role: .destructive) {
                showingUnpairConfirm = true
            } label: {
                Label("Unpair device", systemImage: "minus.circle")
                    .frame(maxWidth: .infinity, alignment: .center)
            }
            .disabled(store.isPrefetching)
            .padding()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background {
            GeometryReader { geo in
                Color.clear
                    .onAppear { containerHeight = geo.size.height }
                    .onChange(of: geo.size) { _, new in containerHeight = new.height }
            }
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { showingInfo = true } label: {
                    Label("Device info", systemImage: "info.circle")
                }
            }
        }
        .sheet(isPresented: $showingInfo) {
            NavigationStack {
                DeviceInfoSheet(device: device)
                    .navigationTitle(device.name)
            }
            .frame(width: 360, height: containerHeight * 0.8)
        }
        .confirmationDialog(
            "Unpair \(device.name)?",
            isPresented: $showingUnpairConfirm,
            titleVisibility: .visible
        ) {
            Button("Unpair", role: .destructive) {
                if !store.unpair(device: device) {
                    unpairFailed = true
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The device will be removed from this receiver. You can pair it again later.")
        }
        .alert("Unpair failed", isPresented: $unpairFailed) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("The receiver did not acknowledge the unpair request.")
        }
    }
}

// MARK: - Device settings panel

// Log-scale DPI slider. Thumb moves continuously during drag; displayed DPI snaps to the
// nearest value from a curated increment scheme. HID write fires once on release.
private struct LogDpiSlider: View {
    let dpiList: [Int]
    @Binding var currentDpi: Int
    let onRelease: (Int) -> Void

    // Raw 0..1 fraction while dragging; nil when at rest.
    @State private var dragFrac: Double? = nil

    private var logMin: Double { log(Double(dpiList.first ?? 200)) }
    private var logMax: Double { log(Double(dpiList.last  ?? 8000)) }

    private func frac(for dpi: Int) -> Double {
        guard logMax > logMin else { return 0 }
        let t = (log(Double(dpi)) - logMin) / (logMax - logMin)
        return pow(t, 1.5)
    }

    private func xPos(for dpi: Int, width: CGFloat) -> CGFloat {
        CGFloat(frac(for: dpi)) * width
    }

    // Curated snap points using the preferred increment scheme, mapped onto the
    // device's actual supported values. Falls back to the full list for devices
    // with few DPI options.
    private var snapList: [Int] {
        guard let lo = dpiList.first, let hi = dpiList.last, dpiList.count > 20 else { return dpiList }
        var candidates: [Int] = []
        var v = 50
        while v <= 1000 { candidates.append(v); v +=  50 }
        v = 1100
        while v <= 2000 { candidates.append(v); v += 100 }
        v = 2250
        while v <= 4000 { candidates.append(v); v += 250 }
        v = 4500
        while v <= 8000 { candidates.append(v); v += 500 }
        let inRange = candidates.filter { $0 >= lo && $0 <= hi }
        var mapped = Array(Set(inRange.compactMap { c in
            dpiList.min(by: { abs($0 - c) < abs($1 - c) })
        })).sorted()
        if mapped.isEmpty { return dpiList }
        if mapped.first != lo { mapped.insert(lo, at: 0) }
        if mapped.last  != hi { mapped.append(hi) }
        return mapped
    }

    // Tick and label positions: the six landmark values clipped to the device range,
    // always including the device min and max.
    private var landmarks: [Int] {
        guard let lo = dpiList.first, let hi = dpiList.last else { return [] }
        let base = [200, 600, 1000, 2000, 4000, 8000].filter { $0 > lo && $0 < hi }
        return ([lo] + base + [hi]).sorted()
    }

    private func dpiAt(frac t: Double) -> Int {
        let clamped = max(0.0, min(1.0, t))
        let logT = pow(clamped, 1.0 / 1.5)  // invert the gamma
        let logVal = logMin + logT * (logMax - logMin)
        let target = Int(exp(logVal).rounded())
        return snapList.min(by: { abs($0 - target) < abs($1 - target) }) ?? currentDpi
    }

    // Thumb position fraction: raw drag position during drag, snapped at rest.
    private var thumbFrac: Double { dragFrac ?? frac(for: currentDpi) }

    // DPI shown in the readout: nearest snap value during drag, committed value at rest.
    private var displayedDpi: Int {
        if let t = dragFrac { return dpiAt(frac: t) }
        return currentDpi
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            // DPI readout: snaps to nearest snap value while dragging.
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Text(String(displayedDpi))
                    .font(.title2)
                    .fontWeight(.bold)
                    .monospacedDigit()
                Text("DPI")
                    .foregroundStyle(.secondary)
            }

            // Track, ticks, and thumb in a single GeometryReader.
            GeometryReader { geo in
                let w = geo.size.width
                let tx = CGFloat(max(0.0, min(1.0, thumbFrac))) * w

                ZStack(alignment: .topLeading) {
                    // Track background
                    Capsule()
                        .fill(Color.secondary.opacity(0.25))
                        .frame(width: w, height: 4)
                        .offset(y: 8)

                    // Filled portion left of thumb
                    if tx > 0 {
                        Capsule()
                            .fill(Color.accentColor.opacity(0.5))
                            .frame(width: tx, height: 4)
                            .offset(y: 8)
                    }

                    // Tick mark at every landmark
                    ForEach(landmarks, id: \.self) { dpi in
                        Capsule()
                            .fill(Color.secondary.opacity(0.35))
                            .frame(width: 1.5, height: 6)
                            .offset(x: xPos(for: dpi, width: w) - 0.75, y: 20)
                    }

                    // Thumb: follows raw drag position continuously.
                    Circle()
                        .fill(.white)
                        .overlay(Circle().strokeBorder(Color.gray.opacity(0.2), lineWidth: 0.5))
                        .shadow(color: .black.opacity(0.18), radius: 2, y: 1)
                        .frame(width: 20, height: 20)
                        .offset(x: tx - 10)
                }
                .frame(height: 36)
                .contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { v in
                        dragFrac = Double(v.location.x / w)
                    }
                    .onEnded { v in
                        let snapped = dpiAt(frac: Double(v.location.x / w))
                        dragFrac = nil
                        currentDpi = snapped
                        onRelease(snapped)
                    }
                )
            }
            .frame(height: 36)

            // Axis labels at landmark positions. Canvas gives us real text measurement
            // so each label can be placed precisely: left-edge-aligned on the first,
            // right-edge-aligned on the last, centered on all others.
            Canvas { context, size in
                let w = size.width
                for (i, dpi) in landmarks.enumerated() {
                    let label = context.resolve(
                        Text(String(dpi)).font(.caption2).foregroundStyle(.secondary)
                    )
                    let tw = label.measure(in: size).width
                    let x = CGFloat(frac(for: dpi)) * w
                    let drawX = i == 0 ? x : i == landmarks.count - 1 ? x - tw : x - tw / 2
                    context.draw(label, at: CGPoint(x: drawX, y: 0), anchor: .topLeading)
                }
            }
            .frame(height: 14)
        }
        .padding(.vertical, 4)
    }
}

private struct DeviceSettingsPanel: View {
    let device: DeviceModel
    @Environment(ReceiverStore.self) private var store
    @State private var settings: DeviceSettingsModel? = nil
    @State private var isLoading = true
    // Phase 1
    @State private var currentDpi: Int = 0
    @State private var scrollInverted: Bool = false
    @State private var hiresEnabled: Bool = false
    // Phase 2
    @State private var wheelMode: WheelMode = .smartShift
    @State private var smartShiftTorque: Int = 50
    @State private var smartShiftTorqueDrag: Double? = nil
    @State private var currentHostIdx: Int = 0
    @State private var fnSwapped: Bool = false
    @State private var currentOsIdx: Int = 0
    @State private var backlightMode: BacklightMode = .disabled
    @State private var backlightBrightness: Int = 50

    var body: some View {
        Group {
            if isLoading {
                Color.clear.overlay {
                    ProgressView("Reading settings...")
                        .foregroundStyle(.secondary)
                }
            } else if let s = settings, s.hasAnySettings {
                settingsForm(s)
            } else {
                Color.clear.overlay {
                    VStack(spacing: 10) {
                        Image(systemName: "slider.horizontal.3")
                            .font(.system(size: 32))
                            .foregroundStyle(.quaternary)
                        Text("No configurable settings")
                            .foregroundStyle(.quaternary)
                    }
                }
            }
        }
        .task(id: device.id) {
            // Clear stale state from any previously selected device before loading new one.
            settings = nil
            isLoading = true
            // Serve from cache immediately if available -- no HID round-trip needed.
            if let cached = store.settingsCache[device.id] {
                pLog("SETTINGS", "task '\(device.name)': cache HIT -> applying")
                applySettings(cached)
                isLoading = false
                return
            }
            // Cache miss. If prefetch is still running it holds the receiver open;
            // wait for it to finish and then try the cache again before opening independently.
            if store.isPrefetching {
                pLog("SETTINGS", "task '\(device.name)': cache MISS but prefetch running -- waiting")
                isLoading = true
                for _ in 0..<24 {
                    try? await Task.sleep(nanoseconds: 250_000_000)
                    if !store.isPrefetching { break }
                }
                if let cached = store.settingsCache[device.id] {
                    pLog("SETTINGS", "task '\(device.name)': cache HIT after prefetch wait -> applying")
                    applySettings(cached)
                    isLoading = false
                    return
                }
                pLog("SETTINGS", "task '\(device.name)': still no cache after wait, loading independently")
            }
            // Load from device.
            pLog("SETTINGS", "task '\(device.name)': cache MISS -> loading from device")
            isLoading = true
            store.pauseEventPolling()
            let capturedDevice = device
            let capturedStore  = store
            let result = await withCheckedContinuation { continuation in
                DispatchQueue.global(qos: .userInitiated).async {
                    continuation.resume(returning: capturedStore.loadSettings(for: capturedDevice))
                }
            }
            store.resumeEventPolling()
            if let s = result {
                pLog("SETTINGS", "task '\(device.name)': loaded ok, applying + caching")
                applySettings(s)
                store.settingsCache[capturedDevice.id] = s
            } else {
                pLog("SETTINGS", "task '\(device.name)': load returned nil")
                // prefetch may have populated the cache while this load was in flight
                if let cached = store.settingsCache[capturedDevice.id] {
                    pLog("SETTINGS", "task '\(device.name)': found cache after failed load -> applying")
                    applySettings(cached)
                    isLoading = false
                    return
                }
            }
            settings  = result
            isLoading = false
        }
        .onChange(of: store.settingsCacheVersion) { _, _ in
            // The event listener detected a device-initiated settings change (FN toggle,
            // scroll mode button, etc.) and refreshed the cache in the background.
            // Re-apply only if we are not in the middle of an initial load.
            guard !isLoading, let updated = store.settingsCache[device.id] else { return }
            pLog("SETTINGS", "'\(device.name)': cache updated by device event -> applying")
            applySettings(updated)
        }
    }

    private func applySettings(_ s: DeviceSettingsModel) {
        settings            = s
        currentDpi          = s.currentDpi
        scrollInverted      = s.scrollInverted
        hiresEnabled        = s.hiresEnabled
        if let wm = s.wheelMode         { wheelMode           = wm }
        smartShiftTorque    = s.smartShiftTorque
        if let hosts = s.hosts, let activeIdx = hosts.firstIndex(where: { $0.isActive }) {
            currentHostIdx = activeIdx
        }
        if let fn = s.fnSwapped         { fnSwapped           = fn }
        currentOsIdx        = s.currentOsIdx
        if let blMode = s.backlightMode { backlightMode       = blMode }
        backlightBrightness = s.backlightBrightness
        pLog("SETTINGS", "applySettings: dpi=\(currentDpi) scrollInv=\(scrollInverted) hires=\(hiresEnabled) wheelMode=\(wheelMode.label) fnSwapped=\(fnSwapped) backlightMode=\(backlightMode.label) brightness=\(backlightBrightness) hostIdx=\(currentHostIdx) osIdx=\(currentOsIdx)")
    }

    // MARK: Write functions

    private func writeDpi(_ dpi: Int) {
        pLog("WRITE", "UI -> writeDpi dpi=\(dpi) device='\(device.name)'")
        let capturedDevice = device
        let capturedStore  = store
        capturedStore.pauseEventPolling()
        DispatchQueue.global(qos: .userInitiated).async {
            capturedStore.setDpi(for: capturedDevice, dpi: dpi)
            DispatchQueue.main.async { capturedStore.resumeEventPolling() }
        }
    }

    private func writeScrollSettings(inverted: Bool, hires: Bool) {
        pLog("WRITE", "UI -> writeScrollSettings inverted=\(inverted) hires=\(hires) device='\(device.name)'")
        let capturedDevice = device
        let capturedStore  = store
        capturedStore.pauseEventPolling()
        DispatchQueue.global(qos: .userInitiated).async {
            capturedStore.setScrollSettings(for: capturedDevice, inverted: inverted, hires: hires)
            DispatchQueue.main.async { capturedStore.resumeEventPolling() }
        }
    }

    private func writeSmartShift(wheelMode: UInt8, torque: UInt8) {
        pLog("WRITE", "UI -> writeSmartShift wheelMode=\(wheelMode) torque=\(torque) device='\(device.name)'")
        let capturedDevice = device
        let capturedStore  = store
        capturedStore.pauseEventPolling()
        DispatchQueue.global(qos: .userInitiated).async {
            capturedStore.setSmartShift(for: capturedDevice, wheelMode: wheelMode, torque: torque)
            DispatchQueue.main.async { capturedStore.resumeEventPolling() }
        }
    }

    private func writeActiveHost(hostSlot: UInt8) {
        pLog("WRITE", "UI -> writeActiveHost hostSlot=\(hostSlot) device='\(device.name)'")
        let capturedDevice = device
        let capturedStore  = store
        capturedStore.pauseEventPolling()
        DispatchQueue.global(qos: .userInitiated).async {
            capturedStore.setActiveHost(for: capturedDevice, hostSlot: hostSlot)
            DispatchQueue.main.async { capturedStore.resumeEventPolling() }
        }
    }

    private func writeFnSwap(swapped: Bool) {
        pLog("WRITE", "UI -> writeFnSwap swapped=\(swapped) device='\(device.name)'")
        let capturedDevice = device
        let capturedStore  = store
        capturedStore.pauseEventPolling()
        DispatchQueue.global(qos: .userInitiated).async {
            capturedStore.setFnSwap(for: capturedDevice, swapped: swapped)
            DispatchQueue.main.async { capturedStore.resumeEventPolling() }
        }
    }

    private func writeMultiplatform(platformIndex: UInt8) {
        pLog("WRITE", "UI -> writeMultiplatform platformIndex=\(platformIndex) device='\(device.name)'")
        let capturedDevice = device
        let capturedStore  = store
        capturedStore.pauseEventPolling()
        DispatchQueue.global(qos: .userInitiated).async {
            capturedStore.setMultiplatform(for: capturedDevice, platformIndex: platformIndex)
            DispatchQueue.main.async { capturedStore.resumeEventPolling() }
        }
    }

    private func writeBacklight(mode: UInt8, brightness: UInt8) {
        pLog("WRITE", "UI -> writeBacklight mode=\(mode) brightness=\(brightness) device='\(device.name)'")
        let capturedDevice = device
        let capturedStore  = store
        capturedStore.pauseEventPolling()
        DispatchQueue.global(qos: .userInitiated).async {
            capturedStore.setBacklight(for: capturedDevice, mode: mode, brightness: brightness)
            DispatchQueue.main.async { capturedStore.resumeEventPolling() }
        }
    }

    // MARK: Settings form

    @ViewBuilder
    private func settingsForm(_ s: DeviceSettingsModel) -> some View {
        Form {
            // Sensitivity (mice)
            if s.hasDpi {
                Section("Sensitivity") {
                    LogDpiSlider(
                        dpiList: s.dpiList,
                        currentDpi: $currentDpi,
                        onRelease: writeDpi
                    )
                }
            }

            // Scroll Wheel (mice)
            if s.hasScrollSettings || s.hasSmartShift {
                Section("Scroll Wheel") {
                    if s.hasSmartShift {
                        Picker("Scroll mode", selection: Binding(
                            get: { wheelMode },
                            set: { newMode in
                                wheelMode = newMode
                                writeSmartShift(wheelMode: newMode.rawValue, torque: UInt8(smartShiftTorque))
                            }
                        )) {
                            ForEach(WheelMode.allCases, id: \.self) { mode in
                                Text(mode.label).tag(mode)
                            }
                        }
                        if s.hasTorque && wheelMode == .smartShift {
                            HStack(spacing: 12) {
                                Text("Ratchet threshold")
                                Slider(
                                    value: Binding(
                                        get: { smartShiftTorqueDrag ?? Double(smartShiftTorque) },
                                        set: { smartShiftTorqueDrag = $0 }
                                    ),
                                    in: 1...100,
                                    onEditingChanged: { editing in
                                        if !editing {
                                            let snapped = max(5, min(100, Int(((smartShiftTorqueDrag ?? Double(smartShiftTorque)) / 5.0).rounded()) * 5))
                                            smartShiftTorque = snapped
                                            smartShiftTorqueDrag = nil
                                            writeSmartShift(wheelMode: wheelMode.rawValue, torque: UInt8(snapped))
                                        }
                                    }
                                )
                                Text("\(max(5, min(100, Int(((smartShiftTorqueDrag ?? Double(smartShiftTorque)) / 5.0).rounded()) * 5)))%")
                                    .monospacedDigit()
                                    .foregroundStyle(.secondary)
                                    .frame(width: 40, alignment: .trailing)
                            }
                        }
                    }
                    if s.hasInvert {
                        Toggle("Invert scroll direction", isOn: Binding(
                            get: { scrollInverted },
                            set: { newVal in
                                scrollInverted = newVal
                                writeScrollSettings(inverted: newVal, hires: hiresEnabled)
                            }
                        ))
                    }
                    if s.hasHires {
                        Toggle("High-resolution scrolling", isOn: Binding(
                            get: { hiresEnabled },
                            set: { newVal in
                                hiresEnabled = newVal
                                writeScrollSettings(inverted: scrollInverted, hires: newVal)
                            }
                        ))
                    }
                }
            }

            // Keyboard section
            if s.hasFnSwap || s.hasMultiplatform {
                Section("Keyboard") {
                    if s.hasFnSwap {
                        Toggle("Swap function keys", isOn: Binding(
                            get: { fnSwapped },
                            set: { newVal in
                                fnSwapped = newVal
                                writeFnSwap(swapped: newVal)
                            }
                        ))
                    }
                    if s.hasMultiplatform, let platforms = s.platforms {
                        Picker("Set OS", selection: Binding(
                            get: { currentOsIdx },
                            set: { newIdx in
                                guard newIdx < platforms.count else { return }
                                currentOsIdx = newIdx
                                writeMultiplatform(platformIndex: platforms[newIdx].id)
                            }
                        )) {
                            ForEach(Array(platforms.enumerated()), id: \.offset) { idx, platform in
                                Text(platform.name).tag(idx)
                            }
                        }
                    }
                }
            }

            // Backlight (keyboards)
            if s.hasBacklight {
                Section("Backlight") {
                    Picker("Mode", selection: Binding(
                        get: { backlightMode },
                        set: { newMode in
                            backlightMode = newMode
                            writeBacklight(mode: newMode.rawValue, brightness: UInt8(backlightBrightness))
                        }
                    )) {
                        Text(BacklightMode.disabled.label).tag(BacklightMode.disabled)
                        Text(BacklightMode.automatic.label).tag(BacklightMode.automatic)
                        Text(BacklightMode.manual.label).tag(BacklightMode.manual)
                    }
                    if backlightMode == .manual {
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                Text("Brightness")
                                Spacer()
                                Text("\(backlightBrightness)%")
                                    .monospacedDigit()
                                    .foregroundStyle(.secondary)
                            }
                            Slider(
                                value: Binding(
                                    get: { Double(backlightBrightness) },
                                    set: { backlightBrightness = Int($0) }
                                ),
                                in: 0...100,
                                onEditingChanged: { editing in
                                    if !editing {
                                        writeBacklight(mode: backlightMode.rawValue, brightness: UInt8(backlightBrightness))
                                    }
                                }
                            )
                        }
                        .listRowSeparator(.hidden)
                    }
                }
            }

            // Connectivity: Change Host (mice and keyboards)
            if s.hasHosts, let hosts = s.hosts {
                Section("Connectivity") {
                    Picker("Switch to", selection: Binding(
                        get: { currentHostIdx },
                        set: { newIdx in
                            guard newIdx < hosts.count else { return }
                            currentHostIdx = newIdx
                            writeActiveHost(hostSlot: hosts[newIdx].id)
                        }
                    )) {
                        ForEach(Array(hosts.enumerated()), id: \.offset) { idx, host in
                            let label = host.name.isEmpty ? "Host \(host.id + 1)" : host.name
                            Text(label).tag(idx)
                        }
                    }
                }
            }

        }
        .formStyle(.grouped)
    }
}

private struct DeviceInfoSheet: View {
    let device: DeviceModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        List {
            if let battery = device.battery {
                Section(battery.isCached ? "Battery (last seen)" : "Battery") {
                    LabeledContent("Level", value: battery.levelText)
                    if let status = battery.status {
                        LabeledContent("Status", value: status.label)
                    }
                    if let voltage = battery.voltage {
                        LabeledContent("Voltage", value: "\(voltage) mV")
                    }
                }
            }
            Section("Device") {
                LabeledContent("Type", value: device.kind.label)
                LabeledContent("Connection", value: device.connectionLabel)
                LabeledContent("Product ID", value: device.productId)
                LabeledContent("Slot", value: "\(device.slot)")
                if !device.serial.isEmpty {
                    LabeledContent("Serial") {
                        Text(device.serial)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
            }
        }
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
    }
}

// MARK: - Receiver header

private struct ReceiverHeader: View {
    let receiver: ReceiverModel

    var body: some View {
        HStack(spacing: 20) {
            if let name = receiver.kind.customImageName {
                Image(name)
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .foregroundStyle(.primary)
                    .frame(width: 44, height: 44)
                    .frame(width: 56, height: 56)
            } else {
                Image(systemName: receiver.kind.systemImage)
                    .font(.system(size: 44))
                    .foregroundStyle(.primary)
                    .frame(width: 56, height: 56)
            }

            VStack(alignment: .leading, spacing: 6) {
                Text(receiver.name)
                    .font(.title2)
                    .fontWeight(.semibold)
                    .lineLimit(1)
                    .truncationMode(.tail)

                Text("\(receiver.devices.count) of \(receiver.maxDevices) devices paired")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary)
    }
}

// MARK: - Receiver detail

struct ReceiverDetailView: View {
    let receiver: ReceiverModel
    @Binding var selection: SidebarItem?
    @Environment(ReceiverStore.self) private var store
    @State private var showingPairingSheet = false

    private var canPair: Bool { receiver.devices.count < Int(receiver.maxDevices) }

    var body: some View {
        VStack(spacing: 0) {
            ReceiverHeader(receiver: receiver)
            Divider()
            List {
                Section("Receiver") {
                    LabeledContent("Kind", value: receiver.kind.label)
                    LabeledContent("Product ID", value: String(format: "0x%04X", receiver.productId))
                    LabeledContent("Serial") {
                        Text(receiver.serial.isEmpty ? "Unknown" : receiver.serial)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    LabeledContent("Max devices", value: "\(receiver.maxDevices)")
                }
                Section("Paired devices") {
                    ForEach(receiver.devices) { device in
                        Button {
                            selection = .device(device.id)
                        } label: {
                            HStack {
                                Label(device.name, systemImage: device.kind.systemImage)
                                Spacer()
                                Text(device.isOnline ? "Online" : "Offline")
                                    .font(.caption)
                                    .foregroundStyle(device.isOnline ? .green : .secondary)
                                Image(systemName: "chevron.right")
                                    .font(.caption)
                                    .foregroundStyle(.tertiary)
                            }
                            .opacity(device.isOnline ? 1.0 : 0.5)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                    if receiver.devices.isEmpty {
                        Text("No devices paired")
                            .foregroundStyle(.secondary)
                    }
                }
            }
            Divider()
            VStack(spacing: 6) {
                Button {
                    showingPairingSheet = true
                } label: {
                    Label("Pair new device", systemImage: "plus.circle")
                        .frame(maxWidth: .infinity, alignment: .center)
                }
                .disabled(!canPair)
                if !canPair {
                    Text("All \(receiver.maxDevices) slots are in use. Unpair a device first.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
            }
            .padding()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .sheet(isPresented: $showingPairingSheet, onDismiss: {
            if store.pairingStage == .paired {
                store.resetPairing()
            } else {
                store.cancelPairing()
            }
        }) {
            PairingSheetView(receiver: receiver)
        }
    }
}

// MARK: - Bluetooth header

private struct BluetoothHeader: View {
    let deviceCount: Int

    private var hostName: String {
        Host.current().localizedName ?? "Bluetooth"
    }

    var body: some View {
        HStack(spacing: 20) {
            Image("bluetooth")
                .renderingMode(.template)
                .resizable()
                .scaledToFit()
                .frame(width: 44, height: 44)
                .foregroundStyle(.primary)
                .frame(width: 56, height: 56)

            VStack(alignment: .leading, spacing: 6) {
                Text(hostName)
                    .font(.title2)
                    .fontWeight(.semibold)
                    .lineLimit(1)
                    .truncationMode(.tail)

                Text("\(deviceCount) device\(deviceCount == 1 ? "" : "s") paired via Bluetooth")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary)
    }
}

// MARK: - Bluetooth detail

struct BluetoothDetailView: View {
    let devices: [DirectDeviceModel]
    @Binding var selection: SidebarItem?

    private var hostName: String {
        Host.current().localizedName ?? "Bluetooth"
    }

    var body: some View {
        VStack(spacing: 0) {
            BluetoothHeader(deviceCount: devices.count)
            Divider()
            List {
                Section("Paired devices") {
                    ForEach(devices) { device in
                        Button {
                            selection = .directDevice(device.id)
                        } label: {
                            HStack {
                                Label(device.name, systemImage: device.kind.systemImage)
                                Spacer()
                                Text(device.isOnline ? "Online" : "Offline")
                                    .font(.caption)
                                    .foregroundStyle(device.isOnline ? .green : .secondary)
                                Image(systemName: "chevron.right")
                                    .font(.caption)
                                    .foregroundStyle(.tertiary)
                            }
                            .opacity(device.isOnline ? 1.0 : 0.5)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                    if devices.isEmpty {
                        Text("No devices paired")
                            .foregroundStyle(.secondary)
                    }
                }
            }
            Divider()
            Button {
                NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.BluetoothSettings")!)
            } label: {
                Label("Open Bluetooth Settings", systemImage: "arrow.up.right.square")
                    .frame(maxWidth: .infinity, alignment: .center)
            }
            .padding()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: - Direct device sidebar row

struct DirectDeviceSidebarRow: View {
    let device: DirectDeviceModel
    let isLast: Bool

    var body: some View {
        HStack(spacing: 0) {
            TreeConnector(isLast: isLast)
                .frame(width: 28)
            Label {
                Text(device.name)
            } icon: {
                Image(systemName: device.kind.systemImage)
                    .foregroundStyle(.primary)
            }

            Spacer()

            if let battery = device.battery {
                HStack(spacing: 3) {
                    if battery.isCached {
                        Image(systemName: "clock")
                            .font(.caption2)
                    }
                    Text(battery.levelText)
                        .font(.caption2)
                    Image(systemName: battery.batterySystemImage)
                        .font(.caption2)
                }
                .foregroundStyle(sidebarBatteryColor(battery))
            }
        }
    }
}

// MARK: - Direct device detail

struct DirectDeviceDetailView: View {
    let device: DirectDeviceModel
    @State private var showingInfo = false
    @State private var containerHeight: CGFloat = 480

    var body: some View {
        VStack(spacing: 0) {
            DeviceHeader(
                name: device.name,
                kindImage: device.kind.systemImage,
                isOnline: device.isOnline,
                battery: device.battery
            )
            Divider()
            Color.clear
                .overlay {
                    VStack(spacing: 10) {
                        Image(systemName: "slider.horizontal.3")
                            .font(.system(size: 32))
                            .foregroundStyle(.quaternary)
                        Text("Settings coming soon")
                            .foregroundStyle(.quaternary)
                    }
                }
            Divider()
            Button {
                NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.BluetoothSettings")!)
            } label: {
                Label("Unpair in Bluetooth Settings", systemImage: "arrow.up.right.square")
                    .frame(maxWidth: .infinity, alignment: .center)
            }
            .padding()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background {
            GeometryReader { geo in
                Color.clear
                    .onAppear { containerHeight = geo.size.height }
                    .onChange(of: geo.size) { _, new in containerHeight = new.height }
            }
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { showingInfo = true } label: {
                    Label("Device info", systemImage: "info.circle")
                }
            }
        }
        .sheet(isPresented: $showingInfo) {
            NavigationStack {
                DirectDeviceInfoSheet(device: device)
                    .navigationTitle(device.name)
            }
            .frame(width: 360, height: containerHeight * 0.8)
        }
    }
}

private struct DirectDeviceInfoSheet: View {
    let device: DirectDeviceModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        List {
            if let battery = device.battery {
                Section(battery.isCached ? "Battery (last seen)" : "Battery") {
                    LabeledContent("Level", value: battery.levelText)
                    if let status = battery.status {
                        LabeledContent("Status", value: status.label)
                    }
                    if let voltage = battery.voltage {
                        LabeledContent("Voltage", value: "\(voltage) mV")
                    }
                }
            }
            Section("Device") {
                LabeledContent("Type", value: device.kind.label)
                LabeledContent("Connection", value: device.connectionLabel)
                LabeledContent("Product ID", value: String(format: "0x%04X", device.productId))
                if !device.serial.isEmpty {
                    LabeledContent("Serial") {
                        Text(device.serial)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
            }
        }
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
    }
}

#Preview {
    ContentView()
        .environment(ReceiverStore())
}
