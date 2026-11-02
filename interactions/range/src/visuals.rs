//! Composable visual parts used by [`Slider`](super::Slider) controls.

use aimer_attribute::{Dimension, ResolvedSize, Size, Vec2d};
use aimer_canvas::Canvas;
use aimer_cupid::utilities::{Color as V2Color, Rect};
use aimer_widget::InteractionBounds;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, LayoutElement, PortableWidget, Rebuildable,
    VisitorElement, Widget,
};

/// Interaction state used to configure a slider's built-in visual children.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SliderVisualState {
    pub(crate) disabled: bool,
    pub(crate) pressed: bool,
    pub(crate) focused: bool,
}

/// The default unfilled track rendered by a [`Slider`](super::Slider).
pub(crate) struct SliderTrack;

impl SliderTrack {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Widget for SliderTrack {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawSliderTrack {
            bounds: InteractionBounds::new(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "SliderTrack"
    }
}

impl PortableWidget for SliderTrack {}

struct RawSliderTrack {
    bounds: InteractionBounds,
}

impl RawSliderTrack {
    fn local_size(&self, ctx: &BuildContext) -> Option<(f32, f32)> {
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let size = self.computed_size(ctx);
        let width = size.width / scale;
        let height = size.height / scale;
        (width.is_finite()
            && height.is_finite()
            && (0.0..=1_000_000.0).contains(&width)
            && (0.0..=1_000_000.0).contains(&height))
        .then_some((width, height))
    }
}

impl Drawable for RawSliderTrack {
    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("SliderTrack")
    }

    fn update(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
        // let radius = size.height.min(size.width) / 2.0;
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.local_size(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let (width, height) = self
            .local_size(ctx)
            .expect("SliderTrack v2 support must be checked before painting");
        let color = Color::Rgba(190, 196, 205, 255);
        let (red, green, blue, alpha) = color.to_rgba();
        let canvas = Canvas::of(ctx);
        canvas.fill_rect_styled(
            Rect::new(0.0, 0.0, width, height),
            V2Color::rgba8(red, green, blue, alpha),
            [height.min(width) / 2.0; 4],
            [0.0; 4],
            V2Color::transparent(),
            [0.0; 4],
            V2Color::transparent(),
        );
        canvas.finish();
    }
}

impl EventElement for RawSliderTrack {}

impl LayoutElement for RawSliderTrack {
    fn size(&self) -> Option<Size> {
        Some(Size::new(Dimension::Percent(100.0), Dimension::Px(4.0)))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize {
            width: ctx
                .box_constraint
                .max_width
                .clamp(ctx.box_constraint.min_width, ctx.box_constraint.max_width),
            height: (4.0 * ctx.scale.max(0.0))
                .clamp(ctx.box_constraint.min_height, ctx.box_constraint.max_height),
        }
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl Rebuildable for RawSliderTrack {}

impl VisitorElement for RawSliderTrack {
    fn debug_name(&self) -> &'static str {
        "SliderTrack"
    }
}

/// The default circular thumb rendered by a [`Slider`](super::Slider).
///
/// A thumb is also a complete widget, so it can be used directly in a custom
/// slider composition or configured with the builder methods below. When the
/// slider creates its own default thumb it supplies interaction state so the
/// focused and disabled colors continue to follow pointer and keyboard input.
#[derive(Clone)]
pub struct SliderThumb {
    size: f32,
    radius: f32,
    color: Color,
    focused_color: Color,
    disabled_color: Color,
}

impl SliderThumb {
    /// Creates the default 20-pixel circular thumb.
    #[inline]
    pub fn new() -> Self {
        Self {
            size: 20.0,
            radius: 10.0,
            color: Color::WHITE,
            focused_color: Color::Rgba(20, 80, 190, 255),
            disabled_color: Color::Rgba(150, 155, 165, 180),
        }
    }

    /// Sets the thumb's square edge length in logical pixels.
    #[inline]
    pub fn size(mut self, size: f32) -> Self {
        if size.is_finite() && size >= 0.0 {
            self.size = size;
        }
        self
    }

