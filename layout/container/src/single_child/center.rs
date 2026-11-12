use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_macro::{EventElement, Rebuildable};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, LayoutElement, RequiredChild, VisitorElement, Widget,
};

use super::hug::hugged_size;

/// Centers one child on both axes within the space its parent gives it.
///
/// `Center` is the cheap form of a `Row` with both alignments set to center:
/// it has no flex pass, no per-child tables, and no configuration to resolve.
/// On every axis the parent bounds, it fills the available space and places the
/// child at half the remaining distance; on an unbounded axis (inside a
/// scrollable, for example) it wraps the child instead, so there is nothing to
/// center within and the child sits at the origin.
///
/// The child is measured at its own size, the way `Align` measures it: an
/// unsized [`Container`] that declares a pixel size through its content is
/// hugged rather than stretched to fill the parent. A child larger than the
/// space is pinned to the origin rather than offset negatively.
///
/// Attach a child with [`Center::child`] to retain its concrete type, or with
/// [`Center::box_child`] when branches need a shared erased type.
///
/// # Examples
///
/// ```
/// use aimer_container::{Center, ZeroSizedBox};
/// use aimer_widget::Widget;
///
/// let widget = Center::new().child(ZeroSizedBox);
/// let _erased = widget.boxed();
/// ```
///
/// [`Container`]: crate::Container
pub struct Center<W = RequiredChild> {
    child: W,
}

impl Default for Center {
    fn default() -> Self {
        Self::new()
    }
}

impl Center {
    /// Creates a centering builder.
    ///
    /// Finish the builder with [`Center::child`] or [`Center::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
        }
    }

    /// Attaches the required child and completes this builder.
    ///
    /// The concrete child type is preserved. Use [`Center::box_child`] for
    /// branch type erasure.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Center<W> {
        Center { child }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    ///
    /// This is equivalent to calling [`Center::child`] followed by
    /// [`Widget::boxed`]. Use it when different branches must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget + 'static> Widget for Center<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawCenter {
            child: self.child.to_element(ctx),
        }
        .boxed()
    }
}

#[derive(EventElement, Rebuildable)]
#[rebuildable(forwards_marks_only)]
struct RawCenter {
    child: AnyElement,
}

/// Constraints beyond this are the "no bound" sentinel (`f32::MAX`), not a size.
#[inline]
fn is_unbounded(value: f32) -> bool {
    value > 1_000_000.0
}

/// Distance that centers `child` inside `available`; never negative, so an
/// oversized child starts at the origin instead of overflowing both sides.
#[inline]
fn centered_offset(available: f32, child: f32) -> f32 {
    (available - child).max(0.0) * 0.5
}

impl RawCenter {
    /// Child size and its offset inside this widget, both in physical pixels.
    #[inline]
    fn place(&self, ctx: &BuildContext) -> (Vec2d, ResolvedSize) {
        let child = hugged_size(&*self.child, ctx);
        let constraint = ctx.box_constraint;
        let offset = Vec2d {
            x: if is_unbounded(constraint.max_width) {
                0.0
            } else {
                centered_offset(constraint.max_width, child.width)
            },
            y: if is_unbounded(constraint.max_height) {
                0.0
            } else {
                centered_offset(constraint.max_height, child.height)
            },
        };
        (offset, child)
    }

    /// The child sees exactly its own size, so a child that fills (a `Row`, a
    /// `Container` with an `Auto` axis) fills itself and not the whole parent.
    #[inline]
    fn child_context<'a>(
        ctx: &BuildContext<'a>,
        offset: Vec2d,
        size: ResolvedSize,
    ) -> BuildContext<'a> {
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = size;
        child_ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
        child_ctx.set_visible_rect(ctx
            .visible_rect()
            .map(|(x, y, width, height)| (x - offset.x, y - offset.y, width, height)));
        child_ctx
    }

    /// `None` when the layout is not a finite, paintable one.
    #[inline]
    fn retained_layout<'a>(&self, ctx: &BuildContext<'a>) -> Option<(Vec2d, BuildContext<'a>)> {
        if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
            return None;
        }
        let (offset, size) = self.place(ctx);
        let valid = |value: f32| value.is_finite() && (0.0..=1_000_000.0).contains(&value);
        if !valid(size.width) || !valid(size.height) || !offset.x.is_finite() || !offset.y.is_finite()
        {
            return None;
        }
        Some((offset, Self::child_context(ctx, offset, size)))
    }
}

