use aimer_attribute::position::Vec2d;

use crate::element::TouchPhase;

/// The pointer identifier reserved for a drag coming from the operating system.
///
/// A file drag has no pointer of its own — there is no press to own it — but the
/// events that describe a drag are keyed by one, so file drags are filed under
/// an identifier no real device can produce: a mouse has id `0` and a touch id
/// counts up from there, and neither will ever reach `u64::MAX`.
///
/// # Examples
///
/// ```
/// use aimer_events::pointer::FILE_DRAG_POINTER_ID;
///
/// assert_ne!(FILE_DRAG_POINTER_ID, 0);
/// ```
pub const FILE_DRAG_POINTER_ID: u64 = u64::MAX;

thread_local! {
    static LAST_SOURCE: std::cell::Cell<PointerSource> = const { std::cell::Cell::new(PointerSource::Mouse) };
    static ROUTING_SCROLL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SCROLL_USED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Records whether the event now being routed is a scroll.
///
/// The dispatcher sets it as an event enters the tree and clears it when the
/// event has been delivered. A scroll that nothing used is absorbed by whichever
/// opaque element it reaches first, so an element that wants to *see* a scroll
/// before anything inside it can absorb it, as a navigator watching for a
/// trackpad swipe does, has to stand between the event and its children. Hit
/// testing is by position alone, so this is how it learns that this is the event
/// to stand in front of.
#[inline]
pub fn note_routing_scroll(routing: bool) {
    ROUTING_SCROLL.with(|flag| flag.set(routing));
}

/// Says that something moved because of the scroll now being routed.
///
/// A scroll that nothing used is absorbed by the first opaque element it meets,
/// and a scroll that a scroller used is consumed too, and a result that comes
/// back through a retained child does not say which. So a widget that actually
/// scrolled for an event says so here, and whoever stood in front of it, such as a
/// navigator watching for a trackpad swipe, reads it with [`take_scroll_used`].
#[inline]
pub fn note_scroll_used() {
    SCROLL_USED.with(|flag| flag.set(true));
}

/// Whether anything said it used the scroll since this was last asked, forgetting
/// the answer. See [`note_scroll_used`].
#[inline]
pub fn take_scroll_used() -> bool {
    SCROLL_USED.with(|flag| flag.replace(false))
}

/// Whether the event now being routed is a scroll; see [`note_routing_scroll`].
#[inline]
pub fn is_routing_scroll() -> bool {
    ROUTING_SCROLL.with(std::cell::Cell::get)
}

/// Records which kind of pointer produced the event now being routed.
///
/// The dispatcher calls this as an event enters the tree. Hit testing is by
/// position alone, so an element that has to treat a finger differently from a
/// cursor learns it here, before the event is routed.
#[inline]
pub fn note_pointer_source(source: PointerSource) {
    LAST_SOURCE.with(|last| last.set(source));
}

/// The kind of pointer that produced the most recent pointer event on this
/// thread: [`PointerSource::Mouse`] until a finger touches the window.
///
/// # Examples
///
/// ```
/// use aimer_events::pointer::{PointerSource, last_pointer_source, note_pointer_source};
///
/// note_pointer_source(PointerSource::Touch);
/// assert_eq!(last_pointer_source(), PointerSource::Touch);
/// ```
#[inline]
pub fn last_pointer_source() -> PointerSource {
    LAST_SOURCE.with(std::cell::Cell::get)
}

/// The smallest touch target, in logical pixels, a platform's guidelines ask for:
/// 44 on Apple platforms (Human Interface Guidelines) and 48 elsewhere
/// (Material Design).
pub const DEFAULT_MIN_TOUCH_TARGET: f32 = if cfg!(any(target_os = "ios", target_os = "macos")) {
    44.0
} else {
    48.0
};

thread_local! {
    static MIN_TOUCH_TARGET: std::cell::Cell<f32> = const { std::cell::Cell::new(DEFAULT_MIN_TOUCH_TARGET) };
}

/// The smallest size, in logical pixels, that an interactive control's hit area
/// grows to when it is touched.
///
/// A fingertip is far less precise than a cursor, so a small control keeps its
/// painted size but answers a touch within a rectangle at least this large,
/// centred on it. Mouse hit testing is never affected. Defaults to
/// [`DEFAULT_MIN_TOUCH_TARGET`]; `0` means the extra area is off.
///
/// The value is per thread, like the rest of the UI state, so read and set it on
/// the UI thread.
#[inline]
pub fn min_touch_target() -> f32 {
    MIN_TOUCH_TARGET.with(std::cell::Cell::get)
}

/// Sets the minimum touch target, in logical pixels. See [`min_touch_target`].
///
/// A negative or non-finite `size` is treated as `0`, which turns the extra
/// area off.
///
/// # Examples
///
/// ```
/// use aimer_events::pointer::{min_touch_target, set_min_touch_target};
///
/// set_min_touch_target(56.0);
/// assert_eq!(min_touch_target(), 56.0);
/// ```
#[inline]
pub fn set_min_touch_target(size: f32) {
    let size = if size.is_finite() && size > 0.0 { size } else { 0.0 };
    MIN_TOUCH_TARGET.with(|target| target.set(size));
}

/// Identifies the origin of a pointer event.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
#[repr(u8)]
pub enum PointerSource {
    Mouse = 0,
    Touch = 1,
}

/// Which button produced a pointer event.
///
/// Named by *role* rather than by side: an operating system configured for a
/// left-handed mouse swaps the physical buttons, so a recognizer that matched on
/// `Left` would have to special-case that setting, while one that matches on
/// [`Primary`] never notices. A touch contact is always [`Primary`].
///
/// [`Primary`]: PointerButton::Primary
///
/// # Examples
///
/// ```
/// use aimer_events::pointer::PointerButton;
///
/// // A tap handler that only reacts to the main button.
/// fn is_activation(button: PointerButton) -> bool {
///     button == PointerButton::Primary
/// }
///
/// assert!(is_activation(PointerButton::default()));
/// assert!(!is_activation(PointerButton::Secondary));
/// ```
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum PointerButton {
    /// The main button: left on a right-handed mouse, and every touch contact.
    #[default]
    Primary,
    /// The context-menu button: right on a right-handed mouse.
    Secondary,
    /// The wheel button, usually pressed for open-in-new-tab.
    Middle,
    /// A device-specific extra button, such as back/forward on a gaming mouse.
    ///
    /// Kept as an escape hatch so a fourth or fifth button reaches the
    /// application without this enum having to grow a variant per device.
    Other(u16),
}

/// Everything the platform knows about one pointer at one instant.
///
/// One type carries a pointer through the whole stack — the platform layer
/// builds it, [`crate::element::ElementEvent`] transports it, and the gesture
/// recognizers read it. It is bundled rather than spread across tuple fields
/// because a tuple variant cannot be extended: adding `button` to
/// `PointerDown(Vec2d, PointerSource, u64)` breaks every pattern that matches
/// it, and so would adding `pressure` later. With a struct, only the code that
/// wants the new field changes.
///
/// # Examples
///
/// ```
/// use aimer_attribute::position::Vec2d;
/// use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
///
/// let click = PointerInfo::mouse(Vec2d { x: 12.0, y: 8.0 }, PointerButton::Secondary);
///
/// assert_eq!(click.source, PointerSource::Mouse);
/// assert_eq!(click.id, 0);
/// assert_eq!(click.button, PointerButton::Secondary);
///
/// // A touch contact is always the primary button.
/// let touch = PointerInfo::touch(Vec2d { x: 4.0, y: 4.0 }, 3);
///
/// assert_eq!(touch.button, PointerButton::Primary);
/// assert_eq!(touch.id, 3);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerInfo {
    /// Position in logical pixels, in window coordinates.
    pub pos: Vec2d,
    /// The device class that produced the event.
    pub source: PointerSource,
    /// Source-local pointer identifier: `0` for the mouse, a finger index for
    /// touch.
    pub id: u64,
    /// The button this event is about.
    ///
    /// For a press or a release, the button that changed state. For a move, the
    /// button being held — or [`PointerButton::Primary`] when none is, since a
    /// hover has no button and callers filter on the press instead.
    pub button: PointerButton,
}

