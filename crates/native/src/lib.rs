//! Platform glue with no UI: where Pulsaar keeps its files.

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
