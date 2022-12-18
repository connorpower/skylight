//! Keyboard state and text input tracking.

use ::bitvec::prelude::*;
use ::std::{char::REPLACEMENT_CHARACTER, collections::VecDeque};
use ::tracing::trace;
use ::widestring::WideChar;

use super::{KeyCode, KeyEvent};

/// Length of the [`Keyboard`] input queue, after which point the earliest
/// characters are dropped (FIFO).
pub const INPUT_QUEUE_CAPACITY: usize = 32;

const BACKSPACE: char = '\x08';

/// The central object which tracks and manages keyboard state and text input.
///
/// # Key Pressed Tracking
///
/// Windows communicates keyboard changes by sending messages to process. This
/// can make handling keyboard state difficult as the events must be processed
/// immediately, or otherwise stored. Most applications aren't prepared to
/// handle the key events immediately as they come in. For instance, a typical
/// game loop has a well-defined location in an update loop where key state is
/// looked at and appropriate actions are taken for the next render loop. This
/// task is made difficult without persistent keyboard state.
///
/// Keeping track of these messages and maintaining a persistent view of which
/// keys are in which state is one of the primary tasks for the [`Keyboard`]
/// object. The application is free to ask [`Keyboard`] for the state of a key
/// at any time. Windows process events are handled opaquely in the background
/// to keep the [`Keyboard`] state constantly up to date.
///
/// # Text Input
///
/// Similar to key pressed state, text input is also communicated by Windows by
/// sending messages to the process.  Much the same problems occur - it's not
/// always convenient to handle these messages _immediately_ as they arrive.
/// Often, components would prefer to look at the input buffer during their own
/// update cycles.
///
/// Text input does not have a 1:1 relationship to the virtual key events that
/// indicate which keys are pressed. Keyboard layouts and languages might mean
/// that the same physical key corresponds to different text input.
/// International input modes allow sequences of keys to input a single
/// character (often with umlaut, accent, or other modifier) which means there
/// is no direct relationship between key pressed and input text. Lastly,
/// alternative input modes are supported such as the emoji or special character
/// keyboard, or the lesser known manual hex-code input (which if raw events
/// were observed, would look like alt-key, plus-key, and a series of
/// hexadecimal characters).
///
/// Text input is further complicated by modern unicode. The Windows process
/// messages may send UTF-16 surrogate pairs which are invalid on their own,
/// necessitating that they be stored somewhere as pending until the
/// corresponding low surrogate arrives.
///
/// Lastly, Windows deals natively with UTF-16 and makes no guarantee about
/// unicode validity. Rust strings operate on UTF-8 and require valid unicode
/// and will panic otherwise. It's therefore relatively dangerous to naively
/// interact with Windows strings.
///
/// The [`Keyboard`] object solves each of these problems by maintaining an
/// input queue for text input. The input queue stores text input until it is
/// explicitly drained by a caller. Draining the input queue yields an iterator
/// over valid UTF-8 rust characters. UTF-16 surrogates are handled internally,
/// and not added to the queue until both upper and lower pairs have arrived.
/// Invalid unicode is converted into the unicode unknown character.
///
/// # Example
///
/// ```
/// use ::skylight::input::keyboard::{Keyboard, KeyCode};
///
/// # struct Window {};
/// # impl Window {
/// #     fn keyboard(&self) -> Keyboard { Keyboard::new() }
/// # }
/// # let window = Window {};
/// // let window = some window...
///
/// // Retrieve the keyboard for a given window.
/// let mut keyboard = window.keyboard();
///
/// // Test the state of a particular virtual key.
/// assert!(!keyboard.is_key_pressed(KeyCode::Left));
///
/// // Drain and collect any pending input. The keyboard input is unicode
/// // compatible and always returns valid unicode chars.
/// let input: String = keyboard.drain_input().chars().collect();
/// assert_ne!(&input, "👌");
/// ```
///
/// [`Window`]: crate::window::Window
pub struct Keyboard {
    /// Bitfield which tracks the press state for the keyboard keys.
    pressed: BitArr!(for 255, in usize, Lsb0),
    /// A queue of printable input text which has been fully processed into
    /// valid unicode.
    input_queue: VecDeque<char>,
    /// The number of pending backspace events which should be applied to any
    /// previously retrieved text.
    n_backspaces: usize,
    /// High surrogate entry from a surrogate pair. This is `Some` pending
    /// receipt of the following low surrogate. Once the low surrogate arrives,
    /// the pair can be converted into a character and appended to
    /// `input_queue`.
    pending_surrogate: Option<WideChar>,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    /// Constructs a new keyboard.
    ///
    /// <p style="background:rgba(255,181,77,0.16);padding:0.75em;">
    /// <strong>Warning:</strong> This API is for advanced use only.
    /// </p>
    ///
    /// You should not usually manually construct a [`Keyboard`] but rather
    /// retrieve the [`Keyboard`] instance for a given [`Window`] via
    /// [`Window::keyboard`].  This constructor is public only for advanced
    /// uses, or to enable unit testing in your own app.
    ///
    /// [`Window`]: crate::window::Window
    /// [`Window::keyboard`]: crate::window::Window::keyboard
    pub fn new() -> Self {
        Self {
            pressed: bitarr![usize, Lsb0; 0; 255],
            input_queue: VecDeque::with_capacity(INPUT_QUEUE_CAPACITY),
            n_backspaces: 0,
            pending_surrogate: None,
        }
    }

