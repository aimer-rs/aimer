use std::cell::Cell;

use aimer_canvas::{Canvas, MaterialDrawRequest, MaterialKind};
use aimer_decoration::{AnyDecor, BorderRadius, Decoration};
use aimer_widget::base::{BuildContext, Color, ResolvedSize, Size, Vec2d};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement, PortableWidget,
    Rebuildable, RequiredChild, VisitorElement, Widget,
};

/// The policy used by a material when deciding whether it may animate.
///
/// The policy is deliberately platform-neutral. A host can choose the reduced
/// policy for an accessibility or battery preference without exposing a
/// renderer or system visual-effect handle to the widget tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaterialMotionPolicy {
    /// Permit the material's configured animation and interaction response.
    #[default]
    Full,
    /// Disable animated fields and keep only the static surface treatment.
    Reduced,
}

/// Plain, bounded values used by [`Glass`] and as the base for [`Liquid`].
///
/// Values are normalized at construction and again when a widget becomes an
/// element. This keeps a stale or externally assembled value from introducing
/// a non-finite number into a canvas command. The bounds are intentionally
/// conservative so a single surface cannot request an unbounded blur or
/// sampling budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassMaterial {
    tint: Color,
    opacity: f32,
    blur_radius: f32,
    saturation: f32,
    brightness: f32,
    contrast: f32,
    border_width: f32,
    border_color: Color,
    border_radius: Option<BorderRadius>,
    shadow_color: Color,
    shadow_blur: f32,
    elevation: f32,
    edge_lighting: f32,
    specular_highlight: f32,
    quality: u8,
    motion_policy: MaterialMotionPolicy,
}

impl GlassMaterial {
    /// The default backdrop blur radius in logical pixels.
    pub const DEFAULT_BLUR_RADIUS: f32 = 16.0;
    /// The largest blur radius accepted by the portable material contract.
    pub const MAX_BLUR_RADIUS: f32 = 96.0;
    /// The largest corner radius accepted before it is resolved against bounds.
    pub const MAX_CORNER_RADIUS: f32 = 1_024.0;
    /// The corner radius used when neither a radius nor a decoration declares a shape.
    pub const DEFAULT_CORNER_RADIUS: f32 = 16.0;
    /// The largest shadow blur accepted by the fallback path.
    pub const MAX_SHADOW_BLUR: f32 = 128.0;
    /// The largest fallback elevation accepted by the material contract.
    pub const MAX_ELEVATION: f32 = 64.0;
    /// The largest normalized raised-edge lighting strength.
    pub const MAX_EDGE_LIGHTING: f32 = 1.0;
    /// The largest normalized surface highlight strength.
    pub const MAX_SPECULAR_HIGHLIGHT: f32 = 1.0;
    /// The minimum number of samples in the portable quality policy.
    pub const MIN_QUALITY: u8 = 1;
    /// The maximum number of samples in the portable quality policy.
    pub const MAX_QUALITY: u8 = 4;

    /// Creates the restrained default translucent surface.
    #[inline]
    pub fn new() -> Self {
        Self {
            tint: Color::Rgba(255, 255, 255, 255),
            opacity: 0.72,
            blur_radius: 16.0,
            saturation: 1.0,
            brightness: 1.0,
            contrast: 1.0,
            border_width: 1.0,
            border_color: Color::Rgba(255, 255, 255, 96),
            border_radius: None,
            shadow_color: Color::Rgba(0, 0, 0, 72),
            shadow_blur: 16.0,
            elevation: 4.0,
            edge_lighting: 0.35,
            specular_highlight: 0.16,
            quality: 2,
            motion_policy: MaterialMotionPolicy::Full,
        }
    }

    /// Sets the tint used by the surface.
    #[inline]
    pub fn tint(mut self, tint: impl Into<Color>) -> Self {
        self.tint = tint.into();
        self
    }

