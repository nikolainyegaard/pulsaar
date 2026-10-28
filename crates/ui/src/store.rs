//! The HID worker. One thread owns every HID handle: each receiver is opened
//! once per reload and kept open, and requests, notifications and pairing all
//! go through that one handle in turn. That removes the exclusive-open races
//! the SwiftUI app worked around (macOS allows one handle per receiver).
//!
//! The UI sends [`Command`]s and reads the shared [`Model`]. The worker locks
//! the model only to copy results in, never across a HID call, and asks egui
//! for a repaint after each change.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui;
use pulsaar_core::direct::enumerate_direct_devices;
use pulsaar_core::receiver::{AllDeviceSettings, DeviceEvent, PairingEvent, PairingSession};
use pulsaar_core::{
    enumerate_receivers, Battery, BatteryStatus, DeviceInfo, DirectDeviceInfo, HidApi, Receiver,
    ReceiverKind,
};
use pulsaar_native::cache::{CachedDevice, DeviceCache};

/// A paired device: its receiver's position in [`Model::receivers`] and its slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceKey {
    pub receiver: usize,
    pub slot: u8,
}

/// A setting to write to a device. Values are the device's own encoding.
#[derive(Clone, Copy, Debug)]
pub enum Write {
    Dpi(u16),
    Scroll { inverted: bool, hires: bool },
    SmartShift { wheel_mode: u8, torque: u8 },
    Host(u8),
    FnSwap(bool),
    Platform(u8),
    Backlight { mode: u8, level: u8 },
}

pub enum Command {
    Reload,
    Write(DeviceKey, Write),
    Unpair(DeviceKey),
    StartPairing(usize),
    CancelPairing,
    /// Close the pairing dialog after success or failure.
    ClearPairing,
}

#[derive(Clone, Debug)]
pub struct ReceiverView {
    pub name: &'static str,
    pub kind: ReceiverKind,
    pub product_id: u16,
    pub serial: String,
    pub max_devices: u8,
    pub devices: Vec<DeviceInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PairingStage {
    Waiting,
    /// Bolt: a device was found and pairing started; the name is in `device_name`.
    Found,
    /// Bolt: digits to type on the keyboard, or an L/R sequence of mouse buttons.
    Passkey {
        code: String,
        numeric: bool,
    },
    Paired {
        slot: u8,
    },
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Pairing {
    pub receiver: usize,
    pub stage: PairingStage,
    pub device_name: String,
}

#[derive(Default)]
pub struct Model {
    pub receivers: Vec<ReceiverView>,
    pub direct: Vec<DirectDeviceInfo>,
    /// Settings of online devices; a device missing here has none or is offline.
    pub settings: HashMap<DeviceKey, AllDeviceSettings>,
    /// Devices whose battery is the last one seen, not a live read.
    pub cached_battery: HashSet<DeviceKey>,
    /// True while a reload enumerates; devices appear one by one meanwhile.
    pub loading: bool,
    pub error: Option<String>,
    pub pairing: Option<Pairing>,
    /// A short confirmation and when it should disappear.
    pub toast: Option<(String, Instant)>,
}

impl Model {
    pub fn device(&self, key: DeviceKey) -> Option<&DeviceInfo> {
        self.receivers
            .get(key.receiver)?
            .devices
            .iter()
            .find(|d| d.slot == key.slot)
    }
}

/// The UI's side of the worker.
pub struct Store {
    model: Arc<Mutex<Model>>,
    tx: Sender<Command>,
}

impl Store {
    /// Starts the worker, which reloads at once.
    pub fn start(ctx: egui::Context) -> Self {
        let model = Arc::new(Mutex::new(Model::default()));
        let (tx, rx) = mpsc::channel();
        let shared = Arc::clone(&model);
        let spawned = std::thread::Builder::new()
            .name("hid".into())
            .spawn(move || {
                // A malformed reply can panic the parser (see crates/core/src/lib.rs).
                // Start over a few times rather than leave a window that never updates.
                for _ in 0..3 {
                    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        match pulsaar_core::init() {
                            Ok(api) => Worker::new(api, Arc::clone(&shared), ctx.clone()).run(&rx),
                            Err(e) => {
                                lock(&shared).error = Some(format!("HID is unavailable: {e}"));
                            }
                        }
                    }));
                    let Err(panic) = run else {
                        break;
                    };
                    let what = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    let mut m = lock(&shared);
                    // A toast, since the restart's reload clears `error`.
                    let text = format!(
                        "A device sent a reply Pulsaar could not read ({what}). Scanning again."
                    );
                    m.toast = Some((text, Instant::now() + Duration::from_secs(8)));
                    m.loading = false;
                    m.pairing = None;
                }
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            lock(&model).error = Some(format!("Could not start the HID thread: {e}"));
        }
        Self { model, tx }
    }

