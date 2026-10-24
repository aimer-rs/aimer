use core::fmt;
use std::sync::Arc;

use crate::ShapeBounds;
use crate::hit_test::ShapeHitTest;
use crate::{FillStyle, Point, StrokeStyle};

/// Extra limits applied while constructing a path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeLimits {
    /// Maximum number of commands, including `MoveTo` and `Close`.
    pub max_commands: usize,
    /// Maximum number of contours.
    pub max_contours: usize,
    /// Maximum absolute coordinate, radius, or extent.
    pub max_abs_coordinate: f32,
}

impl Default for ShapeLimits {
    fn default() -> Self {
        Self {
            max_commands: crate::DEFAULT_MAX_COMMANDS,
            max_contours: crate::DEFAULT_MAX_CONTOURS,
            max_abs_coordinate: crate::DEFAULT_MAX_ABS_COORDINATE,
        }
    }
}

impl ShapeLimits {
    /// Validates that the configured limits can bound an allocation.
    ///
    /// Explicit values may tighten the crate defaults but may not enlarge the
    /// renderer-safe maxima. This keeps iterator-based construction bounded
    /// even when input is malformed or unexpectedly long.
    pub fn validate(self) -> Result<Self, ShapeError> {
        if self.max_commands == 0 {
            return Err(ShapeError::InvalidLimit("max_commands"));
        }
        if self.max_commands > crate::DEFAULT_MAX_COMMANDS {
            return Err(ShapeError::InvalidLimit("max_commands"));
        }
        if self.max_contours == 0 {
            return Err(ShapeError::InvalidLimit("max_contours"));
        }
        if self.max_contours > crate::DEFAULT_MAX_CONTOURS {
            return Err(ShapeError::InvalidLimit("max_contours"));
        }
        if !self.max_abs_coordinate.is_finite() || self.max_abs_coordinate <= 0.0 {
            return Err(ShapeError::InvalidLimit("max_abs_coordinate"));
        }
        if self.max_abs_coordinate > crate::DEFAULT_MAX_ABS_COORDINATE {
            return Err(ShapeError::InvalidLimit("max_abs_coordinate"));
        }
        Ok(self)
    }
}

/// An error from an arc or ellipse command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArcError {
    /// A radius was non-finite, zero, or negative.
    InvalidRadius,
    /// The sweep was non-finite or zero.
    InvalidSweep,
    /// The sweep exceeded one full turn.
    SweepTooLarge,
    /// The arc's mathematical start did not join the current point.
    StartMismatch,
}

/// Errors returned by path construction and validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapeError {
    /// No command was supplied.
    EmptyPath,
    /// A contour contained only a `MoveTo`.
    EmptyContour { command_index: usize },
    /// A drawing command appeared before a `MoveTo`.
    CommandBeforeMove { command_index: usize },
    /// A `Close` appeared without an active contour.
    CloseWithoutContour { command_index: usize },
    /// A second `Close` appeared for the same contour.
    RepeatedClose { command_index: usize },
    /// A numeric value was not finite.
    NonFinite {
        command_index: usize,
        field: &'static str,
    },
    /// A finite coordinate exceeded the configured safety limit.
    CoordinateOutOfRange {
        command_index: usize,
        field: &'static str,
    },
    /// A segment had no length and no curve control movement.
    ZeroLengthSegment { command_index: usize },
    /// The command count exceeded the configured limit.
    TooManyCommands { limit: usize },
    /// The contour count exceeded the configured limit.
    TooManyContours { limit: usize },
    /// A configured limit could not provide a bounded path.
    InvalidLimit(&'static str),
    /// An arc or ellipse was malformed.
    InvalidArc {
        command_index: usize,
        reason: ArcError,
    },
    /// A flattening tolerance was invalid.
    InvalidTolerance,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => f.write_str("shape path must contain at least one segment"),
            Self::EmptyContour { command_index } => {
                write!(f, "shape contour at command {command_index} is empty")
            }
            Self::CommandBeforeMove { command_index } => {
                write!(f, "shape command {command_index} appears before MoveTo")
            }
            Self::CloseWithoutContour { command_index } => {
                write!(
                    f,
                    "shape Close at command {command_index} has no active contour"
                )
            }
            Self::RepeatedClose { command_index } => {
                write!(
                    f,
                    "shape Close at command {command_index} repeats a closed contour"
                )
            }
            Self::NonFinite {
                command_index,
                field,
            } => write!(f, "shape command {command_index} has a non-finite {field}"),
            Self::CoordinateOutOfRange {
                command_index,
                field,
            } => write!(
                f,
                "shape command {command_index} exceeds the {field} coordinate limit"
            ),
            Self::ZeroLengthSegment { command_index } => {
                write!(f, "shape command {command_index} is a zero-length segment")
            }
            Self::TooManyCommands { limit } => {
                write!(f, "shape path exceeds the {limit}-command limit")
            }
            Self::TooManyContours { limit } => {
                write!(f, "shape path exceeds the {limit}-contour limit")
            }
            Self::InvalidLimit(name) => write!(f, "shape limit {name} is invalid"),
            Self::InvalidArc {
                command_index,
                reason,
            } => write!(
                f,
                "shape arc at command {command_index} is invalid: {reason:?}"
            ),
            Self::InvalidTolerance => {
                f.write_str("shape flattening tolerance must be finite and positive")
            }
        }
    }
}

impl std::error::Error for ShapeError {}

