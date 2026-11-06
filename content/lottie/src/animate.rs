//! Evaluating properties at a frame.

use crate::ease::cubic_bezier;
use crate::model::{Bezier, Interpolation, Property, Rgba, Trim, Vec2};

/// A value that can be blended between two keyframes.
pub trait Lerp: Clone {
    /// `self` at `t == 0`, `other` at `t == 1`; `t` may leave `0..=1` when an
    /// easing curve overshoots.
    fn lerp(&self, other: &Self, t: f32) -> Self;
}

impl Lerp for f32 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Lerp for Vec2 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        [self[0].lerp(&other[0], t), self[1].lerp(&other[1], t)]
    }
}

impl Lerp for Rgba {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        std::array::from_fn(|i| self[i].lerp(&other[i], t))
    }
}

impl Lerp for Bezier {
    /// Blends vertex by vertex. Paths with different vertex counts cannot be
    /// blended, so the first one holds until `t` reaches the second.
    fn lerp(&self, other: &Self, t: f32) -> Self {
        if self.vertices.len() != other.vertices.len() {
            return if t < 1.0 { self.clone() } else { other.clone() };
        }
        let blend = |a: &[Vec2], b: &[Vec2]| -> Vec<Vec2> {
            a.iter().zip(b).map(|(a, b)| a.lerp(b, t)).collect()
        };
        Bezier {
            closed: self.closed,
            vertices: blend(&self.vertices, &other.vertices),
            in_tangents: blend(&self.in_tangents, &other.in_tangents),
            out_tangents: blend(&self.out_tangents, &other.out_tangents),
        }
    }
}

impl<T: Lerp> Property<T> {
    /// The value at `frame`. Before the first keyframe and after the last one
    /// the property holds that keyframe's value; a NaN frame reads as before
    /// the first.
    ///
    /// Keyframes are expected in time order, as exporters write them. Finding
    /// the segment is a binary search, so evaluation is `O(log n)` in the
    /// keyframe count and allocation-free for scalar, vector and color values.
    pub fn value_at(&self, frame: f32) -> T {
        let keys = match self {
            Property::Static(value) => return value.clone(),
            Property::Animated(keys) => keys,
        };
        let next = keys.partition_point(|key| key.time <= frame);
        if next == 0 {
            return keys[0].value.clone();
        }
        let Some(b) = keys.get(next) else {
            return keys[next - 1].value.clone();
        };
        let a = &keys[next - 1];
        let span = b.time - a.time;
        // Two keyframes on the same frame: the later one wins from that frame on.
        if span <= 0.0 {
            return b.value.clone();
        }
        let t = (frame - a.time) / span;
        match a.interpolation {
            Interpolation::Linear => a.value.lerp(&b.value, t),
            Interpolation::Hold => a.value.clone(),
            Interpolation::Bezier { ease_out, ease_in } => {
                a.value.lerp(&b.value, cubic_bezier(ease_out, ease_in, t))
            }
        }
    }
}

impl Trim {
    /// The visible parts of the path at `frame`, as `(start, end)` fractions of
    /// its length. A trim that wraps past the end of the path yields two parts;
    /// one that shows nothing yields none.
    pub fn segments_at(&self, frame: f32) -> [Option<(f32, f32)>; 2] {
        let fraction = |p: &Property<f32>| (p.value_at(frame) / 100.0).clamp(0.0, 1.0);
        let (mut start, mut end) = (fraction(&self.start), fraction(&self.end));
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        let length = end - start;
        if length <= 0.0 {
            return [None, None];
        }
        if length >= 1.0 {
            return [Some((0.0, 1.0)), None];
        }
        // The offset is in degrees; a full turn moves the window once around.
        let shift = self.offset.value_at(frame) / 360.0;
        let start = (start + shift).rem_euclid(1.0);
        let end = start + length;
        if end <= 1.0 {
            [Some((start, end)), None]
        } else {
            [Some((start, 1.0)), Some((0.0, end - 1.0))]
        }
    }
}
