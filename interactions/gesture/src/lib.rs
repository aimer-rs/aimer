//! Gesture recognition: pointer events in, [`GestureEvent`]s out.
//!
//! The recognizer is a **pure function over a state machine**
//! ([`recognize::recognize`]) rather than a method on a widget. Nothing in
//! [`state`] or [`recognize`] touches an `Rc`, a window, a callback or a
//! `BuildContext`, and the current time arrives as a parameter — so the whole
//! tap / double-tap / long-press / slop / pinch matrix is unit-testable with a
//! fake clock, which is exactly what the previous design could not do.
//!
//! [`gesture_detector::GestureDetector`] is the thin widget on top: it
//! translates [`aimer_events::element::ElementEvent`] into
//! [`aimer_events::pointer::PointerEvent`], calls the recognizer, and fans the
//! output out to handlers.

use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};

use crate::callback::VoidParamedFunction;

pub mod gesture_detector;
pub mod handlers;
pub mod recognize;
pub mod state;

pub(crate) const DOUBLE_TAP_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);
/// How long a pointer must rest before a press becomes a long press.
///
/// Public so that gestures built outside this crate — a drag that only starts
/// after a long press, so an enclosing scrollable keeps working on touch — agree
/// with the recognizers here instead of inventing a second threshold.
pub const LONG_PRESS_DURATION: std::time::Duration = std::time::Duration::from_millis(500);

/// How far a *touch* may travel and still count as a tap rather than a drag, in
/// logical pixels.
///
/// A finger is a blunt instrument: the contact patch is centimetres wide and it
/// rolls as it presses, so a tap that was meant to be still moves. This is
/// Flutter's `kTouchSlop`.
///
/// Public for the same reason as [`LONG_PRESS_DURATION`]: a drag recognized
/// elsewhere must begin exactly where a tap stops being a tap, or the two
/// disagree about what the user did. Prefer [`tap_slop`], which picks the right
/// value for the device.
pub const TAP_SLOP: f32 = 18.0;

/// How far a *mouse* must travel before a press becomes a drag, in logical
/// pixels.
///
/// A mouse is precise and rests where it is put, so a drag should start almost
/// at once. Using the touch slop for it — which this crate did until the slop
/// became per-source — meant a deliberate short click-drag of up to 18 px was
/// reported as a plain click and the drag never started. This is Flutter's
/// `kPrecisePointerHitSlop`.
///
/// This decides when a *drag* starts, not whether a click survives: a press
/// that nobody may drag with is judged by [`MOUSE_TAP_TOLERANCE`].
pub const MOUSE_SLOP: f32 = 1.0;

/// How far a *mouse* may drift between press and release and still be a click,
/// in logical pixels.
///
/// Pressing a trackpad or a mouse button nudges the cursor by a pixel or three,
/// and a click the user meant is not abandoned for that. It is still far smaller
/// than the touch slop, so sliding off a control and releasing there cancels.
///
/// Only a press that cannot become a drag is judged by this: when a drag is
/// listened for, travel past [`MOUSE_SLOP`] is a drag before it could fail to be
/// a click, so the two never disagree.
pub const MOUSE_TAP_TOLERANCE: f32 = 8.0;

/// The movement tolerance for `source`, in logical pixels.
///
/// # Examples
///
/// ```
/// use aimer_events::pointer::PointerSource;
/// use aimer_input::gesture::{MOUSE_SLOP, TAP_SLOP, tap_slop};
///
/// assert_eq!(tap_slop(PointerSource::Touch), TAP_SLOP);
/// assert_eq!(tap_slop(PointerSource::Mouse), MOUSE_SLOP);
/// assert!(tap_slop(PointerSource::Mouse) < tap_slop(PointerSource::Touch));
/// ```
#[inline]
pub const fn tap_slop(source: PointerSource) -> f32 {
    match source {
        PointerSource::Touch => TAP_SLOP,
        PointerSource::Mouse => MOUSE_SLOP,
    }
}

