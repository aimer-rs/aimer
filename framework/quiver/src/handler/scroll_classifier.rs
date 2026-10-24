use crate::handler::scroll_utils::MomentumScroller;
use aimer_attribute::Vec2d;
use aimer_utils::AnimInstant as Instant;
use std::collections::VecDeque;
use winit::dpi::PhysicalPosition;
use winit::event::TouchPhase;

/// Input-device behavior inferred from a stream of scroll deltas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollSource {
    /// A high-frequency, often two-axis pixel stream.
    Trackpad,
    /// A repeated pulse stream produced by a discrete wheel.
    Wheel,
    /// Too little history is available to decide safely.
    Unknown,
}

struct EventSample {
    delta: PhysicalPosition<f64>,
    at: Instant,
}

pub struct ScrollClassifier {
    history: VecDeque<EventSample>,
    window: usize,
    current: ScrollSource,
    reset_gap_ms: f64,
}

impl ScrollClassifier {
    pub fn new() -> Self {
        Self {
            history: VecDeque::with_capacity(8),
            window: 6,
            current: ScrollSource::Unknown,
            reset_gap_ms: 250.0,
        }
    }

    pub fn classify(&mut self, delta: PhysicalPosition<f64>) -> ScrollSource {
        let now = Instant::now();

        if let Some(last) = self.history.back()
            && now.duration_since(last.at).as_secs_f64() * 1000.0 > self.reset_gap_ms
        {
            self.history.clear();
            self.current = ScrollSource::Unknown;
        }

        self.history.push_back(EventSample { delta, at: now });
        if self.history.len() > self.window {
            self.history.pop_front();
        }

        if self.history.len() < 2 {
            return self.current;
        }

        let samples = self
            .history
            .iter()
            .map(|sample| sample.delta)
            .collect::<Vec<_>>();
        let total_gap_ms = self
            .history
            .iter()
            .zip(self.history.iter().skip(1))
            .map(|(previous, next)| next.at.duration_since(previous.at).as_secs_f64() * 1000.0)
            .sum::<f64>();
        let avg_gap_ms = total_gap_ms / (self.history.len() - 1) as f64;

        let classified = Self::classify_samples(&samples, avg_gap_ms);
        if classified != ScrollSource::Unknown {
            self.current = classified;
        }

        self.current
    }

    fn clear(&mut self) {
        self.history.clear();
        self.current = ScrollSource::Unknown;
    }

    /// Classifies a short pixel-delta history independently of platform event
    /// variants.
    ///
    /// Browsers expose both touchpads and many mouse wheels as `PixelDelta`.
    /// Touchpads are identified by fast cadence or two-axis variation, while a
    /// wheel is identified by sparse, repeated pulses. Ambiguous histories
    /// remain unknown rather than forcing a potentially jarring device switch.
    fn classify_samples(samples: &[PhysicalPosition<f64>], avg_gap_ms: f64) -> ScrollSource {
        if samples.len() < 2 {
            return ScrollSource::Unknown;
        }

        if samples.iter().any(|sample| sample.x.abs() > 0.01) {
            return ScrollSource::Trackpad;
        }

        let repeated = samples
            .iter()
            .filter(|sample| (sample.y - samples[0].y).abs() <= 0.01)
            .count();
        let repeat_ratio = repeated as f64 / samples.len() as f64;
        if (samples.len() >= 3 && repeat_ratio >= 0.8)
            || (repeat_ratio >= 0.5 && avg_gap_ms >= 25.0)
        {
            ScrollSource::Wheel
        } else if avg_gap_ms < 20.0 {
            ScrollSource::Trackpad
        } else {
            ScrollSource::Unknown
        }
    }
}

impl Default for ScrollClassifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Longest gap that still attributes an opening phase to inertia instead of a
/// new touch.
///
/// macOS reports the momentum that follows a lift as a second gesture start,
/// and it arrives within the same event burst as the lift itself. A human
/// cannot lift and touch down again that quickly, so a start seen this soon
/// after contact ended is inertia.
const MOMENTUM_BEGIN_GAP_MS: u128 = 32;