    /// Sets the surface opacity in `0.0..=1.0`.
    ///
    /// Non-finite values use the default opacity and finite values are clamped.
    #[inline]
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = normalize(opacity, 0.72, 0.0, 1.0);
        self
    }

    /// Sets the backdrop blur radius in logical pixels.
    #[inline]
    pub fn blur_radius(mut self, blur_radius: f32) -> Self {
        self.blur_radius = normalize(
            blur_radius,
            Self::DEFAULT_BLUR_RADIUS,
            0.0,
            Self::MAX_BLUR_RADIUS,
        );
        self
    }

    /// Sets the backdrop blur as a normalized intensity from `0.0` to `1.0`.
    ///
    /// `0.0` keeps the captured backdrop sharp and `1.0` uses the maximum
    /// bounded blur radius. [`Self::blur_radius`] remains available when an
    /// exact logical-pixel radius is preferred.
    #[inline]
    pub fn blur_intensity(mut self, intensity: f32) -> Self {
        let default_intensity = Self::DEFAULT_BLUR_RADIUS / Self::MAX_BLUR_RADIUS;
        let intensity = normalize(intensity, default_intensity, 0.0, 1.0);
        self.blur_radius = intensity * Self::MAX_BLUR_RADIUS;
        self
    }

    /// Sets the backdrop saturation multiplier.
    #[inline]
    pub fn saturation(mut self, saturation: f32) -> Self {
        self.saturation = normalize(saturation, 1.0, 0.0, 3.0);
        self
    }

    /// Sets the backdrop brightness multiplier.
    #[inline]
    pub fn brightness(mut self, brightness: f32) -> Self {
        self.brightness = normalize(brightness, 1.0, 0.0, 3.0);
        self
    }

    /// Sets the backdrop contrast multiplier.
    #[inline]
    pub fn contrast(mut self, contrast: f32) -> Self {
        self.contrast = normalize(contrast, 1.0, 0.0, 3.0);
        self
    }

    /// Sets the uniform border width in logical pixels.
    #[inline]
    pub fn border_width(mut self, border_width: f32) -> Self {
        self.border_width = normalize(border_width, 1.0, 0.0, 16.0);
        self
    }

    /// Sets the border highlight color.
    #[inline]
    pub fn border_color(mut self, border_color: impl Into<Color>) -> Self {
        self.border_color = border_color.into();
        self
    }

    /// Sets the corner radii of the surface, per corner, in logical pixels.
    ///
    /// Percentages resolve against the surface size. Each radius is bounded to
    /// `0.0..=MAX_CORNER_RADIUS`, nonfinite values give square corners, and the
    /// canvas fallback fits radii within half the shorter side. An explicit
    /// radius overrides any shape declared by the widget's decoration; without
    /// either the surface uses [`Self::DEFAULT_CORNER_RADIUS`].
    #[inline]
    pub fn border_radius(mut self, border_radius: impl Into<BorderRadius>) -> Self {
        self.border_radius = Some(border_radius.into());
        self
    }

    /// Sets the shadow color used by the GPU path and bounded fallback.
    #[inline]
    pub fn shadow_color(mut self, shadow_color: impl Into<Color>) -> Self {
        self.shadow_color = shadow_color.into();
        self
    }

    /// Sets the shadow blur radius in logical pixels.
    #[inline]
    pub fn shadow_blur(mut self, shadow_blur: f32) -> Self {
        self.shadow_blur = normalize(shadow_blur, 16.0, 0.0, Self::MAX_SHADOW_BLUR);
        self
    }

    /// Sets the non-negative shadow elevation in logical pixels.
    #[inline]
    pub fn elevation(mut self, elevation: f32) -> Self {
        self.elevation = normalize(elevation, 4.0, 0.0, Self::MAX_ELEVATION);
        self
    }

    /// Sets the normalized lighting strength around the raised glass edge.
    #[inline]
    pub fn edge_lighting(mut self, strength: f32) -> Self {
        self.edge_lighting = normalize(strength, 0.35, 0.0, Self::MAX_EDGE_LIGHTING);
        self
    }

    /// Sets the normalized strength of highlights across the glass surface.
    #[inline]
    pub fn specular_highlight(mut self, strength: f32) -> Self {
        self.specular_highlight = normalize(strength, 0.16, 0.0, Self::MAX_SPECULAR_HIGHLIGHT);
        self
    }

    /// Sets the bounded sampling quality. Values outside `1..=4` are clamped.
    #[inline]
    pub fn quality(mut self, quality: u8) -> Self {
        self.quality = quality.clamp(Self::MIN_QUALITY, Self::MAX_QUALITY);
        self
    }

    /// Sets the reduced-motion policy.
    #[inline]
    pub fn motion_policy(mut self, motion_policy: MaterialMotionPolicy) -> Self {
        self.motion_policy = motion_policy;
        self
    }

    /// Returns the configured tint.
    #[inline]
    pub fn tint_color(self) -> Color {
        self.tint
    }

    /// Returns the normalized opacity.
    #[inline]
    pub fn opacity_value(self) -> f32 {
        self.opacity
    }

    /// Returns the normalized blur radius.
    #[inline]
    pub fn blur_radius_value(self) -> f32 {
        self.blur_radius
    }

    /// Returns the normalized backdrop blur intensity.
    #[inline]
    pub fn blur_intensity_value(self) -> f32 {
        let default_intensity = Self::DEFAULT_BLUR_RADIUS / Self::MAX_BLUR_RADIUS;
        normalize(
            self.blur_radius / Self::MAX_BLUR_RADIUS,
            default_intensity,
            0.0,
            1.0,
        )
    }

    /// Returns the normalized saturation multiplier.
    #[inline]
    pub fn saturation_value(self) -> f32 {
        self.saturation
    }

    /// Returns the normalized brightness multiplier.
    #[inline]
    pub fn brightness_value(self) -> f32 {
        self.brightness
    }

    /// Returns the normalized contrast multiplier.
    #[inline]
    pub fn contrast_value(self) -> f32 {
        self.contrast
    }

    /// Returns the normalized border width.
    #[inline]
    pub fn border_width_value(self) -> f32 {
        self.border_width
    }

    /// Returns the border color.
    #[inline]
    pub fn border_color_value(self) -> Color {
        self.border_color
    }

    /// Returns the explicitly configured radii, if any.
    #[inline]
    pub fn border_radius_value(self) -> Option<BorderRadius> {
        self.border_radius
    }

    /// Resolves the surface's corner radii in logical pixels.
    ///
    /// Precedence: the explicit [`Self::border_radius`], then the shape that
    /// the decorations painted so far declared on `canvas` (a declared square
    /// counts), then `fallback`. Results are bounded to `0.0..=MAX_CORNER_RADIUS`.
    pub(super) fn shape_radii(self, canvas: &Canvas, size: ResolvedSize, fallback: f32) -> [f32; 4] {
        let radii = match self.border_radius {
            Some(radius) => radius.resolve(size.width, size.height, 1.0),
            None => canvas.declared_corner_radii().unwrap_or([fallback; 4]),
        };
        radii.map(|radius| normalize(radius, 0.0, 0.0, Self::MAX_CORNER_RADIUS))
    }

    /// Returns the shadow color.
    #[inline]
    pub fn shadow_color_value(self) -> Color {
        self.shadow_color
    }

    /// Returns the normalized shadow blur.
    #[inline]
    pub fn shadow_blur_value(self) -> f32 {
        self.shadow_blur
    }

    /// Returns the normalized elevation.
    #[inline]
    pub fn elevation_value(self) -> f32 {
        self.elevation
    }

    /// Returns the normalized raised-edge lighting strength.
    #[inline]
    pub fn edge_lighting_value(self) -> f32 {
        self.edge_lighting
    }

    /// Returns the normalized surface highlight strength.
    #[inline]
    pub fn specular_highlight_value(self) -> f32 {
        self.specular_highlight
    }

    /// Returns the bounded quality value.
    #[inline]
    pub fn quality_value(self) -> u8 {
        self.quality
    }

    /// Returns the configured motion policy.
    #[inline]
    pub fn motion_policy_value(self) -> MaterialMotionPolicy {
        self.motion_policy
    }

    pub(crate) fn normalized(mut self) -> Self {
        self.opacity = normalize(self.opacity, 0.72, 0.0, 1.0);
        self.blur_radius = normalize(
            self.blur_radius,
            Self::DEFAULT_BLUR_RADIUS,
            0.0,
            Self::MAX_BLUR_RADIUS,
        );
        self.saturation = normalize(self.saturation, 1.0, 0.0, 3.0);
        self.brightness = normalize(self.brightness, 1.0, 0.0, 3.0);
        self.contrast = normalize(self.contrast, 1.0, 0.0, 3.0);
        self.border_width = normalize(self.border_width, 1.0, 0.0, 16.0);
        self.shadow_blur = normalize(self.shadow_blur, 16.0, 0.0, Self::MAX_SHADOW_BLUR);
        self.elevation = normalize(self.elevation, 4.0, 0.0, Self::MAX_ELEVATION);
        self.edge_lighting = normalize(self.edge_lighting, 0.35, 0.0, Self::MAX_EDGE_LIGHTING);
        self.specular_highlight = normalize(
            self.specular_highlight,
            0.16,
            0.0,
            Self::MAX_SPECULAR_HIGHLIGHT,
        );
        self.quality = self.quality.clamp(Self::MIN_QUALITY, Self::MAX_QUALITY);
        self
    }

    fn fallback_tint(self) -> Color {
        with_opacity(self.tint, self.opacity)
    }

    fn fallback_border(self) -> Color {
        with_opacity(self.border_color, self.opacity)
    }

    fn fallback_shadow(self) -> Color {
        with_opacity(self.shadow_color, self.opacity)
    }
}

