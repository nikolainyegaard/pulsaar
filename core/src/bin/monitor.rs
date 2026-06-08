/// Bolt receiver raw HID++ event monitor.
///
/// Run with:  cargo run --bin monitor
///
/// Opens the Bolt receiver's vendor HID++ interface (the same one the app uses)
/// and prints every incoming report with a relative timestamp. Use this to
/// confirm what the receiver actually sends when a device turns on, turns off,
/// or wakes from inactivity.
///
/// Expected behaviour on Windows:
///   - Device turned OFF:     0x41 link-change report may appear (offline bit set)
///   - Device turned ON:      0x41 link-change and/or SettingsChanged reports
///   - Device wakes (mouse move, no button): NOTHING on this interface --
///     mouse input goes through the standard HID input collection, not here.

use hidapi::HidApi;
use std::time::Instant;

const LOGITECH_VID: u16 = 0x046D;

fn main() {
    let api = HidApi::new().expect("Failed to init HID API");

    // Find the receiver by vendor usage page (0xFF00) + usage 0x0002.
    // This matches Col02 regardless of the specific Bolt receiver PID.
    let info = api
        .device_list()
        .find(|d| d.vendor_id() == LOGITECH_VID && d.usage_page() == 0xFF00 && d.usage() == 0x0002);

    let info = match info {
        Some(i) => i,
        None => {
            eprintln!("No Logitech receiver found (VID=0x046D, usage_page=0xFF00, usage=0x0002).");
            eprintln!("Make sure the receiver is plugged in.");
            std::process::exit(1);
        }
    };

    println!(
        "Opened: {} (PID=0x{:04X})",
        info.product_string().unwrap_or("Receiver"),
        info.product_id(),
    );
    println!("Monitoring HID++ reports. Press Ctrl+C to stop.\n");
    println!("{:<14}  {:<7}  {}", "Time", "Type", "Payload");
    println!("{}", "-".repeat(70));

    let dev = info.open_device(&api).expect("Failed to open device");
    dev.set_blocking_mode(false).ok();

    let start = Instant::now();
    let mut buf = [0u8; 32];

    loop {
        match dev.read_timeout(&mut buf, 50) {
            Ok(0) => {}
            Ok(n) => {
                let ms = start.elapsed().as_millis();
                let bytes = &buf[..n];
                let decoded = decode(bytes);
                println!("[T+{ms:<8}ms]  {decoded}");
            }
            Err(e) => {
                eprintln!("Read error: {e}");
                break;
            }
        }
    }
}

fn decode(data: &[u8]) -> String {
    let hex: String = data.iter().map(|b| format!("{b:02X} ")).collect();
    let hex = hex.trim_end();

    if data.len() < 4 {
        return format!("(short) {hex}");
    }

    let report_id = data[0];
    let device_idx = data[1];
    let feature = data[2];
    let func_evt = data[3];

    let kind = match report_id {
        0x10 => "VERY-SHORT",
        0x11 => "SHORT     ",
        0x12 => "LONG      ",
        other => return format!("UNKNOWN(0x{other:02X}) {hex}"),
    };

    // Annotate well-known feature codes
    let note: &str = match feature {
        0x41 => " << LINK_CHANGE",
        0x00 => " (root/ping)",
        _ => "",
    };

    // For link-change: decode online/offline from byte 4
    let link_detail = if feature == 0x41 && data.len() > 4 {
        let proto = data[4];
        let online = (proto & 0x40) != 0;
        let encrypted = (proto & 0x20) != 0;
        format!("  [device={device_idx} online={online} encrypted={encrypted}]")
    } else {
        String::new()
    };

    format!(
        "{kind}  dev=0x{device_idx:02X}  feat=0x{feature:02X}  fn/evt=0x{func_evt:02X}{note}{link_detail}  raw=[{hex}]"
    )
}
