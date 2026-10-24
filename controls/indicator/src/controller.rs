//! The retained state of an indicator.

use std::cell::RefCell;
use std::rc::Rc;

use std::time::Duration;

use aimer_animation::{AnimInstant, AnyMotion, Motion, MotionState, motions};

use crate::track::IndicatorTrack;

/// A snapshot of where an indicator is, as the layers present it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndicatorSample {
    /// The animated slot position: `0.0` is the first slot. It can pass its
    /// target while a spring settles.
    pub position: f32,
    /// How lifted the indicator is: `0.0` at rest, `1.0` fully lifted. It can
    /// pass `1.0` while a spring settles.
    pub lift: f32,
    /// How visible the indicator is: `1.0` shown, `0.0` hidden, in between while
    /// it fades. It multiplies the opacity of every layer.
    pub presence: f32,
}

/// Drives a [`SlidingIndicator`](crate::SlidingIndicator).
///
/// The controller owns the indicator's motion: where it is, where it is going,
/// and how lifted it is. Hold one across rebuilds, as with an
/// [`AnimationController`](aimer_animation::AnimationController), hand it to the
/// indicator, and retarget it with [`select`](Self::select) when the selection
/// changes. A retarget in mid-flight continues from the current speed, so
/// quickly tapping through the slots never jerks.
///
/// The indicator *lifts* while it moves or is held down, and settles back as soon
/// as it arrives and is released. It does not wait for the travel spring to stop
/// ringing, so a bouncy spring gives a lens that is already flat as it overshoots. What lifting looks like is up to the
/// [`IndicatorStyle`](crate::IndicatorStyle).
///
/// The controller is a cheap handle: clones share one state.
///
/// ```
/// use aimer_indicator::IndicatorController;
///
/// let controller = IndicatorController::new(0.0);
/// controller.select(2.0);
/// assert_eq!(controller.target(), 2.0);
/// // Nothing has moved until a frame is drawn.
/// assert_eq!(controller.position(), 0.0);
/// ```
#[derive(Clone)]
pub struct IndicatorController {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    position: MotionState,
    target: f32,
    lift: MotionState,
    pressed: bool,
    /// The grab of a drag in progress.
    drag: Option<Drag>,
    /// Whether the indicator has come within [`ARRIVAL`] of its target since it
    /// was last pointed somewhere new. It stays set while the spring rings out,
    /// so an overshoot does not lift the indicator again.
    arrived: bool,
    /// Whether the indicator is meant to be hidden, and how visible it is now.
    hidden: bool,
    presence: MotionState,
    reduced_motion: bool,
    motion: AnyMotion,
    lift_motion: AnyMotion,
    /// When the state was last advanced. `None` while idle, so that the first
    /// frame after waking advances by nothing instead of by the whole time spent
    /// idle, which would snap the motion to its end.
    last_tick: Option<AnimInstant>,
}

/// How close the indicator has to be to its target to count as having arrived,
/// in slots. The indicator lets go of its lift then, not when the travel spring
/// has finished ringing, which takes several times longer and is far too small
/// a movement to see.
const ARRIVAL: f32 = 0.1;

/// Where a drag began: the slot that was grabbed and the pointer's place on it.
#[derive(Clone, Copy)]
struct Drag {
    origin: f32,
    anchor: (f32, f32),
}

impl Inner {
    fn lift_target(&self) -> f32 {
        if (!self.arrived || self.pressed) && !self.reduced_motion { 1.0 } else { 0.0 }
    }

    /// Runs `change`, and restarts the clock if it woke an idle indicator.
    ///
    /// The clock is only restarted from idle: a drag retargets on every pointer
    /// event, and restarting it each time would leave the motion no time to
    /// advance between them.
    fn waking(&mut self, change: impl FnOnce(&mut Self)) {
        let was_active = self.is_active();
        change(self);
        if !was_active {
            self.last_tick = None;
        }
    }

