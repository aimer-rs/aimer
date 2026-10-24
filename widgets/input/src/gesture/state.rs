//! The gesture state machine's data, and nothing else.
//!
//! Deliberately free of `Rc`, `RefCell`, windows and callbacks: a
//! [`GestureState`] is plain data that a test can build, drive and inspect
//! directly, which is what makes the recognizers in [`super::recognize`]
//! testable without a live widget tree.

use aimer_attribute::position::Vec2d;
use aimer_events::pointer::{PointerInfo, PointerSource};
use aimer_utils::AnimInstant;

use super::{tap_slop, tap_tolerance};

/// How many simultaneous contacts a detector tracks.
///
/// Two are needed for a pinch; four gives room for a hand resting on a
/// touchscreen without the array overflowing. Beyond that, extra contacts are
/// ignored — a gesture detector has no use for a fifth finger, and paying for a
/// heap-allocated map on every pointer move to store one would be a poor trade.
pub const MAX_TRACKED_TOUCHES: usize = 4;

/// The contacts currently down on a detector, in the order they arrived.
///
/// A fixed inline array rather than a map: this is read and written on every
/// pointer move, and hashing a key to reach one of at most four entries costs
/// more than scanning them. It also makes the order deterministic, which a map
/// never was — the pinch focal point used to be computed from whichever two
/// entries the hasher happened to yield first.
///
/// # Examples
///
/// ```
/// use aimer_attribute::position::Vec2d;
/// use aimer_events::pointer::{PointerInfo, PointerSource};
/// use aimer_input::gesture::state::ActiveTouches;
///
/// let mut touches = ActiveTouches::default();
/// touches.insert(PointerInfo::touch(Vec2d { x: 0.0, y: 0.0 }, 1));
/// touches.insert(PointerInfo::touch(Vec2d { x: 10.0, y: 0.0 }, 2));
///
/// assert_eq!(touches.len(), 2);
/// assert!(touches.contains(PointerSource::Touch, 1));
/// assert!(!touches.contains(PointerSource::Mouse, 1));
///
/// touches.remove(PointerSource::Touch, 1);
///
/// assert_eq!(touches.len(), 1);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActiveTouches {
    slots: [Option<PointerInfo>; MAX_TRACKED_TOUCHES],
}

impl ActiveTouches {
    /// Records a contact, or updates the position of one already down.
    ///
    /// A contact beyond [`MAX_TRACKED_TOUCHES`] is dropped.
    #[inline]
    pub fn insert(&mut self, pointer: PointerInfo) {
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_some_and(|held| Self::is_same(&held, pointer.source, pointer.id)))
        {
            *slot = Some(pointer);
            return;
        }
        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(pointer);
        }
    }

    /// Forgets a contact, returning its last known position.
    ///
    /// Later contacts shift down so the order of arrival is preserved.
    #[inline]
    pub fn remove(&mut self, source: PointerSource, id: u64) -> Option<PointerInfo> {
        let index = self
            .slots
            .iter()
            .position(|slot| slot.is_some_and(|held| Self::is_same(&held, source, id)))?;
        let removed = self.slots[index].take();
        for shift in index..MAX_TRACKED_TOUCHES - 1 {
            self.slots[shift] = self.slots[shift + 1].take();
        }
        removed
    }

    /// Whether that contact is currently down.
    #[inline]
    pub fn contains(&self, source: PointerSource, id: u64) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.is_some_and(|held| Self::is_same(&held, source, id)))
    }

    /// How many contacts are down.
    #[inline]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    /// Whether no contact is down.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|slot| slot.is_none())
    }

    /// Forgets every contact.
    #[inline]
    pub fn clear(&mut self) {
        self.slots = [None; MAX_TRACKED_TOUCHES];
    }

    /// The contacts, oldest first.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &PointerInfo> {
        self.slots.iter().flatten()
    }

    /// The two oldest contacts, which are the pair a pinch is measured between.
    ///
    /// Returns `None` while fewer than two are down.
    #[inline]
    pub fn pinch_pair(&self) -> Option<(PointerInfo, PointerInfo)> {
        match (self.slots[0], self.slots[1]) {
            (Some(first), Some(second)) => Some((first, second)),
            _ => None,
        }
    }

    #[inline]
    fn is_same(held: &PointerInfo, source: PointerSource, id: u64) -> bool {
        held.source == source && held.id == id
    }
}

/// A pointer that is down, and how long it has been.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    /// Where the pointer went down.
    pub pointer: PointerInfo,
    /// When it went down.
    pub down_at: AnimInstant,
    /// Whether the long-press threshold has already been reported, so it is
    /// reported once per press rather than on every poll after it elapses.
    pub long_pressed: bool,
    /// The last position reported while the long press was held, so a
    /// long-press move can report a delta.
    pub long_press_last: Option<PointerInfo>,
}

/// A completed tap, kept only long enough to decide whether the next one
/// doubles it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    pub pointer: PointerInfo,
    pub at: AnimInstant,
}

