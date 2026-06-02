// Smoke test for the FFI layer. Exercises the full call sequence from C-like Rust code,
// simulating what Swift/C#/Python would do.

use pulsaar_core::ffi::{
    PulsaarStatus, CReceiverInfo, COpenedReceiverInfo, CDeviceInfo,
    pulsaar_init, pulsaar_destroy,
    pulsaar_get_receiver_count, pulsaar_get_receiver_info,
    pulsaar_open_receiver, pulsaar_close_receiver,
    pulsaar_get_opened_receiver_info,
    pulsaar_enumerate_devices, pulsaar_get_device_count, pulsaar_get_device_info,
};
fn buf_to_str(buf: &[u8]) -> &str {
    // Find the null terminator and return everything before it.
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    std::str::from_utf8(&buf[..end]).unwrap_or("<invalid utf8>")
}

fn main() {
    // Step 1: init
    let ctx = pulsaar_init();
    if ctx.is_null() {
        eprintln!("pulsaar_init returned null");
        std::process::exit(1);
    }
    println!("init ok");

    // Step 2: enumerate receivers
    let count = unsafe { pulsaar_get_receiver_count(ctx) };
    println!("receivers: {count}");

    for i in 0..count {
        // Step 3: receiver info (pre-open)
        let mut info = unsafe { std::mem::zeroed::<CReceiverInfo>() };
        let s = unsafe { pulsaar_get_receiver_info(ctx, i, &mut info) };
        if !matches!(s, PulsaarStatus::Ok) {
            println!("  [{i}] get_receiver_info failed");
            continue;
        }
        let kind_str = ["Unifying", "Bolt", "Nano", "LightSpeed"]
            .get(info.kind as usize)
            .copied()
            .unwrap_or("?");
        println!(
            "  [{i}] {} (pid={:#06X}, kind={})",
            buf_to_str(&info.name), info.product_id, kind_str
        );

        // Step 4: open receiver
        let mut open_status = PulsaarStatus::Unknown;
        let rctx = unsafe { pulsaar_open_receiver(ctx, i, &mut open_status) };
        if rctx.is_null() {
            println!("       open failed: {:?}", open_status as u32);
            continue;
        }

        // Step 5: opened receiver info
        let mut rinfo = unsafe { std::mem::zeroed::<COpenedReceiverInfo>() };
        unsafe { pulsaar_get_opened_receiver_info(rctx, &mut rinfo) };
        println!(
            "       serial={} max_devices={}",
            buf_to_str(&rinfo.serial), rinfo.max_devices
        );

        // Step 6: enumerate devices
        let estate = unsafe { pulsaar_enumerate_devices(rctx) };
        if !matches!(estate, PulsaarStatus::Ok) {
            println!("       enumerate_devices failed: {:?}", estate as u32);
            unsafe { pulsaar_close_receiver(rctx) };
            continue;
        }

        // Step 7-8: query devices
        let dcount = unsafe { pulsaar_get_device_count(rctx) };
        if dcount == 0 {
            println!("       no devices paired");
        }
        for j in 0..dcount {
            let mut dev = unsafe { std::mem::zeroed::<CDeviceInfo>() };
            unsafe { pulsaar_get_device_info(rctx, j, &mut dev) };
            let kind_str = [
                "unknown", "keyboard", "mouse", "numpad", "presenter", "remote",
                "trackball", "touchpad", "tablet", "gamepad", "joystick",
                "headset", "remote_control", "receiver",
            ]
            .get(dev.kind as usize)
            .copied()
            .unwrap_or("?");
            print!(
                "       slot {}: {} ({}) serial={}",
                dev.slot, buf_to_str(&dev.name), kind_str, buf_to_str(&dev.serial)
            );
            if dev.has_battery != 0 {
                let b = &dev.battery;
                let level = if b.level == 0xFF { "?".to_string() } else { format!("{}%", b.level) };
                let status = match b.status {
                    0 => "Discharging", 1 => "Recharging", 2 => "AlmostFull",
                    3 => "Full",        4 => "SlowRecharge", _ => "?",
                };
                print!("  battery: {level} ({status})");
            }
            println!();
        }

        // Step 9: close receiver
        unsafe { pulsaar_close_receiver(rctx) };
    }

    // Step 10: destroy
    unsafe { pulsaar_destroy(ctx) };
    println!("done");
}
