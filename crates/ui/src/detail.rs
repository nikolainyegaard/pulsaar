//! The detail pane: a device with its settings, a receiver, the Bluetooth
//! group, a Bluetooth device.

use eframe::egui::{self, Color32, RichText};
use pulsaar_core::receiver::AllDeviceSettings;
use pulsaar_core::{Battery, DeviceKind};

use crate::store::{Command, DeviceKey, Write};
use crate::{
    battery_color, connection_label, kind_label, level_text, status_label, PulsaarApp, Selection,
};

pub fn device(app: &mut PulsaarApp, ui: &mut egui::Ui, key: DeviceKey) {
    let m = app.model();
    let (Some(d), Some(r)) = (
        m.device(key).cloned(),
        m.receivers.get(key.receiver).cloned(),
    ) else {
        drop(m);
        app.selection = None;
        return;
    };
    let settings = m.settings.get(&key).cloned();
    let loading = m.loading;
    let battery = d
        .battery
        .as_ref()
        .map(|b| (b, m.cached_battery.contains(&key)));
    drop(m);

    header(ui, &d.name, kind_label(d.kind), Some(d.online), battery);
    ui.separator();
    if !d.online {
        ui.weak(
            "The device is offline. Its settings show when it connects; press a key or move it.",
        );
    } else {
        match &settings {
            Some(s) => settings_form(app, ui, key, s),
            None if loading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Reading settings...");
                });
            }
            None => {
                ui.weak("No configurable settings");
            }
        }
    }
    ui.add_space(12.0);
    egui::CollapsingHeader::new("Device info").show(ui, |ui| {
        info_grid(ui, battery, |ui| {
            row(ui, "Type", kind_label(d.kind));
            row(ui, "Connection", connection_label(r.kind));
            row(
                ui,
                "Product ID",
                &format!("0x{:02X}{:02X}", d.wpid[0], d.wpid[1]),
            );
            row(ui, "Slot", &d.slot.to_string());
            if !d.serial.is_empty() {
                row(ui, "Serial", &d.serial);
            }
        });
    });
    ui.add_space(12.0);
    if ui.button("Unpair device...").clicked() {
        app.confirm_unpair = Some(key);
    }
}

pub fn receiver(app: &mut PulsaarApp, ui: &mut egui::Ui, index: usize) {
    let Some(r) = app.model().receivers.get(index).cloned() else {
        app.selection = None;
        return;
    };
    ui.add_space(8.0);
    ui.heading(r.name);
    ui.weak(format!(
        "{} of {} devices paired",
        r.devices.len(),
        r.max_devices
    ));
    ui.separator();
    egui::Grid::new("receiver")
        .num_columns(2)
        .spacing([24.0, 6.0])
        .show(ui, |ui| {
            row(ui, "Product ID", &format!("0x{:04X}", r.product_id));
            row(
                ui,
                "Serial",
                if r.serial.is_empty() {
                    "Unknown"
                } else {
                    &r.serial
                },
            );
            row(ui, "Max devices", &r.max_devices.to_string());
        });
    ui.add_space(12.0);
    ui.label(RichText::new("Paired devices").strong());
    if r.devices.is_empty() {
        ui.weak("No devices paired");
    }
    for d in &r.devices {
        let key = DeviceKey {
            receiver: index,
            slot: d.slot,
        };
        if device_link(ui, &d.name, d.kind, d.online).clicked() {
            app.selection = Some(Selection::Device(key));
        }
    }
    ui.add_space(12.0);
    let can_pair = r.devices.len() < usize::from(r.max_devices);
    if ui
        .add_enabled(can_pair, egui::Button::new("Pair new device..."))
        .clicked()
    {
        app.store.send(Command::StartPairing(index));
    }
    if !can_pair {
        ui.weak(format!(
            "All {} slots are in use. Unpair a device first.",
            r.max_devices
        ));
    }
}

pub fn bluetooth(app: &mut PulsaarApp, ui: &mut egui::Ui) {
    let direct = app.model().direct.clone();
    ui.add_space(8.0);
    ui.heading(&app.host_name);
    let n = direct.len();
    ui.weak(format!(
        "{n} device{} paired via Bluetooth",
        if n == 1 { "" } else { "s" }
    ));
    ui.separator();
    for (i, d) in direct.iter().enumerate() {
        if device_link(ui, &d.name, d.kind, true).clicked() {
            app.selection = Some(Selection::Direct(i));
        }
    }
    ui.add_space(12.0);
    if ui.button("Open Bluetooth Settings").clicked() {
        pulsaar_native::open_bluetooth_settings();
    }
}

