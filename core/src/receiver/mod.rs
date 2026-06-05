// Receiver enumeration and device management.
// Reference: reference/lib/logitech_receiver/receiver.py, base_usb.py

use hidapi::HidApi;

use crate::devices::types::{Battery, DeviceInfo, DeviceKind};
use crate::error::Result;
use crate::hidpp::{hidpp10, hidpp20};
use crate::transport::Transport;

const LOGITECH_VID: u16 = 0x046D;

/// All known receiver product IDs. Exported so the direct-device enumerator can
/// skip receiver HID interfaces that might otherwise look like BT direct devices.
pub const RECEIVER_PIDS: &[u16] = &[
    0xC548, // Bolt
    0xC52B, 0xC532, // Unifying
    0xC52F, 0xC521, 0xC525, 0xC526, 0xC52E, 0xC531, 0xC534, 0xC535, 0xC537, // Nano
    0xC539, 0xC53A, 0xC53D, 0xC53F, 0xC541, 0xC545, 0xC547, 0xC54D, // LightSpeed
];

/// All known receiver product IDs with their kind and display name.
const RECEIVERS: &[(u16, ReceiverKind, &str)] = &[
    // Bolt
    (0xC548, ReceiverKind::Bolt,       "Bolt Receiver"),
    // Unifying
    (0xC52B, ReceiverKind::Unifying,   "Unifying Receiver"),
    (0xC532, ReceiverKind::Unifying,   "Unifying Receiver"),
    // Nano
    (0xC52F, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC521, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC525, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC526, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC52E, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC531, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC534, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC535, ReceiverKind::Nano,       "Nano Receiver"),
    (0xC537, ReceiverKind::Nano,       "Nano Receiver"),
    // LightSpeed
    (0xC539, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC53A, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC53D, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC53F, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC541, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC545, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC547, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
    (0xC54D, ReceiverKind::LightSpeed, "Lightspeed Receiver"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    Unifying,
    Bolt,
    Nano,
    LightSpeed,
}

/// Lightweight descriptor returned from enumerate_receivers before opening.
#[derive(Debug, Clone)]
pub struct ReceiverHandle {
    pub path: String,
    pub product_id: u16,
    pub kind: ReceiverKind,
    pub name: &'static str,
}

/// All configurable settings for one device, batched from a single feature discovery.
pub struct AllDeviceSettings {
    pub dpi:                  Option<hidpp20::DpiInfo>,
    pub scroll:               Option<hidpp20::ScrollInfo>,
    pub smart_shift:          Option<hidpp20::SmartShiftInfo>,
    pub hosts:                Option<Vec<hidpp20::HostInfo>>,
    pub fn_settings:          Option<hidpp20::FnInfo>,
    pub multiplatform:        Option<hidpp20::MultiplatformInfo>,
    pub backlight:            Option<hidpp20::BacklightInfo>,
    /// Feature index of REPROG_CONTROLS_V4 (0x1B04) for this device, or 0 if absent.
    /// Used to filter out button-CID HID++ notifications (which look like settings
    /// change events) in the event listener.
    pub reprog_controls_idx:  u8,
}

/// Event returned by poll_device_event.
pub enum DeviceEvent {
    /// A device came online or went offline.
    LinkChange { slot: u8, online: bool },
    /// The device reported an unsolicited HID++ 2.0 feature notification.
    /// feature_index is the sub_id of the message (= 0x1B04 feature index for
    /// button-CID events, or a settings feature index for mode changes).
    SettingsChanged { slot: u8, feature_index: u8 },
}

/// An opened receiver. Provides access to paired device info.
pub struct Receiver {
    transport: Transport,
    pub kind: ReceiverKind,
    pub name: &'static str,
    pub product_id: u16,
    pub serial: String,
    pub max_devices: u8,
}

/// Enumerate all Logitech receivers attached to the machine.
///
/// Filters to usage_page=0xFF00, usage=0x0001 (the HID++ vendor interface) to
/// get exactly one entry per physical receiver and avoid the HID interfaces
/// that require input-monitoring permissions on macOS.
pub fn enumerate_receivers(api: &HidApi) -> Vec<ReceiverHandle> {
    api.device_list()
        .filter(|d| d.vendor_id() == LOGITECH_VID)
        .filter(|d| d.usage_page() == 0xFF00 && d.usage() == 0x0001)
        .filter_map(|d| {
            let pid = d.product_id();
            RECEIVERS.iter().find(|&&(p, _, _)| p == pid).map(|&(_, kind, name)| {
                let path = d.path().to_string_lossy().into_owned();
                ReceiverHandle { path, product_id: pid, kind, name }
            })
        })
        .collect()
}

impl Receiver {
    /// Open a receiver by its handle descriptor.
    pub fn open(api: &HidApi, handle: &ReceiverHandle) -> Result<Self> {
        let transport = Transport::open(api, &handle.path)?;

        // Bolt uses a different register for serial and does not support the standard
        // RECEIVER_INFO sub-register 0x03 that Unifying/Nano/LightSpeed use.
        let (serial, max_devices) = if handle.kind == ReceiverKind::Bolt {
            let serial = hidpp10::get_bolt_serial(&transport)
                .unwrap_or_else(|_| String::from("unknown"));
            (serial, 6u8) // Bolt supports up to 6 paired devices
        } else {
            hidpp10::get_receiver_info(&transport).unwrap_or_else(|_| {
                (String::from("unknown"), 1)
            })
        };

        Ok(Self {
            transport,
            kind: handle.kind,
            name: handle.name,
            product_id: handle.product_id,
            serial,
            max_devices,
        })
    }

    /// Enumerate all devices paired to this receiver.
    ///
    /// For each occupied slot, reads pairing info (HID++ 1.0) to determine WPID
    /// and kind, then tries to read battery and name.
    pub fn enumerate_devices(&self) -> Result<Vec<DeviceInfo>> {
        let mut devices = Vec::new();

        for slot in 1..=self.max_devices {
            // Bolt uses different pairing sub-registers and embeds serial in the pairing response.
            let (wpid, kind, serial, name) = if self.kind == ReceiverKind::Bolt {
                let bolt = match hidpp10::get_bolt_pairing_info(&self.transport, slot)? {
                    Some(p) => p,
                    None => continue,
                };
                let name = hidpp10::get_bolt_device_codename(&self.transport, slot)?
                    .unwrap_or_else(|| format!("Device {}", slot));
                (bolt.wpid, bolt.kind, bytes_to_hex(&bolt.serial), name)
            } else {
                let pairing = match hidpp10::get_pairing_info(&self.transport, slot)? {
                    Some(p) => p,
                    None => continue,
                };
                let serial = hidpp10::get_extended_pairing_info(&self.transport, slot)?
                    .map(|s| bytes_to_hex(&s))
                    .unwrap_or_default();
                let name = hidpp10::get_device_codename(&self.transport, slot)?
                    .unwrap_or_else(|| format!("Device {}", slot));
                (pairing.wpid, pairing.kind, serial, name)
            };

            // Try HID++ 2.0 battery and name first; fall back to HID++ 1.0.
            let (battery, firmware) = self.read_device_info(slot, &kind);

            devices.push(DeviceInfo {
                slot,
                name,
                kind,
                serial,
                wpid,
                battery,
                firmware,
            });
        }

        Ok(devices)
    }

    /// Attempt to read battery and firmware from a device, trying HID++ 2.0 first.
    fn read_device_info(&self, slot: u8, _kind: &DeviceKind) -> (Option<Battery>, Vec<crate::devices::types::FirmwareInfo>) {
        // Probe for HID++ 2.0 support.
        match hidpp20::discover_features(&self.transport, slot) {
            Ok(features) if !features.is_empty() => {
                let battery = hidpp20::get_unified_battery(&self.transport, slot, &features)
                    .ok()
                    .flatten()
                    .or_else(|| {
                        hidpp20::get_battery_status(&self.transport, slot, &features)
                            .ok()
                            .flatten()
                    })
                    .or_else(|| {
                        hidpp20::get_battery_voltage(&self.transport, slot, &features)
                            .ok()
                            .flatten()
                    });
                let firmware = hidpp20::get_firmware(&self.transport, slot, &features)
                    .unwrap_or_default();
                (battery, firmware)
            }
            _ => {
                // HID++ 1.0 fallback.
                let battery = hidpp10::get_battery(&self.transport, slot).ok().flatten();
                let firmware = hidpp10::get_firmware(&self.transport, slot).unwrap_or_default();
                (battery, firmware)
            }
        }
    }

    /// Expose the transport for debug/probe tooling.
    pub fn transport(&self) -> &crate::transport::Transport { &self.transport }

    /// Read DPI capabilities and current DPI for the device in the given slot.
    /// Returns None if the device does not support FEAT_ADJUSTABLE_DPI (0x2201).
    pub fn get_dpi_info(&self, slot: u8) -> crate::error::Result<Option<hidpp20::DpiInfo>> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::get_dpi_info(&self.transport, slot, &features)
    }

    /// Set the active DPI for the device in the given slot.
    /// Returns Ok without sending anything if the device does not support FEAT_ADJUSTABLE_DPI.
    pub fn set_dpi(&self, slot: u8, dpi: u16) -> crate::error::Result<()> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::set_dpi(&self.transport, slot, &features, dpi)
    }

    /// Read scroll wheel capabilities and current mode for the device in the given slot.
    /// Returns None if the device does not support FEAT_HIRES_WHEEL (0x2121).
    pub fn get_scroll_info(&self, slot: u8) -> crate::error::Result<Option<hidpp20::ScrollInfo>> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::get_scroll_info(&self.transport, slot, &features)
    }

    /// Set scroll wheel inversion and hi-res mode for the device in the given slot.
    /// Returns Ok without sending anything if the device does not support FEAT_HIRES_WHEEL.
    pub fn set_scroll_settings(&self, slot: u8, inverted: bool, hires_enabled: bool) -> crate::error::Result<()> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::set_scroll_settings(&self.transport, slot, &features, inverted, hires_enabled)
    }

    /// Read smart-shift ratchet mode and torque for the device in the given slot.
    /// Returns None if the device does not support FEAT_SMART_SHIFT_ENHANCED (0x2111).
    pub fn get_smart_shift(&self, slot: u8) -> crate::error::Result<Option<hidpp20::SmartShiftInfo>> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::get_smart_shift(&self.transport, slot, &features)
    }

    /// Set smart-shift wheel mode and torque for the device in the given slot.
    pub fn set_smart_shift(&self, slot: u8, wheel_mode: u8, torque: u8) -> crate::error::Result<()> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::set_smart_shift(&self.transport, slot, &features, wheel_mode, torque)
    }

    /// Read the host slot list for the device in the given slot.
    /// Returns None if the device does not support FEAT_CHANGE_HOST (0x1814).
    pub fn get_hosts(&self, slot: u8) -> crate::error::Result<Option<Vec<hidpp20::HostInfo>>> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::get_hosts(&self.transport, slot, &features)
    }

    /// Switch the active host for the device in the given slot.
    /// The device will disconnect immediately; no reply is expected.
    pub fn set_active_host(&self, slot: u8, host_slot: u8) -> crate::error::Result<()> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::set_active_host(&self.transport, slot, &features, host_slot)
    }

    /// Read FN key inversion state for the device in the given slot.
    /// Returns None if the device does not support any FN inversion feature.
    pub fn get_fn_settings(&self, slot: u8) -> crate::error::Result<Option<hidpp20::FnInfo>> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::get_fn_settings(&self.transport, slot, &features)
    }

    /// Set FN key inversion for the device in the given slot.
    pub fn set_fn_swap(&self, slot: u8, swapped: bool) -> crate::error::Result<()> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::set_fn_swap(&self.transport, slot, &features, swapped)
    }

    /// Read multiplatform OS selection state for the device in the given slot.
    /// Returns None if the device does not support FEAT_MULTIPLATFORM (0x4531) or cannot change OS.
    pub fn get_multiplatform(&self, slot: u8) -> crate::error::Result<Option<hidpp20::MultiplatformInfo>> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::get_multiplatform(&self.transport, slot, &features)
    }

    /// Set the active OS platform for the device in the given slot.
    pub fn set_multiplatform(&self, slot: u8, platform_index: u8) -> crate::error::Result<()> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::set_multiplatform(&self.transport, slot, &features, platform_index)
    }

    /// Read backlight state for the device in the given slot.
    /// Returns None if the device does not support FEAT_BACKLIGHT2 (0x1982).
    pub fn get_backlight(&self, slot: u8) -> crate::error::Result<Option<hidpp20::BacklightInfo>> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::get_backlight(&self.transport, slot, &features)
    }

    /// Set backlight mode and brightness for the device in the given slot.
    pub fn set_backlight(&self, slot: u8, mode: u8, level: u8) -> crate::error::Result<()> {
        let features = hidpp20::discover_features(&self.transport, slot)?;
        hidpp20::set_backlight(&self.transport, slot, &features, mode, level)
    }

    /// Read all configurable settings for the device in the given slot in a single operation.
    ///
    /// Calls discover_features once and reads every feature with the same map, avoiding
    /// the 7x redundant discovery overhead of calling each get_* method individually.
    /// Individual feature errors are treated as absent (None) rather than propagated.
    pub fn get_all_settings(&self, slot: u8) -> crate::error::Result<AllDeviceSettings> {
        let features     = hidpp20::discover_features(&self.transport, slot)?;
        // Clear any 0x1B04 (REPROG_CONTROLS_V4) temporary diversions set by Options+.
        // Diverted buttons send CID events via HID++ that Pulsaar reads and discards,
        // making the physical buttons inert (SmartShift toggle, etc.). Clearing here
        // restores firmware-default behavior while the receiver is open for settings.
        let _ = hidpp20::clear_reprog_controls_diversions(&self.transport, slot, &features);
        let reprog_controls_idx = features.get(&hidpp20::FEAT_REPROG_CONTROLS).copied().unwrap_or(0);
        let dpi          = hidpp20::get_dpi_info(&self.transport, slot, &features).unwrap_or(None);
        let scroll       = hidpp20::get_scroll_info(&self.transport, slot, &features).unwrap_or(None);
        let smart_shift  = hidpp20::get_smart_shift(&self.transport, slot, &features).unwrap_or(None);
        let hosts        = hidpp20::get_hosts(&self.transport, slot, &features).unwrap_or(None);
        let fn_settings  = hidpp20::get_fn_settings(&self.transport, slot, &features).unwrap_or(None);
        let multiplatform = hidpp20::get_multiplatform(&self.transport, slot, &features).unwrap_or(None);
        let backlight    = hidpp20::get_backlight(&self.transport, slot, &features).unwrap_or(None);
        Ok(AllDeviceSettings { dpi, scroll, smart_shift, hosts, fn_settings, multiplatform, backlight, reprog_controls_idx })
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect()
}

