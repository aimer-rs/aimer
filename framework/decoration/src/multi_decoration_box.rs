//! [`MultiDecorationBox`], which stacks decorations behind and over one child.

use std::cell::Cell;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::{Canvas, composite_background};
use aimer_color::prelude::Color;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, PortableWidget, RequiredChild, Widget};

use crate::{BoxDecoration, Decoration};

/// Paints an ordered stack of decorations behind one child.
///
/// Every layer receives the same box geometry. Later layers paint over earlier
/// layers, then the child paints over the stack. Borders do not inset the child,
/// corner radii do not clip it, and the wrapper passes events through to it.
/// Paint outsets are the maximum on each side across all layers.
///
/// The stack paints behind the child unless [`Self::foreground`] moves it over
/// the child, [`Self::masked`] shows it only through the child's coverage, or
/// [`Self::faded`] draws the child through the stack's alpha. The layers keep
/// their order and geometry in all four placements.
///
/// Chained decorations retain their concrete types. Runtime collections are
/// consumed once and retained; mixed collections can use `Box<dyn Decoration>`
/// or `Rc<dyn Decoration>`. Neither form requires decorations to implement
/// `Clone`. Custom decoration widgets do not support portable lowering.
///
/// # Examples
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{BoxDecoration, Decoration, LightEdge, MultiDecorationBox};
/// use aimer_color::prelude::Color;
///
/// let widget = MultiDecorationBox::new()
///     .decoration(BoxDecoration::new().background_color(Color::BLUE).border_radius(24.0))
///     .decoration(LightEdge::new())
///     .child(SizedBox::new().width(240).height(80));
///
/// let layers: Vec<Box<dyn Decoration>> = vec![
///     Box::new(BoxDecoration::new().background_color(Color::BLUE).border_radius(24.0)),
///     Box::new(LightEdge::new()),
/// ];
/// let dynamic = MultiDecorationBox::new().decorations(layers)
///     .child(SizedBox::new().width(240).height(80));
///
/// // The same layers over the child, so an opaque child cannot cover the rim.
/// let over = MultiDecorationBox::new()
///     .decoration(BoxDecoration::new().border_radius(24.0))
///     .decoration(LightEdge::new())
///     .foreground()
///     .child(SizedBox::new().width(240).height(80));
/// ```
pub struct MultiDecorationBox<D = BoxDecoration, W = RequiredChild> {
    decoration: D,
    child: W,
    placement: Placement,
}

/// Where a stack paints relative to its child.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Placement {
    /// Before the child, so the child paints over it.
    #[default]
    Behind,
    /// After the child, over it.
    Over,
    /// Through the child's coverage, so it shows only where the child has pixels.
    Through,
    /// Over nothing: the stack is only alpha, and the child is drawn through it.
    Fade,
}

impl Default for MultiDecorationBox {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiDecorationBox {
    /// Creates an empty stack builder requiring a child.
    /// Finish with [`Self::child`] or [`Self::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            decoration: BoxDecoration::new(),
            child: RequiredChild,
            placement: Placement::Behind,
        }
    }
}

impl<D: Decoration> MultiDecorationBox<D> {
    /// Paints the whole stack over the child instead of behind it.
    ///
    /// Every layer keeps its order and its shared box geometry; they just
    /// paint after the child, as [`ForegroundDecorationBox`](crate::ForegroundDecorationBox)
    /// does for one decoration. Replaces an earlier [`Self::masked`] or
    /// [`Self::faded`].
    #[inline]
    pub fn foreground(mut self) -> Self {
        self.placement = Placement::Over;
        self
    }

    /// Shows the whole stack only where the child has coverage.
    ///
    /// The layers keep their order, and the child's own colors are not drawn,
    /// as [`MaskedDecorationBox`](crate::MaskedDecorationBox) does for one
    /// decoration. Replaces an earlier [`Self::foreground`] or [`Self::faded`].
    #[inline]
    pub fn masked(mut self) -> Self {
        self.placement = Placement::Through;
        self
    }

    /// Draws the child through the whole stack's alpha instead of showing the stack.
    ///
    /// The layers keep their order and paint together into one mask, and only
    /// that mask's alpha is used: the child's pixels are scaled by it, and the
    /// layers' own colors are never drawn. This is
    /// [`OpacityMaskBox`](crate::OpacityMaskBox) for a stack, so any layer can
    /// contribute: an [`OpacityGradient`](crate::OpacityGradient) fades, a
    /// [`BorderRadius`](crate::BorderRadius) rounds, and a translucent
    /// [`Gradient`](crate::Gradient) does both. A stack that paints nothing leaves
    /// the child as it is. Replaces an earlier [`Self::foreground`] or
    /// [`Self::masked`].
    ///
    /// ```
    /// use aimer_container::SizedBox;
    /// use aimer_decoration::{BorderRadius, MultiDecorationBox, OpacityGradient};
    ///
    /// // A child with rounded corners that also fades out toward the bottom.
    /// let widget = MultiDecorationBox::new()
    ///     .decoration(BorderRadius::from(16))
    ///     .decoration(OpacityGradient::linear().opacities([1.0, 0.0]))
    ///     .faded()
    ///     .child(SizedBox::new().width(240).height(160));
    /// ```
    #[inline]
    pub fn faded(mut self) -> Self {
        self.placement = Placement::Fade;
        self
    }

    /// Appends a layer after the existing layers without boxing it.
    #[inline]
    pub fn decoration<E: Decoration>(self, decoration: E) -> MultiDecorationBox<impl Decoration> {
        MultiDecorationBox {
            decoration: Layers::new(self.decoration, decoration),
            child: self.child,
            placement: self.placement,
        }
    }

