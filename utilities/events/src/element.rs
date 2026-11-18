use std::path::PathBuf;
use std::sync::Arc;

use aimer_attribute::position::Vec2d;
pub use winit::event::TouchPhase;

use crate::pointer::{PointerInfo, PointerSource};
use crate::text_editing::TextEditingDelta;

/// Key actions for keyboard events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Pressed,
    Released,
    Repeat,
}

/// Modifier key state carried with keyboard events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

aimer_tls::thread_local! {
    static CURRENT_MODIFIERS: std::cell::Cell<Modifiers> = const {
        std::cell::Cell::new(Modifiers { ctrl: false, shift: false, alt: false, meta: false })
    };
}

impl Modifiers {
    /// The modifier keys that are held now, as the application's event loop last saw them.
    ///
    /// Keyboard events carry their own modifiers, but a pointer event does not, and some pointer
    /// gestures depend on them: a click that extends a selection when `shift` is held, say. The
    /// state is per thread, which is the thread the event loop and the widgets share. It is the
    /// default (nothing held) until the loop reports a change, and again when the window loses
    /// focus.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_events::element::Modifiers;
    ///
    /// assert!(!Modifiers::current().shift);
    /// Modifiers::set_current(Modifiers { shift: true, ..Modifiers::default() });
    /// assert!(Modifiers::current().shift);
    /// Modifiers::set_current(Modifiers::default());
    /// ```
    #[inline]
    pub fn current() -> Self {
        CURRENT_MODIFIERS.with(std::cell::Cell::get)
    }

    /// Records the modifier keys that are held now. The event loop calls this when they change;
    /// a test that drives a widget with a held key calls it too.
    #[inline]
    pub fn set_current(modifiers: Self) {
        CURRENT_MODIFIERS.with(|current| current.set(modifiers));
    }
}

/// Named (non-text) keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamedKey {
    Backspace,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Escape,
    Tab,
    Other(String),
}

/// Identifies whether a scroll delta came from a stepped wheel or a
/// pixel-precise input device such as a trackpad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDeltaKind {
    /// A discrete line-based mouse-wheel delta.
    Line,
    /// A pixel-precise trackpad or smooth-wheel delta.
    Pixel,
}

/// How far a press on a pressure-sensitive trackpad has gone: the stages of a macOS Force Touch trackpad, which
/// reports one as the finger presses harder.
///
/// The stages are in order, so `stage >= PressureStage::Click` says that the pad is pressed at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PressureStage {
    /// No press: the finger is off the pad, or has not pressed it down.
    None,
    /// An ordinary click.
    Click,
    /// A deep press, the force click that a trackpad that is pressed hard gives.
    Force,
}

impl PressureStage {
    /// The stage a platform number stands for: `0` is no press, `1` a click and `2` a force click. A number that is not one
    /// of them is the nearest, which is none for what is below `0` and force for what is above `2`.
    #[inline]
    pub const fn from_platform(stage: i64) -> Self {
        match stage {
            i64::MIN..=0 => Self::None,
            1 => Self::Click,
            _ => Self::Force,
        }
    }
}