    /// Process an event from the Win32 system and update internal state.
    ///
    /// <p style="background:rgba(255,181,77,0.16);padding:0.75em;">
    /// <strong>Warning:</strong> This API is for advanced use only.
    /// </p>
    ///
    /// Process an event from the Win32 system and update internal state. This
    /// event will be reflected in the next user call to [`is_key_pressed`] or
    /// added to the input buffer as appropriate.
    ///
    /// You do not normally need to call this function. A [`Window`] will manage
    /// its own keyboard state automatically. This method is public only for
    /// advanced uses or to enable unit testing in your own application.
    ///
    /// [`is_key_pressed`]: Self::is_key_pressed
    /// [`Window`]: crate::window::Window
    pub fn process_evt(&mut self, evt: KeyEvent) {
        match evt {
            KeyEvent::KeyDown { key_code, flags } => {
                if !flags.was_previous_state_down {
                    *self.mut_bit_for_key(key_code).as_mut() = true;
                }
            }
            KeyEvent::KeyUp { key_code, .. } => {
                *self.mut_bit_for_key(key_code).as_mut() = false;
            }
            KeyEvent::Input { wchar, .. } => {
                match self.pending_surrogate.take() {
                    Some(high) => {
                        let low = wchar;
                        // Combine surrogates & append to input queue. If anything fails at this
                        // point we don't have a recourse for recovery so we take the unicode
                        // replacement character instead.
                        self.process_char_input(
                            char::decode_utf16([high, low])
                                .map(|r| r.unwrap_or(REPLACEMENT_CHARACTER)),
                        );
                    }
                    None => match char::decode_utf16([wchar])
                        .next()
                        .expect("Iterator contains a wchar and should yield at least one result")
                    {
                        // If we've received the first high-surrogate, we must first wait for the
                        // following low surrogate.
                        Err(err) => self.pending_surrogate = Some(err.unpaired_surrogate()),
                        // Happy-path for non-surrogate-pair unicode characters
                        Ok(ch) => self.process_char_input([ch]),
                    },
                }
            }
        }
    }

    /// Returns `true` if the given key is currently pressed, otherwise `false`.
    pub fn is_key_pressed(&self, key: KeyCode) -> bool {
        *self.bit_for_key(key).as_ref()
    }

    /// Drains all accumulated input in the input queue and clears any pending
    /// backspace events.
    pub fn drain_input(&mut self) -> InputBuffer<impl ExactSizeIterator<Item = char> + '_> {
        let n_backspaces = self.n_backspaces;
        self.n_backspaces = 0;