    /// Sets the thumb corner radius in logical pixels.
    #[inline]
    pub fn radius(mut self, radius: f32) -> Self {
        if radius.is_finite() && radius >= 0.0 {
            self.radius = radius;
        }
        self
    }

    /// Sets the normal thumb color.
    #[inline]
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Sets the thumb color while a connected slider owns keyboard focus.
    ///
    /// The default thumb uses this color when the slider owns keyboard focus.
    /// A thumb passed as a custom child is rendered with its configured
    /// [`Self::color`].
    #[inline]
    pub fn focused_color(mut self, color: Color) -> Self {
        self.focused_color = color;
        self
    }

    /// Sets the thumb color while a connected slider is disabled.
    ///
    /// The default thumb uses this color while the slider is disabled. A thumb
    /// passed as a custom child is rendered with its configured [`Self::color`].
    #[inline]
    pub fn disabled_color(mut self, color: Color) -> Self {
        self.disabled_color = color;
        self
    }

    pub(crate) fn for_state(state: SliderVisualState) -> Self {
        let mut thumb = Self::new();
        if state.disabled {
            thumb.color = thumb.disabled_color;
        } else if state.focused {
            thumb.color = thumb.focused_color;
        }
        thumb
    }
}

impl Default for SliderThumb {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for SliderThumb {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawSliderThumb {
            size: self.size,
            radius: self.radius,
            color: self.color,
            bounds: InteractionBounds::new(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "SliderThumb"
    }
}

impl PortableWidget for SliderThumb {}

struct RawSliderThumb {
    size: f32,
    radius: f32,
    color: Color,
    bounds: InteractionBounds,
}

impl RawSliderThumb {
    fn local_size(&self, ctx: &BuildContext) -> Option<(f32, f32)> {
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let size = self.computed_size(ctx);
        let width = size.width / scale;
        let height = size.height / scale;
        (width.is_finite()
            && height.is_finite()
            && (0.0..=1_000_000.0).contains(&width)
            && (0.0..=1_000_000.0).contains(&height))
        .then_some((width, height))
    }
}

impl Drawable for RawSliderThumb {
    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("SliderThumb")
    }

    fn update(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);

        let radius = (self.radius * ctx.scale)
            .min(size.width / 2.0)
            .min(size.height / 2.0)
            .max(0.0);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.size.is_finite() && self.radius.is_finite() && self.local_size(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let (width, height) = self
            .local_size(ctx)
            .expect("SliderThumb v2 support must be checked before painting");
        let (red, green, blue, alpha) = self.color.to_rgba();
        let radius = self.radius.max(0.0).min(width / 2.0).min(height / 2.0);
        let canvas = Canvas::of(ctx);
        canvas.fill_rect_styled(
            Rect::new(0.0, 0.0, width, height),
            V2Color::rgba8(red, green, blue, alpha),
            [radius; 4],
            [0.0; 4],
            V2Color::transparent(),
            [0.0; 4],
            V2Color::transparent(),
        );
        canvas.finish();
    }
}

impl EventElement for RawSliderThumb {}

impl LayoutElement for RawSliderThumb {
    fn size(&self) -> Option<Size> {
        Some(Size::new(self.size, self.size))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let requested = (self.size.max(0.0) * ctx.scale.max(0.0)).max(0.0);
        ResolvedSize {
            width: requested.clamp(ctx.box_constraint.min_width, ctx.box_constraint.max_width),
            height: requested.clamp(ctx.box_constraint.min_height, ctx.box_constraint.max_height),
        }
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl Rebuildable for RawSliderThumb {}

impl VisitorElement for RawSliderThumb {
    fn debug_name(&self) -> &'static str {
        "SliderThumb"
    }
}

/// The default active trail rendered by a [`Slider`](super::Slider).
///
/// The slider positions and clips this widget to the portion represented by
/// the current value. Consequently a custom trail can use gradients, images,
/// or any other widget while retaining the slider's pointer and keyboard
/// behavior.
#[derive(Clone)]
pub struct SliderTrail {
    width: Dimension,
    height: f32,
    radius: f32,
    color: Color,
    pressed_color: Color,
    disabled_color: Color,
}

impl SliderTrail {
    /// Creates a full-width, four-pixel rounded trail.
    #[inline]
    pub fn new() -> Self {
        Self {
            width: Dimension::Percent(100.0),
            height: 4.0,
            radius: 2.0,
            color: Color::Rgba(35, 110, 220, 255),
            pressed_color: Color::Rgba(20, 80, 190, 255),
            disabled_color: Color::Rgba(35, 110, 220, 100),
        }
    }