// ---------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------

// Sub-IDs for receiver notifications.
const NOTIF_DJ_PAIRING:           u8 = 0x41; // device connection/disconnection
const NOTIF_PAIRING_LOCK:         u8 = 0x4A; // Unifying: lock opened/closed
const NOTIF_POWER:                u8 = 0x4B; // device powered on/off
const NOTIF_PASSKEY_REQUEST:      u8 = 0x4D; // Bolt: passkey request
const NOTIF_DEVICE_DISCOVERY:     u8 = 0x4F; // Bolt: device found during discovery
const NOTIF_DISCOVERY_STATUS:     u8 = 0x53; // Bolt: discovery lock open/closed
const NOTIF_PAIRING_STATUS:       u8 = 0x54; // Bolt: pairing lock open/closed

/// Device discovered by a Bolt receiver during the discovery phase.
/// Collected across up to two 0x4F notification chunks before pairing is initiated.
pub struct BoltDiscoveredDevice {
    pub address:        [u8; 6],
    pub authentication: u8,
    pub kind:           u8,
    pub name:           String,
}

/// State kept across calls to Receiver::poll_pairing.
pub struct PairingSession {
    // Unifying: slot of a newly connected device (set on 0x41 notification).
    pub unifying_new_slot:  Option<u8>,
    // Bolt: data collected from 0x4F discovery notifications.
    pub bolt_discovered:    Option<BoltDiscoveredDevice>,
    // Bolt: passkey string (numeric digits to type, or button sequence).
    pub passkey:            Option<String>,
}