impl PointerInfo {
    /// Assembles a pointer from all four of its parts.
    #[inline]
    pub const fn new(pos: Vec2d, source: PointerSource, id: u64, button: PointerButton) -> Self {
        Self {
            pos,
            source,
            id,
            button,
        }
    }

    /// A mouse pointer, which always has id `0` because a machine has one
    /// cursor.
    #[inline]
    pub const fn mouse(pos: Vec2d, button: PointerButton) -> Self {
        Self::new(pos, PointerSource::Mouse, 0, button)
    }

    /// A touch contact, which is always the primary button.
    #[inline]
    pub const fn touch(pos: Vec2d, id: u64) -> Self {
        Self::new(pos, PointerSource::Touch, id, PointerButton::Primary)
    }

    /// The same pointer moved to `pos`, keeping its identity and button.
    ///
    /// Used when a recognizer needs a derived position — a pinch focal point,
    /// say — that belongs to the same pointer stream.
    #[inline]
    pub const fn at(self, pos: Vec2d) -> Self {
        Self { pos, ..self }
    }

    /// Horizontal position, in logical pixels.
    #[inline]
    pub const fn x(&self) -> f32 {
        self.pos.x
    }

    /// Vertical position, in logical pixels.
    #[inline]
    pub const fn y(&self) -> f32 {
        self.pos.y
    }

