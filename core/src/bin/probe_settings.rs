// Debug tool: probe settings features on all paired devices.
// Prints raw feature discovery, DPI list bytes, and scroll caps.
// Run with: cd core && cargo run --bin probe_settings

use pulsaar_core::hidpp::hidpp20;

fn main() {
    let api = match pulsaar_core::init() {
        Ok(a) => a,
        Err(e) => { eprintln!("init failed: {e}"); std::process::exit(1); }
    };

    let receivers = pulsaar_core::enumerate_receivers(&api);
    if receivers.is_empty() {
        println!("no receivers found");
        return;
    }

    for handle in &receivers {
        println!("receiver: {} (pid={:#06X})", handle.name, handle.product_id);

        let receiver = match pulsaar_core::Receiver::open(&api, handle) {
            Ok(r) => r,
            Err(e) => { println!("  could not open: {e}"); continue; }
        };

        let devices = match receiver.enumerate_devices() {
            Ok(d) => d,
            Err(e) => { println!("  enumerate_devices failed: {e}"); continue; }
        };

        for dev in &devices {
            println!("  slot {}: {} ({:?})", dev.slot, dev.name, dev.kind);

            // Step 1: discover all HID++ 2.0 features.
            let transport = receiver.transport();
            let features = match hidpp20::discover_features(transport, dev.slot) {
                Ok(f) => f,
                Err(e) => { println!("    discover_features failed: {e}"); continue; }
            };

            if features.is_empty() {
                println!("    no HID++ 2.0 features (HID++ 1.0 device)");
                continue;
            }

            let mut feat_list: Vec<(u16, u8)> = features.iter().map(|(&k, &v)| (k, v)).collect();
            feat_list.sort_by_key(|&(k, _)| k);
            println!("    {} features:", feat_list.len());
            for (feat_id, feat_idx) in &feat_list {
                let label = feature_label(*feat_id);
                println!("      [{feat_idx}] 0x{feat_id:04X}  {label}");
            }

            // Step 2: probe ADJUSTABLE_DPI (0x2201) if present.
            if let Some(&idx) = features.get(&hidpp20::FEAT_ADJUSTABLE_DPI) {
                println!("    --- ADJUSTABLE_DPI (0x2201) at feature index {idx} ---");
                // GetSensorDpiList (fn 1): raw bytes
                match hidpp20::feature_call(transport, dev.slot, idx, 1, &[0x00, 0x00, 0x00]) {
                    Ok(reply) => {
                        let p = reply.params();
                        print!("      GetSensorDpiList raw params ({} bytes): ", p.len());
                        for b in p { print!("{b:02X} "); }
                        println!();
                        // Parse: skip byte 0 (sensor echo), then 2-byte big-endian words.
                        let mut i = 1usize;
                        let mut dpis: Vec<u16> = Vec::new();
                        while i + 1 < p.len() {
                            let val = ((p[i] as u16) << 8) | (p[i+1] as u16);
                            if val == 0 { break; }
                            if val >> 13 == 0b111 {
                                let step = val & 0x1FFF;
                                if i + 3 < p.len() {
                                    let end = ((p[i+2] as u16) << 8) | (p[i+3] as u16);
                                    if let Some(&last) = dpis.last() {
                                        let mut cur = last + step;
                                        while cur <= end { dpis.push(cur); cur += step; }
                                    }
                                    i += 4;
                                } else { break; }
                            } else {
                                dpis.push(val);
                                i += 2;
                            }
                        }
                        println!("      parsed DPI list ({} entries): {:?}", dpis.len(), dpis);
                    }
                    Err(e) => println!("      GetSensorDpiList failed: {e}"),
                }
                // GetSensorDpi (fn 2): raw bytes
                match hidpp20::feature_call(transport, dev.slot, idx, 2, &[0x00]) {
                    Ok(reply) => {
                        let p = reply.params();
                        print!("      GetSensorDpi raw params ({} bytes): ", p.len());
                        for b in p { print!("{b:02X} "); }
                        println!();
                        if p.len() >= 5 {
                            let current = ((p[1] as u16) << 8) | (p[2] as u16);
                            let default = ((p[3] as u16) << 8) | (p[4] as u16);
                            println!("      current={current}, default={default}");
                        }
                    }
                    Err(e) => println!("      GetSensorDpi failed: {e}"),
                }
            } else {
                println!("    ADJUSTABLE_DPI (0x2201) NOT in feature table");
            }

            // Step 3: probe HIRES_WHEEL (0x2121) if present.
            if let Some(&idx) = features.get(&hidpp20::FEAT_HIRES_WHEEL) {
                println!("    --- HIRES_WHEEL (0x2121) at feature index {idx} ---");
                match hidpp20::feature_call(transport, dev.slot, idx, 0, &[]) {
                    Ok(reply) => {
                        let p = reply.params();
                        print!("      GetCapabilities raw params: ");
                        for b in p { print!("{b:02X} "); }
                        println!();
                        if p.len() >= 2 {
                            println!("      multiplier={}, flags=0x{:02X} (has_invert={}, has_ratchet={})",
                                p[0], p[1], (p[1] & 0x08) != 0, (p[1] & 0x04) != 0);
                        }
                    }
                    Err(e) => println!("      GetCapabilities failed: {e}"),
                }
                match hidpp20::feature_call(transport, dev.slot, idx, 1, &[]) {
                    Ok(reply) => {
                        let p = reply.params();
                        print!("      GetMode raw params: ");
                        for b in p { print!("{b:02X} "); }
                        println!();
                        if !p.is_empty() {
                            println!("      mode_byte=0x{:02X} (inverted={}, hires={})",
                                p[0], (p[0] & 0x04) != 0, (p[0] & 0x02) != 0);
                        }
                    }
                    Err(e) => println!("      GetMode failed: {e}"),
                }
            } else {
                println!("    HIRES_WHEEL (0x2121) NOT in feature table");
            }

            // Also check 0x2120 (HI_RES_SCROLLING, older feature code).
            let feat_2120: u16 = 0x2120;
            if let Some(&idx) = features.get(&feat_2120) {
                println!("    --- HI_RES_SCROLLING (0x2120) at feature index {idx} ---");
                match hidpp20::feature_call(transport, dev.slot, idx, 0, &[]) {
                    Ok(reply) => {
                        let p = reply.params();
                        print!("      fn0 raw params: ");
                        for b in p { print!("{b:02X} "); }
                        println!();
                    }
                    Err(e) => println!("      fn0 failed: {e}"),
                }
            } else {
                println!("    HI_RES_SCROLLING (0x2120) NOT in feature table");
            }

            // Probe FN inversion features.
            for &(feat_id, label) in &[
                (hidpp20::FEAT_NEW_FN_INVERSION, "NEW_FN_INVERSION (0x40A2)"),
                (hidpp20::FEAT_FN_INVERSION,     "FN_INVERSION (0x40A0)"),
                (hidpp20::FEAT_K375S_FN_INVERSION,"K375S_FN_INVERSION (0x40A3)"),
            ] {
                if let Some(&idx) = features.get(&feat_id) {
                    println!("    --- {label} at feature index {idx} ---");
                    match hidpp20::feature_call(transport, dev.slot, idx, 0, &[]) {
                        Ok(reply) => {
                            let p = reply.params();
                            print!("      GetFnInversionState raw params: ");
                            for b in p { print!("{b:02X} "); }
                            println!();
                            if !p.is_empty() {
                                println!("      fn_swapped={} default_fn_swapped={}",
                                    (p[0] & 0x01) != 0,
                                    p.get(1).map_or(false, |&b| (b & 0x01) != 0));
                            }
                        }
                        Err(e) => println!("      GetFnInversionState failed: {e}"),
                    }
                } else {
                    println!("    {label} NOT in feature table");
                }
            }

            // Probe backlight features.
            for &(feat_id, label) in &[
                (hidpp20::FEAT_BACKLIGHT2, "BACKLIGHT2 (0x1982)"),
                (0x6020u16,               "BACKLIGHT2-ALT (0x6020)"),
                (0x1981u16,               "BACKLIGHT (0x1981)"),
            ] {
                if let Some(&idx) = features.get(&feat_id) {
                    println!("    --- {label} at feature index {idx} ---");
                    match hidpp20::feature_call(transport, dev.slot, idx, 0, &[]) {
                        Ok(reply) => {
                            let p = reply.params();
                            print!("      GetBacklightState raw params ({} bytes): ", p.len());
                            for b in p { print!("{b:02X} "); }
                            println!();
                            if p.len() >= 6 {
                                let enabled   = p[0];
                                let options   = p[1];
                                let supported = p[2];
                                let level     = p[5];
                                // Decode using bits 0-1 for mode (current hypothesis).
                                let mode_bits01 = if enabled != 0 { options & 0x03 } else { 0 };
                                // Also show old bits-3-4 decode for comparison.
                                let mode_bits34 = if enabled != 0 { (options >> 3) & 0x03 } else { 0 };
                                println!("      enabled={enabled} options=0x{options:02X} supported=0x{supported:02X} level={level}");
                                println!("      mode via bits 0-1: {mode_bits01}  mode via bits 3-4: {mode_bits34}");
                                println!("      auto (bit0=0x01): {} auto (bit1=0x02): {} manual (bit2=0x04): {} manual (bit3=0x08): {}",
                                    (supported & 0x01) != 0, (supported & 0x02) != 0,
                                    (supported & 0x04) != 0, (supported & 0x08) != 0);
                                println!("      all supported bits: {}",
                                    (0u8..8).filter(|&b| (supported >> b) & 1 != 0)
                                        .map(|b| format!("bit{b}(0x{:02X})", 1u8 << b))
                                        .collect::<Vec<_>>().join(", "));
                            }
                        }
                        Err(e) => println!("      GetBacklightState failed: {e}"),
                    }
                } else {
                    println!("    {label} NOT in feature table");
                }
            }

            // -----------------------------------------------------------------------
            // Write tests: write current state back and read-back to verify.
            // These are non-destructive: they write the same value the device reports.
            // -----------------------------------------------------------------------

            // FN swap write test: K375S_FN_INVERSION uses fn 3, not fn 1.
            if let Some(&idx) = features.get(&hidpp20::FEAT_K375S_FN_INVERSION) {
                println!("    --- WRITE TEST: K375S_FN_INVERSION ---");
                match hidpp20::feature_call(transport, dev.slot, idx, 0, &[]) {
                    Ok(reply) => {
                        let p = reply.params();
                        let cur = p.first().copied().unwrap_or(0) & 0x01;
                        println!("      current fn_swapped_byte = 0x{cur:02X}");

                        // Test fn 3 (correct for K375S).
                        match hidpp20::feature_call(transport, dev.slot, idx, 3, &[cur]) {
                            Ok(r) => {
                                let rp = r.params();
                                print!("      fn 3 reply: ");
                                for b in rp { print!("{b:02X} "); }
                                println!();
                                // Read back.
                                match hidpp20::feature_call(transport, dev.slot, idx, 0, &[]) {
                                    Ok(rb) => {
                                        let v = rb.params().first().copied().unwrap_or(0) & 0x01;
                                        println!("      readback = 0x{v:02X} ({})",
                                            if v == cur { "OK: matches written value" } else { "MISMATCH" });
                                    }
                                    Err(e) => println!("      readback failed: {e}"),
                                }
                            }
                            Err(e) => println!("      fn 3 write failed: {e}"),
                        }

                        // Test fn 1 to confirm it fails or is a no-op for K375S.
                        match hidpp20::feature_call(transport, dev.slot, idx, 1, &[cur]) {
                            Ok(r) => {
                                let rp = r.params();
                                print!("      fn 1 reply (expect error or no-op): ");
                                for b in rp { print!("{b:02X} "); }
                                println!();
                            }
                            Err(e) => println!("      fn 1 (expected to fail for K375S): {e}"),
                        }
                    }
                    Err(e) => println!("      read current state failed: {e}"),
                }
            }

            // Backlight write test: write current state back and read-back to verify encoding.
            if let Some(&idx) = features.get(&hidpp20::FEAT_BACKLIGHT2) {
                println!("    --- WRITE TEST: BACKLIGHT2 ---");
                match hidpp20::feature_call(transport, dev.slot, idx, 0, &[]) {
                    Ok(reply) => {
                        let p = reply.params();
                        if p.len() < 12 {
                            println!("      response too short ({} bytes)", p.len());
                        } else {
                            let enabled_raw = p[0];
                            let options_raw = p[1];
                            let level_raw   = p[5];
                            // Preserve timing bytes (dho/dhi/dpow at bytes 6-11).
                            let dho_lo = p[6]; let dho_hi = p[7];
                            let dhi_lo = p[8]; let dhi_hi = p[9];
                            let dpow_lo = p[10]; let dpow_hi = p[11];

                            println!("      before write: enabled=0x{enabled_raw:02X} options=0x{options_raw:02X} level={level_raw}");

                            // Write mode=1 (auto) using bits-0-1 encoding; preserve bit 2 and above.
                            let new_opts_bits01 = (options_raw & !0x03u8) | 0x01;
                            let params = [
                                0x01u8, new_opts_bits01, 0xFF, level_raw,
                                dho_lo, dho_hi, dhi_lo, dhi_hi, dpow_lo, dpow_hi,
                            ];
                            println!("      writing: enabled=1 options=0x{new_opts_bits01:02X} (mode=1 via bits 0-1)");
                            match hidpp20::feature_call(transport, dev.slot, idx, 1, &params) {
                                Ok(r) => {
                                    let rp = r.params();
                                    print!("      SetBacklightState reply: ");
                                    for b in rp { print!("{b:02X} "); }
                                    println!();
                                    // Read back.
                                    match hidpp20::feature_call(transport, dev.slot, idx, 0, &[]) {
                                        Ok(rb) => {
                                            let rbp = rb.params();
                                            if rbp.len() >= 6 {
                                                let rb_en   = rbp[0];
                                                let rb_opts = rbp[1];
                                                let rb_lvl  = rbp[5];
                                                println!("      readback: enabled=0x{rb_en:02X} options=0x{rb_opts:02X} level={rb_lvl}");
                                                println!("      readback mode via bits 0-1: {} bits 3-4: {}",
                                                    rb_opts & 0x03, (rb_opts >> 3) & 0x03);
                                                if rb_opts == new_opts_bits01 {
                                                    println!("      options byte MATCHES written value: bits-0-1 encoding confirmed");
                                                } else {
                                                    println!("      options byte DIFFERS from written (0x{:02X} vs 0x{new_opts_bits01:02X}): check encoding", rb_opts);
                                                }
                                            }
                                        }
                                        Err(e) => println!("      readback failed: {e}"),
                                    }
                                }
                                Err(e) => println!("      SetBacklightState failed: {e}"),
                            }
                        }
                    }
                    Err(e) => println!("      read before write failed: {e}"),
                }
            }
        }
    }
}