    pub fn model(&self) -> MutexGuard<'_, Model> {
        lock(&self.model)
    }

    /// Write a setting. The model shows the new value at once; the worker
    /// confirms it, or reads the device again when the write fails.
    pub fn write(&self, key: DeviceKey, w: Write) {
        if let Some(s) = self.model().settings.get_mut(&key) {
            patch(s, w);
        }
        self.send(Command::Write(key, w));
    }

    pub fn send(&self, cmd: Command) {
        // Only fails when the worker is gone, which the error line already says.
        let _ = self.tx.send(cmd);
    }
}

/// A panic on the worker poisons the lock; the model is still usable.
fn lock(m: &Mutex<Model>) -> MutexGuard<'_, Model> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How long a device-initiated event waits for more before it is acted on.
const DEBOUNCE: Duration = Duration::from_millis(750);
const PLUG_CHECK: Duration = Duration::from_secs(2);
const TOAST: Duration = Duration::from_secs(2);
const PAIRING_TIMEOUT_SECS: u8 = 30;

struct Worker {
    api: HidApi,
    model: Arc<Mutex<Model>>,
    ctx: egui::Context,
    /// Open receivers, parallel to `Model::receivers`.
    receivers: Vec<Receiver>,
    /// HID++ interface paths at the last reload; a change means plug or unplug.
    paths: Vec<String>,
    unprobeable: HashSet<String>,
    cache: DeviceCache,
    session: Option<PairingSession>,
    reload_at: Option<Instant>,
    refresh_at: HashMap<DeviceKey, Instant>,
    /// Devices whose link came up, read once they have settled.
    linked_at: HashMap<DeviceKey, Instant>,
    /// Serials whose active host slot got this computer's name this run.
    named: HashSet<String>,
    next_plug_check: Instant,
}

impl Worker {
    fn new(api: HidApi, model: Arc<Mutex<Model>>, ctx: egui::Context) -> Self {
        Self {
            api,
            model,
            ctx,
            receivers: Vec::new(),
            paths: Vec::new(),
            unprobeable: HashSet::new(),
            cache: DeviceCache::load(),
            session: None,
            reload_at: None,
            refresh_at: HashMap::new(),
            linked_at: HashMap::new(),
            named: HashSet::new(),
            next_plug_check: Instant::now() + PLUG_CHECK,
        }
    }

    fn update(&self, f: impl FnOnce(&mut Model)) {
        f(&mut lock(&self.model));
        self.ctx.request_repaint();
    }

    fn toast(&self, text: impl Into<String>) {
        let text = text.into();
        self.update(|m| m.toast = Some((text, Instant::now() + TOAST)));
    }