    /// Whether this event is about the main button.
    #[inline]
    pub const fn is_primary(&self) -> bool {
        matches!(self.button, PointerButton::Primary)
    }

    /// Distance to another pointer position, in logical pixels.
    #[inline]
    pub fn distance_to(&self, other: &Self) -> f32 {
        let dx = self.pos.x - other.pos.x;
        let dy = self.pos.y - other.pos.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// The point halfway between two pointers, reported as this pointer moved
    /// there.
    ///
    /// The identity of `self` is kept, so a pinch focal point stays attributable
    /// to a real pointer stream rather than to a synthetic one.
    #[inline]
    pub fn midpoint(&self, other: &Self) -> Self {
        self.at(Vec2d {
            x: (self.pos.x + other.pos.x) / 2.0,
            y: (self.pos.y + other.pos.y) / 2.0,
        })
    }
}

/// A pointer event as the gesture recognizers see it.
///
/// There is no separate secondary-click variant: the button travels on
/// [`PointerInfo`], so a right press is a `Down` like any other and a recognizer
/// that cares filters on [`PointerInfo::button`]. A variant per button would need
/// three more of them the moment the middle button mattered.
#[derive(Clone, Debug)]
pub enum PointerEvent {
    /// A button went down, or a finger touched the surface.
    Down(PointerInfo),
    /// A button came up, or a finger left the surface.
    Up(PointerInfo),
    /// The pointer moved.
    Move(PointerInfo),
    /// The gesture was interrupted by something other than the user — the app
    /// was backgrounded, or the window lost the pointer.
    Cancel,
    /// Scroll wheel or trackpad gesture.
    Scroll { delta_x: f32, delta_y: f32 },
    /// A platform pinch: the trackpad's magnify gesture, or a pinch the
    /// operating system recognized itself. See [`crate::element::ElementEvent::Pinch`].
    Pinch {
        pos: Vec2d,
        delta: f32,
        phase: TouchPhase,
    },
    /// A platform rotation, in radians, clockwise positive. See
    /// [`crate::element::ElementEvent::Rotate`].
    Rotate {
        pos: Vec2d,
        delta: f32,
        phase: TouchPhase,
    },
    /// A platform multi-finger pan. See [`crate::element::ElementEvent::Pan`].
    Pan {
        pos: Vec2d,
        delta_x: f32,
        delta_y: f32,
        phase: TouchPhase,
    },
    /// The platform's smart-zoom double tap. See
    /// [`crate::element::ElementEvent::SmartZoom`].
    SmartZoom { pos: Vec2d },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_touch_contact_is_always_the_primary_button() {
        let touch = PointerInfo::touch(Vec2d { x: 1.0, y: 2.0 }, 7);

        assert_eq!(touch.button, PointerButton::Primary);
        assert!(touch.is_primary());
    }