    /// Consumes a collection and appends its layers in iteration order.
    ///
    /// Empty collections add no paint. Changing stack membership requires
    /// rebuilding the widget; each layer's [`Decoration::revision`] still
    /// controls reuse of its paint as part of the stack.
    /// Mix this with [`Self::decoration`] to append further layers.
    #[inline]
    pub fn decorations<I>(self, decorations: I) -> MultiDecorationBox<impl Decoration>
    where
        I: IntoIterator,
        I::Item: Decoration,
    {
        self.decoration(CollectedLayers {
            layers: decorations
                .into_iter()
                .map(|decoration| Layer {
                    decoration,
                    recorded_revision: Cell::new(None),
                })
                .collect(),
            revision: Cell::new(0),
        })
    }

    /// Attaches the required child and completes the builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> MultiDecorationBox<D, W> {
        MultiDecorationBox {
            decoration: self.decoration,
            child,
            placement: self.placement,
        }
    }

    /// Attaches a child and erases the completed widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget
    where
        D: 'static,
    {
        self.child(child).boxed()
    }
}

impl<D: Decoration + 'static, W: Widget> PortableWidget for MultiDecorationBox<D, W> {}

impl<D: Decoration + 'static, W: Widget> Widget for MultiDecorationBox<D, W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        const NAME: &str = "MultiDecorationBox";
        match self.placement {
            Placement::Behind => {
                crate::decoration_box::decoration_element(self.decoration, self.child, ctx, NAME)
            }
            Placement::Over => crate::foreground_decoration_box::foreground_element(
                self.decoration,
                self.child.to_element(ctx),
                NAME,
            ),
            Placement::Through => crate::decoration_box::decoration_element_with(
                self.decoration,
                self.child,
                ctx,
                NAME,
                crate::decoration_box::ChildMask::DecorationByChild,
            ),
            Placement::Fade => crate::decoration_box::decoration_element_with(
                self.decoration,
                self.child,
                ctx,
                NAME,
                crate::decoration_box::ChildMask::ChildByDecoration,
            ),
        }
    }

    fn debug_name(&self) -> &'static str {
        "MultiDecorationBox"
    }
}

struct Layers<A, B> {
    first: A,
    second: B,
    recorded_revisions: Cell<Option<(u64, u64)>>,
    revision: Cell<u64>,
}

impl<A, B> Layers<A, B> {
    #[inline]
    fn new(first: A, second: B) -> Self {
        Self {
            first,
            second,
            recorded_revisions: Cell::new(None),
            revision: Cell::new(0),
        }
    }
}

impl<A: Decoration, B: Decoration> Decoration for Layers<A, B> {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.first.paint_layer(canvas, origin, size, scale);
        self.second.paint_layer(canvas, origin, size, scale);
    }

    #[inline]
    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.paint(canvas, origin, size, scale);
    }

    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        let mut outsets = [0.0; 4];
        extend_outsets(&mut outsets, self.first.paint_outsets(size, scale));
        extend_outsets(&mut outsets, self.second.paint_outsets(size, scale));
        outsets
    }

    fn background_color(&self) -> Option<Color> {
        let color = composite_background(
            self.second.background_color(),
            composite_background(self.first.background_color(), Color::Transparent),
        );
        (color != Color::Transparent).then_some(color)
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.first.is_empty() && self.second.is_empty()
    }

    fn revision(&self) -> u64 {
        // Hashes or sums can hide simultaneous changes in different layers.
        let revisions = (self.first.revision(), self.second.revision());
        if self.recorded_revisions.get() != Some(revisions) {
            self.recorded_revisions.set(Some(revisions));
            self.revision.set(self.revision.get().wrapping_add(1));
        }
        self.revision.get()
    }
}

struct Layer<D> {
    decoration: D,
    recorded_revision: Cell<Option<u64>>,
}

struct CollectedLayers<D> {
    layers: Vec<Layer<D>>,
    revision: Cell<u64>,
}

impl<D: Decoration> Decoration for CollectedLayers<D> {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        for layer in &self.layers {
            layer.decoration.paint_layer(canvas, origin, size, scale);
        }
    }

    #[inline]
    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.paint(canvas, origin, size, scale);
    }

    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        let mut outsets = [0.0; 4];
        for layer in &self.layers {
            extend_outsets(&mut outsets, layer.decoration.paint_outsets(size, scale));
        }
        outsets
    }

    fn background_color(&self) -> Option<Color> {
        let color = self
            .layers
            .iter()
            .fold(Color::Transparent, |background, layer| {
                composite_background(layer.decoration.background_color(), background)
            });
        (color != Color::Transparent).then_some(color)
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.layers.iter().all(|layer| layer.decoration.is_empty())
    }

    fn revision(&self) -> u64 {
        let mut changed = false;
        // Visit every entry, even after finding a change, so a simultaneous
        // update produces one stable aggregate revision and no later repaint.
        for layer in &self.layers {
            let revision = layer.decoration.revision();
            if layer.recorded_revision.get() != Some(revision) {
                layer.recorded_revision.set(Some(revision));
                changed = true;
            }
        }
        if changed {
            self.revision.set(self.revision.get().wrapping_add(1));
        }
        self.revision.get()
    }
}

#[inline]
pub(crate) fn extend_outsets(outsets: &mut [f32; 4], candidate: [f32; 4]) {
    for (outset, value) in outsets.iter_mut().zip(candidate) {
        // Sanitize before merging: an infinite layer must not erase a valid
        // finite outset when the retained wrapper sanitizes the final bounds.
        if value.is_finite() {
            *outset = outset.max(value);
        }
    }
}
