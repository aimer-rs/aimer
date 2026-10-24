use std::cell::Cell;

use aimer_decoration::{AnyDecor, BorderRadius, Decoration};
use aimer_widget::base::{BuildContext, Color, ResolvedSize, Size, Vec2d};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement, PortableWidget,
    Rebuildable, RequiredChild, VisitorElement, Widget,
};

use aimer_canvas::MaterialKind;

use super::glass::{
    GlassMaterial, MaterialMotionPolicy, build_material_request, decoration_needs_recording,
    logical_size, paint_decoration, resolved_radii, surface_outsets, v2_color,
};

/// Bounded values for a [`Liquid`] water-droplet surface.
///
/// The static glass treatment (tint, blur, border, shadow) is kept in
/// [`GlassMaterial`]. Liquid adds only deterministic, finite effect inputs on
/// top of it: a refraction/magnification field that actually bends and zooms
/// whatever sits behind the surface, an organic droplet silhouette
/// (`blob_amount`/`blob_seed`/`tip_pull`), a fresnel-style rim shadow
/// (`edge_lighting`), and a glossy highlight (`specular_highlight`). A
/// renderer turns these into GPU fields without making renderer details part
/// of the container API; the container itself only paints a deterministic
/// rounded-rect fallback so it never depends on a renderer being present.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiquidMaterial {
    base: GlassMaterial,
    distortion_strength: f32,
    edge_lighting: f32,
    specular_highlight: f32,
    animation_speed: f32,
    animation_time: f32,
    interaction: f32,
    blob_amount: f32,
    blob_seed: f32,
    magnification: f32,
    tip_pull: f32,
    chromatic_aberration: f32,
    bevel_radius: f32,
}

impl LiquidMaterial {
    /// The largest normalized refraction strength accepted by the contract.
    pub const MAX_DISTORTION_STRENGTH: f32 = 1.0;
    /// The largest normalized rim-shadow strength accepted by the contract.
    pub const MAX_EDGE_LIGHTING: f32 = 1.0;
    /// The largest normalized specular strength accepted by the contract.
    pub const MAX_SPECULAR_HIGHLIGHT: f32 = 1.0;
    /// The largest animation speed accepted by the contract.
    pub const MAX_ANIMATION_SPEED: f32 = 4.0;
    /// The largest normalized interaction amount accepted by the contract.
    pub const MAX_INTERACTION: f32 = 1.0;
    /// The largest absolute injected time accepted by the contract.
    pub const MAX_ANIMATION_TIME: f32 = 1_000_000.0;
    /// The largest normalized organic-silhouette wobble accepted by the contract.
    pub const MAX_BLOB_AMOUNT: f32 = 1.0;
    /// The largest normalized silhouette variation seed accepted by the contract.
    pub const MAX_BLOB_SEED: f32 = 1.0;
    /// The largest normalized backdrop magnification accepted by the contract.
    pub const MAX_MAGNIFICATION: f32 = 1.0;
    /// The largest normalized teardrop-tip taper accepted by the contract.
    pub const MAX_TIP_PULL: f32 = 1.0;
    /// The largest normalized chromatic-aberration strength accepted by the contract.
    pub const MAX_CHROMATIC_ABERRATION: f32 = 1.0;
    /// The largest fake 3D bevel radius, in logical pixels, accepted by the contract.
    pub const MAX_BEVEL_RADIUS: f32 = 512.0;
    /// The corner radius used when neither a radius nor a decoration declares a shape.
    pub const DEFAULT_CORNER_RADIUS: f32 = 28.0;

