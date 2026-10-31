use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::Curve;
use aimer_attribute::position::Vec2d;
use aimer_utils::callback::Callback;

use super::controller::ScrollState;

/// A programmable, app-held handle to a [`Scrollable`](crate::Scrollable)'s
/// scroll position — the framework's equivalent of Flutter's
/// `ScrollController`.
///
/// Create one in your widget/state, pass it to `Scrollable`'s `controller`
/// field, and keep it across rebuilds. It lets application code read the live
/// position ([`offset`](Self::offset) / [`max_extent`](Self::max_extent)) and
/// drive it programmatically ([`jump_to`](Self::jump_to) /
/// [`animate_to`](Self::animate_to)).
///
/// Positions are expressed in logical (unscaled) pixels, positive toward the
/// content end (i.e. `jump_to(100)` scrolls 100px down in a vertical list),
/// matching the Flutter mental model.
///
/// The handle is cheap to [`Clone`] — clones share the same underlying state —
/// and is single-threaded (UI thread only); it is intentionally not `Send`.
#[derive(Clone, Default)]
pub struct ScrollController {
    inner: Rc<ScrollControllerInner>,
}

#[derive(Default)]
struct ScrollControllerInner {
    /// The live engine, attached by `Scrollable::to_element` once the widget is
    /// built. `None` before the first build.
    state: RefCell<Option<Rc<ScrollState>>>,
    /// A position requested before the controller was attached (e.g. `jump_to`
    /// called before the first build). Applied on [`ScrollController::attach`].
    pending: Cell<Option<Vec2d>>,
    /// App callback fired when a scroll session begins. Kept on the controller
    /// (not just the element) so it survives rebuilds and is re-shared into
    /// each freshly built [`ScrollState`] on attach.
    on_scroll_start: RefCell<Callback<Vec2d>>,
    /// App callback fired when a scroll session fully settles.
    on_scroll_end: RefCell<Callback<Vec2d>>,
    /// App callback fired on every frame the offset moves. Kept on the
    /// controller so it survives rebuilds and is re-shared into each freshly
    /// built [`ScrollState`] on attach.
    on_scroll: RefCell<Callback<Vec2d>>,
}