/// Tracks whether the user's fingers are currently on the pointing device.
///
/// Contact is a property of the *device*, not of a smoothing channel: the
/// classifier may attribute the opening deltas of one physical gesture to a
/// different channel than its later deltas, so a per-channel flag would report
/// a held gesture as released halfway through it. Widgets rely on this signal
/// to keep an overscrolled edge stretched exactly as long as the fingers stay
/// down, so it must never flicker within a gesture.
///
/// The platform vocabulary is ambiguous — [`TouchPhase::Started`] opens both a
/// touch and its post-lift inertia — and is disambiguated with two facts:
///
/// * inertia can only follow contact, never other inertia, and
/// * a trackpad announces a new touch twice (may-begin, then begin) while
///   inertia announces itself once.
#[derive(Debug, Default)]
struct ContactTracker {
    /// The fingers are on the device right now.
    contact: bool,
    /// The previous platform input opened a gesture.
    last_was_start: bool,
    /// When contact last ended, cleared once inertia or a new touch consumed
    /// it, so inertia is never inferred twice in a row.
    contact_ended_at: Option<Instant>,
}

impl ContactTracker {
    /// Folds one platform phase into the tracked contact state.
    fn on_input(&mut self, phase: TouchPhase) {
        match phase {
            TouchPhase::Started => {
                let opens_momentum = !self.last_was_start
                    && self.contact_ended_at.is_some_and(|at| {
                        Instant::now().duration_since(at).as_millis() <= MOMENTUM_BEGIN_GAP_MS
                    });
                self.contact = !opens_momentum;
                self.last_was_start = true;
                self.contact_ended_at = None;
            }
            TouchPhase::Moved => {
                // A phase-less stream (a plain mouse wheel) is never contact,
                // and a glide keeps whatever the opening phase established.
                self.last_was_start = false;
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.contact_ended_at = self.contact.then(Instant::now);
                self.contact = false;
                self.last_was_start = false;
            }
        }
    }

