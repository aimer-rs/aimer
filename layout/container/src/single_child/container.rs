use std::rc::Rc;

use aimer_attribute::dimension::Dimension;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_decoration::{BoxDecoration, Decoration};
use aimer_events::element::ElementEvent;
use aimer_macro::Rebuildable;
pub use aimer_style::*;
use aimer_widget::InteractionBounds;
use aimer_widget::base::*;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, EventTreeRole,
    LayoutCache, LayoutElement, RequiredChild, VisitorElement, Widget,
};

mod decoration;
pub use decoration::RetainedBoxDecoration;

const LOCAL_V2_CONTAINER_EXTENT_LIMIT: f32 = 1_000_000.0;

#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, SourceFingerprint};

/// A decorated single-child layout box with optional size, spacing, and color.
///
/// Width and height default to [`Dimension::Auto`] and resolve within the
/// parent's constraints. Margin contributes outside the painted box; padding
/// and decoration borders inset the child. All spacing and pixel dimensions are
/// logical units that are scaled by the build context.
///
/// [`Container::decoration`] accepts any [`Decoration`] without boxing it.
/// The default [`BoxDecoration`] supplies border insets and rounded child
/// clipping. [`crate::Scalable`] may render beyond ancestor clips without
/// changing layout. Custom decorations choose their own insets and optional clip.
///
/// Attach a child with [`Container::child`] to retain its concrete type, or
/// with [`Container::box_child`] when branches need a shared erased type.
pub struct Container<T = RequiredChild, D: Decoration + 'static = BoxDecoration> {
    pub(crate) width: Dimension,
    pub(crate) height: Dimension,
    pub padding: LayoutSpacing,
    pub margin: LayoutSpacing,
    pub box_decoration: D,
    decoration_source: Option<Rc<dyn RetainedBoxDecoration>>,
    pub color: Option<Color>,
    pub child: T,
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

impl Container {
    /// Creates an automatically sized container with no spacing or decoration.
    ///
    /// Finish the builder with [`Container::child`] or
    /// [`Container::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            width: Dimension::Auto,
            height: Dimension::Auto,
            padding: LayoutSpacing::default(),
            margin: LayoutSpacing::default(),
            box_decoration: BoxDecoration::default(),
            decoration_source: None,
            color: None,
            child: RequiredChild,
        }
    }
}