/// A finite path command in local coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeCommand {
    /// Starts a contour.
    MoveTo { x: f32, y: f32 },
    /// Adds a straight segment.
    LineTo { x: f32, y: f32 },
    /// Adds a quadratic Bézier segment.
    QuadraticTo {
        control_x: f32,
        control_y: f32,
        x: f32,
        y: f32,
    },
    /// Adds a cubic Bézier segment.
    CubicTo {
        control1_x: f32,
        control1_y: f32,
        control2_x: f32,
        control2_y: f32,
        x: f32,
        y: f32,
    },
    /// Adds an elliptical arc from the given angular start.
    ArcTo {
        center_x: f32,
        center_y: f32,
        radius_x: f32,
        radius_y: f32,
        start_angle: f32,
        sweep_angle: f32,
        rotation: f32,
    },
    /// Closes the active contour.
    Close,
}

impl ShapeCommand {
    /// Creates a `MoveTo` command.
    #[inline]
    pub const fn move_to(x: f32, y: f32) -> Self {
        Self::MoveTo { x, y }
    }

    /// Creates a `LineTo` command.
    #[inline]
    pub const fn line_to(x: f32, y: f32) -> Self {
        Self::LineTo { x, y }
    }

    /// Creates a quadratic command.
    #[inline]
    pub const fn quadratic_to(control_x: f32, control_y: f32, x: f32, y: f32) -> Self {
        Self::QuadraticTo {
            control_x,
            control_y,
            x,
            y,
        }
    }

    /// Creates a cubic command.
    #[inline]
    pub const fn cubic_to(
        control1_x: f32,
        control1_y: f32,
        control2_x: f32,
        control2_y: f32,
        x: f32,
        y: f32,
    ) -> Self {
        Self::CubicTo {
            control1_x,
            control1_y,
            control2_x,
            control2_y,
            x,
            y,
        }
    }

    /// Creates an arc command.
    #[inline]
    pub const fn arc_to(
        center_x: f32,
        center_y: f32,
        radius_x: f32,
        radius_y: f32,
        start_angle: f32,
        sweep_angle: f32,
        rotation: f32,
    ) -> Self {
        Self::ArcTo {
            center_x,
            center_y,
            radius_x,
            radius_y,
            start_angle,
            sweep_angle,
            rotation,
        }
    }

    /// Returns the command tag used by the deterministic encoding.
    #[inline]
    pub const fn tag(self) -> u8 {
        match self {
            Self::MoveTo { .. } => 0,
            Self::LineTo { .. } => 1,
            Self::QuadraticTo { .. } => 2,
            Self::CubicTo { .. } => 3,
            Self::ArcTo { .. } => 4,
            Self::Close => 5,
        }
    }
}

/// One flattened contour used by deterministic hit testing and adapters.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapePolyline {
    /// Points in contour order. Consecutive points form straight segments.
    pub points: Arc<[Point]>,
    /// Whether the last point is joined to the first point.
    pub closed: bool,
}

/// A validated, immutable path and its local bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapePath {
    commands: Arc<[ShapeCommand]>,
    bounds: ShapeBounds,
    limits: ShapeLimits,
    contour_count: usize,
    closed_contours: usize,
    id: ShapePathId,
}

impl ShapePath {
    /// Starts a path builder with the default safety limits.
    #[inline]
    pub fn builder() -> ShapePathBuilder {
        ShapePathBuilder::new()
    }

    /// Validates commands using the default safety limits.
    pub fn try_from_commands(
        commands: impl IntoIterator<Item = ShapeCommand>,
    ) -> Result<Self, ShapeError> {
        Self::try_from_commands_with_limits(commands, ShapeLimits::default())
    }

