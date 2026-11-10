//! The handlers a detector was configured with, and the fan-out from
//! [`GestureEvent`] to them.
//!
//! Held behind a single [`std::rc::Rc`] by the detector, so rebuilding a widget
//! costs one refcount bump however many gestures are configured. Storing a
//! callback per gesture in the widget meant ten clones per detector per rebuild
//! — sixteen once the press and long-press lifecycles were added — times every
//! row of a list, times every frame.

use aimer_events::pointer::PointerButton;

use crate::callback::{CallbackExecutor, VoidCallback};
use crate::gesture::{
    DragCallback, DragUpdateCallback, DragUpdateData, GestureEvent, GestureMask, GestureOutput,
    GestureStreamCallback, PanCallback, PanData, ScaleCallback, ScaleData, ScrollCallback,
    ScrollData, SmartZoomCallback, SmartZoomData, SwipeCallback,
};

/// Every handler a [`super::gesture_detector::GestureDetector`] may have, plus
/// the [`GestureMask`] describing which of them are actually set.
///
/// The mask is maintained as handlers are installed rather than derived on
/// demand, so the recognizer's "does anyone want a pinch?" question is one
/// integer test on the hot path instead of eleven `Option` probes.
#[derive(Default)]
pub struct GestureHandlers {
    pub on_tap: VoidCallback,
    pub on_double_press: VoidCallback,
    pub on_long_press: VoidCallback,
    pub on_drag_start: DragCallback,
    pub on_drag_update: DragUpdateCallback,
    pub on_drag_end: VoidCallback,
    pub on_right_tap: VoidCallback,
    pub on_swipe: SwipeCallback,
    pub on_scroll: ScrollCallback,
    pub on_scale: ScaleCallback,
    pub on_pan: PanCallback,
    pub on_smart_zoom: SmartZoomCallback,
    pub on_gesture: GestureStreamCallback,
    mask: GestureMask,
    /// The gestures a specific handler was installed for, as opposed to the ones
    /// the raw [`Self::on_gesture`] merely observes.
    claims: GestureMask,
}

impl GestureHandlers {
    /// No handlers at all.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// The gestures these handlers care about.
    #[inline]
    pub const fn mask(&self) -> GestureMask {
        self.mask
    }

    /// The gestures a handler was installed for, which own the press they are
    /// recognized on. The rest of [`mask`](Self::mask) is only observed.
    #[inline]
    pub const fn claimed(&self) -> GestureMask {
        self.claims
    }

    /// Whether a handler was installed for the drag itself.
    ///
    /// [`Self::claimed`] cannot answer this: `on_swipe` has to switch the drag
    /// recognizer on to measure a flick, so it claims [`GestureMask::DRAG`] too.
    /// A swipe moves nothing while the finger travels, so only `on_drag_start`,
    /// `on_drag_update` or `on_drag_end` own the pointer for the length of the
    /// drag.
    #[inline]
    pub fn owns_drag(&self) -> bool {
        self.on_drag_start.is_set() || self.on_drag_update.is_set() || self.on_drag_end.is_set()
    }

    /// Whether a scroll event should be consumed rather than left to fall
    /// through.
    ///
    /// A scroll carries no position, so this cannot be decided by hit test: a
    /// detector with no scroll handler must let the event reach whatever is
    /// behind it. Getting this wrong is why wheel scrolling once appeared dead
    /// — a handler-less detector on a top layer swallowed every scroll before a
    /// `Scrollable` below could see it.
    #[inline]
    pub const fn consumes_scroll(&self) -> bool {
        self.claims.intersects(GestureMask::SCROLL)
    }

    /// Whether a platform gesture of this kind should be consumed rather than
    /// left to fall through to the widgets behind the detector.
    ///
    /// The same distinction as [`Self::consumes_scroll`], for the same reason: a
    /// platform pinch carries no contact to capture, so it is delivered to every
    /// detector under the cursor, innermost first. A [`Button`] that watches its
    /// press through `on_gesture` must not swallow the trackpad pinch meant for
    /// the zoomable canvas it sits on. Only a handler installed for the gesture
    /// itself claims it.
    ///
    /// [`Button`]: crate::button::Button
    #[inline]
    pub const fn claims(&self, gestures: GestureMask) -> bool {
        self.claims.intersects(gestures)
    }