impl PairingSession {
    pub fn new() -> Self {
        Self { unifying_new_slot: None, bolt_discovered: None, passkey: None }
    }
}

/// Events returned by Receiver::poll_pairing.
pub enum PairingEvent {
    /// Still waiting; no relevant notification received within the timeout.
    Waiting,
    /// Bolt: a compatible device was found and pairing has been initiated.
    BoltDeviceFound(String),
    /// Bolt: numeric passkey -- user must type these digits on the keyboard, then press Enter.
    PasskeyNumeric(String),
    /// Bolt: button passkey -- encoded as a string of 'L'/'R' characters; press simultaneously when done.
    PasskeyButton(String),
    /// Pairing complete. slot is the 1-based slot of the new device.
    Paired(u8),
    /// Pairing failed.
    Failed(String),
}

impl Receiver {
    /// Unpair the device in the given 1-based slot.
    pub fn unpair_device(&self, slot: u8) -> Result<()> {
        if self.kind == ReceiverKind::Bolt {
            hidpp10::bolt_unpair_device(&self.transport, slot)
        } else {
            hidpp10::unpair_device(&self.transport, slot)
        }
    }

    /// Open the pairing lock (Unifying) or start device discovery (Bolt).
    /// Returns a PairingSession that must be passed to poll_pairing.
    /// timeout_secs: maximum time the receiver will wait for a device (0-255).
    pub fn start_pairing(&self, timeout_secs: u8) -> Result<PairingSession> {
        // Set exactly WIRELESS | SOFTWARE_PRESENT for pairing notifications.
        // Do not use flags | needed -- that preserves KEYBOARD_MULTIMEDIA_RAW and
        // other flags left by Options+ that route key/button events to HID++ and
        // away from standard USB HID.
        let needed = hidpp10::NOTIF_WIRELESS | hidpp10::NOTIF_SOFTWARE_PRESENT;
        if let Ok(flags) = hidpp10::get_notification_flags(&self.transport) {
            if flags != needed {
                let _ = hidpp10::set_notification_flags(&self.transport, needed);
            }
        }

        if self.kind == ReceiverKind::Bolt {
            hidpp10::bolt_start_discovery(&self.transport, false, timeout_secs)?;
        } else {
            hidpp10::set_pairing_lock(&self.transport, true, timeout_secs)?;
        }
        Ok(PairingSession::new())
    }