    /// Validates commands using explicit safety limits.
    pub fn try_from_commands_with_limits(
        commands: impl IntoIterator<Item = ShapeCommand>,
        limits: ShapeLimits,
    ) -> Result<Self, ShapeError> {
        validate_limits(limits)?;
        let mut validated_input = Vec::new();
        for command in commands {
            if validated_input.len() >= limits.max_commands {
                return Err(ShapeError::TooManyCommands {
                    limit: limits.max_commands,
                });
            }
            validated_input.push(command);
        }
        let commands = validated_input;
        if commands.is_empty() {
            return Err(ShapeError::EmptyPath);
        }

        let mut bounds = BoundsAccumulator::default();
        let mut current = None;
        let mut start = None;
        let mut active = false;
        let mut closed = false;
        let mut segments = 0usize;
        let mut contour_count = 0usize;
        let mut closed_contours = 0usize;

        for (command_index, command) in commands.iter().copied().enumerate() {
            match command {
                ShapeCommand::MoveTo { x, y } => {
                    check_xy(x, y, command_index, "coordinate", limits)?;
                    if active && segments == 0 {
                        return Err(ShapeError::EmptyContour { command_index });
                    }
                    contour_count += 1;
                    if contour_count > limits.max_contours {
                        return Err(ShapeError::TooManyContours {
                            limit: limits.max_contours,
                        });
                    }
                    let point = Point::new(x, y);
                    bounds.include(point);
                    current = Some(point);
                    start = Some(point);
                    active = true;
                    closed = false;
                    segments = 0;
                }
                ShapeCommand::LineTo { x, y } => {
                    let previous = if active {
                        current.expect("an active contour has a current point")
                    } else {
                        return Err(ShapeError::CommandBeforeMove { command_index });
                    };
                    check_xy(x, y, command_index, "coordinate", limits)?;
                    let point = Point::new(x, y);
                    if same_point(previous, point) {
                        return Err(ShapeError::ZeroLengthSegment { command_index });
                    }
                    bounds.include(point);
                    current = Some(point);
                    segments += 1;
                }
                ShapeCommand::QuadraticTo {
                    control_x,
                    control_y,
                    x,
                    y,
                } => {
                    let previous = if active {
                        current.expect("an active contour has a current point")
                    } else {
                        return Err(ShapeError::CommandBeforeMove { command_index });
                    };
                    check_xy(control_x, control_y, command_index, "control", limits)?;
                    check_xy(x, y, command_index, "coordinate", limits)?;
                    let control = Point::new(control_x, control_y);
                    let point = Point::new(x, y);
                    if same_point(previous, point) && same_point(previous, control) {
                        return Err(ShapeError::ZeroLengthSegment { command_index });
                    }
                    include_quadratic_bounds(&mut bounds, previous, control, point);
                    current = Some(point);
                    segments += 1;
                }
                ShapeCommand::CubicTo {
                    control1_x,
                    control1_y,
                    control2_x,
                    control2_y,
                    x,
                    y,
                } => {
                    let previous = if active {
                        current.expect("an active contour has a current point")
                    } else {
                        return Err(ShapeError::CommandBeforeMove { command_index });
                    };
                    check_xy(control1_x, control1_y, command_index, "control1", limits)?;
                    check_xy(control2_x, control2_y, command_index, "control2", limits)?;
                    check_xy(x, y, command_index, "coordinate", limits)?;
                    let control1 = Point::new(control1_x, control1_y);
                    let control2 = Point::new(control2_x, control2_y);
                    let point = Point::new(x, y);
                    if same_point(previous, point)
                        && same_point(previous, control1)
                        && same_point(previous, control2)
                    {
                        return Err(ShapeError::ZeroLengthSegment { command_index });
                    }
                    include_cubic_bounds(&mut bounds, previous, control1, control2, point);
                    current = Some(point);
                    segments += 1;
                }
                ShapeCommand::ArcTo {
                    center_x,
                    center_y,
                    radius_x,
                    radius_y,
                    start_angle,
                    sweep_angle,
                    rotation,
                } => {
                    let previous = if active {
                        current.expect("an active contour has a current point")
                    } else {
                        return Err(ShapeError::CommandBeforeMove { command_index });
                    };
                    check_xy(center_x, center_y, command_index, "center", limits)?;
                    check_xy(radius_x, radius_y, command_index, "radius", limits)?;
                    check_finite(start_angle, command_index, "start_angle")?;
                    check_finite(sweep_angle, command_index, "sweep_angle")?;
                    check_finite(rotation, command_index, "rotation")?;
                    if radius_x <= 0.0 || radius_y <= 0.0 {
                        return Err(ShapeError::InvalidArc {
                            command_index,
                            reason: ArcError::InvalidRadius,
                        });
                    }
                    if sweep_angle.abs() <= f32::EPSILON {
                        return Err(ShapeError::InvalidArc {
                            command_index,
                            reason: ArcError::InvalidSweep,
                        });
                    }
                    if sweep_angle.abs() > core::f32::consts::TAU + 1.0e-4 {
                        return Err(ShapeError::InvalidArc {
                            command_index,
                            reason: ArcError::SweepTooLarge,
                        });
                    }
                    let arc_start = arc_point(
                        center_x,
                        center_y,
                        radius_x,
                        radius_y,
                        start_angle,
                        rotation,
                    );
                    if !previous.approx_eq(arc_start) {
                        return Err(ShapeError::InvalidArc {
                            command_index,
                            reason: ArcError::StartMismatch,
                        });
                    }
                    let arc_extent = arc_bounds(
                        center_x,
                        center_y,
                        radius_x,
                        radius_y,
                        start_angle,
                        sweep_angle,
                        rotation,
                    )
                    .ok_or(ShapeError::NonFinite {
                        command_index,
                        field: "extent",
                    })?;
                    if !bounds_within_limit(arc_extent, limits.max_abs_coordinate) {
                        return Err(ShapeError::CoordinateOutOfRange {
                            command_index,
                            field: "extent",
                        });
                    }
                    include_bounds(&mut bounds, arc_extent);
                    current = Some(arc_point(
                        center_x,
                        center_y,
                        radius_x,
                        radius_y,
                        start_angle + sweep_angle,
                        rotation,
                    ));
                    segments += 1;
                }
                ShapeCommand::Close => {
                    let contour_start =
                        start.ok_or(ShapeError::CloseWithoutContour { command_index })?;
                    if !active {
                        return Err(ShapeError::RepeatedClose { command_index });
                    }
                    if segments == 0 {
                        return Err(ShapeError::EmptyContour { command_index });
                    }
                    bounds.include(contour_start);
                    current = Some(contour_start);
                    active = false;
                    closed = true;
                    closed_contours += 1;
                }
            }

            if !active && !matches!(command, ShapeCommand::Close) {
                closed = false;
            }
        }

        if active && segments == 0 {
            return Err(ShapeError::EmptyContour {
                command_index: commands.len(),
            });
        }
        if !bounds.has_value {
            return Err(ShapeError::EmptyPath);
        }
        let _ = (closed, current);
        let commands: Arc<[ShapeCommand]> = commands.into();
        let id = compute_id(&commands);
        Ok(Self {
            commands,
            bounds: bounds.finish().expect("a validated path has bounds"),
            limits,
            contour_count,
            closed_contours,
            id,
        })
    }

    /// Returns the immutable validated command sequence.
    #[inline]
    pub fn commands(&self) -> &[ShapeCommand] {
        &self.commands
    }

    /// Returns the number of commands.
    #[inline]
    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    /// Returns the number of contours.
    #[inline]
    pub const fn contour_count(&self) -> usize {
        self.contour_count
    }

    /// Returns whether every contour is explicitly closed.
    #[inline]
    pub const fn is_closed(&self) -> bool {
        self.contour_count == self.closed_contours
    }

    /// Returns the limits used when validating this path.
    #[inline]
    pub const fn limits(&self) -> ShapeLimits {
        self.limits
    }

    /// Returns the exact axis-aligned local bounds of the line, curve, and arc geometry.
    #[inline]
    pub const fn bounds(&self) -> ShapeBounds {
        self.bounds
    }

