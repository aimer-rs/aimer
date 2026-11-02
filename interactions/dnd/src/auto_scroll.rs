//! Bounded edge scrolling for drag interactions.
//!
//! The auto-scroll policy deliberately knows nothing about a scroll engine. It
//! reads a small viewport snapshot and sends a bounded logical delta through a
//! [`ScrollTarget`] adapter. This keeps `aimer_scroll`'s physics, overscroll,
//! and animation policy unchanged while making the drag behavior deterministic
//! to test.

use std::fmt;
use std::ops::BitOr;
use std::time::Duration;

use aimer_attribute::position::Vec2d;

/// The geometry and logical position exposed by a scroll adapter.
///
/// `offset` and `max_offset` are positive logical distances from the content's
/// start. The adapter may clamp its own state as well; the policy clamps the
/// request before sending it so an edge tick cannot cross the viewport's
/// bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollViewport {
    /// The viewport's top-left corner in window coordinates.
    pub origin: Vec2d,
    /// The viewport's width and height in logical pixels.
    pub size: Vec2d,
    /// The current logical scroll offset.
    pub offset: Vec2d,
    /// The greatest valid logical offset on each axis.
    pub max_offset: Vec2d,
}

impl ScrollViewport {
    /// Creates a viewport snapshot.
    #[inline]
    pub const fn new(origin: Vec2d, size: Vec2d, offset: Vec2d, max_offset: Vec2d) -> Self {
        Self {
            origin,
            size,
            offset,
            max_offset,
        }
    }

    /// Whether every value is finite and the geometry has non-negative extents.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.origin.x.is_finite()
            && self.origin.y.is_finite()
            && self.size.x.is_finite()
            && self.size.y.is_finite()
            && self.size.x >= 0.0
            && self.size.y >= 0.0
            && self.offset.x.is_finite()
            && self.offset.y.is_finite()
            && self.max_offset.x.is_finite()
            && self.max_offset.y.is_finite()
            && self.max_offset.x >= 0.0
            && self.max_offset.y >= 0.0
    }

    #[inline]
    fn clamped_offset(self) -> Vec2d {
        Vec2d {
            x: self.offset.x.clamp(0.0, self.max_offset.x),
            y: self.offset.y.clamp(0.0, self.max_offset.y),
        }
    }
}

/// The platform-neutral seam used by drag auto-scroll.
///
/// Implement this for a scroll controller or a collection viewport. The
/// `request_scroll` call receives a logical delta that has already been
/// bounded by [`AutoScrollPolicy`]. Implementations should still clamp their
/// own state because another input source may have changed the viewport after
/// the snapshot was read.
pub trait ScrollTarget {
    /// Returns the latest viewport geometry and logical offset.
    fn viewport(&self) -> ScrollViewport;

    /// Requests a logical scroll delta.
    fn request_scroll(&self, delta: Vec2d);
}

/// The edge or edges that caused an auto-scroll step.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AutoScrollEdges(u8);

impl AutoScrollEdges {
    /// No edge is active.
    pub const NONE: Self = Self(0);
    /// The pointer is in the top edge band.
    pub const TOP: Self = Self(1 << 0);
    /// The pointer is in the bottom edge band.
    pub const BOTTOM: Self = Self(1 << 1);
    /// The pointer is in the left edge band.
    pub const LEFT: Self = Self(1 << 2);
    /// The pointer is in the right edge band.
    pub const RIGHT: Self = Self(1 << 3);

    /// Whether this set contains every flag in `other`.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no edge is active.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for AutoScrollEdges {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// The result of one auto-scroll calculation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AutoScrollStep {
    /// The requested velocity before the elapsed-time step, in logical pixels
    /// per second.
    pub velocity: Vec2d,
    /// The bounded logical delta sent to the target for this tick.
    pub delta: Vec2d,
    /// The edge bands contributing to `velocity`.
    pub edges: AutoScrollEdges,
}