/// How far `source` may move between press and release and still be a tap or a
/// click, in logical pixels.
///
/// Unlike [`tap_slop`], which is where a press turns into a drag, this is
/// what a release is judged by when no drag took the press.
///
/// # Examples
///
/// ```
/// use aimer_events::pointer::PointerSource;
/// use aimer_input::gesture::{MOUSE_TAP_TOLERANCE, TAP_SLOP, tap_tolerance};
///
/// assert_eq!(tap_tolerance(PointerSource::Touch), TAP_SLOP);
/// assert_eq!(tap_tolerance(PointerSource::Mouse), MOUSE_TAP_TOLERANCE);
/// ```
#[inline]
pub const fn tap_tolerance(source: PointerSource) -> f32 {
    match source {
        PointerSource::Touch => TAP_SLOP,
        PointerSource::Mouse => MOUSE_TAP_TOLERANCE,
    }
}

pub(crate) const SWIPE_VELOCITY_THRESHOLD: f32 = 300.0; // px/sec
pub(crate) const SWIPE_MAX_DURATION_MS: u64 = 500;

/// Time (ms) after which orphan touches are considered stale (e.g. app was
/// backgrounded without Cancel/Up) and cleared on the next PointerDown.
pub(crate) const STALE_GESTURE_TOUCH_MS: u64 = 1000;

/// Position and movement of an in-progress drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragUpdateData {
    pub position: PointerInfo,
    pub delta_x: f32,
    pub delta_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollData {
    pub delta_x: f32,
    pub delta_y: f32,
}

/// One step of a pinch, rotate and two-finger pan, as handed to
/// [`gesture_detector::GestureDetector::on_scale`].
///
/// It is the whole transform of the contacts, not only their spread: two fingers
/// that pinch, twist and drift are one gesture. A consumer that only wants one
/// part ignores the others — a zoom-only view reads `delta_scale`, a rotation
/// dial reads `delta_rotation`.
///
/// The `delta_*` fields are relative to the previous update and are what a
/// transform applies incrementally; `scale` and `rotation` are relative to the
/// start of the gesture.
///
/// A trackpad pinch reports no focal movement, and a trackpad rotation reports a
/// `delta_scale` of exactly `1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaleData {
    /// The midpoint of the gesture, in logical window coordinates.
    pub focal_x: f32,
    pub focal_y: f32,
    /// The scale since the gesture began; `1.0` is unchanged.
    pub scale: f32,
    /// The scale change since the previous update; `1.0` is unchanged.
    pub delta_scale: f32,
    /// The rotation since the gesture began, in radians, clockwise positive.
    pub rotation: f32,
    /// The rotation since the previous update, in radians, clockwise positive.
    pub delta_rotation: f32,
    /// How far the midpoint moved since the previous update, in logical pixels.
    pub focal_delta_x: f32,
    pub focal_delta_y: f32,
}

/// One step of a platform multi-finger pan, as handed to
/// [`gesture_detector::GestureDetector::on_pan`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanData {
    /// Where the pan is anchored, in logical window coordinates.
    pub x: f32,
    pub y: f32,
    /// The movement since the previous update, in logical pixels.
    pub delta_x: f32,
    pub delta_y: f32,
}

/// Where a smart-zoom gesture happened, as handed to
/// [`gesture_detector::GestureDetector::on_smart_zoom`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmartZoomData {
    /// The point to zoom to, in logical window coordinates.
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeDirection {
    Up,
    Down,
    Left,
    Right,
}

pub type DragCallback = VoidParamedFunction<PointerInfo>;
pub type DragUpdateCallback = VoidParamedFunction<DragUpdateData>;
pub type SwipeCallback = VoidParamedFunction<SwipeDirection>;
pub type ScrollCallback = VoidParamedFunction<ScrollData>;
pub type ScaleCallback = VoidParamedFunction<ScaleData>;
pub type PanCallback = VoidParamedFunction<PanData>;
pub type SmartZoomCallback = VoidParamedFunction<SmartZoomData>;
/// Receives every recognized gesture, whatever it is.
pub type GestureStreamCallback = VoidParamedFunction<GestureEvent>;