        InputBuffer {
            n_backspaces,
            chars: self.input_queue.drain(..),
        }
    }

    /// Reset all keyboard state.
    pub fn reset(&mut self) {
        self.input_queue.clear();
        self.pending_surrogate = None;
        self.pressed = BitArray::ZERO;
    }

    /// Handles character input and appends or modifies the input queue. The
    /// char iterator could contain only a single char, multiple characters, and
    /// could include control characters such as backspace or delete.
    /// [process_char_input] will account for deletion events.
    fn process_char_input<I>(&mut self, chars: I)
    where
        I: IntoIterator<Item = char>,
    {
        let chars = chars.into_iter();
        for c in chars {
            match c {
                // TODO: detect delete
                BACKSPACE => {
                    if self.input_queue.pop_back().is_none() {
                        self.n_backspaces += 1;
                    }
                }
                // Drop any control characters that are not whitespace
                _ if c.is_control() && !c.is_whitespace() => (),
                _ => self.input_queue.push_back(c),
            }
        }

        // Trim queue to avoid growing continuously
        while self.input_queue.len() >= INPUT_QUEUE_CAPACITY {
            let char = self.input_queue.pop_front().unwrap();
            trace!("Trimming keyboard input queue, dropped '{char}'.");
        }
    }

    fn bit_for_key(&self, key: KeyCode) -> impl AsRef<bool> + '_ {
        self.pressed.get(key.value() as usize).unwrap()
    }

    fn mut_bit_for_key(&mut self, key: KeyCode) -> impl AsMut<bool> + '_ {
        self.pressed.get_mut(key.value() as usize).unwrap()
    }
}

/// An object returned by [`Keyboard::drain_input()`] which encapsulates the
/// pending text input.
///
/// The [`InputBuffer`] yields an iterator over the pending characters in
/// the buffer via [`chars()`]
///
/// The [`InputBuffer`] also indicates the number of backspace events that
/// occurred **prior** to the text in the buffer via [`num_backspaces()`].
/// Typically, you would erase as many characters from the end of previously
/// drained text as there are backspaces, before concatenating the new chars.
/// Backspace events which occurred within the buffer are already applied to
/// remove data in the buffer. It is only the backspace events which occurred
/// prior to any input which you must handle yourself.
///
/// # Example
///
/// ```
/// use ::skylight::input::keyboard::{Keyboard, KeyCode};
///
/// # struct Window {};
/// # impl Window {
/// #     fn keyboard(&self) -> Keyboard { Keyboard::new() }
/// # }
/// # let window = Window {};
/// # let mut text = String::new();
/// // let window = some window...
/// // let mut text = some previous text input...
///
/// // Retrieve the keyboard for a given window.
/// let mut keyboard = window.keyboard();
///
/// // Drain any pending text input and backspace events that came before the
/// // input characters.
/// let mut input = keyboard.drain_input();
///
/// // First process any pending backspace events by removing characters from
/// // our existing text (backspaces _within_ the new pending text will have
/// // already been applied for us):
/// text.truncate(text.len().saturating_sub(input.num_backspaces()));
///
/// // Append any new characters in the input buffer:
/// text.extend(input.chars());
/// ```
///
/// [`Keyboard::drain_input()`]: crate::input::keyboard::Keyboard::drain_input
/// [`chars()`]: InputBuffer::chars
/// [`num_backspaces()`]: InputBuffer::num_backspaces
pub struct InputBuffer<I>
where
    I: ExactSizeIterator<Item = char>,
{
    chars: I,
    n_backspaces: usize,
}