pub fn direct(app: &mut PulsaarApp, ui: &mut egui::Ui, index: usize) {
    let Some(d) = app.model().direct.get(index).cloned() else {
        app.selection = None;
        return;
    };
    header(
        ui,
        &d.name,
        kind_label(d.kind),
        None,
        d.battery.as_ref().map(|b| (b, false)),
    );
    ui.separator();
    ui.weak("Settings for Bluetooth devices are not supported yet.");
    ui.add_space(12.0);
    egui::CollapsingHeader::new("Device info").show(ui, |ui| {
        info_grid(ui, d.battery.as_ref().map(|b| (b, false)), |ui| {
            row(ui, "Type", kind_label(d.kind));
            row(ui, "Connection", "Bluetooth");
            row(ui, "Product ID", &format!("0x{:04X}", d.product_id));
            if !d.serial.is_empty() {
                row(ui, "Serial", &d.serial);
            }
        });
    });
    ui.add_space(12.0);
    if ui.button("Unpair in Bluetooth Settings").clicked() {
        pulsaar_native::open_bluetooth_settings();
    }
}

/// Name, kind, online state and battery. `online` is None for Bluetooth
/// devices, which are listed only while connected.
fn header(
    ui: &mut egui::Ui,
    name: &str,
    kind: &str,
    online: Option<bool>,
    battery: Option<(&Battery, bool)>,
) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.heading(name);
            ui.horizontal(|ui| {
                ui.weak(kind);
                if let Some(online) = online {
                    let (color, text) = if online {
                        (Color32::from_rgb(0x2e, 0xa0, 0x43), "Online")
                    } else {
                        (ui.visuals().weak_text_color(), "Offline")
                    };
                    let (rect, _) = ui.allocate_exact_size([8.0, 8.0].into(), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.0, color);
                    ui.label(text);
                }
            });
        });
        if let Some((b, cached)) = battery {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(level_text(b))
                        .heading()
                        .color(battery_color(ui, b, cached)),
                )
                .on_hover_text(if cached {
                    "Last seen, the device is offline"
                } else {
                    "Battery"
                });
            });
        }
    });
}

fn info_grid(
    ui: &mut egui::Ui,
    battery: Option<(&Battery, bool)>,
    rows: impl FnOnce(&mut egui::Ui),
) {
    egui::Grid::new(ui.next_auto_id())
        .num_columns(2)
        .spacing([24.0, 6.0])
        .show(ui, |ui| {
            if let Some((b, cached)) = battery {
                let label = if cached {
                    "Battery (last seen)"
                } else {
                    "Battery"
                };
                row(ui, label, &level_text(b));
                if let Some(s) = b.status {
                    row(ui, "Status", status_label(s));
                }
                if let Some(v) = b.voltage {
                    row(ui, "Voltage", &format!("{v} mV"));
                }
            }
            rows(ui);
        });
}

fn row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.weak(label);
    ui.add(egui::Label::new(value).truncate());
    ui.end_row();
}

/// A device in the receiver or Bluetooth list; clicking selects it.
fn device_link(ui: &mut egui::Ui, name: &str, kind: DeviceKind, online: bool) -> egui::Response {
    let state = if online { "Online" } else { "Offline" };
    let text = format!("{name}    {}, {state}", kind_label(kind));
    let text = if online {
        RichText::new(text)
    } else {
        RichText::new(text).weak()
    };
    ui.add(egui::Button::selectable(false, text))
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).strong());
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Grid::new(title)
            .num_columns(2)
            .spacing([24.0, 8.0])
            .show(ui, body);
    });
}

