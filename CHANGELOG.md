# Changelog

All notable changes to Pulsaar will be documented here.

## [Unreleased]

First release: one Rust and egui binary for Linux, macOS and Windows,
a recreation in the spirit of Solaar.

### Added

- Receiver sidebar with Bolt, Unifying and Bluetooth icons, online state and
  battery per device, and a details pane per device, receiver and direct
  Bluetooth device.
- Settings: DPI (macOS-style slider), scroll direction and mode, SmartShift,
  host switching, fn-key swap, platform and backlight. Writes are optimistic
  and skipped when nothing changes.
- Pairing with live passkey progress for mice (L and R badges) and keyboards
  (digits plus Enter), unpairing with confirmation.
- Live online and offline state from receiver link notifications, battery
  from device broadcasts and reads, and a cache of the last battery and name
  per device for the next start.
- Hot-plug of receivers and a full re-read after resume from sleep.
- Linux packages (deb, rpm) with a udev rule, a macOS app bundle and a
  Windows zip with an optional software OpenGL fallback.

### Changed

- The project is licensed under GPL-2.0-or-later.

[Unreleased]: https://github.com/nikolainyegaard/pulsaar/commits/HEAD