impl AutoScrollStep {
    /// Returns a step with no velocity, edge, or target request.
    #[inline]
    pub const fn idle() -> Self {
        Self {
            velocity: Vec2d::ZERO,
            delta: Vec2d::ZERO,
            edges: AutoScrollEdges::NONE,
        }
    }

    /// Whether this step requested movement.
    #[inline]
    pub fn is_active(self) -> bool {
        self.delta.x != 0.0 || self.delta.y != 0.0
    }
}

/// Why an auto-scroll policy could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutoScrollPolicyError {
    /// The edge band must be finite and greater than zero.
    InvalidEdgeExtent,
    /// The maximum velocity must be finite and greater than zero.
    InvalidMaxVelocity,
    /// The per-tick delta must be finite and greater than zero.
    InvalidMaxDelta,
    /// The elapsed-time cap must not be zero.
    InvalidFrameInterval,
}

impl fmt::Display for AutoScrollPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidEdgeExtent => "edge extent must be finite and greater than zero",
            Self::InvalidMaxVelocity => "maximum velocity must be finite and greater than zero",
            Self::InvalidMaxDelta => "maximum delta must be finite and greater than zero",
            Self::InvalidFrameInterval => "frame interval must be greater than zero",
        };
        f.write_str(message)
    }
}

impl std::error::Error for AutoScrollPolicyError {}

/// Configuration for edge-triggered drag scrolling.
///
/// The default policy uses a 48 logical-pixel edge band, a 720 logical-pixel
/// per-second velocity cap, a 32-pixel per-tick cap, and ignores elapsed gaps
/// longer than 50 ms. The last two bounds prevent a stalled frame or a paused
/// tab from producing a large catch-up jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoScrollPolicy {
    edge_extent: f32,
    max_velocity: f32,
    max_delta_per_tick: f32,
    max_frame_interval: Duration,
}

impl Default for AutoScrollPolicy {
    fn default() -> Self {
        Self {
            edge_extent: 48.0,
            max_velocity: 720.0,
            max_delta_per_tick: 32.0,
            max_frame_interval: Duration::from_millis(50),
        }
    }
}

impl AutoScrollPolicy {
    /// Creates a policy with the requested edge extent and velocity cap.
    ///
    /// The per-tick bound defaults to one-thirtieth of the velocity cap and
    /// the elapsed-time cap defaults to 50 ms. Use the builder methods to
    /// tighten either bound.
    pub fn new(edge_extent: f32, max_velocity: f32) -> Result<Self, AutoScrollPolicyError> {
        if !edge_extent.is_finite() || edge_extent <= 0.0 {
            return Err(AutoScrollPolicyError::InvalidEdgeExtent);
        }
        if !max_velocity.is_finite() || max_velocity <= 0.0 {
            return Err(AutoScrollPolicyError::InvalidMaxVelocity);
        }
        Ok(Self {
            edge_extent,
            max_velocity,
            max_delta_per_tick: (max_velocity / 30.0).max(f32::EPSILON),
            max_frame_interval: Duration::from_millis(50),
        })
    }

    /// Sets the width of each edge trigger band.
    #[inline]
    pub fn with_edge_extent(mut self, edge_extent: f32) -> Result<Self, AutoScrollPolicyError> {
        if !edge_extent.is_finite() || edge_extent <= 0.0 {
            return Err(AutoScrollPolicyError::InvalidEdgeExtent);
        }
        self.edge_extent = edge_extent;
        Ok(self)
    }

    /// Sets the maximum speed on either axis.
    #[inline]
    pub fn with_max_velocity(mut self, max_velocity: f32) -> Result<Self, AutoScrollPolicyError> {
        if !max_velocity.is_finite() || max_velocity <= 0.0 {
            return Err(AutoScrollPolicyError::InvalidMaxVelocity);
        }
        self.max_velocity = max_velocity;
        Ok(self)
    }

