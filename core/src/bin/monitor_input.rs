/// Raw keyboard input monitor.
///
/// Run with: cargo run --bin monitor_input
///
/// Creates a message-only Win32 window, registers for keyboard raw input
/// (RIDEV_INPUTSINK so it receives events even when not the foreground app),
/// and prints every key press with the originating device path.
///
/// Key question to answer: does the path contain "VID_046D"?
///   YES -> Pulsaar's wakeup detection WILL trigger for this keyboard.
///   NO  -> The keyboard is arriving through a different HID node and the
///          current detection won't see it.

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem;
use std::time::Instant;

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::HBRUSH,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::{
            GetRawInputData, GetRawInputDeviceInfoW, RegisterRawInputDevices,
            RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK,
            RID_INPUT, RIDI_DEVICENAME,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
            RegisterClassExW, TranslateMessage, HWND_MESSAGE, MSG, WM_INPUT,
            WNDCLASSEXW,
        },
    },
};

// Single-threaded message loop: no mutex needed.
static mut START: Option<Instant> = None;
// Key = hDevice as usize (raw pointer value used as a stable identity).
static mut CACHE: Option<HashMap<usize, String>> = None;

fn main() {
    unsafe {
        START = Some(Instant::now());
        CACHE = Some(HashMap::new());

        let hinstance = GetModuleHandleW(std::ptr::null());

        // Register a minimal window class.
        let class_name: Vec<u16> = "PulsaarInputMon\0".encode_utf16().collect();
        let mut wc: WNDCLASSEXW = mem::zeroed();
        wc.cbSize        = mem::size_of::<WNDCLASSEXW>() as u32;
        wc.lpfnWndProc   = Some(wnd_proc);
        wc.hInstance     = hinstance;
        wc.lpszClassName = class_name.as_ptr();
        wc.hbrBackground = 0 as HBRUSH;
        RegisterClassExW(&wc);

        // Message-only window (HWND_MESSAGE parent) -- never shown.
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            0,
            0, 0, 0, 0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );

        if hwnd.is_null() {
            eprintln!("CreateWindowExW failed");
            return;
        }

        // Register for keyboard raw input even when this window is not focused.
        let rid = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage:     0x06,  // Keyboard
            dwFlags:     RIDEV_INPUTSINK,
            hwndTarget:  hwnd,
        };
        if RegisterRawInputDevices(&rid, 1, mem::size_of::<RAWINPUTDEVICE>() as u32) == 0 {
            eprintln!("RegisterRawInputDevices failed");
            return;
        }

        println!("Monitoring raw keyboard input. Press keys on the target keyboard...");
        println!();
        println!("{:<14}  {}", "Time", "Device path");
        println!("{}", "-".repeat(90));

        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        let h_raw = lparam as *mut c_void;
        let header_size = mem::size_of::<RAWINPUTHEADER>() as u32;
        let mut cb: u32 = 0;

        GetRawInputData(h_raw, RID_INPUT, std::ptr::null_mut(), &mut cb, header_size);

        if cb > 0 && cb <= 512 {
            let mut buf = vec![0u8; cb as usize];
            if GetRawInputData(
                h_raw,
                RID_INPUT,
                buf.as_mut_ptr() as *mut c_void,
                &mut cb,
                header_size,
            ) != u32::MAX
            {
                let ri = &*(buf.as_ptr() as *const RAWINPUT);
                let h_device = ri.header.hDevice;
                let path = device_path(h_device);
                let ms   = START.as_ref().unwrap().elapsed().as_millis();
                let tag  = if path.to_ascii_uppercase().contains("VID_046D") {
                    "  <- VID_046D (Logitech) -- wakeup detection WILL fire"
                } else {
                    ""
                };
                println!("[T+{ms:<8}ms]  {path}{tag}");
            }
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn device_path(h_device: *mut c_void) -> String {
    let cache = CACHE.as_mut().unwrap();
    let key = h_device as usize;

    if let Some(p) = cache.get(&key) {
        return p.clone();
    }

    let mut name_len: u32 = 0;
    GetRawInputDeviceInfoW(h_device, RIDI_DEVICENAME, std::ptr::null_mut(), &mut name_len);

    let path = if name_len > 0 {
        let mut buf: Vec<u16> = vec![0u16; name_len as usize];
        GetRawInputDeviceInfoW(
            h_device,
            RIDI_DEVICENAME,
            buf.as_mut_ptr() as *mut c_void,
            &mut name_len,
        );
        String::from_utf16_lossy(&buf)
            .trim_end_matches('\0')
            .to_string()
    } else {
        format!("<unknown 0x{key:X}>")
    };

    cache.insert(key, path.clone());
    path
}