    /// Points the motion at `target`, jumping there when motion is reduced.
    fn retarget(&mut self, target: f32) {
        self.target = target;
        // Decided now, not on the next frame: pointed at where it already is, the
        // indicator may look idle and never get that frame.
        self.arrived = (self.position.value - target).abs() <= ARRIVAL;
        if self.reduced_motion {
            self.position = MotionState::at_rest(target);
        }
    }

    fn presence_target(&self) -> f32 {
        if self.hidden { 0.0 } else { 1.0 }
    }

    fn is_active(&self) -> bool {
        !self.motion.is_settled(self.position, self.target)
            || !self.lift_motion.is_settled(self.lift, self.lift_target())
            || !self.lift_motion.is_settled(self.presence, self.presence_target())
    }

    fn sample(&self) -> IndicatorSample {
        IndicatorSample {
            position: self.position.value,
            lift: self.lift.value,
            presence: self.presence.value,
        }
    }

    /// Advances both motions by `dt` seconds.
    fn advance(&mut self, dt: f32) {
        self.position = self.motion.step(self.position, self.target, dt);
        if self.motion.is_settled(self.position, self.target) {
            // Arriving is exact, so that an idle indicator sits on its slot and
            // not within a thousandth of it.
            self.position = MotionState::at_rest(self.target);
        }
        self.arrived |= (self.position.value - self.target).abs() <= ARRIVAL;
        let lift_target = self.lift_target();
        self.lift = self.lift_motion.step(self.lift, lift_target, dt);
        if self.lift_motion.is_settled(self.lift, lift_target) {
            self.lift = MotionState::at_rest(lift_target);
        }
        let presence_target = self.presence_target();
        self.presence = self.lift_motion.step(self.presence, presence_target, dt);
        if self.lift_motion.is_settled(self.presence, presence_target) {
            self.presence = MotionState::at_rest(presence_target);
        }
    }
}

impl IndicatorController {
    /// Creates a controller resting on `position`, using a bouncy spring to
    /// travel and a snappy one to lift.
    pub fn new(position: f32) -> Self {
        let position = if position.is_finite() { position } else { 0.0 };
        Self {
            inner: Rc::new(RefCell::new(Inner {
                position: MotionState::at_rest(position),
                target: position,
                lift: MotionState::at_rest(0.0),
                pressed: false,
                drag: None,
                arrived: true,
                hidden: false,
                presence: MotionState::at_rest(1.0),
                reduced_motion: false,
                motion: motions::BOUNCY.into_any(),
                lift_motion: motions::SNAPPY.into_any(),
                last_tick: None,
            })),
        }
    }

    /// Sets how the indicator travels between slots.
    #[must_use]
    pub fn motion(self, motion: impl Motion) -> Self {
        self.inner.borrow_mut().motion = motion.into_any();
        self
    }

    /// Sets how the indicator lifts and settles.
    #[must_use]
    pub fn lift_motion(self, motion: impl Motion) -> Self {
        self.inner.borrow_mut().lift_motion = motion.into_any();
        self
    }

    /// Sets how long the indicator takes to lift and to settle back: a lift of
    /// one whole unit, from flat to fully lifted or the other way, takes
    /// `duration`, at a steady pace.
    ///
    /// This is [`lift_motion`](Self::lift_motion) with a constant-speed ramp,
    /// for when the exact time matters more than the feel of a spring. A zero
    /// duration makes the change instant.
    ///
    /// The lens lets go as the indicator arrives, so this is also how long the
    /// lens takes to flatten after that.
    ///
    /// ```
    /// use std::time::Duration;
    /// use aimer_indicator::IndicatorController;
    ///
    /// // A lens that forms and flattens in a tenth of a second.
    /// let controller = IndicatorController::new(0.0).fade_duration(Duration::from_millis(100));
    /// ```
    #[must_use]
    pub fn fade_duration(self, duration: Duration) -> Self {
        self.lift_motion(motions::ramp(duration))
    }

    /// Makes the indicator jump to its target and never lift, for a user who
    /// asked the system to reduce motion.
    #[must_use]
    pub fn reduced_motion(self, reduced: bool) -> Self {
        self.inner.borrow_mut().reduced_motion = reduced;
        self
    }

    /// Where the indicator is now, as of the last frame.
    pub fn position(&self) -> f32 {
        self.inner.borrow().position.value
    }

