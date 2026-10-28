// Receiver enumeration and device management.
// Reference: reference/lib/logitech_receiver/receiver.py, base_usb.py

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use hidapi::HidApi;

use crate::devices::types::{Battery, DeviceInfo};
use crate::error::Result;
use crate::hidpp::hidpp10::{self, bytes_to_hex};
use crate::hidpp::hidpp20;
use crate::transport::Transport;

const LOGITECH_VID: u16 = 0x046D;

/// All known receiver product IDs with their kind.
const RECEIVERS: &[(u16, ReceiverKind)] = &[
    // Bolt
    (0xC548, ReceiverKind::Bolt),
    // Unifying
    (0xC52B, ReceiverKind::Unifying),
    (0xC532, ReceiverKind::Unifying),
    // Nano
    (0xC52F, ReceiverKind::Nano),
    (0xC521, ReceiverKind::Nano),
    (0xC525, ReceiverKind::Nano),
    (0xC526, ReceiverKind::Nano),
    (0xC52E, ReceiverKind::Nano),
    (0xC531, ReceiverKind::Nano),
    (0xC534, ReceiverKind::Nano),
    (0xC535, ReceiverKind::Nano),
    (0xC537, ReceiverKind::Nano),
    // LightSpeed
    (0xC539, ReceiverKind::LightSpeed),
    (0xC53A, ReceiverKind::LightSpeed),
    (0xC53D, ReceiverKind::LightSpeed),
    (0xC53F, ReceiverKind::LightSpeed),
    (0xC541, ReceiverKind::LightSpeed),
    (0xC545, ReceiverKind::LightSpeed),
    (0xC547, ReceiverKind::LightSpeed),
    (0xC54D, ReceiverKind::LightSpeed),
];

/// True if the product ID belongs to a known receiver. Used by the direct-device
/// enumerator to skip receiver HID interfaces that might otherwise look like BT
/// direct devices.
pub fn is_receiver_pid(pid: u16) -> bool {
    RECEIVERS.iter().any(|&(p, _)| p == pid)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ReceiverKind {
    Unifying = 0,
    Bolt = 1,
    Nano = 2,
    LightSpeed = 3,
}

impl ReceiverKind {
    /// Display name shown in the frontends.
    pub fn name(self) -> &'static str {
        match self {
            ReceiverKind::Unifying => "Unifying Receiver",
            ReceiverKind::Bolt => "Bolt Receiver",
            ReceiverKind::Nano => "Nano Receiver",
            ReceiverKind::LightSpeed => "Lightspeed Receiver",
        }
    }
}

/// Lightweight descriptor returned from enumerate_receivers before opening.
#[derive(Debug, Clone)]
pub struct ReceiverHandle {
    /// HID path for sending commands (usage=0x0001 on Windows, the sole path elsewhere).
    pub write_path: String,
    /// HID path for receiving replies. Equals write_path on macOS/Linux; on Windows HID++
    /// receivers replies come from the usage=0x0002 collection of the same interface.
    pub read_path: String,
    pub product_id: u16,
    pub kind: ReceiverKind,
    pub name: &'static str,
}

/// All configurable settings for one device, batched from a single feature discovery.
#[derive(Debug, Clone)]
pub struct AllDeviceSettings {
    pub dpi: Option<hidpp20::DpiInfo>,
    pub scroll: Option<hidpp20::ScrollInfo>,
    pub smart_shift: Option<hidpp20::SmartShiftInfo>,
    pub hosts: Option<Vec<hidpp20::HostInfo>>,
    pub fn_settings: Option<hidpp20::FnInfo>,
    pub multiplatform: Option<hidpp20::MultiplatformInfo>,
    pub backlight: Option<hidpp20::BacklightInfo>,
    /// Feature index of REPROG_CONTROLS_V4 (0x1B04) for this device, or 0 if absent.
    /// Used to filter out button-CID HID++ notifications (which look like settings
    /// change events) in the event listener.
    pub reprog_controls_idx: u8,
    /// Read with the same feature map, so a device notification (a charger
    /// plugged in, the level dropping) refreshes the battery too.
    pub battery: Option<Battery>,
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
    /// Feature table per slot, read once: a slot's features only change when
    /// another device is paired there, and pairing reopens the receiver.
    features: RefCell<HashMap<u8, HashMap<u16, u8>>>,
}

