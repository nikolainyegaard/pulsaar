# Architecture

Native desktop app for managing Logitech Unifying and Bolt receivers: pairing and unpairing, battery, device info, device settings. Spiritual successor to Solaar (Linux-only, GTK). Logitech Options+ requires an account and phones home; Pulsaar fills the gap without either. The name's double A nods to Solaar.

Not a Solaar fork: a clean reimplementation using Solaar's source as protocol reference (snapshot in `reference/`, see [core.md](core.md)). No git relationship; new device support and protocol fixes are ported manually from Solaar changelogs a few times a year. Scope: a feature-complete Solaar port where the OS allows it. Remapping/diversion via evdev/uinput is Linux-kernel-only; document that limitation clearly, never silently drop it.

One Rust binary with an egui window on Linux, macOS and Windows, laid out like OxTree (`~/Documents/GitHub/OxTree`), which is the reference for structure, lints, fonts, packaging and CI. Decided 2026-10-28, replacing the SwiftUI plus WinUI 3 plan: one UI instead of two mirrored ones, and Linux back in scope. The user is self-taught (Python/Docker background), building this with Claude.

## Platform direction

Linux, macOS and Windows from one codebase. The rewrite lives on the `egui-rewrite` branch. `macos/` and `windows/` stay in the tree as the behaviour spec for the egui UI and are deleted at parity; they no longer build there, since `ffi.rs` is gone.

## Tech stack

| Layer | Choice |
|---|---|
| Core | Rust + hidapi (pure-Rust `linux-native-basic-udev` backend on Linux, the C backends elsewhere) |
| UI | egui 0.37 through eframe with the glow renderer |
| CI | `ci.yml`: fmt, clippy and tests on Linux, clippy on macOS and Windows |
| Release | `release.yml` on `v*` tags: Linux x86_64 and aarch64 against glibc 2.28 (tarball, .deb, .rpm, AppImage), universal macOS app zip, Windows zip plus a software OpenGL zip |

## File structure

```
pulsaar/
├── Cargo.toml                   workspace, shared lints, release profile
├── clippy.toml                  unwrap, expect, panic and indexing allowed in tests only
├── crates/
│   ├── core/                    HID++ protocol, no UI (pulsaar-core)
│   │   └── src/
│   │       ├── lib.rs           init(): macOS exclusive-open flag, then hidapi
│   │       ├── transport.rs     HID open/read/write, request timeouts, split handles on Windows
│   │       ├── error.rs
│   │       ├── hidpp/           hidpp10.rs, hidpp20.rs, message.rs
│   │       ├── devices/         device descriptors (mod.rs, types.rs)
│   │       ├── receiver/        Unifying/Bolt receiver logic, AllDeviceSettings
│   │       ├── direct/          direct Bluetooth device enumeration
│   │       └── bin/             list_devices.rs, probe_settings.rs (debug tools)
│   ├── native/                  platform glue without UI: config directory, host name, Bluetooth settings (pulsaar-native)
│   └── ui/                      egui views and application state (pulsaar-ui)
│       └── src/                 lib.rs (app, menu, sidebar, toast), detail.rs (panes, settings, DPI slider), dialogs.rs, icons.rs, store.rs (HID worker); assets/ holds the Bolt, Unifying and Bluetooth marks as 64 px RGBA
├── apps/pulsaar/                the binary: window, fonts, icon; deb and rpm metadata
├── packaging/                   desktop entry, icons, AppStream metainfo, udev rule, Info.plist
├── macos/, windows/             previous native apps, kept as the spec until parity
├── reference/                   Solaar source snapshot (protocol reference, never built)
├── .github/workflows/           ci.yml, release.yml
└── docs/
```

## Layers

Dependencies point downwards only: `apps/pulsaar` uses `ui`, `native` and `core`; `ui` uses `native` and `core`; nothing below the binary opens a window. All HID++ traffic runs on one worker thread (`crates/ui/src/store.rs`). It opens each receiver once per reload and keeps that handle for requests, notifications and pairing, polling notifications every 100 ms and the OS device list every 2 s for plug and unplug. The UI sends commands and reads a shared model; the worker locks the model only to copy results in, never across a HID call.

## Build and release infrastructure

`cargo build --release` on any of the three platforms. The release workflow is OxTree's with the names changed, the polkit action replaced by the udev rule, and the Windows administrator manifest dropped. Packages need a `license` field before the first tag, see [status.md](status.md).