    /// Where the indicator is going.
    pub fn target(&self) -> f32 {
        self.inner.borrow().target
    }

    /// How lifted the indicator is now, as of the last frame.
    pub fn lift(&self) -> f32 {
        self.inner.borrow().lift.value
    }

    /// Whether the indicator still needs frames: it is travelling, or lifting or
    /// settling.
    pub fn is_animating(&self) -> bool {
        self.inner.borrow().is_active()
    }

    /// Sends the indicator to `position`, lifting while it travels.
    ///
    /// A non-finite position is ignored, and so is any position while a drag is
    /// in progress. With reduced motion the indicator jumps instead. Asks for a
    /// frame so that the move is seen.
    pub fn select(&self, position: f32) {
        if !position.is_finite() {
            return;
        }
        {
            let mut inner = self.inner.borrow_mut();
            // A drag owns the target until it ends.
            if inner.target == position || inner.drag.is_some() {
                return;
            }
            inner.waking(|inner| inner.retarget(position));
        }
        aimer_events::window::request_animation_frame();
    }

    /// Puts the indicator on `position` at once, without lifting.
    pub fn jump_to(&self, position: f32) {
        if !position.is_finite() {
            return;
        }
        {
            let mut inner = self.inner.borrow_mut();
            inner.target = position;
            inner.position = MotionState::at_rest(position);
            inner.arrived = true;
        }
        aimer_events::window::request_animation_frame();
    }

    /// Starts a drag with the pointer at `pointer` over the slot at `position`.
    ///
    /// The indicator is held lifted and heads for `position`, so pressing an
    /// item that is not selected brings the indicator under the pointer. Does
    /// nothing if either is not finite.
    pub fn begin_drag(&self, position: f32, pointer: (f32, f32)) {
        if !position.is_finite() || !pointer.0.is_finite() || !pointer.1.is_finite() {
            return;
        }
        {
            let mut inner = self.inner.borrow_mut();
            inner.waking(|inner| {
                inner.drag = Some(Drag { origin: position, anchor: pointer });
                inner.pressed = true;
                // Pressed, the indicator comes under the pointer even if it was
                // hidden: there is something to choose.
                inner.hidden = false;
                if inner.reduced_motion {
                    inner.presence = MotionState::at_rest(1.0);
                }
                inner.retarget(position);
            });
        }
        aimer_events::window::request_animation_frame();
    }

    /// Moves a drag begun by [`begin_drag`](Self::begin_drag) to `pointer`.
    ///
    /// The indicator follows by the distance the pointer has moved, as the
    /// `track` reads it, and is kept within the track's slots. The motion still
    /// chases the pointer, so the indicator trails it a little. Does nothing
    /// without a drag, or if `pointer` is not finite.
    pub fn drag_to(&self, pointer: (f32, f32), track: &impl IndicatorTrack) {
        let Some(last_slot) = track.slot_count().checked_sub(1) else {
            return;
        };
        {
            let mut inner = self.inner.borrow_mut();
            let Some(drag) = inner.drag else {
                return;
            };
            let moved = track.shifted(drag.origin, (pointer.0 - drag.anchor.0, pointer.1 - drag.anchor.1));
            if !moved.is_finite() {
                return;
            }
            let target = moved.clamp(0.0, last_slot as f32);
            inner.waking(|inner| inner.retarget(target));
        }
        aimer_events::window::request_animation_frame();
    }

    /// Ends a drag: the indicator is let go and settles on the slot nearest to
    /// where it was dragged, which is returned.
    ///
    /// Without a drag this only reports the slot the indicator is heading for.
    pub fn end_drag(&self, track: &impl IndicatorTrack) -> usize {
        let last_slot = track.slot_count().saturating_sub(1);
        let mut inner = self.inner.borrow_mut();
        let nearest = (inner.target.round().max(0.0) as usize).min(last_slot);
        if inner.drag.is_some() {
            inner.waking(|inner| {
                inner.drag = None;
                inner.pressed = false;
                inner.retarget(nearest as f32);
            });
            drop(inner);
            aimer_events::window::request_animation_frame();
        }
        nearest
    }

