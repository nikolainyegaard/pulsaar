import SwiftUI
import AppKit

// Sidebar selection: a receiver row, a receiver-hosted device row, or a direct BT device row.
enum SidebarItem: Hashable {
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
                .navigationSplitViewColumnWidth(min: 200, ideal: 280)
        } detail: {
            detailPane
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
                // Direct (Bluetooth) devices at the top level, no tree lines.
                ForEach(store.directDevices) { device in
                    DirectDeviceSidebarRow(device: device)
                        .tag(SidebarItem.directDevice(device.id))
                }

                // Receivers with their paired devices indented beneath.
                ForEach(store.receivers) { receiver in
                    ReceiverSidebarRow(receiver: receiver)
                        .tag(SidebarItem.receiver(receiver.id))

                    ForEach(Array(receiver.devices.enumerated()), id: \.element.id) { index, device in
                        DeviceSidebarRow(device: device, isLast: index == receiver.devices.count - 1)
                            .tag(SidebarItem.device(device.id))
                            .listRowInsets(EdgeInsets(top: 3, leading: 16, bottom: 3, trailing: 8))
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
        case .device(let id):
            if let device = findDevice(id: id) {
                DeviceDetailView(device: device)
            } else {
                emptyState
            }
        case .receiver(let id):
            if let receiver = store.receivers.first(where: { $0.id == id }) {
                ReceiverDetailView(receiver: receiver)
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

// MARK: - Receiver sidebar row

struct ReceiverSidebarRow: View {
    let receiver: ReceiverModel

    var body: some View {
        Label {
            Text(receiver.name)
                .fontWeight(.medium)
        } icon: {
            Image(systemName: receiver.kind.systemImage)
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
            Button(role: .destructive) {
                showingUnpairConfirm = true
            } label: {
                Label("Unpair device", systemImage: "minus.circle")
                    .frame(maxWidth: .infinity, alignment: .center)
            }
            .padding()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .overlay {
            if showingInfo {
                ZStack {
                    Color.black.opacity(0.15)
                        .onTapGesture { showingInfo = false }
                    DeviceInfoSheet(device: device)
                        .frame(width: 360, height: 380)
                        .background(.background)
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                        .shadow(color: .black.opacity(0.25), radius: 20)
                }
                .transition(.opacity)
            }
        }
        .animation(.easeInOut(duration: 0.15), value: showingInfo)
        .navigationTitle(device.name)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { showingInfo = true } label: {
                    Label("Device info", systemImage: "info.circle")
                }
            }
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

private struct DeviceInfoSheet: View {
    let device: DeviceModel

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
                    LabeledContent("Serial", value: device.serial)
                }
            }
        }
    }
}

// MARK: - Receiver detail

struct ReceiverDetailView: View {
    let receiver: ReceiverModel
    @Environment(ReceiverStore.self) private var store
    @State private var showingPairingSheet = false

    var body: some View {
        List {
            Section("Receiver") {
                LabeledContent("Name", value: receiver.name)
                LabeledContent("Kind", value: receiver.kind.label)
                LabeledContent("Product ID", value: String(format: "0x%04X", receiver.productId))
                LabeledContent("Serial", value: receiver.serial.isEmpty ? "unknown" : receiver.serial)
                LabeledContent("Max devices", value: "\(receiver.maxDevices)")
            }

            Section("Paired devices") {
                ForEach(receiver.devices) { device in
                    HStack {
                        Label(device.name, systemImage: device.kind.systemImage)
                        Spacer()
                        Text(device.isOnline ? "Online" : "Offline")
                            .font(.caption)
                            .foregroundStyle(device.isOnline ? .green : .secondary)
                    }
                    .opacity(device.isOnline ? 1.0 : 0.5)
                }

                if receiver.devices.isEmpty {
                    Text("No devices paired")
                        .foregroundStyle(.secondary)
                }
            }

            let canPair = receiver.devices.count < Int(receiver.maxDevices)
            Section {
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
                }
            }
        }
        .navigationTitle(receiver.name)
        .sheet(isPresented: $showingPairingSheet, onDismiss: {
            // Sheet dismissed by any means: clean up pairing state.
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

// MARK: - Direct device sidebar row

struct DirectDeviceSidebarRow: View {
    let device: DirectDeviceModel

    var body: some View {
        HStack {
            Label {
                Text(device.name)
                    .fontWeight(.medium)
            } icon: {
                Image(systemName: device.kind.systemImage)
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
        .overlay {
            if showingInfo {
                ZStack {
                    Color.black.opacity(0.15)
                        .onTapGesture { showingInfo = false }
                    DirectDeviceInfoSheet(device: device)
                        .frame(width: 360, height: 380)
                        .background(.background)
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                        .shadow(color: .black.opacity(0.25), radius: 20)
                }
                .transition(.opacity)
            }
        }
        .animation(.easeInOut(duration: 0.15), value: showingInfo)
        .navigationTitle(device.name)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { showingInfo = true } label: {
                    Label("Device info", systemImage: "info.circle")
                }
            }
        }
    }
}

private struct DirectDeviceInfoSheet: View {
    let device: DirectDeviceModel

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
                    LabeledContent("Serial", value: device.serial)
                }
            }
        }
    }
}

#Preview {
    ContentView()
        .environment(ReceiverStore())
}