fn settings_form(app: &mut PulsaarApp, ui: &mut egui::Ui, key: DeviceKey, s: &AllDeviceSettings) {
    let store = &app.store;

    if let Some(dpi) = s.dpi.as_ref().filter(|d| !d.dpi_list.is_empty()) {
        section(ui, "Sensitivity", |ui| {
            let lo = f64::from(dpi.dpi_list.first().copied().unwrap_or(200));
            let hi = f64::from(dpi.dpi_list.last().copied().unwrap_or(8000));
            let snaps = snap_list(&dpi.dpi_list);
            let mut value = app.dpi_drag.unwrap_or(f64::from(dpi.current_dpi));
            let shown = nearest(&snaps, value).unwrap_or(dpi.current_dpi);
            ui.label("DPI");
            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::Slider::new(&mut value, lo..=hi)
                        .logarithmic(true)
                        .show_value(false),
                );
                ui.label(RichText::new(shown.to_string()).strong());
                if resp.changed() {
                    app.dpi_drag = Some(value);
                }
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                    app.dpi_drag = None;
                    if let Some(v) = nearest(&snaps, value) {
                        store.write(key, Write::Dpi(v));
                    }
                }
            });
            ui.end_row();
        });
    }

    let ss = s.smart_shift.as_ref().filter(|ss| ss.wheel_mode != 0);
    let scroll = s.scroll.as_ref().filter(|sc| sc.has_invert || sc.has_hires);
    if ss.is_some() || scroll.is_some() {
        section(ui, "Scroll wheel", |ui| {
            if let Some(ss) = ss {
                let mut mode = ss.wheel_mode;
                ui.label("Scroll mode");
                egui::ComboBox::from_id_salt("wheel_mode")
                    .selected_text(if mode == 1 { "Freespin" } else { "Ratchet" })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut mode, 1, "Freespin");
                        ui.selectable_value(&mut mode, 2, "Ratchet");
                    });
                ui.end_row();
                if mode != ss.wheel_mode {
                    store.write(
                        key,
                        Write::SmartShift {
                            wheel_mode: mode,
                            torque: ss.torque,
                        },
                    );
                }
                if ss.has_torque && ss.wheel_mode == 2 {
                    ui.label("Ratchet threshold");
                    if let Some(t) =
                        level_slider(ui, &mut app.level_drag, ss.torque, 1.0..=100.0, 5)
                    {
                        store.write(
                            key,
                            Write::SmartShift {
                                wheel_mode: 2,
                                torque: t,
                            },
                        );
                    }
                    ui.end_row();
                }
            }
            if let Some(sc) = scroll {
                if sc.has_invert {
                    let mut v = sc.inverted;
                    ui.label("");
                    if ui.checkbox(&mut v, "Invert scroll direction").changed() {
                        store.write(
                            key,
                            Write::Scroll {
                                inverted: v,
                                hires: sc.hires_enabled,
                            },
                        );
                    }
                    ui.end_row();
                }
                if sc.has_hires {
                    let mut v = sc.hires_enabled;
                    ui.label("");
                    if ui.checkbox(&mut v, "High-resolution scrolling").changed() {
                        store.write(
                            key,
                            Write::Scroll {
                                inverted: sc.inverted,
                                hires: v,
                            },
                        );
                    }
                    ui.end_row();
                }
            }
        });
    }

    let mp = s
        .multiplatform
        .as_ref()
        .filter(|mp| mp.platforms.len() >= 2);
    if s.fn_settings.is_some() || mp.is_some() {
        section(ui, "Keyboard", |ui| {
            if let Some(f) = &s.fn_settings {
                let mut v = f.fn_swapped;
                ui.label("");
                if ui.checkbox(&mut v, "Swap function keys").changed() {
                    store.write(key, Write::FnSwap(v));
                }
                ui.end_row();
            }
            if let Some(mp) = mp {
                let mut cur = mp.current;
                let name = |i: u8| {
                    mp.platforms
                        .iter()
                        .find(|p| p.index == i)
                        .map_or_else(|| format!("Platform {}", i + 1), |p| p.name.clone())
                };
                ui.label("Set OS");
                egui::ComboBox::from_id_salt("platform")
                    .selected_text(name(cur))
                    .show_ui(ui, |ui| {
                        for p in &mp.platforms {
                            ui.selectable_value(&mut cur, p.index, &p.name);
                        }
                    });
                ui.end_row();
                if cur != mp.current {
                    store.write(key, Write::Platform(cur));
                }
            }
        });
    }

    if let Some(bl) = &s.backlight {
        section(ui, "Backlight", |ui| {
            let label = |m: u8| match m {
                1 => "Automatic",
                3 => "Always on",
                _ => "Off",
            };
            let mut mode = bl.mode;
            ui.label("Mode");
            egui::ComboBox::from_id_salt("backlight")
                .selected_text(label(mode))
                .show_ui(ui, |ui| {
                    for m in [0, 1, 3] {
                        ui.selectable_value(&mut mode, m, label(m));
                    }
                });
            ui.end_row();
            if mode != bl.mode {
                store.write(
                    key,
                    Write::Backlight {
                        mode,
                        level: bl.level,
                    },
                );
            }
            if bl.mode == 3 {
                ui.label("Brightness");
                if let Some(level) = level_slider(ui, &mut app.level_drag, bl.level, 0.0..=100.0, 1)
                {
                    store.write(key, Write::Backlight { mode: 3, level });
                }
                ui.end_row();
            }
        });
    }

    if let Some(hosts) = s.hosts.as_ref().filter(|h| !h.is_empty()) {
        section(ui, "Connectivity", |ui| {
            let label = |slot: u8| {
                hosts
                    .iter()
                    .find(|h| h.slot == slot)
                    .filter(|h| !h.name.is_empty())
                    .map_or_else(|| format!("Host {}", slot + 1), |h| h.name.clone())
            };
            let active = hosts.iter().find(|h| h.is_active).map_or(0, |h| h.slot);
            let mut cur = active;
            ui.label("Switch to");
            egui::ComboBox::from_id_salt("host")
                .selected_text(label(cur))
                .show_ui(ui, |ui| {
                    for h in hosts {
                        ui.selectable_value(&mut cur, h.slot, label(h.slot));
                    }
                });
            ui.end_row();
            if cur != active {
                store.write(key, Write::Host(cur));
            }
        });
    }

    let none = s.dpi.as_ref().is_none_or(|d| d.dpi_list.is_empty())
        && ss.is_none()
        && scroll.is_none()
        && s.fn_settings.is_none()
        && mp.is_none()
        && s.backlight.is_none()
        && s.hosts.as_ref().is_none_or(Vec::is_empty);
    if none {
        ui.weak("No configurable settings");
    }
}

