# Status and to-do

Keep this file current: after every change, remove finished items and adjust the order of next work. What is already built is documented in [core.md](core.md), [macos.md](macos.md), and [windows.md](windows.md).

## Not done

egui rewrite (branch `egui-rewrite`):

- Done 2026-10-28: workspace skeleton; HID worker (`crates/ui/src/store.rs`); the macOS UI ported (sidebar tree, device, receiver and Bluetooth panes, settings, pairing and unpair dialogs, toasts, Ctrl+R). Not yet tested by the user with an online device, pairing or unpairing
- DPI slider: tick labels and the log^1.5 curve of the macOS slider (egui's plain log scale for now)
- Receiver and device-kind icons (the macOS app used SF Symbols and three SVGs)
- Pick a license; cargo-deb and cargo-generate-rpm need the `license` field before the first tag
- Core tests cover message parsing and the button passkey; the per-feature parsers need a fake transport before they can be tested
- Delete `macos/` and `windows/` at parity

Rust core:

- Online status is inferred from battery read success; 0x41 is not stored on DeviceInfo
- Phase 2f: Disable Keys (0x4540, probable feature ID); probe the encoding first
- Phase 3: SKMS button remapping (0x1B04); design session required
- Firmware updates (0x00C2): deferred, bricking risk, needs safety gate design

macOS:

- Online status: 0x41 triggers a full reload, not an in-place update
- Phase 2f: Disable Keys UI
- Phase 3: button remapping UI
- Direct BT device settings (separate HID path needed)
- Window minimum size enforcement
- Notarization (requires a paid Apple Developer account)

Windows (deprioritized): stages 5, 6, and 8, detailed in [windows.md](windows.md).

CI/CD: the GitHub Actions workflows are stubs (Rust core build only, packaging TODOs).

Linux: dropped. The GTK4 app was never started; `build-linux.yml` and the README mentions are leftovers.

## Suggested order of next work

Reordered 2026-07 for the macOS-first direction (the Windows stages led before that):

1. Phase 2f: Disable Keys; probe 0x4540 first to confirm the encoding
2. Phase 3: SKMS button remapping (design session first)
3. GitHub Actions CI/CD: build + release on version tags, macOS first
4. Windows stages 5, 6, 8 when Windows work resumes
5. Firmware updates (deferred)

## Remaining device settings phases

Phase 2f, Disable Keys (0x4540): probe first, the encoding is unconfirmed. UI is a Keyboard Advanced Options section: counter plus a list of disableable keys.

Phase 3, SKMS (0x1B04): reads the button capability table and writes per-button action bindings. Structurally different from every Phase 2 feature; hold a design session before starting.
