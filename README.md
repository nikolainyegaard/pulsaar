# Pulsaar

A native desktop app for managing Logitech Unifying and Bolt wireless receivers. Pair and unpair devices, check battery status, and view device info -- no account required, no telemetry.

Pulsaar runs natively on macOS (SwiftUI), Windows (WinUI 3), and Linux (GTK4), with a shared Rust core handling the HID++ protocol.

## Status

Early development. Not yet functional.

## Architecture

A shared Rust library (`core/`) implements the HID++ protocol and exposes a C-compatible FFI. Each platform has its own native UI that calls into this core:

- `core/` - Rust library (HID++ 1.0 and 2.0, device descriptors, receiver logic)
- `macos/` - SwiftUI app
- `windows/` - WinUI 3 app (C#)
- `linux/` - GTK4 app

## Building

See the platform-specific instructions in each subdirectory. The Rust core must be built first.

```
cd core && cargo build
```

## Relationship to Solaar

Pulsaar is not a fork of Solaar. It reimplements the HID++ protocol from scratch in Rust, using Solaar's source as a protocol reference. The `reference/` directory contains a snapshot of Solaar used during development.