    /// Poll for one pairing event. Blocks for at most timeout_ms milliseconds.
    /// Call in a loop (e.g. every 200 ms) until Paired or Failed is returned.
    pub fn poll_pairing(&self, session: &mut PairingSession, timeout_ms: i32) -> Result<PairingEvent> {
        let msg = match self.transport.read_notification(timeout_ms)? {
            Some(m) => m,
            None    => return Ok(PairingEvent::Waiting),
        };

        // Ignore HID++ reply messages (sub_id >= 0x80); we only care about notifications.
        let sub_id = msg.sub_id();
        if sub_id >= 0x80 {
            return Ok(PairingEvent::Waiting);
        }

        let address = msg.address();
        let params  = msg.params().to_vec(); // clone to avoid borrow issues

        if self.kind == ReceiverKind::Bolt {
            match sub_id {
                NOTIF_DISCOVERY_STATUS => {
                    // address 0x00: discovering; anything else: stopped.
                    let error = params.first().copied().unwrap_or(0);
                    if address != 0x00 && error != 0 {
                        return Ok(PairingEvent::Failed(format!("discovery error 0x{:02X}", error)));
                    }
                    Ok(PairingEvent::Waiting)
                }
                NOTIF_DEVICE_DISCOVERY => {
                    // Two chunks arrive for each discovered device.
                    // Chunk type is at params[1]: 0 = address/auth/kind, 1 = name.
                    let chunk = params.get(1).copied().unwrap_or(0xFF);
                    if chunk == 0 && params.len() >= 15 {
                        let mut addr = [0u8; 6];
                        addr.copy_from_slice(&params[6..12]);
                        session.bolt_discovered = Some(BoltDiscoveredDevice {
                            address:        addr,
                            authentication: params[14],
                            kind:           params[3],
                            name:           String::new(),
                        });
                    } else if chunk == 1 {
                        if let Some(dev) = &mut session.bolt_discovered {
                            let name_len = params.get(2).copied().unwrap_or(0) as usize;
                            let end = (3 + name_len).min(params.len());
                            dev.name = String::from_utf8_lossy(&params[3..end]).into_owned();
                        }
                        // Kick off pairing once we have both chunks.
                        if let Some(dev) = &session.bolt_discovered {
                            if !dev.name.is_empty() {
                                // Keyboards get 20 bits of entropy (numeric passkey);
                                // other devices get 10 (button passkey).
                                let entropy = if dev.kind == 1 { 20u8 } else { 10u8 };
                                hidpp10::bolt_pair_device(
                                    &self.transport, 0, &dev.address, dev.authentication, entropy,
                                )?;
                                return Ok(PairingEvent::BoltDeviceFound(dev.name.clone()));
                            }
                        }
                    }
                    Ok(PairingEvent::Waiting)
                }
                NOTIF_PASSKEY_REQUEST => {
                    // params[0..6] = passkey as an ASCII decimal string (e.g. "048" or "512").
                    // Keyboards (auth bit 0 set) display it directly; other devices interpret
                    // the integer as a 10-bit L/R button sequence.
                    let raw = &params[..6.min(params.len())];
                    let passkey = String::from_utf8_lossy(raw).trim_end_matches('\0').to_owned();
                    session.passkey = Some(passkey.clone());
                    let auth = session.bolt_discovered.as_ref().map(|d| d.authentication).unwrap_or(0);
                    if auth & 0x01 != 0 {
                        Ok(PairingEvent::PasskeyNumeric(passkey))
                    } else {
                        Ok(PairingEvent::PasskeyButton(passkey_to_buttons(&passkey)))
                    }
                }
                NOTIF_PAIRING_STATUS => {
                    // address: 0x00=lock open (still pairing), 0x02=paired, other=lock closed.
                    // data[0] (params[0]): 0=no error, 1=timeout, 2=failed.
                    // On success: address=0x02, data[0]=0x00, slot at data[7].
                    let pair_error = params.first().copied().unwrap_or(0);
                    if pair_error != 0 {
                        return Ok(PairingEvent::Failed(format!("pairing error 0x{:02X}", pair_error)));
                    }
                    match address {
                        0x00 => Ok(PairingEvent::Waiting),
                        0x02 => {
                            let slot = params.get(7).copied().unwrap_or(0);
                            Ok(PairingEvent::Paired(slot))
                        }
                        _ => Ok(PairingEvent::Failed("pairing lock closed without a new device".to_owned())),
                    }
                }
                _ => Ok(PairingEvent::Waiting),
            }
        } else {
            // Unifying / Nano / LightSpeed
            match sub_id {
                NOTIF_DJ_PAIRING if msg.device() != 0xFF => {
                    // New device connected; device() holds its 1-based slot.
                    session.unifying_new_slot = Some(msg.device());
                    Ok(PairingEvent::Waiting)
                }
                NOTIF_PAIRING_LOCK => {
                    let lock_open = address & 0x01 != 0;
                    let error     = params.first().copied().unwrap_or(0);
                    if lock_open {
                        return Ok(PairingEvent::Waiting);
                    }
                    // Lock closed.
                    if error != 0 {
                        return Ok(PairingEvent::Failed(format!("pairing error 0x{:02X}", error)));
                    }
                    match session.unifying_new_slot {
                        Some(slot) => Ok(PairingEvent::Paired(slot)),
                        None       => Ok(PairingEvent::Failed("no device connected before lock closed".to_owned())),
                    }
                }
                _ => Ok(PairingEvent::Waiting),
            }
        }
    }