    /// Widens the mask, which is how the raw
    /// [`super::gesture_detector::GestureDetector::on_gesture`] handler declares
    /// interest in everything.
    #[inline]
    pub const fn observe(&mut self, gestures: GestureMask) {
        self.mask.insert(gestures);
    }

    /// Delivers one recognized gesture to whichever handlers want it.
    ///
    /// [`Self::on_gesture`] sees every gesture; the individual handlers are
    /// filters over the same stream, never a parallel path — which is what keeps
    /// the two from disagreeing about what happened.
    pub fn dispatch(&self, event: GestureEvent) {
        Self::call(&self.on_gesture, event);

        match event {
            // Only the main button activates. A secondary click goes to
            // `on_right_tap`, and a middle or extra button reaches the raw
            // handler only — before the button travelled with the pointer, a
            // right-click fired `on_tap`, which is the bug this replaced.
            GestureEvent::Tap { pointer } => match pointer.button {
                PointerButton::Primary => Self::call(&self.on_tap, ()),
                PointerButton::Secondary => Self::call(&self.on_right_tap, ()),
                _ => {}
            },
            GestureEvent::DoubleTap { pointer } if pointer.is_primary() => {
                Self::call(&self.on_double_press, ())
            }
            GestureEvent::LongPress { pointer } if pointer.is_primary() => {
                Self::call(&self.on_long_press, ())
            }
            GestureEvent::DragStart { pointer } => Self::call(&self.on_drag_start, pointer),
            GestureEvent::DragUpdate {
                pointer,
                delta_x,
                delta_y,
            } => Self::call(
                &self.on_drag_update,
                DragUpdateData {
                    position: pointer,
                    delta_x,
                    delta_y,
                },
            ),
            GestureEvent::DragEnd { .. } => Self::call(&self.on_drag_end, ()),
            GestureEvent::Swipe { direction, .. } => Self::call(&self.on_swipe, direction),
            GestureEvent::Scroll { delta_x, delta_y } => {
                Self::call(&self.on_scroll, ScrollData { delta_x, delta_y })
            }
            GestureEvent::ScaleUpdate {
                focal_x,
                focal_y,
                scale,
                delta_scale,
                rotation,
                delta_rotation,
                focal_delta_x,
                focal_delta_y,
            } => Self::call(
                &self.on_scale,
                ScaleData {
                    focal_x,
                    focal_y,
                    scale,
                    delta_scale,
                    rotation,
                    delta_rotation,
                    focal_delta_x,
                    focal_delta_y,
                },
            ),
            GestureEvent::PanUpdate {
                x,
                y,
                delta_x,
                delta_y,
            } => Self::call(
                &self.on_pan,
                PanData {
                    x,
                    y,
                    delta_x,
                    delta_y,
                },
            ),
            GestureEvent::SmartZoom { x, y } => {
                Self::call(&self.on_smart_zoom, SmartZoomData { x, y })
            }
            // The press and long-press lifecycles, drag cancellation, and the
            // pinch and pan boundaries have no sugar setter of their own: they are read
            // through `on_gesture`, which has already been called above.
            _ => {}
        }
    }

    /// Delivers every gesture a pointer event produced, in order.
    #[inline]
    pub fn dispatch_all(&self, output: GestureOutput) {
        for event in output {
            self.dispatch(event);
        }
    }

    /// Invokes one callback, sync or async, whatever its argument type.
    ///
    /// Generic over [`CallbackExecutor`] rather than over a concrete callback
    /// type, because a no-argument [`VoidCallback`] and a parameterised one are
    /// different types that share only that trait. A gesture handler returns
    /// nothing, so there is no synchronous result to pass back on.
    #[inline]
    fn call<C>(callback: &C, arg: C::Args)
    where
        C: CallbackExecutor<Output = ()>,
    {
        callback.execute(arg);
    }
}

