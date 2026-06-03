import SwiftUI

struct ContentView: View {
    @Environment(ReceiverStore.self) private var store
    @State private var selectedReceiverId: Int? = nil

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            detail
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    store.reload()
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
                .disabled(store.isLoading)
            }
        }
    }

    @ViewBuilder
    private var sidebar: some View {
        List(store.receivers, selection: $selectedReceiverId) { receiver in
            ReceiverRow(receiver: receiver)
                .tag(receiver.id)
        }
        .navigationTitle("Pulsaar")
        .overlay {
            if store.isLoading {
                ProgressView()
            } else if let error = store.errorMessage {
                ContentUnavailableView(error, systemImage: "exclamationmark.triangle")
            } else if store.receivers.isEmpty {
                ContentUnavailableView("No receivers found", systemImage: "antenna.radiowaves.left.and.right")
            }
        }
    }

    @ViewBuilder
    private var detail: some View {
        if let id = selectedReceiverId,
           let receiver = store.receivers.first(where: { $0.id == id }) {
            ReceiverDetailView(receiver: receiver)
        } else {
            ContentUnavailableView(
                "Select a receiver",
                systemImage: "antenna.radiowaves.left.and.right",
                description: Text("Choose a receiver from the list to see its paired devices.")
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Receiver sidebar row
// ---------------------------------------------------------------------------

struct ReceiverRow: View {
    let receiver: ReceiverModel

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(receiver.name)
                .font(.body)
            Text("\(receiver.kind.label) - \(receiver.devices.count) device\(receiver.devices.count == 1 ? "" : "s")")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 2)
    }
}

// ---------------------------------------------------------------------------
// Receiver detail
// ---------------------------------------------------------------------------

struct ReceiverDetailView: View {
    let receiver: ReceiverModel

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
                if receiver.devices.isEmpty {
                    Text("No devices paired")
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(receiver.devices) { device in
                        DeviceRow(device: device)
                    }
                }
            }
        }
        .navigationTitle(receiver.name)
    }
}

// ---------------------------------------------------------------------------
// Device row
// ---------------------------------------------------------------------------

struct DeviceRow: View {
    let device: DeviceModel

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(device.name, systemImage: device.kind.systemImage)
                .font(.body)

            HStack(spacing: 12) {
                Text("Slot \(device.slot)")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Text(device.kind.label)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                if !device.serial.isEmpty {
                    Text("S/N: \(device.serial)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            if let battery = device.battery {
                BatteryView(battery: battery)
            }
        }
        .padding(.vertical, 4)
    }
}

// ---------------------------------------------------------------------------
// Battery indicator
// ---------------------------------------------------------------------------

struct BatteryView: View {
    let battery: BatteryModel

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: battery.batterySystemImage)
            Text(battery.levelText)
            if let status = battery.status {
                Text("(\(status.label))")
            }
        }
        .font(.caption)
        .foregroundStyle(batteryColor)
    }

    private var batteryColor: Color {
        guard let level = battery.level else { return .secondary }
        if battery.status?.isCharging == true { return .green }
        if level <= 10 { return .red }
        if level <= 25 { return .orange }
        return .secondary
    }
}

#Preview {
    ContentView()
        .environment(ReceiverStore())
}
