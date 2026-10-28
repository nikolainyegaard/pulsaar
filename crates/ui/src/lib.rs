//! The egui views and the application state.

mod detail;
mod dialogs;
pub mod store;

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, IntoAtoms, Key, KeyboardShortcut, Modifiers, RichText};
use pulsaar_core::{Battery, BatteryStatus, DeviceKind, ReceiverKind};

use store::{Command, DeviceKey, Model, Store};

/// What the detail pane shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Selection {
    Bluetooth,
    Receiver(usize),
    Device(DeviceKey),
    /// Index into `Model::direct`.
    Direct(usize),
}

pub struct PulsaarApp {
    store: Store,
    selection: Option<Selection>,
    /// This computer's name, the title of the Bluetooth group.
    host_name: String,
    /// The DPI under the pointer while the slider is dragged, before it is written.
    dpi_drag: Option<f64>,
    /// The ratchet threshold or backlight level while dragged.
    level_drag: Option<f64>,
    confirm_unpair: Option<DeviceKey>,
    /// When the pairing dialog first showed success; it closes itself after a while.
    paired_at: Option<Instant>,
}

impl PulsaarApp {
    pub fn new(ctx: &egui::Context) -> Self {
        let host_name = pulsaar_native::hostname();
        Self {
            store: Store::start(ctx.clone()),
            selection: None,
            host_name: if host_name.is_empty() {
                "Bluetooth".to_owned()
            } else {
                host_name
            },
            dpi_drag: None,
            level_drag: None,
            confirm_unpair: None,
            paired_at: None,
        }
    }

    pub fn show(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::R)))
        {
            self.store.send(Command::Reload);
        }
        egui::Panel::left("sidebar")
            .resizable(true)
            .min_size(220.0)
            .default_size(280.0)
            .show(root, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.sidebar(ui));
            });
        egui::CentralPanel::default().show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.detail(ui));
        });
        self.toast(&ctx);
        dialogs::show(self, &ctx);
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let m = self.store.model();
        ui.horizontal(|ui| {
            ui.heading("Pulsaar");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if m.loading {
                    ui.spinner();
                } else if ui
                    .small_button("Refresh")
                    .on_hover_text("Scan again (Ctrl+R)")
                    .clicked()
                {
                    self.store.send(Command::Reload);
                }
            });
        });
        ui.separator();
        if let Some(e) = &m.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if !m.loading && m.receivers.is_empty() && m.direct.is_empty() {
            ui.weak("No devices found");
            return;
        }

        // Receivers and the Bluetooth group, sorted by name as in the macOS app.
        let mut groups: Vec<(&str, Option<usize>)> = m
            .receivers
            .iter()
            .enumerate()
            .map(|(i, r)| (r.name, Some(i)))
            .collect();
        if !m.direct.is_empty() {
            groups.push((&self.host_name, None));
        }
        groups.sort_by_key(|(name, _)| name.to_lowercase());

        let mut clicked = None;
        for (name, receiver) in groups {
            match receiver {
                Some(i) => {
                    let sel = Selection::Receiver(i);
                    if group_row(ui, self.selection == Some(sel), name).clicked() {
                        clicked = Some(sel);
                    }
                    let devices = m
                        .receivers
                        .get(i)
                        .map(|r| r.devices.as_slice())
                        .unwrap_or_default();
                    for (n, d) in devices.iter().enumerate() {
                        let sel = Selection::Device(DeviceKey {
                            receiver: i,
                            slot: d.slot,
                        });
                        let last = n + 1 == devices.len();
                        if device_row(
                            ui,
                            self.selection == Some(sel),
                            &d.name,
                            d.battery.as_ref().map(|b| {
                                (
                                    b,
                                    m.cached_battery.contains(&DeviceKey {
                                        receiver: i,
                                        slot: d.slot,
                                    }),
                                )
                            }),
                            d.online,
                            last,
                        )
                        .clicked()
                        {
                            clicked = Some(sel);
                        }
                    }
                }
                None => {
                    if group_row(ui, self.selection == Some(Selection::Bluetooth), name).clicked() {
                        clicked = Some(Selection::Bluetooth);
                    }
                    for (n, d) in m.direct.iter().enumerate() {
                        let sel = Selection::Direct(n);
                        let last = n + 1 == m.direct.len();
                        if device_row(
                            ui,
                            self.selection == Some(sel),
                            &d.name,
                            d.battery.as_ref().map(|b| (b, false)),
                            true,
                            last,
                        )
                        .clicked()
                        {
                            clicked = Some(sel);
                        }
                    }
                }
            }
        }
        drop(m);
        if clicked.is_some() {
            self.selection = clicked;
            self.dpi_drag = None;
            self.level_drag = None;
        }
    }

    fn detail(&mut self, ui: &mut egui::Ui) {
        match self.selection {
            Some(Selection::Device(key)) => detail::device(self, ui, key),
            Some(Selection::Receiver(i)) => detail::receiver(self, ui, i),
            Some(Selection::Bluetooth) => detail::bluetooth(self, ui),
            Some(Selection::Direct(i)) => detail::direct(self, ui, i),
            None => {
                ui.add_space(ui.available_height() / 3.0);
                ui.vertical_centered(|ui| {
                    ui.heading("Select a device");
                    ui.weak("Choose a device from the list to see its details.");
                });
            }
        }
    }

    fn toast(&self, ctx: &egui::Context) {
        let Some((text, until)) = self.store.model().toast.clone() else {
            return;
        };
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        ctx.request_repaint_after(left);
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -24.0])
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .corner_radius(16.0)
                    .inner_margin(egui::Margin::symmetric(16, 8))
                    .show(ui, |ui| ui.label(text));
            });
    }

    fn model(&self) -> std::sync::MutexGuard<'_, Model> {
        self.store.model()
    }
}

