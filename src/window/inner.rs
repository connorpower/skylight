use crate::{
    errors::{self, Context, Result},
    input::keyboard::{Adapter as KbdAdapter, Keyboard},
    types::*,
    window::{Theme, WindowClass, DPI},
};

use ::geoms::d2::{Point2D, Rect2D, Size2D};
use ::parking_lot::RwLock;
use ::std::{
    cell::{Cell, UnsafeCell},
    marker::PhantomData,
    num::NonZeroIsize,
    ops::DerefMut,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use ::tap::Pipe;
use ::tracing::debug;
use ::widestring::U16CString;
use ::windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::{
            Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE},
            Gdi::UpdateWindow,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::AdjustWindowRectExForDpi,
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW,
                SetWindowLongPtrW, SetWindowPos, ShowWindow, CREATESTRUCTW, CW_USEDEFAULT,
                GWLP_USERDATA, GWLP_WNDPROC, SWP_NOMOVE, SW_SHOWNORMAL, WINDOW_EX_STYLE, WM_CLOSE,
                WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WS_OVERLAPPEDWINDOW,
            },
        },
    },
};

pub(super) struct WindowInner {
    /// Force !Send & !Sync, as our window can only be used by the thread on
    /// which it was created.
    phantom: PhantomData<UnsafeCell<()>>,
    /// A reference-counted handle to the Win32 window class registered for
    /// windows of this type. When the last `Window` instance is released, the
    /// corresponding Win32 window class will be de-registered.
    window_class: Arc<WindowClass>,
    /// A handle to our corresponding Win32 window. If zero, the window has been
    /// destroyed on the Win32 size.
    hwnd: Cell<HWND>,
    /// Fixed size for our window's client area.
    size: Size2D<i32>,
    /// The Window's title, as it appears in the Windows title bar.
    title: String,
    /// The system theme in use by the window - "light" or "dark". This does not
    /// auto-update to track the true system value yet.
    theme: Cell<Theme>,
    /// Stores an outstanding close request from the Win32 side. This must
    /// either be actioned by dropping the top level window, or the close
    /// request can be cleared if it is to be ignored.
    close_request: AtomicBool,
    /// Stores an outstanding paint request from the Win32 side.
    paint_request: AtomicBool,
    /// Keyboard and text input state.
    keyboard: RwLock<Keyboard>,
}