    /// Whether the fingers are on the device as of the last platform input.
    #[inline]
    fn is_direct_manipulation(&self) -> bool {
        self.contact
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Tracks the gesture phase of a single smoothing channel.
///
/// A platform delta and the frame that finally delivers it are not the same
/// event: the smoother spreads one input pulse over several frames, so the
/// phase reported by the window system cannot be forwarded verbatim. This
/// tracker converts the platform stream into a well-formed per-channel
/// sequence — exactly one [`TouchPhase::Started`], any number of
/// [`TouchPhase::Moved`], and exactly one terminating [`TouchPhase::Ended`]
/// or [`TouchPhase::Cancelled`].
///
/// Platforms that do report explicit phases (macOS trackpads) end the gesture
/// only after the window system said so *and* the queued distance is fully
/// delivered. Platforms that only ever report [`TouchPhase::Moved`] (a plain
/// mouse wheel on Windows or X11) would otherwise never end, so their gesture
/// ends when the channel drains.
#[derive(Debug, Default)]
struct ScrollPhaseTracker {
    /// A gesture is in flight, meaning `Started` has already been emitted.
    active: bool,
    /// The next emitted frame opens a gesture and must carry `Started`.
    start_pending: bool,
    /// The window system reported the end of the current gesture.
    input_ended: bool,
    /// The window system cancelled the current gesture.
    cancelled: bool,
    /// The window system reports explicit `Started`/`Ended` phases.
    platform_driven: bool,
    /// The gesture was closed on a boundary the window system never reported.
    forced_end: bool,
    /// Input arrived while a forced end was still owed to the receiver.
    deferred_start: bool,
}

impl ScrollPhaseTracker {
    /// Folds a platform phase into the tracked gesture state.
    fn on_input(&mut self, phase: TouchPhase) {
        // A forced end is owed to the receiver and must not be erased by the
        // gesture that follows it. Browsers open the next gesture in the same
        // breath as the previous one is closed, so the new input only records
        // that a start is due once the terminating frame has been delivered.
        if self.forced_end && matches!(phase, TouchPhase::Started | TouchPhase::Moved) {
            self.deferred_start = true;
            return;
        }

        match phase {
            TouchPhase::Started => {
                self.platform_driven = true;
                self.input_ended = false;
                self.cancelled = false;
                if !self.active {
                    self.start_pending = true;
                }
            }
            TouchPhase::Moved => {
                // Momentum deltas keep arriving as `Moved` after the platform
                // reported `Ended`, so they extend the current gesture instead
                // of terminating it early.
                self.input_ended = false;
                self.cancelled = false;
                if !self.active {
                    self.start_pending = true;
                }
            }
            TouchPhase::Ended => {
                self.platform_driven = true;
                self.input_ended = true;
            }
            TouchPhase::Cancelled => {
                self.platform_driven = true;
                self.cancelled = true;
            }
        }
    }

    /// Closes the gesture on a boundary the window system never reported.
    ///
    /// Unlike a [`TouchPhase::Ended`] fed through [`on_input`](Self::on_input),
    /// this end survives later input: it is the only end a phase-less platform
    /// will ever produce, so losing it would leave the gesture open forever.
    /// A tracker with nothing in flight is left untouched, so an end injected
    /// on an idle channel stays silent.
    #[inline]
    fn force_end(&mut self) {
        if !self.active && !self.start_pending {
            return;
        }
        self.forced_end = true;
    }

    /// Returns whether the gesture terminates on this frame.
    ///
    /// A forced end waits for the queued distance to drain, exactly like a
    /// platform-reported one — unless the next gesture is already waiting, in
    /// which case its distance would keep the channel draining forever and the
    /// end is emitted at once.
    #[inline]
    fn should_end(&self, still_draining: bool) -> bool {
        if self.forced_end {
            return self.deferred_start || !still_draining;
        }
        !still_draining && (self.input_ended || !self.platform_driven)
    }

    /// Rearms the tracker after a terminating frame was delivered.
    ///
    /// Input held back while the end was owed opens its gesture on the next
    /// emitted frame, so the receiver sees `Ended` followed by `Started`
    /// instead of two gestures merged into one.
    fn on_end_emitted(&mut self) {
        let deferred = self.deferred_start;
        self.reset();
        if deferred {
            self.start_pending = true;
            self.platform_driven = true;
        }
    }

    /// Returns whether a terminating frame is still owed to the receiver.
    #[inline]
    fn has_pending_end(&self) -> bool {
        self.cancelled || (self.active && self.should_end(false))
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Classifies and frame-synchronizes wheel input without changing its distance.
///
/// Aimer keeps this state at the window boundary because browsers commonly
/// report both trackpads and mouse wheels as `PixelDelta`. The two internal
/// channels use different response rates and remain separate when emitted so
/// widgets can retain device-appropriate momentum policy.
pub struct DualScroller {
    classifier: ScrollClassifier,
    trackpad: MomentumScroller,
    wheel: MomentumScroller,
    trackpad_phase: ScrollPhaseTracker,
    wheel_phase: ScrollPhaseTracker,
    contact: ContactTracker,
    pending_unknown: Vec<PhysicalPosition<f64>>,
    pending_phase: Option<TouchPhase>,
}

/// One frame-synchronized portion of a scroll gesture.
///
/// The delta is the distance to deliver on this frame and the phase describes
/// where the frame sits inside the gesture. A terminating frame may carry a
/// zero delta when the window system ends a gesture whose distance has already
/// been delivered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollStep {
    pub delta: PhysicalPosition<f64>,
    pub phase: TouchPhase,
    /// Whether the user's fingers are on the device as of this frame.
    ///
    /// This reports the state of the *device* (see [`ContactTracker`]), not of
    /// the channel that produced the step, so every step of a frame agrees and
    /// a held gesture never reads as released halfway through. Post-lift
    /// momentum keeps flowing with this set to false, letting widgets tell a
    /// held gesture from its inertial tail. Phase-less mouse wheels are never
    /// direct manipulations.
    pub is_direct_manipulation: bool,
}

/// Frame-synchronized deltas produced for each input-device behavior.
///
/// Trackpad and mouse-wheel output remains separate so callers can preserve
/// pixel-precise behavior for trackpads while preventing wheel-only momentum.
#[derive(Debug, Default)]
pub struct ScrollFrame {
    pub trackpad: Option<ScrollStep>,
    pub wheel: Option<ScrollStep>,
}

impl DualScroller {
    const BASE_PIXEL_LINE: f32 = 1.0;
    const MAX_VELO: f32 = 80.0;

    pub fn new() -> Self {
        let mut trackpad = MomentumScroller::new();
        trackpad.pixels_per_line = Self::BASE_PIXEL_LINE;
        trackpad.friction = 0.15;
        trackpad.max_velocity = Self::MAX_VELO;

        let mut wheel = MomentumScroller::new();
        wheel.pixels_per_line = 1.0;
        wheel.friction = 0.40;
        wheel.max_velocity = 60.0;

        Self {
            classifier: ScrollClassifier::new(),
            trackpad,
            wheel,
            trackpad_phase: ScrollPhaseTracker::default(),
            wheel_phase: ScrollPhaseTracker::default(),
            contact: ContactTracker::default(),
            pending_unknown: Vec::with_capacity(2),
            pending_phase: None,
        }
    }

    /// Queues a browser/native pixel delta after classifying its device shape.
    ///
    /// Ambiguous initial samples are held briefly instead of being emitted with
    /// the wrong behavior. Once cadence or axis variation identifies the
    /// device, all held distance is transferred to the matching smoother. The
    /// platform `phase` is folded into the receiving channel so the gesture is
    /// replayed to widgets with a faithful start and end.
    pub fn on_pixel_delta(&mut self, delta: PhysicalPosition<f64>, phase: TouchPhase) {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return;
        }

        // Contact is tracked before classification so an ambiguous opening
        // delta cannot hide the touch from the channel that ends up owning the
        // rest of the gesture.
        self.contact.on_input(phase);

        match self.classifier.classify(delta) {
            ScrollSource::Wheel => {
                self.flush_unknown(ScrollSource::Wheel);
                self.wheel_phase.on_input(phase);
                Self::queue(&mut self.wheel, delta);
            }
            ScrollSource::Trackpad => {
                self.flush_unknown(ScrollSource::Trackpad);
                self.trackpad_phase.on_input(phase);
                Self::queue(&mut self.trackpad, delta);
            }
            ScrollSource::Unknown => {
                self.pending_unknown.push(delta);
                self.pending_phase = Some(phase);
            }
        }
    }

    /// Queues a native `LineDelta`, whose source is already unambiguous.
    pub fn on_wheel_delta(&mut self, delta: PhysicalPosition<f64>, phase: TouchPhase) {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return;
        }
        self.contact.on_input(phase);
        self.wheel_phase.on_input(phase);
        Self::queue(&mut self.wheel, delta);
    }

    /// Closes the current gesture on every channel without queueing distance.
    ///
    /// Platforms whose wheel stream carries no phase — browsers report every
    /// event as [`TouchPhase::Moved`] — segment gestures themselves and then
    /// inject the boundary here. Feeding a zero-distance delta instead would
    /// work, but it would also feed a fake sample to the device classifier and
    /// bias the next gesture towards a wheel, so the phase is folded in on its
    /// own.
    ///
    /// The queued distance is kept: the terminating frame is emitted once it
    /// has been delivered, exactly like a platform-reported end. Contact is
    /// reset rather than merely ended, because an injected boundary is always
    /// followed by a genuinely new touch — never by the post-lift momentum a
    /// macOS trackpad announces as a second `Started`.
    ///
    /// The device history survives as well. A gesture boundary says nothing
    /// about which device produced it, and dropping the history would send the
    /// opening samples of the next gesture back through classification, where
    /// a lone pixel pulse is conservatively treated as a wheel — a trackpad
    /// would lose its momentum policy on every boundary. The classifier
    /// forgets a truly stale stream on its own after its reset gap.
    pub fn end_gesture(&mut self) {
        // A sample still waiting for classification belongs to the gesture
        // that is being closed, so it is committed first — otherwise the flush
        // would fold its phase in after the end and reopen the gesture.
        self.flush_unknown(ScrollSource::Wheel);
        self.contact.reset();
        self.trackpad_phase.force_end();
        self.wheel_phase.force_end();
    }

    /// Produces at most one step per source for the current rendered frame.
    pub fn tick(&mut self) -> ScrollFrame {
        if !self.pending_unknown.is_empty() {
            // One isolated pixel pulse cannot be classified by cadence. Treat
            // it conservatively as a wheel so it is never lost and never
            // creates trackpad-style release momentum.
            self.flush_unknown(ScrollSource::Wheel);
        }
        let direct = self.contact.is_direct_manipulation();
        ScrollFrame {
            trackpad: Self::tick_channel(&mut self.trackpad, &mut self.trackpad_phase, direct),
            wheel: Self::tick_channel(&mut self.wheel, &mut self.wheel_phase, direct),
        }
    }

    /// Emits the next step of one channel together with its gesture phase.
    ///
    /// A cancelled gesture drops the undelivered distance immediately, because
    /// the input it came from is no longer valid. Otherwise the phase follows
    /// the channel: the first emitted frame opens the gesture, the drain frame
    /// closes it, and a gesture ended after its distance was already delivered
    /// closes with a zero-distance frame so the receiver is never left hanging.
    fn tick_channel(
        scroller: &mut MomentumScroller,
        tracker: &mut ScrollPhaseTracker,
        is_direct_manipulation: bool,
    ) -> Option<ScrollStep> {
        if tracker.cancelled {
            scroller.clear();
            tracker.reset();
            return Some(ScrollStep {
                delta: PhysicalPosition::new(0.0, 0.0),
                phase: TouchPhase::Cancelled,
                is_direct_manipulation,
            });
        }

        match scroller.tick() {
            Some(delta) => {
                let phase = if std::mem::take(&mut tracker.start_pending) {
                    TouchPhase::Started
                } else if tracker.should_end(scroller.is_active()) {
                    TouchPhase::Ended
                } else {
                    TouchPhase::Moved
                };
                if phase == TouchPhase::Ended {
                    tracker.on_end_emitted();
                } else {
                    tracker.active = true;
                }
                Some(ScrollStep {
                    delta,
                    phase,
                    is_direct_manipulation,
                })
            }
            None => {
                if tracker.active && tracker.should_end(false) {
                    tracker.on_end_emitted();
                    Some(ScrollStep {
                        delta: PhysicalPosition::new(0.0, 0.0),
                        phase: TouchPhase::Ended,
                        is_direct_manipulation,
                    })
                } else {
                    None
                }
            }
        }
    }

    /// Returns whether another frame is still owed, either as distance or as a
    /// terminating phase.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.trackpad.is_active()
            || self.wheel.is_active()
            || self.trackpad_phase.has_pending_end()
            || self.wheel_phase.has_pending_end()
    }