    fn run(mut self, rx: &mpsc::Receiver<Command>) {
        self.reload();
        loop {
            let wait = if self.session.is_some() { 50 } else { 100 };
            match rx.recv_timeout(Duration::from_millis(wait)) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.tick();
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Reload => self.reload(),
            Command::Write(key, w) => self.write(key, w),
            Command::Unpair(key) => self.unpair(key),
            Command::StartPairing(r) => self.start_pairing(r),
            Command::CancelPairing => {
                if let Some(r) = self.pairing_receiver() {
                    let _ = r.cancel_pairing();
                    r.listen_for_events();
                }
                self.session = None;
                self.update(|m| m.pairing = None);
            }
            Command::ClearPairing => self.update(|m| m.pairing = None),
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.session.is_some() {
            self.poll_pairing();
        } else {
            self.poll_events(now);
        }
        if self.reload_at.is_some_and(|t| t <= now) {
            self.reload();
            return;
        }
        for key in take_due(&mut self.linked_at, now) {
            self.came_online(key);
        }
        for key in take_due(&mut self.refresh_at, now) {
            self.read_settings(key, false);
        }
        if now >= self.next_plug_check && self.session.is_none() {
            self.next_plug_check = now + PLUG_CHECK;
            let _ = self.api.refresh_devices();
            if hidpp_paths(&self.api) != self.paths {
                self.reload();
            }
        }
    }

    fn poll_events(&mut self, now: Instant) {
        let mut reload = false;
        let mut links = Vec::new();
        let mut refresh = Vec::new();
        for (i, r) in self.receivers.iter().enumerate() {
            // Non-blocking reads; a burst longer than this waits for the next tick.
            for _ in 0..32 {
                match r.poll_device_event(0) {
                    Ok(Some(DeviceEvent::LinkChange { slot, online })) => {
                        links.push((DeviceKey { receiver: i, slot }, online));
                    }
                    Ok(Some(DeviceEvent::SettingsChanged {
                        slot,
                        feature_index,
                    })) => refresh.push((DeviceKey { receiver: i, slot }, feature_index)),
                    Ok(None) => {}
                    // Unplugged, most likely; the reload finds out.
                    Err(_) => {
                        reload = true;
                        break;
                    }
                }
            }
        }
        for (key, online) in links {
            let known = lock(&self.model).device(key).map(|d| d.online);
            match (known, online) {
                // A slot the model does not have: a device paired elsewhere.
                (None, _) => reload = true,
                (Some(false), true) => {
                    self.linked_at.insert(key, now + DEBOUNCE);
                }
                (Some(_), false) => {
                    self.linked_at.remove(&key);
                    self.went_offline(key);
                }
                (Some(true), true) => {}
            }
        }
        if reload && self.reload_at.is_none() {
            self.reload_at = Some(now + DEBOUNCE);
        }
        let model = lock(&self.model);
        for (key, feature_index) in refresh {
            // A device that sends anything is online.
            if model.device(key).is_some_and(|d| !d.online) {
                self.linked_at.entry(key).or_insert(now + DEBOUNCE);
                continue;
            }
            // Persistent 0x1B04 diversions report every button click under the
            // REPROG_CONTROLS index; those are not settings changes.
            let is_click = model.settings.get(&key).is_some_and(|s| {
                s.reprog_controls_idx != 0 && s.reprog_controls_idx == feature_index
            });
            if !is_click {
                self.refresh_at.insert(key, now + DEBOUNCE);
            }
        }
    }

    /// The receiver lost the device: show it offline with its last battery.
    fn went_offline(&self, key: DeviceKey) {
        self.update(|m| {
            m.settings.remove(&key);
            let had_battery = m.device(key).is_some_and(|d| d.battery.is_some());
            if let Some(d) = device_mut(m, key) {
                d.online = false;
            }
            if had_battery {
                m.cached_battery.insert(key);
            }
        });
    }

    /// The device linked again: read its name, battery and settings.
    fn came_online(&mut self, key: DeviceKey) {
        let Some(mut info) = self
            .receivers
            .get(key.receiver)
            .and_then(|r| r.enumerate_slot(key.slot, Some(true)))
        else {
            return;
        };
        self.apply_cache(&mut info);
        self.update(|m| {
            m.cached_battery.remove(&key);
            if let Some(d) = device_mut(m, key) {
                *d = info;
            }
        });
        self.read_settings(key, true);
    }

    /// Reopen every receiver, enumerate devices and direct Bluetooth devices,
    /// then read settings of online devices.
    fn reload(&mut self) {
        self.reload_at = None;
        self.refresh_at.clear();
        self.linked_at.clear();
        // Close before reopening: macOS allows one handle per receiver.
        self.receivers.clear();
        let _ = self.api.refresh_devices();
        self.paths = hidpp_paths(&self.api);
        let handles = enumerate_receivers(&self.api);
        self.update(|m| {
            m.loading = true;
            m.error = None;
            m.receivers.clear();
            m.settings.clear();
            m.cached_battery.clear();
        });
        for h in handles {
            let r = match Receiver::open_for_events(&self.api, &h) {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("Could not open the {}: {e}", h.name);
                    self.update(|m| m.error = Some(msg));
                    continue;
                }
            };
            let index = self.receivers.len();
            let view = ReceiverView {
                name: r.name,
                kind: r.kind,
                product_id: r.product_id,
                serial: r.serial.clone(),
                max_devices: r.max_devices,
                devices: Vec::new(),
            };
            self.update(|m| m.receivers.push(view));
            let paired = r.get_paired_device_count();
            let links = r.link_states(paired);
            let mut found = 0;
            for slot in 1..=r.max_devices {
                if found >= paired {
                    break;
                }
                if let Some(mut info) = r.enumerate_slot(slot, links.get(&slot).copied()) {
                    found += 1;
                    let cached = self.apply_cache(&mut info);
                    let key = DeviceKey {
                        receiver: index,
                        slot,
                    };
                    self.update(|m| {
                        if let Some(v) = m.receivers.get_mut(index) {
                            v.devices.push(info);
                        }
                        if cached {
                            m.cached_battery.insert(key);
                        }
                    });
                }
            }
            self.receivers.push(r);
        }
        let direct = enumerate_direct_devices(&self.api, &mut self.unprobeable);
        self.update(|m| m.direct = direct);

        let online: Vec<DeviceKey> = lock(&self.model)
            .receivers
            .iter()
            .enumerate()
            .flat_map(|(i, r)| {
                r.devices
                    .iter()
                    .filter(|d| d.online)
                    .map(move |d| DeviceKey {
                        receiver: i,
                        slot: d.slot,
                    })
            })
            .collect();
        for key in online {
            self.read_settings(key, true);
        }
        self.update(|m| m.loading = false);
    }

