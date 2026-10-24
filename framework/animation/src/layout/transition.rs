use crate::control::AnimationController;
use crate::primitives::{AnimInstant, AnyCurve, Curve, curves};
use aimer_events::window::request_animation_frame;
use aimer_widget::Element as _;
use std::fmt;
use std::time::Duration;

/// A finite rectangle in the coordinate space of a layout parent.
///
/// Positions may be negative because a layout can be translated or clipped,
/// but extents are always non-negative. Keeping this value validated at the
/// boundary means an animation cannot turn an invalid layout into a `NaN` or
/// infinite frame later on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutGeometry {
    /// Horizontal position relative to the parent.
    pub x: f32,
    /// Vertical position relative to the parent.
    pub y: f32,
    /// Resolved width in logical pixels.
    pub width: f32,
    /// Resolved height in logical pixels.
    pub height: f32,
}

impl LayoutGeometry {
    /// Creates a validated rectangle from position and extent components.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, LayoutGeometryError> {
        Self::try_new(x, y, width, height)
    }

    /// Creates a validated rectangle from position and extent components.
    pub fn try_new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, LayoutGeometryError> {
        let geometry = Self {
            x,
            y,
            width,
            height,
        };
        geometry.validate()?;
        Ok(geometry)
    }

    /// Returns the rectangle with a zero extent at the same position.
    ///
    /// This is the bounded enter/exit baseline used by keyed list transitions.
    #[inline]
    pub const fn collapsed(self) -> Self {
        Self {
            x: self.x,
            y: self.y,
            width: 0.0,
            height: 0.0,
        }
    }

    /// Returns whether all components are finite and the extents are valid.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 0.0
            && self.height >= 0.0
    }

    /// Validates this rectangle without changing it.
    pub fn validate(self) -> Result<(), LayoutGeometryError> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || !self.width.is_finite()
            || !self.height.is_finite()
        {
            return Err(LayoutGeometryError::NonFinite);
        }
        if self.width < 0.0 || self.height < 0.0 {
            return Err(LayoutGeometryError::NegativeExtent);
        }
        Ok(())
    }

    /// Interpolates all four components, clamping progress to `[0, 1]`.
    pub fn interpolate(self, target: &Self, progress: f32) -> Result<Self, LayoutGeometryError> {
        self.validate()?;
        target.validate()?;
        if !progress.is_finite() {
            return Err(LayoutGeometryError::NonFiniteProgress);
        }
        let progress = progress.clamp(0.0, 1.0);
        Self::try_new(
            interpolate_component(self.x, target.x, progress),
            interpolate_component(self.y, target.y, progress),
            interpolate_component(self.width, target.width, progress),
            interpolate_component(self.height, target.height, progress),
        )
    }
}

#[inline]
fn interpolate_component(from: f32, to: f32, progress: f32) -> f32 {
    // Use f64 for the blend so two finite, opposite-sign f32 positions do not
    // overflow in `to - from` before the interpolation can bring them back
    // into range.
    (f64::from(from) + (f64::from(to) - f64::from(from)) * f64::from(progress)) as f32
}

/// Failure returned when a layout rectangle or interpolation input is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutGeometryError {
    /// A position, extent, or progress value was `NaN` or infinite.
    NonFinite,
    /// A rectangle width or height was negative.
    NegativeExtent,
    /// Interpolation progress was `NaN` or infinite.
    NonFiniteProgress,
}

impl fmt::Display for LayoutGeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NonFinite => "layout geometry must contain only finite values",
            Self::NegativeExtent => "layout geometry extents must be non-negative",
            Self::NonFiniteProgress => "layout interpolation progress must be finite",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for LayoutGeometryError {}

/// The largest duration accepted by a layout transition configuration.
///
/// A bounded duration prevents a stale transition from keeping a redraw loop
/// alive indefinitely after a caller accidentally supplies an extreme value.
pub const MAX_LAYOUT_TRANSITION_DURATION: Duration = Duration::from_secs(60);

/// Errors returned when a layout transition configuration cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutTransitionError {
    /// The supplied curve contains a non-finite control point.
    NonFiniteCurve,
    /// The requested duration exceeds [`MAX_LAYOUT_TRANSITION_DURATION`].
    DurationTooLong,
    /// A geometry or progress value was invalid.
    Geometry(LayoutGeometryError),
}

impl fmt::Display for LayoutTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteCurve => formatter.write_str("layout transition curve must be finite"),
            Self::DurationTooLong => formatter.write_str("layout transition duration is too long"),
            Self::Geometry(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for LayoutTransitionError {}

impl From<LayoutGeometryError> for LayoutTransitionError {
    fn from(error: LayoutGeometryError) -> Self {
        Self::Geometry(error)
    }
}

/// Configuration shared by scalar and keyed layout transitions.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutTransitionConfig {
    duration: Duration,
    curve: AnyCurve,
    enabled: bool,
    reduced_motion: bool,
}

