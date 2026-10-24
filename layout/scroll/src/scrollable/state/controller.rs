use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::{AnyCurve, Curve};
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_utils::AnimInstant;
use aimer_utils::callback::Callback;
use aimer_widget::Key;

use crate::scrollable::constants::*;
use crate::scrollable::overscroll_source::{OverscrollSource, OverscrollSources};
use crate::scrollable::scroll_behavior::ScrollBehavior;
use crate::scrollable::spring::advance_overscroll_spring;
use crate::scrollable::velocity_history::VelocityHistory;
use crate::scrollable::{ScrollAxis, ScrollBarPlacement};

/// Minimum logical-pixel movement between two frames for `on_scroll` to fire.
/// Collapses idle frames and sub-pixel jitter to no-ops so the per-frame
/// callback only reports genuine scrolling.
pub(crate) const SCROLL_NOTIFY_EPSILON: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragMode {
    None = 0,
    Content = 1,
    VerticalScrollbar = 2,
    HorizontalScrollbar = 3,
    Pending = 4,
}

impl ScrollState {
    /// Keep scroll state on the axis this engine owns.
    #[inline]
    pub(crate) fn axis_only(axis: ScrollAxis, value: Vec2d) -> Vec2d {
        match axis {
            ScrollAxis::Vertical => Vec2d { x: 0.0, y: value.y },
            ScrollAxis::Horizontal => Vec2d { x: value.x, y: 0.0 },
        }
    }

    pub fn offset(&self) -> Vec2d {
        Self::axis_only(self.axis, self.scroll_offset.get())
    }

    /// Stores a new internal offset and records only actual position changes.
    ///
    /// Tests and state adoption may write the cell directly because they are
    /// establishing a fixture or transferring existing state. Runtime scroll
    /// paths use this boundary so Phase 0 can distinguish an offset update
    /// from a frame that merely redraws an unchanged scrollable.
    #[inline]
    pub(crate) fn set_scroll_offset(&self, offset: Vec2d) {
        let offset = Self::axis_only(self.axis, offset);
        let changed = self.scroll_offset.get() != offset;
        self.scroll_offset.set(offset);
        if changed {
            aimer_widget::record_scroll_offset_update();
            #[cfg(target_arch = "wasm32")]
            // Scrolling changes the transform of an arbitrary child subtree.
            // Its old and new footprints are not available until that subtree
            // is painted, so a browser frame must not preserve a partial
            // compositor target across this event.
            aimer_widget::mark_paint_damage_full();
        }
    }

    /// Request the display-synchronized frame that consumes the latest scroll
    /// state. Keeping this on the live state makes scroll input and momentum
    /// use the same platform-safe scheduling path.
    #[inline]
    pub(crate) fn request_animation_frame(&self) {
        // The only thing this frame has to do is apply the next scroll step, so
        // it says so: the frame loop may then leave alone what the offset does
        // not move.
        aimer_events::window::request_scroll_frame();
    }
}

pub struct ScrollState {
    pub(crate) scroll_behavior: ScrollBehavior,
    /// Which input devices this target lets stretch a bouncy edge.
    ///
    /// Resolved once when the engine is built (see
    /// [`Scrollable::overscroll_sources`](crate::Scrollable::overscroll_sources))
    /// and never mutated, so reading it costs a load and a mask.
    pub(crate) overscroll_sources: OverscrollSources,
    /// The device that drove the most recent scroll input, which is the one
    /// [`bouncy`](Self::bouncy) is resolved against.
    ///
    /// Momentum, spring-back and every other frame the draw loop produces
    /// belong to the gesture that started them, so this deliberately survives
    /// until the next input arrives.
    pub(crate) overscroll_source: Cell<OverscrollSource>,
    pub(crate) axis: ScrollAxis,
    /// Whether the scrollbars overlap the viewport or reserve its edge.
    pub(crate) scroll_bar_placement: ScrollBarPlacement,
    pub(crate) scroll_offset: Cell<Vec2d>,
    /// Identity used for `PageStorage`-style offset persistence when
    /// `remember_scroll_offset` is enabled.
    pub(crate) storage_key: Key,
    /// Whether `storage_key` participates in teardown persistence.
    pub(crate) remember_scroll_offset: bool,
    pub(crate) last_pointer_pos: Cell<Option<Vec2d>>,
    pub(crate) drag_mode: Cell<DragMode>,
    pub(crate) cached_max_scroll: Cell<Vec2d>,
    pub(crate) cached_min_scroll: Cell<Vec2d>,
    pub(crate) pointer_velocity: Cell<Vec2d>,
    pub(crate) last_event_time: Cell<Option<AnimInstant>>,
    pub(crate) last_frame_time: Cell<Option<AnimInstant>>,
    pub(crate) v_thumb_rect: Cell<Option<(f32, f32, f32, f32)>>,
    pub(crate) h_thumb_rect: Cell<Option<(f32, f32, f32, f32)>>,
    /// Whether the cursor was over a scrollbar thumb after the last pointer
    /// move, so the move that crosses a thumb edge — and only that move — can
    /// schedule the frame that repaints the highlight (see
    /// [`sync_thumb_hover`](Self::sync_thumb_hover)).
    pub(crate) thumb_hovered: Cell<bool>,
    pub(crate) v_scroll_multiplier: Cell<f32>,
    pub(crate) h_scroll_multiplier: Cell<f32>,
    pub(crate) last_scale: Cell<f32>,
    pub(crate) speed_multiplier: f32,
    pub(crate) cursor_pos: Cell<Option<Vec2d>>,
    /// Allocated on the first drag sample. Keeping the fixed-size ring inline
    /// here made every idle scrollable carry its full sample buffer.
    pub(crate) velocity_history: RefCell<Option<Box<VelocityHistory>>>,
    pub(crate) cached_viewport: Cell<(f32, f32)>,
    pub(crate) cached_v_track_width: Cell<f32>,
    pub(crate) cached_h_track_width: Cell<f32>,
    pub(crate) key_scroll_strength: f32,
    /// Furthest signed per-axis distance the content was stretched past an
    /// edge during the current overscroll episode, in the same convention as
    /// [`scroll_offset`](Self::scroll_offset).
    ///
    /// This high-water mark rejects weaker outward tail deltas after recovery
    /// starts. It survives reaching the boundary and is cleared when the
    /// device gesture terminates, a new gesture starts, or it goes idle (see
    /// [`overscroll_peak_at`](Self::overscroll_peak_at)).
    pub(crate) highest_overscroll_offset: Cell<Vec2d>,
    /// Instant the overscroll high-water mark was last raised or measured
    /// against a device delta, or `None` when no mark is live.
    ///
    /// The mark is only meaningful while the gesture that produced it can still
    /// have events in flight, so it is timed exactly like the recovery hold:
    /// after [`OVERSCROLL_PEAK_IDLE_MS`] without a single device event the
    /// episode is over and the mark is dropped. Without this the mark depends
    /// on gesture-boundary phases alone, and a boundary the platform never
    /// delivers (a smoothing channel that stays active across two gestures, a
    /// lift swallowed by a focus change) leaves the edge permanently unable to
    /// stretch.
    pub(crate) overscroll_peak_at: Cell<Option<AnimInstant>>,
    /// Whether the last delivered scroll frame reported fingers on the device.
    ///
    /// Used to detect the rising edge of physical contact, which starts a new
    /// overscroll episode regardless of the phase the smoothing layer chose to
    /// label that frame with.
    pub(crate) device_contact: Cell<bool>,
    /// Recognizes the browser's post-lift momentum tail, which is the only
    /// lift signal a web scroll gesture ever gets.
    ///
    /// Native platforms report contact themselves, so this exists on the web
    /// target only and no other target pays for it.
    #[cfg(target_arch = "wasm32")]
    pub(crate) web_overscroll_decay: crate::scrollable::web_overscroll::WebOverscrollDecay,
    /// Closes the synthesized web gesture once its bouncy edge has recovered,
    /// and discards whatever the browser still delivers for it.
    ///
    /// Native platforms end their own gestures, so this exists on the web
    /// target only and no other target pays for it.
    #[cfg(target_arch = "wasm32")]
    pub(crate) web_recovery_end: crate::scrollable::web_recovery_end::WebRecoveryEnd,
    /// Latest content size observed by the scroll container.
    pub(crate) cached_content_size: Cell<ResolvedSize>,
    /// Whether `cached_content_size` is initialized and can be used as a
    /// previous-frame extent hint while new content is verified.
    pub(crate) cached_content_size_valid: Cell<bool>,
    /// Wall-clock instant the current release fling started, or `None` when no
    /// cubic-bézier fling is active. While `Some`, momentum is driven by the
    /// curve rather than by per-frame velocity decay.
    pub(crate) fling_start_time: Cell<Option<AnimInstant>>,
    /// Scroll offset captured at the moment the fling started.
    pub(crate) fling_start_offset: Cell<Vec2d>,
    /// Scroll offset the fling eases toward (`start + projected distance`).
    pub(crate) fling_target_offset: Cell<Vec2d>,
    /// Total duration (seconds) of the active bézier fling.
    pub(crate) fling_duration: Cell<f32>,
    /// Optional easing curve for a programmatic `animate_to`. When `Some`, the
    /// active fling interpolates through this curve instead of the default
    /// release-fling bézier; `None` = a normal touch/mouse release fling.
    pub(crate) anim_curve: RefCell<Option<AnyCurve>>,
    /// Primary touch finger ID that owns this scrollable, or `None` for mouse.
    /// Set on first PointerDown, cleared on PointerUp/Cancel.
    pub(crate) active_touch_id: Cell<Option<u64>>,
    /// Velocity of the spring-back oscillation (px/s). Separate from
    /// `pointer_velocity` (which carries momentum / drag velocity) so the
    /// damped-spring simulation can overshoot and oscillate independently.
    pub(crate) spring_velocity: Cell<Vec2d>,
    /// Wall-clock instant the velocity-based momentum started (set on the
    /// first `update_momentum` frame where velocity exceeds epsilon after a
    /// touch release).  Used to hard-cap the glide at
    /// [`MAX_MOMENTUM_DURATION_S`] so it doesn't creep for 15–20 s.
    pub(crate) momentum_start_time: Cell<Option<AnimInstant>>,
    /// Finger delta accumulated since the last emitted drag-velocity sample.
    /// Coalesced same-frame pointer moves (web) fold in here instead of each
    /// producing its own inflated sample; flushed once a real time slice
    /// ([`VELOCITY_SAMPLE_MIN_DT`]) has elapsed.
    pub(crate) vel_accum: Cell<Vec2d>,
    /// Wall-clock instant of the last emitted drag-velocity sample, or `None`
    /// before the first sample of a gesture.
    pub(crate) vel_sample_time: Cell<Option<AnimInstant>>,
    /// Whether a scroll session is currently in progress — a user drag, a
    /// wheel/ keyboard scroll, release momentum, a spring-back, or a
    /// programmatic animation. Latches the `on_scroll_start` /
    /// `on_scroll_end` callbacks so each fires exactly once per session (on
    /// the idle↔scrolling edge), never once per frame.
    pub(crate) is_scrolling: Cell<bool>,
    /// Debug-only counters for one scroll session. They live on the shared
    /// state because input and draw paths both contribute to the same gesture.
    #[cfg(debug_assertions)]
    pub(crate) session_input_events: Cell<u64>,
    #[cfg(debug_assertions)]
    pub(crate) session_draw_frames: Cell<u64>,
    #[cfg(debug_assertions)]
    pub(crate) session_moved_frames: Cell<u64>,
    /// Fired once when a scroll session begins (idle → scrolling); receives the
    /// current logical offset. Shared in from the [`ScrollController`] on
    /// attach.
    pub(crate) on_scroll_start: RefCell<Callback<Vec2d>>,
    /// Fired once when a scroll session ends (scrolling → idle, after momentum
    /// and any spring-back have fully settled); receives the resting offset.
    pub(crate) on_scroll_end: RefCell<Callback<Vec2d>>,
    /// Fired on **every** frame where the logical offset actually moved (user
    /// drag, wheel/keyboard, momentum/fling, spring-back, or a programmatic
    /// `jump_to`/`animate_to`) — the level-triggered counterpart to the
    /// edge-triggered start/end pair. Receives the live logical offset. Shared
    /// in from the [`ScrollController`] on attach.
    pub(crate) on_scroll: RefCell<Callback<Vec2d>>,
    /// Last logical offset reported through [`on_scroll`](Self::on_scroll),
    /// used to fire only on genuine movement. `None` until the first frame
    /// establishes the baseline (so the initial render never fires
    /// `on_scroll`).
    pub(crate) last_reported_offset: Cell<Option<Vec2d>>,
    /// Visual offset the previous frame was drawn with, or `None` before the
    /// first draw.
    ///
    /// Its delta is how far the viewport travelled in one frame, which is what
    /// biases the cache extent toward the direction of travel (see
    /// [`cache_window`](crate::scrollable::cache_extent::cache_window)). Taken
    /// from the drawn offset rather than from
    /// [`pointer_velocity`](Self::pointer_velocity) because every way the
    /// viewport moves — drag, wheel, momentum, bézier fling, spring-back,
    /// `animate_to` — ends up there, while only some of them go through a
    /// velocity.
    pub(crate) last_drawn_offset: Cell<Option<Vec2d>>,
    /// Instant when a wheel / trackpad gesture first crossed a scroll boundary,
    /// or `None` when no recovery grace period is active.
    ///
    /// While this contains a recent timestamp, bouncy edge recovery is held so
    /// the first overscroll event can render without immediately fighting the
    /// spring. Later events do not refresh this grace period.
    ///
    /// The timestamp is what makes the hold self-terminating: see
    /// [`overscroll_recovery_held_at`](Self::overscroll_recovery_held_at).
    pub(crate) overscroll_hold: Cell<Option<AnimInstant>>,
    /// Whether active trackpad contact is holding the stretched edge.
    ///
    /// Unlike [`overscroll_hold`](Self::overscroll_hold), this does not expire:
    /// the platform's direct-manipulation signal releases it exactly when the
    /// user lifts their fingers. Post-lift momentum never sets it.
    pub(crate) direct_overscroll_hold: Cell<bool>,
}

