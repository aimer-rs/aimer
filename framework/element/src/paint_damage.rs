//! Shared conservative damage tracking for animated and retained paint.
//!
//! Damage tracking and retained paint are separate contracts, and only the
//! first one is target-independent. An owner that paints its child live owes
//! the frame its footprint on every target, or a damage-driven renderer reuses
//! the previous target and the new paint is recorded but never presented. The
//! cache that replays recorded content is a native optimization instead, and is
//! only compiled where the renderer can consume it — so the gates that remove
//! that cache must leave this module and its callers in place.

use std::cell::Cell;

use aimer_attribute::size::ResolvedSize;
use aimer_cupid::damage_region::DamageRect;

use crate::base::BuildContext;

/// The result of projecting a local paint box into device-pixel coordinates.
///
/// `Known(None)` is an empty or completely off-target box. It is different
/// from `Unknown`: an empty box can clear a previously painted footprint,
/// while an invalid transform must invalidate the whole target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaintBounds {
    Unknown,
    Known(Option<DamageRect>),
}

/// Tracks one visual owner's previous transformed footprint.
///
/// Call [`Self::mark_current_bounds`] after applying the owner's current
/// canvas transform and before painting its content. The tracker marks one
/// conservative union of the old and current rectangles so a moving bounded
/// animation does not leave stale pixels behind.
#[doc(hidden)]
pub struct PaintDamageTracker {
    previous: Cell<Option<DamageRect>>,
}

impl Default for PaintDamageTracker {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl PaintDamageTracker {
    /// Creates an empty damage tracker.
    #[inline]
    pub const fn new() -> Self {
        Self {
            previous: Cell::new(None),
        }
    }

    /// Records the current transformed bounds and marks the required damage.
    ///
    /// A visual value change marks the current footprint even when the bounds
    /// are unchanged, which is required for opacity and other in-place paint
    /// changes. A bounds change marks one union of the old and current
    /// footprints. Non-finite geometry marks the complete frame and retires
    /// the old footprint because there is no safe partial rectangle.
    #[inline]
    pub fn mark_current_bounds(
        &self,
        ctx: &BuildContext,
        size: ResolvedSize,
        visual_changed: bool,
    ) {
        self.mark_bounds(transformed_bounds(ctx, size), visual_changed);
    }

    /// Records damage for an owner whose output is clipped to `clip_size`.
    ///
    /// The ordinary two-pixel antialiasing pad is intersected with the
    /// unpadded transformed clip bounds before it is added to the frame. This
    /// keeps a moving scroll viewport from invalidating neighboring pixels its
    /// clip cannot paint. The clip starts at local `(0, 0)` and uses the same
    /// transform as `size`.
    #[doc(hidden)]
    pub fn mark_current_bounds_clipped(
        &self,
        ctx: &BuildContext,
        size: ResolvedSize,
        clip_size: ResolvedSize,
        visual_changed: bool,
    ) {
        let bounds = transformed_bounds(ctx, size);
        let clip = transformed_bounds_with_padding(ctx, clip_size, 0.0);
        let clipped = match (bounds, clip) {
            (PaintBounds::Unknown, _) | (_, PaintBounds::Unknown) => PaintBounds::Unknown,
            (PaintBounds::Known(Some(bounds)), PaintBounds::Known(Some(clip))) => {
                PaintBounds::Known(intersection(bounds, clip))
            }
            (PaintBounds::Known(None), _) | (_, PaintBounds::Known(None)) => {
                PaintBounds::Known(None)
            }
        };
        self.mark_bounds(clipped, visual_changed);
    }

    fn mark_bounds(&self, bounds: PaintBounds, visual_changed: bool) {
        match bounds {
            PaintBounds::Unknown => self.mark_full(),
            PaintBounds::Known(current) => {
                let previous = self.previous.replace(current);
                match (previous, current) {
                    (None, None) => {}
                    (None, Some(current)) => crate::mark_paint_damage(current),
                    (Some(previous), Some(current)) if previous == current => {
                        if visual_changed {
                            crate::mark_paint_damage(current);
                        }
                    }
                    (Some(previous), Some(current)) => {
                        crate::mark_paint_damage(union(previous, current));
                    }
                    (Some(previous), None) => crate::mark_paint_damage(previous),
                }
            }
        }
    }