    /// Hides the indicator, or shows it again: it fades out or in with the lift
    /// motion, and stays where it was. For a bar with nothing selected.
    ///
    /// Ignored while a drag is in progress, and a drag that begins shows the
    /// indicator, so that it is under the pointer.
    pub fn set_hidden(&self, hidden: bool) {
        {
            let mut inner = self.inner.borrow_mut();
            if inner.hidden == hidden || (hidden && inner.drag.is_some()) {
                return;
            }
            inner.waking(|inner| {
                inner.hidden = hidden;
                if inner.reduced_motion {
                    inner.presence = MotionState::at_rest(inner.presence_target());
                }
            });
        }
        aimer_events::window::request_animation_frame();
    }

    /// Whether the indicator is hidden or fading out.
    pub fn is_hidden(&self) -> bool {
        self.inner.borrow().hidden
    }

    /// How visible the indicator is now, as of the last frame.
    pub fn presence(&self) -> f32 {
        self.inner.borrow().presence.value
    }

    /// Whether a drag is in progress.
    pub fn is_dragging(&self) -> bool {
        self.inner.borrow().drag.is_some()
    }

    /// Holds the indicator lifted while a pointer is down on it, and lets it
    /// settle again when released.
    pub fn set_pressed(&self, pressed: bool) {
        {
            let mut inner = self.inner.borrow_mut();
            // The gesture system reports a press as cancelled the moment it
            // turns into a drag; the drag is what holds the indicator then.
            if inner.pressed == pressed || (!pressed && inner.drag.is_some()) {
                return;
            }
            inner.waking(|inner| inner.pressed = pressed);
        }
        aimer_events::window::request_animation_frame();
    }

    /// Advances the motion to `now` and returns where the indicator is.
    ///
    /// The layers call this once each per frame. The motions are frame-rate
    /// independent, so a second call a moment later in the same frame is
    /// harmless.
    pub fn tick(&self, now: AnimInstant) -> IndicatorSample {
        let mut inner = self.inner.borrow_mut();
        if inner.is_active() {
            let dt = inner
                .last_tick
                .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
            inner.advance(dt);
            inner.last_tick = inner.is_active().then_some(now);
        } else {
            inner.last_tick = None;
        }
        inner.sample()
    }

    /// Advances the motion by `seconds`, for a driver that keeps its own clock.
    #[cfg(test)]
    fn advance(&self, seconds: f32) -> IndicatorSample {
        let mut inner = self.inner.borrow_mut();
        inner.advance(seconds);
        inner.sample()
    }
}

