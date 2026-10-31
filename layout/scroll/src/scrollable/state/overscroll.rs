//! Overscroll: which device may stretch an edge, the recovery holds, the
//! per-gesture stretch peak, and the resistance and cap applied to a stretch.

use std::cell::Cell;

use aimer_attribute::position::Vec2d;
use aimer_utils::AnimInstant;

use super::controller::*;
use crate::scrollable::ScrollAxis;
use crate::scrollable::constants::*;
use crate::scrollable::overscroll_source::OverscrollSource;

/// The furthest a gesture has stretched an edge, and when that last mattered.
#[derive(Clone, Copy)]
struct Peak {
    /// Furthest signed per-axis distance the content was stretched past an
    /// edge during the current overscroll episode, in the same convention as
    /// [`scroll_offset`](ScrollState::scroll_offset).
    stretch: Vec2d,
    /// Instant the mark was last raised or measured against a device delta.
    at: AnimInstant,
}

/// The state of one overscroll episode: who is stretching the edge, what holds
/// recovery back, and how far the gesture has pulled.
///
/// Each piece answers a different platform quirk, but they all start and end
/// with the same gesture, so they are kept together and only reachable through
/// the methods below. That is what keeps their invariants (a peak always has a
/// timestamp, a release always clears both holds) in one place.
pub struct OverscrollState {
    /// The device that drove the most recent scroll input, which is the one
    /// [`bouncy`](ScrollState::bouncy) is resolved against.
    ///
    /// Momentum, spring-back and every other frame the draw loop produces
    /// belong to the gesture that started them, so this deliberately survives
    /// until the next input arrives.
    source: Cell<OverscrollSource>,
    /// Instant when a wheel / trackpad gesture first crossed a scroll boundary,
    /// or `None` when no recovery grace period is active.
    ///
    /// While this contains a recent timestamp, bouncy edge recovery is held so
    /// the first overscroll event can render without immediately fighting the
    /// spring. Later events do not refresh this grace period. The timestamp is
    /// what makes the hold self-terminating: see [`held_at`](Self::held_at).
    grace: Cell<Option<AnimInstant>>,
    /// Whether active trackpad contact is holding the stretched edge.
    ///
    /// Unlike the grace period, this does not expire: the platform's
    /// direct-manipulation signal releases it exactly when the user lifts their
    /// fingers. Post-lift momentum never sets it.
    contact_hold: Cell<bool>,
    /// The high-water mark that rejects weaker outward tail deltas once
    /// recovery has started. `None` when no episode is live.
    ///
    /// It survives reaching the boundary and is cleared when the device
    /// gesture terminates, a new gesture starts, or it goes idle.
    ///
    /// The mark is only meaningful while the gesture that produced it can still
    /// have events in flight, so it is timed exactly like the grace period:
    /// after [`OVERSCROLL_PEAK_IDLE_MS`] without a single device event the
    /// episode is over and the mark is dropped. Without this the mark depends
    /// on gesture-boundary phases alone, and a boundary the platform never
    /// delivers (a smoothing channel that stays active across two gestures, a
    /// lift swallowed by a focus change) leaves the edge permanently unable to
    /// stretch.
    peak: Cell<Option<Peak>>,
    /// Whether the last delivered scroll frame reported fingers on the device.
    ///
    /// Used to detect the rising edge of physical contact, which starts a new
    /// overscroll episode regardless of the phase the smoothing layer chose to
    /// label that frame with.
    fingers_down: Cell<bool>,
    /// Recognizes the browser's post-lift momentum tail, which is the only
    /// lift signal a web scroll gesture ever gets.
    ///
    /// Native platforms report contact themselves, so this exists on the web
    /// target only and no other target pays for it.
    #[cfg(target_arch = "wasm32")]
    pub(crate) web_decay: crate::scrollable::web_overscroll::WebOverscrollDecay,
    /// Closes the synthesized web gesture once its bouncy edge has recovered,
    /// and discards whatever the browser still delivers for it.
    ///
    /// Native platforms end their own gestures, so this exists on the web
    /// target only and no other target pays for it.
    #[cfg(target_arch = "wasm32")]
    pub(crate) web_recovery_end: crate::scrollable::web_recovery_end::WebRecoveryEnd,
}

impl Default for OverscrollState {
    fn default() -> Self {
        Self {
            source: Cell::new(OverscrollSource::Wheel),
            grace: Cell::new(None),
            contact_hold: Cell::new(false),
            peak: Cell::new(None),
            fingers_down: Cell::new(false),
            #[cfg(target_arch = "wasm32")]
            web_decay: crate::scrollable::web_overscroll::WebOverscrollDecay::new(),
            #[cfg(target_arch = "wasm32")]
            web_recovery_end: crate::scrollable::web_recovery_end::WebRecoveryEnd::new(),
        }
    }
}

