//! Modal dialogs: pairing a new device and confirming an unpair.

use std::time::Instant;

use eframe::egui::{self, Color32, Id, RichText};

use crate::store::{Command, PairingStage};
use crate::{PulsaarApp, PAIRED_LINGER};

pub fn show(app: &mut PulsaarApp, ctx: &egui::Context) {
    pairing(app, ctx);
    unpair(app, ctx);
}

fn pairing(app: &mut PulsaarApp, ctx: &egui::Context) {
    let m = app.model();
    let Some(p) = m.pairing.clone() else {
        drop(m);
        app.paired_at = None;
        return;
    };
    let receiver = m.receivers.get(p.receiver).map_or("", |r| r.name);
    drop(m);

    if let PairingStage::Paired { .. } = p.stage {
        let since = *app.paired_at.get_or_insert_with(Instant::now);
        let left = PAIRED_LINGER.saturating_sub(since.elapsed());
        if left.is_zero() {
            app.store.send(Command::ClearPairing);
            return;
        }
        ctx.request_repaint_after(left);
    }

    let mut cmd = None;
    let resp = egui::Modal::new(Id::unique("pairing")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.vertical_centered(|ui| {
            ui.heading("Pair New Device");
            ui.weak(receiver);
            ui.separator();
            ui.add_space(12.0);
            match &p.stage {
                PairingStage::Waiting => {
                    ui.spinner();
                    ui.label("Waiting for a device...");
                    ui.weak("Put your device into pairing mode.");
                }
                PairingStage::Found => {
                    ui.spinner();
                    ui.label(format!("Found: {}", p.device_name));
                    ui.weak("Completing pairing...");
                }
                PairingStage::Passkey {
                    code,
                    numeric: true,
                } => {
                    ui.label("Enter this passkey on your keyboard:");
                    passkey_badges(ui, code, p.entered, "Enter");
                }
                PairingStage::Passkey {
                    code,
                    numeric: false,
                } => {
                    ui.label("Press these buttons in order:");
                    passkey_badges(ui, code, p.entered, "L + R");
                    ui.weak("L = left button, R = right button, L + R = both together");
                }
                PairingStage::Paired { slot } => {
                    let name = if p.device_name.is_empty() {
                        format!("Device (slot {slot})")
                    } else {
                        p.device_name.clone()
                    };
                    ui.label(
                        RichText::new(format!("{name} paired"))
                            .size(18.0)
                            .color(Color32::from_rgb(0x2e, 0xa0, 0x43)),
                    );
                }
                PairingStage::Failed(e) => {
                    ui.label(
                        RichText::new("Pairing failed")
                            .size(18.0)
                            .color(ui.visuals().error_fg_color),
                    );
                    ui.weak(if e.is_empty() {
                        "The receiver did not respond."
                    } else {
                        e
                    });
                }
            }
            ui.add_space(16.0);
            match p.stage {
                PairingStage::Paired { .. } => {
                    if ui.button("Done").clicked() {
                        cmd = Some(Command::ClearPairing);
                    }
                }
                PairingStage::Failed(_) => {
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            cmd = Some(Command::ClearPairing);
                        }
                        if ui.button("Try Again").clicked() {
                            cmd = Some(Command::StartPairing(p.receiver));
                        }
                    });
                }
                _ => {
                    if ui.button("Cancel").clicked() {
                        cmd = Some(Command::CancelPairing);
                    }
                }
            }
        });
    });
    if cmd.is_none() && resp.should_close() {
        cmd = Some(match p.stage {
            PairingStage::Paired { .. } | PairingStage::Failed(_) => Command::ClearPairing,
            _ => Command::CancelPairing,
        });
    }
    if let Some(c) = cmd {
        app.store.send(c);
    }
}

/// The passkey as a row of badges, as in the macOS app: L blue, R orange,
/// digits neutral, then a wider badge for the finishing press (`finish`).
/// The first `entered` badges are greyed as the device reports them pressed;
/// `usize::MAX` means entry completed, which greys the finishing badge too.
fn passkey_badges(ui: &mut egui::Ui, code: &str, entered: usize, finish: &str) {
    const SIZE: f32 = 28.0;
    const GAP: f32 = 5.0;
    const FINISH: f32 = 56.0;
    let n = code.chars().count();
    let width = n as f32 * (SIZE + GAP) + FINISH;
    let (rect, _) = ui.allocate_exact_size([width, SIZE].into(), egui::Sense::hover());
    let p = ui.painter();
    let weak = ui.visuals().weak_text_color();
    let text = ui.visuals().strong_text_color();
    let badges = code
        .chars()
        .map(|c| (c.to_string(), SIZE))
        .chain(std::iter::once((finish.to_owned(), FINISH)));
    let mut x = rect.min.x;
    for (i, (label, w)) in badges.enumerate() {
        let badge = egui::Rect::from_min_size([x, rect.min.y].into(), [w, SIZE].into());
        x += w + GAP;
        let done = i < entered;
        let hue = match label.as_str() {
            "L" => Color32::from_rgb(0x3b, 0x82, 0xf6),
            "R" => Color32::from_rgb(0xf5, 0x9e, 0x0b),
            _ => text,
        };
        let (hue, fg) = if done { (weak, weak) } else { (hue, text) };
        p.rect_filled(
            badge,
            5.0,
            hue.gamma_multiply(if done { 0.06 } else { 0.15 }),
        );
        p.rect_stroke(
            badge,
            5.0,
            egui::Stroke::new(1.0, hue.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
        let size = if w > SIZE { 14.0 } else { 18.0 };
        p.text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(size),
            fg,
        );
    }
}

fn unpair(app: &mut PulsaarApp, ctx: &egui::Context) {
    let Some(key) = app.confirm_unpair else {
        return;
    };
    let name = app.model().device(key).map(|d| d.name.clone());
    let Some(name) = name else {
        app.confirm_unpair = None;
        return;
    };
    let resp = egui::Modal::new(Id::unique("unpair")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.heading(format!("Unpair {name}?"));
        ui.label("The device will be removed from this receiver. You can pair it again later.");
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                app.confirm_unpair = None;
            }
            if ui
                .button(RichText::new("Unpair").color(ui.visuals().error_fg_color))
                .clicked()
            {
                app.confirm_unpair = None;
                app.store.send(Command::Unpair(key));
            }
        });
    });
    if resp.should_close() {
        app.confirm_unpair = None;
    }
}