    fn flush_unknown(&mut self, source: ScrollSource) {
        if self.pending_unknown.is_empty() {
            return;
        }
        let phase = self.pending_phase.take().unwrap_or(TouchPhase::Moved);
        match source {
            ScrollSource::Trackpad => self.trackpad_phase.on_input(phase),
            ScrollSource::Wheel => self.wheel_phase.on_input(phase),
            ScrollSource::Unknown => unreachable!("unknown samples cannot flush as unknown"),
        }
        for delta in self.pending_unknown.drain(..) {
            match source {
                ScrollSource::Trackpad => Self::queue(&mut self.trackpad, delta),
                ScrollSource::Wheel => Self::queue(&mut self.wheel, delta),
                ScrollSource::Unknown => unreachable!("unknown samples cannot flush as unknown"),
            }
        }
    }

    fn queue(scroller: &mut MomentumScroller, delta: PhysicalPosition<f64>) {
        scroller.on_line_delta(Vec2d {
            x: delta.x as f32,
            y: delta.y as f32,
        });
    }

    pub fn clear(&mut self) {
        self.classifier.clear();
        self.trackpad.clear();
        self.wheel.clear();
        self.trackpad_phase.reset();
        self.wheel_phase.reset();
        self.contact.reset();
        self.pending_unknown.clear();
        self.pending_phase = None;
    }
}