/// Enumerate all Logitech receivers attached to the machine.
///
/// Filters to usage_page=0xFF00, usage=0x0001 (the HID++ vendor interface) to
/// get exactly one entry per physical receiver and avoid the HID interfaces
/// that require input-monitoring permissions on macOS.
///
/// On Windows, Logitech USB receivers expose their vendor interface as two HID
/// top-level collections: usage=0x0001 (output/command direction) and usage=0x0002
/// (input/reply direction). We find both so Transport can use split handles.
/// On macOS/Linux a single path covers both directions; read_path == write_path.
pub fn enumerate_receivers(api: &HidApi) -> Vec<ReceiverHandle> {
    // Collect usage=0x0002 paths as candidates for the read handle on Windows.
    let read_candidates: Vec<(u16, String)> = api
        .device_list()
        .filter(|d| d.vendor_id() == LOGITECH_VID)
        .filter(|d| d.usage_page() == 0xFF00 && d.usage() == 0x0002)
        .filter(|d| is_receiver_pid(d.product_id()))
        .map(|d| (d.product_id(), d.path().to_string_lossy().into_owned()))
        .collect();

    api.device_list()
        .filter(|d| d.vendor_id() == LOGITECH_VID)
        .filter(|d| d.usage_page() == 0xFF00 && d.usage() == 0x0001)
        .filter_map(|d| {
            let pid = d.product_id();
            RECEIVERS
                .iter()
                .find(|&&(p, _)| p == pid)
                .map(|&(_, kind)| {
                    let write_path = d.path().to_string_lossy().into_owned();

                    // Match a read candidate by same PID and shared hardware-instance prefix.
                    // Windows paths: "\\?\HID#...#7&391789b7&0&0000#{...}" -- the "7&391789b7"
                    // part identifies the physical interface; Col01 ends in &0&0000, Col02 in &0&0001.
                    let hw_base = hw_instance_base(&write_path).to_owned();
                    let read_path = read_candidates
                        .iter()
                        .find(|(rpid, rpath)| *rpid == pid && rpath.contains(&hw_base))
                        .map(|(_, rpath)| rpath.clone())
                        .unwrap_or_else(|| write_path.clone());

                    ReceiverHandle {
                        write_path,
                        read_path,
                        product_id: pid,
                        kind,
                        name: kind.name(),
                    }
                })
        })
        .collect()
}

/// Extract the device-instance base from a Windows HID path for interface matching.
/// "\\?\HID#...#7&391789b7&0&0000#{...}" returns "7&391789b7".
/// For non-Windows paths the full path is returned (matching falls back to exact equality).
fn hw_instance_base(path: &str) -> &str {
    let mut parts = path.splitn(4, '#');
    let _ = parts.next(); // "\\?\HID"
    let _ = parts.next(); // device-ID segment
    if let Some(instance) = parts.next() {
        // Strip the "&0&XXXX" collection-index suffix from "7&391789b7&0&0000".
        if let Some(idx) = instance.rfind("&0&") {
            return &instance[..idx];
        }
        return instance;
    }
    path
}

impl Receiver {
    /// Open a receiver by its handle descriptor.
    pub fn open(api: &HidApi, handle: &ReceiverHandle) -> Result<Self> {
        let transport = Transport::open(api, &handle.write_path, &handle.read_path)?;

        // Bolt uses a different register for serial and does not support the standard
        // RECEIVER_INFO sub-register 0x03 that Unifying/Nano/LightSpeed use.
        let (serial, max_devices) = if handle.kind == ReceiverKind::Bolt {
            let serial =
                hidpp10::get_bolt_serial(&transport).unwrap_or_else(|_| String::from("unknown"));
            (serial, 6u8) // Bolt supports up to 6 paired devices
        } else {
            hidpp10::get_receiver_info(&transport).unwrap_or_else(|_| (String::from("unknown"), 1))
        };

        Ok(Self {
            transport,
            kind: handle.kind,
            name: handle.name,
            product_id: handle.product_id,
            serial,
            max_devices,
            features: RefCell::default(),
        })
    }

    /// The slot's feature table: from the cache, or discovered (with `probe`
    /// as the first request's timeout) and cached when the device answered.
    fn features(&self, slot: u8, probe: Option<Duration>) -> Result<HashMap<u16, u8>> {
        if let Some(f) = self.features.borrow().get(&slot) {
            return Ok(f.clone());
        }
        let f = hidpp20::discover_features(&self.transport, slot, probe)?;
        if !f.is_empty() {
            self.features.borrow_mut().insert(slot, f.clone());
        }
        Ok(f)
    }

