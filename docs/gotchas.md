# Cross-cutting decisions and gotchas

Protocol and feature quirks live in [core.md](core.md); platform-specific ones in [macos.md](macos.md) and [windows.md](windows.md).

## One egui UI, not native per platform

Reversed 2026-10-28. The original plan was one SwiftUI app, one WinUI 3 app and a shared core, chosen so each platform would feel right. It doubled every UI change and left Linux out. OxTree proved the egui route on all three platforms, so Pulsaar follows it. The cost is a UI that looks the same everywhere rather than native.

## One handle per receiver, held by the worker

The SwiftUI app opened a receiver for each read and stopped its event listeners around every read, because macOS allows one open handle per receiver. The worker opens each receiver once with `open_for_events` and uses that handle for everything, so the prefetch races, `isPrefetching` waits and listener restarts are gone. Notifications that arrive during a request are skipped by `Transport::request`, as they were before. Pairing sets more notification flags; `listen_for_events` puts them back when pairing ends.

## Online means linked, as the receiver reports it

Writing 0x02 to receiver register 0x02 makes the receiver send a 0x41 notification per paired device with its link state (Solaar's notify_devices); `Receiver::link_states` does this at every reload. Before 2026-10-28 a device counted as online when it answered a 500 ms HID++ 2.0 probe, but a linked, dozing Bolt mouse takes 0.8 to 1 s and a keyboard up to 2 s, so both showed offline, and since their link never dropped no notification ever corrected it. Linked devices are now probed with the full 4 s timeout, unlinked ones not at all. After the reload, 0x41 notifications update one device at a time: offline at once, online after the 750 ms debounce, followed by a read of name, battery and settings.

On first read the worker rewrites the scroll mode (clears the HID++ target bit) and writes the host name to the active host slot, the host name once per device per run.

## Feature tables are cached per receiver handle

Feature discovery costs 0.6 to 1.4 s per device (one request per feature). `Receiver` keeps each slot's table after the first successful discovery and every setter and settings read uses it, so a write or a notification-triggered refresh skips discovery. A slot's table only changes when another device is paired there; pairing and unpairing reload, which opens new `Receiver`s with empty caches.

## DJ reports

With the kernel's hid-logitech-dj driver bound (not the case on the author's Fedora laptop, where the receivers use hid-generic), receivers run in DJ mode and send 15-byte reports with ID 0x20. Type 0x42 is the connection status (bit 0 of the first byte set means link lost); types below 0x10 are forwarded input. The core parses them so link changes arrive either way, and `Transport::request` and pairing skip them.

## The release profile unwinds

OxTree aborts on panic; Pulsaar unwinds. The reply parsers index into `params()` without length checks, and the FFI used to hide that behind `catch_unwind`. Until the parsers use `get()`, a malformed reply must only kill the HID worker, not the window: `Store::start` catches the panic, shows a toast and starts the worker again, up to three times.

## Not a Solaar fork

No shared code and no git relationship with Solaar; the protocol was reimplemented in Rust using Solaar's source as reference. New device support and protocol fixes are ported manually from Solaar changelogs a few times a year, not merged.

## Remapping is Linux-kernel-only

Solaar's key remapping/diversion delivery relies on evdev/uinput, which only exists on Linux. Pulsaar documents this limitation clearly wherever it is relevant; never silently drop the feature from comparisons or docs.

## Device settings design

Lazy-load: settings are read only when a panel needs them, and only via the batch `pulsaar_get_all_settings` call. No app-side persistence; the device stores settings in onboard NVM. Writes are live, no Apply button.

## settingsCache write-through

Every `set*`/`Set*()` method patches the settings cache on the main thread immediately after a successful write. Without it, navigating away and back re-applies stale prefetch values and reverts the user's change: the panel's load path serves from cache on re-entry, so the cache must always reflect the last written state.

## Frontends mirror each other

`NativeInterop.cs`, `Models.cs`, `ReceiverStore.cs`, and `DeviceCache.cs` mirror the corresponding Swift files; the DeviceCache JSON schema is identical across platforms. macOS is the reference implementation. A behavior fix on one platform usually belongs on both; the known deliberate divergence is the refresh path (macOS must stop event listeners around HID reads, Windows must not; see the platform docs).