impl Default for GlassMaterial {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds the renderer-neutral request shared by Glass and Liquid.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_material_request(
    kind: MaterialKind,
    base: GlassMaterial,
    size: ResolvedSize,
    corner_radii: [f32; 4],
    distortion_strength: f32,
    edge_lighting: f32,
    specular_highlight: f32,
    animation_speed: f32,
    animation_time: f32,
    interaction: f32,
    // `[blob_amount, blob_seed, magnification, tip_pull, chromatic_aberration,
    // bevel_radius]`: Liquid-only silhouette/refraction shape. Glass passes
    // `[0.0; 6]`.
    liquid_shape: [f32; 6],
) -> MaterialDrawRequest {
    let mut request = MaterialDrawRequest::new(kind, [0.0, 0.0, size.width, size.height]);
    request.tint = color_to_array(base.tint_color());
    request.opacity = base.opacity_value();
    request.blur_radius = base.blur_radius_value();
    request.saturation = base.saturation_value();
    request.brightness = base.brightness_value();
    request.contrast = base.contrast_value();
    request.border_color = color_to_array(base.border_color_value());
    request.border_width = base.border_width_value();
    request.corner_radii = corner_radii;
    request.shadow_color = color_to_array(base.shadow_color_value());
    request.shadow_blur = base.shadow_blur_value();
    request.elevation = base.elevation_value();
    request.distortion_strength = distortion_strength;
    request.edge_lighting = edge_lighting;
    request.specular_highlight = specular_highlight;
    request.animation_speed = animation_speed;
    request.animation_time = animation_time;
    request.interaction = interaction;
    request.blob_amount = liquid_shape[0];
    request.blob_seed = liquid_shape[1];
    request.magnification = liquid_shape[2];
    request.tip_pull = liquid_shape[3];
    request.chromatic_aberration = liquid_shape[4];
    request.bevel_radius = liquid_shape[5];
    request.quality = base.quality_value();
    request.motion_policy = match base.motion_policy_value() {
        MaterialMotionPolicy::Full => aimer_canvas::MaterialMotionPolicy::Full,
        MaterialMotionPolicy::Reduced => aimer_canvas::MaterialMotionPolicy::Reduced,
    };
    request.normalized()
}

#[inline]
fn color_to_array(color: Color) -> [f32; 4] {
    let (red, green, blue, alpha) = color.to_rgba();
    [
        f32::from(red) / 255.0,
        f32::from(green) / 255.0,
        f32::from(blue) / 255.0,
        f32::from(alpha) / 255.0,
    ]
}

/// A translucent, single-child surface.
///
/// `Glass` is a visual wrapper: it has no independent event, focus, or
/// accessibility node. The child is the final type-state transition and is
/// moved into the retained element without cloning. The bounded canvas fallback
/// remains underneath a Cupid-owned material enhancement, without changing
/// this widget's layout or interaction behavior.
pub struct Glass<W = RequiredChild> {
    child: W,
    material: GlassMaterial,
    decoration: Option<AnyDecor>,
}

impl Glass {
    /// Creates a default glass builder without a child.
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            material: GlassMaterial::new(),
            decoration: None,
        }
    }

    /// Replaces the material configuration.
    #[inline]
    pub fn material(mut self, material: GlassMaterial) -> Self {
        self.material = material.normalized();
        self
    }

    /// Sets the material tint.
    #[inline]
    pub fn tint(mut self, tint: impl Into<Color>) -> Self {
        self.material = self.material.tint(tint);
        self
    }

    /// Sets the material opacity.
    #[inline]
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.material = self.material.opacity(opacity);
        self
    }

    /// Sets the backdrop blur radius.
    #[inline]
    pub fn blur_radius(mut self, blur_radius: f32) -> Self {
        self.material = self.material.blur_radius(blur_radius);
        self
    }

    /// Sets the backdrop blur as a normalized intensity from `0.0` to `1.0`.
    #[inline]
    pub fn blur_intensity(mut self, intensity: f32) -> Self {
        self.material = self.material.blur_intensity(intensity);
        self
    }

    /// Sets backdrop saturation, brightness, and contrast independently.
    #[inline]
    pub fn saturation(mut self, saturation: f32) -> Self {
        self.material = self.material.saturation(saturation);
        self
    }

    /// Sets the backdrop brightness multiplier.
    #[inline]
    pub fn brightness(mut self, brightness: f32) -> Self {
        self.material = self.material.brightness(brightness);
        self
    }

    /// Sets the backdrop contrast multiplier.
    #[inline]
    pub fn contrast(mut self, contrast: f32) -> Self {
        self.material = self.material.contrast(contrast);
        self
    }

    /// Sets the border width.
    #[inline]
    pub fn border_width(mut self, border_width: f32) -> Self {
        self.material = self.material.border_width(border_width);
        self
    }

    /// Sets the border highlight color.
    #[inline]
    pub fn border_color(mut self, border_color: impl Into<Color>) -> Self {
        self.material = self.material.border_color(border_color);
        self
    }

    /// Sets the corner radii of the surface; see [`GlassMaterial::border_radius`].
    #[inline]
    pub fn border_radius(mut self, border_radius: impl Into<BorderRadius>) -> Self {
        self.material = self.material.border_radius(border_radius);
        self
    }

    /// Paints `decoration` behind the glass surface and adopts its shape.
    ///
    /// Use a stack such as `vec![BoxShadow.notype(), BorderRadius::from(20).notype()]`
    /// for several layers. Unless [`Self::border_radius`] is set, the surface
    /// takes the corner radii that `BoxDecoration` or `BorderRadius` layers
    /// declared, so the glass and its decoration always share one outline.
    /// The decoration is retained as built: rebuild the widget to change it.
    /// It never affects layout, clipping or events.
    ///
    /// ```
    /// use aimer_container::{Glass, SizedBox};
    /// use aimer_decoration::{BorderRadius, Decoration};
    ///
    /// let pill = Glass::new()
    ///     .decoration(BorderRadius::from(20))
    ///     .child(SizedBox::new().width(120).height(40));
    /// ```
    #[inline]
    pub fn decoration(mut self, decoration: impl Decoration + 'static) -> Self {
        self.decoration = Some(decoration.notype());
        self
    }

    /// Sets shadow color, blur, and elevation.
    #[inline]
    pub fn shadow_color(mut self, shadow_color: impl Into<Color>) -> Self {
        self.material = self.material.shadow_color(shadow_color);
        self
    }

    /// Sets shadow blur.
    #[inline]
    pub fn shadow_blur(mut self, shadow_blur: f32) -> Self {
        self.material = self.material.shadow_blur(shadow_blur);
        self
    }

    /// Sets shadow elevation.
    #[inline]
    pub fn elevation(mut self, elevation: f32) -> Self {
        self.material = self.material.elevation(elevation);
        self
    }

    /// Sets normalized lighting around the raised glass edge.
    #[inline]
    pub fn edge_lighting(mut self, strength: f32) -> Self {
        self.material = self.material.edge_lighting(strength);
        self
    }

    /// Sets normalized highlights across the glass surface.
    #[inline]
    pub fn specular_highlight(mut self, strength: f32) -> Self {
        self.material = self.material.specular_highlight(strength);
        self
    }

    /// Sets the bounded material quality.
    #[inline]
    pub fn quality(mut self, quality: u8) -> Self {
        self.material = self.material.quality(quality);
        self
    }

    /// Sets the reduced-motion policy.
    #[inline]
    pub fn motion_policy(mut self, motion_policy: MaterialMotionPolicy) -> Self {
        self.material = self.material.motion_policy(motion_policy);
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Glass<W> {
        Glass {
            child,
            material: self.material.normalized(),
            decoration: self.decoration,
        }
    }

    /// Attaches a child and erases the resulting widget type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl Default for Glass {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Widget + 'static> PortableWidget for Glass<W> {}

impl<W: Widget + 'static> Widget for Glass<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawGlass {
            child: self.child.to_element(ctx),
            material: self.material.normalized(),
            decoration: self.decoration,
            recorded: Cell::new(None),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Glass"
    }
}