    /// Sets the maximum absolute delta on either axis for one tick.
    #[inline]
    pub fn with_max_delta_per_tick(
        mut self,
        max_delta_per_tick: f32,
    ) -> Result<Self, AutoScrollPolicyError> {
        if !max_delta_per_tick.is_finite() || max_delta_per_tick <= 0.0 {
            return Err(AutoScrollPolicyError::InvalidMaxDelta);
        }
        self.max_delta_per_tick = max_delta_per_tick;
        Ok(self)
    }

    /// Sets the largest elapsed interval a tick may integrate.
    #[inline]
    pub fn with_max_frame_interval(
        mut self,
        max_frame_interval: Duration,
    ) -> Result<Self, AutoScrollPolicyError> {
        if max_frame_interval.is_zero() {
            return Err(AutoScrollPolicyError::InvalidFrameInterval);
        }
        self.max_frame_interval = max_frame_interval;
        Ok(self)
    }

    /// The edge-band width in logical pixels.
    #[inline]
    pub const fn edge_extent(self) -> f32 {
        self.edge_extent
    }

    /// The maximum per-axis velocity in logical pixels per second.
    #[inline]
    pub const fn max_velocity(self) -> f32 {
        self.max_velocity
    }

    /// The maximum per-axis delta sent on one tick.
    #[inline]
    pub const fn max_delta_per_tick(self) -> f32 {
        self.max_delta_per_tick
    }

    /// Computes the bounded velocity for a pointer position.
    #[inline]
    pub fn velocity(self, pointer: Vec2d, viewport: ScrollViewport) -> Vec2d {
        self.velocity_with_edges(pointer, viewport).0
    }

    /// Computes one bounded edge step after `elapsed`.
    ///
    /// `elapsed` is supplied by the caller instead of read from a global clock,
    /// which keeps this seam deterministic in unit and integration tests.
    pub fn step(
        self,
        pointer: Vec2d,
        viewport: ScrollViewport,
        elapsed: Duration,
    ) -> AutoScrollStep {
        let (velocity, edges) = self.velocity_with_edges(pointer, viewport);
        if edges.is_empty() || elapsed.is_zero() {
            return AutoScrollStep {
                velocity,
                delta: Vec2d::ZERO,
                edges,
            };
        }

        let seconds = elapsed.min(self.max_frame_interval).as_secs_f32();
        let mut delta = Vec2d {
            x: (velocity.x * seconds).clamp(-self.max_delta_per_tick, self.max_delta_per_tick),
            y: (velocity.y * seconds).clamp(-self.max_delta_per_tick, self.max_delta_per_tick),
        };
        let offset = viewport.clamped_offset();
        delta.x = delta.x.clamp(-offset.x, viewport.max_offset.x - offset.x);
        delta.y = delta.y.clamp(-offset.y, viewport.max_offset.y - offset.y);

        AutoScrollStep {
            velocity,
            delta,
            edges,
        }
    }

    fn velocity_with_edges(
        self,
        pointer: Vec2d,
        viewport: ScrollViewport,
    ) -> (Vec2d, AutoScrollEdges) {
        if !viewport.is_valid()
            || !pointer.x.is_finite()
            || !pointer.y.is_finite()
            || viewport.size.x <= 0.0
            || viewport.size.y <= 0.0
        {
            return (Vec2d::ZERO, AutoScrollEdges::NONE);
        }

        let x_end = viewport.origin.x + viewport.size.x;
        let y_end = viewport.origin.y + viewport.size.y;
        if !x_end.is_finite() || !y_end.is_finite() {
            return (Vec2d::ZERO, AutoScrollEdges::NONE);
        }

        let (x, x_edges) = axis_velocity(
            pointer.x,
            viewport.origin.x,
            x_end,
            viewport.offset.x,
            viewport.max_offset.x,
            self.edge_extent,
            self.max_velocity,
            AutoScrollEdges::LEFT,
            AutoScrollEdges::RIGHT,
        );
        let (y, y_edges) = axis_velocity(
            pointer.y,
            viewport.origin.y,
            y_end,
            viewport.offset.y,
            viewport.max_offset.y,
            self.edge_extent,
            self.max_velocity,
            AutoScrollEdges::TOP,
            AutoScrollEdges::BOTTOM,
        );
        (Vec2d { x, y }, x_edges | y_edges)
    }
}