impl Default for IndicatorController {
    /// A controller resting on the first slot.
    fn default() -> Self {
        Self::new(0.0)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const FRAME: f32 = 1.0 / 120.0;

    fn run(controller: &IndicatorController, seconds: f32) -> Vec<IndicatorSample> {
        (0..(seconds / FRAME).round() as usize)
            .map(|_| controller.advance(FRAME))
            .collect()
    }

    #[test]
    fn a_new_controller_rests_and_needs_no_frames() {
        let controller = IndicatorController::new(1.0);
        assert_eq!(controller.position(), 1.0);
        assert_eq!(controller.target(), 1.0);
        assert_eq!(controller.lift(), 0.0);
        assert!(!controller.is_animating());
    }

    #[test]
    fn selecting_a_slot_starts_a_move_that_arrives_exactly_and_stops() {
        let controller = IndicatorController::new(0.0);
        controller.select(2.0);
        assert!(controller.is_animating());
        let trace = run(&controller, 3.0);
        let last = *trace.last().unwrap();
        assert_eq!(last, IndicatorSample { position: 2.0, lift: 0.0, presence: 1.0 });
        assert!(!controller.is_animating());
    }

    #[test]
    fn the_indicator_lifts_while_it_travels_and_settles_back_down() {
        let controller = IndicatorController::new(0.0);
        controller.select(2.0);
        let trace = run(&controller, 3.0);
        let peak = trace.iter().map(|sample| sample.lift).fold(f32::MIN, f32::max);
        assert!(peak > 0.9, "the lens never lifted: peak {peak}");
        // Lifted at the start of the journey, flat again at the end.
        assert!(trace[trace.len() / 20].lift > 0.5);
        assert_eq!(trace.last().unwrap().lift, 0.0);
    }

    #[test]
    fn a_bouncy_move_passes_its_target_before_settling() {
        let controller = IndicatorController::new(0.0);
        controller.select(2.0);
        let peak = run(&controller, 3.0).iter().map(|s| s.position).fold(f32::MIN, f32::max);
        assert!(peak > 2.0 && peak < 2.3, "peak {peak}");
    }

    #[test]
    fn retargeting_mid_flight_continues_instead_of_jumping() {
        let controller = IndicatorController::new(0.0);
        controller.select(2.0);
        let before = run(&controller, 0.15).pop().unwrap();
        controller.select(0.0);
        let after = controller.advance(FRAME);
        // One frame later the position moved by about a frame of its old speed.
        assert!((after.position - before.position).abs() < 0.2, "{before:?} -> {after:?}");
        assert!(after.position > 0.0 && after.position < 2.0);
    }

    #[test]
    fn selecting_the_current_target_changes_nothing() {
        let controller = IndicatorController::new(1.0);
        controller.select(1.0);
        assert!(!controller.is_animating());
    }

    #[test]
    fn a_non_finite_selection_is_ignored() {
        let controller = IndicatorController::new(1.0);
        controller.select(f32::NAN);
        controller.select(f32::INFINITY);
        controller.jump_to(f32::NAN);
        assert_eq!(controller.target(), 1.0);
        assert!(!IndicatorController::new(f32::NAN).is_animating());
    }

    #[test]
    fn jumping_moves_at_once_without_lifting() {
        let controller = IndicatorController::new(0.0);
        controller.jump_to(2.0);
        assert_eq!(controller.position(), 2.0);
        assert!(!controller.is_animating());
        assert_eq!(controller.lift(), 0.0);
    }

    #[test]
    fn holding_the_indicator_down_keeps_it_lifted_until_release() {
        let controller = IndicatorController::new(1.0);
        controller.set_pressed(true);
        assert!(controller.is_animating());
        let held = run(&controller, 1.0).pop().unwrap();
        assert_eq!(held, IndicatorSample { position: 1.0, lift: 1.0, presence: 1.0 });
        // Fully lifted and still held: nothing is changing, so no frames.
        assert!(!controller.is_animating());
        controller.set_pressed(false);
        assert!(controller.is_animating());
        assert_eq!(run(&controller, 1.0).pop().unwrap().lift, 0.0);
    }

    #[test]
    fn reduced_motion_jumps_and_never_lifts() {
        let controller = IndicatorController::new(0.0).reduced_motion(true);
        controller.select(2.0);
        assert_eq!(controller.position(), 2.0);
        controller.set_pressed(true);
        let sample = run(&controller, 0.5).pop().unwrap();
        assert_eq!(sample, IndicatorSample { position: 2.0, lift: 0.0, presence: 1.0 });
    }

    #[test]
    fn a_custom_motion_replaces_the_default() {
        struct Jump;
        impl Motion for Jump {
            fn step(&self, _: MotionState, target: f32, _: f32) -> MotionState {
                MotionState::at_rest(target)
            }
        }
        let controller = IndicatorController::new(0.0).motion(Jump);
        controller.select(3.0);
        assert_eq!(controller.advance(FRAME).position, 3.0);
    }

    #[test]
    fn the_first_frame_after_waking_does_not_consume_the_idle_time() {
        let controller = IndicatorController::new(0.0);
        let start = AnimInstant::now();
        // Idle for a long time, then woken and ticked once.
        controller.select(2.0);
        let first = controller.tick(start + Duration::from_secs(30));
        assert_eq!(first.position, 0.0, "the idle time was spent in one step");
        let second = controller.tick(start + Duration::from_secs(30) + Duration::from_millis(16));
        assert!(second.position > 0.0 && second.position < 1.0, "{second:?}");
    }

    fn row() -> crate::track::Linear {
        crate::track::tracks::row(3, 100.0, 56.0)
    }

    #[test]
    fn a_drag_follows_the_pointer_along_the_track_and_stays_lifted() {
        let controller = IndicatorController::new(0.0);
        let track = row();
        controller.begin_drag(0.0, (50.0, 20.0));
        assert!(controller.is_dragging());

        controller.drag_to((150.0, 22.0), &track);
        assert_eq!(controller.target(), 1.0);
        controller.drag_to((200.0, 22.0), &track);
        assert_eq!(controller.target(), 1.5);

        // The indicator is held lifted for as long as the drag lasts.
        let held = run(&controller, 1.0).pop().unwrap();
        assert_eq!(held.lift, 1.0);
        assert!((held.position - 1.5).abs() < 1.0e-3, "{held:?}");
    }

    #[test]
    fn a_drag_is_kept_within_the_slots_of_the_track() {
        let controller = IndicatorController::new(1.0);
        let track = row();
        controller.begin_drag(1.0, (150.0, 20.0));
        controller.drag_to((900.0, 20.0), &track);
        assert_eq!(controller.target(), 2.0);
        controller.drag_to((-900.0, 20.0), &track);
        assert_eq!(controller.target(), 0.0);
    }

    #[test]
    fn pressing_an_unselected_item_brings_the_indicator_under_the_pointer() {
        let controller = IndicatorController::new(0.0);
        controller.begin_drag(2.0, (250.0, 20.0));
        assert_eq!(controller.target(), 2.0);
        // Dragging back by a slot is measured from the pressed item.
        controller.drag_to((150.0, 20.0), &row());
        assert_eq!(controller.target(), 1.0);
    }

    #[test]
    fn letting_go_settles_on_the_nearest_slot_and_reports_it() {
        let track = row();
        for (pointer_x, expected) in [(50.0, 0), (84.0, 0), (116.0, 1), (210.0, 2), (900.0, 2)] {
            let controller = IndicatorController::new(0.0);
            controller.begin_drag(0.0, (50.0, 20.0));
            controller.drag_to((pointer_x, 20.0), &track);
            assert_eq!(controller.end_drag(&track), expected, "pointer at {pointer_x}");
            assert!(!controller.is_dragging());
            assert_eq!(controller.target(), expected as f32);
        }
    }

    #[test]
    fn after_letting_go_the_indicator_glides_in_and_drops_back_down() {
        let controller = IndicatorController::new(0.0);
        let track = row();
        controller.begin_drag(0.0, (50.0, 20.0));
        controller.drag_to((180.0, 20.0), &track);
        run(&controller, 0.5);
        assert_eq!(controller.end_drag(&track), 1);
        assert_eq!(run(&controller, 3.0).pop().unwrap(), IndicatorSample { position: 1.0, lift: 0.0, presence: 1.0 });
    }

    #[test]
    fn a_release_that_does_not_end_the_press_cannot_drop_a_drag() {
        let controller = IndicatorController::new(0.0);
        controller.begin_drag(0.0, (50.0, 20.0));
        // The gesture system reports the press as cancelled once it becomes a
        // drag; that must not let the indicator settle mid-drag.
        controller.set_pressed(false);
        assert_eq!(run(&controller, 0.5).pop().unwrap().lift, 1.0);
    }

    #[test]
    fn a_selection_cannot_pull_the_indicator_out_of_a_drag() {
        let controller = IndicatorController::new(0.0);
        controller.begin_drag(0.0, (50.0, 20.0));
        controller.select(2.0);
        assert_eq!(controller.target(), 0.0);
    }

    #[test]
    fn drag_moves_without_a_drag_or_with_bad_numbers_change_nothing() {
        let controller = IndicatorController::new(1.0);
        let track = row();
        controller.drag_to((500.0, 0.0), &track);
        assert_eq!(controller.target(), 1.0);
        assert_eq!(controller.end_drag(&track), 1);

        controller.begin_drag(1.0, (150.0, 20.0));
        controller.drag_to((f32::NAN, 20.0), &track);
        assert_eq!(controller.target(), 1.0);
        controller.begin_drag(f32::NAN, (150.0, 20.0));
        assert_eq!(controller.target(), 1.0);
    }

    #[test]
    fn retargeting_every_frame_does_not_stall_the_motion() {
        // A drag retargets on every pointer event. If each retarget restarted the
        // clock the motion would never advance between them.
        let controller = IndicatorController::new(0.0);
        let start = AnimInstant::now();
        controller.select(2.0);
        controller.tick(start);
        let mut last = controller.tick(start + Duration::from_millis(16));
        // Only the rise: a bouncy spring turns back after it passes its target.
        for frame in 2..12 {
            controller.select(2.0 + frame as f32 * 1.0e-3);
            let now = controller.tick(start + Duration::from_millis(16 * frame));
            assert!(now.position > last.position, "frame {frame}: {last:?} -> {now:?}");
            last = now;
        }
    }

    #[test]
    fn a_fade_duration_sets_how_long_the_lens_takes_to_form_and_to_flatten() {
        let controller = IndicatorController::new(0.0).fade_duration(Duration::from_millis(100));
        controller.set_pressed(true);
        assert!((controller.advance(0.05).lift - 0.5).abs() < 1.0e-3);
        assert_eq!(controller.advance(0.06).lift, 1.0);

        controller.set_pressed(false);
        assert!((controller.advance(0.05).lift - 0.5).abs() < 1.0e-3);
        assert_eq!(controller.advance(0.06).lift, 0.0);
        assert!(!controller.is_animating());
    }

    #[test]
    fn a_zero_fade_duration_is_instant() {
        let controller = IndicatorController::new(0.0).fade_duration(Duration::ZERO);
        controller.set_pressed(true);
        assert_eq!(controller.advance(FRAME).lift, 1.0);
        controller.set_pressed(false);
        assert_eq!(controller.advance(FRAME).lift, 0.0);
    }

    /// The time at which the lens, lifted at the start, has flattened to
    /// almost nothing, for a move of two slots.
    fn time_to_flatten(controller: &IndicatorController) -> f32 {
        controller.select(2.0);
        let mut peaked = false;
        for (frame, sample) in run(controller, 2.0).into_iter().enumerate() {
            peaked |= sample.lift > 0.9;
            if peaked && sample.lift < 0.05 {
                return (frame + 1) as f32 * FRAME;
            }
        }
        panic!("the lens never flattened");
    }

    #[test]
    fn the_lens_flattens_as_the_indicator_arrives_not_after_its_spring_rings_out() {
        // The travel spring takes the better part of a second to stop ringing;
        // the lens used to wait for all of it. It is on its way home long before.
        let controller = IndicatorController::new(0.0);
        let flattened = time_to_flatten(&controller);
        assert!(flattened < 0.4, "the lens took {flattened} s to flatten");
        // The indicator itself is still settling when the lens is gone.
        assert!(controller.is_animating() || (controller.position() - 2.0).abs() < 0.01);
    }

    #[test]
    fn an_overshoot_after_arriving_does_not_lift_the_lens_again() {
        let controller = IndicatorController::new(0.0);
        controller.select(2.0);
        let trace = run(&controller, 2.0);
        // From the top of the lift on, the lens only ever goes down, even though
        // the indicator goes on overshooting and swinging back.
        let top = trace
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.lift.total_cmp(&b.1.lift))
            .map(|(frame, _)| frame)
            .unwrap();
        let swings_after = trace[top..].iter().any(|sample| sample.position > 2.0);
        assert!(swings_after, "the test needs an overshoot to mean anything");
        let mut lowest = f32::MAX;
        for sample in &trace[top..] {
            assert!(sample.lift <= lowest + 0.02, "the lens rose again: {sample:?}");
            lowest = lowest.min(sample.lift);
        }
    }