impl Default for LayoutTransitionConfig {
    fn default() -> Self {
        Self {
            duration: Duration::from_millis(250),
            curve: curves::EASE_IN_OUT.into_any(),
            enabled: true,
            reduced_motion: false,
        }
    }
}

impl LayoutTransitionConfig {
    /// Creates the default 250ms ease-in-out configuration.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the configured duration.
    #[inline]
    pub const fn configured_duration(&self) -> Duration {
        self.duration
    }

    /// Sets the duration, clamping it to the documented safety bound.
    ///
    /// Use [`Self::try_duration`] when invalid input should be reported rather
    /// than clamped.
    #[inline]
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration.min(MAX_LAYOUT_TRANSITION_DURATION);
        self
    }

    /// Sets the duration and reports values outside the bounded range.
    pub fn try_duration(mut self, duration: Duration) -> Result<Self, LayoutTransitionError> {
        if duration > MAX_LAYOUT_TRANSITION_DURATION {
            return Err(LayoutTransitionError::DurationTooLong);
        }
        self.duration = duration;
        Ok(self)
    }

    /// Returns the easing curve used for progress.
    #[inline]
    pub fn configured_curve(&self) -> AnyCurve {
        self.curve.clone()
    }

    /// Sets the easing curve.
    ///
    /// A non-finite cubic-bezier curve is replaced with linear progress so a
    /// builder cannot accidentally publish invalid frame geometry. Use
    /// [`Self::try_curve`] when the caller needs an explicit error.
    #[inline]
    pub fn curve(mut self, curve: impl Curve) -> Self {
        let curve = curve.into_any();
        self.curve = if curve.is_valid() { curve } else { AnyCurve::default() };
        self
    }

    /// Sets the easing curve and rejects non-finite cubic-bezier control
    /// points.
    pub fn try_curve(mut self, curve: impl Curve) -> Result<Self, LayoutTransitionError> {
        if !curve.is_valid() {
            return Err(LayoutTransitionError::NonFiniteCurve);
        }
        self.curve = curve.into_any();
        Ok(self)
    }

    /// Enables or disables layout movement.
    #[inline]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Returns whether transitions are enabled.
    #[inline]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Sets the reduced-motion policy. Reduced motion settles at the final
    /// geometry while retaining the same layout result.
    #[inline]
    pub const fn reduced_motion(mut self, reduced_motion: bool) -> Self {
        self.reduced_motion = reduced_motion;
        self
    }

    /// Returns whether reduced motion is requested.
    #[inline]
    pub const fn reduced_motion_enabled(&self) -> bool {
        self.reduced_motion
    }

    fn validate(&self) -> Result<(), LayoutTransitionError> {
        if self.duration > MAX_LAYOUT_TRANSITION_DURATION {
            return Err(LayoutTransitionError::DurationTooLong);
        }
        if !self.curve.is_valid() {
            return Err(LayoutTransitionError::NonFiniteCurve);
        }
        Ok(())
    }

    #[inline]
    fn allows_animation(&self) -> bool {
        self.enabled && !self.reduced_motion && !self.duration.is_zero()
    }
}

/// A controller-backed transition between two resolved rectangles.
///
/// The first target establishes the initial geometry immediately. Later
/// targets animate only when the policy allows movement. Calling
/// [`Self::set_target`] while a transition is active samples the current
/// interpolated rectangle first, then starts the new transition from that
/// exact rectangle. This is the interruption/retargeting invariant that keeps
/// a responsive layout from visibly jumping.
#[derive(Debug)]
pub struct LayoutTransition {
    config: LayoutTransitionConfig,
    controller: AnimationController,
    start: Option<LayoutGeometry>,
    current: Option<LayoutGeometry>,
    target: Option<LayoutGeometry>,
}

impl Default for LayoutTransition {
    fn default() -> Self {
        Self::new(LayoutTransitionConfig::default())
    }
}

impl LayoutTransition {
    /// Creates a transition. Invalid builder input is already sanitized by
    /// [`LayoutTransitionConfig::duration`] and [`LayoutTransitionConfig::curve`].
    pub fn new(config: LayoutTransitionConfig) -> Self {
        Self::try_new(config).unwrap_or_else(|_| {
            Self::try_new(LayoutTransitionConfig::default())
                .expect("the default layout transition configuration is valid")
        })
    }

    /// Creates a transition after validating its configuration.
    pub fn try_new(config: LayoutTransitionConfig) -> Result<Self, LayoutTransitionError> {
        config.validate()?;
        Ok(Self {
            controller: AnimationController::new(config.duration, config.curve.clone()),
            config,
            start: None,
            current: None,
            target: None,
        })
    }

    /// Returns the current configuration.
    #[inline]
    pub fn config(&self) -> LayoutTransitionConfig {
        self.config.clone()
    }

    /// Returns the current interpolated rectangle, if a target was installed.
    #[inline]
    pub const fn current(&self) -> Option<LayoutGeometry> {
        self.current
    }

