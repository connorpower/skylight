//! Input and state handling for keyboard events.
//!
//! # Keyboard
//!
//! [`Keyboard`] is the core keyboard abstraction and provides a simplified,
//! unicode compatible, and safe view of the keyboard state which can be queried
//! at any time. A [`Keyboard`] can answer questions such as whether a given key
//! is pressed, and accumulates a text input buffer which contains all text a
//! user has entered since it was last queried.
//!
//! [`Keyboard`] handles Windows process messages directly to maintain its state
//! up to date. Event handling is performed in-line, and state is updated
//! synchronously as soon as messages arrive, so there is no additional async
//! lag or jitter introduced when using a [`Keyboard`].
//!
//! Every [`Window`] has a [`Keyboard`] object which can be accessed via
//! [`Window::keyboard()`].
//!
//! ## Example
//!
//! ```
//! use ::skylight::input::keyboard::{Keyboard, KeyCode};
//!
//! # struct Window {};
//! # impl Window {
//! #     fn keyboard(&self) -> Keyboard { Keyboard::new() }
//! # }
//! # let window = Window {};
//! // let window = some window...
//!
//! // Retrieve the keyboard for a given window.
//! let mut keyboard = window.keyboard();
//!
//! // Test the state of a particular virtual key.
//! assert!(!keyboard.is_key_pressed(KeyCode::Left));
//!
//! // Drain and collect any pending input. The keyboard input is unicode
//! // compatible and always returns valid unicode chars.
//! let input: String = keyboard.drain_input().chars().collect();
//! assert_ne!(&input, "👌");
//! ```
//!
//! ## Advanced Types
//!
//! Although you should almost always interact with a [`Keyboard`], this module
//! exposes several low-level types which might be useful if you are doing your
//! own Windows process message loop handling and not relying on the [`Window`]
//! class to do this automatically internally.
//!
//! **`KeyEvent`**
//!
//! A [`KeyEvent`] is a safe and strongly typed representation Win32 virtual key
//! event sent to this process. A [`KeyEvent`] will give you access to the raw
//! [`KeystrokeFlags`] and the virtual [`KeyCode`] for the key which emitted the
//! event.
//!
//! **`KeystrokeFlags`**
//!
//! [`KeystrokeFlags`] are a safe struct interpretation of the bitfield that
//! accompanies every key event in the Windows platform. These flags identify
//! the OEM key, repeat count, transition state, and other properties of an
//! event.
//!
//! [`Window::keyboard()`]: crate::window::Window::keyboard
//! [`Window`]: crate::window::Window
//! [`Keyboard`]: crate::input::keyboard::Keyboard
//! [`KeystrokeFlags`]: crate::input::keyboard::KeystrokeFlags
//! [`KeyCode`]: crate::input::keyboard::KeyCode

mod codes;
mod event;
#[allow(clippy::module_inception)] // no exposed publicly
mod keyboard;

pub use codes::KeyCode;
pub use event::{KeyEvent, KeystrokeFlags};
pub use keyboard::{InputBuffer, Keyboard, INPUT_QUEUE_CAPACITY};