impl Default for DualScroller {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_trackpad_samples_are_classified_as_trackpad() {
        let samples = [
            PhysicalPosition::new(0.0, -2.0),
            PhysicalPosition::new(0.0, -2.0),
            PhysicalPosition::new(2.0, -4.0),
            PhysicalPosition::new(4.0, -4.0),
            PhysicalPosition::new(4.0, -6.0),
            PhysicalPosition::new(2.0, -2.0),
        ];

        assert_eq!(
            ScrollClassifier::classify_samples(&samples, 8.0),
            ScrollSource::Trackpad
        );
    }

    #[test]
    fn wasm_repeated_sparse_pixel_samples_are_classified_as_wheel() {
        let samples = [
            PhysicalPosition::new(0.0, -8.00048828125),
            PhysicalPosition::new(0.0, -8.00048828125),
            PhysicalPosition::new(0.0, -8.00048828125),
        ];

        assert_eq!(
            ScrollClassifier::classify_samples(&samples, 32.0),
            ScrollSource::Wheel
        );
    }

    #[test]
    fn repeated_wasm_wheel_pulses_stay_wheel_at_fast_cadence() {
        let samples = [
            PhysicalPosition::new(0.0, -8.00048828125),
            PhysicalPosition::new(0.0, -8.00048828125),
            PhysicalPosition::new(0.0, -8.00048828125),
        ];

        assert_eq!(
            ScrollClassifier::classify_samples(&samples, 8.0),
            ScrollSource::Wheel
        );
    }

