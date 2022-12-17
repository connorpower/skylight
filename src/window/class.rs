//! Management of Win32 Windows classes.

use crate::{errors::*, types::*};

use ::std::{
    fmt::Write,
    num::NonZeroU16,
    sync::{Arc, Weak as SyncWeak},
};
use ::tap::prelude::*;
use ::tracing::{debug, error};
use ::widestring::U16CString;
use ::windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            LoadCursorW, LoadImageW, RegisterClassExW, UnregisterClassW, CS_HREDRAW, CS_VREDRAW,
            HICON, IDC_ARROW, IMAGE_ICON, LR_DEFAULTSIZE, WNDCLASSEXW,
        },
    },
};

use ::lazy_static::lazy_static;
use ::parking_lot::Mutex;
use ::std::collections::{hash_map::Entry, HashMap};

/// Typedef for the Win32 windows procedure function - the primary entry point
/// for the Windows message pump.
type WndProc = extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

lazy_static! {
    static ref WINDOW_REGISTRATIONS: Mutex<HashMap<U16CString, SyncWeak<WindowClass>>> =
        Default::default();
}

/// A RAII object which manages Windows class registrations.
///
/// A windows class will be registered with the system the first time one is
/// created. Subsequent requests for a windows class with the same properties
/// will return a reference to the already registered class. When no more live
/// references to a registered class exist, it will be automatically
/// deregistered with the system to free resources.
///
/// Multiple different window classes can be registered and in use
/// simultaneously.
pub(super) struct WindowClass {
    class_name: U16CString,
}

impl WindowClass {
    /// Gets a handle to an existing window class registration, or registers
    /// the window class for the first time.
    pub(super) fn get_or_create(
        class_name_prefix: &str,
        icon_id: Option<ResourceId>,
        wnd_proc_setup: WndProc,
    ) -> Result<Arc<Self>> {
        let mut registry = WINDOW_REGISTRATIONS.lock();
        let mut class_name = class_name_prefix.to_owned();
        if let Some(icon) = icon_id {
            class_name.write_fmt(format_args!("-{icon}")).unwrap();
        }
        let class_name =
            U16CString::from_str(class_name).expect("Null byte found in window class name");

        match registry.entry(class_name) {
            Entry::Vacant(entry) => {
                let class = Self::register(entry.key().clone(), icon_id, wnd_proc_setup)?;
                entry.insert(Arc::downgrade(&class));
                Ok(class)
            }
            Entry::Occupied(mut entry) => {
                if let Some(strong_ref) = entry.get().upgrade() {
                    Ok(strong_ref)
                } else {
                    let class = Self::register(entry.key().clone(), icon_id, wnd_proc_setup)?;
                    entry.insert(Arc::downgrade(&class));
                    Ok(class)
                }
            }
        }
    }

    pub(super) fn class_name(&self) -> &U16CString {
        &self.class_name
    }

    fn register(
        class_name: U16CString,
        icon_id: Option<ResourceId>,
        wnd_proc_setup: WndProc,
    ) -> Result<Arc<Self>> {
        debug!(
            wnd_class = class_name.to_string_lossy(),
            "Register window class"
        );

        let module = unsafe { GetModuleHandleW(None) }
            .context("Failed to get module handle to register window class")
            .function("GetModuleHandleW")?;
        let cursor = unsafe { LoadCursorW(HINSTANCE::default(), IDC_ARROW) }
            .context("Failed to load cursor to register window class")
            .function("LoadCursorW")?;
        let icon = icon_id
            .map(|resource_id: ResourceId| {
                unsafe {
                    LoadImageW(
                        module,
                        resource_id.into_pcwstr(),
                        IMAGE_ICON,
                        0,
                        0,
                        LR_DEFAULTSIZE,
                    )
                }
                .context("Failed to load icon when registering window class")
                .function("LoadImageW")
            })
            .transpose()?;

        let wnd_class = WNDCLASSEXW {
            cbSize: ::std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc_setup),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            hCursor: cursor,
            hIcon: HICON(icon.map(|i| i.0).unwrap_or(0)),
            ..Default::default()
        };
        let _atom = unsafe { RegisterClassExW(&wnd_class) }
            .pipe(NonZeroU16::new)
            .context("Failed to register window class")
            .function("RegisterClassExW")?;

        Ok(Arc::new(Self { class_name }))
    }

    fn unregister(&self) -> Result<()> {
        debug!(wnd_class = ?self.class_name().to_string_lossy(), "Unregister window class");
        let module = unsafe { GetModuleHandleW(None) }
            .context("Failed to get current module handle")
            .function("GetModuleHandleW")?;
        unsafe { UnregisterClassW(PCWSTR::from_raw(self.class_name().as_ptr()), module) }
            .ok()
            .context("Failed to unregister window class")
            .function("UnregisterClassW")?;
        Ok(())
    }
}

impl Drop for WindowClass {
    fn drop(&mut self) {
        if let Err(e) = self.unregister() {
            error!(error = %e);
        }
    }
}
