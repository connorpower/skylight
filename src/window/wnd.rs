//! Top-level rust Window object which abstracts the underlying Win32 API.

use crate::{
    errors::*,
    input::keyboard::Keyboard,
    types::*,
    window::{Theme, WindowInner, DPI},
};

use ::geoms::d2::Size2D;
use ::std::{ops::DerefMut, rc::Rc};
use ::tracing::{debug, error};
use ::widestring::U16CString;
use ::windows::{
    core::PCWSTR,
    Win32::{Foundation::HWND, UI::WindowsAndMessaging::SetWindowTextW},
};

/// A rusty wrapper around Win32 window class.
///
/// A [Window] is `!Sync + !Send` as Win32 windows must be controlled by the
/// same thread on which they were created.
/// # Example
///
/// ```no_run
/// use ::skylight::window::{Window, Theme, Builder};
/// use ::windows::Win32::UI::WindowsAndMessaging::{
///     DispatchMessageW, GetMessageW, PostQuitMessage, TranslateMessage, MSG,
/// };
///
/// let mut window = Builder::new()
///     .with_title("Hello, Redmond!")
///     .with_theme(Theme::DarkMode)
///     .build()
///     .expect("Failed to create main window");
///
/// // Handle requests in the message loop
/// let mut msg = MSG::default();
/// while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
///     unsafe {
///         TranslateMessage(&msg);
///         DispatchMessageW(&msg);
///     }
///
///     if window.is_requesting_paint() {
///         // paint as needed (Direct2D, Direct3D, GDI, etc.)
///         window.clear_paint_request();
///     }
///
///     if window.is_requesting_close() {
///         window.clear_close_request();
///         unsafe {
///             PostQuitMessage(0);
///         }
///     }
/// }
/// ```
pub struct Window {
    /// The inner refcounted window object. A clone of this object is held on
    /// the win32 API side and should be released when the window is destroyed.
    inner: Rc<WindowInner>,
}

impl Window {
    /// Construct and display a new window.
    pub fn new(
        size: Size2D<i32>,
        title: &str,
        icon_id: Option<ResourceId>,
        theme: Theme,
    ) -> Result<Self> {
        debug!(wnd_title = %title, "Creating window");
        WindowInner::new(size, title, icon_id, theme).map(|inner| Self { inner })
    }

    /// The size of the client area of our Win32 window. The window chrome
    /// is in addition to this size.
    pub fn size(&self) -> Size2D<i32> {
        self.inner.size()
    }

    /// Get a handle to the Win32 window's handle. This is often required when
    /// interacting with other APIs.
    pub fn hwnd(&self) -> HWND {
        self.inner.hwnd()
    }

    /// Sets the window's system theme. This currently only controls the color
    /// of the title bar.
    pub fn current_theme(&self) -> Theme {
        self.inner.current_theme()
    }

    /// Sets the window's title bar to match the given theme.
    pub fn set_theme(&self, theme: Theme) {
        self.inner.set_theme(theme)
    }

    /// Returns the dots per inch (dpi) value for the window.
    pub fn dpi(&self) -> DPI {
        DPI::detect(self.hwnd())
    }

    /// Returns whether the window is requesting to close.
    ///
    /// The window is not actually closed until it is dropped, so the [`Window`]
    /// should usually be dropped if this flag is set.  The close request can be
    /// ignored if needed, and the request to close can be cleared.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ::skylight::window::{Window, Theme, Builder};
    /// # let mut window = Builder::new().build().unwrap();
    ///
    /// // Typically invoked within the core message loop:
    /// if window.is_requesting_close() {
    ///     window.clear_close_request();
    ///     // Drop window, or post quit message if the app should terminate,
    ///     // or simply ignore the request.
    /// }
    /// ```
    pub fn is_requesting_close(&self) -> bool {
        self.inner.is_requesting_close()
    }

    /// Clears a pending close request.
    ///
    /// This should called after handling the close request. Handling a close
    /// request involves dropping the window. During this of dropping a
    /// [`Window`], the Win32 API will invoke a flurry of messages, so it can be
    /// sensible to clear the close request flag to avoid repeated handling.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ::skylight::window::{Window, Theme, Builder};
    /// # let mut window = Builder::new().build().unwrap();
    ///
    /// // Typically invoked within the core message loop:
    /// if window.is_requesting_close() {
    ///     window.clear_close_request();
    ///     // Drop window, or post quit message if the app should terminate,
    ///     // or simply ignore the request.
    /// }
    /// ```
    pub fn clear_close_request(&self) {
        self.inner.clear_close_request();
    }

    /// Returns whether the window has requested to be painted.
    ///
    /// The [`Window`] object provides no drawing functionality. This must be
    /// handled by a higher level as appropriate via GDI, Direct2D, or Direct3D
    /// call. The paint request can be ignored if needed.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ::skylight::window::{Window, Theme, Builder};
    /// # let mut window = Builder::new().build().unwrap();
    ///
    /// // Typically invoked within the core message loop:
    /// if window.is_requesting_paint() {
    ///     window.clear_paint_request();
    ///     // paint as needed (Direct2D, Direct3D, GDI, etc.)
    /// }
    /// ```
    pub fn is_requesting_paint(&self) -> bool {
        self.inner.is_requesting_paint()
    }

    /// Clears a pending paint request.
    ///
    /// This should called each time after the window is painted to clear the
    /// pending flag. The pending request can also be cleared without painting.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ::skylight::window::{Window, Theme, Builder};
    /// # let mut window = Builder::new().build().unwrap();
    ///
    /// // Typically invoked within the core message loop:
    /// if window.is_requesting_paint() {
    ///     window.clear_paint_request();
    ///     // paint as needed (Direct2D, Direct3D, GDI, etc.)
    /// }
    pub fn clear_paint_request(&self) {
        self.inner.clear_paint_request();
    }

    /// Reads the keyboard state. A read lock is held during this process, so
    /// the reference must be dropped for further keyboard input to be handled.
    pub fn keyboard(&self) -> impl DerefMut<Target = Keyboard> + '_ {
        self.inner.keyboard()
    }

    /// Set the window title.
    pub fn set_title(&self, title: &str) -> Result<()> {
        let string = U16CString::from_str_truncate(title);
        unsafe { SetWindowTextW(self.hwnd(), PCWSTR::from_raw(string.as_ptr())) }
            .ok()
            .context("Failed to set window title")
            .function("SetWindowTextW")
            .map(|_| ())
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        debug!(wnd_title = %&self.inner.title(), "Dropping window");
        if let Err(e) = self.inner.destroy() {
            error!("Failed to destroy window: {}", e);
        }
    }
}