    /// Query the receiver for the number of currently paired devices.
    /// Falls back to max_devices if the register read fails.
    pub fn get_paired_device_count(&self) -> usize {
        hidpp10::get_paired_device_count(&self.transport).unwrap_or(self.max_devices as usize)
    }

    /// Ask the receiver for the link state of its paired devices and collect
    /// the answers: slot to linked. Waits until `expected` slots answered or
    /// 300 ms passed. Empty when the receiver does not support the request.
    pub fn link_states(&self, expected: usize) -> HashMap<u8, bool> {
        let mut states = HashMap::new();
        if self.trigger_link_notifications().is_err() {
            return states;
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(300);
        while states.len() < expected && std::time::Instant::now() < deadline {
            if let Ok(Some(DeviceEvent::LinkChange { slot, online })) = self.poll_device_event(20) {
                states.insert(slot, online);
            }
        }
        states
    }

    /// Probe one slot and return its DeviceInfo, or None if the slot is empty or errored.
    /// Contains all the per-slot pairing + HID++ 2.0 discovery logic.
    ///
    /// `linked` is the receiver's link state for the slot from `link_states`.
    /// An unlinked device is not probed. A linked one is probed with the full
    /// device timeout, since a dozing device can take most of a second to
    /// answer. Without it, a probe answered within 500 ms counts as online.
    pub fn enumerate_slot(&self, slot: u8, linked: Option<bool>) -> Option<DeviceInfo> {
        let t_slot = std::time::Instant::now();

        let (wpid, kind, serial, name) = if self.kind == ReceiverKind::Bolt {
            let bolt = match hidpp10::get_bolt_pairing_info(&self.transport, slot) {
                Ok(Some(p)) => p,
                Ok(None) | Err(_) => {
                    eprintln!(
                        "[PULSAAR][TIMING] T+{}ms slot {slot}: pairing=empty/err ({:.0}ms)",
                        crate::t_ms(),
                        t_slot.elapsed().as_secs_f64() * 1000.0
                    );
                    return None;
                }
            };
            let name = hidpp10::get_bolt_device_codename(&self.transport, slot)
                .unwrap_or(None)
                .unwrap_or_else(|| format!("Device {}", slot));
            eprintln!(
                "[PULSAAR][TIMING] T+{}ms slot {slot}: pairing+name ({:.0}ms)",
                crate::t_ms(),
                t_slot.elapsed().as_secs_f64() * 1000.0
            );
            (bolt.wpid, bolt.kind, bytes_to_hex(&bolt.serial), name)
        } else {
            let pairing = match hidpp10::get_pairing_info(&self.transport, slot) {
                Ok(Some(p)) => p,
                Ok(None) | Err(_) => {
                    eprintln!(
                        "[PULSAAR][TIMING] T+{}ms slot {slot}: pairing=empty/err ({:.0}ms)",
                        crate::t_ms(),
                        t_slot.elapsed().as_secs_f64() * 1000.0
                    );
                    return None;
                }
            };
            let serial = hidpp10::get_extended_pairing_info(&self.transport, slot)
                .unwrap_or(None)
                .map(|s| bytes_to_hex(&s))
                .unwrap_or_default();
            let name = hidpp10::get_device_codename(&self.transport, slot)
                .unwrap_or(None)
                .unwrap_or_else(|| format!("Device {}", slot));
            eprintln!(
                "[PULSAAR][TIMING] T+{}ms slot {slot}: pairing+name ({:.0}ms)",
                crate::t_ms(),
                t_slot.elapsed().as_secs_f64() * 1000.0
            );
            (pairing.wpid, pairing.kind, serial, name)
        };

        let t_info = std::time::Instant::now();
        let (hidpp2_name, battery, online) = self.read_device_info(slot, linked);
        eprintln!(
            "[PULSAAR][TIMING] T+{}ms slot {slot}: read_device_info ({:.0}ms)",
            crate::t_ms(),
            t_info.elapsed().as_secs_f64() * 1000.0
        );
        let name = hidpp2_name.unwrap_or(name);

        Some(DeviceInfo {
            slot,
            name,
            kind,
            serial,
            wpid,
            battery,
            online,
        })
    }

    /// Enumerate all devices paired to this receiver.
    ///
    /// For each occupied slot, reads pairing info (HID++ 1.0) to determine WPID
    /// and kind, then tries to read battery and name.
    pub fn enumerate_devices(&self) -> Result<Vec<DeviceInfo>> {
        let mut devices = Vec::new();
        let t_total = std::time::Instant::now();

        let paired_count = self.get_paired_device_count();
        eprintln!(
            "[PULSAAR][TIMING] paired_count from receiver: {}",
            paired_count
        );

        let mut found = 0usize;
        for slot in 1..=self.max_devices {
            if found >= paired_count {
                break;
            }
            if let Some(info) = self.enumerate_slot(slot, None) {
                devices.push(info);
                found += 1;
            }
        }

        eprintln!(
            "[PULSAAR][TIMING] enumerate_devices total: {:.0}ms, {} device(s)",
            t_total.elapsed().as_secs_f64() * 1000.0,
            devices.len()
        );
        Ok(devices)
    }

    /// Attempt to read name and battery from a device, trying HID++ 2.0 first.
    /// The third value is the online state: `linked` when known, otherwise
    /// whether the device answered HID++ 2.0.
    fn read_device_info(
        &self,
        slot: u8,
        linked: Option<bool>,
    ) -> (Option<String>, Option<Battery>, bool) {
        if linked == Some(false) {
            return (None, None, false);
        }
        // Without a link state, probe with a short timeout so offline devices are
        // skipped in ~500ms instead of waiting for the full DEVICE_TIMEOUT.
        let probe = if linked.is_some() {
            None
        } else {
            Some(Duration::from_millis(500))
        };
        let t_feat = std::time::Instant::now();
        let feat_result = self.features(slot, probe);
        eprintln!(
            "[PULSAAR][TIMING] T+{}ms slot {slot}: discover_features={} ({:.0}ms)",
            crate::t_ms(),
            if feat_result.is_ok() { "ok" } else { "err" },
            t_feat.elapsed().as_secs_f64() * 1000.0
        );
        match feat_result {
            Ok(features) if !features.is_empty() => {
                // Prefer device name (0x0005, full product name e.g. "MX Anywhere 3S for Business");
                // fall back to friendly name (0x0007, shorter identifier e.g. "MX Anywhere3SB").
                let name = hidpp20::get_device_name(&self.transport, slot, &features)
                    .ok()
                    .flatten()
                    .or_else(|| {
                        hidpp20::get_friendly_name(&self.transport, slot, &features)
                            .ok()
                            .flatten()
                    });
                let battery = self.read_battery(slot, &features);
                (name, battery, true)
            }
            _ => {
                // discover_features failed: device is offline (wireless) or HID++ 1.0 only.
                // Skip the HID++ 1.0 battery fallback -- those requests go to the device
                // slot and each times out at DEVICE_TIMEOUT for offline Bolt/Unifying devices.
                // Battery for HID++ 1.0 devices comes from prefetch/cache instead.
                (None, None, linked == Some(true))
            }
        }
    }

    /// Battery from whichever of UNIFIED_BATTERY, BATTERY_STATUS and
    /// BATTERY_VOLTAGE the device has, in that order.
    fn read_battery(&self, slot: u8, features: &HashMap<u16, u8>) -> Option<Battery> {
        hidpp20::get_unified_battery(&self.transport, slot, features)
            .ok()
            .flatten()
            .or_else(|| {
                hidpp20::get_battery_status(&self.transport, slot, features)
                    .ok()
                    .flatten()
            })
            .or_else(|| {
                hidpp20::get_battery_voltage(&self.transport, slot, features)
                    .ok()
                    .flatten()
            })
    }

    /// Expose the transport for debug/probe tooling.
    pub fn transport(&self) -> &crate::transport::Transport {
        &self.transport
    }

    /// Set the active DPI for the device in the given slot.
    /// Returns Ok without sending anything if the device does not support FEAT_ADJUSTABLE_DPI.
    pub fn set_dpi(&self, slot: u8, dpi: u16) -> crate::error::Result<()> {
        let features = self.features(slot, None)?;
        hidpp20::set_dpi(&self.transport, slot, &features, dpi)
    }

    /// Set scroll wheel inversion and hi-res mode for the device in the given slot.
    /// Returns Ok without sending anything if the device does not support FEAT_HIRES_WHEEL.
    pub fn set_scroll_settings(
        &self,
        slot: u8,
        inverted: bool,
        hires_enabled: bool,
    ) -> crate::error::Result<()> {
        let features = self.features(slot, None)?;
        hidpp20::set_scroll_settings(&self.transport, slot, &features, inverted, hires_enabled)
    }

    /// Set smart-shift wheel mode and torque for the device in the given slot.
    pub fn set_smart_shift(
        &self,
        slot: u8,
        wheel_mode: u8,
        torque: u8,
    ) -> crate::error::Result<()> {
        let features = self.features(slot, None)?;
        hidpp20::set_smart_shift(&self.transport, slot, &features, wheel_mode, torque)
    }

    /// Switch the active host for the device in the given slot.
    /// The device will disconnect immediately; no reply is expected.
    pub fn set_active_host(&self, slot: u8, host_slot: u8) -> crate::error::Result<()> {
        let features = self.features(slot, None)?;
        hidpp20::set_active_host(&self.transport, slot, &features, host_slot)
    }

    /// Write the given name to a host slot for the device.
    /// No-op if FEAT_HOSTS_INFO (0x1815) is absent or the device does not support name writes.
    pub fn set_host_name(&self, slot: u8, host_slot: u8, name: &str) -> crate::error::Result<()> {
        let features = self.features(slot, None)?;
        hidpp20::set_host_name(&self.transport, slot, &features, host_slot, name)
    }

    /// Set FN key inversion for the device in the given slot.
    pub fn set_fn_swap(&self, slot: u8, swapped: bool) -> crate::error::Result<()> {
        let features = self.features(slot, None)?;
        hidpp20::set_fn_swap(&self.transport, slot, &features, swapped)
    }

    /// Set the active OS platform for the device in the given slot.
    pub fn set_multiplatform(&self, slot: u8, platform_index: u8) -> crate::error::Result<()> {
        let features = self.features(slot, None)?;
        hidpp20::set_multiplatform(&self.transport, slot, &features, platform_index)
    }

    /// Set backlight mode and brightness for the device in the given slot.
    pub fn set_backlight(&self, slot: u8, mode: u8, level: u8) -> crate::error::Result<()> {
        let features = self.features(slot, None)?;
        hidpp20::set_backlight(&self.transport, slot, &features, mode, level)
    }

    /// Read all configurable settings for the device in the given slot in a single operation.
    ///
    /// Uses the slot's feature table once for every feature, avoiding
    /// the 7x redundant discovery overhead of calling each get_* method individually.
    /// Individual feature errors are treated as absent (None) rather than propagated.
    pub fn get_all_settings(&self, slot: u8) -> crate::error::Result<AllDeviceSettings> {
        let features = self.features(slot, None)?;
        // Clear any 0x1B04 (REPROG_CONTROLS_V4) temporary diversions set by Options+.
        // Diverted buttons send CID events via HID++ that Pulsaar reads and discards,
        // making the physical buttons inert (SmartShift toggle, etc.). Clearing here
        // restores firmware-default behavior while the receiver is open for settings.
        let _ = hidpp20::clear_reprog_controls_diversions(&self.transport, slot, &features);
        let reprog_controls_idx = features
            .get(&hidpp20::FEAT_REPROG_CONTROLS)
            .copied()
            .unwrap_or(0);
        let dpi = hidpp20::get_dpi_info(&self.transport, slot, &features).unwrap_or(None);
        let scroll = hidpp20::get_scroll_info(&self.transport, slot, &features).unwrap_or(None);
        let smart_shift =
            hidpp20::get_smart_shift(&self.transport, slot, &features).unwrap_or(None);
        let hosts = hidpp20::get_hosts(&self.transport, slot, &features).unwrap_or(None);
        let fn_settings =
            hidpp20::get_fn_settings(&self.transport, slot, &features).unwrap_or(None);
        let multiplatform =
            hidpp20::get_multiplatform(&self.transport, slot, &features).unwrap_or(None);
        let backlight = hidpp20::get_backlight(&self.transport, slot, &features).unwrap_or(None);
        let battery = self.read_battery(slot, &features);
        Ok(AllDeviceSettings {
            dpi,
            scroll,
            smart_shift,
            hosts,
            fn_settings,
            multiplatform,
            backlight,
            reprog_controls_idx,
            battery,
        })
    }
}

// ---------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------

// Sub-IDs for receiver notifications.
const NOTIF_DJ_PAIRING: u8 = 0x41; // device connection/disconnection
const DJ_CONNECTION_STATUS: u8 = 0x42; // DJ report type: link established or lost
const NOTIF_PAIRING_LOCK: u8 = 0x4A; // Unifying: lock opened/closed
const NOTIF_POWER: u8 = 0x4B; // device powered on/off
const NOTIF_PASSKEY_REQUEST: u8 = 0x4D; // Bolt: passkey request
const NOTIF_PASSKEY_PRESSED: u8 = 0x4E; // Bolt: a passkey digit or button was entered
const NOTIF_DEVICE_DISCOVERY: u8 = 0x4F; // Bolt: device found during discovery
const NOTIF_DISCOVERY_STATUS: u8 = 0x53; // Bolt: discovery lock open/closed
const NOTIF_PAIRING_STATUS: u8 = 0x54; // Bolt: pairing lock open/closed

/// Device discovered by a Bolt receiver during the discovery phase.
/// Collected across up to two 0x4F notification chunks before pairing is initiated.
pub struct BoltDiscoveredDevice {
    pub address: [u8; 6],
    pub authentication: u8,
    pub kind: u8,
    pub name: String,
}

/// State kept across calls to Receiver::poll_pairing.
#[derive(Default)]
pub struct PairingSession {
    // Unifying: slot of a newly connected device (set on 0x41 notification).
    pub unifying_new_slot: Option<u8>,
    // Bolt: data collected from 0x4F discovery notifications.
    pub bolt_discovered: Option<BoltDiscoveredDevice>,
    // Bolt: passkey string (numeric digits to type, or button sequence).
    pub passkey: Option<String>,
}

/// One step of passkey entry on the device being paired.
///
/// From Bolt's 0x4E notification, which Solaar ignores. The address byte is
/// the Bluetooth LE SMP keypress type: 0 started, 1 entered, 2 erased,
/// 3 cleared, 4 completed; the params carry the device's address. Confirmed
/// 2026-10-28 pairing an MX Anywhere 3S (00, ten times 01, then 04) and a
/// keyboard with a 6-digit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyEntry {
    Started,
    Entered,
    Erased,
    Cleared,
    Completed,
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
    /// Bolt: the device reported passkey entry progress.
    PasskeyProgress(PasskeyEntry),
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
        Ok(PairingSession::default())
    }

    /// Poll for one pairing event. Blocks for at most timeout_ms milliseconds.
    /// Call in a loop (e.g. every 200 ms) until Paired or Failed is returned.
    pub fn poll_pairing(
        &self,
        session: &mut PairingSession,
        timeout_ms: i32,
    ) -> Result<PairingEvent> {
        let msg = match self.transport.read_one(timeout_ms)? {
            Some(m) => m,
            None => return Ok(PairingEvent::Waiting),
        };

        // Ignore HID++ reply messages (sub_id >= 0x80) and DJ reports; we only
        // care about HID++ notifications.
        let sub_id = msg.sub_id();
        if sub_id >= 0x80 || msg.is_dj() {
            return Ok(PairingEvent::Waiting);
        }

        let address = msg.address();
        let params = msg.params().to_vec(); // clone to avoid borrow issues

        if self.kind == ReceiverKind::Bolt {
            match sub_id {
                NOTIF_DISCOVERY_STATUS => {
                    // address 0x00: discovering; anything else: stopped.
                    let error = params.first().copied().unwrap_or(0);
                    if address != 0x00 && error != 0 {
                        return Ok(PairingEvent::Failed(format!(
                            "discovery error 0x{:02X}",
                            error
                        )));
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
                            address: addr,
                            authentication: params[14],
                            kind: params[3],
                            name: String::new(),
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
                                    &self.transport,
                                    0,
                                    &dev.address,
                                    dev.authentication,
                                    entropy,
                                )?;
                                return Ok(PairingEvent::BoltDeviceFound(dev.name.clone()));
                            }
                        }
                    }
                    Ok(PairingEvent::Waiting)
                }
                NOTIF_PASSKEY_PRESSED => {
                    eprintln!(
                        "[PULSAAR][PAIRING] passkey pressed: address={:02X} params={:02X?}",
                        address, params
                    );
                    let entry = match address {
                        0 => PasskeyEntry::Started,
                        1 => PasskeyEntry::Entered,
                        2 => PasskeyEntry::Erased,
                        3 => PasskeyEntry::Cleared,
                        4 => PasskeyEntry::Completed,
                        _ => return Ok(PairingEvent::Waiting),
                    };
                    Ok(PairingEvent::PasskeyProgress(entry))
                }
                NOTIF_PASSKEY_REQUEST => {
                    // params[0..6] = passkey as an ASCII decimal string (e.g. "048" or "512").
                    // Keyboards (auth bit 0 set) display it directly; other devices interpret
                    // the integer as a 10-bit L/R button sequence.
                    let raw = &params[..6.min(params.len())];
                    let passkey = String::from_utf8_lossy(raw)
                        .trim_end_matches('\0')
                        .to_owned();
                    session.passkey = Some(passkey.clone());
                    let auth = session
                        .bolt_discovered
                        .as_ref()
                        .map(|d| d.authentication)
                        .unwrap_or(0);
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
                        return Ok(PairingEvent::Failed(format!(
                            "pairing error 0x{:02X}",
                            pair_error
                        )));
                    }
                    match address {
                        0x00 => Ok(PairingEvent::Waiting),
                        0x02 => {
                            let slot = params.get(7).copied().unwrap_or(0);
                            Ok(PairingEvent::Paired(slot))
                        }
                        _ => Ok(PairingEvent::Failed(
                            "pairing lock closed without a new device".to_owned(),
                        )),
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
                    let error = params.first().copied().unwrap_or(0);
                    if lock_open {
                        return Ok(PairingEvent::Waiting);
                    }
                    // Lock closed.
                    if error != 0 {
                        return Ok(PairingEvent::Failed(format!(
                            "pairing error 0x{:02X}",
                            error
                        )));
                    }
                    match session.unifying_new_slot {
                        Some(slot) => Ok(PairingEvent::Paired(slot)),
                        None => Ok(PairingEvent::Failed(
                            "no device connected before lock closed".to_owned(),
                        )),
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
        recv.listen_for_events();
        Ok(recv)
    }

    /// Make the receiver report the link state of every paired device as
    /// `poll_device_event` link changes.
    pub fn trigger_link_notifications(&self) -> Result<()> {
        hidpp10::trigger_link_notifications(&self.transport)
    }

    /// Set the notification flags to NOTIF_WIRELESS only, as `open_for_events`
    /// does; call after pairing, which sets more flags.
    pub fn listen_for_events(&self) {
        if let Ok(flags) = hidpp10::get_notification_flags(&self.transport) {
            if flags != hidpp10::NOTIF_WIRELESS {
                let _ = hidpp10::set_notification_flags(&self.transport, hidpp10::NOTIF_WIRELESS);
            }
        }
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
        let msg = match self.transport.read_one(timeout_ms)? {
            Some(m) => m,
            None => return Ok(None),
        };

        let sub_id = msg.sub_id();
        if sub_id >= 0x80 {
            return Ok(None);
        } // HID++ reply, not a notification

        let slot = msg.device();
        if slot == 0xFF || slot == 0 {
            return Ok(None);
        } // receiver-level, not a device
          // DJ mode (Linux kernel driver): 0x42 is the connection status, bit 0
          // of the first byte set means the link is lost (Solaar's
          // _process_dj_notification). Types below 0x10 are forwarded input.
        if msg.is_dj() {
            return Ok(
                (sub_id == DJ_CONNECTION_STATUS).then(|| DeviceEvent::LinkChange {
                    slot,
                    online: msg.address() & 0x01 == 0,
                }),
            );
        }

        match sub_id {
            NOTIF_DJ_PAIRING => {
                // address 0x00 = unknown/legacy protocol; skip.
                // For all other protocols: params[0] bit 6 inverted = link_established.
                if msg.address() == 0x00 {
                    return Ok(None);
                }
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
                    Ok(Some(DeviceEvent::SettingsChanged {
                        slot,
                        feature_index: sub_id,
                    }))
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
    (0..10)
        .rev()
        .map(|i| if (n >> i) & 1 != 0 { 'R' } else { 'L' })
        .collect()
}