/// Pointer and keyboard event types for dispatch.
#[derive(Clone, Debug)]
pub enum ElementEvent {
    /// The element became the authoritative keyboard and input-method target.
    FocusGained,
    /// The element stopped being the authoritative keyboard and input-method target.
    FocusLost,
    /// A button went down, or a finger touched the surface.
    ///
    /// [`PointerInfo`] carries the position, the device, the pointer id and the
    /// button, so a secondary or middle press is this same variant rather than
    /// one of its own.
    PointerDown(PointerInfo),
    /// A button came up, or a finger left the surface.
    PointerUp(PointerInfo),
    /// The pointer moved. [`PointerInfo::button`] reports the button being held,
    /// or [`crate::pointer::PointerButton::Primary`] for a plain hover.
    PointerMove(PointerInfo),
    /// Pointer left the window. The `u64` is the pointer ID (0 for mouse).
    PointerExited(PointerSource, u64),
    Scroll {
        delta: Vec2d,
        phase: TouchPhase,
        kind: ScrollDeltaKind,
        /// Whether the user is still physically controlling the scroll.
        ///
        /// Trackpad contact sets this to `true` until finger lift. Native
        /// momentum events emitted after lift retain their scroll phase but set
        /// this to `false`. Mouse-wheel input is never a direct manipulation.
        is_direct_manipulation: bool,
    },
    /// A pinch recognized by the platform rather than from raw contacts: the
    /// trackpad's magnify gesture on macOS, or the emulated zoom of
    /// `ctrl` + wheel where the platform has no such gesture.
    ///
    /// Like [`Self::Scroll`] it is routed by hit test at `pos`, the cursor.
    Pinch {
        /// Where the gesture is anchored, in logical window coordinates.
        pos: Vec2d,
        /// The fractional change in scale since the previous event: `0.1` grows
        /// the content by 10 %, `-0.1` shrinks it by 10 %. Never `NaN`.
        delta: f32,
        phase: TouchPhase,
    },
    /// A rotation recognized by the platform, such as the trackpad's two-finger
    /// twist on macOS.
    Rotate {
        /// Where the gesture is anchored, in logical window coordinates.
        pos: Vec2d,
        /// The change since the previous event, in radians. Positive is
        /// **clockwise** on screen — the direction of increasing angle in a
        /// y-down coordinate system — which is the opposite of winit's
        /// counterclockwise convention; the platform layer flips it.
        delta: f32,
        phase: TouchPhase,
    },
    /// A multi-finger pan recognized by the platform, such as an N-finger pan
    /// on iOS.
    Pan {
        /// Where the gesture is anchored, in logical window coordinates.
        pos: Vec2d,
        /// The change since the previous event, in logical pixels.
        delta: Vec2d,
        phase: TouchPhase,
    },
    /// The platform's smart-zoom gesture: a two-finger double tap on a macOS
    /// trackpad, conventionally toggling between fitted and actual size.
    SmartZoom {
        /// Where the gesture is anchored, in logical window coordinates.
        pos: Vec2d,
    },
    /// The pressure of a press on a pressure-sensitive trackpad, which a macOS Force Touch trackpad reports as the finger
    /// presses harder and lets go.
    ///
    /// Like [`Self::Pinch`] it is routed by hit test at `pos`, the cursor: the platform gives it no position of its own.
    /// It follows the press all the way: one event for every change of the pressure, and one with
    /// [`PressureStage::None`] when it is let go. A widget that wants the force click waits for the stage to *become*
    /// [`PressureStage::Force`], which is once for each deep press.
    Pressure {
        /// Where the cursor is, in logical window coordinates.
        pos: Vec2d,
        /// How hard the press is, from `0.0` (none) to `1.0` and no more. Never `NaN`.
        pressure: f32,
        stage: PressureStage,
    },
    /// A character was typed (text input).
    CharInput {
        ch: char,
        action: KeyAction,
        modifiers: Modifiers,
    },

    /// A multi-character text payload was produced by the platform, such as an
    /// IME commit of a composed phrase or a dead-key sequence.
    ///
    /// The whole payload travels in one event so a receiver inserts it as a
    /// single edit: one undo entry, one change notification, and one cursor
    /// advance. Single typed characters keep arriving as [`Self::CharInput`].
    TextInput {
        text: String,
        action: KeyAction,
        modifiers: Modifiers,
    },

    /// A named key was pressed or released.
    KeyInput {
        key: NamedKey,
        action: KeyAction,
        modifiers: Modifiers,
    },
    /// IME pre-edit (composition in progress). `text` is the composing string.
    /// `cursor` is the byte range of the active composing segment.
    ImePreedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// A revisioned mutation from the active mirrored native editor.
    TextEditingDelta(TextEditingDelta),
    /// A file dragged in from the operating system is hovering over the window.
    ///
    /// One event per file: a five-file drag reports five hovers, with no marker
    /// saying the batch has ended.
    ///
    /// `pos` is where the cursor was when the platform reported the drag, in
    /// logical window coordinates, or `None` where the platform will not say.
    /// winit attaches no position to this event, and macOS sends no cursor
    /// motion at all during a drag session, so the position is resolved by the
    /// windowing layer rather than carried by winit.
    ///
    /// winit's web backend never emits this event, so an application built for
    /// the browser will not see file drags at all.
    HoveredFile {
        path: PathBuf,
        pos: Option<Vec2d>,
    },

    /// The file drag already over the window has moved to `pos`.
    ///
    /// [`Self::HoveredFile`] is announced once, when the drag crosses into the
    /// window, and the platform then says nothing more until it is dropped or
    /// leaves — while the user goes on moving it. This is the framework's own
    /// continuation of that drag, produced by the windowing layer for every
    /// position the drag is found at, so a region can light up as the files
    /// reach it rather than only where they came in.
    ///
    /// It carries the whole batch, unlike the one-file-per-event hovers it
    /// follows: a drag of any size moves as one thing, and paying per file for
    /// every move of a hundred-file drag would be absurd. The paths are shared
    /// rather than copied for the same reason.
    HoveredFileMoved {
        paths: Arc<[PathBuf]>,
        pos: Vec2d,
    },

    /// A file dragged in from the operating system was released over the
    /// window. One event per file; see [`Self::HoveredFile`] for `pos`.
    DroppedFile {
        path: PathBuf,
        pos: Option<Vec2d>,
    },

    /// The file drag left the window without being dropped.
    HoveredFileCancelled,

    /// A drag is passing over the element, at `pos`.
    ///
    /// Unlike [`Self::PointerMove`], this is delivered by hit test to the
    /// element *under* the pointer even while another element owns the pointer,
    /// which is what lets the widget being dragged keep the capture while the
    /// widget being dragged *onto* still hears about it. The pointer is carried
    /// loose as `(source, id)` because the key type that pairs them lives one
    /// crate above this one.
    DragOver {
        pos: Vec2d,
        source: PointerSource,
        id: u64,
    },