    /// The time the lens takes to flatten once a held indicator is let go.
    fn time_to_let_go(controller: &IndicatorController) -> f32 {
        controller.set_pressed(true);
        run(controller, 1.0);
        assert_eq!(controller.lift(), 1.0);
        controller.set_pressed(false);
        for (frame, sample) in run(controller, 1.0).into_iter().enumerate() {
            if sample.lift < 0.05 {
                return (frame + 1) as f32 * FRAME;
            }
        }
        panic!("the lens never flattened");
    }

    #[test]
    fn pressing_and_letting_go_without_moving_flattens_the_lens() {
        // Nothing moves, so the controller looks idle the moment the lens is let
        // go: it still has to notice that it is already home and flatten.
        let controller = IndicatorController::new(0.0);
        let track = row();
        controller.begin_drag(0.0, (50.0, 20.0));
        run(&controller, 1.0);
        assert_eq!(controller.lift(), 1.0);

        controller.end_drag(&track);
        assert!(controller.is_animating(), "a lens still lifted needs frames");
        assert_eq!(run(&controller, 1.0).pop().unwrap().lift, 0.0);
    }

    #[test]
    fn a_tap_on_the_selected_slot_lifts_and_then_flattens_the_lens() {
        let controller = IndicatorController::new(1.0);
        controller.set_pressed(true);
        run(&controller, 1.0);
        assert_eq!(controller.lift(), 1.0);
        controller.set_pressed(false);
        assert_eq!(run(&controller, 1.0).pop().unwrap().lift, 0.0);
    }