impl<D: Decoration + 'static> Container<RequiredChild, D> {
    /// Sets the outer width before margin is added.
    ///
    /// The default is [`Dimension::Auto`]. Pixel dimensions use logical pixels,
    /// percentages resolve against the parent's maximum width, and the final
    /// value is clamped to the available constraints.
    #[inline]
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the outer height before margin is added.
    ///
    /// The default is [`Dimension::Auto`]. Pixel dimensions use logical pixels,
    /// percentages resolve against the parent's maximum height, and the final
    /// value is clamped to the available constraints.
    #[inline]
    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.height = height.into();
        self
    }

    /// Replaces the spacing between the decoration edge and the child.
    ///
    /// Padding is measured in logical pixels and defaults to zero on every
    /// side. It reduces the constraints available to the child.
    #[inline]
    pub fn padding(mut self, padding: LayoutSpacing) -> Self {
        self.padding = padding;
        self
    }

    /// Replaces the transparent spacing outside the decorated box.
    ///
    /// Margin is measured in logical pixels and defaults to zero on every side.
    /// It contributes to the container's layout footprint but is not painted.
    #[inline]
    pub fn margin(mut self, margin: LayoutSpacing) -> Self {
        self.margin = margin;
        self
    }

    /// Replaces the complete background, border, and corner decoration.
    ///
    /// The default decoration is empty. Its border width is included when
    /// deriving the child's inset and clipping radius.
    #[inline]
    pub fn box_decoration(self, box_decoration: BoxDecoration) -> Container {
        self.decoration(box_decoration)
    }

    /// Replaces the decoration while retaining its concrete type.
    ///
    /// The decoration paints over the box inside the margin, including padding.
    /// Its content insets reduce child layout; its optional clip confines child
    /// painting. Custom decorations do not support portable lowering.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::{Container, ZeroSizedBox};
    /// use aimer_decoration::BackdropFilter;
    ///
    /// let panel = Container::new().width(240).height(80)
    ///     .decoration(BackdropFilter::new().blur_radius(16.0))
    ///     .child(ZeroSizedBox);
    /// ```
    #[inline]
    pub fn decoration<E: Decoration + 'static>(self, decoration: E) -> Container<RequiredChild, E> {
        Container {
            width: self.width,
            height: self.height,
            padding: self.padding,
            margin: self.margin,
            box_decoration: decoration,
            decoration_source: None,
            color: self.color,
            child: self.child,
        }
    }

    /// Supplies decoration for retained native interaction state.
    ///
    /// The source must publish revisions and notify tree geometry changes as
    /// required by [`RetainedBoxDecoration`]. This internal integration does
    /// not support portable lowering.
    #[doc(hidden)]
    #[inline]
    pub fn retained_box_decoration(self, source: Rc<dyn RetainedBoxDecoration>) -> Container {
        let mut container = self.decoration(BoxDecoration::default());
        container.decoration_source = Some(source);
        container
    }

    /// Sets a solid background color.
    ///
    /// By default no separate color is painted. Setting a color also makes the
    /// container opaque to scroll-event fall-through within its bounds.
    #[inline]
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Attaches the required child and completes this builder.
    ///
    /// The child is laid out inside padding and decoration borders, and its
    /// concrete type is preserved. Use [`Container::box_child`] when different
    /// branches need one erased return type.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Container<W, D> {
        Container {
            width: self.width,
            height: self.height,
            padding: self.padding,
            margin: self.margin,
            box_decoration: self.box_decoration,
            decoration_source: self.decoration_source,
            color: self.color,
            child,
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    ///
    /// This is equivalent to calling [`Container::child`] followed by
    /// [`Widget::boxed`]. Use it when different branches must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget, D: Decoration + 'static> Widget for Container<W, D> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = self.child.to_element(ctx);
        RawContainer {
            width: self.width,
            height: self.height,
            child,
            padding: self.padding,
            margin: self.margin,
            box_decoration: self.box_decoration,
            decoration_source: self
                .decoration_source
                .map(decoration::RetainedDecorationState::new),
            cache: LayoutCache::new(),
            flags: std::cell::Cell::new(if self.color.is_some() { HAS_COLOR } else { 0 }),
            bounds: InteractionBounds::new(),
            paint_state: std::cell::Cell::new(ContainerPaintState::new(self.color)),
        }
        .boxed()
    }
}

mod portable;

/// #### Low level container element.
///
/// - **Container**: safe wrapper for RawContainer
///
/// - **SizedBox**: fixed size container or placeholder
#[derive(Default, Rebuildable)]
#[rebuildable(forwards_marks_only)]
pub struct RawContainer<T: Element, D: Decoration = BoxDecoration> {
    pub padding: LayoutSpacing,
    pub margin: LayoutSpacing,
    pub width: Dimension,
    pub height: Dimension,
    pub box_decoration: D,
    decoration_source: Option<Box<decoration::RetainedDecorationState>>,
    pub child: T,
    pub cache: LayoutCache,
    flags: std::cell::Cell<u8>,
    pub bounds: InteractionBounds,
    paint_state: std::cell::Cell<ContainerPaintState>,
}

// Store flags in the former debug-name byte so adding the recorded background
// keeps the paint state at 16 bytes. Growing it would spill ordinary containers
// beyond the pool's 512-byte class once identity/provenance headers are included.
const RAW: u8 = 1;
const HAS_COLOR: u8 = 2;
const RECORDED: u8 = 4;

#[derive(Clone, Copy, Default)]
struct ContainerPaintState {
    revision: u64,
    color: Color,
    background: Color,
}

impl ContainerPaintState {
    #[inline]
    fn new(color: Option<Color>) -> Self {
        Self {
            revision: 0,
            color: color.unwrap_or(Color::Transparent),
            background: Color::Transparent,
        }
    }
}

#[derive(Clone, Copy)]
struct LocalV2ContainerGeometry {
    clip_origin: Vec2d,
    clip_size: ResolvedSize,
    child_origin: Vec2d,
    content_size: ResolvedSize,
    /// Corner radii of the child clip in device pixels (top-left, top-right,
    /// bottom-right, bottom-left): the outer radii shrunk by the border, as
    /// the live path computes them.
    clip_radii: [f32; 4],
    clips_child: bool,
}