impl ScrollState {
    /// Adopt the live interaction and animation state from a previous scroll
    /// engine.
    ///
    /// Used during reconciliation: when a parent rebuild produces a fresh
    /// scrollable, the newly built controller copies the offset from the old
    /// one so the viewport stays where the user left it instead of snapping
    /// to the top. It also carries the last content extent when the scale is
    /// unchanged, allowing a newly measured prediction to be verified before
    /// it affects the preserved offset.
    pub(crate) fn adopt_scroll_state(&self, prev: &ScrollState) {
        self.scroll_offset
            .set(Self::axis_only(self.axis, prev.scroll_offset.get()));
        self.pointer_velocity
            .set(Self::axis_only(self.axis, prev.pointer_velocity.get()));
        self.last_pointer_pos.set(prev.last_pointer_pos.get());
        self.drag_mode.set(prev.drag_mode.get());
        self.last_event_time.set(prev.last_event_time.get());
        self.last_frame_time.set(prev.last_frame_time.get());
        self.cursor_pos.set(prev.cursor_pos.get());
        self.thumb_hovered.set(prev.thumb_hovered.get());
        *self.velocity_history.borrow_mut() = prev
            .velocity_history
            .borrow()
            .as_ref()
            .map(|history| Box::new((**history).clone()));
        self.fling_start_time.set(prev.fling_start_time.get());
        self.fling_start_offset
            .set(Self::axis_only(self.axis, prev.fling_start_offset.get()));
        self.fling_target_offset
            .set(Self::axis_only(self.axis, prev.fling_target_offset.get()));
        self.fling_duration.set(prev.fling_duration.get());
        self.anim_curve.replace(prev.anim_curve.borrow().clone());
        self.active_touch_id.set(prev.active_touch_id.get());
        self.spring_velocity
            .set(Self::axis_only(self.axis, prev.spring_velocity.get()));
        self.momentum_start_time.set(prev.momentum_start_time.get());
        self.vel_accum
            .set(Self::axis_only(self.axis, prev.vel_accum.get()));
        self.vel_sample_time.set(prev.vel_sample_time.get());
        self.is_scrolling.set(prev.is_scrolling.get());
        #[cfg(debug_assertions)]
        {
            self.session_input_events
                .set(prev.session_input_events.get());
            self.session_draw_frames.set(prev.session_draw_frames.get());
            self.session_moved_frames
                .set(prev.session_moved_frames.get());
        }
        self.last_reported_offset.set(
            prev.last_reported_offset
                .get()
                .map(|offset| Self::axis_only(self.axis, offset)),
        );
        self.last_drawn_offset.set(
            prev.last_drawn_offset
                .get()
                .map(|offset| Self::axis_only(self.axis, offset)),
        );
        self.overscroll_hold.set(prev.overscroll_hold.get());
        self.overscroll_source.set(prev.overscroll_source.get());
        self.direct_overscroll_hold
            .set(prev.direct_overscroll_hold.get());
        self.highest_overscroll_offset.set(Self::axis_only(
            self.axis,
            prev.highest_overscroll_offset.get(),
        ));
        self.overscroll_peak_at.set(prev.overscroll_peak_at.get());
        self.device_contact.set(prev.device_contact.get());
        if prev.last_scale.get() == self.last_scale.get() {
            self.cached_content_size.set(prev.cached_content_size.get());
            self.cached_content_size_valid
                .set(prev.cached_content_size_valid.get());
        }
    }

    /// Whether the edges rubber-band for the device that is currently
    /// scrolling.
    ///
    /// [`ScrollBehavior::bouncy`] declares whether the viewport bounces at
    /// all; [`overscroll_sources`](Self::overscroll_sources) decides which
    /// devices are trusted with it on this target. Both must agree, so a
    /// browser can keep bouncy touch scrolling while its wheel stream — which
    /// never reports the end of a gesture — clamps at the edge.
    #[inline]
    pub(crate) fn bouncy(&self) -> bool {
        self.scroll_behavior.bouncy
            && self
                .overscroll_sources
                .contains(self.overscroll_source.get())
    }

    /// Records the device that produced the input now being applied.
    ///
    /// Called at the head of every input path — wheel frames, pointer drags
    /// and keyboard steps — so the frames that follow resolve
    /// [`bouncy`](Self::bouncy) against the device the user actually used.
    #[inline]
    pub(crate) fn set_overscroll_source(&self, source: OverscrollSource) {
        self.overscroll_source.set(source);
    }

    /// Briefly hold bouncy edge recovery when a wheel / trackpad gesture first
    /// crosses a boundary.
    #[inline]
    pub(crate) fn hold_overscroll_recovery(&self) {
        self.hold_overscroll_recovery_at(AnimInstant::now());
    }

    /// [`hold_overscroll_recovery`](Self::hold_overscroll_recovery) with an
    /// explicit instant, so grace-period expiry is testable without sleeping.
    #[inline]
    pub(crate) fn hold_overscroll_recovery_at(&self, at: AnimInstant) {
        self.overscroll_hold.set(Some(at));
    }

    /// Hold bouncy recovery while the user keeps physical contact with the
    /// trackpad after stretching past an edge.
    #[inline]
    pub(crate) fn hold_overscroll_for_direct_manipulation(&self) {
        self.direct_overscroll_hold.set(true);
    }

    /// Release the hold: the gesture ended, so the next frame springs the
    /// content back to the edge. A no-op when nothing is held.
    ///
    /// The overscroll peak deliberately survives this call because it guards
    /// the recovery that starts when the hold is released.
    #[inline]
    pub(crate) fn release_overscroll_recovery(&self) {
        self.overscroll_hold.set(None);
        self.direct_overscroll_hold.set(false);
    }

    /// Release only the contact hold, leaving a timed hold in place.
    ///
    /// Used when a frame proves the fingers left the device while the edge is
    /// still stretched: the non-expiring contact hold must go, but the short
    /// grace period that lets the crossing frame reach the screen may stay.
    #[inline]
    pub(crate) fn release_direct_overscroll_hold(&self) {
        self.direct_overscroll_hold.set(false);
    }

    /// Signed per-axis distance `offset` sits past the valid scroll range, or
    /// zero on an axis that is inside it.
    #[inline]
    pub(crate) fn overscroll_distance(&self, offset: Vec2d) -> Vec2d {
        let offset = Self::axis_only(self.axis, offset);
        let clamped = self.clamp_offset(offset);
        Vec2d {
            x: offset.x - clamped.x,
            y: offset.y - clamped.y,
        }
    }

    /// Raise the current device gesture's high-water mark for each stretched
    /// axis.
    ///
    /// Returning to the valid range deliberately does not clear the peak: the
    /// operating system may still deliver outward momentum-tail events after
    /// spring recovery finishes. Crossing to the opposite edge starts a new
    /// per-axis peak.
    #[inline]
    pub(crate) fn record_overscroll_peak(&self, offset: Vec2d) {
        self.record_overscroll_peak_at(offset, AnimInstant::now());
    }

    /// [`record_overscroll_peak`](Self::record_overscroll_peak) with an explicit
    /// instant, so idle expiry is testable without sleeping.
    ///
    /// `at` is stored only when the mark actually grows: a shrinking stretch
    /// (spring recovery) must not keep the episode alive, otherwise the mark
    /// could never go idle.
    pub(crate) fn record_overscroll_peak_at(&self, offset: Vec2d, at: AnimInstant) {
        let stretch = self.overscroll_distance(offset);
        let mut peak = self.highest_overscroll_offset.get();
        let mut raised = false;
        if stretch.x != 0.0
            && (stretch.x.signum() != peak.x.signum() || stretch.x.abs() > peak.x.abs())
        {
            peak.x = stretch.x;
            raised = true;
        }
        if stretch.y != 0.0
            && (stretch.y.signum() != peak.y.signum() || stretch.y.abs() > peak.y.abs())
        {
            peak.y = stretch.y;
            raised = true;
        }
        if raised {
            self.highest_overscroll_offset.set(peak);
            self.overscroll_peak_at.set(Some(at));
        }
    }

    /// Clear device-tail suppression at a gesture lifecycle boundary.
    #[inline]
    pub(crate) fn reset_overscroll_peak(&self) {
        self.highest_overscroll_offset.set(Vec2d::ZERO);
        self.overscroll_peak_at.set(None);
    }

    /// Fold one frame's contact signal in, reporting whether it opens a new
    /// physical gesture.
    ///
    /// A rising edge clears the previous episode's high-water mark: the fingers
    /// just landed, so nothing about the last gesture may gate this one. This
    /// is deliberately independent of [`TouchPhase`](aimer_events::element::TouchPhase),
    /// which the smoothing layer synthesizes per channel and therefore cannot
    /// be trusted to mark every gesture boundary.
    pub(crate) fn begin_device_contact(&self, contact: bool) -> bool {
        let started = !self.device_contact.replace(contact) && contact;
        if started {
            self.reset_overscroll_peak();
        }
        started
    }

    /// Drop an overscroll high-water mark whose episode went idle at `now`.
    ///
    /// Returns whether a mark is still live afterwards.
    fn overscroll_peak_is_live_at(&self, now: AnimInstant) -> bool {
        match self.overscroll_peak_at.get() {
            Some(at) if now.duration_since(at).as_millis() <= OVERSCROLL_PEAK_IDLE_MS => true,
            Some(_) => {
                self.reset_overscroll_peak();
                false
            }
            None => false,
        }
    }

    /// Whether an input step may update an overscrolled axis at `now`.
    ///
    /// Steps that shorten the stretch always apply. An outward step applies
    /// only when it exceeds the episode's previous peak, which distinguishes
    /// renewed user intent from a decaying device-scroll tail during recovery.
    /// A peak whose episode went idle no longer gates anything, so a gesture
    /// boundary the platform failed to report cannot wedge the edge.
    fn accepts_overscroll_step_at(&self, current: Vec2d, next: Vec2d, now: AnimInstant) -> bool {
        if !self.bouncy() {
            return true;
        }
        if !self.overscroll_peak_is_live_at(now) {
            return true;
        }
        let from = self.overscroll_distance(current);
        let to = self.overscroll_distance(next);
        let peak = self.highest_overscroll_offset.get();
        Self::axis_accepts_overscroll_step(from.x, to.x, peak.x)
            && Self::axis_accepts_overscroll_step(from.y, to.y, peak.y)
    }

    #[inline(always)]
    fn axis_accepts_overscroll_step(from: f32, to: f32, peak: f32) -> bool {
        to.abs() <= from.abs() || to.signum() != peak.signum() || to.abs() > peak.abs()
    }

    /// Whether edge recovery is still in its initial grace period at `now`.
    ///
    /// A hold older than [`OVERSCROLL_HOLD_IDLE_MS`] is expired and cleared on
    /// the spot so recovery starts independently of the device gesture's tail.
    pub(crate) fn overscroll_recovery_held_at(&self, now: AnimInstant) -> bool {
        if self.direct_overscroll_hold.get() {
            return true;
        }
        match self.overscroll_hold.get() {
            Some(last) => {
                let held = now.duration_since(last).as_millis() <= OVERSCROLL_HOLD_IDLE_MS;
                if !held {
                    self.overscroll_hold.set(None);
                }
                held
            }
            None => false,
        }
    }

    /// Current scroll position in logical (unscaled) pixels, measured from the
    /// content start (positive = scrolled toward the content end).
    ///
    /// Internally the offset is stored scaled and negated (content moved up),
    /// so this converts back to the user-facing convention.
    fn logical_offset(&self) -> Vec2d {
        let scale = self.last_scale.get().max(f32::EPSILON);
        let o = Self::axis_only(self.axis, self.scroll_offset.get());
        Vec2d {
            x: -o.x / scale,
            y: -o.y / scale,
        }
    }

    /// Enter the "scrolling" state, firing
    /// [`on_scroll_start`](Self::on_scroll_start) once on the idle →
    /// scrolling edge. A no-op if a session is already active,
    /// so it is safe to call on every drag move / wheel tick.
    pub(crate) fn begin_scroll(&self) {
        if !self.is_scrolling.replace(true) {
            #[cfg(debug_assertions)]
            {
                self.session_input_events.set(0);
                self.session_draw_frames.set(0);
                self.session_moved_frames.set(0);
            }
            // Clone the handle out (cheap `Rc` bump) before invoking so a callback
            // that touches the controller can't re-enter a live `RefCell` borrow.
            let cb = self.on_scroll_start.borrow().clone();
            cb.call(self.logical_offset());
        }
    }

    /// Leave the "scrolling" state, firing
    /// [`on_scroll_end`](Self::on_scroll_end) once on the scrolling → idle
    /// edge. A no-op when already idle, so the draw loop can call it every
    /// settled frame without emitting duplicates.
    pub(crate) fn end_scroll(&self) {
        if self.is_scrolling.replace(false) {
            let cb = self.on_scroll_end.borrow().clone();
            cb.call(self.logical_offset());
        }
    }

    /// Records one accepted input event in the active debug scroll session.
    #[inline]
    pub(crate) fn record_input_event(&self) {
        #[cfg(debug_assertions)]
        self.session_input_events
            .set(self.session_input_events.get().saturating_add(1));
    }

