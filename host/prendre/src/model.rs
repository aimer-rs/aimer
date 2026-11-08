//! The small value types a [`Command`](crate::Command) is made of.
//!
//! They mirror the input types of `aimer_events` one for one, but live here so a
//! driver that only sends commands compiles none of the framework — not even
//! `winit`. The `host` feature converts them into the framework's types with
//! exhaustive matches, so a variant added on either side is a compile error
//! there rather than a command that quietly means something else.

/// Which modifier keys were held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// `Control`.
    pub ctrl: bool,
    /// `Shift`.
    pub shift: bool,
    /// `Alt`, or `Option` on macOS.
    pub alt: bool,
    /// `Command` on macOS, the Windows key elsewhere.
    pub meta: bool,
}

/// A key that is named rather than typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamedKey {
    /// `Backspace`.
    Backspace,
    /// `Delete`.
    Delete,
    /// The up arrow.
    ArrowUp,
    /// The down arrow.
    ArrowDown,
    /// The left arrow.
    ArrowLeft,
    /// The right arrow.
    ArrowRight,
    /// `Page Up`.
    PageUp,
    /// `Page Down`.
    PageDown,
    /// `Home`.
    Home,
    /// `End`.
    End,
    /// `Enter`.
    Enter,
    /// `Escape`.
    Escape,
    /// `Tab`.
    Tab,
    /// Any other key, by the name the application knows it as, such as a
    /// shortcut letter.
    Other(String),
}

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// The key went down.
    Pressed,
    /// The key came up.
    Released,
    /// The key is held and the system is repeating it.
    Repeat,
}

/// Which mouse button an event is about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerButton {
    /// The main button: left on a right-handed mouse.
    #[default]
    Primary,
    /// The context-menu button: right on a right-handed mouse.
    Secondary,
    /// The wheel button.
    Middle,
    /// A device-specific extra button, by its code.
    Other(u16),
}

/// Whether a scroll delta counts pixels or wheel steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDeltaKind {
    /// A pixel-precise delta, as a trackpad reports.
    Pixel,
    /// A number of wheel steps.
    Line,
}

/// Where a touch, scroll or gesture is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    /// It began.
    Started,
    /// It continued.
    Moved,
    /// It finished.
    Ended,
    /// The system abandoned it.
    Cancelled,
}