impl<E: Element, D: Decoration> RawContainer<E, D> {
    /// Returns whether this container only forwards layout and a rectangular
    /// clip. Such a wrapper can expose the child's paint islands without
    /// recording its own decoration or translating an inset.
    #[inline]
    fn can_delegate_paint_islands(&self) -> bool {
        self.decoration_source.is_none()
            && self.width == Dimension::Auto
            && self.height == Dimension::Auto
            && self.padding == LayoutSpacing::default()
            && self.margin == LayoutSpacing::default()
            && self.fallback_background().is_none()
            && self.decoration().is_empty()
    }

    /// A container is *opaque* when it paints a background — either an explicit
    /// `color` or a `box_decoration.background_color`. An opaque container
    /// visually covers whatever sits behind it in a `Stack`, so it must also
    /// occlude it for hit-testing (Flutter's `HitTestBehavior::opaque`).
    fn is_opaque(&self) -> bool {
        self.decoration()
            .background_color()
            .or(self.fallback_background())
            .is_some()
    }

    fn local_v2_paint_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        let scale = ctx.scale;
        let parent_width = ctx.box_constraint.max_width;
        let parent_height = ctx.box_constraint.max_height;
        if !scale.is_finite()
            || scale <= 0.0
            || !parent_width.is_finite()
            || !parent_height.is_finite()
            || parent_width < 0.0
            || parent_height < 0.0
        {
            return None;
        }

        let (margin_left, margin_top, margin_right, margin_bottom) = self.margin(ctx);
        let width = match self.width {
            Dimension::Px(width) => width * scale,
            Dimension::Percent(percent) => {
                parent_width * (percent / 100.0) - margin_left - margin_right
            }
            Dimension::Auto => parent_width - margin_left - margin_right,
        };
        let height = match self.height {
            Dimension::Px(height) => height * scale,
            Dimension::Percent(percent) => {
                parent_height * (percent / 100.0) - margin_top - margin_bottom
            }
            Dimension::Auto => parent_height - margin_top - margin_bottom,
        };
        // An unbounded axis (a scroll view offers `f32::MAX`) stays huge here,
        // exactly as in the live `render`: children size themselves to their
        // content only while their constraint stays unbounded. Rectangles handed
        // to the render tree are clamped separately with `render_extent`.
        if !width.is_finite() || !height.is_finite() {
            return None;
        }

        Some(ResolvedSize {
            width: width.max(0.0),
            height: height.max(0.0),
        })
    }

    fn margin(&self, ctx: &BuildContext) -> (f32, f32, f32, f32) {
        let parent_width = ctx.box_constraint.max_width;
        let parent_height = ctx.box_constraint.max_height;
        let scale = ctx.scale;

        let m_left = self.margin.left.value(parent_width, scale);
        let m_top = self.margin.top.value(parent_height, scale);
        let m_right = self.margin.right.value(parent_width, scale);
        let m_bottom = self.margin.bottom.value(parent_height, scale);

        (m_left, m_top, m_right, m_bottom)
    }

    fn local_v2_geometry(&self, ctx: &BuildContext) -> Option<LocalV2ContainerGeometry> {
        let scale = ctx.scale;
        let size = self.local_v2_paint_size(ctx)?;
        let (margin_left, margin_top, _, _) = self.margin(ctx);
        let width = size.width;
        let height = size.height;
        let padding_left = self.padding.left.value(width, scale);
        let padding_top = self.padding.top.value(height, scale);
        let padding_right = self.padding.right.value(width, scale);
        let padding_bottom = self.padding.bottom.value(height, scale);
        let [border_left, border_top, border_right, border_bottom] =
            self.decoration_insets(width, height, scale);
        let clip_size = ResolvedSize {
            width: (width - border_left - border_right).max(0.0),
            height: (height - border_top - border_bottom).max(0.0),
        };
        let child_origin = Vec2d {
            x: margin_left + padding_left + border_left,
            y: margin_top + padding_top + border_top,
        };
        let content_size = ResolvedSize {
            width: (clip_size.width - padding_left - padding_right).max(0.0),
            height: (clip_size.height - padding_top - padding_bottom).max(0.0),
        };
        let clip = self.decoration().child_clip_radius(
            ResolvedSize {
                width: width / scale,
                height: height / scale,
            },
            scale,
        );
        let clip_radii = clip
            .unwrap_or([0.0; 4])
            .map(|radius| finite_nonnegative(radius) * scale);
        Some(LocalV2ContainerGeometry {
            clip_origin: Vec2d {
                x: margin_left + border_left,
                y: margin_top + border_top,
            },
            clip_size,
            child_origin,
            content_size,
            clip_radii,
            clips_child: clip.is_some(),
        })
    }

    #[inline]
    fn fallback_background(&self) -> Option<Color> {
        (self.flags.get() & HAS_COLOR != 0).then_some(self.paint_state.get().color)
    }

    #[inline]
    fn child_paint_background(&self, ctx: &BuildContext) -> Color {
        aimer_canvas::composite_background(
            self.decoration()
                .background_color()
                .or(self.fallback_background()),
            ctx.paint_background,
        )
    }

    fn local_v2_child_context<'a>(&self, ctx: &BuildContext<'a>) -> Option<BuildContext<'a>> {
        let geometry = self.local_v2_geometry(ctx)?;
        let mut child_ctx = ctx.clone();
        child_ctx.paint_background = self.child_paint_background(ctx);
        child_ctx.box_constraint.max_width = geometry.content_size.width;
        child_ctx.box_constraint.max_height = geometry.content_size.height;
        child_ctx.parent_size = geometry.content_size;
        child_ctx.visible_rect = ctx.visible_rect.map(|(x, y, width, height)| {
            (
                x - geometry.child_origin.x,
                y - geometry.child_origin.y,
                width,
                height,
            )
        });
        Some(child_ctx)
    }
}