    /// Records one frame requested by the active debug scroll session.
    #[inline]
    pub(crate) fn record_draw_frame(&self, active: bool, moved: bool) {
        #[cfg(debug_assertions)]
        if active {
            self.session_draw_frames
                .set(self.session_draw_frames.get().saturating_add(1));
            if moved {
                self.session_moved_frames
                    .set(self.session_moved_frames.get().saturating_add(1));
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = (active, moved);
    }

    /// Fire [`on_scroll`](Self::on_scroll) with the current logical offset, but
    /// only when it has actually moved (beyond [`SCROLL_NOTIFY_EPSILON`]) since
    /// the last report. Level-triggered: call it once per drawn frame after the
    /// offset is finalized; the epsilon guard collapses idle frames and
    /// sub-pixel jitter to no-ops. The very first call only establishes the
    /// baseline, so the initial render never emits a spurious update.
    pub(crate) fn notify_scroll(&self) -> bool {
        let current = self.logical_offset();
        let moved = match self.last_reported_offset.get() {
            Some(prev) => {
                (current.x - prev.x).abs() > SCROLL_NOTIFY_EPSILON
                    || (current.y - prev.y).abs() > SCROLL_NOTIFY_EPSILON
            }
            None => false,
        };
        self.last_reported_offset.set(Some(current));
        if moved {
            // Clone out (cheap `Rc` bump) so a callback that touches the
            // controller can't re-enter a live `RefCell` borrow.
            let cb = self.on_scroll.borrow().clone();
            cb.call(current);
        }
        moved
    }
}

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
        *state.on_scroll_start.borrow_mut() = self.inner.on_scroll_start.borrow().clone();
        *state.on_scroll_end.borrow_mut() = self.inner.on_scroll_end.borrow().clone();
        *state.on_scroll.borrow_mut() = self.inner.on_scroll.borrow().clone();
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
        state.spring_velocity.set(Vec2d { x: 0.0, y: 0.0 });
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
            *s.on_scroll_start.borrow_mut() = cb.clone();
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
            *s.on_scroll_end.borrow_mut() = cb.clone();
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
            *s.on_scroll.borrow_mut() = cb.clone();
        });
    }
}

/// One axis of a cubic Bézier with implicit endpoints `P0 = 0`, `P3 = 1`.
#[inline]
fn bezier_axis(s: f32, p1: f32, p2: f32) -> f32 {
    let mt = 1.0 - s;
    3.0 * mt * mt * s * p1 + 3.0 * mt * s * s * p2 + s * s * s
}

/// Derivative (w.r.t. the parameter `s`) of [`bezier_axis`].
#[inline]
fn bezier_axis_deriv(s: f32, p1: f32, p2: f32) -> f32 {
    let mt = 1.0 - s;
    3.0 * mt * mt * p1 + 6.0 * mt * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
}

/// Evaluate a CSS-style `cubic-bezier(x1, y1, x2, y2)` easing at linear time
/// fraction `t ∈ [0, 1]`, returning the eased progress `∈ [0, 1]`.
///
/// The control points encode time on the x-axis and progress on the y-axis, so
/// we first solve `bezier_x(s) = t` for the curve parameter `s` (Newton-Raphson
/// with a bisection fallback), then read `bezier_y(s)`.
pub(crate) fn cubic_bezier_ease(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }

    // Newton-Raphson from a sensible initial guess.
    let mut s = t;
    for _ in 0..8 {
        let x = bezier_axis(s, x1, x2) - t;
        if x.abs() < 1e-5 {
            return bezier_axis(s, y1, y2);
        }
        let dx = bezier_axis_deriv(s, x1, x2);
        if dx.abs() < 1e-6 {
            break;
        }
        s = (s - x / dx).clamp(0.0, 1.0);
    }

    // Bisection fallback if Newton stalled.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    s = t;
    for _ in 0..16 {
        let x = bezier_axis(s, x1, x2);
        if (x - t).abs() < 1e-5 {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
        s = 0.5 * (lo + hi);
    }
    bezier_axis(s, y1, y2)
}

impl ScrollState {
    /// Clamp the scroll offset within the allowed range.
    /// scroll_offset is negative (content moves up), so min_scroll <= offset <=
    /// 0 typically.
    pub(crate) fn clamp_offset(&self, mut offset: Vec2d) -> Vec2d {
        let min = self.cached_min_scroll.get();
        let max = self.cached_max_scroll.get();
        offset.x = offset.x.max(-max.x).min(-min.x);
        offset.y = offset.y.max(-max.y).min(-min.y);
        offset
    }

    /// Apply a wheel/trackpad scroll delta to `offset` and return the new
    /// offset.
    ///
    /// Non-bouncy scrollables clamp to the valid range immediately (matching
    /// the pointer-drag and keyboard paths). Bouncy ones keep the raw
    /// (possibly out-of-range) offset and let [`visual_offset`] +
    /// [`update_momentum`] apply the rubber-band and spring back.
    ///
    /// This replaces an earlier attempt that zeroed the delta up-front when
    /// `offset` was already at `clamp_offset(offset)`. Because an in-range
    /// offset always equals its own clamp, that predicate matched on *every*
    /// frame and silently discarded all wheel/trackpad deltas — the scrollable
    /// could not be scrolled by mouse wheel or trackpad at all (drag/keyboard
    /// still worked because they clamp *after* applying the delta).
    pub(crate) fn apply_wheel_delta(&self, offset: Vec2d, scroll_delta: Vec2d) -> Vec2d {
        let mut next = Vec2d {
            x: offset.x + scroll_delta.x,
            y: offset.y + scroll_delta.y,
        };
        if !self.bouncy() {
            next = self.clamp_offset(next);
        }
        next
    }

    /// Apply one discrete mouse-wheel detent without starting inertial motion.
    ///
    /// A line wheel has no native gesture velocity to preserve. Each frame step
    /// therefore updates the offset once and clears any previous glide; bouncy
    /// edge recovery remains draw-driven.
    pub(crate) fn apply_line_wheel_delta(&self, scroll_delta: Vec2d) -> bool {
        self.commit_device_scroll_delta(scroll_delta, AnimInstant::now())
    }

    /// Apply one pixel-precise device delta without deriving a second glide.
    ///
    /// Quiver already frame-synchronizes trackpad input, while operating
    /// systems and browsers provide the gesture's own deceleration samples.
    /// Keeping this layer distance-only prevents double integration and leaves
    /// touch/mouse drag release flings unchanged.
    pub(crate) fn apply_precise_scroll_delta(&self, scroll_delta: Vec2d) -> bool {
        self.apply_precise_scroll_delta_at(scroll_delta, AnimInstant::now())
    }

    /// [`apply_precise_scroll_delta`](Self::apply_precise_scroll_delta) at an
    /// explicit instant, so episode expiry is testable without sleeping.
    pub(crate) fn apply_precise_scroll_delta_at(
        &self,
        scroll_delta: Vec2d,
        now: AnimInstant,
    ) -> bool {
        self.commit_device_scroll_delta(scroll_delta, now)
    }

    /// Store one device delta unless it is a weaker outward re-stretch of an
    /// overscroll episode that is already recovering.
    ///
    /// A rejected delta still proves the gesture's tail is running, so it keeps
    /// the high-water mark alive; only a genuine silence lets the mark expire.
    fn commit_device_scroll_delta(&self, scroll_delta: Vec2d, now: AnimInstant) -> bool {
        let current = self.scroll_offset.get();
        let next = self.apply_wheel_delta(current, scroll_delta);
        if !self.accepts_overscroll_step_at(current, next, now) {
            self.overscroll_peak_at.set(Some(now));
            return false;
        }
        self.set_scroll_offset(next);
        self.record_overscroll_peak_at(next, now);
        self.stop_device_scroll_momentum();
        true
    }

    fn stop_device_scroll_momentum(&self) {
        self.pointer_velocity.set(Vec2d::ZERO);
        self.spring_velocity.set(Vec2d::ZERO);
        self.momentum_start_time.set(None);
        self.clear_velocity_history();
        self.cancel_fling();
    }

    /// Per-axis distance (px) the content may be pulled past an edge.
    ///
    /// Derived from the viewport rather than the content size: the stretch is
    /// judged against what the user can see, so the same gesture must feel the
    /// same on a short and on a very long page. [`MIN_VIEWPORT`] keeps the
    /// limit sane before the first layout has measured a viewport.
    #[inline]
    pub(crate) fn overscroll_limit(&self) -> Vec2d {
        let (vp_w, vp_h) = self.cached_viewport.get();
        Vec2d {
            x: vp_w.max(MIN_VIEWPORT) * MAX_OVERSCROLL_VIEWPORT_FRACTION,
            y: vp_h.max(MIN_VIEWPORT) * MAX_OVERSCROLL_VIEWPORT_FRACTION,
        }
    }

    /// Scale one input step by the resistance the current stretch applies to it.
    ///
    /// Only the axis that is *already* past an edge is resisted, and only for
    /// the direction that pulls further out: a step that shortens the stretch
    /// answers the finger one to one, so the content can always be pulled back.
    /// The outward factor starts at [`OOB_DRAG_RESISTANCE`] and fades linearly
    /// to zero at [`overscroll_limit`](Self::overscroll_limit), which turns a
    /// stream of equal deltas into an exponential approach to that limit —
    /// the rubber band a held gesture is supposed to feel.
    ///
    /// The step that first *crosses* an edge is deliberately left untouched, so
    /// a fling still carries its own momentum into the overscroll zone.
    #[inline]
    pub(crate) fn resisted_overscroll_delta(&self, delta: Vec2d) -> Vec2d {
        if !self.bouncy() {
            return delta;
        }
        let stretch = self.overscroll_distance(self.scroll_offset.get());
        let limit = self.overscroll_limit();
        Vec2d {
            x: delta.x * Self::outward_resistance(stretch.x, delta.x, limit.x),
            y: delta.y * Self::outward_resistance(stretch.y, delta.y, limit.y),
        }
    }

    /// Resistance factor for one axis, given its signed `stretch` past the
    /// edge, the signed input `delta`, and the axis' overscroll `limit`.
    #[inline(always)]
    fn outward_resistance(stretch: f32, delta: f32, limit: f32) -> f32 {
        // In range, standing still, or pulling back toward the edge: no
        // resistance at all.
        if stretch == 0.0 || delta == 0.0 || stretch.signum() != delta.signum() {
            return 1.0;
        }
        if limit <= 0.0 {
            return 0.0;
        }
        let used = (stretch.abs() / limit).clamp(0.0, 1.0);
        OOB_DRAG_RESISTANCE * (1.0 - used)
    }

    /// Hard-cap overscroll to [`MAX_OVERSCROLL_VIEWPORT_FRACTION`] of the
    /// viewport.
    ///
    /// Unlike [`visual_offset`] (which applies a rubber-band *display*
    /// transform), this limits the **actual** stored offset so that momentum
    /// and trackpad events can't carry content hundreds of pixels past the
    /// edge — matching Chrome and iOS behaviour.
    fn clamp_overscroll(&self, offset: Vec2d) -> Vec2d {
        let limit = self.overscroll_limit();
        let clamped = self.clamp_offset(offset);
        Vec2d {
            x: offset.x.clamp(clamped.x - limit.x, clamped.x + limit.x),
            y: offset.y.clamp(clamped.y - limit.y, clamped.y + limit.y),
        }
    }

    /// Rational rubber-band visual transform (native macOS/iOS shape).
    ///
    /// Uses `f(x) = (1 - 1/(c·x/d + 1))·d` where `c` is the visual
    /// coefficient scaled by the user's `bouncy_resistance`, `d` is the
    /// viewport dimension, and `x` is the overscroll distance. The curve is
    /// asymptotic — visual stretch grows quickly at first then flattens,
    /// matching the native rubber-band feel.
    #[inline(always)]
    fn apply_bouncy(value: f32, min: f32, max: f32, dimension: f32, resistance: f32) -> f32 {
        let c = RUBBER_BAND_VISUAL_COEFFICIENT * resistance;
        if value < min {
            let diff = min - value;
            let visual = (1.0 - 1.0 / (c * diff / dimension + 1.0)) * dimension;
            min - visual
        } else if value > max {
            let diff = value - max;
            let visual = (1.0 - 1.0 / (c * diff / dimension + 1.0)) * dimension;
            max + visual
        } else {
            value
        }
    }

    pub(crate) fn visual_offset(&self, offset: Vec2d) -> Vec2d {
        let offset = Self::axis_only(self.axis, offset);
        let min = self.cached_min_scroll.get();
        let max = self.cached_max_scroll.get();

        let min_x = -min.x;
        let max_x = -max.x;
        let min_y = -min.y;
        let max_y = -max.y;

        if self.bouncy() {
            let (vp_w, vp_h) = self.cached_viewport.get();
            let resistance = self.scroll_behavior.bouncy_resistance;
            let vx = Self::apply_bouncy(offset.x, max_x, min_x, vp_w.max(MIN_VIEWPORT), resistance);
            let vy = Self::apply_bouncy(offset.y, max_y, min_y, vp_h.max(MIN_VIEWPORT), resistance);

            (vx, vy).into()
        } else {
            (offset.x.clamp(max_x, min_x), offset.y.clamp(max_y, min_y)).into()
        }
    }

    /// Check if a point is inside the vertical thumb rect.
    pub(crate) fn hit_test_v_thumb(&self, p: Vec2d) -> bool {
        if let Some((x, y, w, h)) = self.v_thumb_rect.get() {
            p.x >= x && p.x <= x + w && p.y >= y && p.y <= y + h
        } else {
            false
        }
    }

    /// Check if a point is inside the horizontal thumb rect.
    pub(crate) fn hit_test_h_thumb(&self, p: Vec2d) -> bool {
        if let Some((x, y, w, h)) = self.h_thumb_rect.get() {
            p.x >= x && p.x <= x + w && p.y >= y && p.y <= y + h
        } else {
            false
        }
    }

    /// Reconciles the remembered thumb-hover state with the cursor at `p`
    /// and reports whether it changed.
    ///
    /// The thumb's highlight is resolved at draw time from
    /// [`cursor_pos`](Self::cursor_pos), so it only updates on frames that
    /// actually render. Pointer moves no longer buy a frame each — that is
    /// what used to repaint the window at input rate — so the move that
    /// carries the cursor across a thumb edge is detected here, and the
    /// caller schedules the single frame that repaints the highlight.
    #[inline]
    pub(crate) fn sync_thumb_hover(&self, p: Vec2d) -> bool {
        let hovered = self.hit_test_v_thumb(p) || self.hit_test_h_thumb(p);
        self.thumb_hovered.replace(hovered) != hovered
    }