    /// Returns the latest requested rectangle.
    #[inline]
    pub const fn target(&self) -> Option<LayoutGeometry> {
        self.target
    }

    /// Returns whether a transition is currently moving.
    #[inline]
    pub fn is_animating(&self) -> bool {
        self.current != self.target && self.controller.is_animating()
    }

    /// Returns whether the current rectangle has reached its target.
    #[inline]
    pub fn is_settled(&self) -> bool {
        self.current.is_some() && self.current == self.target
    }

    /// Installs a new target at `now`.
    pub fn set_target(
        &mut self,
        target: LayoutGeometry,
        now: AnimInstant,
    ) -> Result<(), LayoutTransitionError> {
        target.validate()?;
        if self.target == Some(target) && self.current.is_some() {
            return Ok(());
        }

        let Some(from) = self.advance(now).or(self.current) else {
            self.start = Some(target);
            self.current = Some(target);
            self.target = Some(target);
            return Ok(());
        };

        self.target = Some(target);
        if from == target || !self.config.allows_animation() {
            self.start = Some(target);
            self.current = Some(target);
            self.controller.reset();
            return Ok(());
        }

        self.start = Some(from);
        self.current = Some(from);
        self.controller = AnimationController::new(self.config.duration, self.config.curve.clone());
        self.controller.forward_from_first_tick();
        // Arm the controller against the caller's clock. `forward_from_first_tick`
        // deliberately waits for a tick so ordinary widgets do not spend time
        // between construction and their first frame; this model has an
        // injected instant, so the target-installing call is that first frame.
        let _ = self.controller.tick(now);
        Ok(())
    }

    /// Samples the transition at `now` and returns the current rectangle.
    ///
    /// Time is injected through [`AnimInstant`] so callers and tests can drive
    /// the exact same state machine without sleeping.
    pub fn advance(&mut self, now: AnimInstant) -> Option<LayoutGeometry> {
        let (Some(start), Some(target)) = (self.start, self.target) else {
            return self.current;
        };
        if !self.controller.is_animating() {
            self.current = Some(target);
            self.start = Some(target);
            return self.current;
        }

        let progress = self.controller.tick(now);
        self.current = Some(start.interpolate(&target, progress).unwrap_or(target));
        if !self.controller.is_animating() {
            self.current = Some(target);
            self.start = Some(target);
        }
        self.current
    }

    /// Alias for [`Self::advance`] that reads naturally at a frame boundary.
    #[inline]
    pub fn sample(&mut self, now: AnimInstant) -> Option<LayoutGeometry> {
        self.advance(now)
    }

    /// Changes the enabled policy and settles immediately when movement is no
    /// longer allowed.
    pub fn set_enabled(&mut self, enabled: bool, now: AnimInstant) {
        self.config.enabled = enabled;
        if !enabled {
            self.advance(now);
            self.settle();
        }
    }

    /// Changes reduced-motion policy and settles immediately when it is on.
    pub fn set_reduced_motion(&mut self, reduced_motion: bool, now: AnimInstant) {
        self.config.reduced_motion = reduced_motion;
        if reduced_motion {
            self.advance(now);
            self.settle();
        }
    }

    /// Replaces the full configuration. Turning movement off settles at the
    /// final target immediately.
    pub fn set_config(
        &mut self,
        config: LayoutTransitionConfig,
        now: AnimInstant,
    ) -> Result<(), LayoutTransitionError> {
        config.validate()?;
        self.config = config;
        self.controller.set_duration(self.config.duration);
        self.controller.set_curve(self.config.curve.clone());
        if !self.config.allows_animation() {
            self.advance(now);
            self.settle();
        }
        Ok(())
    }

    /// Settles at the latest target without changing that target.
    pub fn settle(&mut self) {
        if let Some(target) = self.target {
            self.start = Some(target);
            self.current = Some(target);
        }
        self.controller.reset();
    }
}

/// A layout rectangle associated with a caller-owned stable key.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutItem<K = String> {
    key: K,
    geometry: LayoutGeometry,
}

impl<K> LayoutItem<K> {
    /// Creates an item. The enclosing snapshot validates its geometry and key
    /// uniqueness before the item can participate in a transition.
    #[inline]
    pub fn new(key: K, geometry: LayoutGeometry) -> Self {
        Self { key, geometry }
    }

    /// Returns this item's stable identity.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Returns this item's resolved rectangle.
    #[inline]
    pub const fn geometry(&self) -> LayoutGeometry {
        self.geometry
    }
}

/// An ordered, validated collection of keyed layout rectangles.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyedLayoutSnapshot<K = String> {
    items: Vec<LayoutItem<K>>,
}

/// Short alias for [`KeyedLayoutSnapshot`].
pub type LayoutSnapshot<K = String> = KeyedLayoutSnapshot<K>;

/// Failure returned while constructing a keyed layout snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyedLayoutError {
    /// One or more items used the same key.
    DuplicateKey,
    /// An optional-item adapter attempted to publish an item without identity.
    MissingKey,
    /// An item's rectangle was invalid.
    Geometry(LayoutGeometryError),
}

