//! The Pulsaar binary: opens the window and hands it to the UI crate.

// No console window behind the GUI on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui;
use pulsaar_ui::PulsaarApp;

const APP_ID: &str = "com.github.nikolainyegaard.pulsaar";

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pulsaar")
            .with_app_id(APP_ID)
            .with_icon(egui::IconData {
                rgba: include_bytes!("../../../packaging/pulsaar-64.rgba").to_vec(),
                width: 64,
                height: 64,
            })
            .with_inner_size([900.0, 600.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "Pulsaar",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_fonts(fonts());
            Ok(Box::new(Shell(PulsaarApp::new())))
        }),
    );
    if let Err(e) = result {
        let text = format!("Pulsaar could not open a window: {e}");
        eprintln!("{text}");
        if cfg!(windows) {
            let _ = std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", "Add-Type -AssemblyName System.Windows.Forms; [void][System.Windows.Forms.MessageBox]::Show($env:PULSAAR_TEXT + \"`n`nOn a machine without a GPU, unpack the software OpenGL zip from the release into the folder with pulsaar.exe.\", 'Pulsaar')"])
                .env("PULSAAR_TEXT", &text)
                .status();
        }
        std::process::exit(1);
    }
}

/// One bundled face for text plus egui's icon font; `system_fonts` covers
/// the rest on demand. Same set as OxTree.
fn fonts() -> egui::FontDefinitions {
    use egui::{FontData, FontFamily, FontTweak};
    use std::sync::Arc;
    let mut defs = egui::FontDefinitions::empty();
    defs.font_data.insert(
        "Ubuntu-Light".to_owned(),
        Arc::new(FontData::from_static(epaint_default_fonts::UBUNTU_LIGHT)),
    );
    defs.font_data.insert(
        "egui-icons".to_owned(),
        Arc::new(
            FontData::from_static(epaint_default_fonts::EGUI_ICONS).tweak(FontTweak {
                scale: 0.90,
                ..Default::default()
            }),
        ),
    );
    let stack = vec!["Ubuntu-Light".to_owned(), "egui-icons".to_owned()];
    defs.families
        .insert(FontFamily::Proportional, stack.clone());
    defs.families.insert(FontFamily::Monospace, stack);
    defs
}

struct Shell(PulsaarApp);

impl eframe::App for Shell {
    /// Opaque on purpose: eframe's default is 70% alpha, which ghosts on
    /// Wayland when a panel paints no background (OxTree docs/gotchas.md).
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.extreme_bg_color.to_normalized_gamma_f32()
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.show(root);
    }
}