    /// Record `velocity` as the speed the finger held for the `dt` seconds
    /// that ended at `at`.
    pub(crate) fn push_velocity(&self, velocity: Vec2d, dt: f32, at: AnimInstant) {
        self.velocity_history
            .borrow_mut()
            .get_or_insert_with(|| Box::new(VelocityHistory::new()))
            .push(velocity, dt, at);
    }

    /// Return the speed the finger is moving at as of `now`, averaged over the
    /// samples that still lie inside [`VELOCITY_HORIZON_S`].
    ///
    /// This is the speed a release fling is launched with, so it deliberately
    /// describes the *end* of the gesture: a drag whose last recorded motion
    /// has fallen out of the horizon reports zero and flings nothing.
    pub(crate) fn smoothed_velocity(&self, now: AnimInstant) -> Vec2d {
        self.velocity_history
            .borrow()
            .as_ref()
            .map_or(Vec2d::ZERO, |history| history.velocity_at(now))
    }

    /// Clear the velocity history (e.g. on pointer-down).
    pub(crate) fn clear_velocity_history(&self) {
        if let Some(history) = self.velocity_history.borrow_mut().as_mut() {
            history.clear();
        }
    }

    /// Fold a raw drag delta (already scaled by `speed_multiplier`) into the
    /// velocity accumulator and, once a real time slice
    /// ([`VELOCITY_SAMPLE_MIN_DT`]) has elapsed since the last sample, emit
    /// the averaged drag velocity (px per 120 Hz-frame, both axes) together
    /// with that slice's `dt`. Returns `None` while the delta is still
    /// being accumulated within the current slice.
    ///
    /// This merges the burst of *coalesced* same-`Instant` pointer moves that
    /// web delivers per native `pointermove` into one realistic velocity
    /// sample, instead of letting each tiny sub-delta / ~0 dt inflate the
    /// release fling (~3x too fast on touch). Native, delivering one move
    /// per frame, emits on every call. The offset is still updated
    /// per-event by the caller, so dragging stays 1:1.
    pub(crate) fn accumulate_drag_velocity(
        &self,
        dx: f32,
        dy: f32,
        now: AnimInstant,
    ) -> Option<(Vec2d, f32)> {
        let mut accum = self.vel_accum.get();
        accum.x += dx;
        accum.y += dy;

        let sample_dt = self
            .vel_sample_time
            .get()
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(FRAME_REF_120);

        if sample_dt >= VELOCITY_SAMPLE_MIN_DT {
            self.vel_accum.set(Vec2d { x: 0.0, y: 0.0 });
            self.vel_sample_time.set(Some(now));
            let velocity = Vec2d {
                x: (accum.x / sample_dt) * FRAME_REF_120,
                y: (accum.y / sample_dt) * FRAME_REF_120,
            };
            Some((velocity, sample_dt))
        } else {
            self.vel_accum.set(accum);
            None
        }
    }

    /// Close the open sampling slice at `now` and return the velocity it
    /// measured, or `None` while the slice is too short
    /// ([`VELOCITY_SAMPLE_MIN_DT`]) to mean anything.
    ///
    /// A gesture ends between samples, and everything that happened since the
    /// last one is still sitting in the accumulator: the tail of a coalesced
    /// burst, or — the case that matters most — no motion at all, because the
    /// finger came to rest on the glass and the platform stopped reporting
    /// moves. Both describe the release far better than the last emitted
    /// sample does, and a resting finger yields exactly the zero velocity that
    /// keeps a deliberate stop from turning into a fling.
    ///
    /// Unlike [`Self::accumulate_drag_velocity`] this always closes the slice,
    /// since no further move will arrive to close it.
    pub(crate) fn flush_drag_velocity(&self, now: AnimInstant) -> Option<(Vec2d, f32)> {
        let sample_time = self.vel_sample_time.get()?;
        let sample_dt = now.duration_since(sample_time).as_secs_f32();
        if sample_dt < VELOCITY_SAMPLE_MIN_DT {
            return None;
        }

        let accum = self.vel_accum.get();
        self.vel_accum.set(Vec2d { x: 0.0, y: 0.0 });
        self.vel_sample_time.set(Some(now));
        let velocity = Vec2d {
            x: (accum.x / sample_dt) * FRAME_REF_120,
            y: (accum.y / sample_dt) * FRAME_REF_120,
        };
        Some((velocity, sample_dt))
    }

    /// Cancel any active cubic-bézier release fling.
    ///
    /// Called whenever a new input (touch-down, wheel, keyboard, scrollbar
    /// paging) should take over momentum, so the curve-driven glide does not
    /// keep fighting the fresh interaction.
    pub(crate) fn cancel_fling(&self) {
        self.fling_start_time.set(None);
        self.anim_curve.replace(None);
    }

    /// Arm a programmatic animation that eases the scroll offset from its
    /// current value to `target` over `duration_s` seconds following `curve`.
    ///
    /// This reuses the existing curve-driven fling machinery in
    /// [`Self::update_momentum`]: it seeds the fling start/target/duration and
    /// records `curve` in [`Self::anim_curve`] so the draw loop interpolates
    /// `start + (target - start) · curve(t / duration)` each frame. A
    /// non-positive duration jumps straight to the (clamped) target.
    ///
    /// `target` is an internal (scaled) scroll offset in the same convention as
    /// [`Self::scroll_offset`] (negative = content moved up).
    pub(crate) fn start_animation(&self, target: Vec2d, duration_s: f32, curve: impl Curve) {
        // Clear any live drag/momentum so the animation fully owns the motion.
        self.pointer_velocity.set(Vec2d { x: 0.0, y: 0.0 });
        self.spring_velocity.set(Vec2d { x: 0.0, y: 0.0 });
        self.momentum_start_time.set(None);
        // A programmatic scroll supersedes any device gesture, so it must not be
        // stalled by an edge-recovery hold left behind by that gesture.
        self.release_overscroll_recovery();
        self.reset_overscroll_peak();

        let start = Self::axis_only(self.axis, self.scroll_offset.get());
        self.scroll_offset.set(start);
        // Non-bouncy scrollables never overshoot; pin the target to the edge.
        let target = if self.bouncy() {
            Self::axis_only(self.axis, target)
        } else {
            self.clamp_offset(Self::axis_only(self.axis, target))
        };

        if duration_s <= 0.0 || (start.x == target.x && start.y == target.y) {
            self.set_scroll_offset(target);
            self.cancel_fling();
            return;
        }

        self.fling_start_offset.set(start);
        self.fling_target_offset.set(target);
        self.fling_duration.set(duration_s);
        self.anim_curve.replace(Some(curve.into_any()));
        self.fling_start_time.set(Some(AnimInstant::now()));
    }

    /// Arm a cubic-bézier release fling.
    ///
    /// `release_velocity` is the projected launch velocity (px per 120 Hz
    /// frame). The fling runs for a fixed [`FLING_DURATION_S`] window and its
    /// position follows `start + distance · cubic-bezier(t / duration)`.
    ///
    /// The coast distance is derived per axis so the curve's initial slope
    /// (`slope0 = y1 / x1`, the curve's `dy/dx` at `t = 0`) matches the release
    /// speed: the animation leaves the finger at exactly the velocity it was
    /// moving (no visible jump), then decelerates along the curve to a gentle
    /// stop. Because the curve is the only deceleration model here, the
    /// friction field no longer participates in the fling —
    /// `FLING_DURATION_S` is the single knob (longer = farther + slower
    /// settle).
    ///
    ///   v0_px_s = release_velocity / FRAME_REF_120         (px per second)
    ///   v(0)    = distance · slope0 / duration  =!  v0_px_s
    ///   ⇒ distance = v0_px_s · duration / slope0
    ///
    /// Currently unused: touch/mouse release now carries momentum through the
    /// shared velocity + friction model (so it matches trackpad feel) rather
    /// than this bézier fling. Kept available as an alternative fling model.
    #[allow(dead_code)]
    pub(crate) fn start_fling(&self, release_velocity: Vec2d, now: AnimInstant) {
        let release_velocity = Self::axis_only(self.axis, release_velocity);
        if release_velocity.x == 0.0 && release_velocity.y == 0.0 {
            self.cancel_fling();
            return;
        }

        let duration = FLING_DURATION_S;
        let slope0 = FLING_BEZIER_Y1 / FLING_BEZIER_X1;
        // distance = (v_px_frame / FRAME_REF_120) · duration / slope0.
        let k = duration / (FRAME_REF_120 * slope0);
        let dist = Vec2d {
            x: release_velocity.x * k,
            y: release_velocity.y * k,
        };

        let start = Self::axis_only(self.axis, self.scroll_offset.get());
        // debug!("Start: {:?}", start);
        let mut target = Vec2d {
            x: start.x + dist.x,
            y: start.y + dist.y,
        };
        // Non-bouncy scrolling never overshoots, so pin the target to the edge
        // and let the curve ease straight into it.
        if !self.bouncy() {
            target = self.clamp_offset(target);
        }

        if duration <= 0.0 || (dist.x == 0.0 && dist.y == 0.0) {
            self.cancel_fling();
            return;
        }

        self.fling_start_offset.set(start);
        self.fling_target_offset.set(target);
        self.fling_duration.set(duration);
        self.fling_start_time.set(Some(now));
    }

    /// Check if a point is inside the vertical scrollbar *track* but outside
    /// the thumb.
    pub(crate) fn hit_test_v_track(
        &self,
        p: Vec2d,
        viewport_w: f32,
        viewport_h: f32,
        track_width: f32,
    ) -> bool {
        if let Some((_tx, y, _tw, h)) = self.v_thumb_rect.get() {
            let track_left = self
                .scroll_bar_placement
                .cross_offset(viewport_w, track_width);
            let in_track_x = p.x >= track_left && p.x <= track_left + track_width;
            let in_track_y = p.y >= 0.0 && p.y <= viewport_h;
            let on_thumb = p.y >= y && p.y <= y + h;
            in_track_x && in_track_y && !on_thumb
        } else {
            false
        }
    }

    /// Check if a point is inside the horizontal scrollbar *track* but outside
    /// the thumb.
    pub(crate) fn hit_test_h_track(
        &self,
        p: Vec2d,
        viewport_w: f32,
        viewport_h: f32,
        track_width: f32,
    ) -> bool {
        if let Some((x, _ty, w, _th)) = self.h_thumb_rect.get() {
            let track_top = self
                .scroll_bar_placement
                .cross_offset(viewport_h, track_width);
            let in_track_y = p.y >= track_top && p.y <= track_top + track_width;
            let in_track_x = p.x >= 0.0 && p.x <= viewport_w;
            let on_thumb = p.x >= x && p.x <= x + w;
            in_track_y && in_track_x && !on_thumb
        } else {
            false
        }
    }