    /// Remember what an online device reported; fill in the last name and
    /// battery of an offline one, which reports only its short codename.
    /// Returns true when the battery shown comes from the cache.
    fn apply_cache(&mut self, info: &mut DeviceInfo) -> bool {
        if info.online {
            let b = info.battery.as_ref();
            self.cache.remember(
                &info.serial,
                CachedDevice {
                    name: info.name.clone(),
                    level: b.and_then(|b| b.level),
                    status: b.and_then(|b| b.status).map(|s| s as u8),
                    voltage: b.and_then(|b| b.voltage),
                    seen: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs()),
                },
            );
            return false;
        }
        let Some(c) = self.cache.get(&info.serial) else {
            return false;
        };
        if !c.name.is_empty() {
            info.name.clone_from(&c.name);
        }
        if info.battery.is_some() || (c.level.is_none() && c.status.is_none()) {
            return false;
        }
        info.battery = Some(Battery {
            level: c.level,
            status: c.status.map(BatteryStatus::from_byte),
            voltage: c.voltage,
        });
        true
    }

    /// Read all settings of one device into the model. `on_connect` also does
    /// the two writes the SwiftUI app did when it first saw a device: rewrite
    /// the scroll mode, which clears the HID++ target bit that otherwise makes
    /// scroll events vanish while Pulsaar runs, and write this computer's name
    /// to the active host slot, once per device per run.
    fn read_settings(&mut self, key: DeviceKey, on_connect: bool) {
        let Some(r) = self.receivers.get(key.receiver) else {
            return;
        };
        let settings = match r.get_all_settings(key.slot) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[PULSAAR][SETTINGS] slot {} read failed: {e}", key.slot);
                return;
            }
        };
        if on_connect {
            if let Some(s) = &settings.scroll {
                if s.has_invert || s.has_hires {
                    let _ = r.set_scroll_settings(key.slot, s.inverted, s.hires_enabled);
                }
            }
            let host = settings
                .hosts
                .as_ref()
                .and_then(|h| h.iter().find(|h| h.is_active));
            let serial = lock(&self.model).device(key).map(|d| d.serial.clone());
            let name = pulsaar_native::hostname();
            if let (Some(host), Some(serial), false) = (host, serial, name.is_empty()) {
                if self.named.insert(serial) {
                    let _ = r.set_host_name(key.slot, host.slot, &name);
                }
            }
        }
        // The battery comes with the settings read; show it and remember it.
        let device = lock(&self.model).device(key).cloned();
        if let (Some(mut info), Some(battery)) = (device, settings.battery.clone()) {
            info.battery = Some(battery);
            self.apply_cache(&mut info);
            self.update(|m| {
                m.cached_battery.remove(&key);
                if let Some(d) = device_mut(m, key) {
                    d.battery = info.battery;
                }
            });
        }
        self.update(|m| {
            m.settings.insert(key, settings);
        });
    }

    fn write(&mut self, key: DeviceKey, w: Write) {
        let Some(r) = self.receivers.get(key.receiver) else {
            return;
        };
        let slot = key.slot;
        let (result, toast) = match w {
            Write::Dpi(v) => (r.set_dpi(slot, v), "DPI saved"),
            Write::Scroll { inverted, hires } => (
                r.set_scroll_settings(slot, inverted, hires),
                "Scroll settings saved",
            ),
            Write::SmartShift { wheel_mode, torque } => (
                r.set_smart_shift(slot, wheel_mode, torque),
                "Scroll mode saved",
            ),
            Write::Host(h) => (r.set_active_host(slot, h), "Host changed"),
            Write::FnSwap(b) => (r.set_fn_swap(slot, b), "Function keys saved"),
            Write::Platform(p) => (r.set_multiplatform(slot, p), "OS layout saved"),
            Write::Backlight { mode, level } => {
                (r.set_backlight(slot, mode, level), "Backlight saved")
            }
        };
        if let Err(e) = result {
            self.toast(format!("Could not save: {e}"));
            self.read_settings(key, false);
            return;
        }
        // Write-through: a refresh that raced the write must not bring back
        // the old value (docs/gotchas.md, settingsCache write-through).
        self.update(|m| {
            if let Some(s) = m.settings.get_mut(&key) {
                patch(s, w);
            }
            m.toast = Some((toast.to_owned(), Instant::now() + TOAST));
        });
    }

    fn unpair(&mut self, key: DeviceKey) {
        let Some(r) = self.receivers.get(key.receiver) else {
            return;
        };
        match r.unpair_device(key.slot) {
            Ok(()) => {
                self.toast("Device unpaired");
                self.reload();
            }
            Err(e) => self.toast(format!("Could not unpair: {e}")),
        }
    }

    fn pairing_receiver(&self) -> Option<&Receiver> {
        let index = lock(&self.model).pairing.as_ref()?.receiver;
        self.receivers.get(index)
    }

    fn start_pairing(&mut self, index: usize) {
        let result = match self.receivers.get(index) {
            Some(r) => r.start_pairing(PAIRING_TIMEOUT_SECS),
            None => return,
        };
        let stage = match result {
            Ok(session) => {
                self.session = Some(session);
                PairingStage::Waiting
            }
            Err(e) => PairingStage::Failed(format!("Could not start pairing: {e}")),
        };
        self.update(|m| {
            m.pairing = Some(Pairing {
                receiver: index,
                stage,
                device_name: String::new(),
            })
        });
    }

    fn poll_pairing(&mut self) {
        let index = lock(&self.model).pairing.as_ref().map(|p| p.receiver);
        let (Some(r), Some(session)) = (
            index.and_then(|i| self.receivers.get(i)),
            self.session.as_mut(),
        ) else {
            self.session = None;
            return;
        };
        let event = match r.poll_pairing(session, 0) {
            Ok(ev) => ev,
            Err(e) => PairingEvent::Failed(e.to_string()),
        };
        let (stage, name) = match event {
            PairingEvent::Waiting => return,
            PairingEvent::BoltDeviceFound(name) => (PairingStage::Found, Some(name)),
            PairingEvent::PasskeyNumeric(code) => (
                PairingStage::Passkey {
                    code,
                    numeric: true,
                },
                None,
            ),
            PairingEvent::PasskeyButton(code) => (
                PairingStage::Passkey {
                    code,
                    numeric: false,
                },
                None,
            ),
            PairingEvent::Paired(slot) => (PairingStage::Paired { slot }, None),
            PairingEvent::Failed(e) => (PairingStage::Failed(e), None),
        };
        let paired = match stage {
            PairingStage::Paired { slot } => Some(slot),
            _ => None,
        };
        if paired.is_some() || matches!(stage, PairingStage::Failed(_)) {
            r.listen_for_events();
            self.session = None;
        }
        self.update(|m| {
            if let Some(p) = m.pairing.as_mut() {
                p.stage = stage;
                if let Some(n) = name {
                    p.device_name = n;
                }
            }
        });
        if let (Some(slot), Some(receiver)) = (paired, index) {
            // The dialog shows success while the new device is read.
            self.reload();
            let key = DeviceKey { receiver, slot };
            self.update(|m| {
                let name = m.device(key).map(|d| d.name.clone());
                if let (Some(p), Some(n)) = (m.pairing.as_mut(), name) {
                    p.device_name = n;
                }
            });
        }
    }
}

