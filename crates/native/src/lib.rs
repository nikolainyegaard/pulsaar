//! Platform glue with no UI: where Pulsaar keeps its files.

pub mod cache;

use std::path::PathBuf;

/// The directory for Pulsaar's own files: `~/Library/Application Support/Pulsaar`
/// on macOS (where the SwiftUI app kept its device cache), `%APPDATA%\Pulsaar`
/// on Windows, `$XDG_CONFIG_HOME/pulsaar` elsewhere.
pub fn config_dir() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support/Pulsaar"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Pulsaar"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home().map(|h| h.join(".config")))
            .map(|c| c.join("pulsaar"))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
}

/// This computer's name as the user sees it, written to the active host slot
/// of devices with Change Host. Empty when it cannot be read.
pub fn hostname() -> String {
    let raw = if cfg!(windows) {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    } else if cfg!(target_os = "macos") {
        // The name from System Settings, General, About, as Host.localizedName was.
        std::process::Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    } else {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .or_else(|_| std::fs::read_to_string("/etc/hostname"))
            .unwrap_or_default()
    };
    raw.trim().to_owned()
}

/// Open the system's Bluetooth settings, where Bluetooth devices are paired
/// and removed. On Linux the first panel that starts wins: KDE, GNOME, Blueman.
pub fn open_bluetooth_settings() {
    use std::process::Command;
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[(
            "open",
            &["x-apple.systempreferences:com.apple.BluetoothSettings"],
        )]
    } else if cfg!(windows) {
        &[("explorer", &["ms-settings:bluetooth"])]
    } else {
        &[
            ("systemsettings", &["kcm_bluetooth"]),
            ("gnome-control-center", &["bluetooth"]),
            ("blueman-manager", &[]),
        ]
    };
    for (program, args) in candidates {
        if Command::new(program).args(*args).spawn().is_ok() {
            return;
        }
    }
}