/// Something the user did, as recognized from a stream of pointer events.
///
/// This is the single source of truth: the recognizer produces these and
/// nothing else, and the individual `on_*` callbacks are filters over the same
/// stream rather than a parallel path. When the two were separate — callbacks
/// fired inside the state machine *and* an event returned from it — they drifted
/// apart, and a fast flick fired `on_drag_end` while reporting only `Swipe`.
///
/// `Copy`, and every payload is inline, so a recognizer can return several of
/// these without allocating.
///
/// There is no `RightTap`: [`PointerInfo::button`] on [`Self::Tap`] says which
/// button it was, so a secondary or middle click needs no variant of its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GestureEvent {
    /// A pointer went down inside the detector. Fires immediately, before it is
    /// known whether this becomes a tap, a drag or a long press — which is what
    /// press-state feedback needs.
    TapDown { pointer: PointerInfo },
    /// The pointer came up while still within the slop.
    ///
    /// Always accompanied by exactly one of [`Self::Tap`] or
    /// [`Self::DoubleTap`]; it exists so a widget can drop its pressed visual
    /// without having to subscribe to every way a press can end.
    TapUp { pointer: PointerInfo },
    /// The press ended without becoming a tap: it turned into a drag, or was
    /// interrupted. The counterpart to [`Self::TapDown`] for the failure path.
    TapCancel,
    /// A completed tap.
    Tap { pointer: PointerInfo },
    /// A second tap inside the double-tap timeout and within the slop of the
    /// first.
    DoubleTap { pointer: PointerInfo },
    /// The press has been held for [`LONG_PRESS_DURATION`] — reported while the
    /// pointer is still down, which is what makes it feel like a long press
    /// rather than a slow tap.
    LongPress { pointer: PointerInfo },
    /// Same instant as [`Self::LongPress`], kept separate so the
    /// start/update/end triple a text-selection handle or a long-press-then-drag
    /// needs is complete on its own.
    LongPressStart { pointer: PointerInfo },
    /// The pointer moved while a long press was held.
    LongPressMoveUpdate {
        pointer: PointerInfo,
        delta_x: f32,
        delta_y: f32,
    },
    /// The long-pressed pointer came up.
    LongPressEnd { pointer: PointerInfo },
    /// Movement crossed the slop for the pointer's device: a drag has begun.
    ///
    /// The reported pointer is where the press started, not where the slop was
    /// crossed, so the drag follows the finger without a jump.
    DragStart { pointer: PointerInfo },
    /// Movement during an active drag.
    DragUpdate {
        pointer: PointerInfo,
        delta_x: f32,
        delta_y: f32,
    },
    /// The dragging pointer came up.
    DragEnd { pointer: PointerInfo },
    /// The drag was interrupted by something other than the user — the app was
    /// backgrounded, or the window lost the pointer.
    ///
    /// Without this a consumer that had seen [`Self::DragStart`] would go on
    /// believing it was still dragging forever.
    DragCancel,
    /// A drag fast enough, and short enough, to read as a flick. Emitted
    /// *alongside* [`Self::DragEnd`], never instead of it.
    Swipe {
        direction: SwipeDirection,
        velocity_x: f32,
        velocity_y: f32,
    },
    /// A wheel or trackpad scroll.
    Scroll { delta_x: f32, delta_y: f32 },
    /// A second pointer went down, or the platform began a pinch or rotation: a
    /// scale gesture has begun.
    ScaleStart { focal_x: f32, focal_y: f32 },
    /// The contacts spread, turned or travelled; see [`ScaleData`] for the
    /// meaning of each field.
    ScaleUpdate {
        focal_x: f32,
        focal_y: f32,
        scale: f32,
        delta_scale: f32,
        rotation: f32,
        delta_rotation: f32,
        focal_delta_x: f32,
        focal_delta_y: f32,
    },
    /// A scale gesture dropped below two pointers, or the platform ended it.
    ScaleEnd,
    /// The platform began a multi-finger pan.
    PanStart { x: f32, y: f32 },
    /// The platform pan moved.
    PanUpdate {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    },
    /// The platform pan ended or was cancelled.
    PanEnd,
    /// The platform's smart-zoom gesture — a two-finger double tap on a macOS
    /// trackpad.
    SmartZoom { x: f32, y: f32 },
}

