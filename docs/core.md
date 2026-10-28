# Rust core (crates/core/)

Library implementing HID++ 1.0 and 2.0 over hidapi, used directly by the egui UI crate.

## Solaar as protocol reference

`reference/` is Solaar's source; it has reverse-engineered this protocol over many years. When stuck on HID++ behavior (how to encode a write, what bytes mean, what function numbers to use), read it before probing blindly. Key files under `reference/lib/logitech_receiver/`:

- `hidpp10.py`: HID++ 1.0
- `hidpp20.py`: HID++ 2.0
- `settings_templates.py`: per-feature read/write classes; check this first for any feature
- `hidpp20_constants.py`: feature IDs and constants
- `special_keys.py`: button/key mappings
- `descriptors.py`: per-device quirks

## FFI (removed 2026-10-28)

`ffi.rs` and the C ABI went with the egui rewrite; the last version is in git history on `master`. Two things it did that the UI's HID worker must now do itself:

- Batch settings read: `Receiver`'s all-settings call is the only settings read path, one `discover_features` for all 7 features. Before the batch call existed, prefetching ran discovery 7 times per device.
- Streaming enumeration: loop `enumerate_slot` from slot 1, stop when found >= the paired count from `get_paired_device_count` or slot > max_devices, and publish each device as it arrives so the sidebar fills one row at a time.

## Implemented protocol surface