    /// Returns bounds expanded by the visible stroke footprint.
    pub fn paint_bounds(&self, stroke: Option<&StrokeStyle>) -> Option<ShapeBounds> {
        let Some(stroke) = stroke else {
            return Some(self.bounds);
        };
        stroke.validate().ok()?;
        let join_factor = match stroke.line_join {
            crate::LineJoin::Miter | crate::LineJoin::MiterClip => stroke.miter_limit,
            crate::LineJoin::Round | crate::LineJoin::Bevel => 1.0,
        };
        self.bounds.expand(stroke.width * 0.5 * join_factor)
    }

    /// Returns a stable 64-bit identity derived from the canonical encoding.
    #[inline]
    pub fn id(&self) -> ShapePathId {
        self.id
    }

    /// Encodes commands in a versioned, little-endian, deterministic format.
    ///
    /// Negative zero is canonicalized to positive zero so equivalent finite
    /// paths do not produce different cache identities. The encoding contains
    /// geometry only; paint and transforms belong to the draw request key.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + self.commands.len() * 28);
        bytes.extend_from_slice(b"ASHP");
        bytes.push(1);
        bytes.extend_from_slice(&(self.commands.len() as u32).to_le_bytes());
        for command in self.commands.iter().copied() {
            bytes.push(command.tag());
            match command {
                ShapeCommand::MoveTo { x, y } | ShapeCommand::LineTo { x, y } => {
                    push_f32(&mut bytes, x);
                    push_f32(&mut bytes, y);
                }
                ShapeCommand::QuadraticTo {
                    control_x,
                    control_y,
                    x,
                    y,
                } => {
                    push_f32(&mut bytes, control_x);
                    push_f32(&mut bytes, control_y);
                    push_f32(&mut bytes, x);
                    push_f32(&mut bytes, y);
                }
                ShapeCommand::CubicTo {
                    control1_x,
                    control1_y,
                    control2_x,
                    control2_y,
                    x,
                    y,
                } => {
                    push_f32(&mut bytes, control1_x);
                    push_f32(&mut bytes, control1_y);
                    push_f32(&mut bytes, control2_x);
                    push_f32(&mut bytes, control2_y);
                    push_f32(&mut bytes, x);
                    push_f32(&mut bytes, y);
                }
                ShapeCommand::ArcTo {
                    center_x,
                    center_y,
                    radius_x,
                    radius_y,
                    start_angle,
                    sweep_angle,
                    rotation,
                } => {
                    push_f32(&mut bytes, center_x);
                    push_f32(&mut bytes, center_y);
                    push_f32(&mut bytes, radius_x);
                    push_f32(&mut bytes, radius_y);
                    push_f32(&mut bytes, start_angle);
                    push_f32(&mut bytes, sweep_angle);
                    push_f32(&mut bytes, rotation);
                }
                ShapeCommand::Close => {}
            }
        }
        bytes
    }

    /// Alias for [`Self::encode`].
    #[inline]
    pub fn encoded(&self) -> Vec<u8> {
        self.encode()
    }

    /// Flattens curves into bounded line segments for adapters and hit testing.
    pub fn flattened(&self, tolerance: f32) -> Result<Vec<ShapePolyline>, ShapeError> {
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(ShapeError::InvalidTolerance);
        }
        let mut result = Vec::new();
        let mut points = Vec::new();
        let mut current = None;
        let mut start = None;
        let mut closed = false;
        for command in self.commands.iter().copied() {
            match command {
                ShapeCommand::MoveTo { x, y } => {
                    push_polyline(&mut result, &mut points, closed);
                    let point = Point::new(x, y);
                    points.push(point);
                    current = Some(point);
                    start = Some(point);
                    closed = false;
                }
                ShapeCommand::LineTo { x, y } => {
                    let point = Point::new(x, y);
                    points.push(point);
                    current = Some(point);
                }
                ShapeCommand::QuadraticTo {
                    control_x,
                    control_y,
                    x,
                    y,
                } => {
                    let Some(previous) = current else { continue };
                    let control = Point::new(control_x, control_y);
                    let point = Point::new(x, y);
                    let count = curve_steps(
                        previous.distance_squared(control).sqrt()
                            + control.distance_squared(point).sqrt(),
                        tolerance,
                        128,
                    );
                    for index in 1..=count {
                        let t = index as f32 / count as f32;
                        points.push(quadratic_point(previous, control, point, t));
                    }
                    current = Some(point);
                }
                ShapeCommand::CubicTo {
                    control1_x,
                    control1_y,
                    control2_x,
                    control2_y,
                    x,
                    y,
                } => {
                    let Some(previous) = current else { continue };
                    let control1 = Point::new(control1_x, control1_y);
                    let control2 = Point::new(control2_x, control2_y);
                    let point = Point::new(x, y);
                    let count = curve_steps(
                        previous.distance_squared(control1).sqrt()
                            + control1.distance_squared(control2).sqrt()
                            + control2.distance_squared(point).sqrt(),
                        tolerance,
                        192,
                    );
                    for index in 1..=count {
                        let t = index as f32 / count as f32;
                        points.push(cubic_point(previous, control1, control2, point, t));
                    }
                    current = Some(point);
                }
                ShapeCommand::ArcTo {
                    center_x,
                    center_y,
                    radius_x,
                    radius_y,
                    start_angle,
                    sweep_angle,
                    rotation,
                } => {
                    let count =
                        curve_steps(sweep_angle.abs() * radius_x.max(radius_y), tolerance, 256);
                    for index in 1..=count {
                        let t = index as f32 / count as f32;
                        points.push(arc_point(
                            center_x,
                            center_y,
                            radius_x,
                            radius_y,
                            start_angle + sweep_angle * t,
                            rotation,
                        ));
                    }
                    current = Some(arc_point(
                        center_x,
                        center_y,
                        radius_x,
                        radius_y,
                        start_angle + sweep_angle,
                        rotation,
                    ));
                }
                ShapeCommand::Close => {
                    if let Some(contour_start) = start
                        && points
                            .last()
                            .is_none_or(|last| !last.approx_eq(contour_start))
                    {
                        points.push(contour_start);
                    }
                    current = start;
                    closed = true;
                }
            }
        }
        push_polyline(&mut result, &mut points, closed);
        Ok(result)
    }

    /// Tests a point using a geometric hit-test policy and optional paints.
    #[inline]
    pub fn hit_test(
        &self,
        point: Point,
        policy: ShapeHitTest,
        fill: Option<&FillStyle>,
        stroke: Option<&StrokeStyle>,
    ) -> bool {
        policy.contains(self, point, fill, stroke)
    }
}

