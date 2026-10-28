//! Receiver and device-kind icons. The Bolt, Unifying and Bluetooth marks
//! are the macOS app's SVGs, rasterized once to 64 px alpha masks
//! (`assets/*.rgba`, white with alpha, tinted when drawn). Device kinds are
//! drawn as small outlines, standing in for the SF Symbols the macOS app used.

use eframe::egui::{self, Color32, Rect, Stroke, StrokeKind, TextureHandle};
use pulsaar_core::{DeviceKind, ReceiverKind};

pub struct Icons {
    bolt: TextureHandle,
    unifying: TextureHandle,
    bluetooth: TextureHandle,
}

impl Icons {
    pub fn load(ctx: &egui::Context) -> Self {
        let load = |name: &str, rgba: &[u8]| {
            let image = egui::ColorImage::from_rgba_unmultiplied([64, 64], rgba);
            ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
        };
        Self {
            bolt: load("bolt", include_bytes!("../assets/bolt.rgba")),
            unifying: load("unifying", include_bytes!("../assets/unifying.rgba")),
            bluetooth: load("bluetooth", include_bytes!("../assets/bluetooth.rgba")),
        }
    }

    /// The receiver's mark, or None for Nano and LightSpeed, which have none.
    pub fn receiver(&self, kind: ReceiverKind) -> Option<&TextureHandle> {
        match kind {
            ReceiverKind::Bolt => Some(&self.bolt),
            ReceiverKind::Unifying => Some(&self.unifying),
            ReceiverKind::Nano | ReceiverKind::LightSpeed => None,
        }
    }

    pub fn bluetooth(&self) -> &TextureHandle {
        &self.bluetooth
    }
}

/// A texture as an image of `size` points, tinted with the text colour.
pub fn image(ui: &egui::Ui, tex: &TextureHandle, size: f32) -> egui::Image<'static> {
    egui::Image::new((tex.id(), egui::Vec2::splat(size))).tint(ui.visuals().text_color())
}

/// An outline of the device kind filling `rect`: a mouse, a keyboard, or a
/// rounded square for everything else.
pub fn paint_device(p: &egui::Painter, rect: Rect, kind: DeviceKind, color: Color32) {
    let s = rect.width().min(rect.height());
    let stroke = Stroke::new((s / 14.0).max(1.0), color);
    let c = rect.center();
    match kind {
        DeviceKind::Mouse | DeviceKind::Trackball => {
            let body = Rect::from_center_size(c, egui::vec2(s * 0.55, s * 0.85));
            p.rect_stroke(body, s * 0.27, stroke, StrokeKind::Inside);
            p.line_segment([body.center_top(), egui::pos2(c.x, c.y - s * 0.12)], stroke);
        }
        DeviceKind::Keyboard | DeviceKind::Numpad => {
            let body = Rect::from_center_size(c, egui::vec2(s * 0.95, s * 0.6));
            p.rect_stroke(body, s * 0.08, stroke, StrokeKind::Inside);
            let dot = s * 0.05;
            for row in 0..2 {
                for col in 0..5 {
                    let x = body.left() + body.width() * (col as f32 + 1.0) / 6.0;
                    let y = body.top() + body.height() * (row as f32 + 1.0) / 4.0;
                    p.circle_filled(egui::pos2(x, y), dot, color);
                }
            }
            let y = body.top() + body.height() * 0.75;
            p.line_segment(
                [egui::pos2(c.x - s * 0.22, y), egui::pos2(c.x + s * 0.22, y)],
                stroke,
            );
        }
        _ => {
            let body = Rect::from_center_size(c, egui::Vec2::splat(s * 0.7));
            p.rect_stroke(body, s * 0.15, stroke, StrokeKind::Inside);
        }
    }
}