    /// Update momentum, spring-back, and friction during the draw phase (when
    /// not dragging). Returns the updated offset and whether a redraw is
    /// needed.
    pub(crate) fn update_momentum(&self, mut offset: Vec2d) -> (Vec2d, bool) {
        offset = Self::axis_only(self.axis, offset);
        let clamped = self.clamp_offset(offset);
        let mut velocity = Self::axis_only(self.axis, self.pointer_velocity.get());
        self.pointer_velocity.set(velocity);
        self.spring_velocity
            .set(Self::axis_only(self.axis, self.spring_velocity.get()));
        let mut needs_redraw = false;

        let now = AnimInstant::now();
        let dt = self
            .last_frame_time
            .get()
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(FRAME_REF_120)
            .min(MAX_FRAME_DT);
        self.last_frame_time.set(Some(now));

        let frame_ratio = dt / FRAME_REF_120;

        let past_edge = offset.x != clamped.x || offset.y != clamped.y;
        if self.bouncy() && past_edge && self.overscroll_recovery_held_at(now) {
            let held = self.clamp_overscroll(offset);
            let out_of_bounds = held.x != clamped.x || held.y != clamped.y;
            return (held, out_of_bounds);
        }

        if let Some(fling_start) = self.fling_start_time.get() {
            let duration = self.fling_duration.get();
            let elapsed = now.duration_since(fling_start).as_secs_f32();
            let u = if duration > 0.0 {
                (elapsed / duration).clamp(0.0, 1.0)
            } else {
                1.0
            };

            let eased = match self.anim_curve.borrow().as_ref() {
                Some(curve) => curve.transform(u),
                None => cubic_bezier_ease(
                    u,
                    FLING_BEZIER_X1,
                    FLING_BEZIER_Y1,
                    FLING_BEZIER_X2,
                    FLING_BEZIER_Y2,
                ),
            };
            let start = self.fling_start_offset.get();
            let target = self.fling_target_offset.get();
            let new = Vec2d {
                x: start.x + (target.x - start.x) * eased,
                y: start.y + (target.y - start.y) * eased,
            };

            let step = Vec2d {
                x: new.x - offset.x,
                y: new.y - offset.y,
            };
            let vel = if dt > 0.0 {
                Vec2d {
                    x: step.x / frame_ratio,
                    y: step.y / frame_ratio,
                }
            } else {
                Vec2d::default()
            };

            offset = new;
            // debug!("New Offset : {:?}", new);
            self.pointer_velocity.set(vel);
            needs_redraw = true;

            let oob = offset.x != clamped.x || offset.y != clamped.y;
            if oob && self.bouncy() {
                // Hand the remaining momentum to the velocity-based spring so the
                // content bounces and recovers from the edge like native iOS.
                self.cancel_fling();
            } else {
                // Snap to rest once finished, or once the tail step becomes
                // sub-pixel (the curve crawls toward the target for a long time
                // after covering ~95% of the distance early).
                let tail_done = u >= FLING_TAIL_START
                    && step.x.abs() < FLING_END_STEP_PX
                    && step.y.abs() < FLING_END_STEP_PX;
                if u >= 1.0 || tail_done {
                    offset = target;
                    self.pointer_velocity.set(Vec2d { x: 0.0, y: 0.0 });
                    self.cancel_fling();
                }
            }
        } else if velocity.x.abs() > VELOCITY_EPSILON || velocity.y.abs() > VELOCITY_EPSILON {
            // Clear any in-flight spring oscillation when fresh momentum begins.
            self.spring_velocity.set(Vec2d { x: 0.0, y: 0.0 });
            let now_instant = AnimInstant::now();
            let momentum_elapsed = match self.momentum_start_time.get() {
                Some(t) => now_instant.duration_since(t).as_secs_f32(),
                None => {
                    self.momentum_start_time.set(Some(now_instant));
                    0.0
                }
            };
            if momentum_elapsed >= MAX_MOMENTUM_DURATION_S {
                self.pointer_velocity.set(Vec2d { x: 0.0, y: 0.0 });
                self.momentum_start_time.set(None);
                return (offset, false);
            } else {
                // Fade-out zone: in the last MOMENTUM_FADEOUT_S seconds before
                // the cap, apply progressively increasing friction so the
                // velocity bleeds to near-zero instead of hitting a wall.
                let remaining = MAX_MOMENTUM_DURATION_S - momentum_elapsed;
                if remaining < MOMENTUM_FADEOUT_S {
                    // progress ∈ [0, 1]: 0 = fade starts, 1 = at the cap.
                    let progress = 1.0 - (remaining / MOMENTUM_FADEOUT_S);
                    // Ramp friction from normal (0.999) down to aggressive (0.90)
                    // as we approach the cap.
                    let fade_friction = 0.999 - 0.099 * progress;
                    velocity.x *= fade_friction.powf(frame_ratio);
                    velocity.y *= fade_friction.powf(frame_ratio);
                }
            }

            let decay = self.scroll_behavior.friction.powf(frame_ratio);
            offset.x += velocity.x * frame_ratio;
            offset.y += velocity.y * frame_ratio;
            if self.bouncy() {
                let capped = self.clamp_overscroll(offset);
                if capped.x != offset.x {
                    offset.x = capped.x;
                    velocity.x = 0.0;
                }
                if capped.y != offset.y {
                    offset.y = capped.y;
                    velocity.y = 0.0;
                }
            }
            velocity.x *= decay;
            velocity.y *= decay;
            let clamped = self.clamp_offset(offset);
            if self.bouncy() {
                let oob_decay = OVERSCROLL_FRICTION.powf(frame_ratio);
                if offset.x != clamped.x {
                    velocity.x *= oob_decay;
                }
                if offset.y != clamped.y {
                    velocity.y *= oob_decay;
                }
            }
            if !self.bouncy() {
                if offset.x != clamped.x {
                    offset.x = clamped.x;
                    velocity.x = 0.0;
                }
                if offset.y != clamped.y {
                    offset.y = clamped.y;
                    velocity.y = 0.0;
                }
            } else {
                let mut sv = self.spring_velocity.get();

                if offset.x != clamped.x {
                    velocity.x = 0.0;
                    let (err_x, vx) = advance_overscroll_spring(offset.x - clamped.x, sv.x, dt);
                    offset.x = clamped.x + err_x;
                    sv.x = vx;
                }
                if offset.y != clamped.y {
                    velocity.y = 0.0;
                    let (err_y, vy) = advance_overscroll_spring(offset.y - clamped.y, sv.y, dt);
                    offset.y = clamped.y + err_y;
                    sv.y = vy;
                }
                self.spring_velocity.set(sv);

                // Snap to boundary if the spring crossed through it.
                let new_clamped = self.clamp_offset(offset);
                if offset.x != clamped.x
                    && ((clamped.x >= offset.x && offset.x >= new_clamped.x)
                        || (clamped.x <= offset.x && offset.x <= new_clamped.x))
                {
                    offset.x = new_clamped.x;
                    sv.x = 0.0;
                }
                if offset.y != clamped.y
                    && ((clamped.y >= offset.y && offset.y >= new_clamped.y)
                        || (clamped.y <= offset.y && offset.y <= new_clamped.y))
                {
                    offset.y = new_clamped.y;
                    sv.y = 0.0;
                }
                self.spring_velocity.set(sv);
            }

            self.pointer_velocity.set(velocity);
            needs_redraw = true;
        } else if velocity.x != 0.0 || velocity.y != 0.0 {
            self.pointer_velocity.set(Vec2d { x: 0.0, y: 0.0 });
            self.momentum_start_time.set(None);
        }
        let v_check = self.pointer_velocity.get();
        let momentum_active =
            v_check.x.abs() > VELOCITY_EPSILON || v_check.y.abs() > VELOCITY_EPSILON;
        if self.bouncy() && !momentum_active && (offset.x != clamped.x || offset.y != clamped.y) {
            let mut sv = self.spring_velocity.get();
            let err_x = offset.x - clamped.x;
            let err_y = offset.y - clamped.y;
            let (new_err_x, vx) = advance_overscroll_spring(err_x, sv.x, dt);
            let (new_err_y, vy) = advance_overscroll_spring(err_y, sv.y, dt);
            sv.x = vx;
            sv.y = vy;
            offset.x = clamped.x + new_err_x;
            offset.y = clamped.y + new_err_y;
            if err_x != 0.0 && (err_x > 0.0) != (new_err_x > 0.0) {
                offset.x = clamped.x;
                sv.x = 0.0;
            }
            if err_y != 0.0 && (err_y > 0.0) != (new_err_y > 0.0) {
                offset.y = clamped.y;
                sv.y = 0.0;
            }

            self.spring_velocity.set(sv);
            needs_redraw = true;

            // Snap to rest when distance from edge and velocity are both negligible.
            if (offset.x - clamped.x).abs() < SNAP_EPSILON && sv.x.abs() < VELOCITY_EPSILON {
                offset.x = clamped.x;
                sv.x = 0.0;
            }
            if (offset.y - clamped.y).abs() < SNAP_EPSILON && sv.y.abs() < VELOCITY_EPSILON {
                offset.y = clamped.y;
                sv.y = 0.0;
            }
            self.spring_velocity.set(sv);
        } else if !self.bouncy() {
            offset = clamped;
        }

        // Hard-cap overscroll to a fraction of viewport (prevents unlimited
        // rubber-band from trackpad or high-velocity flings).
        offset = self.clamp_overscroll(offset);

        // Keep the peak after reaching the boundary: the current device
        // gesture may still have native momentum-tail events in flight.
        self.record_overscroll_peak(offset);

        (Self::axis_only(self.axis, offset), needs_redraw)
    }
}

#[cfg(test)]
impl ScrollState {
    /// A default vertical engine parked at `offset`, shared by the tests of
    /// every module in this crate.
    pub(crate) fn for_test_at(offset: Vec2d) -> Self {
        use aimer_macro::key;

        Self {
            key_scroll_strength: 50f32,
            scroll_behavior: ScrollBehavior::default(),
            overscroll_sources: OverscrollSources::ALL,
            overscroll_source: Cell::new(OverscrollSource::Wheel),
            axis: ScrollAxis::Vertical,
            scroll_bar_placement: ScrollBarPlacement::Floating,
            scroll_offset: Cell::new(offset),
            storage_key: key!(),
            remember_scroll_offset: false,
            last_pointer_pos: Cell::new(None),
            drag_mode: Cell::new(DragMode::None),
            cached_max_scroll: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            cached_min_scroll: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            pointer_velocity: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            last_event_time: Cell::new(None),
            last_frame_time: Cell::new(None),
            v_thumb_rect: Cell::new(None),
            h_thumb_rect: Cell::new(None),
            thumb_hovered: Cell::new(false),
            v_scroll_multiplier: Cell::new(0.0),
            h_scroll_multiplier: Cell::new(0.0),
            last_scale: Cell::new(1.0),
            speed_multiplier: 1.0,
            cursor_pos: Cell::new(None),
            velocity_history: RefCell::new(None),
            cached_viewport: Cell::new((0.0, 0.0)),
            cached_v_track_width: Cell::new(0.0),
            cached_h_track_width: Cell::new(0.0),
            cached_content_size: Cell::new(Default::default()),
            cached_content_size_valid: Cell::new(false),
            fling_start_time: Cell::new(None),
            fling_start_offset: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            fling_target_offset: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            fling_duration: Cell::new(0.0),
            anim_curve: RefCell::new(None),
            active_touch_id: Cell::new(None),
            spring_velocity: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            momentum_start_time: Cell::new(None),
            vel_accum: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            vel_sample_time: Cell::new(None),
            is_scrolling: Cell::new(false),
            #[cfg(debug_assertions)]
            session_input_events: Cell::new(0),
            #[cfg(debug_assertions)]
            session_draw_frames: Cell::new(0),
            #[cfg(debug_assertions)]
            session_moved_frames: Cell::new(0),
            on_scroll_start: RefCell::new(Callback::default()),
            on_scroll_end: RefCell::new(Callback::default()),
            on_scroll: RefCell::new(Callback::default()),
            last_reported_offset: Cell::new(None),
            last_drawn_offset: Cell::new(None),
            overscroll_hold: Cell::new(None),
            direct_overscroll_hold: Cell::new(false),
            highest_overscroll_offset: Cell::new(Vec2d::ZERO),
            overscroll_peak_at: Cell::new(None),
            device_contact: Cell::new(false),
            #[cfg(target_arch = "wasm32")]
            web_overscroll_decay: crate::scrollable::web_overscroll::WebOverscrollDecay::new(),
            #[cfg(target_arch = "wasm32")]
            web_recovery_end: crate::scrollable::web_recovery_end::WebRecoveryEnd::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use aimer_animation::curves;

    fn ctrl_with_offset(offset: Vec2d) -> ScrollState {
        ScrollState::for_test_at(offset)
    }

    #[test]
    fn scroll_state_uses_the_frame_requester() {
        let requests = Rc::new(Cell::new(0));
        let counted = requests.clone();
        let previous = aimer_events::window::set_thread_redraw_requester(move || {
            counted.set(counted.get() + 1);
        });

        ctrl_with_offset(Vec2d::ZERO).request_animation_frame();

        aimer_events::window::restore_thread_redraw_requester(previous);

        assert_eq!(requests.get(), 1);
    }

    #[test]
    fn scrollbar_track_hit_testing_follows_its_placement() {
        let mut ctrl = ctrl_with_offset(Vec2d::ZERO);
        ctrl.cached_viewport.set((320.0, 500.0));
        ctrl.v_thumb_rect.set(Some((312.0, 100.0, 6.0, 80.0)));

        ctrl.scroll_bar_placement = ScrollBarPlacement::Floating;
        assert!(ctrl.hit_test_v_track(Vec2d { x: 309.0, y: 300.0 }, 320.0, 500.0, 12.0,));

        ctrl.scroll_bar_placement = ScrollBarPlacement::Inline;
        assert!(!ctrl.hit_test_v_track(Vec2d { x: 309.0, y: 300.0 }, 320.0, 500.0, 12.0,));
        assert!(ctrl.hit_test_v_track(Vec2d { x: 321.0, y: 300.0 }, 320.0, 500.0, 12.0,));
    }

    // Regression for "moving the cursor pins a core": with per-move redraws
    // gone, the highlight the draw pass resolves from the cursor only updates
    // on a frame somebody asked for — and the moves that cross a thumb edge
    // are the ones that must ask, exactly once per crossing.
    #[test]
    fn thumb_hover_changes_only_on_the_move_that_crosses_the_edge() {
        let ctrl = ctrl_with_offset(Vec2d::default());
        ctrl.v_thumb_rect.set(Some((390.0, 100.0, 10.0, 80.0)));

        assert!(
            !ctrl.sync_thumb_hover(Vec2d { x: 200.0, y: 120.0 }),
            "a move far from the thumb changes nothing"
        );
        assert!(
            ctrl.sync_thumb_hover(Vec2d { x: 395.0, y: 120.0 }),
            "the move that enters the thumb reports the transition"
        );
        assert!(
            !ctrl.sync_thumb_hover(Vec2d { x: 395.0, y: 150.0 }),
            "a move within the thumb reports nothing further"
        );
        assert!(
            ctrl.sync_thumb_hover(Vec2d { x: 200.0, y: 120.0 }),
            "the move that leaves the thumb reports the transition"
        );
    }

    // Reconciliation contract: on rebuild the freshly built controller starts at
    // its initial offset (top), then adopts the previous controller's live scroll
    // position so the viewport doesn't snap back to the top.
    #[test]
    fn adopt_scroll_state_preserves_offset() {
        let prev = ctrl_with_offset(Vec2d { x: 3.0, y: 150.0 });
        prev.pointer_velocity.set(Vec2d { x: 0.0, y: -12.0 });
        prev.spring_velocity.set(Vec2d { x: 0.0, y: -200.0 });
        prev.cached_content_size.set(ResolvedSize {
            width: 320.0,
            height: 1_200.0,
        });
        prev.cached_content_size_valid.set(true);

        let fresh = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        assert_eq!(
            fresh.scroll_offset.get().y,
            0.0,
            "fresh element starts at the top"
        );

        fresh.adopt_scroll_state(&prev);

        assert_eq!(fresh.scroll_offset.get().x, 0.0);
        assert_eq!(fresh.scroll_offset.get().y, 150.0);
        assert_eq!(fresh.pointer_velocity.get().y, -12.0);
        assert_eq!(fresh.spring_velocity.get().y, -200.0);
        assert_eq!(fresh.cached_content_size.get().height, 1_200.0);
        assert!(fresh.cached_content_size_valid.get());
    }

    #[test]
    fn setting_an_offset_clears_the_inactive_axis() {
        let mut ctrl = ctrl_with_offset(Vec2d::ZERO);
        ctrl.set_scroll_offset(Vec2d { x: 24.0, y: -18.0 });
        assert_eq!(ctrl.offset(), Vec2d { x: 0.0, y: -18.0 });

        ctrl.axis = ScrollAxis::Horizontal;
        ctrl.set_scroll_offset(Vec2d { x: 24.0, y: -18.0 });
        assert_eq!(ctrl.offset(), Vec2d { x: 24.0, y: 0.0 });
    }

    /// A bouncy engine parked 40 px past the start edge, in a viewport large
    /// enough that [`MAX_OVERSCROLL_VIEWPORT_FRACTION`] does not clamp the
    /// stretch.
    fn overscrolled_ctrl() -> ScrollState {
        let ctrl = ctrl_with_offset(Vec2d { x: 0.0, y: 40.0 });
        ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });
        ctrl.cached_viewport.set((400.0, 800.0));
        ctrl.cached_content_size.set(ResolvedSize {
            width: 400.0,
            height: 2000.0,
        });
        ctrl
    }