    /// The drag that was over the element is over it no longer.
    ///
    /// It carries no position: by the time an element learns it was left, the
    /// pointer is somewhere else entirely, and the only useful answer is "not
    /// here".
    DragLeave {
        source: PointerSource,
        id: u64,
    },

    /// A drag was released at `pos`, over the element receiving this.
    DragDrop {
        pos: Vec2d,
        source: PointerSource,
        id: u64,
    },

    Cancel,
}

impl ElementEvent {
    /// The pointer this event is about, for the three variants that have one.
    ///
    /// Saves every consumer from re-matching the down/up/move triple just to
    /// reach a position, a pointer id or a button.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_attribute::position::Vec2d;
    /// use aimer_events::element::ElementEvent;
    /// use aimer_events::pointer::{PointerButton, PointerInfo};
    ///
    /// let press = PointerInfo::mouse(Vec2d { x: 3.0, y: 4.0 }, PointerButton::Secondary);
    /// let event = ElementEvent::PointerDown(press);
    ///
    /// assert_eq!(event.pointer().map(|p| p.button), Some(PointerButton::Secondary));
    /// assert!(ElementEvent::Cancel.pointer().is_none());
    /// ```
    #[inline]
    pub fn pointer(&self) -> Option<&PointerInfo> {
        match self {
            ElementEvent::PointerDown(pointer)
            | ElementEvent::PointerUp(pointer)
            | ElementEvent::PointerMove(pointer) => Some(pointer),
            _ => None,
        }
    }

    pub fn get_pointer_pos(&self) -> Option<Vec2d> {
        match self {
            ElementEvent::PointerDown(pointer)
            | ElementEvent::PointerUp(pointer)
            | ElementEvent::PointerMove(pointer) => Some(pointer.pos),
            ElementEvent::DragOver { pos, .. } | ElementEvent::DragDrop { pos, .. } => Some(*pos),
            ElementEvent::HoveredFileMoved { pos, .. } => Some(*pos),
            ElementEvent::HoveredFile { pos, .. } | ElementEvent::DroppedFile { pos, .. } => *pos,
            _ => None,
        }
    }

    /// Returns whether the event belongs to the focused element instead of the
    /// element under the pointer.
    ///
    /// Text and composition events are produced by the keyboard and the input
    /// method, which know nothing about the pointer, so they must reach the
    /// focused element even while the pointer rests over another widget or has
    /// left the window. Named keys stay positional: scrollables scroll the
    /// hovered viewport and text spans resolve shortcuts against the hovered
    /// span.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
    ///
    /// let commit = ElementEvent::TextInput {
    ///     text: "你好".into(),
    ///     action: KeyAction::Pressed,
    ///     modifiers: Modifiers::default(),
    /// };
    /// let arrow = ElementEvent::KeyInput {
    ///     key: NamedKey::ArrowDown,
    ///     action: KeyAction::Pressed,
    ///     modifiers: Modifiers::default(),
    /// };
    ///
    /// assert!(commit.is_focus_directed());
    /// assert!(!arrow.is_focus_directed());
    /// ```
    pub fn is_focus_directed(&self) -> bool {
        matches!(
            self,
            ElementEvent::CharInput { .. }
                | ElementEvent::TextInput { .. }
                | ElementEvent::ImePreedit { .. }
                | ElementEvent::TextEditingDelta(_)
        )
    }
}

unsafe impl Send for ElementEvent {}
unsafe impl Sync for ElementEvent {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stages_of_a_press_are_the_ones_a_trackpad_reports() {
        assert_eq!(PressureStage::from_platform(0), PressureStage::None);
        assert_eq!(PressureStage::from_platform(1), PressureStage::Click);
        assert_eq!(PressureStage::from_platform(2), PressureStage::Force);
    }

    #[test]
    fn a_stage_that_is_not_one_of_them_is_the_nearest() {
        assert_eq!(PressureStage::from_platform(-1), PressureStage::None);
        assert_eq!(PressureStage::from_platform(i64::MIN), PressureStage::None);
        assert_eq!(PressureStage::from_platform(3), PressureStage::Force);
        assert_eq!(PressureStage::from_platform(i64::MAX), PressureStage::Force);
    }

    #[test]
    fn the_stages_are_in_order_of_how_hard_the_press_is() {
        assert!(PressureStage::None < PressureStage::Click);
        assert!(PressureStage::Click < PressureStage::Force);
    }

    #[test]
    fn a_pressure_event_says_where_how_hard_and_what_stage() {
        let event = ElementEvent::Pressure { pos: Vec2d { x: 3.0, y: 4.0 }, pressure: 0.5, stage: PressureStage::Click };
        assert!(matches!(
            event,
            ElementEvent::Pressure { pos, pressure, stage: PressureStage::Click } if pos == (Vec2d { x: 3.0, y: 4.0 }) && pressure == 0.5
        ));
        // It goes to the element under the pointer, as a pinch does, and not to the one that has the focus.
        assert!(!event.is_focus_directed());
    }
}