    /// Creates a clear water-droplet surface with a restrained, deterministic
    /// motion field.
    #[inline]
    pub fn new() -> Self {
        Self {
            base: GlassMaterial::new()
                .tint(Color::Rgba(214, 236, 250, 255))
                .opacity(0.5)
                .blur_radius(0.0)
                .border_width(1.0)
                .border_color(Color::Rgba(255, 255, 255, 110))
                .shadow_color(Color::Rgba(20, 40, 60, 90))
                .shadow_blur(18.0)
                .elevation(6.0)
                .quality(3),
            distortion_strength: 0.12,
            edge_lighting: 0.6,
            specular_highlight: 0.55,
            animation_speed: 0.35,
            animation_time: 0.0,
            interaction: 0.0,
            // Apple's Liquid Glass keeps a clean rounded-rect/pill outline —
            // no organic wobble. `blob_amount` is opt-in for a literal
            // water-droplet look, not part of the default UI-glass identity.
            blob_amount: 0.0,
            blob_seed: 0.5,
            magnification: 0.45,
            tip_pull: 0.0,
            // Apple's "Regular" Liquid Glass shows a faint color fringe at
            // its most-curved edges (light disperses by wavelength through
            // a real lens); a modest default keeps that identity visible
            // without turning into an obvious rainbow.
            chromatic_aberration: 0.3,
            // Matches the default corner radius: the fake 3D bevel and the
            // 2D silhouette round over at roughly the same scale by default.
            bevel_radius: 28.0,
        }
    }

    /// Replaces the static glass portion of the liquid material.
    #[inline]
    pub fn glass(mut self, glass: GlassMaterial) -> Self {
        self.base = glass.normalized();
        self
    }

    /// Sets the tint used by the static and dynamic surface.
    #[inline]
    pub fn tint(mut self, tint: impl Into<Color>) -> Self {
        self.base = self.base.tint(tint);
        self
    }