    #[test]
    fn overscroll_recovery_waits_during_the_initial_grace_period() {
        let ctrl = overscrolled_ctrl();
        ctrl.hold_overscroll_recovery();

        let (offset, needs_redraw) = ctrl.update_momentum(ctrl.scroll_offset.get());

        assert_eq!(
            offset.y, 40.0,
            "the first overscroll frame keeps its complete stretch"
        );
        assert!(
            needs_redraw,
            "a held stretch keeps frames coming so recovery can start on its own"
        );
    }

    #[test]
    fn direct_manipulation_holds_overscroll_past_the_grace_period() {
        let ctrl = overscrolled_ctrl();
        let held_at = AnimInstant::now();
        ctrl.hold_overscroll_recovery_at(held_at);
        ctrl.hold_overscroll_for_direct_manipulation();

        let well_past_grace =
            held_at + Duration::from_millis(OVERSCROLL_HOLD_IDLE_MS as u64 + 1_000);

        assert!(ctrl.overscroll_recovery_held_at(well_past_grace));
        ctrl.release_overscroll_recovery();
        assert!(!ctrl.overscroll_recovery_held_at(well_past_grace));
    }

    #[test]
    fn a_lift_hands_the_contact_hold_over_to_the_grace_period() {
        let ctrl = overscrolled_ctrl();
        ctrl.hold_overscroll_for_direct_manipulation();

        // The first frame after the fingers left keeps the stretch only for the
        // grace period, so recovery starts on its own from there.
        let held_at = AnimInstant::now();
        ctrl.release_direct_overscroll_hold();
        ctrl.hold_overscroll_recovery_at(held_at);

        assert!(ctrl.overscroll_recovery_held_at(held_at));
        let stale = held_at + Duration::from_millis(OVERSCROLL_HOLD_IDLE_MS as u64 + 50);
        assert!(
            !ctrl.overscroll_recovery_held_at(stale),
            "a lifted gesture cannot hold the stretch forever"
        );
    }

    #[test]
    fn overscroll_recovery_starts_once_the_device_gesture_ends() {
        let ctrl = overscrolled_ctrl();
        ctrl.hold_overscroll_recovery();
        ctrl.release_overscroll_recovery();

        let (offset, needs_redraw) = ctrl.update_momentum(ctrl.scroll_offset.get());

        assert!(
            offset.y < 40.0 && offset.y >= 0.0,
            "the spring pulls the content back toward the edge, got {}",
            offset.y
        );
        assert!(needs_redraw);
    }

    #[test]
    fn held_overscroll_recovery_expires_after_the_grace_period() {
        let ctrl = overscrolled_ctrl();
        let held_at = AnimInstant::now();
        ctrl.hold_overscroll_recovery_at(held_at);

        assert!(ctrl.overscroll_recovery_held_at(held_at));

        let stale = held_at + Duration::from_millis(OVERSCROLL_HOLD_IDLE_MS as u64 + 50);
        assert!(
            !ctrl.overscroll_recovery_held_at(stale),
            "an unterminated gesture cannot hold the stretch forever"
        );
        assert!(
            ctrl.overscroll_hold.get().is_none(),
            "the expired hold is cleared so later frames take the normal path"
        );
    }

    // The stretch a user perceives is relative to what they can see, so the cap
    // belongs to the viewport. Measuring it against the content made a long
    // page allow a stretch of hundreds of pixels.
    #[test]
    fn a_held_gesture_never_stretches_past_the_overscroll_cap() {
        let ctrl = overscrolled_ctrl();
        ctrl.hold_overscroll_recovery();

        let (offset, _) = ctrl.update_momentum(Vec2d { x: 0.0, y: 5_000.0 });

        assert_eq!(offset.y, 800.0 * MAX_OVERSCROLL_VIEWPORT_FRACTION);
    }

    #[test]
    fn the_overscroll_cap_ignores_the_content_height() {
        let ctrl = overscrolled_ctrl();
        ctrl.cached_content_size.set(ResolvedSize {
            width: 400.0,
            height: 20_000.0,
        });
        ctrl.hold_overscroll_recovery();

        let (offset, _) = ctrl.update_momentum(Vec2d { x: 0.0, y: 5_000.0 });

        assert_eq!(
            offset.y,
            800.0 * MAX_OVERSCROLL_VIEWPORT_FRACTION,
            "a ten-times-longer page may not stretch ten times as far"
        );
    }

