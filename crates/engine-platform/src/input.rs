//! Input event type definitions.
//!
//! This is the foundation slice: the *vocabulary* of input — the types game
//! code and the editor will pattern-match on. Actual device polling is part of
//! the windowing layer and lands with the renderer in a later phase. Defining
//! the types now lets upper layers compile against a stable input surface.

/// A physical keyboard key, identified by US-QWERTY position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Key {
    /// A letter key `A`–`Z`.
    Letter(char),
    /// A digit key `0`–`9` on the number row.
    Digit(u8),
    /// A function key `F1`–`F24`.
    Function(u8),
    /// The space bar.
    Space,
    /// The return / enter key.
    Enter,
    /// The escape key.
    Escape,
    /// The tab key.
    Tab,
    /// The backspace key.
    Backspace,
    /// The delete key.
    Delete,
    /// Left arrow.
    ArrowLeft,
    /// Right arrow.
    ArrowRight,
    /// Up arrow.
    ArrowUp,
    /// Down arrow.
    ArrowDown,
    /// Left or right shift.
    Shift,
    /// Left or right control.
    Control,
    /// Left or right alt / option.
    Alt,
    /// Left or right super / meta / command.
    Super,
    /// A key not otherwise named, identified by its raw OS scancode.
    Unknown(u32),
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// Primary (usually left) button.
    Left,
    /// Secondary (usually right) button.
    Right,
    /// Middle button / wheel click.
    Middle,
    /// An extra button, identified by index.
    Other(u8),
}

/// Whether a button transitioned down or up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    /// The button was pressed.
    Pressed,
    /// The button was released.
    Released,
}

/// The set of modifier keys held when an event was produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// A shift key is held.
    pub shift: bool,
    /// A control key is held.
    pub control: bool,
    /// An alt key is held.
    pub alt: bool,
    /// A super / meta / command key is held.
    pub super_key: bool,
}

/// A single input event delivered to the engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    /// A key transitioned to the down state. `repeat` is set for auto-repeat.
    KeyDown {
        /// The key.
        key: Key,
        /// Modifier keys held.
        mods: Modifiers,
        /// `true` if this is an auto-repeat rather than a fresh press.
        repeat: bool,
    },
    /// A key transitioned to the up state.
    KeyUp {
        /// The key.
        key: Key,
        /// Modifier keys held.
        mods: Modifiers,
    },
    /// A committed text character (after IME / dead-key composition).
    Text(char),
    /// The pointer moved. Coordinates are in logical pixels.
    MouseMoved {
        /// Absolute X position.
        x: f64,
        /// Absolute Y position.
        y: f64,
        /// X delta since the previous move.
        dx: f64,
        /// Y delta since the previous move.
        dy: f64,
    },
    /// A mouse button changed state.
    MouseButton {
        /// Which button.
        button: MouseButton,
        /// Pressed or released.
        state: ButtonState,
    },
    /// The scroll wheel or trackpad scrolled.
    Scroll {
        /// Horizontal scroll amount.
        dx: f32,
        /// Vertical scroll amount.
        dy: f32,
    },
    /// The window gained (`true`) or lost (`false`) focus.
    Focus(bool),
    /// The drawable surface was resized, in physical pixels.
    Resized {
        /// New width.
        width: u32,
        /// New height.
        height: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_matchable() {
        let ev = InputEvent::KeyDown {
            key: Key::Letter('W'),
            mods: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            repeat: false,
        };
        match ev {
            InputEvent::KeyDown { key, mods, .. } => {
                assert_eq!(key, Key::Letter('W'));
                assert!(mods.shift && !mods.control);
            }
            _ => panic!("wrong variant"),
        }
    }
}