#[inline]
fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

/// Clamps a device-pixel extent to what the render tree can carry.
///
/// A scroll view offers an unbounded axis as `f32::MAX`; scaled to device
/// pixels (or added to a position) that overflows to infinity, which the tree
/// rejects. `LOCAL_V2_CONTAINER_EXTENT_LIMIT` logical pixels is far beyond any
/// viewport, so clamping to it does not change what is visible.
#[inline]
fn render_extent(device_extent: f32, scale: f32) -> f32 {
    device_extent.min(LOCAL_V2_CONTAINER_EXTENT_LIMIT * scale)
}

#[inline]
fn intersect_retained_clip(
    left: aimer_cupid::draw_cmd_v2::Rect,
    right: aimer_cupid::draw_cmd_v2::Rect,
) -> aimer_cupid::draw_cmd_v2::Rect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    aimer_cupid::draw_cmd_v2::Rect::new(x, y, (right_edge - x).max(0.0), (bottom_edge - y).max(0.0))
}

mod painting;

impl<T: Element, D: Decoration> Drawable for RawContainer<T, D> {
    /// Hit-tested over the box that is drawn: the computed size less the
    /// margins, which start `retained_v2_interaction_offset` in from the node.
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        let (m_left, m_top, m_right, m_bottom) = self.margin(ctx);
        let computed = self.computed_size(ctx);
        Some(ResolvedSize {
            width: (computed.width - m_left - m_right).max(0.0),
            height: (computed.height - m_top - m_bottom).max(0.0),
        })
    }

    #[inline]
    fn retained_v2_interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        let (m_left, m_top, _, _) = self.margin(ctx);
        (m_left, m_top)
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("Container")
    }

    fn update(&self, ctx: &BuildContext) {
        self.render(ctx);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.local_v2_paint_size(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let Some(size) = self.local_v2_paint_size(ctx) else {
            unreachable!("RawContainer v2 support must be checked before painting")
        };

        let canvas = aimer_canvas::Canvas::of(ctx);
        let (margin_left, margin_top, _, _) = self.margin(ctx);
        // The render node starts where its paint starts, which is the shadow's or
        // outline's outer corner and not the layout box. The list is recorded
        // relative to the node, so the decorated box sits that far inside it;
        // otherwise a shadowed panel is drawn up and to the left of where it was
        // laid out, under children that are in the right place.
        let [outset_left, outset_top, _, _] =
            self.retained_v2_paint_outsets(ctx).unwrap_or([0.0; 4]);
        canvas.add_background_color(
            self.decoration()
                .background_color()
                .or(self.fallback_background()),
        );
        self.decoration().paint_with_background(
            &canvas,
            Vec2d {
                x: margin_left / ctx.scale + outset_left,
                y: margin_top / ctx.scale + outset_top,
            },
            ResolvedSize {
                width: render_extent(size.width, ctx.scale) / ctx.scale,
                height: render_extent(size.height, ctx.scale) / ctx.scale,
            },
            ctx.scale,
            self.fallback_background(),
        );
        canvas.finish();
        self.mark_decoration_recorded();
        let mut state = self.paint_state.get();
        state.revision = self.decoration().revision();
        state.background = ctx.paint_background;
        self.flags.set(self.flags.get() | RECORDED);
        self.paint_state.set(state);
    }

    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        let state = self.paint_state.get();
        self.flags.get() & RECORDED == 0
            || state.background != ctx.paint_background
            || self.retained_decoration_needs_recording()
            || state.revision != self.decoration().revision()
    }

    fn retained_clip(&self, ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
        let _ = ctx;
        None
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        let size = self.local_v2_paint_size(ctx)?;
        let mut outsets = self
            .decoration()
            .paint_outsets(
                ResolvedSize {
                    width: render_extent(size.width, ctx.scale) / ctx.scale,
                    height: render_extent(size.height, ctx.scale) / ctx.scale,
                },
                ctx.scale,
            )
            .map(finite_nonnegative);

        // The render node is as large as the container's content, but the
        // decoration is painted over the whole box: padding, border and the
        // margin it is offset by included. Damage comes from the node, so
        // without this a repaint stopped short of the box's right and bottom
        // edges and left the old border color there. The left and top are
        // already covered, since the node starts at the layout box.
        let (margin_left, margin_top, _, _) = self.margin(ctx);
        let content = self.content_size(ctx);
        let overshoot_right = (margin_left + size.width - content.width).max(0.0) / ctx.scale;
        let overshoot_bottom = (margin_top + size.height - content.height).max(0.0) / ctx.scale;
        if overshoot_right.is_finite() {
            outsets[2] += overshoot_right;
        }
        if overshoot_bottom.is_finite() {
            outsets[3] += overshoot_bottom;
        }
        Some(outsets)
    }

    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        if !std::ptr::eq(child, &self.child as &dyn Element) {
            return None;
        }
        self.local_v2_child_context(ctx)
    }

    fn retained_v2_child_clip_radius(&self, ctx: &BuildContext, child: &dyn Element) -> [f32; 4] {
        if !std::ptr::eq(child, &self.child as &dyn Element) {
            return [0.0; 4];
        }
        self.local_v2_geometry(ctx).map_or([0.0; 4], |geometry| {
            geometry
                .clip_radii
                .map(|radius| (radius / ctx.scale).min(LOCAL_V2_CONTAINER_EXTENT_LIMIT))
        })
    }

    fn retained_v2_child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        if !std::ptr::eq(child, &self.child as &dyn Element) {
            return None;
        }
        let scale = ctx.scale;
        let geometry = self.local_v2_geometry(ctx)?;
        let child_ctx = self.local_v2_child_context(ctx)?;
        let child_size = child.content_size(&child_ctx);
        let position = child.pos().unwrap_or_default();
        let x = geometry.child_origin.x + position.x;
        let y = geometry.child_origin.y + position.y;
        let bounds = aimer_cupid::draw_cmd_v2::Rect::new(
            x / scale,
            y / scale,
            child_size.width / scale,
            child_size.height / scale,
        );
        let clip = aimer_cupid::draw_cmd_v2::Rect::new(
            (geometry.clip_origin.x - x) / scale,
            (geometry.clip_origin.y - y) / scale,
            render_extent(geometry.clip_size.width, scale) / scale,
            render_extent(geometry.clip_size.height, scale) / scale,
        );
        let clip = geometry.clips_child.then_some(clip);
        let clip = match (clip, child.retained_clip(&child_ctx)) {
            (Some(clip), Some(child_clip)) => Some(intersect_retained_clip(clip, child_clip)),
            (clip, child_clip) => clip.or(child_clip),
        };
        Some((bounds, clip))
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let constraint = ctx.box_constraint;
        let scale = ctx.scale;
        let (m_left, m_top, m_right, m_bottom) = self.margin(ctx);
        let box_width = match self.width {
            Dimension::Px(width) => width * scale,
            Dimension::Percent(percent) => {
                constraint.max_width * (percent / 100.0) - (m_left + m_right)
            }
            Dimension::Auto => constraint.max_width - m_left - m_right,
        }
        .max(0.0);
        let box_height = match self.height {
            Dimension::Px(height) => height * scale,
            Dimension::Percent(percent) => {
                constraint.max_height * (percent / 100.0) - (m_top + m_bottom)
            }
            Dimension::Auto => constraint.max_height - m_top - m_bottom,
        }
        .max(0.0);
        let computed = self.computed_size(ctx);
        let draw_width = (computed.width - m_left - m_right).max(0.0);
        let draw_height = (computed.height - m_top - m_bottom).max(0.0);
        let (start_x, start_y) = ctx.canvas.get_transform_translation();
        if scale.is_finite() && scale > 0.0 {
            self.bounds.save(
                scale,
                start_x + m_left,
                start_y + m_top,
                draw_width,
                draw_height,
            );
        }

        let padding_left = self.padding.left.value(box_width, scale);
        let padding_top = self.padding.top.value(box_height, scale);
        let [border_left, border_top, border_right, border_bottom] =
            self.decoration_insets(box_width, box_height, scale);
        let content_width = (box_width
            - padding_left
            - border_left
            - self.padding.right.value(box_width, scale)
            - border_right)
            .max(0.0);
        let content_height = (box_height
            - padding_top
            - border_top
            - self.padding.bottom.value(box_height, scale)
            - border_bottom)
            .max(0.0);
        let inset_x = m_left + padding_left + border_left;
        let inset_y = m_top + padding_top + border_top;

        let mut child_ctx = ctx.clone();
        child_ctx.paint_background = self.child_paint_background(ctx);
        child_ctx.box_constraint.max_width = content_width;
        child_ctx.box_constraint.max_height = content_height;
        child_ctx.parent_size = ResolvedSize {
            width: content_width,
            height: content_height,
        };
        child_ctx.visible_rect = ctx
            .visible_rect
            .map(|(x, y, width, height)| (x - inset_x, y - inset_y, width, height));

        ctx.canvas.save();
        ctx.canvas.translate(Vec2d {
            x: inset_x,
            y: inset_y,
        });
        self.child.sync_paint_geometry(&child_ctx);
        ctx.canvas.restore();
    }
}

