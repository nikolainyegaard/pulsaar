//! The egui views and the application state.

use eframe::egui;
use pulsaar_core::ReceiverHandle;

pub struct PulsaarApp {
    /// The receivers found at start, or why hidapi could not start.
    receivers: Result<Vec<ReceiverHandle>, String>,
}

impl PulsaarApp {
    /// Lists the receivers. Only reads the HID device list, no HID++ traffic,
    /// so it is fast enough for the UI thread.
    pub fn new() -> Self {
        let receivers = pulsaar_core::init()
            .map(|api| pulsaar_core::enumerate_receivers(&api))
            .map_err(|e| e.to_string());
        Self { receivers }
    }

    pub fn show(&mut self, root: &mut egui::Ui) {
        egui::CentralPanel::default().show(root, |ui| match &self.receivers {
            Err(e) => {
                ui.colored_label(ui.visuals().error_fg_color, format!("HID unavailable: {e}"));
            }
            Ok(list) if list.is_empty() => {
                ui.label("No Logitech receiver found.");
            }
            Ok(list) => {
                for r in list {
                    ui.label(format!("{} ({:04X})", r.name, r.product_id));
                }
            }
        });
    }
}

impl Default for PulsaarApp {
    fn default() -> Self {
        Self::new()
    }
}