    /// Sets the trail width before the slider clips it to the active segment.
    #[inline]
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the trail height in logical pixels.
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() && height >= 0.0 {
            self.height = height;
        }
        self
    }

    /// Sets the trail corner radius in logical pixels.
    #[inline]
    pub fn radius(mut self, radius: f32) -> Self {
        if radius.is_finite() && radius >= 0.0 {
            self.radius = radius;
        }
        self
    }

    /// Sets the normal active-trail color.
    #[inline]
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Sets the active-trail color while a connected slider is pressed.
    ///
    /// The slider supplies interaction state to its built-in trail. A trail
    /// passed as a custom child is rendered with its configured normal color.
    #[inline]
    pub fn pressed_color(mut self, color: Color) -> Self {
        self.pressed_color = color;
        self
    }

    /// Sets the active-trail color while a connected slider is disabled.
    ///
    /// The slider supplies interaction state to its built-in trail. A trail
    /// passed as a custom child is rendered with its configured normal color.
    #[inline]
    pub fn disabled_color(mut self, color: Color) -> Self {
        self.disabled_color = color;
        self
    }

    pub(crate) fn for_state(state: SliderVisualState) -> Self {
        let mut trail = Self::new();
        if state.disabled {
            trail.color = trail.disabled_color;
        } else if state.pressed {
            trail.color = trail.pressed_color;
        }
        trail
    }
}

impl Default for SliderTrail {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for SliderTrail {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawSliderTrail {
            width: self.width,
            height: self.height,
            radius: self.radius,
            color: self.color,
            bounds: InteractionBounds::new(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "SliderTrail"
    }
}

impl PortableWidget for SliderTrail {}

struct RawSliderTrail {
    width: Dimension,
    height: f32,
    radius: f32,
    color: Color,
    bounds: InteractionBounds,
}

impl Drawable for RawSliderTrail {
    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("SliderTrail")
    }

    fn update(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);

        let radius = (self.radius * ctx.scale)
            .min(size.width / 2.0)
            .min(size.height / 2.0)
            .max(0.0);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
            return false;
        }
        let size = self.computed_size(ctx);
        let width = size.width / ctx.scale;
        let height = size.height / ctx.scale;
        width.is_finite()
            && height.is_finite()
            && (0.0..=1_000_000.0).contains(&width)
            && (0.0..=1_000_000.0).contains(&height)
            && self.radius.is_finite()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let width = size.width / ctx.scale;
        let height = size.height / ctx.scale;
        let (red, green, blue, alpha) = self.color.to_rgba();
        let radius = self.radius.max(0.0).min(width / 2.0).min(height / 2.0);
        let canvas = Canvas::of(ctx);
        canvas.fill_rect_styled(
            Rect::new(0.0, 0.0, width, height),
            V2Color::rgba8(red, green, blue, alpha),
            [radius; 4],
            [0.0; 4],
            V2Color::transparent(),
            [0.0; 4],
            V2Color::transparent(),
        );
        canvas.finish();
    }
}

impl EventElement for RawSliderTrail {}

impl LayoutElement for RawSliderTrail {
    fn size(&self) -> Option<Size> {
        Some(Size::new(self.width, Dimension::Px(self.height)))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let width = self
            .width
            .resolve(ctx.box_constraint.max_width, ctx.scale)
            .max(0.0);
        let height = (self.height.max(0.0) * ctx.scale.max(0.0)).max(0.0);
        ResolvedSize {
            width: width.clamp(ctx.box_constraint.min_width, ctx.box_constraint.max_width),
            height: height.clamp(ctx.box_constraint.min_height, ctx.box_constraint.max_height),
        }
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl Rebuildable for RawSliderTrail {}

impl VisitorElement for RawSliderTrail {
    fn debug_name(&self) -> &'static str {
        "SliderTrail"
    }
}
