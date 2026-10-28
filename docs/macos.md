# macOS app (macos/)

SwiftUI app, deployment target macOS 14.0. Not sandboxed (no App Store); the IOKit entitlement is required. Distribution plan: direct download + Homebrew cask. Notarization needs a paid Apple Developer account and is not done.

## Build

Xcode project with sandbox off, the bridging header wired, and `libpulsaar_core.a` linked statically. A cargo script phase builds the Rust core automatically: Release builds a universal binary via lipo (aarch64 + x86_64), Debug builds native only. `ASSETCATALOG_COMPILER_APPICON_NAME` is set in both Debug and Release configs in project.pbxproj.

## HID exclusive access

Even with `hid_darwin_set_open_exclusive(0)` (see [core.md](core.md)), only one handle per receiver can be open at a time, which shapes several flows:

- Settings refresh path: 0.75s debounce (`scheduleSettingsRefresh`) -> `stopEventListeners` (frees the HID handle; a second handle cannot open while the event listener holds one) -> background HID read -> update cache + increment `settingsCacheVersion` -> `restartEventListeners`. `DeviceSettingsPanel` observes `settingsCacheVersion` via `.onChange` and re-applies live.
- `prefetchSettings` holds the receiver open on a background thread (~4s); if the settings panel task fires in that window it races to open the same receiver and gets an exclusive-access error. The `isPrefetching` flag lets the task wait up to 6s for prefetch to populate the cache before falling back to `loadSettings`. The 3s USB connect debounce can also fire during prefetch; the debounced work item checks `isPrefetching` and skips the reload.
- The Unpair button is disabled while `isPrefetching` for the same reason.

## USB monitoring

IOKit auto-reloads on Logitech receiver plug/unplug. `IOHIDManagerOpen` fires device-matching callbacks for already-connected devices at startup, not just new connections, which caused 4+ spurious reloads within 8 seconds of launch. Fixed with a 3s debounce (`scheduleUSBConnectReload`) on the matching callback only; the removal callback stays immediate.

## ReceiverStore and caching

- Event listeners: 250ms poll, 750ms settings-refresh debounce; stopped and restarted around the prefetch batch; paused during pairing.
- `prefetchSettings` runs once at launch (1s delay) and after pairing (0.75s). It writes HIRES_WHEEL SetMode to clear the HID++ target bit (fixes scroll events being swallowed while Pulsaar is open) and writes the Mac hostname (`Host.current().localizedName`) to the active host slot via `pulsaar_set_host_name` on every launch.
- Event filtering: persistent 0x1B04 diversions keep firing on every mouse click even after the temporary ones are cleared (see core.md). Swift compares `CDeviceConnectionEvent.feature_index` against `reprogControlsIdx` (from `CAllDeviceSettings` / `DeviceSettingsModel`) to discard button-CID events and only refresh on genuine settings changes.
- `set*` write functions patch `settingsCache` immediately on success (see [gotchas.md](gotchas.md)); they share a `writeSetting` helper (same logs, toasts, cache write-through).
- Settings loading race: `prefetchSettings` increments `settingsCacheVersion` on completion; the panel task checks the cache after a failed independent load.
- `DeviceCache` persists last-known battery per serial (`device-cache.json`) and full DEVICE_NAME per serial (`device-names.json`) under `~/Library/Application Support/Pulsaar/`. Cached values are injected for offline devices so the sidebar always shows battery and the full name; `DeviceModel.name` is `var` for post-init injection.
- Online status is inferred from battery read success; a 0x41 link event triggers a full reload, not an in-place update.

## UI

- `NavigationSplitView`: tree sidebar (TreeConnector dashed lines with T/L corners) plus device, receiver, and direct-BT detail panes. Sidebar sorted alphabetically by display name, receivers and the Bluetooth row interleaved. Direct BT devices shown above receivers.
- `DeviceHeader`: 56pt icon, name, battery bolt/level icons. `ReceiverHeader`: 44pt icon, X of Y devices. `navigationTitle` removed from all detail views; the name lives only in the header block.
- Battery: live + cached, clock icon for cached values; sidebar color-coded (green=charging, orange<=25%, red<=10%); `battery.100percent.bolt` for all charging states.
- Settings panel (`DeviceSettingsPanel`): `LogDpiSlider` (log^1.5 scale, tiered 50/100/250/500-step snap, Canvas tick labels); scroll toggles with explicit Binding setters; SmartShift mode picker plus ratchet threshold slider inlined into the Scroll Wheel section (no `step: 1`, which would draw tick marks); FN swap toggle; Set OS picker; Backlight mode picker (all 3 modes shown unconditionally) plus manual brightness slider; Change Host picker. `WheelMode.smartShift.label = "Ratchet"`. The ratchet threshold slider snaps to 5% intervals on release and during drag (`smartShiftTorqueDrag` state). Write functions share a `performWrite` helper. The panel clears settings/isLoading at task start so stale data from the previous device never shows after navigation.
- Info sheets: one `DeviceInfoSheet` for receiver and direct-BT devices (slot row only when slot != nil); NavigationStack with a `.confirmationAction` Done button; serial uses `.truncationMode(.middle)`; fixed 360x420 frame.
- Pairing sheet: per-stage UI (spinner, numeric or L/R passkey, success checkmark, failure with retry); auto-dismisses 2s after pairing.
- Toasts: `showToast()` in ReceiverStore; animated capsule overlay at the bottom of the detail pane; auto-dismiss after 2s; shown on every successful `set*` write.
- `BluetoothDetailView`: BT logo, hostname, device count, list of paired BT devices, "Open Bluetooth Settings" button. Direct BT devices get an "Unpair in Bluetooth Settings" button (no HID++ unpair path).
- Custom icons: Bluetooth (inverted badge, mask technique), Bolt (glyph only, cropped viewBox), Unifying (inverted badge, alternating winding direction).
- Menus: "Receivers > Force Refresh" (Cmd+R); "Window > Return to Default Size". Window minimum size enforcement is not done.
- 2026-07 slim-down: ReceiverKind/DeviceKind/BatteryStatus are UInt8 raw-value enums mirroring the Rust `#[repr(u8)]` values; sidebar rows, device list rows, info sheets, and batteryColor were deduplicated; the bridging header matches the trimmed FFI surface.

## Debug logging

`[PULSAAR][TAG]` to the console (Xcode captures stderr and stdout); set `pulsaarLoggingEnabled = false` in ReceiverStore.swift to silence. Swift tags: STORE (reload/init/unpair), SETTINGS (prefetch/load/apply), WRITE (all FFI writes plus return status), EVENTS (listener open/close/events), PAIRING, USB. Rust-side tags are in [core.md](core.md).