/// A receiver or the Bluetooth group in the sidebar.
fn group_row(ui: &mut egui::Ui, selected: bool, name: &str) -> egui::Response {
    ui.add(
        egui::Button::selectable(selected, (RichText::new(name).strong(), egui::Atom::grow()))
            .min_size([ui.available_width(), 0.0].into()),
    )
}

/// A device under its group, with a tree connector, its name and its battery.
/// The battery's flag is true for the last one seen of an offline device.
fn device_row(
    ui: &mut egui::Ui,
    selected: bool,
    name: &str,
    battery: Option<(&Battery, bool)>,
    online: bool,
    last: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(
            [18.0, ui.spacing().interact_size.y].into(),
            egui::Sense::hover(),
        );
        let stroke = egui::Stroke::new(1.0, ui.visuals().weak_text_color());
        let x = rect.center().x;
        let bottom = if last {
            rect.center().y
        } else {
            rect.bottom() + ui.spacing().item_spacing.y
        };
        ui.painter()
            .line_segment([[x, rect.top()].into(), [x, bottom].into()], stroke);
        ui.painter()
            .line_segment([rect.center(), rect.right_center()], stroke);

        let text = if online {
            RichText::new(name)
        } else {
            RichText::new(name).weak()
        };
        // Name on the left; the grow atom pushes the battery to the right edge.
        let glyph = egui::IdSalt::new("battery");
        let mut atoms: egui::Atoms<'_> = (text, egui::Atom::grow()).into_atoms();
        let color = battery.map(|(b, cached)| battery_color(ui, b, cached));
        if let (Some((b, _)), Some(color)) = (battery, color) {
            atoms.push_right(egui::Atom::custom(glyph, [20.0, 10.0]));
            atoms.push_right(RichText::new(level_text(b)).size(11.0).color(color));
        }
        let resp = egui::Button::selectable(selected, atoms)
            .min_size([ui.available_width(), 0.0].into())
            .atom_ui(ui);
        if let (Some(rect), Some((b, _)), Some(color)) = (resp.rect(glyph), battery, color) {
            paint_battery(ui.painter(), rect, b.level, color);
        }
        resp.response
    })
    .inner
}

/// A small battery outline filled to `level`.
fn paint_battery(p: &egui::Painter, rect: egui::Rect, level: Option<u8>, color: Color32) {
    let body = egui::Rect::from_min_max(rect.min, [rect.max.x - 2.0, rect.max.y].into());
    p.rect_stroke(
        body,
        2.0,
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
    p.rect_filled(
        egui::Rect::from_center_size(
            [rect.max.x - 1.0, rect.center().y].into(),
            [2.0, 4.0].into(),
        ),
        0.0,
        color,
    );
    if let Some(level) = level {
        let mut fill = body.shrink(2.0);
        fill.set_width(fill.width() * f32::from(level.min(100)) / 100.0);
        p.rect_filled(fill, 1.0, color);
    }
}

/// Green while charging, orange at 25% and red at 10%; grey when `cached`.
fn battery_color(ui: &egui::Ui, b: &Battery, cached: bool) -> Color32 {
    if cached {
        return ui.visuals().weak_text_color();
    }
    if b.status.is_some_and(is_charging) {
        return Color32::from_rgb(0x2e, 0xa0, 0x43);
    }
    match b.level {
        Some(l) if l <= 10 => ui.visuals().error_fg_color,
        Some(l) if l <= 25 => ui.visuals().warn_fg_color,
        _ => ui.visuals().text_color(),
    }
}

fn is_charging(s: BatteryStatus) -> bool {
    matches!(
        s,
        BatteryStatus::Recharging
            | BatteryStatus::AlmostFull
            | BatteryStatus::Full
            | BatteryStatus::SlowRecharge
    )
}

fn level_text(b: &Battery) -> String {
    b.level.map_or_else(|| "?".to_owned(), |l| format!("{l}%"))
}

fn status_label(s: BatteryStatus) -> &'static str {
    match s {
        BatteryStatus::Discharging => "Not charging",
        BatteryStatus::Recharging => "Charging",
        BatteryStatus::AlmostFull => "Charging (almost full)",
        BatteryStatus::Full => "Fully charged",
        BatteryStatus::SlowRecharge => "Charging slowly",
        BatteryStatus::InvalidBattery => "Battery fault",
        BatteryStatus::ThermalError => "Too hot to charge",
    }
}

fn kind_label(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Unknown => "Unknown",
        DeviceKind::Keyboard => "Keyboard",
        DeviceKind::Mouse => "Mouse",
        DeviceKind::Numpad => "Numpad",
        DeviceKind::Presenter => "Presenter",
        DeviceKind::Remote => "Remote",
        DeviceKind::Trackball => "Trackball",
        DeviceKind::Touchpad => "Touchpad",
        DeviceKind::Tablet => "Tablet",
        DeviceKind::Gamepad => "Gamepad",
        DeviceKind::Joystick => "Joystick",
        DeviceKind::Headset => "Headset",
        DeviceKind::RemoteControl => "Remote Control",
        DeviceKind::Receiver => "Receiver",
    }
}

fn connection_label(k: ReceiverKind) -> &'static str {
    match k {
        ReceiverKind::Bolt => "Bolt (Encrypted)",
        ReceiverKind::Unifying => "Unifying",
        ReceiverKind::LightSpeed => "LightSpeed",
        ReceiverKind::Nano => "Nano",
    }
}

/// How long the pairing dialog shows success before it closes.
const PAIRED_LINGER: Duration = Duration::from_secs(2);