impl GestureEvent {
    /// The pointer the gesture happened at, for the variants that have one.
    #[inline]
    pub const fn pointer(&self) -> Option<PointerInfo> {
        match self {
            Self::TapDown { pointer }
            | Self::TapUp { pointer }
            | Self::Tap { pointer }
            | Self::DoubleTap { pointer }
            | Self::LongPress { pointer }
            | Self::LongPressStart { pointer }
            | Self::LongPressMoveUpdate { pointer, .. }
            | Self::LongPressEnd { pointer }
            | Self::DragStart { pointer }
            | Self::DragUpdate { pointer, .. }
            | Self::DragEnd { pointer } => Some(*pointer),
            _ => None,
        }
    }

    /// The button the gesture was made with, defaulting to
    /// [`PointerButton::Primary`] for gestures that have no pointer of their own
    /// (a scroll, a pinch).
    #[inline]
    pub const fn button(&self) -> PointerButton {
        match self.pointer() {
            Some(pointer) => pointer.button,
            None => PointerButton::Primary,
        }
    }
}

/// The gestures a detector was actually configured to hear about.
///
/// Recognition is cheap for most gestures but not all: a pinch runs distance and
/// midpoint math on every move, and a swipe runs a velocity calculation on every
/// release. This bitset lets [`recognize::recognize`] skip work whose result
/// nobody would read, in one integer test rather than by probing a handful of
/// `Option`s.
///
/// # Examples
///
/// ```
/// use aimer_input::gesture::GestureMask;
///
/// let mask = GestureMask::TAP.union(GestureMask::SCALE);
///
/// assert!(mask.contains(GestureMask::SCALE));
/// assert!(!mask.contains(GestureMask::SWIPE));
/// assert!(GestureMask::ALL.contains(GestureMask::SWIPE));
/// assert!(GestureMask::NONE.is_empty());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GestureMask(u16);

impl GestureMask {
    /// Nothing is being listened for.
    pub const NONE: Self = Self(0);
    pub const TAP: Self = Self(1 << 0);
    pub const DOUBLE_TAP: Self = Self(1 << 1);
    pub const LONG_PRESS: Self = Self(1 << 2);
    pub const DRAG: Self = Self(1 << 3);
    pub const SWIPE: Self = Self(1 << 4);
    pub const SCROLL: Self = Self(1 << 5);
    pub const SCALE: Self = Self(1 << 6);
    /// Press lifecycle: [`GestureEvent::TapDown`], [`GestureEvent::TapUp`] and
    /// [`GestureEvent::TapCancel`].
    pub const PRESS: Self = Self(1 << 7);
    /// A multi-finger pan reported by the platform, as opposed to the
    /// single-pointer [`Self::DRAG`].
    pub const PAN: Self = Self(1 << 8);
    /// The platform's smart-zoom double tap.
    pub const SMART_ZOOM: Self = Self(1 << 9);
    /// Every gesture there is.
    pub const ALL: Self = Self(u16::MAX);
    /// Everything a pointer can be doing to the detector itself, but not
    /// scrolling.
    ///
    /// What a raw
    /// [`gesture_detector::GestureDetector::on_gesture`] handler asks for.
    /// Scrolling is left out deliberately: observing a scroll and *claiming* it
    /// are different decisions, and a detector that claims one stops it reaching
    /// whatever is behind it. Consuming a scroll has to be asked for explicitly,
    /// with [`gesture_detector::GestureDetector::on_scroll`].
    pub const EVERY_POINTER_GESTURE: Self = Self::ALL.without(Self::SCROLL);