impl fmt::Display for KeyedLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey => formatter.write_str("layout item keys must be unique"),
            Self::MissingKey => formatter.write_str("animated layout items require a stable key"),
            Self::Geometry(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for KeyedLayoutError {}

impl From<LayoutGeometryError> for KeyedLayoutError {
    fn from(error: LayoutGeometryError) -> Self {
        Self::Geometry(error)
    }
}

impl<K> KeyedLayoutSnapshot<K>
where
    K: Clone + Eq + std::hash::Hash,
{
    /// Creates a snapshot and rejects duplicate keys or invalid geometry.
    pub fn try_new(
        items: impl IntoIterator<Item = LayoutItem<K>>,
    ) -> Result<Self, KeyedLayoutError> {
        let mut keys = std::collections::HashSet::new();
        let mut validated = Vec::new();
        for item in items {
            item.geometry.validate()?;
            if !keys.insert(item.key.clone()) {
                return Err(KeyedLayoutError::DuplicateKey);
            }
            validated.push(item);
        }
        Ok(Self { items: validated })
    }

    /// Creates a snapshot from an optional-key adapter.
    ///
    /// This is the explicit failure path for a Flex child or list item that
    /// has no stable identity. Callers must choose an identity before asking
    /// the transition engine to animate a replacement or reorder.
    pub fn try_new_optional(
        items: impl IntoIterator<Item = Option<LayoutItem<K>>>,
    ) -> Result<Self, KeyedLayoutError> {
        let items = items
            .into_iter()
            .map(|item| item.ok_or(KeyedLayoutError::MissingKey))
            .collect::<Result<Vec<_>, _>>()?;
        Self::try_new(items)
    }

    /// Returns items in their paint/order sequence.
    #[inline]
    pub fn items(&self) -> &[LayoutItem<K>] {
        &self.items
    }

    /// Consumes the snapshot and returns its ordered items.
    #[inline]
    pub fn into_items(self) -> Vec<LayoutItem<K>> {
        self.items
    }

    /// Returns the item matching `key`, if present.
    #[inline]
    pub fn get(&self, key: &K) -> Option<&LayoutItem<K>> {
        self.items.iter().find(|item| &item.key == key)
    }

    /// Returns the number of keyed items.
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether this snapshot contains no items.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The lifecycle of an item in a keyed layout transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutTransitionPhase {
    /// The item was added and is growing from a collapsed rectangle.
    Entering,
    /// The item exists in both snapshots and follows its keyed target.
    Stable,
    /// The item was removed and is shrinking before it leaves the output.
    Exiting,
}

/// One item emitted by [`KeyedLayoutTransition::advance`].
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutTransitionItem<K = String> {
    key: K,
    geometry: LayoutGeometry,
    phase: LayoutTransitionPhase,
}

impl<K> LayoutTransitionItem<K> {
    /// Returns this item's stable identity.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Returns this item's interpolated geometry.
    #[inline]
    pub const fn geometry(&self) -> LayoutGeometry {
        self.geometry
    }

    /// Returns whether this item is entering, retained, or exiting.
    #[inline]
    pub const fn phase(&self) -> LayoutTransitionPhase {
        self.phase
    }
}

#[derive(Debug)]
struct PlannedLayoutItem<K> {
    key: K,
    from: LayoutGeometry,
    to: LayoutGeometry,
    phase: LayoutTransitionPhase,
}

/// A controller-backed keyed list transition.
///
/// Existing keys interpolate from their current rectangles, new keys enter
/// from a collapsed rectangle at their target position, and removed keys exit
/// to a collapsed rectangle at their last position. Reordering changes output
/// order immediately while each key keeps its own geometry, so state follows
/// identity rather than an index. A new target can be installed during any
/// frame; the already interpolated output becomes the next transition's
/// starting geometry.
#[derive(Debug)]
pub struct KeyedLayoutTransition<K = String> {
    config: LayoutTransitionConfig,
    controller: AnimationController,
    current: Option<Vec<LayoutTransitionItem<K>>>,
    target: Option<KeyedLayoutSnapshot<K>>,
    plan: Option<Vec<PlannedLayoutItem<K>>>,
}

/// Alias for [`KeyedLayoutTransition`].
pub type LayoutSnapshotTransition<K = String> = KeyedLayoutTransition<K>;

impl<K> Default for KeyedLayoutTransition<K>
where
    K: Clone + Eq + std::hash::Hash,
{
    fn default() -> Self {
        Self::new(LayoutTransitionConfig::default())
    }
}

impl<K> KeyedLayoutTransition<K>
where
    K: Clone + Eq + std::hash::Hash,
{
    /// Creates an empty keyed transition with `config`.
    pub fn new(config: LayoutTransitionConfig) -> Self {
        Self::try_new(config).unwrap_or_else(|_| {
            Self::try_new(LayoutTransitionConfig::default())
                .expect("the default layout transition configuration is valid")
        })
    }

    /// Creates an empty keyed transition after validating its configuration.
    pub fn try_new(config: LayoutTransitionConfig) -> Result<Self, LayoutTransitionError> {
        config.validate()?;
        Ok(Self {
            controller: AnimationController::new(config.duration, config.curve.clone()),
            config,
            current: None,
            target: None,
            plan: None,
        })
    }

    /// Returns the latest target snapshot.
    #[inline]
    pub fn target(&self) -> Option<&KeyedLayoutSnapshot<K>> {
        self.target.as_ref()
    }

    /// Returns the latest emitted items, if a target has been installed.
    #[inline]
    pub fn current(&self) -> Option<&[LayoutTransitionItem<K>]> {
        self.current.as_deref()
    }

    /// Returns whether an item transition is in flight.
    #[inline]
    pub fn is_animating(&self) -> bool {
        self.plan.is_some() && self.controller.is_animating()
    }

    /// Installs a target snapshot at `now`.
    pub fn set_target(
        &mut self,
        target: KeyedLayoutSnapshot<K>,
        now: AnimInstant,
    ) -> Result<(), LayoutTransitionError> {
        if self.target.as_ref() == Some(&target) {
            return Ok(());
        }

        let previous = self.advance(now);
        if self.current.is_none() {
            self.current = Some(stable_items(&target));
            self.target = Some(target);
            self.plan = None;
            return Ok(());
        }

        let target_keys = target
            .items
            .iter()
            .map(|item| item.key.clone())
            .collect::<std::collections::HashSet<_>>();
        let mut previous_by_key = previous
            .iter()
            .cloned()
            .map(|item| (item.key.clone(), item))
            .collect::<std::collections::HashMap<_, _>>();
        let mut plan = Vec::with_capacity(target.len() + previous.len());

        for item in &target.items {
            let (from, phase) = match previous_by_key.remove(&item.key) {
                Some(previous) => (previous.geometry, LayoutTransitionPhase::Stable),
                None => (item.geometry.collapsed(), LayoutTransitionPhase::Entering),
            };
            plan.push(PlannedLayoutItem {
                key: item.key.clone(),
                from,
                to: item.geometry,
                phase,
            });
        }

        for item in &previous {
            if !target_keys.contains(&item.key) {
                plan.push(PlannedLayoutItem {
                    key: item.key.clone(),
                    from: item.geometry,
                    to: item.geometry.collapsed(),
                    phase: LayoutTransitionPhase::Exiting,
                });
            }
        }

        self.target = Some(target);
        if plan.is_empty() || !self.config.allows_animation() {
            self.plan = None;
            self.current = Some(stable_items(
                self.target.as_ref().expect("target installed"),
            ));
            self.controller.reset();
            return Ok(());
        }

        self.plan = Some(plan);
        self.current = Some(previous);
        self.controller = AnimationController::new(self.config.duration, self.config.curve.clone());
        self.controller.forward_from_first_tick();
        let _ = self.controller.tick(now);
        Ok(())
    }

    /// Samples all visible keyed items at `now`.
    pub fn advance(&mut self, now: AnimInstant) -> Vec<LayoutTransitionItem<K>> {
        let Some(plan) = self.plan.as_ref() else {
            return self.current.clone().unwrap_or_default();
        };
        if !self.controller.is_animating() {
            let settled = self.target.as_ref().map(stable_items).unwrap_or_default();
            self.current = Some(settled.clone());
            self.plan = None;
            return settled;
        }

        let progress = self.controller.tick(now);
        let output = plan
            .iter()
            .map(|item| LayoutTransitionItem {
                key: item.key.clone(),
                geometry: item.from.interpolate(&item.to, progress).unwrap_or(item.to),
                phase: item.phase,
            })
            .collect::<Vec<_>>();
        self.current = Some(output.clone());

        if !self.controller.is_animating() {
            let settled = self.target.as_ref().map(stable_items).unwrap_or_default();
            self.current = Some(settled.clone());
            self.plan = None;
            return settled;
        }
        output
    }

    /// Settles at the target and removes exiting items.
    pub fn settle(&mut self) {
        if let Some(target) = self.target.as_ref() {
            self.current = Some(stable_items(target));
        }
        self.plan = None;
        self.controller.reset();
    }

    /// Changes the enabled policy and settles at the target when disabled.
    pub fn set_enabled(&mut self, enabled: bool, now: AnimInstant) {
        self.config.enabled = enabled;
        if !enabled {
            let _ = self.advance(now);
            self.settle();
        }
    }

    /// Changes reduced-motion policy and settles at the target when enabled.
    pub fn set_reduced_motion(&mut self, reduced_motion: bool, now: AnimInstant) {
        self.config.reduced_motion = reduced_motion;
        if reduced_motion {
            let _ = self.advance(now);
            self.settle();
        }
    }
}

fn stable_items<K: Clone>(snapshot: &KeyedLayoutSnapshot<K>) -> Vec<LayoutTransitionItem<K>> {
    snapshot
        .items
        .iter()
        .map(|item| LayoutTransitionItem {
            key: item.key.clone(),
            geometry: item.geometry,
            phase: LayoutTransitionPhase::Stable,
        })
        .collect()
}

/// A widget wrapper that opts one child into layout-size transitions.
///
/// The wrapper delegates layout and events to the existing child and uses a
/// bounded canvas scale while the child's resolved size moves. It is an opt-in
/// primitive: ordinary `Row`, `Column`, and `FlexList` widgets retain their
/// immediate behavior, and no parallel `AnimatedRow`/`AnimatedColumn` types
/// are introduced.
pub struct AnimatedLayout<T = aimer_widget::RequiredChild> {
    config: LayoutTransitionConfig,
    child: T,
}

impl AnimatedLayout<aimer_widget::RequiredChild> {
    /// Creates an empty animated-layout builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            config: LayoutTransitionConfig::default(),
            child: aimer_widget::RequiredChild,
        }
    }
}