    /// Sets the surface opacity in `0.0..=1.0`.
    #[inline]
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.base = self.base.opacity(opacity);
        self
    }

    /// Sets the backdrop blur radius. A droplet is usually crisp, so this
    /// defaults to `0.0`; raise it for a frosted droplet look.
    #[inline]
    pub fn blur_radius(mut self, blur_radius: f32) -> Self {
        self.base = self.base.blur_radius(blur_radius);
        self
    }

    /// Sets backdrop saturation.
    #[inline]
    pub fn saturation(mut self, saturation: f32) -> Self {
        self.base = self.base.saturation(saturation);
        self
    }

    /// Sets backdrop brightness.
    #[inline]
    pub fn brightness(mut self, brightness: f32) -> Self {
        self.base = self.base.brightness(brightness);
        self
    }

    /// Sets backdrop contrast.
    #[inline]
    pub fn contrast(mut self, contrast: f32) -> Self {
        self.base = self.base.contrast(contrast);
        self
    }

    /// Sets the border width.
    #[inline]
    pub fn border_width(mut self, border_width: f32) -> Self {
        self.base = self.base.border_width(border_width);
        self
    }

    /// Sets the border highlight color.
    #[inline]
    pub fn border_color(mut self, border_color: impl Into<Color>) -> Self {
        self.base = self.base.border_color(border_color);
        self
    }

    /// Sets the corner radii of the surface; see [`GlassMaterial::border_radius`].
    ///
    /// Without a radius or a shaping decoration the surface uses
    /// [`Self::DEFAULT_CORNER_RADIUS`].
    #[inline]
    pub fn border_radius(mut self, border_radius: impl Into<BorderRadius>) -> Self {
        self.base = self.base.border_radius(border_radius);
        self
    }

    /// Sets shadow color.
    #[inline]
    pub fn shadow_color(mut self, shadow_color: impl Into<Color>) -> Self {
        self.base = self.base.shadow_color(shadow_color);
        self
    }

    /// Sets shadow blur.
    #[inline]
    pub fn shadow_blur(mut self, shadow_blur: f32) -> Self {
        self.base = self.base.shadow_blur(shadow_blur);
        self
    }

    /// Sets shadow elevation.
    #[inline]
    pub fn elevation(mut self, elevation: f32) -> Self {
        self.base = self.base.elevation(elevation);
        self
    }

    /// Sets the bounded sampling quality.
    #[inline]
    pub fn quality(mut self, quality: u8) -> Self {
        self.base = self.base.quality(quality);
        self
    }

    /// Sets the reduced-motion policy. Reduced motion zeroes refraction
    /// ripple and animation speed; it never changes the droplet's static
    /// silhouette (`blob_amount`, `blob_seed`, `magnification`, `tip_pull`).
    #[inline]
    pub fn motion_policy(mut self, policy: MaterialMotionPolicy) -> Self {
        self.base = self.base.motion_policy(policy);
        self
    }

    /// Sets normalized refraction strength: how strongly the backdrop bends
    /// and ripples behind the droplet.
    #[inline]
    pub fn distortion_strength(mut self, strength: f32) -> Self {
        self.distortion_strength = normalize(strength, 0.12, 0.0, Self::MAX_DISTORTION_STRENGTH);
        self
    }

    /// Sets the normalized rim-shadow strength: the dark fresnel ring at the
    /// droplet's outline.
    #[inline]
    pub fn edge_lighting(mut self, strength: f32) -> Self {
        self.edge_lighting = normalize(strength, 0.6, 0.0, Self::MAX_EDGE_LIGHTING);
        self
    }

    /// Sets normalized specular-highlight strength: the bright glossy glint.
    #[inline]
    pub fn specular_highlight(mut self, strength: f32) -> Self {
        self.specular_highlight = normalize(strength, 0.55, 0.0, Self::MAX_SPECULAR_HIGHLIGHT);
        self
    }

    /// Sets the animation speed. Reduced motion makes its effective value zero.
    #[inline]
    pub fn animation_speed(mut self, speed: f32) -> Self {
        self.animation_speed = normalize(speed, 0.35, 0.0, Self::MAX_ANIMATION_SPEED);
        self
    }

    /// Injects deterministic animation time for a frame or a test.
    #[inline]
    pub fn animation_time(mut self, time: f32) -> Self {
        self.animation_time = normalize(
            time,
            0.0,
            -Self::MAX_ANIMATION_TIME,
            Self::MAX_ANIMATION_TIME,
        );
        self
    }

    /// Sets normalized pointer/interaction influence.
    #[inline]
    pub fn interaction(mut self, interaction: f32) -> Self {
        self.interaction = normalize(interaction, 0.0, 0.0, Self::MAX_INTERACTION);
        self
    }

    /// Sets how strongly the droplet's outline departs from a plain rounded
    /// rect into an organic, wobbled silhouette. `0.0` keeps a plain rounded
    /// rect; values near `1.0` read as an irregular water-droplet blob.
    #[inline]
    pub fn blob_amount(mut self, amount: f32) -> Self {
        self.blob_amount = normalize(amount, 0.0, 0.0, Self::MAX_BLOB_AMOUNT);
        self
    }

    /// Sets a deterministic seed in `0.0..=1.0` that varies which lobes the
    /// organic silhouette takes, without introducing per-frame randomness.
    #[inline]
    pub fn blob_seed(mut self, seed: f32) -> Self {
        self.blob_seed = normalize(seed, 0.5, 0.0, Self::MAX_BLOB_SEED);
        self
    }

    /// Sets the optical density of the fake 3D glass bevel described by
    /// [`Self::bevel_radius`], like a higher index of refraction: `0.0` is
    /// flat window glass with no bend, and higher values bend the backdrop
    /// more strongly. The bend is computed from an actual surface normal, so
    /// a flat run only bends one direction (like a fluted glass rod) while a
    /// doubly-curved corner or pill end bends radially — a full little lens.
    #[inline]
    pub fn magnification(mut self, magnification: f32) -> Self {
        self.magnification = normalize(magnification, 0.45, 0.0, Self::MAX_MAGNIFICATION);
        self
    }

    /// Sets how strongly the droplet tapers into a pointed teardrop tip at
    /// its bottom edge. `0.0` keeps a symmetric droplet.
    #[inline]
    pub fn tip_pull(mut self, tip_pull: f32) -> Self {
        self.tip_pull = normalize(tip_pull, 0.0, 0.0, Self::MAX_TIP_PULL);
        self
    }

    /// Sets how strongly red/blue split apart from green at the droplet's
    /// most-curved, most-refractive edge — real dispersion through a lens,
    /// concentrated exactly where [`Self::magnification`] bends the
    /// backdrop the most. `0.0` disables the color fringe entirely.
    #[inline]
    pub fn chromatic_aberration(mut self, strength: f32) -> Self {
        self.chromatic_aberration = normalize(strength, 0.3, 0.0, Self::MAX_CHROMATIC_ABERRATION);
        self
    }

    /// Sets the depth, in logical pixels, of the fake 3D rounded bevel the
    /// refraction is computed from — a "Z-Radius" independent of the 2D
    /// [`Self::border_radius`]. A small bevel makes the surface normal tilt
    /// sharply right at the boundary (a tight, dramatic lens); a large one
    /// spreads that tilt further inward (a gentler, wider bend).
    #[inline]
    pub fn bevel_radius(mut self, bevel_radius: f32) -> Self {
        self.bevel_radius = normalize(bevel_radius, 28.0, 0.0, Self::MAX_BEVEL_RADIUS);
        self
    }

    /// Returns the static glass material.
    #[inline]
    pub fn glass_material(self) -> GlassMaterial {
        self.base
    }

    /// Returns the configured refraction strength.
    #[inline]
    pub fn distortion_strength_value(self) -> f32 {
        self.distortion_strength
    }

    /// Returns the configured rim-shadow strength.
    #[inline]
    pub fn edge_lighting_value(self) -> f32 {
        self.edge_lighting
    }

    /// Returns the configured specular-highlight strength.
    #[inline]
    pub fn specular_highlight_value(self) -> f32 {
        self.specular_highlight
    }

    /// Returns the configured animation speed.
    #[inline]
    pub fn animation_speed_value(self) -> f32 {
        self.animation_speed
    }

    /// Returns the injected animation time.
    #[inline]
    pub fn animation_time_value(self) -> f32 {
        self.animation_time
    }

    /// Returns the normalized interaction amount.
    #[inline]
    pub fn interaction_value(self) -> f32 {
        self.interaction
    }

    /// Returns the configured organic-silhouette wobble strength.
    #[inline]
    pub fn blob_amount_value(self) -> f32 {
        self.blob_amount
    }

    /// Returns the configured silhouette variation seed.
    #[inline]
    pub fn blob_seed_value(self) -> f32 {
        self.blob_seed
    }

    /// Returns the configured backdrop magnification.
    #[inline]
    pub fn magnification_value(self) -> f32 {
        self.magnification
    }

    /// Returns the configured teardrop-tip taper.
    #[inline]
    pub fn tip_pull_value(self) -> f32 {
        self.tip_pull
    }

    /// Returns the configured chromatic-aberration strength.
    #[inline]
    pub fn chromatic_aberration_value(self) -> f32 {
        self.chromatic_aberration
    }

    /// Returns the configured fake 3D bevel radius.
    #[inline]
    pub fn bevel_radius_value(self) -> f32 {
        self.bevel_radius
    }

    /// Returns zero for dynamic fields when reduced motion is requested.
    #[inline]
    pub fn effective_distortion(self) -> f32 {
        match self.base.motion_policy_value() {
            MaterialMotionPolicy::Full => self.distortion_strength,
            MaterialMotionPolicy::Reduced => 0.0,
        }
    }

    /// Returns zero for dynamic fields when reduced motion is requested.
    #[inline]
    pub fn effective_animation_speed(self) -> f32 {
        match self.base.motion_policy_value() {
            MaterialMotionPolicy::Full => self.animation_speed,
            MaterialMotionPolicy::Reduced => 0.0,
        }
    }

    /// Returns the deterministic phase used by a renderer's highlight field.
    #[inline]
    pub fn effective_phase(self) -> f32 {
        let speed = self.effective_animation_speed();
        let phase = self.animation_time * speed + self.interaction;
        if phase.is_finite() { phase } else { 0.0 }
    }

    pub(crate) fn normalized(mut self) -> Self {
        self.base = self.base.normalized();
        self.distortion_strength = normalize(
            self.distortion_strength,
            0.12,
            0.0,
            Self::MAX_DISTORTION_STRENGTH,
        );
        self.edge_lighting = normalize(self.edge_lighting, 0.6, 0.0, Self::MAX_EDGE_LIGHTING);
        self.specular_highlight = normalize(
            self.specular_highlight,
            0.55,
            0.0,
            Self::MAX_SPECULAR_HIGHLIGHT,
        );
        self.animation_speed =
            normalize(self.animation_speed, 0.35, 0.0, Self::MAX_ANIMATION_SPEED);
        self.animation_time = normalize(
            self.animation_time,
            0.0,
            -Self::MAX_ANIMATION_TIME,
            Self::MAX_ANIMATION_TIME,
        );
        self.interaction = normalize(self.interaction, 0.0, 0.0, Self::MAX_INTERACTION);
        self.blob_amount = normalize(self.blob_amount, 0.0, 0.0, Self::MAX_BLOB_AMOUNT);
        self.blob_seed = normalize(self.blob_seed, 0.5, 0.0, Self::MAX_BLOB_SEED);
        self.magnification = normalize(self.magnification, 0.45, 0.0, Self::MAX_MAGNIFICATION);
        self.tip_pull = normalize(self.tip_pull, 0.0, 0.0, Self::MAX_TIP_PULL);
        self.chromatic_aberration = normalize(
            self.chromatic_aberration,
            0.3,
            0.0,
            Self::MAX_CHROMATIC_ABERRATION,
        );
        self.bevel_radius = normalize(self.bevel_radius, 28.0, 0.0, Self::MAX_BEVEL_RADIUS);
        self
    }
}