    /// Whether every gesture in `other` is present in `self`.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any gesture in `other` is present in `self`.
    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Both sets together.
    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// `self` with everything in `other` removed.
    #[inline]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Adds `other` in place.
    #[inline]
    pub const fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Whether nothing at all is being listened for.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The gestures one pointer event produced, in the order they happened.
///
/// A list, not an `Option`, because a single event legitimately means two
/// things: releasing a fast drag is both a [`GestureEvent::DragEnd`] and a
/// [`GestureEvent::Swipe`], and reaching the long-press threshold is both
/// [`GestureEvent::LongPress`] and [`GestureEvent::LongPressStart`]. The old
/// single-slot return is why `DragEnd` went missing on every flick.
///
/// Backed by a fixed inline array, so recognizing a gesture never allocates —
/// this runs on every pointer move.
///
/// # Examples
///
/// ```
/// use aimer_attribute::position::Vec2d;
/// use aimer_events::pointer::PointerInfo;
/// use aimer_input::gesture::{GestureEvent, GestureOutput};
///
/// let pointer = PointerInfo::touch(Vec2d { x: 1.0, y: 1.0 }, 0);
/// let mut output = GestureOutput::new();
///
/// assert!(output.is_empty());
///
/// output.push(GestureEvent::TapDown { pointer });
/// output.push(GestureEvent::Tap { pointer });
///
/// assert_eq!(output.len(), 2);
/// assert!(output.contains(&GestureEvent::Tap { pointer }));
/// assert_eq!(
///     output.iter().copied().collect::<Vec<_>>(),
///     vec![
///         GestureEvent::TapDown { pointer },
///         GestureEvent::Tap { pointer },
///     ],
/// );
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GestureOutput {
    events: [Option<GestureEvent>; Self::CAPACITY],
    len: usize,
}

impl GestureOutput {
    /// How many gestures one pointer event can produce.
    ///
    /// Five: a release that ends a long-pressed drag is `LongPressEnd`,
    /// `DragEnd`, `Swipe`, `TapUp`; a cancellation can end a drag, a press, a
    /// pinch and a platform pan at once — `DragCancel`, `TapCancel`, `ScaleEnd`,
    /// `PanEnd`; and a platform pinch restarting over one that never ended is
    /// `ScaleEnd`, `ScaleStart`, `ScaleUpdate`. Pushing beyond this is a bug in a
    /// recognizer, and is caught by a debug assertion rather than by growing a
    /// heap buffer on the hot path.
    pub const CAPACITY: usize = 5;

    /// An empty output.
    #[inline]
    pub const fn new() -> Self {
        Self {
            events: [None; Self::CAPACITY],
            len: 0,
        }
    }

    /// An output holding exactly one gesture.
    #[inline]
    pub const fn once(event: GestureEvent) -> Self {
        let mut output = Self::new();
        output.events[0] = Some(event);
        output.len = 1;
        output
    }

    /// Appends a gesture.
    ///
    /// # Panics
    ///
    /// In debug builds, if more than [`Self::CAPACITY`] gestures are pushed. In
    /// release builds the extra gesture is dropped rather than costing a bounds
    /// check failure on the hot path.
    #[inline]
    pub fn push(&mut self, event: GestureEvent) {
        debug_assert!(
            self.len < Self::CAPACITY,
            "more than {} gestures from one pointer event: {event:?}",
            Self::CAPACITY,
        );
        if self.len < Self::CAPACITY {
            self.events[self.len] = Some(event);
            self.len += 1;
        }
    }

    /// The gestures, in the order they were recognized.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &GestureEvent> {
        self.events[..self.len].iter().flatten()
    }

    /// How many gestures were recognized.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the pointer event produced no gesture at all — the common case
    /// for a move that has not yet crossed the slop.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether `event` is among the recognized gestures.
    #[inline]
    pub fn contains(&self, event: &GestureEvent) -> bool {
        self.iter().any(|recognized| recognized == event)
    }

    /// The first recognized gesture, if any.
    #[inline]
    pub const fn first(&self) -> Option<GestureEvent> {
        self.events[0]
    }
}

impl IntoIterator for GestureOutput {
    type Item = GestureEvent;
    type IntoIter =
        std::iter::Flatten<std::array::IntoIter<Option<GestureEvent>, { GestureOutput::CAPACITY }>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter().flatten()
    }
}