    #[test]
    fn a_secondary_press_keeps_its_button() {
        let press = PointerInfo::mouse(Vec2d { x: 1.0, y: 2.0 }, PointerButton::Secondary);

        assert!(!press.is_primary());
        assert_eq!(press.button, PointerButton::Secondary);
    }

    #[test]
    fn distance_is_euclidean() {
        let a = PointerInfo::mouse(Vec2d { x: 0.0, y: 0.0 }, PointerButton::Primary);
        let b = PointerInfo::mouse(Vec2d { x: 3.0, y: 4.0 }, PointerButton::Primary);

        assert_eq!(a.distance_to(&b), 5.0);
        assert_eq!(b.distance_to(&a), 5.0);
    }

    #[test]
    fn the_default_touch_target_follows_the_platform_guideline() {
        // Apple's HIG asks for 44pt, Material for 48dp.
        let expected = if cfg!(any(target_os = "ios", target_os = "macos")) {
            44.0
        } else {
            48.0
        };

        assert_eq!(DEFAULT_MIN_TOUCH_TARGET, expected);
        assert_eq!(min_touch_target(), DEFAULT_MIN_TOUCH_TARGET);
    }

    #[test]
    fn the_touch_target_can_be_overridden_and_disabled() {
        set_min_touch_target(56.0);
        assert_eq!(min_touch_target(), 56.0);

        set_min_touch_target(0.0);
        assert_eq!(min_touch_target(), 0.0, "zero turns the extra area off");

        set_min_touch_target(DEFAULT_MIN_TOUCH_TARGET);
    }

    #[test]
    fn a_touch_target_that_is_not_a_size_is_treated_as_off() {
        for invalid in [-4.0, f32::NAN, f32::NEG_INFINITY] {
            set_min_touch_target(invalid);
            assert_eq!(min_touch_target(), 0.0, "{invalid} is not a size");
        }

        set_min_touch_target(DEFAULT_MIN_TOUCH_TARGET);
    }

    #[test]
    fn a_scroll_that_was_used_is_reported_once() {
        let _ = take_scroll_used();
        assert!(!take_scroll_used(), "nothing used it");

        note_scroll_used();
        assert!(take_scroll_used());
        assert!(!take_scroll_used(), "and it is reported once");
    }

    #[test]
    fn a_scroll_being_routed_is_known_to_hit_testing_and_forgotten_after() {
        assert!(!is_routing_scroll(), "nothing is being routed to begin with");

        note_routing_scroll(true);
        assert!(is_routing_scroll());

        note_routing_scroll(false);
        assert!(!is_routing_scroll());
    }

    #[test]
    fn midpoint_keeps_the_identity_of_the_pointer_it_was_taken_from() {
        let a = PointerInfo::touch(Vec2d { x: 0.0, y: 0.0 }, 1);
        let b = PointerInfo::touch(Vec2d { x: 4.0, y: 8.0 }, 2);

        let mid = a.midpoint(&b);

        assert_eq!(mid.pos, Vec2d { x: 2.0, y: 4.0 });
        assert_eq!(mid.id, 1);
        assert_eq!(mid.source, PointerSource::Touch);
    }
}