struct RawGlass {
    child: AnyElement,
    material: GlassMaterial,
    decoration: Option<AnyDecor>,
    recorded: Cell<Option<u64>>,
}

impl Rebuildable for RawGlass {}

impl Drawable for RawGlass {
    fn update(&self, ctx: &BuildContext) {
        // The frosted surface is painted by `paint_local_v2`; the child keeps its
        // own render node above it.
        self.child.update(ctx);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && logical_size(ctx, self.child.computed_size(ctx)).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        let material = self.material.normalized();
        if let Some(size) = logical_size(ctx, self.child.computed_size(ctx))
            && size.width > 0.0
            && size.height > 0.0
        {
            paint_decoration(self.decoration.as_ref(), &canvas, size, ctx.scale);
            let shape = material.shape_radii(&canvas, size, GlassMaterial::DEFAULT_CORNER_RADIUS);
            let radii = resolved_radii(shape, size);
            let shadow = material.fallback_shadow();
            if shadow.alpha() != 0 && material.shadow_blur_value() > 0.0 {
                canvas.draw_shadow_rect(
                    aimer_cupid::utilities::Rect::new(0.0, 0.0, size.width, size.height),
                    v2_color(shadow),
                    [
                        0.0,
                        material.elevation_value(),
                        material.shadow_blur_value(),
                        0.0,
                    ],
                    radii,
                    false,
                    [0.0; 3],
                );
            }
            let border = material.fallback_border();
            let border_width = if border.alpha() == 0 {
                [0.0; 4]
            } else {
                [material.border_width_value(); 4]
            };
            canvas.fill_rect_styled(
                aimer_cupid::utilities::Rect::new(0.0, 0.0, size.width, size.height),
                v2_color(material.fallback_tint()),
                radii,
                border_width,
                v2_color(border),
                [0.0; 4],
                aimer_cupid::utilities::Color::transparent(),
            );
            canvas.draw_material(build_material_request(
                MaterialKind::Glass,
                material,
                size,
                shape,
                0.0,
                material.edge_lighting_value(),
                material.specular_highlight_value(),
                0.0,
                0.0,
                0.0,
                [0.0; 6],
            ));
        }
        canvas.finish();
        self.recorded.set(self.decoration.as_ref().map(|decoration| decoration.revision()));
    }