/// A drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    /// Where the press that became this drag started — the position reported by
    /// `DragStart`, so the dragged thing does not jump by the slop distance.
    pub origin: PointerInfo,
    /// The most recent position, which the next delta is measured from.
    pub last: PointerInfo,
    /// When the drag started, for the swipe velocity.
    pub started_at: AnimInstant,
}

/// A two-pointer pinch in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pinch {
    /// The distance between the two pointers when the pinch began; the
    /// denominator of the reported scale.
    pub initial_distance: f32,
    /// The most recently reported scale, which the next delta is measured
    /// against.
    pub scale: f32,
    /// The direction of the line between the two pointers at the last report,
    /// in radians; the reference the next rotation delta is measured from.
    pub angle: f32,
    /// The rotation accumulated since the pinch began, in radians, clockwise
    /// positive.
    pub rotation: f32,
    /// The midpoint at the last report, which the next focal delta is measured
    /// from.
    pub focal: Vec2d,
}

/// A trackpad (or other platform-recognized) pinch and rotation in progress.
///
/// The platform reports magnification and rotation as two independent gestures
/// that may overlap, but a consumer wants one transform. This is the shared
/// session they both feed: it opens with the first of them, accumulates both,
/// and closes when the last one ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlatformTransform {
    /// Where the transform is anchored; the platform reports one position, so
    /// the focal point does not move.
    pub focal: Vec2d,
    /// The scale accumulated since the session began.
    pub scale: f32,
    /// The rotation accumulated since the session began, in radians, clockwise
    /// positive.
    pub rotation: f32,
    /// Whether a platform pinch is currently between its start and end.
    pub pinching: bool,
    /// Whether a platform rotation is currently between its start and end.
    pub rotating: bool,
}

/// Everything the recognizers remember between pointer events.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GestureState {
    /// The single-pointer press being tracked, if any.
    pub press: Option<Press>,
    /// The previous tap, for double-tap detection.
    pub last_tap: Option<Tap>,
    /// The drag in progress, if the press has crossed the slop.
    pub drag: Option<Drag>,
    /// Every contact currently down.
    pub touches: ActiveTouches,
    /// The pinch in progress, if two contacts are down.
    pub pinch: Option<Pinch>,
    /// The platform-recognized pinch and rotation in progress, if any.
    pub platform_transform: Option<PlatformTransform>,
    /// Whether a platform multi-finger pan is in progress.
    pub platform_pan: bool,
}

impl GestureState {
    /// Whether a drag is in progress.
    #[inline]
    pub const fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Whether a long press has been reported and not yet ended.
    #[inline]
    pub fn is_long_pressed(&self) -> bool {
        self.press.is_some_and(|press| press.long_pressed)
    }

    /// Whether that contact is currently down on this detector.
    ///
    /// Used to accept a release that happened outside the detector's bounds: the
    /// finger that pressed inside owns the gesture until it lifts, wherever it
    /// lifts.
    #[inline]
    pub fn has_active_touch(&self, source: PointerSource, id: u64) -> bool {
        self.touches.contains(source, id)
    }

    /// Whether any gesture is currently in flight — a press being held, a drag
    /// past the slop, a pinch, or any contact still down.
    ///
    /// The idle counterpart is what matters: a pointer event reaching an idle
    /// recognizer that also produced no output was a pure hover, which the
    /// detector lets fall through instead of claiming — and repainting for —
    /// every mouse move that merely crosses it.
    #[inline]
    pub fn is_engaged(&self) -> bool {
        self.press.is_some()
            || self.drag.is_some()
            || self.pinch.is_some()
            || self.platform_transform.is_some()
            || self.platform_pan
            || !self.touches.is_empty()
    }

    /// Forgets everything about the single-pointer press: the press itself, any
    /// drag it became, and any long press it reached.
    #[inline]
    pub fn clear_press(&mut self) {
        self.press = None;
        self.drag = None;
    }

    /// Forgets the pinch, so the next pair of contacts starts a fresh one.
    #[inline]
    pub fn clear_pinch(&mut self) {
        self.pinch = None;
    }
}

/// Whether moving from `from` to `to` is far enough to begin a drag.
///
/// The threshold comes from the *moving* pointer's device: a finger is allowed
/// eighteen pixels of wobble, a mouse barely one. Whether a release is still a
/// tap is a separate question, answered by [`moved_beyond_tap_tolerance`].
#[inline]
pub fn moved_beyond_slop(from: &PointerInfo, to: &PointerInfo) -> bool {
    from.distance_to(to) > tap_slop(to.source)
}

/// Whether moving from `from` to `to` is far enough that a release there is no
/// longer a tap.
///
/// Looser than [`moved_beyond_slop`] for a mouse: a click drifts by a few pixels
/// as the button goes down, which must not cost the user the click. A press that
/// may become a drag has already become one by then.
#[inline]
pub fn moved_beyond_tap_tolerance(from: &PointerInfo, to: &PointerInfo) -> bool {
    from.distance_to(to) > tap_tolerance(to.source)
}

