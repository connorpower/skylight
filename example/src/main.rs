#![windows_subsystem = "windows"]

use ::geoms::d2::Size2D;
use ::skylight::{
    types::ResourceId,
    window::{Theme, Window},
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostQuitMessage, TranslateMessage, MSG,
};

pub fn main() {
    // Build and display a new window.
    let mut main_window = Window::new(
        Size2D {
            width: 720,
            height: 640,
        },
        "Hello, Redmond!",
        Some(ResourceId(1)),
        Theme::DarkMode,
    )
    .expect("Failed to create main window");

    // Pump our Win32 message loop. The window will automatically handle most
    // aspects, we just need to test for any pending close or redraw requests
    // and action them accordingly.
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if main_window.clear_redraw_request() {
            // TODO: paint background
        }

        if main_window.clear_close_request() {
            unsafe {
                PostQuitMessage(0);
            }
        }
    }
}
