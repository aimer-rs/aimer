//! Per-frame layer state: transforms with parents resolved, opacity, visibility.

use crate::affine::Affine;
use crate::model::{Composition, Position, Transform};

/// A transform evaluated at one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalTransform {
    pub matrix: Affine,
    /// `0..=1`.
    pub opacity: f32,
}

impl Transform {
    /// Evaluates the transform at `frame`.
    ///
    /// The matrix is `translate(position) * rotate * scale * translate(-anchor)`,
    /// built in closed form so a frame costs one `sin_cos` and no matrix products.
    pub fn local_at(&self, frame: f32) -> LocalTransform {
        let anchor = self.anchor.value_at(frame);
        let position = match &self.position {
            Position::Joint(p) => p.value_at(frame),
            Position::Split { x, y } => [x.value_at(frame), y.value_at(frame)],
        };
        let scale = self.scale.value_at(frame);
        let (sin, cos) = self.rotation.value_at(frame).to_radians().sin_cos();
        let (sx, sy) = (scale[0] / 100.0, scale[1] / 100.0);
        // Linear part is R * S; the translation keeps the anchor on the position.
        let (a, b, c, d) = (cos * sx, sin * sx, -sin * sy, cos * sy);
        LocalTransform {
            matrix: Affine {
                a,
                b,
                c,
                d,
                e: position[0] - (a * anchor[0] + c * anchor[1]),
                f: position[1] - (b * anchor[0] + d * anchor[1]),
            },
            opacity: (self.opacity.value_at(frame) / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// One layer's evaluated state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerState {
    /// The layer's own transform.
    pub local: Affine,
    /// Parent transforms applied: maps layer space to composition space.
    pub world: Affine,
    /// The layer's own opacity, `0..=1`. Parent opacity is not inherited.
    pub opacity: f32,
    /// Inside the layer's frame range and not hidden.
    pub visible: bool,
}

impl Composition {
    /// Evaluates every layer at `frame`, replacing the contents of `out` so a
    /// caller can reuse one buffer across frames. `out[i]` belongs to
    /// `self.layers[i]`.
    ///
    /// Parents are resolved once at load time, so this walks each layer's
    /// ancestor chain without lookups and cannot loop.
    pub fn layer_states(&self, frame: f32, out: &mut Vec<LayerState>) {
        out.clear();
        out.reserve(self.layers.len());
        for layer in &self.layers {
            let local = layer.transform.local_at(frame);
            out.push(LayerState {
                local: local.matrix,
                world: local.matrix,
                opacity: local.opacity,
                visible: !layer.hidden && frame >= layer.in_frame && frame < layer.out_frame,
            });
        }
        for (i, layer) in self.layers.iter().enumerate() {
            let mut world = out[i].local;
            let mut next = layer.parent_slot;
            while let Some(slot) = next {
                world = out[slot].local * world;
                next = self.layers[slot].parent_slot;
            }
            out[i].world = world;
        }
    }

    /// The frame to show `seconds` after the start, held at the first and last
    /// frames.
    pub fn frame_at(&self, seconds: f32) -> f32 {
        (self.in_frame + seconds * self.frame_rate).clamp(self.in_frame, self.out_frame)
    }
}
