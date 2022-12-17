//! Builder object which constructs [`Window`]s
//!
//! [`Window`]: crate::window::Window

use crate::{
    errors::Result,
    types::ResourceId,
    window::{Theme, Window},
};

use ::geoms::d2::Size2D;

/// A builder pattern object which simplifies the process of creating a
/// [`Window`].
///
/// The same builder can be re-used to create multiple windows with the same
/// configuration, as a type of prototype.
///
/// ```no_run
/// use ::skylight::window::{Theme, Builder};
///
/// let window = Builder::new()
///     .with_title("Hello, Redmond!")
///     .with_theme(Theme::DarkMode)
///     .build()
///     .expect("Window creation failed");
/// ```
///
/// [`Window`]: crate::window::Window
#[derive(Clone, Debug)]
pub struct Builder {
    title: Option<String>,
    size: Size2D<i32>,
    icon_id: Option<ResourceId>,
    theme: Theme,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    /// Construct a new builder. Default values will be used for all properties
    /// until explicitly set.
    pub fn new() -> Self {
        Self {
            title: None,
            size: Size2D {
                width: 720,
                height: 640,
            },
            icon_id: None,
            theme: Theme::LightMode,
        }
    }

    /// Set the window title, as it appears in the title bar and task manager.
    ///
    /// Defaults to the empty string if not set.
    pub fn with_title(self, title: impl AsRef<str>) -> Self {
        Self {
            title: title.as_ref().to_owned().into(),
            ..self
        }
    }

    /// Set a size for the window.
    ///
    /// Defaults to 720 x 640 if not set.
    pub fn with_size(self, size: Size2D<i32>) -> Self {
        Self { size, ..self }
    }

    /// Set the window's icon, as it appears in the title bar and task manager.
    ///
    /// Defaults to no icon (the system default generic app icon).
    pub fn with_icon(self, icon: ResourceId) -> Self {
        Self {
            icon_id: Some(icon),
            ..self
        }
    }

    /// Sets the [`Theme`] - either light or dark.
    ///
    /// Defaults to [`Theme::LightMode`] if not set.
    ///
    /// [`Theme`]: crate::window::Theme
    /// [`Theme::LightMode`]: crate::window::Theme::LightMode
    pub fn with_theme(self, theme: Theme) -> Self {
        Self { theme, ..self }
    }

    /// Gets the currently set window title.
    pub fn title(&self) -> Option<&str> {
        self.title.as_ref().map(String::as_ref)
    }

    /// Gets the currently set window size
    pub fn size(&self) -> Size2D<i32> {
        self.size
    }

    /// Gets the currently set icon.
    pub fn icon(&self) -> Option<ResourceId> {
        self.icon_id
    }

    /// Gets the currently set window theme.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// Build a new [`Window`] with the properties of the builder.
    ///
    /// The builder can be re-used to create multiple windows with these same
    /// properties.
    ///
    /// [`Window`] crate::window::Window
    pub fn build(&self) -> Result<Window> {
        Window::new(
            self.size,
            self.title.as_ref().map(|s| s.as_ref()).unwrap_or(""),
            self.icon_id,
            self.theme,
        )
    }
}