impl Default for LiquidMaterial {
    fn default() -> Self {
        Self::new()
    }
}

/// A dynamic, single-child water-droplet surface.
///
/// Liquid retains exactly the same child and layout/event semantics as Glass.
/// Its bounded shape and motion values are data for the canvas/Cupid seam; the
/// container itself only paints the deterministic static fallback so it never
/// depends on a renderer or native visual-effect API.
pub struct Liquid<W = RequiredChild> {
    child: W,
    material: LiquidMaterial,
    decoration: Option<AnyDecor>,
}

impl Liquid {
    /// Creates a default liquid builder without a child.
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            material: LiquidMaterial::new(),
            decoration: None,
        }
    }

    /// Replaces the material configuration.
    #[inline]
    pub fn material(mut self, material: LiquidMaterial) -> Self {
        self.material = material.normalized();
        self
    }

    /// Sets the static glass material portion.
    #[inline]
    pub fn glass(mut self, glass: GlassMaterial) -> Self {
        self.material = self.material.glass(glass);
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

    /// Sets backdrop saturation.
    #[inline]
    pub fn saturation(mut self, saturation: f32) -> Self {
        self.material = self.material.saturation(saturation);
        self
    }

    /// Sets backdrop brightness.
    #[inline]
    pub fn brightness(mut self, brightness: f32) -> Self {
        self.material = self.material.brightness(brightness);
        self
    }

    /// Sets backdrop contrast.
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

    /// Sets the corner radii of the surface; see [`LiquidMaterial::border_radius`].
    #[inline]
    pub fn border_radius(mut self, border_radius: impl Into<BorderRadius>) -> Self {
        self.material = self.material.border_radius(border_radius);
        self
    }

    /// Paints `decoration` behind the liquid surface and adopts its shape.
    ///
    /// Behaves like [`Glass::decoration`](super::Glass::decoration): unless
    /// [`Self::border_radius`] is set, the surface takes the corner radii that
    /// `BoxDecoration` or `BorderRadius` layers declared. It never affects
    /// layout, clipping or events.
    #[inline]
    pub fn decoration(mut self, decoration: impl Decoration + 'static) -> Self {
        self.decoration = Some(decoration.notype());
        self
    }

    /// Sets shadow color.
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

    /// Sets bounded sampling quality.
    #[inline]
    pub fn quality(mut self, quality: u8) -> Self {
        self.material = self.material.quality(quality);
        self
    }

    /// Sets the reduced-motion policy.
    #[inline]
    pub fn motion_policy(mut self, policy: MaterialMotionPolicy) -> Self {
        self.material = self.material.motion_policy(policy);
        self
    }

    /// Sets normalized refraction strength.
    #[inline]
    pub fn distortion_strength(mut self, strength: f32) -> Self {
        self.material = self.material.distortion_strength(strength);
        self
    }

    /// Sets normalized rim-shadow strength.
    #[inline]
    pub fn edge_lighting(mut self, strength: f32) -> Self {
        self.material = self.material.edge_lighting(strength);
        self
    }

    /// Sets normalized specular highlights.
    #[inline]
    pub fn specular_highlight(mut self, strength: f32) -> Self {
        self.material = self.material.specular_highlight(strength);
        self
    }

    /// Sets animation speed.
    #[inline]
    pub fn animation_speed(mut self, speed: f32) -> Self {
        self.material = self.material.animation_speed(speed);
        self
    }

    /// Injects deterministic animation time.
    #[inline]
    pub fn animation_time(mut self, time: f32) -> Self {
        self.material = self.material.animation_time(time);
        self
    }

    /// Sets normalized pointer/interaction influence.
    #[inline]
    pub fn interaction(mut self, interaction: f32) -> Self {
        self.material = self.material.interaction(interaction);
        self
    }

    /// Sets the organic-silhouette wobble strength.
    #[inline]
    pub fn blob_amount(mut self, amount: f32) -> Self {
        self.material = self.material.blob_amount(amount);
        self
    }

    /// Sets the deterministic silhouette variation seed.
    #[inline]
    pub fn blob_seed(mut self, seed: f32) -> Self {
        self.material = self.material.blob_seed(seed);
        self
    }

    /// Sets the edge-refraction strength.
    #[inline]
    pub fn magnification(mut self, magnification: f32) -> Self {
        self.material = self.material.magnification(magnification);
        self
    }

    /// Sets the teardrop-tip taper strength.
    #[inline]
    pub fn tip_pull(mut self, tip_pull: f32) -> Self {
        self.material = self.material.tip_pull(tip_pull);
        self
    }

    /// Sets the chromatic-aberration strength at the refracting edge.
    #[inline]
    pub fn chromatic_aberration(mut self, strength: f32) -> Self {
        self.material = self.material.chromatic_aberration(strength);
        self
    }

    /// Sets the fake 3D bevel radius the refraction is computed from.
    #[inline]
    pub fn bevel_radius(mut self, bevel_radius: f32) -> Self {
        self.material = self.material.bevel_radius(bevel_radius);
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Liquid<W> {
        Liquid {
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

impl Default for Liquid {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Widget + 'static> PortableWidget for Liquid<W> {}

impl<W: Widget + 'static> Widget for Liquid<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawLiquid {
            child: self.child.to_element(ctx),
            material: self.material.normalized(),
            decoration: self.decoration,
            recorded: Cell::new(None),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Liquid"
    }
}

struct RawLiquid {
    child: AnyElement,
    material: LiquidMaterial,
    decoration: Option<AnyDecor>,
    recorded: Cell<Option<u64>>,
}

impl Rebuildable for RawLiquid {}

impl Drawable for RawLiquid {
    fn update(&self, ctx: &BuildContext) {
        // The liquid surface is painted by `paint_local_v2`; the child keeps its
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
        let base = material.glass_material();
        if let Some(size) = logical_size(ctx, self.child.computed_size(ctx))
            && size.width > 0.0
            && size.height > 0.0
        {
            paint_decoration(self.decoration.as_ref(), &canvas, size, ctx.scale);
            let shape = base.shape_radii(&canvas, size, LiquidMaterial::DEFAULT_CORNER_RADIUS);
            let radii = resolved_radii(shape, size);
            let shadow = with_opacity(base.shadow_color_value(), base.opacity_value());
            if shadow.alpha() != 0 && base.shadow_blur_value() > 0.0 {
                canvas.draw_shadow_rect(
                    aimer_cupid::utilities::Rect::new(0.0, 0.0, size.width, size.height),
                    v2_color(shadow),
                    [0.0, base.elevation_value(), base.shadow_blur_value(), 0.0],
                    radii,
                    false,
                    [0.0; 3],
                );
            }
            let border = with_opacity(base.border_color_value(), base.opacity_value());
            let border_width = if border.alpha() == 0 {
                [0.0; 4]
            } else {
                [base.border_width_value(); 4]
            };
            canvas.fill_rect_styled(
                aimer_cupid::utilities::Rect::new(0.0, 0.0, size.width, size.height),
                v2_color(with_opacity(base.tint_color(), base.opacity_value())),
                radii,
                border_width,
                v2_color(border),
                [0.0; 4],
                aimer_cupid::utilities::Color::transparent(),
            );
            canvas.draw_material(build_material_request(
                MaterialKind::Liquid,
                base,
                size,
                shape,
                material.effective_distortion(),
                material.edge_lighting_value(),
                material.specular_highlight_value(),
                material.effective_animation_speed(),
                material.animation_time_value(),
                material.interaction_value(),
                [
                    material.blob_amount_value(),
                    material.blob_seed_value(),
                    material.magnification_value(),
                    material.tip_pull_value(),
                    material.chromatic_aberration_value(),
                    material.bevel_radius_value(),
                ],
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
            self.material.normalized().glass_material(),
            self.decoration.as_ref(),
            ctx,
            self.child.computed_size(ctx),
        )
    }
}

impl EventElement for RawLiquid {
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    #[inline]
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl LayoutElement for RawLiquid {
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

impl VisitorElement for RawLiquid {
    #[inline]
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Liquid"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ZeroSizedBox;

    #[test]
    fn liquid_values_are_finite_and_clamped() {
        let material = LiquidMaterial::new()
            .distortion_strength(f32::INFINITY)
            .edge_lighting(-1.0)
            .specular_highlight(2.0)
            .animation_speed(f32::NAN)
            .animation_time(f32::INFINITY)
            .interaction(-2.0)
            .blob_amount(f32::NAN)
            .blob_seed(-1.0)
            .magnification(5.0)
            .tip_pull(-1.0)
            .chromatic_aberration(f32::NAN)
            .bevel_radius(-10.0)
            .quality(255);

        assert_eq!(material.distortion_strength_value(), 0.12);
        assert_eq!(material.edge_lighting_value(), 0.0);
        assert_eq!(
            material.specular_highlight_value(),
            LiquidMaterial::MAX_SPECULAR_HIGHLIGHT
        );
        assert_eq!(material.animation_speed_value(), 0.35);
        assert_eq!(material.animation_time_value(), 0.0);
        assert_eq!(material.interaction_value(), 0.0);
        assert_eq!(material.blob_amount_value(), 0.0);
        assert_eq!(material.blob_seed_value(), 0.0);
        assert_eq!(
            material.magnification_value(),
            LiquidMaterial::MAX_MAGNIFICATION
        );
        assert_eq!(material.tip_pull_value(), 0.0);
        assert_eq!(material.chromatic_aberration_value(), 0.3);
        assert_eq!(material.bevel_radius_value(), 0.0);
        assert_eq!(
            material.glass_material().quality_value(),
            GlassMaterial::MAX_QUALITY
        );
    }

    #[test]
    fn reduced_motion_disables_dynamic_fields_without_changing_static_values() {
        let material = LiquidMaterial::new()
            .distortion_strength(0.8)
            .animation_speed(2.0)
            .animation_time(10.0)
            .interaction(0.25)
            .motion_policy(MaterialMotionPolicy::Reduced);

        assert_eq!(material.effective_distortion(), 0.0);
        assert_eq!(material.effective_animation_speed(), 0.0);
        assert_eq!(material.effective_phase(), 0.25);
        assert_eq!(material.glass_material().opacity_value(), 0.5);
    }

    #[test]
    fn reduced_motion_never_changes_the_static_droplet_silhouette() {
        let material = LiquidMaterial::new()
            .blob_amount(0.9)
            .blob_seed(0.7)
            .magnification(0.8)
            .tip_pull(0.6)
            .chromatic_aberration(0.7)
            .bevel_radius(64.0)
            .motion_policy(MaterialMotionPolicy::Reduced);

        assert_eq!(material.blob_amount_value(), 0.9);
        assert_eq!(material.blob_seed_value(), 0.7);
        assert_eq!(material.magnification_value(), 0.8);
        assert_eq!(material.tip_pull_value(), 0.6);
        assert_eq!(material.chromatic_aberration_value(), 0.7);
        assert_eq!(material.bevel_radius_value(), 64.0);
    }

    #[test]
    fn injected_time_makes_the_phase_deterministic() {
        let material = LiquidMaterial::new()
            .animation_speed(0.5)
            .animation_time(8.0)
            .interaction(0.25);
        assert_eq!(material.effective_phase(), 4.25);
        assert_eq!(material.effective_phase(), material.effective_phase());
    }
}