/// A compact identity for a validated path's canonical command encoding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ShapePathId(pub u64);

/// A chainable path builder. Invalid calls are retained as a typed error and
/// returned by [`ShapePathBuilder::build`]; the `try_*` methods expose the same
/// validation immediately for callers that prefer explicit error handling.
#[derive(Clone, Debug)]
pub struct ShapePathBuilder {
    commands: Vec<ShapeCommand>,
    limits: ShapeLimits,
    error: Option<ShapeError>,
    current: Option<Point>,
    start: Option<Point>,
    contour_has_segment: bool,
}

impl ShapePathBuilder {
    /// Creates a builder with default limits.
    #[inline]
    pub fn new() -> Self {
        Self::with_limits(ShapeLimits::default())
    }

    /// Creates a builder with explicit bounded limits.
    pub fn with_limits(limits: ShapeLimits) -> Self {
        let error = limits.validate().err();
        Self {
            commands: Vec::new(),
            limits,
            error,
            current: None,
            start: None,
            contour_has_segment: false,
        }
    }

    /// Appends a move command, retaining any validation error until `build`.
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        if let Err(error) = self.try_move_to(x, y) {
            self.record_error(error);
        }
        self
    }

    /// Appends a line command, retaining any validation error until `build`.
    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        if let Err(error) = self.try_line_to(x, y) {
            self.record_error(error);
        }
        self
    }

    /// Appends a quadratic command, retaining any validation error until `build`.
    pub fn quadratic_to(mut self, control_x: f32, control_y: f32, x: f32, y: f32) -> Self {
        if let Err(error) = self.try_quadratic_to(control_x, control_y, x, y) {
            self.record_error(error);
        }
        self
    }

    /// Appends a cubic command, retaining any validation error until `build`.
    pub fn cubic_to(
        mut self,
        control1_x: f32,
        control1_y: f32,
        control2_x: f32,
        control2_y: f32,
        x: f32,
        y: f32,
    ) -> Self {
        if let Err(error) = self.try_cubic_to(control1_x, control1_y, control2_x, control2_y, x, y)
        {
            self.record_error(error);
        }
        self
    }

    /// Appends an elliptical arc, adding a finite joining line when needed.
    #[allow(clippy::too_many_arguments)]
    pub fn arc(
        mut self,
        center_x: f32,
        center_y: f32,
        radius_x: f32,
        radius_y: f32,
        start_angle: f32,
        sweep_angle: f32,
        rotation: f32,
    ) -> Self {
        if let Err(error) = self.try_arc(
            center_x,
            center_y,
            radius_x,
            radius_y,
            start_angle,
            sweep_angle,
            rotation,
        ) {
            self.record_error(error);
        }
        self
    }

    /// Appends a complete rotated ellipse as a closed contour.
    pub fn ellipse(
        mut self,
        center_x: f32,
        center_y: f32,
        radius_x: f32,
        radius_y: f32,
        rotation: f32,
    ) -> Self {
        let result = self.try_ellipse(center_x, center_y, radius_x, radius_y, rotation);
        if let Err(error) = result {
            self.record_error(error);
        }
        self
    }

    /// Appends a close command, retaining any validation error until `build`.
    pub fn close(mut self) -> Self {
        if let Err(error) = self.try_close() {
            self.record_error(error);
        }
        self
    }

    /// Explicitly appends a move command and returns its error immediately.
    pub fn try_move_to(&mut self, x: f32, y: f32) -> Result<(), ShapeError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let index = self.commands.len();
        if self.current.is_some() && !self.contour_has_segment {
            return Err(ShapeError::EmptyContour {
                command_index: index,
            });
        }
        validate_builder_xy(x, y, index, self.limits)?;
        self.push(ShapeCommand::MoveTo { x, y })?;
        let point = Point::new(x, y);
        self.current = Some(point);
        self.start = Some(point);
        self.contour_has_segment = false;
        Ok(())
    }

    /// Explicitly appends a line command and returns its error immediately.
    pub fn try_line_to(&mut self, x: f32, y: f32) -> Result<(), ShapeError> {
        let previous = self.current.ok_or(ShapeError::CommandBeforeMove {
            command_index: self.commands.len(),
        })?;
        let index = self.commands.len();
        validate_builder_xy(x, y, index, self.limits)?;
        let point = Point::new(x, y);
        if same_point(previous, point) {
            return Err(ShapeError::ZeroLengthSegment {
                command_index: index,
            });
        }
        self.push(ShapeCommand::LineTo { x, y })?;
        self.current = Some(point);
        self.contour_has_segment = true;
        Ok(())
    }

    /// Explicitly appends a quadratic command and returns its error immediately.
    pub fn try_quadratic_to(
        &mut self,
        control_x: f32,
        control_y: f32,
        x: f32,
        y: f32,
    ) -> Result<(), ShapeError> {
        let previous = self.current.ok_or(ShapeError::CommandBeforeMove {
            command_index: self.commands.len(),
        })?;
        let index = self.commands.len();
        validate_builder_xy(control_x, control_y, index, self.limits)?;
        validate_builder_xy(x, y, index, self.limits)?;
        let control = Point::new(control_x, control_y);
        let point = Point::new(x, y);
        if same_point(previous, point) && same_point(previous, control) {
            return Err(ShapeError::ZeroLengthSegment {
                command_index: index,
            });
        }
        self.push(ShapeCommand::QuadraticTo {
            control_x,
            control_y,
            x,
            y,
        })?;
        self.current = Some(point);
        self.contour_has_segment = true;
        Ok(())
    }

    /// Explicitly appends a cubic command and returns its error immediately.
    pub fn try_cubic_to(
        &mut self,
        control1_x: f32,
        control1_y: f32,
        control2_x: f32,
        control2_y: f32,
        x: f32,
        y: f32,
    ) -> Result<(), ShapeError> {
        let previous = self.current.ok_or(ShapeError::CommandBeforeMove {
            command_index: self.commands.len(),
        })?;
        let index = self.commands.len();
        validate_builder_xy(control1_x, control1_y, index, self.limits)?;
        validate_builder_xy(control2_x, control2_y, index, self.limits)?;
        validate_builder_xy(x, y, index, self.limits)?;
        let control1 = Point::new(control1_x, control1_y);
        let control2 = Point::new(control2_x, control2_y);
        let point = Point::new(x, y);
        if same_point(previous, point)
            && same_point(previous, control1)
            && same_point(previous, control2)
        {
            return Err(ShapeError::ZeroLengthSegment {
                command_index: index,
            });
        }
        self.push(ShapeCommand::CubicTo {
            control1_x,
            control1_y,
            control2_x,
            control2_y,
            x,
            y,
        })?;
        self.current = Some(point);
        self.contour_has_segment = true;
        Ok(())
    }

    /// Explicitly appends an arc and returns its error immediately.
    #[allow(clippy::too_many_arguments)]
    pub fn try_arc(
        &mut self,
        center_x: f32,
        center_y: f32,
        radius_x: f32,
        radius_y: f32,
        start_angle: f32,
        sweep_angle: f32,
        rotation: f32,
    ) -> Result<(), ShapeError> {
        validate_builder_xy(center_x, center_y, self.commands.len(), self.limits)?;
        validate_builder_xy(radius_x, radius_y, self.commands.len(), self.limits)?;
        for (value, field) in [
            (start_angle, "start_angle"),
            (sweep_angle, "sweep_angle"),
            (rotation, "rotation"),
        ] {
            if !value.is_finite() {
                return Err(ShapeError::NonFinite {
                    command_index: self.commands.len(),
                    field,
                });
            }
        }
        if radius_x <= 0.0 || radius_y <= 0.0 {
            return Err(ShapeError::InvalidArc {
                command_index: self.commands.len(),
                reason: ArcError::InvalidRadius,
            });
        }
        if sweep_angle.abs() <= f32::EPSILON {
            return Err(ShapeError::InvalidArc {
                command_index: self.commands.len(),
                reason: ArcError::InvalidSweep,
            });
        }
        if sweep_angle.abs() > core::f32::consts::TAU + 1.0e-4 {
            return Err(ShapeError::InvalidArc {
                command_index: self.commands.len(),
                reason: ArcError::SweepTooLarge,
            });
        }
        let arc_extent = arc_bounds(
            center_x,
            center_y,
            radius_x,
            radius_y,
            start_angle,
            sweep_angle,
            rotation,
        )
        .ok_or(ShapeError::NonFinite {
            command_index: self.commands.len(),
            field: "extent",
        })?;
        if !bounds_within_limit(arc_extent, self.limits.max_abs_coordinate) {
            return Err(ShapeError::CoordinateOutOfRange {
                command_index: self.commands.len(),
                field: "extent",
            });
        }
        let arc_start = arc_point(
            center_x,
            center_y,
            radius_x,
            radius_y,
            start_angle,
            rotation,
        );
        if let Some(current) = self.current {
            if !current.approx_eq(arc_start) {
                self.try_line_to(arc_start.x, arc_start.y)?;
            }
        } else {
            self.try_move_to(arc_start.x, arc_start.y)?;
        }
        self.push(ShapeCommand::ArcTo {
            center_x,
            center_y,
            radius_x,
            radius_y,
            start_angle,
            sweep_angle,
            rotation,
        })?;
        self.current = Some(arc_point(
            center_x,
            center_y,
            radius_x,
            radius_y,
            start_angle + sweep_angle,
            rotation,
        ));
        self.contour_has_segment = true;
        Ok(())
    }

    /// Explicitly appends a closed ellipse contour.
    pub fn try_ellipse(
        &mut self,
        center_x: f32,
        center_y: f32,
        radius_x: f32,
        radius_y: f32,
        rotation: f32,
    ) -> Result<(), ShapeError> {
        let start_angle = 0.0;
        let start = arc_point(
            center_x,
            center_y,
            radius_x,
            radius_y,
            start_angle,
            rotation,
        );
        self.try_move_to(start.x, start.y)?;
        self.try_arc(
            center_x,
            center_y,
            radius_x,
            radius_y,
            start_angle,
            core::f32::consts::TAU,
            rotation,
        )?;
        self.try_close()
    }

    /// Explicitly appends a close command and returns its error immediately.
    pub fn try_close(&mut self) -> Result<(), ShapeError> {
        let index = self.commands.len();
        if self.current.is_none() {
            return Err(ShapeError::CloseWithoutContour {
                command_index: index,
            });
        }
        if !self.contour_has_segment {
            return Err(ShapeError::EmptyContour {
                command_index: index,
            });
        }
        self.push(ShapeCommand::Close)?;
        self.current = None;
        self.start = None;
        self.contour_has_segment = false;
        Ok(())
    }

    /// Finishes validation and returns an immutable path.
    pub fn build(self) -> Result<ShapePath, ShapeError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        ShapePath::try_from_commands_with_limits(self.commands, self.limits)
    }

    /// Alias for [`Self::build`].
    #[inline]
    pub fn finish(self) -> Result<ShapePath, ShapeError> {
        self.build()
    }

    fn push(&mut self, command: ShapeCommand) -> Result<(), ShapeError> {
        if self.commands.len() >= self.limits.max_commands {
            return Err(ShapeError::TooManyCommands {
                limit: self.limits.max_commands,
            });
        }
        self.commands.push(command);
        Ok(())
    }

    fn record_error(&mut self, error: ShapeError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }
}