impl Drawable for RawCenter {
    fn can_paint(&self, ctx: &BuildContext) -> bool {
        self.retained_layout(ctx).is_some()
    }

    fn paint(&self, _ctx: &BuildContext) {}

    fn child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        _child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        self.retained_layout(ctx).map(|(_, child_ctx)| child_ctx)
    }

    fn child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
    ) -> Option<(
        aimer_cupid::draw_cmd::Rect,
        Option<aimer_cupid::draw_cmd::Rect>,
    )> {
        let (offset, child_ctx) = self.retained_layout(ctx)?;
        let position = child.pos().unwrap_or_default();
        let size = child
            .paint_bounds(&child_ctx)
            .unwrap_or_else(|| child.content_size(&child_ctx));
        let bounds = aimer_cupid::draw_cmd::Rect::new(
            (offset.x + position.x) / ctx.scale,
            (offset.y + position.y) / ctx.scale,
            size.width / ctx.scale,
            size.height / ctx.scale,
        );
        if !bounds.x.is_finite()
            || !bounds.y.is_finite()
            || !bounds.width.is_finite()
            || !bounds.height.is_finite()
            || bounds.width < 0.0
            || bounds.height < 0.0
        {
            return None;
        }
        Some((bounds, child.retained_clip(&child_ctx)))
    }

    fn update(&self, ctx: &BuildContext) {
        let (offset, size) = self.place(ctx);
        ctx.canvas.save();
        if offset.x == 0.0 && offset.y == 0.0 && size == ctx.parent_size {
            // The child already fills the parent at the same origin, so cloning
            // the context would only add work. The save/restore pair stays: it
            // protects siblings if a child leaves canvas state behind.
            self.child.update(ctx);
        } else {
            ctx.canvas.translate(offset);
            self.child.update(&Self::child_context(ctx, offset, size));
        }
        ctx.canvas.restore();
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let Some((offset, child_ctx)) = self.retained_layout(ctx) else {
            self.child.sync_paint_geometry(ctx);
            return;
        };
        ctx.canvas.save();
        ctx.canvas.translate(offset);
        self.child.sync_paint_geometry(&child_ctx);
        ctx.canvas.restore();
    }
}

impl LayoutElement for RawCenter {
    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    /// A bounded axis is filled without measuring the child at all; only an
    /// unbounded axis needs the child's extent.
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let constraint = ctx.box_constraint;
        let width_unbounded = is_unbounded(constraint.max_width);
        let height_unbounded = is_unbounded(constraint.max_height);
        if !width_unbounded && !height_unbounded {
            return ResolvedSize {
                width: constraint.max_width.max(0.0),
                height: constraint.max_height.max(0.0),
            };
        }
        let child = hugged_size(&*self.child, ctx);
        ResolvedSize {
            width: if width_unbounded {
                child.width
            } else {
                constraint.max_width.max(0.0)
            },
            height: if height_unbounded {
                child.height
            } else {
                constraint.max_height.max(0.0)
            },
        }
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
}

impl VisitorElement for RawCenter {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Center"
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_widget::base::WindowHandle;
    use aimer_widget::{EventElement, Rebuildable};

    use super::*;

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Observation {
        max_width: f32,
        max_height: f32,
        parent_width: f32,
        parent_height: f32,
        translation_x: f32,
        translation_y: f32,
    }

    struct Probe {
        size: ResolvedSize,
        observation: Rc<Cell<Observation>>,
    }

    impl Drawable for Probe {
        fn update(&self, ctx: &BuildContext) {
            let (translation_x, translation_y) = ctx.canvas.get_transform_translation();
            self.observation.set(Observation {
                max_width: ctx.box_constraint.max_width,
                max_height: ctx.box_constraint.max_height,
                parent_width: ctx.parent_size.width,
                parent_height: ctx.parent_size.height,
                translation_x,
                translation_y,
            });
        }
    }