/// A 0 to 100 slider that writes once on release, snapped to `step`.
/// Returns the value to write.
fn level_slider(
    ui: &mut egui::Ui,
    drag: &mut Option<f64>,
    current: u8,
    range: std::ops::RangeInclusive<f64>,
    step: u8,
) -> Option<u8> {
    let snap = |v: f64| {
        let step = f64::from(step);
        let v = ((v / step).round() * step).clamp(step.max(*range.start()), *range.end());
        // In range 0..=100 by the clamp.
        v as u8
    };
    let mut value = drag.unwrap_or(f64::from(current));
    let mut out = None;
    ui.horizontal(|ui| {
        let resp = ui.add(egui::Slider::new(&mut value, range.clone()).show_value(false));
        ui.label(format!("{}%", snap(value)));
        if resp.changed() {
            *drag = Some(value);
        }
        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
            *drag = None;
            out = Some(snap(value));
        }
    });
    out
}

/// DPI values the slider stops at: 50 apart up to 1000, then 100 to 2000,
/// 250 to 4000 and 500 to 8000, each moved to the nearest value the sensor
/// supports. Devices with few DPI steps use their own list.
fn snap_list(dpi_list: &[u16]) -> Vec<u16> {
    let (Some(&lo), Some(&hi)) = (dpi_list.first(), dpi_list.last()) else {
        return Vec::new();
    };
    if dpi_list.len() <= 20 {
        return dpi_list.to_vec();
    }
    let candidates = (50..=1000)
        .step_by(50)
        .chain((1100..=2000).step_by(100))
        .chain((2250..=4000).step_by(250))
        .chain((4500..=8000).step_by(500));
    let mut out: Vec<u16> = candidates
        .filter(|c| (lo..=hi).contains(c))
        .filter_map(|c| nearest(dpi_list, f64::from(c)))
        .chain([lo, hi])
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn nearest(list: &[u16], target: f64) -> Option<u16> {
    list.iter().copied().min_by(|a, b| {
        (f64::from(*a) - target)
            .abs()
            .total_cmp(&(f64::from(*b) - target).abs())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_list_uses_the_increment_scheme_on_a_fine_sensor() {
        let list: Vec<u16> = (200..=8000).step_by(50).collect();
        let snaps = snap_list(&list);
        assert_eq!(snaps.first(), Some(&200));
        assert_eq!(snaps.last(), Some(&8000));
        assert!(snaps.contains(&1100) && !snaps.contains(&1150));
        assert!(snaps.contains(&2250) && !snaps.contains(&2300));
        assert!(snaps.contains(&4500) && !snaps.contains(&4750));
    }

    #[test]
    fn snap_list_keeps_a_short_list() {
        let list = vec![400, 800, 1600];
        assert_eq!(snap_list(&list), list);
        assert_eq!(nearest(&list, 1000.0), Some(800));
    }
}