    /// Open a receiver dedicated to monitoring device connection-state events.
    ///
    /// Sets the receiver notification flags to NOTIF_WIRELESS only. This clears
    /// every other flag that Options+ or another Logitech app may have left set,
    /// specifically:
    ///
    ///   KEYBOARD_MULTIMEDIA_RAW (0x010000): when set, the receiver routes consumer-
    ///     control keys (Mute, Volume, etc.) as HID++ notifications instead of standard
    ///     USB HID reports. Pulsaar reads those reports on the vendor interface and
    ///     discards them, so the OS never sees them and the keys stop working.
    ///
    ///   SOFTWARE_PRESENT (0x000800): when set, devices with SPECIAL_KEYS_BUTTONS
    ///     (0x1B04) diversions route media/special key events through HID++ instead of
    ///     standard USB HID. Pulsaar does not handle those events.
    ///
    ///   MOUSE_EXTRA_BUTTONS and others: same problem -- events routed to HID++ that
    ///     Pulsaar does not handle are consumed and lost.
    ///
    /// Setting ONLY NOTIF_WIRELESS ensures that only device-link 0x41 notifications
    /// reach the vendor interface. Everything else routes through standard USB HID.
    pub fn open_for_events(api: &HidApi, handle: &ReceiverHandle) -> Result<Self> {
        let recv = Self::open(api, handle)?;
        if let Ok(flags) = hidpp10::get_notification_flags(&recv.transport) {
            if flags != hidpp10::NOTIF_WIRELESS {
                let _ = hidpp10::set_notification_flags(&recv.transport, hidpp10::NOTIF_WIRELESS);
            }
        }
        Ok(recv)
    }

