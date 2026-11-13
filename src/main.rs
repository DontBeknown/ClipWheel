#![windows_subsystem = "windows"]
mod app;
mod clipboard;
mod render;
fn main() {
    if let Err(error) = app::run() {
        let message: Vec<u16> = format!("ClipWheel could not start: {error}\0")
            .encode_utf16()
            .collect();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                None,
                windows::core::PCWSTR(message.as_ptr()),
                windows::core::w!("ClipWheel"),
                windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
}
