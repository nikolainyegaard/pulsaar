# Pulsaar

A native desktop app for managing Logitech Unifying and Bolt wireless receivers. Pair and unpair devices, check battery status, and view device info -- no account required, no telemetry.

Pulsaar is one Rust binary with an egui window for Linux, macOS and Windows.

## Status

Early development. Not yet functional.

## Architecture

- `crates/core/` - HID++ 1.0 and 2.0, receivers, device settings
- `crates/native/` - platform glue without UI
- `crates/ui/` - egui views and application state
- `apps/pulsaar/` - the binary

`macos/` and `windows/` hold the previous native apps; they are the behaviour spec for the egui UI and go away once it reaches parity.

## Building

Rust 1.88 or newer and a working OpenGL driver.

```
cargo build --release
```

On Linux the user needs access to `/dev/hidraw*`. The .deb and .rpm install `packaging/42-pulsaar.rules` for that; otherwise copy it to `/etc/udev/rules.d/`.

## Relationship to Solaar

Pulsaar is not a fork of Solaar. It reimplements the HID++ protocol from scratch in Rust, using Solaar's source as a protocol reference. The `reference/` directory contains a snapshot of Solaar used during development.