    #[inline]
    fn local_v2_paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        decoration_needs_recording(self.decoration.as_ref(), self.recorded.get())
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        surface_outsets(
            self.material.normalized(),
            self.decoration.as_ref(),
            ctx,
            self.child.computed_size(ctx),
        )
    }
}

impl EventElement for RawGlass {
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    #[inline]
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl LayoutElement for RawGlass {
    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}

impl VisitorElement for RawGlass {
    #[inline]
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Glass"
    }
}

#[inline]
fn normalize(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback.clamp(min, max)
    }
}

#[inline]
fn with_opacity(color: Color, opacity: f32) -> Color {
    let (red, green, blue, alpha) = color.to_rgba();
    let alpha = (f32::from(alpha) * opacity).round().clamp(0.0, 255.0) as u8;
    Color::Rgba(red, green, blue, alpha)
}

#[inline]
pub(super) fn logical_size(ctx: &BuildContext, size: ResolvedSize) -> Option<ResolvedSize> {
    let scale = ctx.scale;
    if !scale.is_finite() || scale <= 0.0 || !size.width.is_finite() || !size.height.is_finite() {
        return None;
    }
    Some(ResolvedSize {
        width: size.width / scale,
        height: size.height / scale,
    })
}

pub(super) fn shadow_outsets(material: GlassMaterial) -> Option<[f32; 4]> {
    if material.fallback_shadow().alpha() == 0 || material.shadow_blur_value() <= 0.0 {
        return None;
    }
    let blur = (material.shadow_blur_value() * 3.0).ceil();
    let elevation = material.elevation_value();
    Some([blur, blur, blur, blur + elevation])
}

