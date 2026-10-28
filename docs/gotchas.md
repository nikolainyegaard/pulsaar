# Cross-cutting decisions and gotchas

Protocol and feature quirks live in [core.md](core.md); platform-specific ones in [macos.md](macos.md) and [windows.md](windows.md).

## One egui UI, not native per platform

Reversed 2026-10-28. The original plan was one SwiftUI app, one WinUI 3 app and a shared core, chosen so each platform would feel right. It doubled every UI change and left Linux out. OxTree proved the egui route on all three platforms, so Pulsaar follows it. The cost is a UI that looks the same everywhere rather than native.

## One handle per receiver, held by the worker

The SwiftUI app opened a receiver for each read and stopped its event listeners around every read, because macOS allows one open handle per receiver. The worker opens each receiver once with `open_for_events` and uses that handle for everything, so the prefetch races, `isPrefetching` waits and listener restarts are gone. Notifications that arrive during a request are skipped by `Transport::request`, as they were before. Pairing sets more notification flags; `listen_for_events` puts them back when pairing ends.

## Settings are read only for online devices

`DeviceInfo::online` is true when the device answered HID++ 2.0 during enumeration. Reading settings of an offline device waits the full 4 s device timeout, so the worker skips them; a link-change notification triggers a reload, which reads the device once it is online. On first read the worker rewrites the scroll mode (clears the HID++ target bit) and writes the host name to the active host slot, as the macOS prefetch did.

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