impl Default for ShapePathBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_limits(limits: ShapeLimits) -> Result<(), ShapeError> {
    limits.validate().map(|_| ())
}

fn validate_builder_xy(
    x: f32,
    y: f32,
    command_index: usize,
    limits: ShapeLimits,
) -> Result<(), ShapeError> {
    check_xy(x, y, command_index, "coordinate", limits)
}

fn check_xy(
    x: f32,
    y: f32,
    command_index: usize,
    field: &'static str,
    limits: ShapeLimits,
) -> Result<(), ShapeError> {
    check_finite(x, command_index, field)?;
    check_finite(y, command_index, field)?;
    if x.abs() > limits.max_abs_coordinate || y.abs() > limits.max_abs_coordinate {
        return Err(ShapeError::CoordinateOutOfRange {
            command_index,
            field,
        });
    }
    Ok(())
}

fn check_finite(value: f32, command_index: usize, field: &'static str) -> Result<(), ShapeError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ShapeError::NonFinite {
            command_index,
            field,
        })
    }
}

#[inline]
fn same_point(first: Point, second: Point) -> bool {
    first.x == second.x && first.y == second.y
}

#[derive(Default)]
struct BoundsAccumulator {
    min: Point,
    max: Point,
    has_value: bool,
}

impl BoundsAccumulator {
    fn include(&mut self, point: Point) {
        if !self.has_value {
            self.min = point;
            self.max = point;
            self.has_value = true;
            return;
        }
        self.min.x = self.min.x.min(point.x);
        self.min.y = self.min.y.min(point.y);
        self.max.x = self.max.x.max(point.x);
        self.max.y = self.max.y.max(point.y);
    }