impl AutoScrollStep {
    /// Whether no edge contributed to this step.
    #[inline]
    pub const fn is_idle(self) -> bool {
        self.edges.is_empty()
    }
}

/// A stateful adapter that turns deterministic clock ticks into target requests.
pub struct AutoScroller {
    policy: AutoScrollPolicy,
    last_tick: Option<Duration>,
}

impl AutoScroller {
    /// Creates an inactive scroller using `policy`.
    #[inline]
    pub const fn new(policy: AutoScrollPolicy) -> Self {
        Self {
            policy,
            last_tick: None,
        }
    }

    /// Creates an inactive scroller using the default policy.
    #[inline]
    pub fn default_policy() -> Self {
        Self::new(AutoScrollPolicy::default())
    }

    /// Returns the configured policy.
    #[inline]
    pub const fn policy(&self) -> AutoScrollPolicy {
        self.policy
    }

    /// Applies one tick at a monotonic, caller-provided time.
    ///
    /// The first tick only establishes a baseline. A backwards clock jump is
    /// treated the same way and re-baselines without requesting a catch-up
    /// scroll. This makes cancellation, tab suspension, and test clocks safe.
    pub fn tick_at<T: ScrollTarget + ?Sized>(
        &mut self,
        now: Duration,
        pointer: Vec2d,
        target: &T,
    ) -> AutoScrollStep {
        let Some(previous) = self.last_tick.replace(now) else {
            return AutoScrollStep::idle();
        };
        let Some(elapsed) = now.checked_sub(previous) else {
            return AutoScrollStep::idle();
        };
        let step = self.policy.step(pointer, target.viewport(), elapsed);
        if step.is_active() {
            target.request_scroll(step.delta);
        }
        step
    }

    /// Stops the current edge-scroll episode and clears its clock baseline.
    #[inline]
    pub fn cancel(&mut self) {
        self.last_tick = None;
    }
}

impl Default for AutoScroller {
    fn default() -> Self {
        Self::default_policy()
    }
}