impl From<GestureEvent> for GestureOutput {
    #[inline]
    fn from(event: GestureEvent) -> Self {
        Self::once(event)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use aimer_attribute::position::Vec2d;

    use super::*;

    fn pointer(x: f32, y: f32) -> PointerInfo {
        PointerInfo::touch(Vec2d { x, y }, 0)
    }

    #[test]
    fn a_mouse_needs_far_less_slop_than_a_finger() {
        assert_eq!(tap_slop(PointerSource::Touch), TAP_SLOP);
        assert_eq!(tap_slop(PointerSource::Mouse), MOUSE_SLOP);
        assert!(tap_slop(PointerSource::Mouse) < tap_slop(PointerSource::Touch));
    }

    #[test]
    fn a_mouse_click_tolerates_more_drift_than_a_drag_takes_to_start() {
        // A press that has not become a drag must still be able to be a click, so
        // the click tolerance can never be tighter than the drag slop.
        assert!(tap_tolerance(PointerSource::Mouse) >= tap_slop(PointerSource::Mouse));
        assert!(tap_tolerance(PointerSource::Mouse) > MOUSE_SLOP);
        assert_eq!(tap_tolerance(PointerSource::Mouse), MOUSE_TAP_TOLERANCE);
        assert_eq!(tap_tolerance(PointerSource::Touch), TAP_SLOP);
    }

    #[test]
    fn an_output_reports_the_gestures_in_recognition_order() {
        let end = pointer(10.0, 10.0);
        let mut output = GestureOutput::new();
        output.push(GestureEvent::DragEnd { pointer: end });
        output.push(GestureEvent::Swipe {
            direction: SwipeDirection::Right,
            velocity_x: 400.0,
            velocity_y: 0.0,
        });

        let recognized: Vec<_> = output.iter().copied().collect();

        assert_eq!(recognized.len(), 2);
        assert_eq!(recognized[0], GestureEvent::DragEnd { pointer: end });
        assert!(matches!(recognized[1], GestureEvent::Swipe { .. }));
    }

    #[test]
    fn an_empty_output_yields_nothing() {
        let output = GestureOutput::new();

        assert!(output.is_empty());
        assert_eq!(output.len(), 0);
        assert_eq!(output.first(), None);
        assert_eq!(output.iter().count(), 0);
        assert_eq!(output.into_iter().count(), 0);
    }

    #[test]
    fn once_holds_exactly_one_gesture() {
        let tap = GestureEvent::Tap {
            pointer: pointer(1.0, 2.0),
        };
        let output = GestureOutput::once(tap);

        assert_eq!(output.len(), 1);
        assert_eq!(output.first(), Some(tap));
        assert_eq!(GestureOutput::from(tap), output);
    }

    #[test]
    fn a_mask_only_contains_what_was_inserted() {
        let mut mask = GestureMask::NONE;

        assert!(mask.is_empty());

        mask.insert(GestureMask::TAP);
        mask.insert(GestureMask::SCALE);

        assert!(mask.contains(GestureMask::TAP));
        assert!(mask.contains(GestureMask::SCALE));
        assert!(mask.contains(GestureMask::TAP.union(GestureMask::SCALE)));
        assert!(!mask.contains(GestureMask::SWIPE));
        assert!(mask.intersects(GestureMask::SCALE.union(GestureMask::SWIPE)));
        assert!(GestureMask::ALL.contains(mask));
    }

    // Observing every gesture and claiming a scroll are separate decisions: a
    // detector that claims one stops it reaching whatever sits behind it.
    #[test]
    fn every_pointer_gesture_stops_short_of_claiming_a_scroll() {
        assert!(GestureMask::EVERY_POINTER_GESTURE.contains(GestureMask::TAP));
        assert!(GestureMask::EVERY_POINTER_GESTURE.contains(GestureMask::SCALE));
        assert!(GestureMask::EVERY_POINTER_GESTURE.contains(GestureMask::LONG_PRESS));
        assert!(!GestureMask::EVERY_POINTER_GESTURE.contains(GestureMask::SCROLL));
    }

    #[test]
    fn a_gesture_reports_the_button_it_was_made_with() {
        let secondary = PointerInfo::mouse(Vec2d { x: 1.0, y: 1.0 }, PointerButton::Secondary);

        assert_eq!(
            GestureEvent::Tap { pointer: secondary }.button(),
            PointerButton::Secondary
        );
        assert_eq!(
            GestureEvent::Tap { pointer: secondary }.pointer(),
            Some(secondary)
        );
        // A scroll has no pointer of its own.
        assert_eq!(
            GestureEvent::Scroll {
                delta_x: 0.0,
                delta_y: 1.0
            }
            .button(),
            PointerButton::Primary
        );
        assert_eq!(GestureEvent::TapCancel.pointer(), None);
    }
}