impl<T: Element, D: Decoration> VisitorElement for RawContainer<T, D> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    fn debug_name(&self) -> &'static str {
        if self.flags.get() & RAW != 0 {
            "RawContainer"
        } else {
            "Container"
        }
    }
}

impl<T: Element, D: Decoration> EventElement for RawContainer<T, D> {
    fn event_tree_role(&self) -> EventTreeRole {
        // A dynamic background must keep its routing role when it becomes transparent.
        if self.decoration_source.is_some() || self.is_opaque() {
            EventTreeRole::IndexedTarget
        } else {
            EventTreeRole::Transparent
        }
    }

    /// The opaque container has one child in both structural views.
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        // An opaque container occludes lower `Stack` layers: a wheel / trackpad
        // `Scroll` that lands on it must not fall through to a `Scrollable`
        // behind it. `dispatch_event` only calls `on_event` once the event
        // position is already inside our bounds and after our children had a
        // chance to consume it (deepest-first), so returning `true` here simply
        // absorbs a scroll that nothing in front of us wanted — this is exactly
        // why the website's scrollable used to scroll while the pointer was on
        // the opaque header. Non-scroll events keep falling through so existing
        // click/drag routing is unchanged.
        if matches!(event, ElementEvent::Scroll { .. }) && self.is_opaque() {
            return EventResult::consumed();
        }
        EventResult::ignored()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    /// An opaque container covers the layers behind it in a `Stack` for the
    /// pointer as well as for the eye: a press or a hover on its box belongs to
    /// it and what it holds, not to a card painted underneath. Only the box it
    /// paints counts, so a padding-only wrapper around a pill leaves the
    /// pointer free where nothing is drawn.
    #[inline]
    fn occludes_pointer_at(&self, pos: Vec2d) -> bool {
        self.is_opaque() && self.bounds.is_inside(pos.x, pos.y)
    }
}

mod layout;

#[cfg(test)]
mod tests;

impl<E: Element> RawContainer<E> {
    #[allow(dead_code)]
    pub(crate) fn new(child: E) -> Self {
        Self {
            margin: LayoutSpacing::default(),
            padding: LayoutSpacing::default(),
            width: Dimension::Auto,
            height: Dimension::Auto,
            box_decoration: BoxDecoration::default(),
            decoration_source: None,
            child,
            cache: LayoutCache::default(),
            flags: std::cell::Cell::new(RAW),
            bounds: InteractionBounds::new(),
            paint_state: std::cell::Cell::new(ContainerPaintState::new(None)),
        }
    }
}
