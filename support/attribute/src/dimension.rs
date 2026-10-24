use crate::bounds::Bounds;
use crate::position::Vec2d;
use crate::size::ResolvedSize;
use std::cell::Cell;

///
/// Represents a dimension type that can be used to define sizes in different
/// units.
///
/// The `Dimension` enum supports two primary units:
/// - Pixels (`Px`) - Defines the size in absolute pixel values.
/// - Percent (`Percent`) - Defines the size as a percentage of a parent
///   container.
///
/// It also supports an automatic size value, `Auto`, which can be used when the
/// size should be determined by layout or content rules.
///
/// `Px` and `Percent` use `f32` as their underlying type on every supported
/// target, including WebAssembly.
///
/// ### Traits
/// The `Dimension` enum derives the following traits:
/// - `Debug`: Enables formatted output for debugging purposes.
/// - `Clone`: Allows for creating a duplicate of `Dimension` instances.
/// - `Copy`: Allows for `Dimension` to be copied rather than moved.
/// - `PartialEq`: Enables equality comparisons between `Dimension` instances.
///
/// # Example
/// ```
/// use self::aimer_attribute::Dimension;
///
/// let px_dimension = Dimension::Px(100.0);
/// let percent_dimension = Dimension::Percent(50.0);
/// let auto_dimension = Dimension::Auto;
///
/// assert!(px_dimension != percent_dimension);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dimension {
    Px(f32),
    Percent(f32),
    Auto,
}
#[allow(clippy::derivable_impls)]
impl Default for Dimension {
    fn default() -> Self {
        Self::Auto
    }
}

impl From<f32> for Dimension {
    fn from(v: f32) -> Self {
        Self::Px(v)
    }
}

impl From<i32> for Dimension {
    fn from(v: i32) -> Self {
        Self::Px(v as f32)
    }
}

impl From<u32> for Dimension {
    fn from(v: u32) -> Self {
        Self::Px(v as f32)
    }
}

impl Dimension {
    pub fn resolve(&self, parent_value: f32, scale: f32) -> f32 {
        match self {
            Dimension::Px(v) => v * scale,
            Dimension::Percent(p) => parent_value * (p / 100.0),
            Dimension::Auto => parent_value,
        }
    }
}

// `bound` is wrapped in a `Cell` so it can be mutated through a shared
// `&self` (during layout/draw) soundly. The previous version stored a bare
// `Option<Bounds>` and mutated it via a `&raw const ... as *mut` cast, which
// is undefined behaviour: the type was `Freeze`, so the optimizer was free to
// assume it never changed behind a `&self` and cache the stale value. In
// release/wasm-opt builds that meant `is_inside` kept reading the old `None`
// bounds after `save`, so hover hit-testing silently stopped working.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheBounds {
    bound: Cell<Option<Bounds>>,
}

impl CacheBounds {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            bound: Cell::new(None),
        }
    }

    pub const fn with_vec2d(vec2d: Vec2d) -> Self {
        Self {
            bound: Cell::new(Some(Bounds::new(vec2d.x, vec2d.y, 0.0, 0.0))),
        }
    }

    pub fn is_cached(&self) -> bool {
        self.bound.get().is_some()
    }

    pub fn get_bounds(&self) -> Option<Bounds> {
        self.bound.get()
    }

    pub fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bound.get().map(|b| {
            (
                Vec2d { x: b.x, y: b.y },
                Vec2d {
                    x: b.x + b.width,
                    y: b.y + b.height,
                },
            )
        })
    }

    pub fn set_bounds(&self, bounds: Bounds) {
        self.bound.set(Some(bounds));
    }

    /// Forgets the cached rectangle, so nothing is inside it.
    pub fn clear(&self) {
        self.bound.set(None);
    }

    pub fn set_size(&self, size: ResolvedSize) {
        if let Some(mut bound) = self.bound.get() {
            bound.width = size.width;
            bound.height = size.height;
            self.bound.set(Some(bound));
        }
    }

    pub fn save(&self, scale: f32, x: f32, y: f32, width: f32, height: f32) {
        let cache_x = x / scale;
        let cache_y = y / scale;
        let cache_w = width / scale;
        let cache_h = height / scale;
        let bound = Bounds::new(cache_x, cache_y, cache_w, cache_h);
        self.set_bounds(bound);
    }

    pub fn is_inside(&self, x: f32, y: f32) -> bool {
        let Some(bound) = self.bound.get() else {
            return false;
        };
        bound.x <= x && x <= bound.x + bound.width && bound.y <= y && y <= bound.y + bound.height
    }
}

#[cfg(test)]
mod cache_bounds_tests {
    use super::*;

    // Regression: `save` mutates through a shared `&self`, then `is_inside`
    // reads it back. With the old `Freeze` field + raw-pointer write the
    // optimizer cached the stale `None` in release/wasm-opt builds and hover
    // hit-testing silently failed. This must hold in release too.
    #[test]
    fn save_is_visible_to_is_inside_through_shared_ref() {
        let bounds = CacheBounds::new();
        assert!(
            !bounds.is_inside(50.0, 50.0),
            "empty bounds contain nothing"
        );

        bounds.save(1.0, 10.0, 20.0, 100.0, 50.0);
        assert!(
            bounds.is_inside(50.0, 40.0),
            "point inside must be detected after save"
        );
        assert!(
            !bounds.is_inside(200.0, 40.0),
            "point outside must be rejected"
        );
    }
}