impl WindowInner {
    /// Construct and display a new window.
    pub(super) fn new(
        size: Size2D<i32>,
        title: &str,
        icon_id: Option<ResourceId>,
        theme: Theme,
    ) -> Result<Rc<Self>> {
        debug!(wnd_title = %title, "Creating window inner");

        let this = Rc::new(Self {
            phantom: Default::default(),
            title: title.to_string(),
            window_class: WindowClass::get_or_create("MainWindow", icon_id, Self::wnd_proc_setup)?,
            hwnd: Default::default(),
            size,
            theme: Cell::new(theme),
            close_request: AtomicBool::new(false),
            paint_request: AtomicBool::new(true), // Request immediate draw
            keyboard: RwLock::new(Keyboard::new()),
        });

        let hwnd = {
            let module = unsafe { GetModuleHandleW(None) }
                .context("Failed to construct new window")
                .function("GetModuleHandleW")?;
            let title = U16CString::from_str(title).expect("Window name contained null byte");

            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(this.window_class.class_name().as_ptr()),
                    PCWSTR::from_raw(title.as_ptr()),
                    WS_OVERLAPPEDWINDOW,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    // 0 pixel width and height. We show window as hidden first
                    // so we can first detect the monitor's DPI and request an
                    // appropriate scaled size.
                    0,
                    0,
                    None,
                    None,
                    module,
                    Some(Rc::into_raw(this.clone()) as *const _),
                )
            }
            .pipe(|hwnd| (hwnd.0 != 0).then_some(hwnd))
            .context("Failed to create window")
            .function("CreateWindowExW")?
        };
        this.hwnd.set(hwnd);

        // `SetWindowPos` function takes its size in pixels, so we
        // obtain the window's DPI and use it to scale the window size_
        let dpi = DPI::detect(hwnd);
        let mut rect = dpi
            .scale_rect(Rect2D::with_size_and_origin(size, Point2D::zero()))
            .into();
        unsafe {
            AdjustWindowRectExForDpi(
                &mut rect,
                WS_OVERLAPPEDWINDOW,
                false,
                WINDOW_EX_STYLE::default(),
                dpi.into(),
            )
        }
        .ok()
        .context("Failed to calculate High-DPI window size")
        .function("AdjustWindowRectExForDpi")?;

        let pixel_width = rect.right - rect.left;
        let pixel_height = rect.bottom - rect.top;
        ::tracing::warn!("adjusted window size: {pixel_width} x {pixel_height}");

        unsafe {
            SetWindowPos(
                hwnd,
                HWND::default(),
                0,
                0,
                pixel_width,
                pixel_height,
                SWP_NOMOVE,
            )
        }
        .ok()
        .context("Failed to position window for initial display")
        .function("SetWindowPos")?;

        this.set_theme(theme);
        unsafe {
            ShowWindow(hwnd, SW_SHOWNORMAL);
            UpdateWindow(hwnd);
        }

        Ok(this)
    }

    /// The size of the client area of our Win32 window. The window chrome
    /// is in addition to this siz3.
    pub(super) const fn size(&self) -> Size2D<i32> {
        self.size
    }

    pub(super) fn title(&self) -> &str {
        &self.title
    }

    /// Get a handle to the Win32 window's handle. This is often required when
    /// interacting with other APIs.
    ///
    /// If `None`, then the window has already been destroyed on the Win32 side.
    pub(super) fn hwnd(&self) -> HWND {
        let val = self.hwnd.get();
        assert_ne!(val.0, 0, "Window handle was NULL");
        val
    }

    /// Sets the window's system theme. This currently only controls the color
    /// of the title bar.
    pub(super) fn current_theme(&self) -> Theme {
        self.theme.get()
    }

    /// Sets the window's title bar to match the given theme.
    pub(super) fn set_theme(&self, theme: Theme) {
        let val: i32 = match theme {
            Theme::DarkMode => 0x01,
            Theme::LightMode => 0x00,
        };

        self.theme.set(theme);

        unsafe {
            DwmSetWindowAttribute(
                self.hwnd(),
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &val as *const i32 as _,
                ::std::mem::size_of::<i32>() as u32,
            )
        }
        .context("Failed to set immersive dark mode preferences")
        .function("DwmSetWindowAttribute")
        .unwrap();
    }

    /// Returns whether the window is requesting to close.
    pub(super) fn is_requesting_close(&self) -> bool {
        self.close_request.load(Ordering::SeqCst)
    }

    /// Clears a pending request to close. The window will not request to close
    /// until the next interaction or message triggers this.
    pub(super) fn clear_close_request(&self) {
        self.close_request.store(false, Ordering::SeqCst);
    }

    /// Returns whether the window is requesting to paint.
    pub(super) fn is_requesting_paint(&self) -> bool {
        self.paint_request.load(Ordering::SeqCst)
    }

    /// Clears a pending request to paint. The window will not request to paint
    /// until the next interaction or message triggers this.
    pub(super) fn clear_paint_request(&self) {
        self.paint_request.store(false, Ordering::SeqCst)
    }

    pub fn keyboard(&self) -> impl DerefMut<Target = Keyboard> + '_ {
        self.keyboard.write()
    }

    pub(super) fn destroy(&self) -> Result<()> {
        unsafe { DestroyWindow(self.hwnd()) }
            .ok()
            .context("Failed to destroy window")
            .function("DestroyWindow")
            .map(|_| ())
    }

    /// Handles a Win32 message.
    ///
    /// ## Return Value
    ///
    /// Returns `true` if the message was handled and should not be forwarded to
    /// the default window procedure. Returns `false` if the message was not
    /// handled, or was only intercepted/tapped on the way though and should
    /// still be forwarded to the default procedure.
    fn handle_message(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
        ::tracing::trace!(msg = %crate::debug::msgs::DebugMsg::new(umsg, wparam, lparam));

        if KbdAdapter::handles_msg(umsg, wparam, lparam) {
            if let Some(event) = KbdAdapter::adapt(umsg, wparam, lparam) {
                self.keyboard.write().process_evt(event);
            }
            return true;
        }

        match umsg {
            WM_PAINT => {
                self.paint_request.store(true, Ordering::SeqCst);
                false
            }
            WM_CLOSE => {
                self.close_request.store(true, Ordering::SeqCst);
                true
            }
            WM_NCDESTROY => {
                debug!(wnd_title = %self.title, "Destroying window inner");

                // Our window is being destroyed, so we must clean up our Rc'd
                // handle on the Win32 side.
                errors::clear_last_error();

                let self_ = unsafe { SetWindowLongPtrW(self.hwnd(), GWLP_USERDATA, 0) }
                    .pipe(|val| errors::get_last_err().map(|_| val))
                    .context("Failed to clear Rust window reference from Win32 window data")
                    .function("SetWindowLongPtrW")
                    .unwrap() as *const Self;
                let _ = unsafe { Rc::from_raw(self_) };

                // Clear our window handle now that we're destroyed.
                self.hwnd.set(HWND(0));

                // forward to default procedure too
                false
            }
            _ => false,
        }
    }

    /// C-function Win32 window procedure performs one-time setup of the
    /// structures on the Win32 side to associate our Rust object with the Win32
    /// object.
    extern "system" fn wnd_proc_setup(
        hwnd: HWND,
        umsg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // If we've received a create event, then we populate an `Rc`'ed
        // reference our rust window type in the user data section of the Win32
        // window.
        if umsg == WM_NCCREATE {
            let create_struct = lparam.0 as *const CREATESTRUCTW;
            // SAFETY:
            // The `CREATESRUCTA` structure is guaranteed by the Win32 API to be
            // valid if we've received an event of type `WM_NCCREATE`.
            let self_ = unsafe { (*create_struct).lpCreateParams } as *const Self;

            errors::clear_last_error();
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, self_ as _);
            }
            errors::get_last_err()
                .context("Failed to store reference to Rust window in Win32 window data")
                .function("SetWindowLongPtrW")
                .unwrap();
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_WNDPROC, (Self::wnd_proc_thunk as usize) as isize);
            }
            errors::get_last_err()
                .context("Failed to swap Win32 window proc function")
                .function("SetWindowLongPtrW")
                .unwrap();
        }

        // We _always_ pass our message through to the default window procedure.
        unsafe { DefWindowProcW(hwnd, umsg, wparam, lparam) }
    }

    /// A minimal shim which forwards Win32 window proc messages to our own
    /// type for handling.
    extern "system" fn wnd_proc_thunk(
        hwnd: HWND,
        umsg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if let Ok(ptr) = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) }
            .pipe(NonZeroIsize::new)
            .context("Failed to setup window messaging")
            .function("GetWindowLongPtrW")
        {
            let self_ = ptr.get() as *const Self;

            unsafe {
                // Add extra retain for the duration of following call
                Rc::increment_strong_count(self_);
                if Rc::from_raw(self_).handle_message(umsg, wparam, lparam) {
                    return LRESULT(0);
                }
            }
        }

        unsafe { DefWindowProcW(hwnd, umsg, wparam, lparam) }
    }
}