/// Paints a surface's optional decoration behind its glass.
pub(super) fn paint_decoration(
    decoration: Option<&AnyDecor>, canvas: &Canvas, size: ResolvedSize, scale: f32,
) {
    if let Some(decoration) = decoration {
        decoration.paint_layer(canvas, Vec2d::default(), size, scale);
    }
}

/// Whether the decoration's revision moved since it was last recorded.
#[inline]
pub(super) fn decoration_needs_recording(
    decoration: Option<&AnyDecor>, recorded: Option<u64>,
) -> bool {
    decoration.is_some_and(|decoration| recorded != Some(decoration.revision()))
}

/// Paint outsets of the shadow and the decoration, whichever reaches farther.
pub(super) fn surface_outsets(
    material: GlassMaterial, decoration: Option<&AnyDecor>, ctx: &BuildContext, size: ResolvedSize,
) -> Option<[f32; 4]> {
    let shadow = shadow_outsets(material);
    let Some(decoration) = decoration else {
        return shadow;
    };
    let Some(size) = logical_size(ctx, size) else {
        return shadow;
    };
    let mut merged = shadow.unwrap_or([0.0; 4]);
    for (outset, candidate) in merged.iter_mut().zip(decoration.paint_outsets(size, ctx.scale)) {
        // A nonfinite layer must not erase a valid finite outset.
        if candidate.is_finite() {
            *outset = outset.max(candidate);
        }
    }
    (merged != [0.0; 4]).then_some(merged)
}