impl Default for AnimatedLayout<aimer_widget::RequiredChild> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> AnimatedLayout<T> {
    /// Sets the duration, bounded by [`MAX_LAYOUT_TRANSITION_DURATION`].
    #[inline]
    pub fn duration(mut self, duration: Duration) -> Self {
        self.config = self.config.duration(duration);
        self
    }

    /// Sets the easing curve used for layout progress.
    #[inline]
    pub fn curve(mut self, curve: impl Curve) -> Self {
        self.config = self.config.curve(curve);
        self
    }

    /// Enables or disables movement for this wrapper.
    #[inline]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.config = self.config.enabled(enabled);
        self
    }

    /// Enables reduced-motion settling for this wrapper.
    #[inline]
    pub fn reduced_motion(mut self, reduced_motion: bool) -> Self {
        self.config = self.config.reduced_motion(reduced_motion);
        self
    }

    /// Attaches the child and completes the builder.
    #[inline]
    pub fn child<W>(self, child: W) -> AnimatedLayout<W> {
        AnimatedLayout {
            config: self.config,
            child,
        }
    }
}

impl<T: aimer_widget::Widget + 'static> aimer_widget::Widget for AnimatedLayout<T> {
    fn to_element(self, ctx: &aimer_widget::base::BuildContext) -> aimer_widget::AnyElement {
        AnimatedLayoutElement {
            child: self.child.to_element(ctx),
            transition: std::cell::RefCell::new(LayoutTransition::new(self.config)),
            window: ctx.window.clone(),
            last_geometry: std::cell::Cell::new(None),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedLayout"
    }
}

