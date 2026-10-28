// Debug tool, read-only: for every online device, print the version and
// flags of CHANGE_HOST (0x1814), HOSTS_INFO (0x1815) and DEVICE_RESET
// (0x1802), the HOSTS_INFO capability reply and each host's info reply.
// DEVICE_RESET is never called, only looked up in the feature table.
// Run with: cargo run --bin probe_hosts
#![allow(clippy::indexing_slicing)]

use pulsaar_core::hidpp::hidpp20::{self, discover_features, feature_call};

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    let api = match pulsaar_core::init() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("init failed: {e}");
            std::process::exit(1);
        }
    };
    for h in pulsaar_core::enumerate_receivers(&api) {
        let Ok(r) = pulsaar_core::Receiver::open(&api, &h) else {
            continue;
        };
        let links = r.link_states(r.get_paired_device_count());
        for (slot, _) in links.iter().filter(|(_, up)| **up) {
            let t = r.transport();
            let Ok(f) = discover_features(t, *slot, None) else {
                continue;
            };
            let name = hidpp20::get_device_name(t, *slot, &f)
                .ok()
                .flatten()
                .unwrap_or_default();
            println!("\n{} slot {slot}: {name}", h.name);
            let fs = f.get(&hidpp20::FEAT_FEATURE_SET).copied().unwrap_or(0);
            for id in [0x1814u16, 0x1815, 0x1802] {
                let Some(&idx) = f.get(&id) else {
                    println!("  0x{id:04X}: absent");
                    continue;
                };
                // FEATURE_SET.GetFeature: [id hi, id lo, flags, version]
                let info = feature_call(t, *slot, fs, 1, &[idx]).map(|m| hex(&m.params()[..4]));
                println!("  0x{id:04X} at index {idx}: id/flags/version {info:?}");
            }
            let Some(&hi) = f.get(&0x1815) else { continue };
            let Ok(cap) = feature_call(t, *slot, hi, 0, &[]) else {
                continue;
            };
            println!("  HOSTS_INFO fn0 (capabilities): {}", hex(cap.params()));
            let hosts = cap.params()[2];
            for host in 0..hosts {
                let Ok(m) = feature_call(t, *slot, hi, 1, &[host]) else {
                    continue;
                };
                // GetHostFriendlyName (fn 3): [host, offset] -> [host, offset, name chunk]
                let len = m.params()[4] as usize;
                let mut name = Vec::new();
                while name.len() < len {
                    let Ok(c) = feature_call(t, *slot, hi, 3, &[host, name.len() as u8]) else {
                        break;
                    };
                    let chunk = &c.params()[2..];
                    let take = chunk.len().min(len - name.len());
                    if take == 0 {
                        break;
                    }
                    name.extend_from_slice(&chunk[..take]);
                }
                println!(
                    "  host {host}: {}  name {:?}",
                    hex(&m.params()[..6]),
                    String::from_utf8_lossy(&name)
                );
            }
        }
    }
}