    impl EventElement for Probe {}
    impl Rebuildable for Probe {}

    impl VisitorElement for Probe {
        fn debug_name(&self) -> &'static str {
            "Probe"
        }
    }

    impl LayoutElement for Probe {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            self.size
        }
    }

    impl Widget for Probe {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            Element::boxed(self)
        }
    }

    fn context(max_width: f32, max_height: f32, scale: f32) -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let mut ctx = BuildContext::new(
            canvas,
            ResolvedSize {
                width: max_width,
                height: max_height,
            },
            scale,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), scale as f64),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width,
            max_height,
        };
        ctx
    }

    fn centered(
        child_width: f32,
        child_height: f32,
        ctx: &BuildContext,
    ) -> (AnyElement, Rc<Cell<Observation>>) {
        let observation = Rc::new(Cell::new(Observation::default()));
        let element = Center::new()
            .child(Probe {
                size: ResolvedSize {
                    width: child_width,
                    height: child_height,
                },
                observation: observation.clone(),
            })
            .to_element(ctx);
        (element, observation)
    }

    #[tokio::test]
    async fn bounded_axes_fill_the_parent_and_center_the_child() {
        let ctx = context(100.0, 80.0, 1.0);
        let (element, observation) = centered(20.0, 10.0, &ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 100.0,
                height: 80.0,
            }
        );
        element.update(&ctx);

        let observed = observation.get();
        assert_eq!(observed.translation_x, 40.0);
        assert_eq!(observed.translation_y, 35.0);
        assert_eq!(observed.max_width, 20.0);
        assert_eq!(observed.max_height, 10.0);
        assert_eq!(observed.parent_width, 20.0);
        assert_eq!(observed.parent_height, 10.0);
    }

    #[tokio::test]
    async fn unbounded_axes_wrap_the_child_without_offset() {
        let ctx = context(f32::MAX, f32::MAX, 1.0);
        let (element, observation) = centered(20.0, 10.0, &ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 20.0,
                height: 10.0,
            }
        );
        element.update(&ctx);

        let observed = observation.get();
        assert_eq!(observed.translation_x, 0.0);
        assert_eq!(observed.translation_y, 0.0);
    }

    #[tokio::test]
    async fn only_the_bounded_axis_is_centered() {
        let ctx = context(f32::MAX, 80.0, 1.0);
        let (element, observation) = centered(20.0, 10.0, &ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 20.0,
                height: 80.0,
            }
        );
        element.update(&ctx);

        let observed = observation.get();
        assert_eq!(observed.translation_x, 0.0);
        assert_eq!(observed.translation_y, 35.0);
    }

    #[tokio::test]
    async fn oversized_child_is_pinned_to_the_origin() {
        let ctx = context(100.0, 80.0, 1.0);
        let (element, observation) = centered(120.0, 10.0, &ctx);

        element.update(&ctx);

        let observed = observation.get();
        assert_eq!(observed.translation_x, 0.0);
        assert_eq!(observed.translation_y, 35.0);
    }

    #[tokio::test]
    async fn offsets_are_in_physical_pixels() {
        // The probe reports physical pixels, so the 100x80 parent is already
        // scaled; the offset must not be scaled a second time.
        let ctx = context(100.0, 80.0, 2.0);
        let (element, observation) = centered(20.0, 10.0, &ctx);

        element.update(&ctx);

        let observed = observation.get();
        assert_eq!(observed.translation_x, 40.0);
        assert_eq!(observed.translation_y, 35.0);
    }

    #[tokio::test]
    async fn retained_geometry_matches_the_painted_position() {
        let ctx = context(100.0, 80.0, 2.0);
        let (element, _) = centered(20.0, 10.0, &ctx);

        let mut bounds = None;
        element.visit_children(&mut |child| {
            bounds = element.child_geometry(&ctx, child).map(|(rect, _)| rect);
        });

        let rect = bounds.expect("center provides retained child geometry");
        assert_eq!((rect.x, rect.y), (20.0, 17.5));
        assert_eq!((rect.width, rect.height), (10.0, 5.0));
    }
}
