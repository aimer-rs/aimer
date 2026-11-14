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

/// Which edge or corner of the window a resize drags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEdge {
    /// The top edge.
    North,
    /// The bottom edge.
    South,
    /// The right edge.
    East,
    /// The left edge.
    West,
    /// The top-right corner.
    NorthEast,
    /// The top-left corner.
    NorthWest,
    /// The bottom-right corner.
    SouthEast,
    /// The bottom-left corner.
    SouthWest,
}

impl WindowEdge {
    /// Every edge and corner, with the short name the wire format writes first
    /// and the long one it also reads.
    pub(crate) const ALL: [(&'static str, &'static str, Self); 8] = [
        ("n", "north", Self::North),
        ("s", "south", Self::South),
        ("e", "east", Self::East),
        ("w", "west", Self::West),
        ("ne", "north-east", Self::NorthEast),
        ("nw", "north-west", Self::NorthWest),
        ("se", "south-east", Self::SouthEast),
        ("sw", "south-west", Self::SouthWest),
    ];

    /// The short name the wire format writes: `n`, `s`, `e`, `w`, `ne`, `nw`,
    /// `se` or `sw`.
    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(_, _, edge)| *edge == self).map_or("", |(short, ..)| short)
    }

    /// Whether the pointer's horizontal travel moves this edge.
    #[inline]
    pub const fn moves_x(self) -> bool {
        !matches!(self, Self::North | Self::South)
    }

    /// Whether the pointer's vertical travel moves this edge.
    #[inline]
    pub const fn moves_y(self) -> bool {
        !matches!(self, Self::East | Self::West)
    }
}

/// A change of the window's state, as its title-bar buttons make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    /// Fills the screen's work area.
    Maximize,
    /// Leaves the maximized state, back to the size it had.
    Unmaximize,
    /// Hides the window in the dock or taskbar.
    Minimize,
    /// Brings a minimized window back.
    Unminimize,
}

impl WindowAction {
    /// Every action with the name the wire format writes.
    pub(crate) const ALL: [(&'static str, Self); 4] = [
        ("maximize", Self::Maximize),
        ("unmaximize", Self::Unmaximize),
        ("minimize", Self::Minimize),
        ("unminimize", Self::Unminimize),
    ];

    /// The name the wire format writes: `maximize`, `unmaximize`, `minimize` or
    /// `unminimize`.
    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(_, action)| *action == self).map_or("", |(name, _)| name)
    }
}
