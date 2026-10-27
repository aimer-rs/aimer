//! [`AnyDecor`], an owned, type-erased [`Decoration`] with inline storage.

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_color::prelude::Color;

use std::ops::{Deref, DerefMut};

use aimer_rubick::Rubick;

use super::Decoration;

/// An owned, type-erased [`Decoration`] with eight words of inline capacity.
///
/// Construct with [`Decoration::notype`] or [`AnyDecor::new`]. Small, suitably
/// aligned values stay inline; larger or over-aligned values use Rubick's
/// pooled heap storage. The owner forwards painting, child geometry,
/// backgrounds and revisions, and is exactly the size of the
/// [`aimer_rubick::Rubick`] it wraps.
#[repr(transparent)]
pub struct AnyDecor(Rubick<dyn Decoration, 8>);

impl AnyDecor {
    /// Payload bytes a decoration may occupy and still be stored inside the
    /// handle. A larger or more strictly aligned one takes a pooled heap block.
    pub const INLINE_CAPACITY: usize = Rubick::<dyn Decoration, 8>::INLINE_CAPACITY;

    /// Erases `decoration`, embedding it when it fits and pooling it otherwise.
    #[inline]
    pub fn new<D: Decoration + 'static>(decoration: D) -> Self {
        Self(Rubick::erase(decoration))
    }

    /// Returns whether the decoration is stored inside this handle.
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.0.is_inline()
    }

    /// Returns whether the decoration lives in a separate heap block.
    #[inline]
    pub fn is_heap(&self) -> bool {
        self.0.is_heap()
    }

    /// Returns whether borrowing this handle needs no adapter call.
    ///
    /// Every `AnyDecor` is built by plain erasure, so this is always `true`;
    /// it exists to let a test pin that no projection adapter is ever stored.
    #[inline]
    pub fn is_direct(&self) -> bool {
        self.0.is_direct()
    }
}

impl Deref for AnyDecor {
    type Target = dyn Decoration;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for AnyDecor {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl AsRef<dyn Decoration> for AnyDecor {
    #[inline]
    fn as_ref(&self) -> &(dyn Decoration + 'static) {
        &*self.0
    }
}

impl AsMut<dyn Decoration> for AnyDecor {
    #[inline]
    fn as_mut(&mut self) -> &mut (dyn Decoration + 'static) {
        &mut *self.0
    }
}

impl Decoration for AnyDecor {
    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.as_ref().paint(canvas, origin, size, scale);
    }

    #[inline]
    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.as_ref().paint_layer(canvas, origin, size, scale);
    }

    #[inline]
    fn paint_with_background(
        &self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32,
        background: Option<Color>,
    ) {
        self.as_ref().paint_with_background(canvas, origin, size, scale, background);
    }

    #[inline]
    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.as_ref().paint_outsets(size, scale)
    }

    #[inline]
    fn content_insets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.as_ref().content_insets(size, scale)
    }

    #[inline]
    fn child_clip_radius(&self, size: ResolvedSize, scale: f32) -> Option<[f32; 4]> {
        self.as_ref().child_clip_radius(size, scale)
    }

    #[inline]
    fn background_color(&self) -> Option<Color> {
        self.as_ref().background_color()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.as_ref().is_empty()
    }

    #[inline]
    fn revision(&self) -> u64 {
        self.as_ref().revision()
    }

    #[inline]
    fn notype(self) -> AnyDecor {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrapper_adds_nothing_to_the_owner_it_wraps() {
        assert_eq!(size_of::<AnyDecor>(), size_of::<Rubick<dyn Decoration, 8>>());
        assert_eq!(align_of::<AnyDecor>(), align_of::<Rubick<dyn Decoration, 8>>());
        assert_eq!(
            size_of::<Option<AnyDecor>>(),
            size_of::<Option<Rubick<dyn Decoration, 8>>>(),
            "an optional decoration must keep whatever niche the owner has"
        );
    }
}
