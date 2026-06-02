// Smoke test: enumerate all receivers and their paired devices.
// Run with: cd core && cargo run --bin list_devices

fn main() {
    let api = match pulsaar_core::init() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("init failed: {e}");
            std::process::exit(1);
        }
    };

    let receivers = pulsaar_core::enumerate_receivers(&api);
    if receivers.is_empty() {
        println!("no receivers found");
        return;
    }

    for handle in &receivers {
        println!(
            "receiver: {} (pid={:#06X}, kind={:?})",
            handle.name, handle.product_id, handle.kind
        );

        let receiver = match pulsaar_core::Receiver::open(&api, handle) {
            Ok(r) => r,
            Err(e) => {
                println!("  could not open: {e}");
                continue;
            }
        };

        println!("  serial={} max_devices={}", receiver.serial, receiver.max_devices);

        let devices = match receiver.enumerate_devices() {
            Ok(d) => d,
            Err(e) => {
                println!("  enumerate_devices failed: {e}");
                continue;
            }
        };

        if devices.is_empty() {
            println!("  no devices paired");
            continue;
        }

        for dev in &devices {
            println!(
                "  slot {}: {} ({}) wpid={:02X}{:02X} serial={}",
                dev.slot, dev.name, dev.kind, dev.wpid[0], dev.wpid[1], dev.serial
            );

            if let Some(bat) = &dev.battery {
                let level = bat.level.map(|l| format!("{l}%")).unwrap_or_else(|| "?".into());
                let status = bat.status.map(|s| format!("{s:?}")).unwrap_or_else(|| "?".into());
                let voltage = bat.voltage.map(|v| format!(", {v}mV")).unwrap_or_default();
                println!("    battery: {level} ({status}){voltage}");
            } else {
                println!("    battery: not available");
            }

            for fw in &dev.firmware {
                println!("    firmware ({:?}): {}", fw.kind, fw.version);
            }
        }
    }
}