    // Out-of-range input must feel like a rubber band, not like a constant
    // gear ratio: the further the content is pulled, the less each further
    // step of the gesture moves it.
    #[test]
    fn outward_resistance_grows_with_the_stretch() {
        let ctrl = overscrolled_ctrl();
        let limit = ctrl.overscroll_limit().y;

        ctrl.scroll_offset.set(Vec2d { x: 0.0, y: 1.0 });
        let near_edge = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 }).y;

        ctrl.scroll_offset.set(Vec2d {
            x: 0.0,
            y: limit * 0.5,
        });
        let half_way = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 }).y;

        ctrl.scroll_offset.set(Vec2d { x: 0.0, y: limit });
        let at_limit = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 }).y;

        assert!(near_edge > half_way, "{near_edge} > {half_way}");
        assert!(half_way > at_limit, "{half_way} > {at_limit}");
        assert_eq!(at_limit, 0.0, "the limit absorbs the whole step");
        assert!(
            near_edge <= 30.0 * OOB_DRAG_RESISTANCE,
            "the base resistance stays the softest the edge ever is"
        );
    }

    // The reported bug: holding a trackpad gesture at an edge kept feeding
    // outward deltas, and a constant resistance let them creep on until a
    // content-relative cap stopped them half a screen away.
    #[test]
    fn a_sustained_gesture_at_the_edge_converges_on_the_overscroll_limit() {
        let ctrl = overscrolled_ctrl();
        ctrl.scroll_offset.set(Vec2d::ZERO);
        let limit = ctrl.overscroll_limit().y;

        for _ in 0..400 {
            let step = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 });
            let next = ctrl.apply_wheel_delta(ctrl.scroll_offset.get(), step);
            ctrl.scroll_offset.set(next);
        }

        let stretch = ctrl.overscroll_distance(ctrl.scroll_offset.get()).y;
        assert!(
            stretch <= limit,
            "a held gesture may never pull past the limit, got {stretch} > {limit}"
        );
        assert!(
            stretch > limit * 0.5,
            "but it must still reach a clearly visible stretch, got {stretch}"
        );
    }

    #[test]
    fn pulling_back_from_a_stretch_is_never_resisted() {
        let ctrl = overscrolled_ctrl();

        let step = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: -25.0 });

        assert_eq!(
            step.y, -25.0,
            "input that shortens the stretch answers the finger one to one"
        );
    }

    #[test]
    fn an_in_range_step_is_never_resisted() {
        let ctrl = overscrolled_ctrl();
        ctrl.scroll_offset.set(Vec2d { x: 0.0, y: -200.0 });

        let step = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 });

        assert_eq!(step.y, 30.0);
    }

    #[test]
    fn a_non_bouncy_scrollable_ignores_the_recovery_hold() {
        let mut ctrl = overscrolled_ctrl();
        ctrl.scroll_behavior = ScrollBehavior::no_bounce();
        ctrl.hold_overscroll_recovery();

        let (offset, _) = ctrl.update_momentum(Vec2d { x: 0.0, y: 40.0 });

        assert_eq!(offset.y, 0.0, "a non-bouncy viewport clamps to the edge");
    }

    #[test]
    fn overscroll_recovery_idle_timeout_is_short_enough_to_feel_immediate() {
        let timeout_ms = std::hint::black_box(OVERSCROLL_HOLD_IDLE_MS);
        assert!(
            timeout_ms <= 4,
            "edge recovery should be eligible by the next rendered frame"
        );
    }

    // A delta that stretches the content beyond the episode's existing peak is
    // renewed user intent and must still move the content.
    #[test]
    fn a_stronger_device_delta_extends_the_overscroll_peak() {
        let ctrl = overscrolled_ctrl();
        ctrl.record_overscroll_peak(ctrl.scroll_offset.get());

        let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 15.0 });

        assert!(applied, "a stronger stretch is never ignored");
        assert_eq!(ctrl.scroll_offset.get().y, 55.0);
        assert_eq!(ctrl.highest_overscroll_offset.get().y, 55.0);
    }

    // Once recovery starts, weaker outward tail events must not pump the
    // content back away from the edge.
    #[test]
    fn a_weaker_re_stretch_is_ignored_while_the_recovery_runs() {
        let ctrl = overscrolled_ctrl();
        ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
        ctrl.scroll_offset.set(Vec2d { x: 0.0, y: 12.0 });

        let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 8.0 });

        assert!(!applied, "20 px of stretch cannot beat a 40 px peak");
        assert_eq!(ctrl.scroll_offset.get().y, 12.0);
        assert_eq!(ctrl.highest_overscroll_offset.get().y, 40.0);
    }

    #[test]
    fn a_delta_pulling_back_toward_the_edge_is_never_ignored() {
        let ctrl = overscrolled_ctrl();
        ctrl.record_overscroll_peak(ctrl.scroll_offset.get());

        let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: -25.0 });

        assert!(applied);
        assert_eq!(ctrl.scroll_offset.get().y, 15.0);
        assert_eq!(ctrl.highest_overscroll_offset.get().y, 40.0);
    }

    #[test]
    fn returning_in_range_keeps_the_peak_for_the_remaining_device_tail() {
        let ctrl = overscrolled_ctrl();
        ctrl.record_overscroll_peak(ctrl.scroll_offset.get());

        let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: -80.0 });

        assert!(applied);
        assert_eq!(ctrl.scroll_offset.get().y, -40.0);
        assert_eq!(
            ctrl.highest_overscroll_offset.get().y,
            40.0,
            "the old gesture's remaining events must stay gated after recovery"
        );
    }

    #[test]
    fn a_late_device_tail_cannot_bounce_after_recovery_finishes() {
        let ctrl = overscrolled_ctrl();
        ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
        ctrl.release_overscroll_recovery();

        assert_eq!(
            ctrl.highest_overscroll_offset.get().y,
            40.0,
            "the peak must survive recovery start to guard the remaining tail"
        );

        let (offset, _) = ctrl.update_momentum(Vec2d { x: 0.0, y: 0.0 });
        ctrl.scroll_offset.set(offset);
        let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 8.0 });

        assert_eq!(offset.y, 0.0);
        assert!(!applied, "the recovered gesture's outward tail is ignored");
        assert_eq!(ctrl.scroll_offset.get().y, 0.0);
        assert_eq!(ctrl.highest_overscroll_offset.get().y, 40.0);
    }

    #[test]
    fn a_new_device_gesture_can_overscroll_the_same_edge() {
        let ctrl = overscrolled_ctrl();
        ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
        ctrl.scroll_offset.set(Vec2d::ZERO);

        ctrl.reset_overscroll_peak();
        let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 8.0 });

        assert!(applied);
        assert_eq!(ctrl.scroll_offset.get().y, 8.0);
        assert_eq!(ctrl.highest_overscroll_offset.get().y, 8.0);
    }

    // The peak only guards against a *live* device tail. Once the device stops
    // sending events there is no tail left to guard, so a peak that nothing
    // refreshed must stop gating — otherwise a missed gesture boundary wedges
    // the edge and no later gesture can overscroll at all.
    #[test]
    fn an_idle_overscroll_peak_stops_gating_the_next_gesture() {
        let ctrl = overscrolled_ctrl();
        ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
        ctrl.scroll_offset.set(Vec2d::ZERO);

        let idle = AnimInstant::now() + Duration::from_millis(OVERSCROLL_PEAK_IDLE_MS as u64 + 10);
        let applied = ctrl.apply_precise_scroll_delta_at(Vec2d { x: 0.0, y: 8.0 }, idle);

        assert!(applied, "a stale peak cannot block the edge forever");
        assert_eq!(ctrl.scroll_offset.get().y, 8.0);
        assert_eq!(ctrl.highest_overscroll_offset.get().y, 8.0);
    }

    // A native momentum tail delivers an event every frame. Each of those
    // events — including the ones the peak rejects — proves the tail is still
    // running, so it must keep the peak alive.
    #[test]
    fn a_rejected_tail_event_keeps_the_peak_alive() {
        let ctrl = overscrolled_ctrl();
        let started = AnimInstant::now();
        ctrl.record_overscroll_peak_at(ctrl.scroll_offset.get(), started);
        ctrl.scroll_offset.set(Vec2d::ZERO);

        let mut at = started;
        for _ in 0..12 {
            at += Duration::from_millis(OVERSCROLL_PEAK_IDLE_MS as u64 / 2);
            assert!(
                !ctrl.apply_precise_scroll_delta_at(Vec2d { x: 0.0, y: 8.0 }, at),
                "a weaker tail event stays gated while the tail keeps arriving"
            );
        }

        assert_eq!(ctrl.scroll_offset.get().y, 0.0);
        assert_eq!(ctrl.highest_overscroll_offset.get().y, 40.0);
    }

    #[test]
    fn fresh_contact_clears_the_previous_gesture_peak() {
        let ctrl = overscrolled_ctrl();
        ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
        ctrl.scroll_offset.set(Vec2d::ZERO);

        assert!(
            ctrl.begin_device_contact(true),
            "fingers landing on the device is a new gesture"
        );
        assert_eq!(ctrl.highest_overscroll_offset.get(), Vec2d::ZERO);
        assert!(
            !ctrl.begin_device_contact(true),
            "later frames of the same contact are not a new gesture"
        );

        assert!(ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 8.0 }));
        assert_eq!(ctrl.scroll_offset.get().y, 8.0);
    }

    #[test]
    fn losing_contact_arms_the_next_gesture_detection() {
        let ctrl = overscrolled_ctrl();
        assert!(ctrl.begin_device_contact(true));
        assert!(!ctrl.begin_device_contact(false));
        assert!(
            ctrl.begin_device_contact(true),
            "a lift followed by a touch is a new gesture again"
        );
    }

    #[test]
    fn the_opposite_edge_starts_a_new_peak_during_the_same_gesture() {
        let ctrl = overscrolled_ctrl();
        ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
        ctrl.scroll_offset.set(Vec2d { x: 0.0, y: -1000.0 });

        let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: -8.0 });

        assert!(applied);
        assert_eq!(ctrl.scroll_offset.get().y, -1008.0);
        assert_eq!(ctrl.highest_overscroll_offset.get().y, -8.0);
    }

    #[test]
    fn controller_reattach_preserves_in_flight_scroll_state() {
        let ctrl = ScrollController::new();
        let first = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: -120.0 }));
        let fling_started = AnimInstant::now();
        first.pointer_velocity.set(Vec2d { x: 0.0, y: -12.0 });
        first.fling_start_time.set(Some(fling_started));
        first.fling_start_offset.set(Vec2d { x: 0.0, y: -120.0 });
        first.fling_target_offset.set(Vec2d { x: 0.0, y: -360.0 });
        first.fling_duration.set(1.25);
        first.is_scrolling.set(true);
        ctrl.attach(first);

        let rebuilt = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        ctrl.attach(rebuilt.clone());

        assert_eq!(rebuilt.scroll_offset.get().y, -120.0);
        assert_eq!(rebuilt.pointer_velocity.get().y, -12.0);
        assert!(rebuilt.fling_start_time.get().is_some());
        assert_eq!(rebuilt.fling_start_offset.get().y, -120.0);
        assert_eq!(rebuilt.fling_target_offset.get().y, -360.0);
        assert_eq!(rebuilt.fling_duration.get(), 1.25);
        assert!(rebuilt.is_scrolling.get());

        let (_, needs_redraw) = rebuilt.update_momentum(rebuilt.scroll_offset.get());
        assert!(
            needs_redraw,
            "the rebuilt engine continues the remaining fling"
        );
    }

    #[test]
    fn controller_reattach_preserves_active_drag() {
        let ctrl = ScrollController::new();
        let first = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: -120.0 }));
        first.drag_mode.set(DragMode::Content);
        first.active_touch_id.set(Some(7));
        first.last_pointer_pos.set(Some(Vec2d { x: 20.0, y: 80.0 }));
        ctrl.attach(first);

        let rebuilt = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        ctrl.attach(rebuilt.clone());

        assert_eq!(rebuilt.drag_mode.get(), DragMode::Content);
        assert_eq!(rebuilt.active_touch_id.get(), Some(7));
        let last_pointer_pos = rebuilt
            .last_pointer_pos
            .get()
            .expect("active drag keeps its last pointer position");
        assert_eq!(last_pointer_pos.x, 20.0);
        assert_eq!(last_pointer_pos.y, 80.0);
    }

    // Coalesced-events contract (the web "scroll too fast" bug): winit delivers
    // one native `pointermove` on web as a BURST of coalesced samples that all
    // read the same `Instant`. A naive per-sample `delta / dt` divides a tiny
    // sub-delta by a ~0 dt, so the release-fling velocity explodes (~Nx the real
    // finger speed for N coalesced samples). `accumulate_drag_velocity` folds a
    // whole frame's coalesced sub-samples into ONE realistic value, so a drag fed
    // as many fine same-instant sub-moves yields the same steady velocity as the
    // identical travel fed as one coarse move per frame — NOT an inflated one.
    #[test]
    fn coalesced_moves_match_coarse_drag_velocity() {
        use std::time::Duration;

        let frame = Duration::from_millis(16); // ~60 Hz
        let t0 = AnimInstant::now();
        let travel_per_frame = 16.0_f32; // px moved each frame
        let frames = 6;
        let sub = 8; // coalesced sub-samples per frame on web

        // Native path: one move per frame.
        let native = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        let mut last_native = Vec2d { x: 0.0, y: 0.0 };
        let mut t = t0;
        for _ in 0..frames {
            if let Some((v, _)) = native.accumulate_drag_velocity(0.0, travel_per_frame, t) {
                last_native = v;
            }
            t += frame;
        }

        // Web path: the SAME travel each frame, split into `sub` coalesced
        // sub-moves that all share the frame's instant.
        let web = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        let mut last_web = Vec2d { x: 0.0, y: 0.0 };
        let mut t = t0;
        let step = travel_per_frame / sub as f32;
        for _ in 0..frames {
            for _ in 0..sub {
                if let Some((v, _)) = web.accumulate_drag_velocity(0.0, step, t) {
                    last_web = v;
                }
            }
            t += frame;
        }

        // Steady-state velocities must match (both = travel / frame time); the
        // coalesced burst is not inflated by the number of sub-samples.
        assert!(
            (last_web.y - last_native.y).abs() < 0.5,
            "coalesced web velocity {} must match coarse native velocity {} (not inflated)",
            last_web.y,
            last_native.y
        );
        // Guard the direction of the bug: the fix must not let the web value run
        // away above the real per-frame speed.
        assert!(last_web.y <= last_native.y * 1.2 + 0.5);
    }

    // Small reverse-flick contract: the release fling reads `smoothed_velocity()`,
    // a weighted average of the velocity ring buffer. After a fast fling leaves the
    // buffer full of old-direction samples, a SMALL reverse flick pushes only 1–2
    // opposite samples — not enough to flip that average, so without clearing the
    // buffer the release still coasts the OLD way (the reported bug). Clearing the
    // history on a detected reversal (as handle_scroll now does) makes the release
    // reflect only the new direction.
    #[test]
    fn small_reverse_flick_clears_stale_velocity_history() {
        let c = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        let slice = Duration::from_micros(4_000);
        let slice_s = 0.004;
        let mut at = AnimInstant::now();

        // Prior fling: the horizon is full of positive (old-direction) samples.
        for _ in 0..6 {
            at += slice;
            c.push_velocity(Vec2d { x: 0.0, y: 100.0 }, slice_s, at);
        }

        // A small reverse flick adds a single opposite sample.
        at += slice;
        c.push_velocity(Vec2d { x: 0.0, y: -20.0 }, slice_s, at);
        // Without clearing, the average is still the OLD direction.
        assert!(
            c.smoothed_velocity(at).y > 0.0,
            "stale samples keep the average pointing the old way"
        );

        // Reversal handling clears the buffer before the new sample is recorded.
        c.clear_velocity_history();
        c.push_velocity(Vec2d { x: 0.0, y: -20.0 }, slice_s, at);
        assert!(
            c.smoothed_velocity(at).y < 0.0,
            "after clearing, the release follows the new direction"
        );
    }

    /// An engine tuned like a browser build: bouncy edges declared, but the
    /// wheel left out of the trusted devices.
    fn web_ctrl() -> ScrollState {
        let mut ctrl = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        ctrl.overscroll_sources = OverscrollSources::WEB_DEFAULT;
        ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });
        ctrl
    }

    #[test]
    fn a_wheel_gesture_does_not_bounce_where_the_bitmap_leaves_the_wheel_out() {
        let ctrl = web_ctrl();
        ctrl.set_overscroll_source(OverscrollSource::Wheel);

        assert!(
            !ctrl.bouncy(),
            "a browser wheel stream is not trusted with a rubber band"
        );
        let next = ctrl.apply_wheel_delta(Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 0.0, y: 40.0 });
        assert_eq!(next.y, 0.0, "the edge clamps instead of stretching");
    }

    #[test]
    fn a_touch_gesture_still_bounces_where_only_the_wheel_is_left_out() {
        let ctrl = web_ctrl();
        ctrl.set_overscroll_source(OverscrollSource::Touch);

        assert!(
            ctrl.bouncy(),
            "a touch screen reports its lift, so it keeps the rubber band"
        );
        let next = ctrl.apply_wheel_delta(Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 0.0, y: 40.0 });
        assert_eq!(next.y, 40.0, "the finger stretches the edge as usual");
    }

    #[test]
    fn a_rigid_viewport_stays_rigid_for_every_trusted_device() {
        let mut ctrl = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        ctrl.scroll_behavior.bouncy = false;
        ctrl.overscroll_sources = OverscrollSources::ALL;

        for source in [OverscrollSource::Wheel, OverscrollSource::Touch] {
            ctrl.set_overscroll_source(source);
            assert!(!ctrl.bouncy(), "the bitmap never grants a bounce");
        }
    }

    #[test]
    fn the_active_device_survives_reconciliation() {
        let prev = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        prev.set_overscroll_source(OverscrollSource::Touch);
        let fresh = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });

        fresh.adopt_scroll_state(&prev);

        assert_eq!(
            fresh.overscroll_source.get(),
            OverscrollSource::Touch,
            "a rebuild mid-gesture must not re-judge the edge as a wheel"
        );
    }

    // Regression: a non-bouncy scrollable must actually move when a wheel /
    // trackpad delta arrives while the offset is anywhere strictly inside the
    // valid range. The old handler compared the current offset against
    // `clamp_offset(offset)` and zeroed the delta when they were equal — which
    // they always are for an in-range offset — so wheel/trackpad scrolling was
    // completely dead. `apply_wheel_delta` must instead apply the delta first.
    #[test]
    fn wheel_delta_moves_non_bouncy_from_midrange() {
        let mut c = ctrl_with_offset(Vec2d { x: 0.0, y: -100.0 });
        c.scroll_behavior.bouncy = false;
        // Valid vertical range is [-1000, 0]; -100 is strictly inside it.
        c.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });

        // Scroll further down (offset grows more negative).
        let next = c.apply_wheel_delta(Vec2d { x: 0.0, y: -100.0 }, Vec2d { x: 0.0, y: -20.0 });
        assert_eq!(
            next.y, -120.0,
            "wheel delta must move an in-range non-bouncy offset"
        );

        // Scroll back up.
        let up = c.apply_wheel_delta(Vec2d { x: 0.0, y: -100.0 }, Vec2d { x: 0.0, y: 15.0 });
        assert_eq!(up.y, -85.0);
    }

    // A non-bouncy scrollable never overscrolls: a delta past the edge clamps to
    // the boundary instead of running away.
    #[test]
    fn wheel_delta_clamps_non_bouncy_at_boundary() {
        let mut c = ctrl_with_offset(Vec2d { x: 0.0, y: -990.0 });
        c.scroll_behavior.bouncy = false;
        c.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });

        // Overshoot the bottom edge (-1000): must clamp, not exceed.
        let next = c.apply_wheel_delta(Vec2d { x: 0.0, y: -990.0 }, Vec2d { x: 0.0, y: -50.0 });
        assert_eq!(
            next.y, -1000.0,
            "non-bouncy offset clamps at the bottom edge"
        );

        // Overshoot the top edge (0): must clamp to 0.
        let top = c.apply_wheel_delta(Vec2d { x: 0.0, y: -10.0 }, Vec2d { x: 0.0, y: 40.0 });
        assert_eq!(top.y, 0.0, "non-bouncy offset clamps at the top edge");
    }

    // A bouncy scrollable keeps the raw (out-of-range) offset so the visual
    // rubber-band + spring-back can act; `apply_wheel_delta` must not clamp it.
    #[test]
    fn wheel_delta_allows_overscroll_when_bouncy() {
        let c = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        assert!(c.scroll_behavior.bouncy, "default behavior is bouncy");
        c.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });

        // Overscroll past the top edge (0) is preserved, not clamped.
        let next = c.apply_wheel_delta(Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 0.0, y: 30.0 });
        assert_eq!(
            next.y, 30.0,
            "bouncy offset keeps the overscroll for the rubber-band"
        );

        // A normal in-range scroll still moves.
        let down = c.apply_wheel_delta(Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 0.0, y: -30.0 });
        assert_eq!(down.y, -30.0);
    }

    // -- Public ScrollController handle -------------------------------------

    /// Build an attached controller over a fresh engine with a vertical range
    /// of `[0, max_y]` (logical == internal, since `last_scale` is 1.0).
    fn attached(max_y: f32) -> (ScrollController, Rc<ScrollState>) {
        let state = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        state.cached_max_scroll.set(Vec2d { x: 0.0, y: max_y });
        let ctrl = ScrollController::new();
        ctrl.attach(state.clone());
        (ctrl, state)
    }

    #[test]
    fn controller_is_attached_toggles() {
        let ctrl = ScrollController::new();
        assert!(!ctrl.is_attached(), "a fresh controller is detached");
        let state = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        ctrl.attach(state);
        assert!(ctrl.is_attached(), "after attach the controller is live");
    }

    // `jump_to` takes a logical, positive-toward-content-end position and maps it
    // to the internal negated/scaled offset; `offset()` reads it back logical.
    #[test]
    fn controller_jump_to_converts_and_reads_back() {
        let (ctrl, state) = attached(1000.0);

        ctrl.jump_to(Vec2d { x: 0.0, y: 120.0 });
        assert_eq!(
            state.scroll_offset.get().y,
            -120.0,
            "internal offset is negated"
        );
        assert_eq!(
            ctrl.offset().y,
            120.0,
            "public offset reads back the logical position"
        );
    }

    // `jump_to` clamps to the valid range (never past the bottom edge).
    #[test]
    fn controller_jump_to_clamps_to_range() {
        let (ctrl, _state) = attached(1000.0);
        ctrl.jump_to(Vec2d { x: 0.0, y: 5000.0 });
        assert_eq!(
            ctrl.offset().y,
            1000.0,
            "over-scroll target clamps to max extent"
        );
    }

    // A position requested before the controller is attached is remembered and
    // applied on attach (Flutter's `initialScrollOffset` behaviour).
    #[test]
    fn controller_jump_to_before_attach_is_pending_then_applied() {
        let ctrl = ScrollController::new();
        ctrl.jump_to(Vec2d { x: 0.0, y: 75.0 });
        // Detached: reported via the pending value.
        assert_eq!(ctrl.offset().y, 75.0);

        let state = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        ctrl.attach(state.clone());
        assert_eq!(
            state.scroll_offset.get().y,
            -75.0,
            "pending position applied on attach"
        );
    }

    // `max_extent` reports the scrollable range in logical pixels.
    #[test]
    fn controller_max_extent_reports_logical() {
        let (ctrl, _state) = attached(640.0);
        assert_eq!(ctrl.max_extent().y, 640.0);
    }

    // `animate_to` with a real duration arms the curve-driven fling rather than
    // snapping: the fling target/curve are set and the position hasn't moved yet.
    #[test]
    fn controller_animate_to_arms_curve_driven_fling() {
        let (ctrl, state) = attached(1000.0);

        ctrl.animate_to(
            Vec2d { x: 0.0, y: 200.0 },
            Duration::from_millis(300),
            curves::LINEAR,
        );

        assert!(
            state.fling_start_time.get().is_some(),
            "a timed animation arms the fling"
        );
        assert_eq!(
            state.anim_curve.borrow().clone(),
            Some(curves::LINEAR.into_any()),
            "the requested curve drives the fling"
        );
        assert_eq!(
            state.fling_target_offset.get().y,
            -200.0,
            "target stored in internal convention"
        );
        assert_eq!(
            state.scroll_offset.get().y,
            0.0,
            "position has not jumped — it will ease over time"
        );
    }

    // A zero-duration `animate_to` degenerates to an instant jump.
    #[test]
    fn controller_animate_to_zero_duration_jumps_immediately() {
        let (ctrl, state) = attached(1000.0);
        ctrl.animate_to(Vec2d { x: 0.0, y: 300.0 }, Duration::ZERO, curves::EASE_IN_OUT);
        assert_eq!(ctrl.offset().y, 300.0);
        assert!(
            state.fling_start_time.get().is_none(),
            "no fling is left running"
        );
    }

    // Once the animation's duration has elapsed, the draw-phase `update_momentum`
    // lands exactly on the target and clears the fling.
    #[test]
    fn controller_animate_to_reaches_target_after_duration() {
        let (ctrl, state) = attached(1000.0);
        ctrl.animate_to(
            Vec2d { x: 0.0, y: 250.0 },
            Duration::from_millis(300),
            curves::LINEAR,
        );

        // Pretend the whole duration has already passed.
        state
            .fling_start_time
            .set(Some(AnimInstant::now() - Duration::from_millis(400)));
        let (offset, _redraw) = state.update_momentum(state.scroll_offset.get());

        assert!(
            (offset.y - (-250.0)).abs() < 1.0,
            "animation settles on the target (got {})",
            offset.y
        );
        assert!(
            state.fling_start_time.get().is_none(),
            "the fling is cleared once complete"
        );
        assert!(
            state.anim_curve.borrow().is_none(),
            "the animation curve is cleared once complete"
        );
    }

    #[test]
    fn line_wheel_delta_applies_immediately_without_momentum() {
        let (_ctrl, state) = attached(1000.0);
        state.start_animation(Vec2d { x: 0.0, y: -100.0 }, 1.0, curves::EASE_OUT);
        state.pointer_velocity.set(Vec2d { x: 0.0, y: -8.0 });

        state.apply_line_wheel_delta(Vec2d { x: 0.0, y: -20.0 });

        assert_eq!(state.scroll_offset.get().x, 0.0);
        assert_eq!(state.scroll_offset.get().y, -20.0);
        assert_eq!(state.pointer_velocity.get().x, 0.0);
        assert_eq!(state.pointer_velocity.get().y, 0.0);
        assert!(state.fling_start_time.get().is_none());
    }

    #[test]
    fn precise_device_delta_does_not_add_widget_side_momentum() {
        let (_ctrl, state) = attached(1000.0);
        state.pointer_velocity.set(Vec2d { x: 0.0, y: -8.0 });

        state.apply_precise_scroll_delta(Vec2d { x: 0.0, y: -4.0 });

        assert_eq!(state.scroll_offset.get().y, -4.0);
        assert_eq!(state.pointer_velocity.get().x, 0.0);
        assert_eq!(state.pointer_velocity.get().y, 0.0);
        assert!(state.momentum_start_time.get().is_none());
    }

    // -- Scroll-lifecycle callbacks (on_scroll_start / on_scroll_end) --------

    /// A recording callback: pushes each offset it receives into a shared log
    /// so tests can assert how many times (and with what offset) it fired.
    fn recorder() -> (Callback<Vec2d>, Rc<RefCell<Vec<Vec2d>>>) {
        let log: Rc<RefCell<Vec<Vec2d>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = log.clone();
        let cb = Callback::from(move |o: Vec2d| sink.borrow_mut().push(o));
        (cb, log)
    }

    // `begin_scroll` / `end_scroll` are edge-triggered: each fires its callback
    // exactly once per idle↔scrolling transition, never per redundant call.
    #[test]
    fn scroll_callbacks_are_edge_triggered() {
        let state = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        let (start_cb, starts) = recorder();
        let (end_cb, ends) = recorder();
        *state.on_scroll_start.borrow_mut() = start_cb;
        *state.on_scroll_end.borrow_mut() = end_cb;

        state.begin_scroll();
        state.begin_scroll(); // already scrolling → ignored
        assert_eq!(
            starts.borrow().len(),
            1,
            "start fires once on the idle→scrolling edge"
        );
        assert_eq!(ends.borrow().len(), 0, "end has not fired yet");

        state.end_scroll();
        state.end_scroll(); // already idle → ignored
        assert_eq!(
            ends.borrow().len(),
            1,
            "end fires once on the scrolling→idle edge"
        );

        // A brand-new session fires start again.
        state.begin_scroll();
        assert_eq!(starts.borrow().len(), 2, "a fresh session re-fires start");
    }

    // The start callback receives the logical offset at the moment scrolling
    // begins.
    #[test]
    fn scroll_start_reports_logical_offset() {
        let state = ctrl_with_offset(Vec2d { x: 0.0, y: -150.0 });
        let (start_cb, starts) = recorder();
        *state.on_scroll_start.borrow_mut() = start_cb;

        state.begin_scroll();
        assert_eq!(
            starts.borrow()[0].y,
            150.0,
            "offset is reported logical (positive toward content end)"
        );
    }

    // A programmatic `jump_to` is a self-contained session: it fires exactly one
    // start and one end, in that order, around the instant position change.
    #[test]
    fn jump_to_fires_one_start_then_one_end() {
        let (ctrl, _state) = attached(1000.0);
        let (start_cb, starts) = recorder();
        let (end_cb, ends) = recorder();
        ctrl.on_scroll_start(start_cb);
        ctrl.on_scroll_end(end_cb);

        ctrl.jump_to(Vec2d { x: 0.0, y: 120.0 });

        assert_eq!(starts.borrow().len(), 1, "jump fires start once");
        assert_eq!(ends.borrow().len(), 1, "jump fires end once");
        assert_eq!(ends.borrow()[0].y, 120.0, "end reports the landing offset");
    }

    // `animate_to` announces the session start immediately but leaves it open —
    // the draw loop fires `end` only once the animation settles (see
    // `draw_scroll`), so no `end` is emitted at arming time.
    #[test]
    fn animate_to_fires_start_but_not_end_until_settled() {
        let (ctrl, state) = attached(1000.0);
        let (start_cb, starts) = recorder();
        let (end_cb, ends) = recorder();
        ctrl.on_scroll_start(start_cb);
        ctrl.on_scroll_end(end_cb);

        ctrl.animate_to(
            Vec2d { x: 0.0, y: 200.0 },
            Duration::from_millis(300),
            curves::LINEAR,
        );
        assert_eq!(starts.borrow().len(), 1, "animation fires start when armed");
        assert_eq!(
            ends.borrow().len(),
            0,
            "end waits for the animation to settle"
        );

        // Simulate the draw loop reporting the motion as fully settled.
        state.end_scroll();
        assert_eq!(ends.borrow().len(), 1, "end fires once the session settles");
    }

    // Callbacks are held on the controller, so they keep firing after a rebuild
    // re-attaches the controller to a freshly built engine.
    #[test]
    fn callbacks_survive_reattach_across_rebuild() {
        let ctrl = ScrollController::new();
        let (start_cb, starts) = recorder();
        ctrl.on_scroll_start(start_cb);

        // First build.
        let first = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        ctrl.attach(first);

        // Rebuild: a brand-new engine adopts the same controller.
        let second = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        ctrl.attach(second.clone());

        second.begin_scroll();
        assert_eq!(
            starts.borrow().len(),
            1,
            "the callback still fires on the re-attached engine"
        );
    }

    // Registering a callback before the controller is attached still works: it is
    // stored on the controller and shared into the engine on attach.
    #[test]
    fn callback_registered_before_attach_is_applied_on_attach() {
        let ctrl = ScrollController::new();
        let (start_cb, starts) = recorder();
        ctrl.on_scroll_start(start_cb);

        let state = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        ctrl.attach(state.clone());

        state.begin_scroll();
        assert_eq!(
            starts.borrow().len(),
            1,
            "a pre-attach registration is applied on attach"
        );
    }

    // -- Per-frame scroll update callback (on_scroll) -----------------------

    // `notify_scroll` is level-triggered: the first call only establishes the
    // baseline (no fire, so the initial render is silent); afterwards it fires on
    // every call where the logical offset actually moved.
    #[test]
    fn on_scroll_is_level_triggered_after_baseline() {
        let state = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
        let (cb, log) = recorder();
        *state.on_scroll.borrow_mut() = cb;

        // First frame: only establishes the baseline, no callback.
        state.notify_scroll();
        assert_eq!(
            log.borrow().len(),
            0,
            "the initial frame establishes the baseline silently"
        );

        // Offset moves → fires with the new logical offset.
        state.scroll_offset.set(Vec2d { x: 0.0, y: -40.0 });
        state.notify_scroll();
        assert_eq!(log.borrow().len(), 1, "a genuine move fires the callback");
        assert_eq!(
            log.borrow()[0].y,
            40.0,
            "reports the logical offset (positive toward content end)"
        );

        // Another distinct move fires again (fires per frame, not per session).
        state.scroll_offset.set(Vec2d { x: 0.0, y: -90.0 });
        state.notify_scroll();
        assert_eq!(
            log.borrow().len(),
            2,
            "each frame with movement fires again"
        );
        assert_eq!(log.borrow()[1].y, 90.0);
    }

    // An unchanged (or sub-epsilon) offset does not re-fire — idle frames and
    // sub-pixel jitter are collapsed to no-ops.
    #[test]
    fn on_scroll_does_not_fire_without_movement() {
        let state = ctrl_with_offset(Vec2d { x: 0.0, y: -40.0 });
        let (cb, log) = recorder();
        *state.on_scroll.borrow_mut() = cb;

        state.notify_scroll(); // baseline at y = 40 (logical)
        state.notify_scroll(); // identical offset → no fire
        assert_eq!(log.borrow().len(), 0, "an unchanged offset does not fire");

        // Sub-epsilon jitter is ignored too.
        state.scroll_offset.set(Vec2d {
            x: 0.0,
            y: -40.0 - SCROLL_NOTIFY_EPSILON / 2.0,
        });
        state.notify_scroll();
        assert_eq!(log.borrow().len(), 0, "sub-epsilon jitter is suppressed");
    }

    // A programmatic `jump_to` moves the offset, so the very next drawn frame
    // reports the new position through `on_scroll`.
    #[test]
    fn on_scroll_fires_after_jump_on_next_frame() {
        let (ctrl, state) = attached(1000.0);
        let (cb, log) = recorder();
        ctrl.on_scroll(cb);

        // Establish the baseline (as the first render frame would).
        state.notify_scroll();
        assert_eq!(log.borrow().len(), 0);

        ctrl.jump_to(Vec2d { x: 0.0, y: 150.0 });
        // The draw loop calls `notify_scroll` once per frame; simulate that.
        state.notify_scroll();
        assert_eq!(
            log.borrow().len(),
            1,
            "the jump is reported on the next frame"
        );
        assert_eq!(log.borrow()[0].y, 150.0);
    }

    // The per-frame callback is held on the controller, so it keeps firing after
    // a rebuild re-attaches the controller to a freshly built engine.
    #[test]
    fn on_scroll_survives_reattach_across_rebuild() {
        let ctrl = ScrollController::new();
        let (cb, log) = recorder();
        ctrl.on_scroll(cb);

        let first = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        ctrl.attach(first);

        // Rebuild: a brand-new engine adopts the same controller.
        let second = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
        ctrl.attach(second.clone());

        second.notify_scroll(); // baseline on the fresh engine
        second.scroll_offset.set(Vec2d { x: 0.0, y: -60.0 });
        second.notify_scroll();
        assert_eq!(
            log.borrow().len(),
            1,
            "the callback still fires on the re-attached engine"
        );
        assert_eq!(log.borrow()[0].y, 60.0);
    }
}