impl<I> InputBuffer<I>
where
    I: ExactSizeIterator<Item = char>,
{
    /// The number of backspaces which **preceded** any text in the [`chars()`]
    /// buffer and should be removed from to any **previously** drained input
    /// if required.
    ///
    /// [`chars()`]: Self::chars
    pub fn num_backspaces(&self) -> usize {
        self.n_backspaces
    }

    /// The pending text input buffer.
    ///
    /// Any backspace events which happened **within** this buffer have already
    /// been applied to the buffer contents and do not need to be taken into
    /// consideration.
    pub fn chars(&mut self) -> &mut impl ExactSizeIterator<Item = char> {
        &mut self.chars
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{input::keyboard::KeystrokeFlags, window::WindowsProcessMessage};

    use ::std::ops::Not;
    use ::strum::IntoEnumIterator;
    use ::widestring::u16str;
    use ::windows::Win32::UI::WindowsAndMessaging::*;

    mod event_samples {
        use super::*;

        /// Press and release "a" character.
        pub const PRESS_RELEASE_A: &[WindowsProcessMessage] = &[
            WindowsProcessMessage {
                umsg: WM_KEYDOWN,
                wparam: 0x41,
                lparam: 0x000001E0001,
            },
            WindowsProcessMessage {
                umsg: WM_CHAR,
                wparam: 0x61,
                lparam: 0x000001E0001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYUP,
                wparam: 0x41,
                lparam: 0x000C01E0001,
            },
        ];

        /// Press and release "b" character.
        pub const PRESS_RELEASE_B: &[WindowsProcessMessage] = &[
            WindowsProcessMessage {
                umsg: WM_KEYDOWN,
                wparam: 0x42,
                lparam: 0x00000300001,
            },
            WindowsProcessMessage {
                umsg: WM_CHAR,
                wparam: 0x62,
                lparam: 0x00000300001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYUP,
                wparam: 0x42,
                lparam: 0x000C0300001,
            },
        ];

        /// Press and release "c" character.
        pub const PRESS_RELEASE_C: &[WindowsProcessMessage] = &[
            WindowsProcessMessage {
                umsg: WM_KEYDOWN,
                wparam: 0x43,
                lparam: 0x000002E0001,
            },
            WindowsProcessMessage {
                umsg: WM_CHAR,
                wparam: 0x63,
                lparam: 0x000002E0001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYUP,
                wparam: 0x43,
                lparam: 0x000C02E0001,
            },
        ];

        /// Press and release "backspace" key.
        pub const PRESS_RELEASE_BACKSPACE: &[WindowsProcessMessage] = &[
            WindowsProcessMessage {
                umsg: WM_KEYDOWN,
                wparam: 0x8,
                lparam: 0x000000E0001,
            },
            WindowsProcessMessage {
                umsg: WM_CHAR,
                wparam: 0x8,
                lparam: 0x000000E0001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYUP,
                wparam: 0x8,
                lparam: 0x000C00E0001,
            },
        ];

        /// Text entry for 'ö' ('"' + 'o' combo on international keyboard).
        pub const PRESS_RELEASE_INTERNATIONAL_UMLAUT: &[WindowsProcessMessage] = &[
            WindowsProcessMessage {
                umsg: WM_KEYDOWN,
                wparam: 0x10,
                lparam: 0x002A0001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYDOWN,
                wparam: 0xDE,
                lparam: 0x00280001,
            },
            WindowsProcessMessage {
                umsg: WM_DEADCHAR,
                wparam: 0x22,
                lparam: 0x00280001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYUP,
                wparam: 0xDE,
                lparam: 0xC0280001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYUP,
                wparam: 0x10,
                lparam: 0xC02A0001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYDOWN,
                wparam: 0x4F,
                lparam: 0x00180001,
            },
            WindowsProcessMessage {
                umsg: WM_CHAR,
                wparam: 0xF6,
                lparam: 0x00180001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYUP,
                wparam: 0x4F,
                lparam: 0xC0180001,
            },
        ];

        /// Emoji input for 👌 using emoji keyboard ("win-.")
        pub const EMOJI_INPUT_OK_HAND: &[WindowsProcessMessage] = &[
            WindowsProcessMessage {
                umsg: WM_IME_REQUEST,
                wparam: 0x0006,
                lparam: 0x643E50BC90,
            },
            WindowsProcessMessage {
                umsg: WM_GETICON,
                wparam: 0x0000,
                lparam: 0x0000000078,
            },
            WindowsProcessMessage {
                umsg: WM_KEYDOWN,
                wparam: 0x005B,
                lparam: 0x00015B0001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYUP,
                wparam: 0x00BE,
                lparam: 0x0080340001,
            },
            WindowsProcessMessage {
                umsg: WM_KEYUP,
                wparam: 0x005B,
                lparam: 0x00C15B0001,
            },
            WindowsProcessMessage {
                umsg: WM_IME_STARTCOMPOSITION,
                wparam: 0x0000,
                lparam: 0x0000000000,
            },
            WindowsProcessMessage {
                umsg: WM_IME_NOTIFY,
                wparam: 0x000F,
                lparam: 0x0020600A01,
            },
            WindowsProcessMessage {
                umsg: WM_IME_NOTIFY,
                wparam: 0x000F,
                lparam: 0x0020600A01,
            },
            WindowsProcessMessage {
                umsg: WM_IME_KEYLAST,
                wparam: 0xD83D,
                lparam: 0x0000000800,
            },
            WindowsProcessMessage {
                umsg: WM_IME_CHAR,
                wparam: 0xD83D,
                lparam: 0x0000000001,
            },
            WindowsProcessMessage {
                umsg: WM_IME_CHAR,
                wparam: 0xDC4C,
                lparam: 0x0000000001,
            },
            WindowsProcessMessage {
                umsg: WM_IME_NOTIFY,
                wparam: 0x010D,
                lparam: 0x0000000000,
            },
            WindowsProcessMessage {
                umsg: WM_IME_ENDCOMPOSITION,
                wparam: 0x0000,
                lparam: 0x0000000000,
            },
            WindowsProcessMessage {
                umsg: WM_IME_NOTIFY,
                wparam: 0x010E,
                lparam: 0x0000000000,
            },
            WindowsProcessMessage {
                umsg: WM_CHAR,
                wparam: 0xD83D,
                lparam: 0x0000000001,
            },
            WindowsProcessMessage {
                umsg: WM_CHAR,
                wparam: 0xDC4C,
                lparam: 0x0000000001,
            },
            WindowsProcessMessage {
                umsg: 0xC052,
                wparam: 0x0001,
                lparam: 0x643E50D570,
            }, // Unknown message
            WindowsProcessMessage {
                umsg: WM_IME_REQUEST,
                wparam: 0x0006,
                lparam: 0x643E50D570,
            },
        ];
    }

    #[derive(PartialEq, Eq)]
    enum KeyRepeat {
        Repeat,
        Initial,
    }

    impl KeystrokeFlags {
        fn test_key_down_flags(repeat: KeyRepeat) -> Self {
            Self {
                repeat_count: u16::from(repeat == KeyRepeat::Repeat),
                scan_code: 0x1E, // 'A'
                is_extended_key: false,
                is_alt_pressed: false,
                was_previous_state_down: repeat == KeyRepeat::Repeat,
                is_key_release: false,
            }
        }

        fn test_key_up_flags(repeat: KeyRepeat) -> Self {
            Self {
                repeat_count: u16::from(repeat == KeyRepeat::Repeat),
                scan_code: 0x1E, // 'A'
                is_extended_key: false,
                is_alt_pressed: false,
                was_previous_state_down: repeat == KeyRepeat::Initial,
                is_key_release: true,
            }
        }
    }

    /// A basic smoke test for key pressed events.
    #[test]
    fn test_key_pressed_basic() {
        let mut kbd = Keyboard::new();

        assert!(!kbd.is_key_pressed(KeyCode::Up));
        kbd.process_evt(KeyEvent::KeyDown {
            key_code: KeyCode::Up,
            flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
        });
        assert!(kbd.is_key_pressed(KeyCode::Up));
    }

    /// Tests correct handling of a series of key down and key up events.
    #[test]
    fn test_key_pressed() {
        let mut kbd = Keyboard::new();

        for key_code in KeyCode::iter() {
            assert!(!kbd.is_key_pressed(key_code));
        }

        for evt in [
            KeyEvent::KeyDown {
                key_code: KeyCode::A,
                flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
            },
            KeyEvent::KeyDown {
                key_code: KeyCode::Left,
                flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
            },
            KeyEvent::KeyDown {
                key_code: KeyCode::Space,
                flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
            },
            KeyEvent::KeyDown {
                key_code: KeyCode::Left,
                flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Repeat),
            },
            KeyEvent::KeyUp {
                key_code: KeyCode::A,
                flags: KeystrokeFlags::test_key_up_flags(KeyRepeat::Initial),
            },
            KeyEvent::KeyDown {
                key_code: KeyCode::Left,
                flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Repeat),
            },
        ] {
            kbd.process_evt(evt);
        }

        let expected_pressed = [KeyCode::Space, KeyCode::Left];
        for key_code in expected_pressed {
            assert!(kbd.is_key_pressed(key_code));
        }
        for key_code in KeyCode::iter().filter(|key_code| expected_pressed.contains(key_code).not())
        {
            assert!(!kbd.is_key_pressed(key_code));
        }
    }

    /// We expect that a basic stream of ASCII characters (less than the queue
    /// size), should be collected and returned correctly.
    #[test]
    fn test_input_queue_basic() {
        let mut kbd = Keyboard::new();

        // Test state before any events
        let input: String = kbd.drain_input().chars().collect();
        assert!(
            input.is_empty(),
            "Queue should be empty before first input key event event"
        );

        // Add basic ASCII chars to queue
        for evt in "Hello, world!".chars().map(|c| KeyEvent::Input {
            flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
            wchar: c as _,
        }) {
            kbd.process_evt(evt);
        }

        // Confirm queue state after events have been processed
        let input: String = kbd.drain_input().chars().collect();
        assert_eq!(&input, "Hello, world!");
        assert!(
            kbd.drain_input().chars().next().is_none(),
            "Queue should be empty after last call to drain"
        );
    }

    /// Test that valid unicode is handled correctly.
    ///
    /// We use a "Musical Symbol G Clef" character which requires surrogate
    /// pairs to encode in UTF16.
    #[test]
    fn test_input_queue_unicode() {
        let mut kbd = Keyboard::new();

        for evt in [0xD834_u16, 0xDD1E, 0x006d, 0x0075, 0x0073, 0x0069, 0x0063]
            .into_iter()
            .map(|wchar| KeyEvent::Input {
                flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
                wchar,
            })
        {
            kbd.process_evt(evt);
        }

        let input: String = kbd.drain_input().chars().collect();
        assert_eq!(&input, "𝄞music");
    }

    /// Test pending surrogate pair handling by enqueueing the high surrogate
    /// and expecting that our drain method returns nothing until the following
    /// low surrogate is enqueued.
    ///
    /// We use a "Musical Symbol G Clef" character which requires surrogate
    /// pairs to encode in UTF16.
    #[test]
    fn test_input_queue_surrogate_pair_handling() {
        let mut kbd = Keyboard::new();

        kbd.process_evt(KeyEvent::Input {
            flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
            wchar: 0xD834,
        });
        assert!(
            kbd.drain_input().chars().next().is_none(),
            "Input queue should wait for following low surrogate before returning"
        );

        kbd.process_evt(KeyEvent::Input {
            flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
            wchar: 0xDD1E,
        });

        let input: String = kbd.drain_input().chars().collect();
        assert_eq!(&input, "𝄞");
    }

    /// Test pending surrogate pair handling by enqueueing an out-of-order low
    /// surrogate (high surrogates must precede low surrogates)
    /// and expecting that our drain method immediately returns the replacement
    /// character.
    ///
    /// We use a "Musical Symbol G Clef" character which requires surrogate
    /// pairs to encode in UTF16.
    #[test]
    fn test_input_queue_lone_low_surrogate() {
        let mut kbd = Keyboard::new();

        for evt in [0xDD1E, 0x006d].into_iter().map(|wchar| KeyEvent::Input {
            flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
            wchar,
        }) {
            kbd.process_evt(evt);
        }

        let input: String = kbd.drain_input().chars().collect();
        assert_eq!(&input, "�m");
    }

    // Test that several unicode characters requiring surrogate pairs are correctly
    // captured.
    ///
    /// We use alternating "Musical Symbol G Clef" and "Bridge at Night Emoji"
    /// characters which both require surrogate pairs to encode in UTF16.
    #[test]
    fn test_input_queue_multiple_surrogate_pair_characters() {
        let mut kbd = Keyboard::new();

        for evt in u16str!("𝄞🌉𝄞🌉a𝄞b🌉c")
            .as_slice()
            .iter()
            .map(|c| KeyEvent::Input {
                flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
                wchar: *c as _,
            })
        {
            kbd.process_evt(evt);
        }

        // Confirm queue state after events have been processed
        let input: String = kbd.drain_input().chars().collect();
        assert_eq!(&input, "𝄞🌉𝄞🌉a𝄞b🌉c");

        assert!(
            kbd.drain_input().chars().next().is_none(),
            "Queue should be empty after last call to drain"
        );
    }

    /// Tests that our input buffer is trimmed to avoid continuous growth if it
    /// is not regularly drained by the caller.
    #[test]
    fn test_input_queue_buffer_trim() {
        let mut kbd = Keyboard::new();

        // Add basic ASCII chars to queue
        for evt in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
            .chars()
            .map(|c| KeyEvent::Input {
                flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
                wchar: c as _,
            })
        {
            kbd.process_evt(evt);
        }

        // Confirm queue state after events have been processed
        let input: String = kbd.drain_input().chars().collect();
        assert_eq!(&input, "vwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ");
        assert_eq!(input.len(), INPUT_QUEUE_CAPACITY - 1);

        assert!(
            kbd.drain_input().chars().next().is_none(),
            "Queue should be empty after last call to drain"
        );
    }

    // Test that buffer trimming does not result in surrogate pair truncation.
    // If the first character to be truncated is a high surrogate pair
    // character, then the following low surrogate pair character should be
    // trimmed too.
    ///
    /// We use alternating "Musical Symbol G Clef" and "Bridge at Night Emoji"
    /// characters which both require surrogate pairs to encode in UTF16.
    #[test]
    fn test_input_queue_buffer_trim_unicode() {
        let mut kbd = Keyboard::new();

        for evt in u16str!("𝄞🌉1𝄞🌉2𝄞🌉3𝄞🌉4𝄞🌉5𝄞🌉6𝄞🌉7𝄞🌉8𝄞🌉9𝄞🌉0𝄞🌉A𝄞🌉B𝄞🌉C𝄞🌉")
            .as_slice()
            .iter()
            .map(|c| KeyEvent::Input {
                flags: KeystrokeFlags::test_key_down_flags(KeyRepeat::Initial),
                wchar: *c as _,
            })
        {
            kbd.process_evt(evt);
        }

        // Confirm queue state after events have been processed
        let input: String = kbd.drain_input().chars().collect();
        assert_eq!(&input, "🌉4𝄞🌉5𝄞🌉6𝄞🌉7𝄞🌉8𝄞🌉9𝄞🌉0𝄞🌉A𝄞🌉B𝄞🌉C𝄞🌉");
        assert_eq!(input.chars().count(), INPUT_QUEUE_CAPACITY - 1);

        assert!(
            kbd.drain_input().chars().next().is_none(),
            "Queue should be empty after last call to drain"
        );
    }

    /// Test text entry for 'ö' ('"' + 'o' combo on international keyboard).
    ///
    /// Events were captured via debugging utils.
    #[test]
    fn test_input_queue_international_input() {
        let mut kbd = Keyboard::new();

        for &msg in event_samples::PRESS_RELEASE_INTERNATIONAL_UMLAUT {
            if let Some(evt) = KeyEvent::new(msg) {
                kbd.process_evt(evt);
            }
        }

        let input: String = kbd.drain_input().chars().collect();
        assert_eq!(input, "ö");
        for key_code in KeyCode::iter() {
            assert!(
                !kbd.is_key_pressed(key_code),
                "{key_code:?} key still pressed"
            );
        }
    }

    /// Test emoji input for 👌 (using emoji keyboard: "win-.")
    ///
    /// Events captured using debug utils.
    #[test]
    fn test_input_queue_emoji() {
        let mut kbd = Keyboard::new();

        for &msg in event_samples::EMOJI_INPUT_OK_HAND {
            if let Some(evt) = KeyEvent::new(msg) {
                println!("{evt:#?}");
                kbd.process_evt(evt);
            }
        }

        let input: String = kbd.drain_input().chars().collect();
        assert_eq!(input, "👌");
        for key_code in KeyCode::iter() {
            assert!(
                !kbd.is_key_pressed(key_code),
                "{key_code:?} key still pressed"
            );
        }
    }

    /// Pressing backspace without any input in the queue should accumulate
    /// pending delete backspace events that can be applied to previously
    /// drained characters. If backspace is pressed while the input queue has
    /// some input should result in pending input being removed.
    #[test]
    fn test_backspace_key() {
        let mut kbd = Keyboard::new();

        for &msg in [
            event_samples::PRESS_RELEASE_BACKSPACE,
            event_samples::PRESS_RELEASE_BACKSPACE,
            event_samples::PRESS_RELEASE_A,
            event_samples::PRESS_RELEASE_B,
            event_samples::PRESS_RELEASE_BACKSPACE,
            event_samples::PRESS_RELEASE_C,
        ]
        .into_iter()
        .flatten()
        {
            if let Some(evt) = KeyEvent::new(msg) {
                kbd.process_evt(evt);
            }
        }

        let mut state = kbd.drain_input();
        assert_eq!(state.num_backspaces(), 2);
        let input: String = state.chars().collect();
        assert_eq!(input, "ac");
    }
    // TODO: delete key
}