    #[test]
    fn a_shorter_fade_flattens_the_lens_sooner() {
        let slow = time_to_let_go(&IndicatorController::new(0.0).fade_duration(Duration::from_millis(400)));
        let quick = time_to_let_go(&IndicatorController::new(0.0).fade_duration(Duration::from_millis(60)));
        // A ramp takes the fade it was given, less the last twentieth of it.
        assert!((slow - 0.38).abs() < 0.03, "slow {slow} s");
        assert!((quick - 0.057).abs() < 0.03, "quick {quick} s");
    }

    #[test]
    fn a_hidden_indicator_fades_out_and_a_shown_one_fades_back() {
        let controller = IndicatorController::new(1.0).fade_duration(Duration::from_millis(100));
        assert_eq!(controller.presence(), 1.0);
        controller.set_hidden(true);
        assert!(controller.is_hidden());
        assert!(controller.is_animating());
        assert!((controller.advance(0.05).presence - 0.5).abs() < 1.0e-3);
        assert_eq!(controller.advance(0.06).presence, 0.0);
        assert!(!controller.is_animating(), "a hidden indicator needs no frames");

        controller.set_hidden(false);
        assert!(!controller.is_hidden());
        assert_eq!(run(&controller, 0.5).pop().unwrap().presence, 1.0);
    }

