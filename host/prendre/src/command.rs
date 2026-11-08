use std::path::PathBuf;

use crate::model::{KeyAction, Modifiers, NamedKey, PointerButton, ScrollDeltaKind, TouchPhase};

/// An editing shortcut the application menu offers, delivered without a menu.
///
/// On macOS the menu bar claims `Cmd`+`C` before the window sees it and hands it
/// back as a [`MenuShortcut`]; this is that hand-back, so a driver can copy,
/// paste or undo whatever has focus on any platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditShortcut {
    /// `Cmd`/`Ctrl`+`Z`.
    Undo,
    /// `Cmd`/`Ctrl`+`Shift`+`Z`.
    Redo,
    /// `Cmd`/`Ctrl`+`X`.
    Cut,
    /// `Cmd`/`Ctrl`+`C`.
    Copy,
    /// `Cmd`/`Ctrl`+`V`.
    Paste,
    /// `Cmd`/`Ctrl`+`A`.
    SelectAll,
}

impl EditShortcut {
    pub(crate) const ALL: [(&'static str, Self); 6] = [
        ("undo", Self::Undo),
        ("redo", Self::Redo),
        ("cut", Self::Cut),
        ("copy", Self::Copy),
        ("paste", Self::Paste),
        ("select-all", Self::SelectAll),
    ];
}

/// One step of the system's back.
///
/// Android's predictive back reports a gesture in pieces: it starts, may
/// progress, and then is invoked or cancelled. A plain back is just
/// [`Invoked`](Self::Invoked).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackStep {
    /// The user started the back gesture.
    Started,
    /// How far the gesture has gone, from `0.0` to `1.0`.
    Progressed(f32),
    /// The gesture completed, or a plain back was pressed.
    Invoked,
    /// The user let go without completing the gesture.
    Cancelled,
}

/// The longest a glide may take, in milliseconds. A client gives up waiting for
/// the reply to a command after five seconds.
pub const MAX_GLIDE_MILLIS: u32 = 3000;

/// One synthetic input event that can be sent to a running application.
///
/// Positions and deltas are in **logical pixels** in window coordinates, the
/// same space the platform bridges report in. Every float in a command that
/// came from [`Command::parse`](crate::Command::parse) is finite.
///
/// Every event a person or the system can cause from outside the process has a
/// command. What is left out is not input: focus changes, the drag-over events
/// between in-app widgets, and the revisioned mirror of a native text editor
/// are all produced by the framework in response to input, and the frame,
/// render and window wake-ups are its own bookkeeping.
///
/// # Examples
///
/// ```
/// use aimer_prendre::Command;
///
/// let click: Vec<Command> = ["down 10 20", "up 10 20"]
///     .iter()
///     .map(|line| line.parse().expect("valid command"))
///     .collect();
/// assert_eq!(click.len(), 2);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// The mouse moved to `(x, y)` while `modifiers` were held.
    PointerMove { x: f32, y: f32, modifiers: Modifiers },
    /// `button` went down at `(x, y)` while `modifiers` were held.
    PointerDown { x: f32, y: f32, button: PointerButton, modifiers: Modifiers },
    /// `button` came up at `(x, y)` while `modifiers` were held.
    PointerUp { x: f32, y: f32, button: PointerButton, modifiers: Modifiers },
    /// The mouse left the window.
    PointerLeave,
    /// The mouse glides to `(x, y)` instead of appearing there, and the command
    /// replies at once while it is still on its way.
    ///
    /// The server steers the injected mouse toward the target as ordinary moves,
    /// easing out of the start and into the end, so whatever lies between sees it
    /// pass — a hover starts and ends — and a viewer can follow it. A second
    /// glide sent mid-flight retargets it: the mouse keeps its velocity and turns
    /// toward the new target at a limited rate, in an arc, as a hand does.
    /// `millis` is about how long a glide takes, from 1 to [`MAX_GLIDE_MILLIS`];
    /// without it the time follows the distance. With nowhere to glide from,
    /// because nothing has been injected yet, it is a single move. Handed to
    /// [`Command::deliver`] directly, which has no time to spend, it is that
    /// single move too.
    PointerGlide { x: f32, y: f32, millis: Option<u32>, modifiers: Modifiers },
    /// Waits until the mouse has finished gliding, then replies.
    ///
    /// A `glide` replies at once and the mouse is still on its way, so a command
    /// that acts *at* the mouse — a scroll — sends this first. `move`, `down` and
    /// `up` carry a position, and wait for the mouse by themselves. Replies with
    /// an error if the mouse has not arrived within a few seconds.
    Settle,
    /// A scroll by `(dx, dy)`: pixels, or wheel steps for [`ScrollDeltaKind::Line`].
    ///
    /// `phase` is where the scroll is in its gesture, as a trackpad reports it;
    /// a plain wheel only ever moves.
    Scroll {
        dx: f32,
        dy: f32,
        kind: ScrollDeltaKind,
        phase: TouchPhase,
        modifiers: Modifiers,
    },
    /// A finger `id` touched down, moved, or lifted at `(x, y)`.
    ///
    /// `phase` is never [`TouchPhase::Cancelled`]; that is [`Command::TouchCancel`].
    Touch { phase: TouchPhase, id: u64, x: f32, y: f32 },
    /// The system took the touch away; the framework cancels every press.
    TouchCancel,
    /// A pinch anchored at `(x, y)` that changed the scale by `delta`, as a
    /// fraction: `0.1` grows the content by 10 %.
    Pinch { x: f32, y: f32, delta: f32, phase: TouchPhase },
    /// A rotation anchored at `(x, y)` by `delta` radians, clockwise positive.
    Rotate { x: f32, y: f32, delta: f32, phase: TouchPhase },
    /// A multi-finger pan anchored at `(x, y)` that moved by `(dx, dy)`.
    Pan { x: f32, y: f32, dx: f32, dy: f32, phase: TouchPhase },
    /// A trackpad smart zoom (two-finger double tap) at `(x, y)`.
    SmartZoom { x: f32, y: f32 },
    /// A named key changed state.
    Key {
        key: NamedKey,
        action: KeyAction,
        modifiers: Modifiers,
    },
    /// Text committed as one edit, exactly like an IME commit.
    Text(String),
    /// Text an input method is still composing; empty text ends the composition.
    ///
    /// `cursor` is the byte range of the active segment inside `text`.
    Preedit { text: String, cursor: Option<(usize, usize)> },
    /// The soft keyboard's backspace, which reaches the focused field without
    /// being a key event.
    SoftBackspace,
    /// A step of the system's back: Android's predictive back, or a plain back.
    Back(BackStep),
    /// The app was opened with a link, such as a deep link or a universal link.
    Link(String),
    /// An editing shortcut handed back from the menu bar.
    Menu(EditShortcut),
    /// A file drag entered the window carrying this path.
    FileHover(PathBuf),
    /// The file drag left the window without dropping.
    FileHoverCancel,
    /// The file drag was released over the window.
    FileDrop(PathBuf),
    /// The process was asked to quit, as `SIGTERM` would: the loop exits in an
    /// orderly way and the app's services are told.
    Quit,
}