#[cfg(test)]
mod tests {
    use aimer_attribute::position::Vec2d;
    use aimer_events::pointer::PointerButton;

    use super::*;

    fn touch(x: f32, y: f32, id: u64) -> PointerInfo {
        PointerInfo::touch(Vec2d { x, y }, id)
    }

    #[test]
    fn inserting_the_same_contact_twice_updates_it_rather_than_duplicating() {
        let mut touches = ActiveTouches::default();
        touches.insert(touch(0.0, 0.0, 1));
        touches.insert(touch(5.0, 5.0, 1));

        assert_eq!(touches.len(), 1);
        assert_eq!(touches.iter().next().map(|p| p.pos.x), Some(5.0));
    }

    #[test]
    fn a_mouse_and_a_touch_with_the_same_id_are_different_contacts() {
        let mut touches = ActiveTouches::default();
        touches.insert(touch(0.0, 0.0, 0));
        touches.insert(PointerInfo::mouse(Vec2d::ZERO, PointerButton::Primary));

        assert_eq!(touches.len(), 2);
        assert!(touches.contains(PointerSource::Touch, 0));
        assert!(touches.contains(PointerSource::Mouse, 0));
    }

    // The pinch pair must be the two contacts that arrived first, every time.
    // With the map this replaced, the pair depended on hash order, so the focal
    // point and the reported scale could swap between two moves of the same
    // pinch.
    #[test]
    fn the_pinch_pair_is_the_two_oldest_contacts_in_arrival_order() {
        let mut touches = ActiveTouches::default();

        assert_eq!(touches.pinch_pair(), None);

        touches.insert(touch(0.0, 0.0, 7));
        assert_eq!(touches.pinch_pair(), None);

        touches.insert(touch(10.0, 0.0, 3));
        touches.insert(touch(20.0, 0.0, 5));

        let (first, second) = touches.pinch_pair().expect("two contacts are down");

        assert_eq!(first.id, 7);
        assert_eq!(second.id, 3);
    }

    #[test]
    fn removing_a_contact_keeps_the_remaining_ones_in_order() {
        let mut touches = ActiveTouches::default();
        touches.insert(touch(0.0, 0.0, 1));
        touches.insert(touch(1.0, 0.0, 2));
        touches.insert(touch(2.0, 0.0, 3));

        let removed = touches.remove(PointerSource::Touch, 2);

        assert_eq!(removed.map(|p| p.id), Some(2));
        assert_eq!(
            touches.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![1, 3],
            "the survivors keep their arrival order"
        );
        assert_eq!(touches.remove(PointerSource::Touch, 2), None);
    }

    #[test]
    fn contacts_beyond_the_tracked_maximum_are_dropped_not_panicked_on() {
        let mut touches = ActiveTouches::default();
        for id in 0..(MAX_TRACKED_TOUCHES as u64 + 3) {
            touches.insert(touch(id as f32, 0.0, id));
        }

        assert_eq!(touches.len(), MAX_TRACKED_TOUCHES);
        assert!(touches.contains(PointerSource::Touch, 0));
    }

    #[test]
    fn clearing_forgets_every_contact() {
        let mut touches = ActiveTouches::default();
        touches.insert(touch(0.0, 0.0, 1));
        touches.clear();

        assert!(touches.is_empty());
        assert_eq!(touches.len(), 0);
    }

    // The slop is the whole reason a mouse drag felt broken: eighteen pixels of
    // tolerance meant a deliberate short click-drag was reported as a click.
    #[test]
    fn the_slop_that_decides_a_drag_comes_from_the_device() {
        let touch_down = touch(0.0, 0.0, 0);
        let touch_moved = touch(5.0, 0.0, 0);
        let mouse_down = PointerInfo::mouse(Vec2d::ZERO, PointerButton::Primary);
        let mouse_moved = PointerInfo::mouse(Vec2d { x: 5.0, y: 0.0 }, PointerButton::Primary);

        assert!(
            !moved_beyond_slop(&touch_down, &touch_moved),
            "5 px of finger wobble is still a tap"
        );
        assert!(
            moved_beyond_slop(&mouse_down, &mouse_moved),
            "5 px of mouse travel is a deliberate drag"
        );
    }

    #[test]
    fn clearing_the_press_also_clears_the_drag_it_became() {
        let down = touch(0.0, 0.0, 1);
        let now = AnimInstant::now();
        let mut state = GestureState {
            press: Some(Press {
                pointer: down,
                down_at: now,
                long_pressed: true,
                long_press_last: Some(down),
            }),
            drag: Some(Drag {
                origin: down,
                last: down,
                started_at: now,
            }),
            ..Default::default()
        };

        assert!(state.is_dragging());
        assert!(state.is_long_pressed());

        state.clear_press();

        assert!(!state.is_dragging());
        assert!(!state.is_long_pressed());
    }
}