impl<T: aimer_widget::Widget> aimer_widget::PortableWidget for AnimatedLayout<T> {}

struct AnimatedLayoutElement {
    child: aimer_widget::AnyElement,
    transition: std::cell::RefCell<LayoutTransition>,
    window: aimer_widget::base::WindowHandle,
    last_geometry: std::cell::Cell<Option<LayoutGeometry>>,
}

impl AnimatedLayoutElement {
    fn geometry(&self, ctx: &aimer_widget::base::BuildContext, now: AnimInstant) -> LayoutGeometry {
        let natural = self.child.computed_size(ctx);
        let Ok(target) = LayoutGeometry::try_new(0.0, 0.0, natural.width, natural.height) else {
            return LayoutGeometry::try_new(0.0, 0.0, 0.0, 0.0)
                .expect("zero layout geometry is finite");
        };
        let mut transition = self.transition.borrow_mut();
        if transition.target() != Some(target) {
            let _ = transition.set_target(target, now);
        }
        transition.advance(now).unwrap_or(target)
    }
}

impl aimer_widget::Drawable for AnimatedLayoutElement {
    fn update(&self, ctx: &aimer_widget::base::BuildContext) {
        let now = AnimInstant::now();
        let geometry = self.geometry(ctx, now);
        let natural = self.child.computed_size(ctx);
        let geometry_changed = self.last_geometry.replace(Some(geometry)) != Some(geometry);
        let active = self.transition.borrow().is_animating();
        if geometry_changed
            || active
            || !geometry.is_finite()
            || !natural.width.is_finite()
            || !natural.height.is_finite()
        {
            aimer_widget::mark_paint_damage_full();
        }
        let scale_x = if natural.width > 0.0 {
            (geometry.width / natural.width).max(0.0)
        } else {
            1.0
        };
        let scale_y = if natural.height > 0.0 {
            (geometry.height / natural.height).max(0.0)
        } else {
            1.0
        };

        ctx.canvas.save();
        ctx.canvas.scale(scale_x, scale_y);
        self.child.update(ctx);
        ctx.canvas.restore();

        if active {
            request_animation_frame()
        }
    }

    fn can_paint_local_v2(&self, _ctx: &aimer_widget::base::BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &aimer_widget::base::BuildContext) {}

