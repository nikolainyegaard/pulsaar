// Diagnostic binary: lists every Logitech HID interface visible to hidapi.
// Run with: cargo run --bin list_hid

fn main() {
    let api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => { eprintln!("hidapi init failed: {e}"); return; }
    };

    let mut found = 0usize;
    for d in api.device_list() {
        if d.vendor_id() != 0x046D { continue; }
        found += 1;
        println!(
            "pid=0x{:04X}  page=0x{:04X}  usage=0x{:04X}  transport={:?}  name={:?}  serial={:?}  path={}",
            d.product_id(),
            d.usage_page(),
            d.usage(),
            d.bus_type(),
            d.product_string().unwrap_or(""),
            d.serial_number().unwrap_or(""),
            d.path().to_string_lossy(),
        );
    }

    if found == 0 {
        println!("No Logitech (0x046D) HID devices found.");
    }
}