impl OverscrollState {
    /// Carries the gesture-level state across a rebuild. The web-only
    /// recognizers restart with the new engine, as they always have.
    pub(crate) fn adopt_from(&self, prev: &Self, axis: ScrollAxis) {
        self.grace.set(prev.grace.get());
        self.source.set(prev.source.get());
        self.contact_hold.set(prev.contact_hold.get());
        self.peak.set(prev.peak.get().map(|peak| Peak {
            stretch: ScrollState::axis_only(axis, peak.stretch),
            at: peak.at,
        }));
        self.fingers_down.set(prev.fingers_down.get());
    }

    /// The device that drove the most recent scroll input.
    #[inline]
    pub(crate) fn source(&self) -> OverscrollSource {
        self.source.get()
    }

    #[inline]
    fn set_source(&self, source: OverscrollSource) {
        self.source.set(source);
    }

    /// Starts (or restarts) the recovery grace period at `at`.
    #[inline]
    fn start_grace(&self, at: AnimInstant) {
        self.grace.set(Some(at));
    }

    /// Whether a grace period is running, expired or not.
    #[cfg(test)]
    #[inline]
    pub(crate) fn has_grace(&self) -> bool {
        self.grace.get().is_some()
    }

    /// Holds recovery for as long as the fingers stay on the device.
    #[inline]
    fn hold_for_contact(&self) {
        self.contact_hold.set(true);
    }

    /// Drops both holds. The peak is deliberately kept: it guards the recovery
    /// that starts when the holds are released.
    #[inline]
    fn release(&self) {
        self.grace.set(None);
        self.contact_hold.set(false);
    }

    /// Drops only the contact hold, leaving a running grace period in place.
    #[inline]
    fn release_contact_hold(&self) {
        self.contact_hold.set(false);
    }

    /// Whether recovery is held back at `now`.
    ///
    /// A contact hold always holds. A grace period older than
    /// [`OVERSCROLL_HOLD_IDLE_MS`] is expired and cleared on the spot so
    /// recovery starts independently of the device gesture's tail.
    fn held_at(&self, now: AnimInstant) -> bool {
        if self.contact_hold.get() {
            return true;
        }
        match self.grace.get() {
            Some(start) => {
                let held = now.duration_since(start).as_millis() <= OVERSCROLL_HOLD_IDLE_MS;
                if !held {
                    self.grace.set(None);
                }
                held
            }
            None => false,
        }
    }

    /// The peak's stretch, or zero when no episode is live.
    #[inline]
    pub(crate) fn peak_stretch(&self) -> Vec2d {
        self.peak.get().map_or(Vec2d::ZERO, |peak| peak.stretch)
    }

    /// Raises the peak for each axis `stretch` pushes further out than it, and
    /// stamps it with `at`. A stretch that does not raise anything leaves the
    /// peak, and its timestamp, alone: a shrinking stretch (spring recovery)
    /// must not keep the episode alive, or the mark could never go idle.
    fn raise_peak(&self, stretch: Vec2d, at: AnimInstant) {
        let mut peak = self.peak_stretch();
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
            self.peak.set(Some(Peak { stretch: peak, at }));
        }
    }

    /// Restamps a live peak at `now`; there is nothing to keep alive without
    /// one.
    #[inline]
    fn keep_peak_alive(&self, now: AnimInstant) {
        if let Some(peak) = self.peak.get() {
            self.peak.set(Some(Peak { at: now, ..peak }));
        }
    }

    /// Clears device-tail suppression at a gesture lifecycle boundary.
    #[inline]
    fn reset_peak(&self) {
        self.peak.set(None);
    }

    /// The peak's stretch if its episode is still live at `now`. A peak whose
    /// episode went idle is dropped and no longer gates anything.
    fn live_peak_at(&self, now: AnimInstant) -> Option<Vec2d> {
        match self.peak.get() {
            Some(peak) if now.duration_since(peak.at).as_millis() <= OVERSCROLL_PEAK_IDLE_MS => {
                Some(peak.stretch)
            }
            Some(_) => {
                self.reset_peak();
                None
            }
            None => None,
        }
    }

    /// Folds one frame's contact signal in, reporting whether it opens a new
    /// physical gesture. A rising edge clears the previous episode's peak.
    fn begin_contact(&self, contact: bool) -> bool {
        let started = !self.fingers_down.replace(contact) && contact;
        if started {
            self.reset_peak();
        }
        started
    }
}