/// Generates the mask-maintaining setters, so a handler cannot be installed
/// without the recognizer being told to look for its gesture.
macro_rules! handler_setters {
    ($($setter:ident => $field:ident : $callback:ty, $gestures:expr, $doc:expr;)*) => {
        impl GestureHandlers {
            $(
                #[doc = $doc]
                #[inline]
                pub fn $setter(&mut self, callback: impl Into<$callback>) {
                    self.$field = callback.into();
                    self.mask.insert($gestures);
                    self.claims.insert($gestures);
                }
            )*
        }
    };
}

handler_setters! {
    set_on_tap => on_tap: VoidCallback, GestureMask::TAP,
        "Installs the handler for a completed primary tap.";
    set_on_double_press => on_double_press: VoidCallback,
        GestureMask::TAP.union(GestureMask::DOUBLE_TAP),
        "Installs the handler for two qualifying taps inside the double-tap timeout.";
    set_on_long_press => on_long_press: VoidCallback, GestureMask::LONG_PRESS,
        "Installs the handler for a press held past the long-press threshold.";
    set_on_drag_start => on_drag_start: DragCallback, GestureMask::DRAG,
        "Installs the handler for movement crossing the device's slop.";
    set_on_drag_update => on_drag_update: DragUpdateCallback, GestureMask::DRAG,
        "Installs the handler for movement during an active drag.";
    set_on_drag_end => on_drag_end: VoidCallback, GestureMask::DRAG,
        "Installs the handler for the release that ends a drag.";
    set_on_right_tap => on_right_tap: VoidCallback, GestureMask::TAP,
        "Installs the handler for a completed secondary-button tap.";
    set_on_swipe => on_swipe: SwipeCallback,
        GestureMask::DRAG.union(GestureMask::SWIPE),
        "Installs the handler for a drag fast enough to read as a flick.";
    set_on_scroll => on_scroll: ScrollCallback, GestureMask::SCROLL,
        "Installs the handler for wheel and trackpad scrolling, which also makes the detector consume those events.";
    set_on_scale => on_scale: ScaleCallback, GestureMask::SCALE,
        "Installs the handler for a two-pointer pinch.";
    set_on_pan => on_pan: PanCallback, GestureMask::PAN,
        "Installs the handler for a platform multi-finger pan, which also makes the detector consume those events.";
    set_on_smart_zoom => on_smart_zoom: SmartZoomCallback, GestureMask::SMART_ZOOM,
        "Installs the handler for the platform's smart-zoom double tap, which also makes the detector consume those events.";
}

