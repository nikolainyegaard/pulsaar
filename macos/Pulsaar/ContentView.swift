import SwiftUI

// Sidebar selection: a receiver row or a device row.
enum SidebarItem: Hashable {
    case receiver(Int)
    case device(String)  // device.id ("receiverIndex-slot")
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
            } else if store.receivers.isEmpty {
                Label("No receivers found", systemImage: "antenna.radiowaves.left.and.right")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(store.receivers) { receiver in
                    // Receiver as a top-level selectable row.
                    ReceiverSidebarRow(receiver: receiver)
                        .tag(SidebarItem.receiver(receiver.id))

                    // Devices indented beneath their receiver.
                    ForEach(receiver.devices) { device in
                        DeviceSidebarRow(device: device)
                            .tag(SidebarItem.device(device.id))
                            .listRowInsets(EdgeInsets(top: 3, leading: 44, bottom: 3, trailing: 8))
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
            Image(systemName: "antenna.radiowaves.left.and.right")
        }
    }
}

// MARK: - Device sidebar row

struct DeviceSidebarRow: View {
    let device: DeviceModel

    var body: some View {
        HStack {
            Label {
                Text(device.name)
            } icon: {
                Image(systemName: device.kind.systemImage)
                    .foregroundStyle(device.isOnline ? .primary : .secondary)
            }

            Spacer()

            if let battery = device.battery {
                HStack(spacing: 3) {
                    Text(battery.levelText)
                        .font(.caption2)
                    Image(systemName: battery.batterySystemImage)
                        .font(.caption2)
                }
                .foregroundStyle(.secondary)
            }
        }
        .opacity(device.isOnline ? 1.0 : 0.45)
    }
}

// MARK: - Device detail

struct DeviceDetailView: View {
    let device: DeviceModel
    @Environment(ReceiverStore.self) private var store
    @State private var showingUnpairConfirm = false
    @State private var unpairFailed = false

    var body: some View {
        VStack(spacing: 0) {
            deviceHeader
            Divider()
            deviceProperties
        }
        .navigationTitle(device.name)
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

    private var deviceHeader: some View {
        HStack(spacing: 20) {
            Image(systemName: device.kind.systemImage)
                .font(.system(size: 44))
                .foregroundStyle(device.isOnline ? .primary : .secondary)
                .frame(width: 56, height: 56)

            VStack(alignment: .leading, spacing: 6) {
                Text(device.name)
                    .font(.title2)
                    .fontWeight(.semibold)

                HStack(spacing: 6) {
                    Circle()
                        .fill(device.isOnline ? Color.green : Color.secondary)
                        .frame(width: 8, height: 8)
                    Text(device.isOnline ? "Online" : "Offline")
                        .font(.subheadline)
                        .foregroundStyle(device.isOnline ? .primary : .secondary)
                }
            }

            Spacer()

            if let battery = device.battery, device.isOnline {
                VStack(alignment: .trailing, spacing: 4) {
                    Image(systemName: battery.batterySystemImage)
                        .font(.system(size: 28))
                        .foregroundStyle(batteryColor(battery))
                    Text(battery.levelText)
                        .font(.headline)
                        .foregroundStyle(batteryColor(battery))
                }
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary)
    }

    @ViewBuilder
    private var deviceProperties: some View {
        List {
            if let battery = device.battery, device.isOnline {
                Section("Battery") {
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
                LabeledContent("Slot", value: "\(device.slot)")
                if !device.serial.isEmpty {
                    LabeledContent("Serial", value: device.serial)
                }
            }

            Section {
                Button(role: .destructive) {
                    showingUnpairConfirm = true
                } label: {
                    Label("Unpair device", systemImage: "minus.circle")
                        .frame(maxWidth: .infinity, alignment: .center)
                }
            }
        }
    }

    private func batteryColor(_ battery: BatteryModel) -> Color {
        if battery.status?.isCharging == true { return .green }
        guard let level = battery.level else { return .secondary }
        if level <= 10 { return .red }
        if level <= 25 { return .orange }
        return .primary
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

#Preview {
    ContentView()
        .environment(ReceiverStore())
}