    fn sync_local_v2_state(&self, ctx: &aimer_widget::base::BuildContext) -> bool {
        // The scale is the child node's presentation transform, so the tree
        // damages whatever the child paints; no promise about its paint extent
        // is needed.
        let geometry = self.geometry(ctx, AnimInstant::now());
        let natural = self.child.computed_size(ctx);
        let scale_x = if natural.width > 0.0 {
            (geometry.width / natural.width).max(0.0)
        } else {
            1.0
        };
        let scale_y = if natural.height > 0.0 {
            (geometry.height / natural.height).max(0.0)
        } else {
            1.0
        };
        if !scale_x.is_finite() || !scale_y.is_finite() {
            return false;
        }

        if self.transition.borrow().is_animating() {
            request_animation_frame();
        }
        ctx.set_local_v2_child_presentation_at(0, aimer_canvas::Mat3::scale(scale_x, scale_y), 1.0)
    }

    fn draw_local_v2_compatibility(&self, ctx: &aimer_widget::base::BuildContext) {
        self.child.update(ctx);
        if self.transition.borrow().is_animating() {
            request_animation_frame();
        }
    }
}

impl aimer_widget::VisitorElement for AnimatedLayoutElement {
    fn debug_name(&self) -> &'static str {
        "AnimatedLayoutElement"
    }

    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn aimer_widget::Element)) {
        visitor(self.child.as_ref());
    }
}

impl aimer_widget::EventElement for AnimatedLayoutElement {
    fn on_event(&self, event: &aimer_events::element::ElementEvent) -> aimer_widget::EventResult {
        self.child.on_event(event)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn aimer_widget::Element)) {
        visitor(self.child.as_ref());
    }
}