impl ScrollState {
    /// [`ScrollBehavior::bouncy`] declares whether the viewport bounces at
    /// all; [`overscroll_sources`](Self::overscroll_sources) decides which
    /// devices are trusted with it on this target. Both must agree, so a
    /// browser can keep bouncy touch scrolling while its wheel stream — which
    /// never reports the end of a gesture — clamps at the edge.
    #[inline]
    pub(crate) fn bouncy(&self) -> bool {
        self.scroll_behavior.bouncy && self.overscroll_sources.contains(self.overscroll.source())
    }

    /// Records the device that produced the input now being applied.
    ///
    /// Called at the head of every input path — wheel frames, pointer drags
    /// and keyboard steps — so the frames that follow resolve
    /// [`bouncy`](ScrollState::bouncy) against the device the user actually used.
    #[inline]
    pub(crate) fn set_overscroll_source(&self, source: OverscrollSource) {
        self.overscroll.set_source(source);
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
        self.overscroll.start_grace(at);
    }

    /// Hold bouncy recovery while the user keeps physical contact with the
    /// trackpad after stretching past an edge.
    #[inline]
    pub(crate) fn hold_overscroll_for_direct_manipulation(&self) {
        self.overscroll.hold_for_contact();
    }

    /// Release the hold: the gesture ended, so the next frame springs the
    /// content back to the edge. A no-op when nothing is held.
    ///
    /// The overscroll peak deliberately survives this call because it guards
    /// the recovery that starts when the hold is released.
    #[inline]
    pub(crate) fn release_overscroll_recovery(&self) {
        self.overscroll.release();
    }

    /// Release only the contact hold, leaving a timed hold in place.
    ///
    /// Used when a frame proves the fingers left the device while the edge is
    /// still stretched: the non-expiring contact hold must go, but the short
    /// grace period that lets the crossing frame reach the screen may stay.
    #[inline]
    pub(crate) fn release_direct_overscroll_hold(&self) {
        self.overscroll.release_contact_hold();
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
    pub(crate) fn record_overscroll_peak_at(&self, offset: Vec2d, at: AnimInstant) {
        self.overscroll
            .raise_peak(self.overscroll_distance(offset), at);
    }

    /// Clear device-tail suppression at a gesture lifecycle boundary.
    #[inline]
    pub(crate) fn reset_overscroll_peak(&self) {
        self.overscroll.reset_peak();
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
        self.overscroll.begin_contact(contact)
    }

    /// Keeps a live high-water mark alive after a device delta was measured
    /// against it and rejected: a rejected tail event still proves the gesture
    /// is running.
    #[inline]
    pub(super) fn keep_overscroll_peak_alive(&self, now: AnimInstant) {
        self.overscroll.keep_peak_alive(now);
    }

    /// Whether an input step may update an overscrolled axis at `now`.
    ///
    /// Steps that shorten the stretch always apply. An outward step applies
    /// only when it exceeds the episode's previous peak, which distinguishes
    /// renewed user intent from a decaying device-scroll tail during recovery.
    /// A peak whose episode went idle no longer gates anything, so a gesture
    /// boundary the platform failed to report cannot wedge the edge.
    pub(super) fn accepts_overscroll_step_at(
        &self,
        current: Vec2d,
        next: Vec2d,
        now: AnimInstant,
    ) -> bool {
        if !self.bouncy() {
            return true;
        }
        let Some(peak) = self.overscroll.live_peak_at(now) else {
            return true;
        };
        let from = self.overscroll_distance(current);
        let to = self.overscroll_distance(next);
        Self::axis_accepts_overscroll_step(from.x, to.x, peak.x)
            && Self::axis_accepts_overscroll_step(from.y, to.y, peak.y)
    }

    #[inline(always)]
    pub(super) fn axis_accepts_overscroll_step(from: f32, to: f32, peak: f32) -> bool {
        to.abs() <= from.abs() || to.signum() != peak.signum() || to.abs() > peak.abs()
    }

    /// Whether edge recovery is still held back at `now`: by contact, or by the
    /// initial grace period (see [`OverscrollState::held_at`]).
    pub(crate) fn overscroll_recovery_held_at(&self, now: AnimInstant) -> bool {
        self.overscroll.held_at(now)
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
    pub(super) fn outward_resistance(stretch: f32, delta: f32, limit: f32) -> f32 {
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
    pub(super) fn clamp_overscroll(&self, offset: Vec2d) -> Vec2d {
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
    pub(super) fn apply_bouncy(value: f32, min: f32, max: f32, dimension: f32, resistance: f32) -> f32 {
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

}
