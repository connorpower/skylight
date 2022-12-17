// The feature flag `stdio` can be used to conditionally disable the windows
// subsystem which allows program output to be sent to the console which
// launched the app. Useful mostly for debugging.
#![cfg_attr(not(feature = "stdio"), windows_subsystem = "windows")]

mod resources;

use crate::resources::FERRIS_ICON;
use ::geoms::d2::Size2D;
use ::skylight::window::{Builder, Theme};
use ::tracing_subscriber::{fmt, prelude::*, EnvFilter};
use ::windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostQuitMessage, TranslateMessage, MSG,
};

pub fn main() {
    ::tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    // Build and display a new window.
    let main_window = Builder::new()
        .with_size(Size2D {
            width: 720,
            height: 640,
        })
        .with_title("Hello, Redmond!")
        .with_icon(FERRIS_ICON.id().into())
        .with_theme(Theme::DarkMode)
        .build()
        .expect("Failed to create main window");

    // Pump our Win32 message loop. The window will automatically handle most
    // aspects, we just need to test for any pending close or paint requests
    // and action them accordingly.
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if main_window.is_requesting_paint() {
            // paint as needed (Direct2D, Direct3D, GDI, etc.)
            main_window.clear_paint_request();
        }

        if main_window.is_requesting_close() {
            main_window.clear_close_request();
            unsafe {
                PostQuitMessage(0);
            }
        }
    }
}