impl GestureHandlers {
    /// Installs the handler that sees every recognized gesture, including the
    /// ones with no sugar setter of their own.
    ///
    /// It *observes* without *claiming*: scrolls and platform gestures are still
    /// reported to it where the detector receives them, but they are not consumed
    /// on its account — see [`GestureMask::EVERY_POINTER_GESTURE`] and
    /// [`Self::claims`].
    #[inline]
    pub fn set_on_gesture(&mut self, callback: impl Into<GestureStreamCallback>) {
        self.on_gesture = callback.into();
        self.mask.insert(GestureMask::EVERY_POINTER_GESTURE);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use aimer_attribute::position::Vec2d;
    use aimer_events::pointer::PointerInfo;

    use super::*;
    use crate::gesture::SwipeDirection;

    fn mouse(button: PointerButton) -> PointerInfo {
        PointerInfo::mouse(Vec2d { x: 4.0, y: 4.0 }, button)
    }

    /// Records which handler names fired, in order.
    fn recorder() -> (Rc<RefCell<Vec<&'static str>>>, GestureHandlers) {
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let mut handlers = GestureHandlers::new();

        let tap = log.clone();
        handlers.set_on_tap(move || tap.borrow_mut().push("tap"));
        let right = log.clone();
        handlers.set_on_right_tap(move || right.borrow_mut().push("right_tap"));
        let double = log.clone();
        handlers.set_on_double_press(move || double.borrow_mut().push("double"));
        let long = log.clone();
        handlers.set_on_long_press(move || long.borrow_mut().push("long"));
        let start = log.clone();
        handlers.set_on_drag_start(move |_: PointerInfo| start.borrow_mut().push("drag_start"));
        let end = log.clone();
        handlers.set_on_drag_end(move || end.borrow_mut().push("drag_end"));
        let swipe = log.clone();
        handlers.set_on_swipe(move |_: SwipeDirection| swipe.borrow_mut().push("swipe"));

        (log, handlers)
    }

    #[test]
    fn no_handlers_means_an_empty_mask_and_no_scroll_consumed() {
        let handlers = GestureHandlers::new();

        assert!(handlers.mask().is_empty());
        assert!(!handlers.consumes_scroll());
    }

    #[test]
    fn installing_a_handler_declares_interest_in_its_gesture() {
        let mut handlers = GestureHandlers::new();
        handlers.set_on_scale(|_: ScaleData| {});

        assert!(handlers.mask().contains(GestureMask::SCALE));
        assert!(!handlers.mask().contains(GestureMask::TAP));
    }

    // A swipe is only ever produced by a drag, so listening for one has to make
    // the recognizer track drags even when nothing listens for the drag itself.
    #[test]
    fn listening_for_a_swipe_also_asks_for_drag_tracking() {
        let mut handlers = GestureHandlers::new();
        handlers.set_on_swipe(|_: SwipeDirection| {});

        assert!(handlers.mask().contains(GestureMask::DRAG));
        assert!(handlers.mask().contains(GestureMask::SWIPE));
    }

    // The raw handler observes everything, but observing a scroll is not the same
    // as claiming it: a `Button` that watches the press lifecycle must not start
    // swallowing the wheel scrolls of the list it sits in.
    #[test]
    fn the_raw_handler_asks_for_every_gesture_but_claims_no_scroll() {
        let mut handlers = GestureHandlers::new();
        handlers.set_on_gesture(|_: GestureEvent| {});

        assert!(handlers.mask().contains(GestureMask::SCALE));
        assert!(handlers.mask().contains(GestureMask::LONG_PRESS));
        assert!(handlers.mask().contains(GestureMask::PRESS));
        assert!(!handlers.consumes_scroll());
    }

    #[test]
    fn only_a_scroll_handler_makes_a_detector_consume_scrolls() {
        let mut handlers = GestureHandlers::new();
        handlers.set_on_tap(|| {});

        assert!(
            !handlers.consumes_scroll(),
            "a handler-less detector must let the scroll reach lower layers"
        );

        handlers.set_on_scroll(|_: ScrollData| {});

        assert!(handlers.consumes_scroll());
    }

    // The headline bug: a right-click used to fire `on_tap`, because the button
    // never reached the recognizer at all.
    #[test]
    fn a_secondary_tap_fires_only_the_secondary_handler() {
        let (log, handlers) = recorder();

        handlers.dispatch(GestureEvent::Tap {
            pointer: mouse(PointerButton::Secondary),
        });

        assert_eq!(*log.borrow(), vec!["right_tap"]);
    }

    #[test]
    fn a_primary_tap_fires_only_the_primary_handler() {
        let (log, handlers) = recorder();

        handlers.dispatch(GestureEvent::Tap {
            pointer: mouse(PointerButton::Primary),
        });

        assert_eq!(*log.borrow(), vec!["tap"]);
    }

    // Middle-click was dropped by the platform layer entirely. It now arrives,
    // and reaches the raw handler without pretending to be an activation.
    #[test]
    fn a_middle_tap_fires_no_sugar_handler() {
        let (log, handlers) = recorder();

        handlers.dispatch(GestureEvent::Tap {
            pointer: mouse(PointerButton::Middle),
        });

        assert!(log.borrow().is_empty());
    }

    #[test]
    fn the_raw_handler_sees_gestures_that_have_no_sugar_setter() {
        let seen: Rc<RefCell<Vec<GestureEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let mut handlers = GestureHandlers::new();
        let recorded = seen.clone();
        handlers.set_on_gesture(move |event: GestureEvent| recorded.borrow_mut().push(event));

        let pointer = mouse(PointerButton::Primary);
        handlers.dispatch(GestureEvent::TapDown { pointer });
        handlers.dispatch(GestureEvent::TapCancel);
        handlers.dispatch(GestureEvent::DragCancel);
        handlers.dispatch(GestureEvent::LongPressStart { pointer });

        assert_eq!(
            *seen.borrow(),
            vec![
                GestureEvent::TapDown { pointer },
                GestureEvent::TapCancel,
                GestureEvent::DragCancel,
                GestureEvent::LongPressStart { pointer },
            ]
        );
    }

    // The old state machine called `on_drag_end` and then reported only the
    // swipe. Both handlers must fire, in order, from one release.
    #[test]
    fn a_flick_fires_the_drag_end_and_the_swipe_in_order() {
        let (log, handlers) = recorder();
        let pointer = mouse(PointerButton::Primary);
        let mut output = GestureOutput::new();
        output.push(GestureEvent::DragEnd { pointer });
        output.push(GestureEvent::Swipe {
            direction: SwipeDirection::Right,
            velocity_x: 900.0,
            velocity_y: 0.0,
        });

        handlers.dispatch_all(output);

        assert_eq!(*log.borrow(), vec!["drag_end", "swipe"]);
    }

    #[test]
    fn dispatching_to_a_missing_handler_is_a_no_op() {
        let handlers = GestureHandlers::new();

        handlers.dispatch(GestureEvent::Tap {
            pointer: mouse(PointerButton::Primary),
        });
    }

    #[test]
    fn installing_pan_and_smart_zoom_declares_interest_and_claims_them() {
        let mut handlers = GestureHandlers::new();
        handlers.set_on_pan(|_: PanData| {});
        handlers.set_on_smart_zoom(|_: SmartZoomData| {});

        assert!(handlers.mask().contains(GestureMask::PAN));
        assert!(handlers.mask().contains(GestureMask::SMART_ZOOM));
        assert!(handlers.claims(GestureMask::PAN));
        assert!(handlers.claims(GestureMask::SMART_ZOOM));
        assert!(!handlers.claims(GestureMask::SCALE));
    }

    // The raw handler listens for everything, but it claims nothing: a Button
    // observing its press through it must not swallow a pinch, a pan or a scroll.
    #[test]
    fn the_raw_handler_observes_every_platform_gesture_but_claims_none() {
        let mut handlers = GestureHandlers::new();
        handlers.set_on_gesture(|_: GestureEvent| {});

        for gesture in [
            GestureMask::SCALE,
            GestureMask::PAN,
            GestureMask::SMART_ZOOM,
        ] {
            assert!(handlers.mask().contains(gesture));
            assert!(!handlers.claims(gesture));
        }
    }

    #[test]
    fn a_scale_update_hands_the_whole_transform_to_on_scale() {
        let seen: Rc<RefCell<Vec<ScaleData>>> = Rc::default();
        let recorded = seen.clone();
        let mut handlers = GestureHandlers::new();
        handlers.set_on_scale(move |data: ScaleData| recorded.borrow_mut().push(data));

        handlers.dispatch(GestureEvent::ScaleUpdate {
            focal_x: 1.0,
            focal_y: 2.0,
            scale: 3.0,
            delta_scale: 1.5,
            rotation: 0.5,
            delta_rotation: 0.25,
            focal_delta_x: 4.0,
            focal_delta_y: -4.0,
        });

        assert_eq!(
            *seen.borrow(),
            vec![ScaleData {
                focal_x: 1.0,
                focal_y: 2.0,
                scale: 3.0,
                delta_scale: 1.5,
                rotation: 0.5,
                delta_rotation: 0.25,
                focal_delta_x: 4.0,
                focal_delta_y: -4.0,
            }]
        );
    }

    #[test]
    fn pan_updates_and_smart_zoom_reach_their_handlers() {
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        let mut handlers = GestureHandlers::new();
        let pan = log.clone();
        handlers.set_on_pan(move |_: PanData| pan.borrow_mut().push("pan"));
        let zoom = log.clone();
        handlers.set_on_smart_zoom(move |_: SmartZoomData| zoom.borrow_mut().push("zoom"));

        handlers.dispatch(GestureEvent::PanStart { x: 0.0, y: 0.0 });
        handlers.dispatch(GestureEvent::PanUpdate {
            x: 0.0,
            y: 0.0,
            delta_x: 1.0,
            delta_y: 1.0,
        });
        handlers.dispatch(GestureEvent::PanEnd);
        handlers.dispatch(GestureEvent::SmartZoom { x: 0.0, y: 0.0 });

        assert_eq!(*log.borrow(), vec!["pan", "zoom"]);
    }
}
