//! Ordered stacks of decorations: `[D]`, `[D; N]` and `Vec<D>`.
//!
//! A stack paints its layers in order, so later layers cover earlier ones, and
//! every layer sees what the earlier ones declared (background and box shape).
//! The collections own no state, so they work behind `Box<[D]>`/`Rc<[D]>` and
//! as the decoration of a `Container`, which `MultiDecorationBox` cannot be.

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::{Canvas, composite_background};
use aimer_color::prelude::Color;

use super::Decoration;
use crate::multi_decoration_box::extend_outsets;

/// Paints each element in order, bottom to top, behind the child.
///
/// Geometry combines conservatively because the layers overlap in one box:
/// paint outsets and child insets are the largest on each side, and the child
/// clip is the largest radius at each corner (the tightest region). The
/// background is the layers' backgrounds composited in order. The stack is
/// empty when every layer is, including when there are none.
///
/// Beware that `Vec` and slices have an inherent `is_empty` that counts layers;
/// call `Decoration::is_empty(&stack)` (or use a generic bound) for the
/// decoration meaning.
///
/// A container's fallback background goes behind the first layer, unless some
/// layer supplies a background of its own.
///
/// The revision is an order-sensitive 64-bit digest of the layers' revisions:
/// any single layer changing always changes it, and the stack keeps no state.
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{AnyDecor, BorderRadius, Decoration, DecorationBox, Magnification};
///
/// let layers: Vec<AnyDecor> = vec![
///     BorderRadius::from(16).notype(),
///     Magnification::new().zoom(2.0).notype(),
/// ];
/// let widget = DecorationBox::new()
///     .decoration(layers)
///     .child(SizedBox::new().width(240).height(160));
/// ```
impl<D: Decoration> Decoration for [D] {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        for layer in self {
            layer.paint_layer(canvas, origin, size, scale);
        }
    }

    // `paint` already propagates each leaf's background once; doing it again
    // here would composite translucent layers twice.
    #[inline]
    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.paint(canvas, origin, size, scale);
    }

    fn paint_with_background(
        &self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32,
        background: Option<Color>,
    ) {
        let Some((first, rest)) = self.split_first() else {
            if let Some(color) = background {
                canvas.fill_color_rect(origin, size, color);
            }
            return;
        };
        if background.is_none() || self.background_color().is_some() {
            return self.paint(canvas, origin, size, scale);
        }
        // The fallback sits behind everything, so only the bottom layer may
        // paint it; no layer has a background to propagate in this branch.
        first.paint_with_background(canvas, origin, size, scale, background);
        for layer in rest {
            layer.paint_layer(canvas, origin, size, scale);
        }
    }

    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        let mut outsets = [0.0; 4];
        for layer in self {
            extend_outsets(&mut outsets, layer.paint_outsets(size, scale));
        }
        outsets
    }

    fn content_insets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        let mut insets = [0.0; 4];
        for layer in self {
            extend_outsets(&mut insets, layer.content_insets(size, scale));
        }
        insets
    }

    fn child_clip_radius(&self, size: ResolvedSize, scale: f32) -> Option<[f32; 4]> {
        let mut clip: Option<[f32; 4]> = None;
        for layer in self {
            if let Some(radii) = layer.child_clip_radius(size, scale) {
                extend_outsets(clip.get_or_insert([0.0; 4]), radii);
            }
        }
        clip
    }

    fn background_color(&self) -> Option<Color> {
        let color = self.iter().fold(Color::Transparent, |background, layer| {
            composite_background(layer.background_color(), background)
        });
        (color != Color::Transparent).then_some(color)
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.iter().all(Decoration::is_empty)
    }

    fn revision(&self) -> u64 {
        // Each step is a bijection of the running digest, so one layer's change
        // can never cancel out; only simultaneous changes could collide.
        self.iter().fold(self.len() as u64, |digest, layer| {
            (digest.rotate_left(5) ^ layer.revision()).wrapping_mul(0x517c_c1b7_2722_0a95)
        })
    }
}

macro_rules! forward_to_slice {
    ($ty:ty; $($generics:tt)*) => {
        impl<$($generics)*> Decoration for $ty {
            #[inline]
            fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
                self.as_slice().paint(canvas, origin, size, scale);
            }

            #[inline]
            fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
                self.as_slice().paint_layer(canvas, origin, size, scale);
            }

            #[inline]
            fn paint_with_background(
                &self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32,
                background: Option<Color>,
            ) {
                self.as_slice().paint_with_background(canvas, origin, size, scale, background);
            }

            #[inline]
            fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
                self.as_slice().paint_outsets(size, scale)
            }

            #[inline]
            fn content_insets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
                self.as_slice().content_insets(size, scale)
            }

            #[inline]
            fn child_clip_radius(&self, size: ResolvedSize, scale: f32) -> Option<[f32; 4]> {
                self.as_slice().child_clip_radius(size, scale)
            }

            #[inline]
            fn background_color(&self) -> Option<Color> {
                self.as_slice().background_color()
            }

            #[inline]
            fn is_empty(&self) -> bool {
                // Name the trait: the slice's inherent `is_empty` counts layers.
                Decoration::is_empty(self.as_slice())
            }

            #[inline]
            fn revision(&self) -> u64 {
                self.as_slice().revision()
            }
        }
    };
}

// An owned runtime stack and a fixed-size stack; both defer to the slice impl.
forward_to_slice!(Vec<D>; D: Decoration);
forward_to_slice!([D; N]; D: Decoration, const N: usize);