- HID++ 1.0: receiver/device/pairing info; Bolt serials + pairing (sub-reg 0x50/0x60); Bolt passkey (ASCII decimal + 10-bit L/R)
- HID++ 2.0: feature discovery, device name, battery (0x1000/0x1001/0x1004)
- Unpairing: Unifying (0xB2), Bolt (0x2C1); full pairing event loop (Unifying 0x4A/0x41; Bolt 0x4F/0x53/0x54/0x4D)
- Direct BT enumeration (`src/direct/`): vendor=0x046D, usage_page=0xFF00, usage!=0x0001; skips receiver PIDs; probes HID++ 2.0 ROOT at 0xFF; battery via 0x1004/0x1000/0x1001
- Event listeners: per-receiver, 250ms poll; `poll_device_event` decodes 0x41 (link), 0x4B (power), and unsolicited 2.0 notifications
- Settings Phase 1: ADJUSTABLE_DPI (0x2201) get/set with DPI list pagination and range-step expansion (`CDpiSettings.dpi_list = [u16; 200]`); HIRES_WHEEL (0x2121) get/set scroll
- Settings Phase 2: SmartShift Enhanced (0x2111), Change Host (0x1814/0x1815), FN inversion (0x40A0/0x40A2/0x40A3), Multiplatform (0x4531), Backlight2 (0x1982); all with set FFI exports and C structs
- HOSTS_INFO (0x1815): `read_host_name` loops in 14-byte chunks via fn 3 with increasing offset, handles names up to maxNameLen (24 on tested devices), cap_flags bit 0 = can read. `set_host_name` writes the computer's hostname to the active host slot in 14-byte chunks via fn 4; cap_flags bit 1 = can write; maxNameLen from fn 1 byte 5; called from the frontends' prefetch.
- DEVICE_NAME (0x0005): full product name, e.g. "MX Anywhere 3S for Business"; read during enumeration when the device is online. Preferred over DEVICE_FRIENDLY_NAME (0x0007), which is a short codename like "MX Anywhere3SB". `get_friendly_name` reads the fn 1 reply from params[1..] (one skipped prefix byte vs DEVICE_NAME's params[0..]).

## Event decoding

`poll_device_event` returns `LinkChange` for 0x41/0x4B and `SettingsChanged{slot, feature_index}` for any other unsolicited HID++ 2.0 feature notification (address & 0x0F == 0; Pulsaar uses software_id=0x0A, so its own replies have & 0x0F == 0x0A). `CDeviceConnectionEvent.feature_index` lets a frontend compare against the 0x1B04 feature index and discard button-CID events; the macOS refresh path built on this is in [macos.md](macos.md).

## Feature quirks

- K375S_FN_INVERSION (0x40A3): write params are [host_byte, swap_byte] via fn 1; host_byte = p[0] of the fn 0 read. Omitting the prefix causes a silent ignore. Read the active state from p[1], not p[0]. Confirmed with probe_settings.
- BACKLIGHT2 (0x1982): mode lives in bits 0-1 of the options byte, not bits 3-4 (confirmed with probe_settings); supported bitmask bit 0 = auto, bit 2 = manual. Writes are read-modify-write: set bits 0-1, preserve bit 2 and up, and preserve the dho/dhi/dpow timing fields at bytes 6-11.
- REPROG_CONTROLS_V4 (0x1B04): diversion persists in device RAM across app restarts. Options+ configures per-button diversions (SmartShift toggle etc.); a listener that reads and discards those CID notifications leaves the buttons inert. `get_all_settings` therefore calls `clear_reprog_controls_diversions`: iterate all controls via fn 0/1, read current flags via fn 2 (bit 0x01 = DIVERTED), call fn 3 with flags_byte=0x02 (bit 0 clear, bit 1 = valid) to restore firmware-default behavior. Only temporary diversions (bit 0x01) are cleared; persistent remaps (bit 0x04) are left alone. Persistent diversions set by Options+ survive power cycles and keep firing notifications on every click even after the temporary ones are cleared.
- Receiver notification flags (hidpp10): KEYBOARD_MULTIMEDIA_RAW (0x010000) routes consumer-control keys (Mute, Volume, etc.) through the HID++ vendor interface; Pulsaar does not forward them, so they would be lost. SOFTWARE_PRESENT (0x000800) routes 0x1B04-diverted keys through HID++; same problem. Options+ sets both when active. `open_for_events` sets ONLY NOTIF_WIRELESS (0x000100), clearing everything else; `start_pairing` sets WIRELESS | SOFTWARE_PRESENT (needed for pairing notifications), not flags | needed.
- Bolt register 0x02: Bolt receivers do not respond to RECEIVER_CONNECTION (0x02), so `get_paired_device_count` always returns Err(Timeout) for Bolt and the fallback returns max_devices (6). Empty Bolt slots never return a HID++ error either; the receiver times out silently. `get_bolt_pairing_info_fast` uses a 50ms probe: valid slots respond in ~8ms, empty slots time out at 50ms instead of the default 300ms. `transport.request_timeout` applies the override unconditionally when Some, not only for wireless device requests.

## Transport and platform HID handling

- Receiver enumeration is filtered to usage_page=0xFF00 / usage=0x0001: one interface per receiver, and it avoids macOS input-monitoring permission errors.
- macOS: `hid_darwin_set_open_exclusive(0)` must run before any HID open, or macOS claims exclusive access; it is behind `#[cfg(target_os = "macos")]`. IOKit opens at the interface level and one handle receives reports from all collections, which is why usage=0x0001 alone works there. read_path == write_path and read_dev is None (falls back to write_dev).
- Windows: Logitech USB receivers expose MI_02 (the vendor interface) as multiple top-level HID collections, each a separate device path needing its own handle. Col01 (usage=0x0001) is command-out only: writes succeed but reads time out because the collection declares no input reports. Col02 (usage=0x0002) receives the HID++ replies. `Transport` opens both: `write_dev` (Col01) and `read_dev` (Col02; None falls back to write_dev). `ReceiverHandle` stores `write_path` and `read_path`; `enumerate_receivers` matches the pair by extracting the hardware-instance prefix (`7&XXXXXXXX`) from the device path (stripping the `&0&XXXX` collection-index suffix from the instance segment) and checking contains() among usage=0x0002 candidates with the same PID. The Bolt receiver has Col01+Col02; the Unifying receiver also has Col03 (usage=0x0004, purpose unknown). `enumerate_devices` skips failed slots; it used `?` before, which aborted the whole scan on one bad slot.
- Linux: one hidraw node per receiver carries both the 0x0001 and 0x0002 usages, so read_path == write_path as on macOS. The kernel's extra hidraw nodes per receiver have other usage pages and are filtered out. Non-root access needs `packaging/42-pulsaar.rules`. Verified 2026-10-28 on Fedora with two Bolt receivers, with both the C hidraw backend and the pure-Rust `linux-native-basic-udev` backend now in use.

## Debug bins

Run from the workspace root with a receiver attached:

- `cargo run --bin list_devices` lists receivers and devices
- `cargo run --bin listen [SECONDS]` asks each receiver for link notifications, then prints every HID++ and DJ notification; wake, sleep or switch a device meanwhile
- `cargo run --bin probe_hosts` prints, read-only, the Change Host, Hosts Info and Device Reset feature versions and flags and the Hosts Info replies of online devices. 2026-10-28 on the MX Anywhere 3S and MX Keys Mini: Device Reset (0x1802) has flags 0x70, hidden and engineering-only, so a software factory reset is not available; Hosts Info capability flags are 0x13, where only bit 0 (host names readable) is documented by Solaar, so per-host delete is unconfirmed. Calling Hosts Info functions 5 and 6 with host 1 on the mouse both returned HID++ 2.0 error 0x05 (Logitech internal), not 0x07 (invalid function): the functions exist but refuse, and the host list was unchanged. Without Logitech's spec this is a dead end
- `cargo run --bin probe_settings` probes and writes HID++ features with write tests and readback; this confirmed the K375S host-prefixed write and the Backlight2 bits 0-1 encoding

## Debug logging

Rust logs `[PULSAAR][TAG]` lines: HIDPP20 (every feature_call with reply bytes, the feature discovery map, set_* entry). The Swift-side tags and the on/off switch are in [macos.md](macos.md).