    /// Poll for one device connection-state event. Blocks for at most timeout_ms milliseconds.
    ///
    /// Returns Some((slot, online)) when a device comes online or goes offline.
    /// Returns None on timeout or for any message that is not a 0x41/0x4B connection notification.
    /// Other HID++ 2.0 unsolicited notifications (key events, battery updates, etc.) are
    /// intentionally ignored here; they must not be treated as connection-state changes.
    ///
    /// Protocol reference (matches Solaar's _process_hidpp10_notification):
    ///   0x41 (DJ_PAIRING): params[0] & 0x40 == 0 => link established (online)
    ///   0x4B (POWER):      address == 0x01        => device powered on (online)
    pub fn poll_device_event(&self, timeout_ms: i32) -> Result<Option<DeviceEvent>> {
        let msg = match self.transport.read_notification(timeout_ms)? {
            Some(m) => m,
            None    => return Ok(None),
        };

        let sub_id = msg.sub_id();
        if sub_id >= 0x80 { return Ok(None); } // HID++ reply, not a notification

        let slot = msg.device();
        if slot == 0xFF || slot == 0 { return Ok(None); } // receiver-level, not a device

        match sub_id {
            NOTIF_DJ_PAIRING => {
                // address 0x00 = unknown/legacy protocol; skip.
                // For all other protocols: params[0] bit 6 inverted = link_established.
                if msg.address() == 0x00 { return Ok(None); }
                let online = (msg.params().first().copied().unwrap_or(0) & 0x40) == 0;
                Ok(Some(DeviceEvent::LinkChange { slot, online }))
            }
            NOTIF_POWER => {
                // address 0x01 = device powered on.
                if msg.address() == 0x01 {
                    Ok(Some(DeviceEvent::LinkChange { slot, online: true }))
                } else {
                    Ok(None)
                }
            }
            _ => {
                // HID++ 2.0 unsolicited feature notification: sub_id = feature index,
                // address bits [3:0] = software_id = 0 for device-initiated events.
                // (Pulsaar uses SOFTWARE_ID = 0x0A for its own requests; replies have
                // address & 0x0F == 0x0A and are NOT seen here since we use request()
                // on a separate handle. A value of 0 here means the device sent this
                // without prompting -- i.e. a state-change notification.)
                if (msg.address() & 0x0F) == 0 {
                    Ok(Some(DeviceEvent::SettingsChanged { slot, feature_index: sub_id }))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Cancel an in-progress pairing by closing the lock / stopping discovery.
    pub fn cancel_pairing(&self) -> Result<()> {
        if self.kind == ReceiverKind::Bolt {
            hidpp10::bolt_start_discovery(&self.transport, true, 0)?;
        } else {
            hidpp10::set_pairing_lock(&self.transport, false, 0)?;
        }
        Ok(())
    }
}

/// Convert a Bolt button-passkey string to an L/R sequence.
///
/// The receiver sends the passkey as an ASCII decimal string (e.g. "048").
/// Parse it as a u32, then read the 10 least-significant bits MSB-first:
/// bit=1 -> 'R' (right button), bit=0 -> 'L' (left button).
///
/// This matches Solaar's conversion: int(passkey_str) formatted as `{:010b}`,
/// where '1'=right and '0'=left.
fn passkey_to_buttons(passkey_str: &str) -> String {
    let n: u32 = passkey_str.trim_end_matches('\0').parse().unwrap_or(0);
    (0..10).rev().map(|i| if (n >> i) & 1 != 0 { 'R' } else { 'L' }).collect()
}