    /// Forces a complete target repaint and forgets the partial baseline.
    #[inline]
    pub fn mark_full(&self) {
        self.previous.set(None);
        crate::mark_paint_damage_full();
    }

    /// Forgets the previous footprint without adding damage.
    #[inline]
    pub fn clear(&self) {
        self.previous.set(None);
    }
}

/// Projects a local content box through the current canvas transform.
///
/// The result uses the same conservative two-pixel edge pad as retained
/// paint. Callers that need to distinguish an empty box from invalid geometry
/// should match on [`PaintBounds`] directly.
pub(crate) fn transformed_bounds(ctx: &BuildContext, size: ResolvedSize) -> PaintBounds {
    transformed_bounds_with_padding(ctx, size, 2.0)
}

fn transformed_bounds_with_padding(
    ctx: &BuildContext,
    size: ResolvedSize,
    padding: f32,
) -> PaintBounds {
    let width = size.width;
    let height = size.height;
    if !width.is_finite() || !height.is_finite() || width < 0.0 || height < 0.0 {
        return PaintBounds::Unknown;
    }
    if width == 0.0 || height == 0.0 {
        return PaintBounds::Known(None);
    }

    let transform = ctx.canvas.get_transform();
    let points = [
        transform.transform_point(0.0, 0.0),
        transform.transform_point(width, 0.0),
        transform.transform_point(0.0, height),
        transform.transform_point(width, height),
    ];
    if points
        .iter()
        .any(|(x, y)| !x.is_finite() || !y.is_finite())
    {
        return PaintBounds::Unknown;
    }

    let min_x = points.iter().map(|(x, _)| *x).fold(f32::INFINITY, f32::min) - padding;
    let min_y = points.iter().map(|(_, y)| *y).fold(f32::INFINITY, f32::min) - padding;
    let max_x = points
        .iter()
        .map(|(x, _)| *x)
        .fold(f32::NEG_INFINITY, f32::max)
        + padding;
    let max_y = points
        .iter()
        .map(|(_, y)| *y)
        .fold(f32::NEG_INFINITY, f32::max)
        + padding;
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return PaintBounds::Unknown;
    }
    if max_x <= 0.0 || max_y <= 0.0 {
        return PaintBounds::Known(None);
    }

    let x = pixel_floor(min_x);
    let y = pixel_floor(min_y);
    let right = pixel_ceil(max_x);
    let bottom = pixel_ceil(max_y);
    if right <= x || bottom <= y {
        PaintBounds::Known(None)
    } else {
        PaintBounds::Known(Some(DamageRect::new(
            x,
            y,
            right.saturating_sub(x),
            bottom.saturating_sub(y),
        )))
    }
}

#[inline]
fn intersection(left: DamageRect, right: DamageRect) -> Option<DamageRect> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (u64::from(left.x) + u64::from(left.width))
        .min(u64::from(right.x) + u64::from(right.width));
    let bottom_edge = (u64::from(left.y) + u64::from(left.height))
        .min(u64::from(right.y) + u64::from(right.height));
    (right_edge > u64::from(x) && bottom_edge > u64::from(y)).then_some(DamageRect::new(
        x,
        y,
        (right_edge - u64::from(x)) as u32,
        (bottom_edge - u64::from(y)) as u32,
    ))
}

#[inline]
fn union(left: DamageRect, right: DamageRect) -> DamageRect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (u64::from(left.x) + u64::from(left.width))
        .max(u64::from(right.x) + u64::from(right.width))
        .min(u64::from(u32::MAX));
    let bottom_edge = (u64::from(left.y) + u64::from(left.height))
        .max(u64::from(right.y) + u64::from(right.height))
        .min(u64::from(u32::MAX));
    DamageRect::new(
        x,
        y,
        right_edge.saturating_sub(u64::from(x)) as u32,
        bottom_edge.saturating_sub(u64::from(y)) as u32,
    )
}

#[inline]
fn pixel_floor(value: f32) -> u32 {
    if value <= 0.0 {
        0
    } else if value >= u32::MAX as f32 {
        u32::MAX
    } else {
        value.floor() as u32
    }
}

#[inline]
fn pixel_ceil(value: f32) -> u32 {
    if value <= 0.0 {
        0
    } else if value >= u32::MAX as f32 {
        u32::MAX
    } else {
        value.ceil() as u32
    }
}
