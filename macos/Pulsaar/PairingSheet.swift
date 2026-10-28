import SwiftUI

// Sheet that guides the user through pairing a new device to a receiver.
// Presented from ReceiverDetailView. Calls pulsaar_start_pairing on appear,
// then polls pulsaar_poll_pairing via a timer in ReceiverStore.
struct PairingSheetView: View {
    let receiver: ReceiverModel
    @Environment(ReceiverStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 24) {
            // Header
            Image(systemName: "antenna.radiowaves.left.and.right")
                .font(.system(size: 44))
                .foregroundStyle(.tint)
                .padding(.top, 8)

            VStack(spacing: 4) {
                Text("Pair New Device")
                    .font(.title2)
                    .fontWeight(.semibold)
                Text(receiver.name)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }

            Divider()

            // Stage-specific content
            stageContent
                .frame(maxWidth: .infinity)
                .frame(minHeight: 160)

            Spacer(minLength: 24)

            bottomActions
        }
        .padding(28)
        .frame(width: 400)
        .onAppear {
            store.startPairing(receiverIndex: receiver.id)
        }
        .onChange(of: store.pairingStage) { _, newStage in
            // Auto-dismiss 2 s after the success animation begins (finalizePairing fires at 1.5 s).
            if newStage == .paired {
                DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) {
                    dismiss()
                }
            }
        }
    }

    // MARK: - Stage content

    @ViewBuilder
    private var stageContent: some View {
        switch store.pairingStage {
        case .idle, .waiting:
            waitingView

        case .deviceFound:
            deviceFoundView

        case .passkey:
            passkeyView

        case .paired:
            pairedView

        case .failed:
            failedView
        }
    }

    private var waitingView: some View {
        VStack(spacing: 18) {
            ProgressView()
                .scaleEffect(1.3)
            Text("Waiting for a device...")
                .foregroundStyle(.secondary)
            Text("Put your device into pairing mode.")
                .font(.callout)
                .foregroundStyle(.tertiary)
                .multilineTextAlignment(.center)
        }
    }

    private var deviceFoundView: some View {
        VStack(spacing: 18) {
            ProgressView()
                .scaleEffect(1.3)
            VStack(spacing: 6) {
                Text("Found: \(store.pairingDeviceName)")
                    .fontWeight(.medium)
                Text("Completing pairing...")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private var passkeyView: some View {
        if store.pairingPasskeyIsNumeric {
            VStack(spacing: 14) {
                Text("Enter this passkey on your keyboard:")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                Text(store.pairingPasskey)
                    .font(.system(size: 38, weight: .bold, design: .monospaced))
                Text("Then press Enter.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        } else {
            VStack(spacing: 14) {
                Text("Press these buttons in order:")
                    .foregroundStyle(.secondary)
                ButtonPasskeyView(passkey: store.pairingPasskey)
                Text("L = left button, R = right button")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                Text("Then press both buttons simultaneously.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
        }
    }

    private var pairedView: some View {
        VStack(spacing: 14) {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 52))
                .foregroundStyle(.green)
            let name = store.pairingDeviceName.isEmpty
                ? "Device (slot \(store.pairingNewSlot))"
                : store.pairingDeviceName
            Text("\(name) paired!")
                .fontWeight(.semibold)
                .multilineTextAlignment(.center)
        }
    }

    private var failedView: some View {
        VStack(spacing: 14) {
            Image(systemName: "xmark.circle.fill")
                .font(.system(size: 52))
                .foregroundStyle(.red)
            Text("Pairing failed")
                .fontWeight(.semibold)
            let msg = store.pairingError.isEmpty
                ? "The receiver did not respond."
                : store.pairingError
            Text(msg)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
    }

    // MARK: - Bottom actions

    @ViewBuilder
    private var bottomActions: some View {
        switch store.pairingStage {
        case .paired:
            Button("Done") {
                dismiss()
            }
            .buttonStyle(.borderedProminent)

        case .failed:
            HStack(spacing: 12) {
                Button("Cancel") {
                    dismiss()
                }
                Button("Try Again") {
                    store.startPairing(receiverIndex: receiver.id)
                }
                .buttonStyle(.borderedProminent)
            }

        default:
            Button("Cancel") {
                dismiss()
            }
        }
    }
}

// Renders a 10-character L/R button passkey as a row of styled badges.
private struct ButtonPasskeyView: View {
    let passkey: String

    var body: some View {
        HStack(spacing: 5) {
            ForEach(Array(passkey.enumerated()), id: \.offset) { _, ch in
                Text(String(ch))
                    .font(.system(size: 18, weight: .bold, design: .monospaced))
                    .frame(width: 28, height: 28)
                    .background(ch == "L" ? Color.blue.opacity(0.15) : Color.orange.opacity(0.15))
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                    .overlay(
                        RoundedRectangle(cornerRadius: 5)
                            .strokeBorder(ch == "L" ? Color.blue.opacity(0.4) : Color.orange.opacity(0.4), lineWidth: 1)
                    )
            }
        }
    }
}