    #[test]
    fn hiding_does_not_move_or_lift_the_indicator() {
        let controller = IndicatorController::new(2.0);
        controller.set_hidden(true);
        let hidden = run(&controller, 1.0).pop().unwrap();
        assert_eq!(hidden, IndicatorSample { position: 2.0, lift: 0.0, presence: 0.0 });
    }

    #[test]
    fn pressing_a_hidden_indicator_shows_it_under_the_pointer() {
        let controller = IndicatorController::new(0.0);
        controller.set_hidden(true);
        run(&controller, 1.0);
        controller.begin_drag(2.0, (250.0, 20.0));
        assert!(!controller.is_hidden());
        let shown = run(&controller, 1.0).pop().unwrap();
        assert_eq!((shown.position, shown.lift, shown.presence), (2.0, 1.0, 1.0));
    }

    #[test]
    fn a_drag_cannot_be_hidden_from_under_the_pointer() {
        let controller = IndicatorController::new(0.0);
        controller.begin_drag(0.0, (50.0, 20.0));
        controller.set_hidden(true);
        assert!(!controller.is_hidden());
    }

    #[test]
    fn with_reduced_motion_hiding_is_instant() {
        let controller = IndicatorController::new(0.0).reduced_motion(true);
        controller.set_hidden(true);
        assert_eq!(controller.advance(FRAME).presence, 0.0);
    }

    #[test]
    fn clones_share_one_state() {
        let controller = IndicatorController::new(0.0);
        let clone = controller.clone();
        clone.select(2.0);
        assert_eq!(controller.target(), 2.0);
    }
}