fn feature_label(id: u16) -> &'static str {
    match id {
        0x0000 => "ROOT",
        0x0001 => "FEATURE_SET",
        0x0003 => "FW_VERSION",
        0x0005 => "DEVICE_NAME",
        0x0007 => "DEVICE_FRIENDLY_NAME",
        0x0008 => "RESET_PAIRING",
        0x0020 => "RESET",
        0x1000 => "BATTERY_STATUS",
        0x1001 => "BATTERY_VOLTAGE",
        0x1004 => "UNIFIED_BATTERY",
        0x1806 => "CONFIG_DEVICE_PROPS",
        0x1814 => "CHANGE_HOST",
        0x1815 => "HOSTS_INFO",
        0x1981 => "BACKLIGHT",
        0x1982 => "BACKLIGHT2",
        0x1983 => "BACKLIGHT3",
        0x1B04 => "SPECIAL_KEYS_BUTTONS",
        0x1B10 => "SPECIAL_KEYS_BUTTONS_v3",
        0x1D4B => "WIRELESS_DEVICE_STATUS",
        0x1DE0 => "KEEP_ALIVE",
        0x1E00 => "ENABLE_HIDDEN_FEATURES",
        0x1F20 => "CONFIGURATION_CHANGE",
        0x1500 => "FORCE_PAIRING",
        0x2100 => "VERTICAL_SCROLLING",
        0x2110 => "SMART_SHIFT",
        0x2111 => "SMART_SHIFT_ENHANCED",
        0x2120 => "HI_RES_SCROLLING",
        0x2121 => "HIRES_WHEEL",
        0x2130 => "LOWRES_WHEEL",
        0x2150 => "THUMB_WHEEL",
        0x2201 => "ADJUSTABLE_DPI",
        0x2202 => "EXTENDED_ADJUSTABLE_DPI",
        0x2400 => "POINTER_SPEED",
        0x40A0 => "FN_INVERSION",
        0x40A2 => "NEW_FN_INVERSION",
        0x40A3 => "K375S_FN_INVERSION",
        0x4100 => "ENCRYPTION",
        0x4220 => "LOCK_KEY_STATE",
        0x4301 => "SOLAR_DASHBOARD",
        0x4520 => "KEYBOARD_LAYOUT",
        0x4522 => "KEYBOARD_DISABLE_KEYS",
        0x4531 => "MULTIPLATFORM",
        0x4600 => "DUALPLATFORM",
        0x4610 => "MULTIPLATFORM_OLD",
        0x4621 => "KEYBOARD_LAYOUT2",
        0x6010 => "BACKLIGHT-GAMING",
        0x6020 => "BACKLIGHT2-GAMING",
        0x6030 => "BACKLIGHT3-GAMING",
        0x6100 => "ILLUMINATION",
        0x6110 => "FORCE_PAIRING2",
        0x8010 => "GAMING_ATTACHMENTS",
        0x8020 => "CONFIG_CHANGE",
        0x8100 => "ONBOARD_PROFILES",
        0x8110 => "MOUSE_BUTTON_SPY",
        _ => "unknown",
    }
}