    fn finish(self) -> Option<ShapeBounds> {
        ShapeBounds::new(self.min, self.max)
    }
}

fn include_quadratic_bounds(bounds: &mut BoundsAccumulator, p0: Point, p1: Point, p2: Point) {
    bounds.include(p0);
    bounds.include(p2);
    for t in [
        quadratic_extremum(p0.x, p1.x, p2.x),
        quadratic_extremum(p0.y, p1.y, p2.y),
    ]
    .into_iter()
    .flatten()
    {
        bounds.include(quadratic_point(p0, p1, p2, t));
    }
}

fn include_cubic_bounds(
    bounds: &mut BoundsAccumulator,
    p0: Point,
    p1: Point,
    p2: Point,
    p3: Point,
) {
    bounds.include(p0);
    bounds.include(p3);
    for t in cubic_extrema(p0.x, p1.x, p2.x, p3.x)
        .into_iter()
        .chain(cubic_extrema(p0.y, p1.y, p2.y, p3.y))
        .flatten()
    {
        bounds.include(cubic_point(p0, p1, p2, p3, t));
    }
}

#[allow(clippy::too_many_arguments)]
fn include_arc_bounds(
    bounds: &mut BoundsAccumulator,
    center_x: f32,
    center_y: f32,
    radius_x: f32,
    radius_y: f32,
    start_angle: f32,
    sweep_angle: f32,
    rotation: f32,
) {
    bounds.include(arc_point(
        center_x,
        center_y,
        radius_x,
        radius_y,
        start_angle,
        rotation,
    ));
    bounds.include(arc_point(
        center_x,
        center_y,
        radius_x,
        radius_y,
        start_angle + sweep_angle,
        rotation,
    ));

    let (sin_rotation, cos_rotation) = rotation.sin_cos();
    let x_phase = (-sin_rotation * radius_y).atan2(cos_rotation * radius_x);
    let y_phase = (cos_rotation * radius_y).atan2(sin_rotation * radius_x);
    for phase in [
        x_phase,
        x_phase + core::f32::consts::PI,
        y_phase,
        y_phase + core::f32::consts::PI,
    ] {
        if angle_in_sweep(start_angle, sweep_angle, phase) {
            bounds.include(arc_point(
                center_x, center_y, radius_x, radius_y, phase, rotation,
            ));
        }
    }
}

fn arc_bounds(
    center_x: f32,
    center_y: f32,
    radius_x: f32,
    radius_y: f32,
    start_angle: f32,
    sweep_angle: f32,
    rotation: f32,
) -> Option<ShapeBounds> {
    let mut bounds = BoundsAccumulator::default();
    include_arc_bounds(
        &mut bounds,
        center_x,
        center_y,
        radius_x,
        radius_y,
        start_angle,
        sweep_angle,
        rotation,
    );
    bounds.finish()
}

#[inline]
fn bounds_within_limit(bounds: ShapeBounds, limit: f32) -> bool {
    [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y]
        .into_iter()
        .all(|value| value.is_finite() && value.abs() <= limit)
}