fn axis_velocity(
    position: f32,
    start: f32,
    end: f32,
    offset: f32,
    max_offset: f32,
    configured_edge: f32,
    max_velocity: f32,
    start_edge: AutoScrollEdges,
    end_edge: AutoScrollEdges,
) -> (f32, AutoScrollEdges) {
    if max_offset <= 0.0 || end <= start {
        return (0.0, AutoScrollEdges::NONE);
    }

    let edge = configured_edge.min((end - start) * 0.5);
    if edge <= 0.0 {
        return (0.0, AutoScrollEdges::NONE);
    }

    let start_intensity = ((edge - (position - start)) / edge).clamp(0.0, 1.0);
    let end_intensity = ((edge - (end - position)) / edge).clamp(0.0, 1.0);
    let can_start = offset > f32::EPSILON;
    let can_end = offset < max_offset - f32::EPSILON;

    let mut start_intensity = if can_start { start_intensity } else { 0.0 };
    let mut end_intensity = if can_end { end_intensity } else { 0.0 };

    if start_intensity > 0.0 && end_intensity > 0.0 {
        if start_intensity >= end_intensity {
            end_intensity = 0.0;
        } else {
            start_intensity = 0.0;
        }
    }

    if start_intensity > 0.0 {
        (-max_velocity * start_intensity, start_edge)
    } else if end_intensity > 0.0 {
        (max_velocity * end_intensity, end_edge)
    } else {
        (0.0, AutoScrollEdges::NONE)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    use aimer_attribute::position::Vec2d;

    use super::*;

    #[derive(Debug)]
    struct FakeTarget {
        viewport: Cell<ScrollViewport>,
        requests: Cell<Vec2d>,
    }

    impl ScrollTarget for FakeTarget {
        fn viewport(&self) -> ScrollViewport {
            self.viewport.get()
        }

        fn request_scroll(&self, delta: Vec2d) {
            self.requests.set(delta);
        }
    }

    fn viewport(offset_y: f32) -> ScrollViewport {
        ScrollViewport::new(
            Vec2d { x: 10.0, y: 20.0 },
            Vec2d { x: 300.0, y: 200.0 },
            Vec2d {
                x: 0.0,
                y: offset_y,
            },
            Vec2d { x: 0.0, y: 800.0 },
        )
    }

    fn policy() -> AutoScrollPolicy {
        AutoScrollPolicy::new(40.0, 600.0)
            .expect("the test policy is finite")
            .with_max_delta_per_tick(20.0)
            .expect("the test delta is positive")
    }

    #[test]
    fn the_pointer_must_enter_the_edge_band_before_scrolling() {
        let policy = policy();
        let view = viewport(200.0);

        assert_eq!(
            policy.velocity(Vec2d { x: 150.0, y: 179.0 }, view),
            Vec2d::ZERO
        );
        assert!(policy.velocity(Vec2d { x: 150.0, y: 181.0 }, view).y > 0.0);
    }

    #[test]
    fn edge_velocity_and_per_tick_delta_are_bounded() {
        let policy = policy();
        let view = viewport(200.0);
        let pointer = Vec2d { x: 150.0, y: 220.0 };

        let velocity = policy.velocity(pointer, view);
        let step = policy.step(pointer, view, Duration::from_secs(10));

        assert!(velocity.y <= 600.0);
        assert!(velocity.y >= 0.0);
        assert_eq!(step.delta, Vec2d { x: 0.0, y: 20.0 });
    }

    #[test]
    fn a_boundary_does_not_request_more_scroll_in_that_direction() {
        let policy = policy();

        assert_eq!(
            policy.velocity(Vec2d { x: 150.0, y: 20.0 }, viewport(0.0)),
            Vec2d::ZERO
        );
        assert_eq!(
            policy.velocity(Vec2d { x: 150.0, y: 219.0 }, viewport(800.0)),
            Vec2d::ZERO
        );
    }

    #[test]
    fn a_tick_uses_a_deterministic_clock_and_discards_backwards_time() {
        let target = FakeTarget {
            viewport: Cell::new(viewport(200.0)),
            requests: Cell::new(Vec2d::ZERO),
        };
        let mut scroller = AutoScroller::new(policy());
        let pointer = Vec2d { x: 150.0, y: 219.0 };

        assert_eq!(
            scroller.tick_at(Duration::from_millis(100), pointer, &target),
            AutoScrollStep::idle()
        );
        let step = scroller.tick_at(Duration::from_millis(116), pointer, &target);
        assert!(step.delta.y > 0.0);
        assert_eq!(target.requests.get(), step.delta);

        target.requests.set(Vec2d::ZERO);
        let backwards = scroller.tick_at(Duration::from_millis(90), pointer, &target);
        assert_eq!(backwards.delta, Vec2d::ZERO);
        assert_eq!(target.requests.get(), Vec2d::ZERO);
    }

    #[test]
    fn cancelling_resets_the_clock_without_a_catch_up_delta() {
        let target = FakeTarget {
            viewport: Cell::new(viewport(200.0)),
            requests: Cell::new(Vec2d::ZERO),
        };
        let mut scroller = AutoScroller::new(policy());
        let pointer = Vec2d { x: 150.0, y: 219.0 };

        let _ = scroller.tick_at(Duration::from_millis(10), pointer, &target);
        scroller.cancel();
        assert_eq!(
            scroller.tick_at(Duration::from_secs(5), pointer, &target),
            AutoScrollStep::idle()
        );
    }
}