    #[test]
    fn isolated_ambiguous_pixel_delta_is_emitted_without_trackpad_momentum() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(
            PhysicalPosition::new(0.0, -8.00048828125),
            TouchPhase::Moved,
        );

        let frame = scroller.tick();

        assert!(frame.trackpad.is_none());
        assert!(frame.wheel.is_some());
    }

    #[test]
    fn rapid_two_axis_pixel_stream_uses_trackpad_channel() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(PhysicalPosition::new(0.0, -2.0), TouchPhase::Moved);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -4.0), TouchPhase::Moved);

        let frame = scroller.tick();

        assert!(frame.trackpad.is_some());
        assert!(frame.wheel.is_none());
    }

    #[test]
    fn non_finite_pixel_delta_is_ignored() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(
            PhysicalPosition::new(f64::NAN, f64::INFINITY),
            TouchPhase::Moved,
        );

        let frame = scroller.tick();

        assert!(frame.trackpad.is_none());
        assert!(frame.wheel.is_none());
        assert!(!scroller.is_active());
    }

    fn drain_wheel_phases(scroller: &mut DualScroller) -> Vec<TouchPhase> {
        let mut phases = Vec::new();
        let mut frames = 0;
        loop {
            if let Some(step) = scroller.tick().wheel {
                phases.push(step.phase);
            }
            frames += 1;
            assert!(frames < 64, "scroll channel never terminated");
            if !scroller.is_active() {
                return phases;
            }
        }
    }

    #[test]
    fn wheel_gesture_reports_started_moved_and_ended_phases() {
        let mut scroller = DualScroller::new();
        scroller.on_wheel_delta(PhysicalPosition::new(0.0, -40.0), TouchPhase::Moved);

        let phases = drain_wheel_phases(&mut scroller);

        assert_eq!(phases.first(), Some(&TouchPhase::Started));
        assert_eq!(phases.last(), Some(&TouchPhase::Ended));
        assert!(phases.len() > 2);
        assert!(
            phases[1..phases.len() - 1]
                .iter()
                .all(|phase| *phase == TouchPhase::Moved)
        );
    }

    #[test]
    fn platform_reported_end_waits_for_the_queued_distance() {
        let mut scroller = DualScroller::new();
        scroller.on_wheel_delta(PhysicalPosition::new(0.0, -40.0), TouchPhase::Started);
        scroller.on_wheel_delta(PhysicalPosition::new(0.0, -40.0), TouchPhase::Ended);

        let phases = drain_wheel_phases(&mut scroller);

        assert_eq!(phases.first(), Some(&TouchPhase::Started));
        assert_eq!(phases.last(), Some(&TouchPhase::Ended));
        assert_eq!(
            phases
                .iter()
                .filter(|phase| **phase == TouchPhase::Ended)
                .count(),
            1
        );
    }

    #[test]
    fn platform_gesture_stays_open_until_the_platform_ends_it() {
        let mut scroller = DualScroller::new();
        scroller.on_wheel_delta(PhysicalPosition::new(0.0, -40.0), TouchPhase::Started);

        let mut phases = Vec::new();
        for _ in 0..32 {
            if let Some(step) = scroller.tick().wheel {
                phases.push(step.phase);
            }
        }

        // The distance is fully delivered, yet the gesture stays open because
        // the window system has not lifted the fingers.
        assert_eq!(phases.first(), Some(&TouchPhase::Started));
        assert!(!phases.contains(&TouchPhase::Ended));
        assert!(!scroller.is_active());

        scroller.on_wheel_delta(PhysicalPosition::new(0.0, 0.0), TouchPhase::Ended);
        assert!(scroller.is_active());
        let tail = drain_wheel_phases(&mut scroller);

        assert_eq!(tail, vec![TouchPhase::Ended]);
        assert!(!scroller.is_active());
    }

    #[test]
    fn direct_manipulation_survives_late_trackpad_classification() {
        let mut scroller = DualScroller::new();
        // A single pure-vertical pixel pulse cannot be attributed to a device,
        // so the frame flushes it to the wheel channel. The trackpad channel
        // therefore never receives the platform `Started` of this gesture.
        scroller.on_pixel_delta(PhysicalPosition::new(0.0, -12.0), TouchPhase::Started);
        let flushed = scroller.tick();
        assert!(flushed.trackpad.is_none());
        assert!(
            flushed
                .wheel
                .expect("the ambiguous pulse must not be lost")
                .is_direct_manipulation
        );

        // Cross-axis variation now identifies the trackpad mid-gesture.
        scroller.on_pixel_delta(PhysicalPosition::new(1.5, -12.0), TouchPhase::Moved);
        scroller.on_pixel_delta(PhysicalPosition::new(1.5, -12.0), TouchPhase::Moved);

        let step = scroller
            .tick()
            .trackpad
            .expect("trackpad distance must be delivered");
        assert!(
            step.is_direct_manipulation,
            "contact belongs to the device, not to a smoothing channel"
        );
    }

    #[test]
    fn a_drained_channel_end_does_not_report_the_fingers_as_lifted() {
        let mut scroller = DualScroller::new();
        // Identify the trackpad first so the live gesture below is routed to its
        // own channel instead of being held as an ambiguous pulse.
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Moved);
        assert!(scroller.tick().trackpad.is_some());
        // A phase-less line pulse ends when its channel drains, which must not
        // be mistaken for the end of the live trackpad contact.
        scroller.on_wheel_delta(PhysicalPosition::new(0.0, -2.0), TouchPhase::Moved);

        let mut ended = None;
        for _ in 0..64 {
            if let Some(step) = scroller.tick().wheel
                && step.phase == TouchPhase::Ended
            {
                ended = Some(step);
                break;
            }
        }

        let ended = ended.expect("a phase-less wheel gesture ends when it drains");
        assert!(ended.is_direct_manipulation);
    }

    #[test]
    fn a_fresh_touch_pair_regains_contact_right_after_a_gesture() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Moved);
        assert!(
            scroller
                .tick()
                .trackpad
                .expect("contact distance must be delivered")
                .is_direct_manipulation
        );

        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -8.0), TouchPhase::Ended);
        // A trackpad announces a new touch twice (may-begin, then begin), which
        // momentum never does — so this is contact even though it follows the
        // previous gesture immediately.
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -6.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -6.0), TouchPhase::Started);

        let step = scroller
            .tick()
            .trackpad
            .expect("the new gesture must be delivered");
        assert!(step.is_direct_manipulation);
    }

    #[test]
    fn a_touch_after_momentum_finished_is_contact_again() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Ended);
        // Post-lift momentum: begins, glides, then finishes.
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -20.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -10.0), TouchPhase::Moved);
        scroller.on_pixel_delta(PhysicalPosition::new(0.0, 0.0), TouchPhase::Ended);

        // Momentum cannot follow momentum, so the next opening phase is a touch.
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -30.0), TouchPhase::Started);

        let step = scroller
            .tick()
            .trackpad
            .expect("the new gesture must be delivered");
        assert!(step.is_direct_manipulation);
    }

    #[test]
    fn direct_manipulation_ends_before_post_lift_momentum() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Moved);

        let held_step = scroller
            .tick()
            .trackpad
            .expect("direct trackpad distance must be delivered");
        assert!(held_step.is_direct_manipulation);

        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -8.0), TouchPhase::Ended);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -4.0), TouchPhase::Started);

        let momentum_step = scroller
            .tick()
            .trackpad
            .expect("post-lift momentum must still be delivered");
        assert!(!momentum_step.is_direct_manipulation);
    }

    #[test]
    fn cancelled_gesture_drops_pending_distance_and_reports_cancel() {
        let mut scroller = DualScroller::new();
        scroller.on_wheel_delta(PhysicalPosition::new(0.0, -400.0), TouchPhase::Started);
        assert!(scroller.tick().wheel.is_some());

        scroller.on_wheel_delta(PhysicalPosition::new(0.0, 0.0), TouchPhase::Cancelled);
        let step = scroller.tick().wheel.expect("cancel must be delivered");

        assert_eq!(step.phase, TouchPhase::Cancelled);
        assert_eq!(step.delta.y, 0.0);
        assert!(!scroller.is_active());
        assert!(scroller.tick().wheel.is_none());
    }

    #[test]
    fn an_injected_end_closes_a_gesture_without_queueing_distance() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Moved);
        assert!(scroller.tick().trackpad.is_some());

        scroller.end_gesture();

        let mut phases = Vec::new();
        for _ in 0..64 {
            if let Some(step) = scroller.tick().trackpad {
                phases.push(step.phase);
            }
            if !scroller.is_active() {
                break;
            }
        }

        assert_eq!(phases.last(), Some(&TouchPhase::Ended));
        assert!(!scroller.is_active());
    }

    #[test]
    fn an_injected_start_after_an_injected_end_is_a_fresh_contact() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Moved);
        assert!(scroller.tick().trackpad.is_some());

        // A browser gesture is segmented by cadence, so the end and the start
        // of the next gesture are injected back to back. That pair is a new
        // touch, never the momentum tail a trackpad reports on macOS.
        scroller.end_gesture();
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Moved);

        let step = scroller
            .tick()
            .trackpad
            .expect("the new gesture must be delivered");
        assert!(step.is_direct_manipulation);
    }

    /// Collects the phase of every step both channels still owe.
    fn drain_phases(scroller: &mut DualScroller) -> Vec<TouchPhase> {
        let mut phases = Vec::new();
        for _ in 0..256 {
            let frame = scroller.tick();
            phases.extend(frame.trackpad.map(|step| step.phase));
            phases.extend(frame.wheel.map(|step| step.phase));
            if !scroller.is_active() {
                break;
            }
        }
        phases
    }

    #[test]
    fn an_injected_end_is_reported_even_when_the_next_gesture_opens_at_once() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Moved);
        assert!(scroller.tick().trackpad.is_some());

        // A browser reports no gesture boundary, so the end is injected and the
        // next gesture opens in the same breath — before a single frame was
        // rendered. The terminating frame is still owed to the widget.
        scroller.end_gesture();
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -40.0), TouchPhase::Started);

        let phases = drain_phases(&mut scroller);
        let ended = phases
            .iter()
            .position(|phase| *phase == TouchPhase::Ended)
            .expect("the injected end must reach the widget");
        assert_eq!(
            phases.get(ended + 1),
            Some(&TouchPhase::Started),
            "the deferred gesture opens on the frame after the end"
        );
        assert_eq!(
            phases
                .iter()
                .filter(|phase| **phase == TouchPhase::Ended)
                .count(),
            1,
            "the deferred gesture stays open until its own boundary is injected"
        );

        scroller.end_gesture();
        assert_eq!(drain_phases(&mut scroller).last(), Some(&TouchPhase::Ended));
        assert!(!scroller.is_active());
    }

    #[test]
    fn an_injected_end_is_reported_for_a_sample_that_is_still_unclassified() {
        let mut scroller = DualScroller::new();
        // A single pixel pulse cannot be classified yet, so it waits for the
        // next frame — a cursor move can inject the end before that frame.
        scroller.on_pixel_delta(PhysicalPosition::new(0.0, -40.0), TouchPhase::Started);

        scroller.end_gesture();

        let phases = drain_phases(&mut scroller);
        assert_eq!(phases.first(), Some(&TouchPhase::Started));
        assert_eq!(phases.last(), Some(&TouchPhase::Ended));
        assert!(!scroller.is_active());
    }

    #[test]
    fn an_injected_end_on_an_idle_scroller_stays_idle() {
        let mut scroller = DualScroller::new();

        scroller.end_gesture();

        assert!(!scroller.is_active());
        assert!(scroller.tick().trackpad.is_none());
        assert!(scroller.tick().wheel.is_none());
    }

    #[test]
    fn trackpad_and_wheel_channels_track_their_phases_independently() {
        let mut scroller = DualScroller::new();
        scroller.on_pixel_delta(PhysicalPosition::new(2.0, -4.0), TouchPhase::Started);
        scroller.on_pixel_delta(PhysicalPosition::new(4.0, -4.0), TouchPhase::Started);
        scroller.on_wheel_delta(PhysicalPosition::new(0.0, -40.0), TouchPhase::Moved);

        let frame = scroller.tick();

        assert_eq!(
            frame.trackpad.map(|step| step.phase),
            Some(TouchPhase::Started)
        );
        assert_eq!(
            frame.wheel.map(|step| step.phase),
            Some(TouchPhase::Started)
        );
    }
}