impl ScrollController {
    /// Create a new, detached controller. Attach it by passing it to a
    /// `Scrollable`'s `controller` field.
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub(crate) fn shares_identity_with(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// Whether this controller is currently attached to a live `Scrollable`.
    pub fn is_attached(&self) -> bool {
        self.inner.state.borrow().is_some()
    }

    /// Attach (or re-attach, on rebuild) the live scroll engine. Applies any
    /// position requested before attachment.
    ///
    /// Re-attaching the engine already bound is a pointer comparison, so every
    /// build of a `Scrollable` can offer the engine it is about to draw with
    /// without paying for it. That is what keeps the controller pointing at a
    /// live engine: a rebuild may create an engine it then discards in favour of
    /// the one it carried over, and the controller has to follow the survivor.
    pub(crate) fn attach(&self, state: Rc<ScrollState>) {
        let previous = self.inner.state.borrow().clone();
        if let Some(previous) = previous {
            if Rc::ptr_eq(&previous, &state) {
                return;
            }
            state.adopt_scroll_state(&previous);
        }
        if let Some(pos) = self.inner.pending.take() {
            let scale = state.last_scale.get().max(f32::EPSILON);
            state.set_scroll_offset(Vec2d {
                x: -pos.x * scale,
                y: -pos.y * scale,
            });
        }
        // Re-share the app's scroll-lifecycle callbacks into the freshly built
        // engine so they keep firing across rebuilds.
        *state.session.on_start.borrow_mut() = self.inner.on_scroll_start.borrow().clone();
        *state.session.on_end.borrow_mut() = self.inner.on_scroll_end.borrow().clone();
        *state.session.on_scroll.borrow_mut() = self.inner.on_scroll.borrow().clone();
        *self.inner.state.borrow_mut() = Some(state);
    }

    fn with_state<R>(&self, f: impl FnOnce(&Rc<ScrollState>) -> R) -> Option<R> {
        self.inner.state.borrow().as_ref().map(f)
    }

    /// The current scroll position in logical (unscaled) pixels, positive
    /// toward the content end. Returns the pending/initial position while
    /// detached. The component perpendicular to the configured scroll axis is
    /// always zero.
    pub fn offset(&self) -> Vec2d {
        self.with_state(|s| s.logical_offset())
            .unwrap_or_else(|| self.inner.pending.get().unwrap_or_default())
    }

    /// How opaque the scroll bar is right now, from `0.0` when it is hidden to
    /// `1.0` when it is fully shown.
    ///
    /// The bar shows itself while the content scrolls and while a pointer is on
    /// it, and fades out by itself once neither is the case. Zero while
    /// detached, before the first paint, or when the content fits.
    pub fn scroll_bar_opacity(&self) -> f32 {
        self.with_state(|s| s.bar.opacity.get()).unwrap_or(0.0)
    }

    /// Whether a finger currently holds the scroll bar thumb: it rested on it
    /// long enough to take it, and has not lifted. `false` while detached.
    pub fn is_thumb_grabbed(&self) -> bool {
        self.with_state(|s| s.thumb_armed()).unwrap_or(false)
    }

    /// The maximum scrollable extent per axis in logical pixels. Zero while
    /// detached or before the first layout.
    pub fn max_extent(&self) -> Vec2d {
        self.with_state(|s| {
            let scale = s.last_scale.get().max(f32::EPSILON);
            let m = s.cached_max_scroll.get();
            Vec2d {
                x: m.x / scale,
                y: m.y / scale,
            }
        })
        .unwrap_or_default()
    }

    /// Jump instantly to `position` (logical pixels, positive toward the
    /// content end), clamped to the valid range, and request a repaint. If
    /// the controller is not yet attached, the position is remembered and
    /// applied on attachment.
    pub fn jump_to(&self, position: Vec2d) {
        let Some(state) = self.inner.state.borrow().clone() else {
            self.inner.pending.set(Some(position));
            return;
        };

        let scale = state.last_scale.get().max(f32::EPSILON);
        let internal = ScrollState::axis_only(
            state.axis,
            Vec2d {
                x: -position.x * scale,
                y: -position.y * scale,
            },
        );
        state.cancel_fling();
        state.pointer_velocity.set(Vec2d { x: 0.0, y: 0.0 });
        state.fling.spring_velocity.set(Vec2d { x: 0.0, y: 0.0 });
        state.release_overscroll_recovery();
        state.reset_overscroll_peak();
        // An instant jump is a self-contained scroll session: fire the
        // start/end edges around the position change so listeners still see
        // a matched pair even though no frames elapse.
        state.begin_scroll();
        state.set_scroll_offset(state.clamp_offset(internal));
        state.end_scroll();
        state.request_animation_frame();
    }

    /// Animate to `position` (logical pixels, positive toward the content end)
    /// over `duration`, easing with `curve`, and request repaints. If the
    /// controller is not yet attached, the target is remembered as the initial
    /// position (no animation) and applied on attachment.
    pub fn animate_to(&self, position: Vec2d, duration: Duration, curve: impl Curve) {
        let Some(state) = self.inner.state.borrow().clone() else {
            self.inner.pending.set(Some(position));
            return;
        };

        let scale = state.last_scale.get().max(f32::EPSILON);
        let target = ScrollState::axis_only(
            state.axis,
            Vec2d {
                x: -position.x * scale,
                y: -position.y * scale,
            },
        );
        // Announce the session now; the draw loop fires `end` once the
        // animation settles. A zero-duration animation degenerates to an
        // instant jump, which the draw loop then reports as settled.
        state.begin_scroll();
        state.start_animation(target, duration.as_secs_f32(), curve);
        state.request_animation_frame();
    }

    /// Register a callback fired once each time a scroll session **begins** —
    /// the moment the view goes from at-rest to moving, whether from a user
    /// drag, a wheel/trackpad/keyboard scroll, or a programmatic
    /// [`jump_to`](Self::jump_to) / [`animate_to`](Self::animate_to). The
    /// callback receives the logical offset at the start of the session.
    ///
    /// Only one start callback is kept; registering again replaces the previous
    /// one. The callback survives widget rebuilds. This is the framework's
    /// equivalent of Flutter's `ScrollStartNotification`.
    ///
    /// Accepts anything convertible into a [`Callback<Vec2d>`] — a plain
    /// closure `|offset: Vec2d| { .. }` or a pre-built `Callback`.
    pub fn on_scroll_start(&self, callback: impl Into<Callback<Vec2d>>) {
        let cb: Callback<Vec2d> = callback.into();
        *self.inner.on_scroll_start.borrow_mut() = cb.clone();
        self.with_state(|s| {
            *s.session.on_start.borrow_mut() = cb.clone();
        });
    }

    /// Register a callback fired once each time a scroll session **ends** — the
    /// moment the view comes fully to rest after any momentum, fling, or
    /// spring-back has settled. The callback receives the resting logical
    /// offset.
    ///
    /// Only one end callback is kept; registering again replaces the previous
    /// one. The callback survives widget rebuilds. This is the framework's
    /// equivalent of Flutter's `ScrollEndNotification`.
    ///
    /// Accepts anything convertible into a [`Callback<Vec2d>`] — a plain
    /// closure `|offset: Vec2d| { .. }` or a pre-built `Callback`.
    pub fn on_scroll_end(&self, callback: impl Into<Callback<Vec2d>>) {
        let cb: Callback<Vec2d> = callback.into();
        *self.inner.on_scroll_end.borrow_mut() = cb.clone();
        self.with_state(|s| {
            *s.session.on_end.borrow_mut() = cb.clone();
        });
    }

    /// Register a callback fired on **every frame the scroll position moves** —
    /// the live, per-frame counterpart to
    /// [`on_scroll_start`](Self::on_scroll_start)
    /// / [`on_scroll_end`](Self::on_scroll_end). It fires for user drags,
    /// wheel/trackpad/keyboard scrolls, release momentum, spring-back, and
    /// programmatic [`jump_to`](Self::jump_to) /
    /// [`animate_to`](Self::animate_to), receiving the current logical
    /// offset each time. Idle frames and sub-pixel jitter are suppressed,
    /// so it only reports genuine movement.
    ///
    /// Only one callback is kept; registering again replaces the previous one.
    /// The callback survives widget rebuilds. This is the framework's
    /// equivalent of Flutter's `ScrollUpdateNotification`.
    ///
    /// Accepts anything convertible into a [`Callback<Vec2d>`] — a plain
    /// closure `|offset: Vec2d| { .. }` or a pre-built `Callback`.
    pub fn on_scroll(&self, callback: impl Into<Callback<Vec2d>>) {
        let cb: Callback<Vec2d> = callback.into();
        *self.inner.on_scroll.borrow_mut() = cb.clone();
        self.with_state(|s| {
            *s.session.on_scroll.borrow_mut() = cb.clone();
        });
    }
}