pub(super) fn v2_color(color: Color) -> aimer_cupid::utilities::Color {
    let (red, green, blue, alpha) = color.to_rgba();
    aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha)
}

pub(super) fn resolved_radii(radii: [f32; 4], size: ResolvedSize) -> [f32; 4] {
    let limit = (size.width.min(size.height) * 0.5).max(0.0);
    radii.map(|radius| radius.min(limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_values_are_finite_and_bounded() {
        let material = GlassMaterial::new()
            .opacity(f32::NAN)
            .blur_radius(f32::INFINITY)
            .saturation(-1.0)
            .brightness(99.0)
            .contrast(f32::NEG_INFINITY)
            .shadow_blur(f32::INFINITY)
            .elevation(-5.0)
            .edge_lighting(2.0)
            .specular_highlight(f32::NAN)
            .quality(0);

        assert_eq!(material.opacity_value(), 0.72);
        assert_eq!(material.blur_radius_value(), 16.0);
        assert_eq!(material.saturation_value(), 0.0);
        assert_eq!(material.brightness_value(), 3.0);
        assert_eq!(material.contrast_value(), 1.0);
        assert_eq!(material.shadow_blur_value(), 16.0);
        assert_eq!(material.elevation_value(), 0.0);
        assert_eq!(
            material.edge_lighting_value(),
            GlassMaterial::MAX_EDGE_LIGHTING
        );
        assert_eq!(material.specular_highlight_value(), 0.16);
        assert_eq!(material.quality_value(), GlassMaterial::MIN_QUALITY);
    }

    #[test]
    fn border_radius_is_unset_until_configured() {
        assert_eq!(GlassMaterial::new().border_radius_value(), None);
        let radius = BorderRadius::new().top_left(4.0).bottom_right(9.0);
        assert_eq!(
            GlassMaterial::new().border_radius(radius).border_radius_value(),
            Some(radius)
        );
        assert_eq!(GlassMaterial::new().border_radius(12).border_radius_value(), Some(12.into()));
    }

    #[test]
    fn blur_intensity_is_a_bounded_public_alias_for_backdrop_radius() {
        let material = GlassMaterial::new().blur_intensity(0.75);

        assert_eq!(material.blur_intensity_value(), 0.75);
        assert_eq!(
            material.blur_radius_value(),
            GlassMaterial::MAX_BLUR_RADIUS * 0.75
        );

        let invalid = GlassMaterial::new().blur_intensity(f32::NAN);
        assert_eq!(
            invalid.blur_intensity_value(),
            16.0 / GlassMaterial::MAX_BLUR_RADIUS
        );
    }

    #[test]
    fn radii_are_limited_to_half_the_surface_extent() {
        assert_eq!(
            resolved_radii(
                [100.0, 8.0, 100.0, 0.0],
                ResolvedSize {
                    width: 40.0,
                    height: 20.0
                }
            ),
            [10.0, 8.0, 10.0, 0.0]
        );
    }

    #[test]
    fn fallback_alpha_multiplies_material_opacity() {
        let material = GlassMaterial::new()
            .tint(Color::Rgba(10, 20, 30, 200))
            .opacity(0.5);
        assert_eq!(material.fallback_tint().to_rgba(), (10, 20, 30, 100));
    }

    #[test]
    fn reduced_motion_is_a_plain_portable_policy() {
        let material = GlassMaterial::new().motion_policy(MaterialMotionPolicy::Reduced);
        assert_eq!(
            material.motion_policy_value(),
            MaterialMotionPolicy::Reduced
        );
    }
}