#[inline]
fn include_bounds(accumulator: &mut BoundsAccumulator, bounds: ShapeBounds) {
    accumulator.include(bounds.min);
    accumulator.include(bounds.max);
}

#[inline]
fn arc_point(
    center_x: f32,
    center_y: f32,
    radius_x: f32,
    radius_y: f32,
    angle: f32,
    rotation: f32,
) -> Point {
    let (sin_angle, cos_angle) = angle.sin_cos();
    let (sin_rotation, cos_rotation) = rotation.sin_cos();
    Point::new(
        center_x + cos_rotation * radius_x * cos_angle - sin_rotation * radius_y * sin_angle,
        center_y + sin_rotation * radius_x * cos_angle + cos_rotation * radius_y * sin_angle,
    )
}

fn angle_in_sweep(start: f32, sweep: f32, angle: f32) -> bool {
    if sweep.abs() >= core::f32::consts::TAU - 1.0e-4 {
        return true;
    }
    let delta = if sweep >= 0.0 {
        (angle - start).rem_euclid(core::f32::consts::TAU)
    } else {
        (start - angle).rem_euclid(core::f32::consts::TAU)
    };
    delta <= sweep.abs() + 1.0e-4
}

#[inline]
fn quadratic_extremum(p0: f32, p1: f32, p2: f32) -> Option<f32> {
    let denominator = p0 - 2.0 * p1 + p2;
    if denominator.abs() <= f32::EPSILON {
        return None;
    }
    let t = (p0 - p1) / denominator;
    (0.0..1.0).contains(&t).then_some(t)
}

fn cubic_extrema(p0: f32, p1: f32, p2: f32, p3: f32) -> [Option<f32>; 2] {
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    if a.abs() <= f32::EPSILON {
        if b.abs() <= f32::EPSILON {
            return [None, None];
        }
        let t = -c / b;
        return [(0.0..1.0).contains(&t).then_some(t), None];
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 || !discriminant.is_finite() {
        return [None, None];
    }
    let sqrt = discriminant.sqrt();
    let first = (-b + sqrt) / (2.0 * a);
    let second = (-b - sqrt) / (2.0 * a);
    [
        (0.0..1.0).contains(&first).then_some(first),
        (0.0..1.0).contains(&second).then_some(second),
    ]
}

#[inline]
fn quadratic_point(p0: Point, p1: Point, p2: Point, t: f32) -> Point {
    let inverse = 1.0 - t;
    Point::new(
        inverse * inverse * p0.x + 2.0 * inverse * t * p1.x + t * t * p2.x,
        inverse * inverse * p0.y + 2.0 * inverse * t * p1.y + t * t * p2.y,
    )
}

#[inline]
fn cubic_point(p0: Point, p1: Point, p2: Point, p3: Point, t: f32) -> Point {
    let inverse = 1.0 - t;
    Point::new(
        inverse.powi(3) * p0.x
            + 3.0 * inverse.powi(2) * t * p1.x
            + 3.0 * inverse * t.powi(2) * p2.x
            + t.powi(3) * p3.x,
        inverse.powi(3) * p0.y
            + 3.0 * inverse.powi(2) * t * p1.y
            + 3.0 * inverse * t.powi(2) * p2.y
            + t.powi(3) * p3.y,
    )
}

fn curve_steps(length: f32, tolerance: f32, max: usize) -> usize {
    let estimate = (length / (tolerance * 2.0)).ceil();
    if !estimate.is_finite() {
        max
    } else {
        estimate.max(2.0).min(max as f32) as usize
    }
}

fn push_polyline(result: &mut Vec<ShapePolyline>, points: &mut Vec<Point>, closed: bool) {
    if points.len() >= 2 {
        result.push(ShapePolyline {
            points: std::mem::take(points).into(),
            closed,
        });
    } else {
        points.clear();
    }
}

#[inline]
fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&canonical_f32_bits(value).to_le_bytes());
}

fn compute_id(commands: &[ShapeCommand]) -> ShapePathId {
    let mut hash = 0xcbf29ce484222325u64;
    let mut update = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    };
    for byte in b"ASHP" {
        update(*byte);
    }
    update(1);
    for byte in (commands.len() as u32).to_le_bytes() {
        update(byte);
    }
    for command in commands.iter().copied() {
        update(command.tag());
        match command {
            ShapeCommand::MoveTo { x, y } | ShapeCommand::LineTo { x, y } => {
                for byte in canonical_f32_bits(x).to_le_bytes() {
                    update(byte);
                }
                for byte in canonical_f32_bits(y).to_le_bytes() {
                    update(byte);
                }
            }
            ShapeCommand::QuadraticTo {
                control_x,
                control_y,
                x,
                y,
            } => {
                for value in [control_x, control_y, x, y] {
                    for byte in canonical_f32_bits(value).to_le_bytes() {
                        update(byte);
                    }
                }
            }
            ShapeCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => {
                for value in [control1_x, control1_y, control2_x, control2_y, x, y] {
                    for byte in canonical_f32_bits(value).to_le_bytes() {
                        update(byte);
                    }
                }
            }
            ShapeCommand::ArcTo {
                center_x,
                center_y,
                radius_x,
                radius_y,
                start_angle,
                sweep_angle,
                rotation,
            } => {
                for value in [
                    center_x,
                    center_y,
                    radius_x,
                    radius_y,
                    start_angle,
                    sweep_angle,
                    rotation,
                ] {
                    for byte in canonical_f32_bits(value).to_le_bytes() {
                        update(byte);
                    }
                }
            }
            ShapeCommand::Close => {}
        }
    }
    ShapePathId(hash)
}

#[inline]
fn canonical_f32_bits(value: f32) -> u32 {
    if value == 0.0 { 0 } else { value.to_bits() }
}