fn device_mut(m: &mut Model, key: DeviceKey) -> Option<&mut DeviceInfo> {
    m.receivers
        .get_mut(key.receiver)?
        .devices
        .iter_mut()
        .find(|d| d.slot == key.slot)
}

/// Remove and return the keys whose time has come.
fn take_due(timers: &mut HashMap<DeviceKey, Instant>, now: Instant) -> Vec<DeviceKey> {
    let due: Vec<DeviceKey> = timers
        .iter()
        .filter(|(_, t)| **t <= now)
        .map(|(k, _)| *k)
        .collect();
    for k in &due {
        timers.remove(k);
    }
    due
}

/// Apply a successful write to the cached settings.
fn patch(s: &mut AllDeviceSettings, w: Write) {
    match w {
        Write::Dpi(v) => {
            if let Some(d) = s.dpi.as_mut() {
                d.current_dpi = v;
            }
        }
        Write::Scroll { inverted, hires } => {
            if let Some(sc) = s.scroll.as_mut() {
                sc.inverted = inverted;
                sc.hires_enabled = hires;
            }
        }
        Write::SmartShift { wheel_mode, torque } => {
            if let Some(ss) = s.smart_shift.as_mut() {
                ss.wheel_mode = wheel_mode;
                ss.torque = torque;
            }
        }
        Write::Host(slot) => {
            for h in s.hosts.iter_mut().flatten() {
                h.is_active = h.slot == slot;
            }
        }
        Write::FnSwap(b) => {
            if let Some(f) = s.fn_settings.as_mut() {
                f.fn_swapped = b;
            }
        }
        Write::Platform(p) => {
            if let Some(mp) = s.multiplatform.as_mut() {
                mp.current = p;
            }
        }
        Write::Backlight { mode, level } => {
            if let Some(b) = s.backlight.as_mut() {
                b.mode = mode;
                b.level = level;
            }
        }
    }
}

/// Every Logitech HID++ interface path, receivers and Bluetooth devices,
/// sorted. Cheap: reads the OS device list without opening anything.
fn hidpp_paths(api: &HidApi) -> Vec<String> {
    let mut paths: Vec<String> = api
        .device_list()
        .filter(|d| d.vendor_id() == 0x046D)
        .filter(|d| d.usage_page() == 0xFF00 || d.usage_page() == 0xFF43)
        .map(|d| d.path().to_string_lossy().into_owned())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}
