// Debug tool: print every HID++ and DJ notification from all receivers for a
// minute, after asking each receiver for the link state of its devices.
// Wake, sleep or switch a device meanwhile to see what arrives.
// Run with: cargo run --bin listen [SECONDS]

use std::time::{Duration, Instant};

fn main() {
    let api = match pulsaar_core::init() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("init failed: {e}");
            std::process::exit(1);
        }
    };
    let receivers: Vec<_> = pulsaar_core::enumerate_receivers(&api)
        .iter()
        .filter_map(|h| pulsaar_core::Receiver::open_for_events(&api, h).ok())
        .collect();
    let secs = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(60);
    println!("listening on {} receiver(s) for {secs} s", receivers.len());
    for r in &receivers {
        let _ = r.trigger_link_notifications();
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        for (i, r) in receivers.iter().enumerate() {
            while let Ok(Some(msg)) = r.transport().read_one(0) {
                // DJ types below 0x10 are forwarded key and mouse input.
                if msg.is_dj() && msg.sub_id() < 0x10 {
                    continue;
                }
                let hex: Vec<String> = msg.as_bytes().iter().map(|b| format!("{b:02X}")).collect();
                println!(
                    "{:6.1}s receiver {i}: {}",
                    start.elapsed().as_secs_f32(),
                    hex.join(" ")
                );
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