impl aimer_widget::Rebuildable for AnimatedLayoutElement {
    fn rebuild_if_dirty(&self, ctx: &aimer_widget::base::BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl aimer_widget::LayoutElement for AnimatedLayoutElement {
    fn pos(&self) -> Option<aimer_attribute::position::Vec2d> {
        self.child.pos()
    }

    fn computed_size(
        &self,
        ctx: &aimer_widget::base::BuildContext,
    ) -> aimer_attribute::size::ResolvedSize {
        let geometry = self.geometry(ctx, AnimInstant::now());
        aimer_attribute::size::ResolvedSize {
            width: geometry.width,
            height: geometry.height,
        }
    }

    fn content_size(
        &self,
        ctx: &aimer_widget::base::BuildContext,
    ) -> aimer_attribute::size::ResolvedSize {
        self.computed_size(ctx)
    }

    fn get_size_from_child(&self) -> Option<aimer_attribute::size::Size> {
        self.child.get_size_from_child()
    }

    fn is_layout_stable(&self) -> bool {
        false
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn geometry_interpolation_is_componentwise_and_finite() {
        let from = LayoutGeometry::try_new(10.0, 20.0, 100.0, 40.0).unwrap();
        let to = LayoutGeometry::try_new(30.0, 60.0, 200.0, 80.0).unwrap();

        assert_eq!(
            from.interpolate(&to, 0.5).unwrap(),
            LayoutGeometry::try_new(20.0, 40.0, 150.0, 60.0).unwrap()
        );
        assert!(LayoutGeometry::try_new(f32::NAN, 0.0, 1.0, 1.0).is_err());
        assert!(LayoutGeometry::try_new(0.0, 0.0, -1.0, 1.0).is_err());
        assert!(LayoutGeometry::try_new(f32::INFINITY, 0.0, 1.0, 1.0).is_err());
        assert!(from.interpolate(&to, f32::NAN).is_err());

        let opposite_extremes = LayoutGeometry::try_new(f32::MAX, 0.0, 1.0, 1.0).unwrap();
        let negative_extreme = LayoutGeometry::try_new(-f32::MAX, 0.0, 1.0, 1.0).unwrap();
        assert_eq!(
            opposite_extremes
                .interpolate(&negative_extreme, 0.5)
                .unwrap()
                .x,
            0.0
        );

        assert_eq!(
            LayoutTransitionConfig::default()
                .try_duration(MAX_LAYOUT_TRANSITION_DURATION + Duration::from_nanos(1),),
            Err(LayoutTransitionError::DurationTooLong)
        );
        assert_eq!(
            LayoutTransitionConfig::default().try_curve(curves::cubic_bezier(
                f32::NAN,
                0.0,
                1.0,
                1.0,
            )),
            Err(LayoutTransitionError::NonFiniteCurve)
        );
    }

    #[test]
    fn first_zero_disabled_and_reduced_motion_settle_at_final_geometry() {
        let now = AnimInstant::now();
        let first = LayoutGeometry::try_new(0.0, 0.0, 20.0, 20.0).unwrap();
        let final_geometry = LayoutGeometry::try_new(40.0, 12.0, 80.0, 44.0).unwrap();

        let mut first_layout = LayoutTransition::new(LayoutTransitionConfig::default());
        first_layout.set_target(first, now).unwrap();
        assert_eq!(first_layout.current(), Some(first));
        assert!(!first_layout.is_animating());

        let mut zero_duration =
            LayoutTransition::new(LayoutTransitionConfig::default().duration(Duration::ZERO));
        zero_duration.set_target(first, now).unwrap();
        zero_duration.set_target(final_geometry, now).unwrap();
        assert_eq!(zero_duration.current(), Some(final_geometry));
        assert!(!zero_duration.is_animating());

        let mut disabled = LayoutTransition::new(LayoutTransitionConfig::default().enabled(false));
        disabled.set_target(first, now).unwrap();
        disabled.set_target(final_geometry, now).unwrap();
        assert_eq!(disabled.current(), Some(final_geometry));
        assert!(!disabled.is_animating());

        let mut reduced =
            LayoutTransition::new(LayoutTransitionConfig::default().reduced_motion(true));
        reduced.set_target(first, now).unwrap();
        reduced.set_target(final_geometry, now).unwrap();
        assert_eq!(reduced.current(), Some(final_geometry));
        assert!(!reduced.is_animating());
    }

    #[test]
    fn active_transition_retargets_from_the_sampled_geometry() {
        let now = AnimInstant::now();
        let initial = LayoutGeometry::try_new(0.0, 0.0, 20.0, 20.0).unwrap();
        let first_target = LayoutGeometry::try_new(100.0, 0.0, 20.0, 20.0).unwrap();
        let second_target = LayoutGeometry::try_new(200.0, 0.0, 20.0, 20.0).unwrap();
        let config = LayoutTransitionConfig::default()
            .duration(Duration::from_millis(100))
            .curve(curves::LINEAR);
        let mut transition = LayoutTransition::new(config);

        transition.set_target(initial, now).unwrap();
        transition.set_target(first_target, now).unwrap();
        let halfway = now + Duration::from_millis(50);
        let sampled = transition.advance(halfway).unwrap();
        assert_eq!(sampled.x, 50.0);

        transition.set_target(second_target, halfway).unwrap();
        assert_eq!(transition.current().unwrap().x, 50.0);
        assert_eq!(
            transition
                .advance(halfway + Duration::from_millis(50))
                .unwrap()
                .x,
            125.0
        );
        assert_eq!(
            transition.advance(halfway + Duration::from_millis(100)),
            Some(second_target)
        );
    }

    #[test]
    fn keyed_transition_preserves_identity_across_insert_remove_and_reorder() {
        let now = AnimInstant::now();
        let a = LayoutGeometry::try_new(0.0, 0.0, 20.0, 20.0).unwrap();
        let b = LayoutGeometry::try_new(100.0, 0.0, 20.0, 20.0).unwrap();
        let b_reordered = LayoutGeometry::try_new(0.0, 0.0, 20.0, 20.0).unwrap();
        let c = LayoutGeometry::try_new(100.0, 0.0, 20.0, 20.0).unwrap();
        let config = LayoutTransitionConfig::default()
            .duration(Duration::from_millis(100))
            .curve(curves::LINEAR);
        let mut transition = KeyedLayoutTransition::new(config);

        transition
            .set_target(
                KeyedLayoutSnapshot::try_new(vec![
                    LayoutItem::new("a", a),
                    LayoutItem::new("b", b),
                ])
                .unwrap(),
                now,
            )
            .unwrap();
        transition
            .set_target(
                KeyedLayoutSnapshot::try_new(vec![
                    LayoutItem::new("b", b_reordered),
                    LayoutItem::new("c", c),
                ])
                .unwrap(),
                now,
            )
            .unwrap();

        let items = transition.advance(now + Duration::from_millis(50));
        assert_eq!(
            items.iter().map(|item| *item.key()).collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );
        assert_eq!(items[0].geometry().x, 50.0);
        assert_eq!(items[1].geometry().width, 10.0);
        assert_eq!(items[2].geometry().width, 10.0);
        assert_eq!(items[0].phase(), LayoutTransitionPhase::Stable);
        assert_eq!(items[1].phase(), LayoutTransitionPhase::Entering);
        assert_eq!(items[2].phase(), LayoutTransitionPhase::Exiting);

        let settled = transition.advance(now + Duration::from_millis(100));
        assert_eq!(
            settled.iter().map(|item| *item.key()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert!(!transition.is_animating());
    }

    #[test]
    fn keyed_snapshot_rejects_duplicate_and_missing_identity() {
        let geometry = LayoutGeometry::try_new(0.0, 0.0, 10.0, 10.0).unwrap();
        assert_eq!(
            KeyedLayoutSnapshot::try_new(vec![
                LayoutItem::new("same", geometry),
                LayoutItem::new("same", geometry),
            ]),
            Err(KeyedLayoutError::DuplicateKey)
        );

        let missing: Vec<Option<LayoutItem<&str>>> = vec![None];
        assert_eq!(
            KeyedLayoutSnapshot::try_new_optional(missing),
            Err(KeyedLayoutError::MissingKey)
        );
    }
}
