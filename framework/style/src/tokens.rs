use aimer_animation::Animatable;
use aimer_color::prelude::Color;

/// The appearance variant a token set is intended to render.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ThemeVariant {
    /// Standard light appearance.
    Light,
    /// Standard dark appearance.
    Dark,
    /// High-contrast appearance, independent of light/dark palette details.
    HighContrast,
}

impl Default for ThemeVariant {
    fn default() -> Self {
        Self::Light
    }
}

/// Semantic color roles used by components instead of raw palette literals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorTokens {
    /// Accent or action color.
    pub primary: Color,
    /// Content placed on [`Self::primary`].
    pub on_primary: Color,
    /// Application background.
    pub background: Color,
    /// Content placed on [`Self::background`].
    pub on_background: Color,
    /// Grouped or elevated surface.
    pub surface: Color,
    /// Content placed on [`Self::surface`].
    pub on_surface: Color,
    /// Non-emphasized border and divider color.
    pub outline: Color,
    /// Error indication color.
    pub error: Color,
    /// Content placed on [`Self::error`].
    pub on_error: Color,
    /// Success indication color.
    pub success: Color,
    /// Content placed on [`Self::success`].
    pub on_success: Color,
}

impl ColorTokens {
    /// Returns the default light semantic color roles.
    pub const fn light() -> Self {
        Self {
            primary: Color::RED,
            on_primary: Color::WHITE,
            background: Color::WHITE,
            on_background: Color::BLACK,
            surface: Color::WHITE,
            on_surface: Color::BLACK,
            outline: Color::Grayscale(100, 255),
            error: Color::Rgba(186, 26, 26, 255),
            on_error: Color::WHITE,
            success: Color::Rgba(25, 120, 55, 255),
            on_success: Color::WHITE,
        }
    }

    /// Returns the default dark semantic color roles.
    pub const fn dark() -> Self {
        Self {
            primary: Color::Rgba(144, 202, 249, 255),
            on_primary: Color::BLACK,
            background: Color::Rgba(18, 18, 18, 255),
            on_background: Color::WHITE,
            surface: Color::Rgba(30, 30, 30, 255),
            on_surface: Color::WHITE,
            outline: Color::Grayscale(160, 255),
            error: Color::Rgba(255, 180, 171, 255),
            on_error: Color::Rgba(105, 0, 5, 255),
            success: Color::Rgba(125, 220, 145, 255),
            on_success: Color::Rgba(0, 55, 20, 255),
        }
    }

    /// Returns the default high-contrast semantic color roles.
    pub const fn high_contrast() -> Self {
        Self {
            primary: Color::BLUE,
            on_primary: Color::WHITE,
            background: Color::WHITE,
            on_background: Color::BLACK,
            surface: Color::WHITE,
            on_surface: Color::BLACK,
            outline: Color::BLACK,
            error: Color::Rgba(160, 0, 0, 255),
            on_error: Color::WHITE,
            success: Color::Rgba(0, 100, 0, 255),
            on_success: Color::WHITE,
        }
    }

    /// Creates semantic roles from the six core theme colors.
    pub fn from_core_colors(
        primary: Color,
        on_primary: Color,
        background: Color,
        on_background: Color,
        surface: Color,
        on_surface: Color,
    ) -> Self {
        let defaults = Self::light();
        Self {
            primary,
            on_primary,
            background,
            on_background,
            surface,
            on_surface,
            outline: on_surface.with_alpha(0.48),
            error: defaults.error,
            on_error: defaults.on_error,
            success: defaults.success,
            on_success: defaults.on_success,
        }
    }

    fn is_finite(&self) -> bool {
        true
    }
}

impl Default for ColorTokens {
    fn default() -> Self {
        Self::light()
    }
}

/// The semantic design-token set consumed by component styling.
///
/// `ThemeTokens` is deliberately independent from a renderer. Applications can
/// provide it through the existing [`crate::AnimatedTheme`] provider, animate
/// it with the existing theme transition machinery, or derive it from a
/// [`crate::ThemeData`] palette with [`crate::ThemeData::tokens`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeTokens {
    /// Semantic color roles, including error and success colors.
    pub colors: ColorTokens,
    /// Typography roles such as body, title, and label.
    pub typography: TypographyTokens,
    /// The standard spacing scale in logical pixels.
    pub spacing: SpacingTokens,
    /// Corner-radius roles in logical pixels.
    pub shape: ShapeTokens,
    /// Minimum-size and scale policy for compact or comfortable controls.
    pub density: DensityTokens,
    /// Global keyboard-focus indicator policy.
    pub focus: FocusTokens,
    /// State-layer and semantic feedback roles.
    pub state: StateTokens,
    /// Component-specific aliases and geometry defaults.
    pub control: ControlTokens,
    /// Elevation levels for surfaces and overlays.
    pub elevation: ElevationTokens,
    /// Motion durations and reduced-motion policy.
    pub motion: MotionTokens,
}

impl ThemeTokens {
    /// Returns Aimer's default light token set.
    pub const fn light() -> Self {
        Self {
            colors: ColorTokens::light(),
            typography: TypographyTokens::standard(),
            spacing: SpacingTokens::standard(),
            shape: ShapeTokens::standard(),
            density: DensityTokens::standard(),
            focus: FocusTokens::light(),
            state: StateTokens::light(),
            control: ControlTokens::light(),
            elevation: ElevationTokens::standard(),
            motion: MotionTokens::standard(),
        }
    }

    /// Returns Aimer's default dark token set.
    pub const fn dark() -> Self {
        Self {
            colors: ColorTokens::dark(),
            typography: TypographyTokens::dark(),
            spacing: SpacingTokens::standard(),
            shape: ShapeTokens::standard(),
            density: DensityTokens::standard(),
            focus: FocusTokens::dark(),
            state: StateTokens::dark(),
            control: ControlTokens::dark(),
            elevation: ElevationTokens::dark(),
            motion: MotionTokens::standard(),
        }
    }

    /// Returns a high-contrast token set with strong foreground/background
    /// separation and a prominent focus ring.
    pub const fn high_contrast() -> Self {
        Self {
            colors: ColorTokens::high_contrast(),
            typography: TypographyTokens::standard(),
            spacing: SpacingTokens::standard(),
            shape: ShapeTokens::standard(),
            density: DensityTokens::standard(),
            focus: FocusTokens::high_contrast(),
            state: StateTokens::high_contrast(),
            control: ControlTokens::high_contrast(),
            elevation: ElevationTokens::standard(),
            motion: MotionTokens::standard(),
        }
    }

    /// Returns the built-in token set for `variant`.
    pub const fn for_variant(variant: ThemeVariant) -> Self {
        match variant {
            ThemeVariant::Light => Self::light(),
            ThemeVariant::Dark => Self::dark(),
            ThemeVariant::HighContrast => Self::high_contrast(),
        }
    }

    /// Builds tokens from the six core roles in [`crate::ThemeData`].
    ///
    /// The remaining semantic roles use appearance-appropriate defaults and
    /// are then derived from the supplied palette. This is an additive bridge
    /// for existing themes; it does not alter `ThemeData`'s portable six-color
    /// wire format.
    pub fn from_core_colors(
        primary: Color,
        on_primary: Color,
        background: Color,
        on_background: Color,
        surface: Color,
        on_surface: Color,
    ) -> Self {
        let mut tokens = if relative_luminance(background) < 0.5 {
            Self::dark()
        } else {
            Self::light()
        };
        tokens.colors = ColorTokens::from_core_colors(
            primary,
            on_primary,
            background,
            on_background,
            surface,
            on_surface,
        );
        tokens.focus.ring_color = primary;
        tokens.control.focus_ring.ring_color = primary;
        tokens.state.hover.color = primary;
        tokens.state.pressed.color = primary;
        tokens.state.selected.color = primary;
        tokens.state.error.color = tokens.colors.error;
        tokens.state.success.color = tokens.colors.success;
        tokens
    }

    /// Returns a deterministic high-contrast fallback for a partially
    /// specified variant. Structural tokens are retained while visual roles
    /// are replaced with the built-in high-contrast policy.
    pub fn high_contrast_fallback(&self) -> Self {
        let mut fallback = *self;
        let high_contrast = Self::high_contrast();
        fallback.colors = high_contrast.colors;
        fallback.focus = high_contrast.focus;
        fallback.state = high_contrast.state;
        fallback.control = high_contrast.control;
        fallback.normalized()
    }

    /// Returns a safe token set with finite, bounded values.
    ///
    /// Public fields are intentionally transparent for ergonomic theme
    /// authoring, so callers may also construct a token set with an invalid
    /// value. Interpolation and renderer adapters use this normalization seam
    /// before consuming those values.
    pub fn normalized(&self) -> Self {
        Self {
            colors: self.colors,
            typography: self.typography.normalized(),
            spacing: self.spacing.normalized(),
            shape: self.shape.normalized(),
            density: self.density.normalized(),
            focus: self.focus.normalized(),
            state: self.state.normalized(),
            control: self.control.normalized(),
            elevation: self.elevation.normalized(),
            motion: self.motion.normalized(),
        }
    }

    /// Returns whether every numeric token is finite and within its safe range.
    pub fn is_finite(&self) -> bool {
        self.colors.is_finite()
            && self.typography.is_finite()
            && self.spacing.is_finite()
            && self.shape.is_finite()
            && self.density.is_finite()
            && self.focus.is_finite()
            && self.state.is_finite()
            && self.control.is_finite()
            && self.elevation.is_finite()
            && self.motion.is_finite()
    }
}

impl Default for ThemeTokens {
    fn default() -> Self {
        Self::light()
    }
}

impl Animatable for ThemeTokens {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = normalized_progress(t);
        let begin = self.normalized();
        let end = other.normalized();

        Self {
            colors: begin.colors.lerp(&end.colors, t),
            typography: begin.typography.lerp(&end.typography, t),
            spacing: begin.spacing.lerp(&end.spacing, t),
            shape: begin.shape.lerp(&end.shape, t),
            density: begin.density.lerp(&end.density, t),
            focus: begin.focus.lerp(&end.focus, t),
            state: begin.state.lerp(&end.state, t),
            control: begin.control.lerp(&end.control, t),
            elevation: begin.elevation.lerp(&end.elevation, t),
            motion: begin.motion.lerp(&end.motion, t),
        }
        .normalized()
    }
}

impl crate::Theme for ThemeTokens {}

/// A light/dark/high-contrast token selection with explicit fallback rules.
///
/// Missing dark values fall back to the light value, matching
/// [`crate::ThemeSelection`]. A missing high-contrast value derives from the
/// dark value when present, otherwise from the light value, and applies the
/// built-in high-contrast visual policy while retaining structural tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeTokenVariants {
    /// The required light token set.
    pub light: ThemeTokens,
    /// Optional dark token set.
    pub dark: Option<ThemeTokens>,
    /// Optional high-contrast token set.
    pub high_contrast: Option<ThemeTokens>,
}

impl ThemeTokenVariants {
    /// Creates a selection with one required light value.
    pub const fn new(light: ThemeTokens) -> Self {
        Self {
            light,
            dark: None,
            high_contrast: None,
        }
    }

    /// Supplies the dark variant.
    pub const fn dark(mut self, dark: ThemeTokens) -> Self {
        self.dark = Some(dark);
        self
    }

    /// Supplies the explicit high-contrast variant.
    pub const fn high_contrast(mut self, high_contrast: ThemeTokens) -> Self {
        self.high_contrast = Some(high_contrast);
        self
    }

    /// Resolves a variant using the documented deterministic fallbacks.
    pub fn resolve(&self, variant: ThemeVariant) -> ThemeTokens {
        match variant {
            ThemeVariant::Light => self.light,
            ThemeVariant::Dark => self.dark.unwrap_or(self.light),
            ThemeVariant::HighContrast => self
                .high_contrast
                .unwrap_or_else(|| self.dark.unwrap_or(self.light).high_contrast_fallback()),
        }
    }
}

impl Default for ThemeTokenVariants {
    fn default() -> Self {
        Self::new(ThemeTokens::light())
            .dark(ThemeTokens::dark())
            .high_contrast(ThemeTokens::high_contrast())
    }
}

/// Semantic typography roles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypographyTokens {
    /// Large display text.
    pub display: TypographyStyle,
    /// Section headings.
    pub headline: TypographyStyle,
    /// Component and page titles.
    pub title: TypographyStyle,
    /// Normal reading text.
    pub body: TypographyStyle,
    /// Compact labels and control text.
    pub label: TypographyStyle,
}

impl TypographyTokens {
    /// Returns the standard typography scale.
    pub const fn standard() -> Self {
        Self {
            display: TypographyStyle::new(40.0, 48.0, 400.0, 0.0),
            headline: TypographyStyle::new(30.0, 38.0, 600.0, 0.0),
            title: TypographyStyle::new(22.0, 28.0, 600.0, 0.0),
            body: TypographyStyle::new(16.0, 24.0, 400.0, 0.0),
            label: TypographyStyle::new(14.0, 20.0, 600.0, 0.1),
        }
    }

    const fn dark() -> Self {
        Self::standard()
    }

    fn is_finite(&self) -> bool {
        self.display.is_finite()
            && self.headline.is_finite()
            && self.title.is_finite()
            && self.body.is_finite()
            && self.label.is_finite()
    }

    fn normalized(&self) -> Self {
        Self {
            display: self.display.normalized(),
            headline: self.headline.normalized(),
            title: self.title.normalized(),
            body: self.body.normalized(),
            label: self.label.normalized(),
        }
    }
}

impl Default for TypographyTokens {
    fn default() -> Self {
        Self::standard()
    }
}

/// A typography role's renderer-neutral metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypographyStyle {
    /// Font size in logical pixels.
    pub font_size: f32,
    /// Line box height in logical pixels.
    pub line_height: f32,
    /// CSS-style numeric weight.
    pub font_weight: f32,
    /// Tracking in logical pixels.
    pub letter_spacing: f32,
}

impl TypographyStyle {
    /// Creates a renderer-neutral typography role.
    ///
    /// Values are normalized when the containing [`ThemeTokens`] is
    /// interpolated or explicitly normalized.
    pub const fn new(
        font_size: f32,
        line_height: f32,
        font_weight: f32,
        letter_spacing: f32,
    ) -> Self {
        Self {
            font_size,
            line_height,
            font_weight,
            letter_spacing,
        }
    }

    fn is_finite(&self) -> bool {
        self.font_size.is_finite()
            && self.line_height.is_finite()
            && self.font_weight.is_finite()
            && self.letter_spacing.is_finite()
            && self.font_size >= 0.0
            && self.line_height >= 0.0
            && (1.0..=1000.0).contains(&self.font_weight)
            && (-10.0..=10.0).contains(&self.letter_spacing)
    }

    fn normalized(&self) -> Self {
        Self {
            font_size: finite_non_negative(self.font_size, 16.0),
            line_height: finite_non_negative(self.line_height, 24.0),
            font_weight: finite_clamped(self.font_weight, 400.0, 1.0, 1000.0),
            letter_spacing: finite_clamped(self.letter_spacing, 0.0, -10.0, 10.0),
        }
    }
}

impl Default for TypographyStyle {
    fn default() -> Self {
        Self::new(16.0, 24.0, 400.0, 0.0)
    }
}

/// The logical-pixel spacing roles used by components.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacingTokens {
    /// Extra-small spacing.
    pub x_small: f32,
    /// Small spacing.
    pub small: f32,
    /// The default component gap.
    pub medium: f32,
    /// Large section spacing.
    pub large: f32,
    /// Extra-large section spacing.
    pub x_large: f32,
}

impl SpacingTokens {
    /// Returns the standard spacing scale.
    pub const fn standard() -> Self {
        Self {
            x_small: 4.0,
            small: 8.0,
            medium: 16.0,
            large: 24.0,
            x_large: 32.0,
        }
    }

    fn is_finite(&self) -> bool {
        self.x_small.is_finite()
            && self.small.is_finite()
            && self.medium.is_finite()
            && self.large.is_finite()
            && self.x_large.is_finite()
            && self.x_small >= 0.0
            && self.small >= 0.0
            && self.medium >= 0.0
            && self.large >= 0.0
            && self.x_large >= 0.0
    }

    fn normalized(&self) -> Self {
        let defaults = Self::standard();
        Self {
            x_small: finite_non_negative(self.x_small, defaults.x_small),
            small: finite_non_negative(self.small, defaults.small),
            medium: finite_non_negative(self.medium, defaults.medium),
            large: finite_non_negative(self.large, defaults.large),
            x_large: finite_non_negative(self.x_large, defaults.x_large),
        }
    }

    /// Returns one spacing role by semantic scale.
    #[inline]
    pub const fn value(self, scale: SpacingScale) -> f32 {
        match scale {
            SpacingScale::XSmall => self.x_small,
            SpacingScale::Small => self.small,
            SpacingScale::Medium => self.medium,
            SpacingScale::Large => self.large,
            SpacingScale::XLarge => self.x_large,
        }
    }
}

impl Default for SpacingTokens {
    fn default() -> Self {
        Self::standard()
    }
}

/// The named spacing roles available from [`SpacingTokens`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SpacingScale {
    /// Four logical pixels by default.
    XSmall,
    /// Eight logical pixels by default.
    Small,
    /// Sixteen logical pixels by default.
    Medium,
    /// Twenty-four logical pixels by default.
    Large,
    /// Thirty-two logical pixels by default.
    XLarge,
}

impl Default for SpacingScale {
    fn default() -> Self {
        Self::Medium
    }
}

/// Corner-radius roles used by surfaces and controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeTokens {
    /// Radius for compact controls.
    pub small: f32,
    /// Default control radius.
    pub medium: f32,
    /// Radius for cards and larger surfaces.
    pub large: f32,
    /// A fully rounded/pill radius.
    pub pill: f32,
}

impl ShapeTokens {
    /// Returns the standard radius scale.
    pub const fn standard() -> Self {
        Self {
            small: 4.0,
            medium: 8.0,
            large: 16.0,
            pill: 999.0,
        }
    }

    fn is_finite(&self) -> bool {
        self.small.is_finite()
            && self.medium.is_finite()
            && self.large.is_finite()
            && self.pill.is_finite()
            && self.small >= 0.0
            && self.medium >= 0.0
            && self.large >= 0.0
            && self.pill >= 0.0
    }

    fn normalized(&self) -> Self {
        let defaults = Self::standard();
        Self {
            small: finite_non_negative(self.small, defaults.small),
            medium: finite_non_negative(self.medium, defaults.medium),
            large: finite_non_negative(self.large, defaults.large),
            pill: finite_non_negative(self.pill, defaults.pill),
        }
    }
}

impl Default for ShapeTokens {
    fn default() -> Self {
        Self::standard()
    }
}

/// Density values that determine scaling and minimum interactive targets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensityTokens {
    /// Multiplier applied to density-sized dimensions.
    pub scale: f32,
    /// Minimum logical size for an interactive target.
    pub minimum_target: f32,
}

impl DensityTokens {
    /// Returns the standard density policy.
    pub const fn standard() -> Self {
        Self {
            scale: 1.0,
            minimum_target: 44.0,
        }
    }

    fn is_finite(&self) -> bool {
        self.scale.is_finite()
            && self.minimum_target.is_finite()
            && self.scale >= 0.0
            && self.minimum_target >= 0.0
    }

    fn normalized(&self) -> Self {
        let defaults = Self::standard();
        Self {
            scale: finite_clamped(self.scale, defaults.scale, 0.0, 4.0),
            minimum_target: finite_non_negative(self.minimum_target, defaults.minimum_target),
        }
    }

    /// Returns `content` scaled by this density while enforcing the minimum
    /// interactive target. Invalid content sizes are treated as zero.
    #[inline]
    pub fn target_size(self, content: f32) -> f32 {
        let content = if content.is_finite() {
            content.max(0.0)
        } else {
            0.0
        };
        let scale = if self.scale.is_finite() {
            self.scale.max(0.0)
        } else {
            1.0
        };
        let minimum = if self.minimum_target.is_finite() {
            self.minimum_target.max(0.0)
        } else {
            0.0
        };
        let scaled = content * scale;
        let scaled = if scaled.is_finite() { scaled } else { f32::MAX };
        scaled.max(minimum)
    }

    /// Returns the density policy for a named ergonomic setting.
    pub const fn for_density(density: Density) -> Self {
        match density {
            Density::Compact => Self {
                scale: 0.9,
                minimum_target: 40.0,
            },
            Density::Standard => Self::standard(),
            Density::Comfortable => Self {
                scale: 1.1,
                minimum_target: 48.0,
            },
        }
    }
}

impl Default for DensityTokens {
    fn default() -> Self {
        Self::standard()
    }
}

/// Named density policies for controls and layout.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Density {
    /// Reduced visual spacing while retaining a 40px target.
    Compact,
    /// The default 44px target.
    Standard,
    /// More generous spacing and a 48px target.
    Comfortable,
}

impl Default for Density {
    fn default() -> Self {
        Self::Standard
    }
}

/// A single elevation level expressed without renderer-specific shadow types.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elevation {
    /// Vertical offset in logical pixels.
    pub offset_y: f32,
    /// Blur radius in logical pixels.
    pub blur: f32,
    /// Spread radius in logical pixels.
    pub spread: f32,
    /// Shadow opacity in the normalized `0..=1` range.
    pub opacity: f32,
}

impl Elevation {
    const fn new(offset_y: f32, blur: f32, spread: f32, opacity: f32) -> Self {
        Self {
            offset_y,
            blur,
            spread,
            opacity,
        }
    }

    fn is_finite(&self) -> bool {
        self.offset_y.is_finite()
            && self.blur.is_finite()
            && self.spread.is_finite()
            && self.opacity.is_finite()
            && self.offset_y >= 0.0
            && self.blur >= 0.0
            && self.spread >= 0.0
            && (0.0..=1.0).contains(&self.opacity)
    }

    fn normalized(&self) -> Self {
        Self {
            offset_y: finite_non_negative(self.offset_y, 0.0),
            blur: finite_non_negative(self.blur, 0.0),
            spread: finite_non_negative(self.spread, 0.0),
            opacity: normalized_opacity(self.opacity),
        }
    }
}

impl Default for Elevation {
    fn default() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0)
    }
}

/// Surface elevation roles from flat content through overlays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevationTokens {
    /// No shadow.
    pub level0: Elevation,
    /// Slightly raised content.
    pub level1: Elevation,
    /// Cards and grouped surfaces.
    pub level2: Elevation,
    /// Menus and floating panels.
    pub level3: Elevation,
    /// Modal or topmost overlay.
    pub level4: Elevation,
}

impl ElevationTokens {
    /// Returns the standard light-surface elevation scale.
    pub const fn standard() -> Self {
        Self {
            level0: Elevation::new(0.0, 0.0, 0.0, 0.0),
            level1: Elevation::new(1.0, 3.0, 0.0, 0.18),
            level2: Elevation::new(2.0, 6.0, 0.0, 0.20),
            level3: Elevation::new(4.0, 10.0, 0.0, 0.22),
            level4: Elevation::new(8.0, 18.0, 0.0, 0.26),
        }
    }

    /// Returns the dark-surface elevation scale.
    pub const fn dark() -> Self {
        Self {
            level0: Elevation::new(0.0, 0.0, 0.0, 0.0),
            level1: Elevation::new(1.0, 3.0, 0.0, 0.28),
            level2: Elevation::new(2.0, 6.0, 0.0, 0.30),
            level3: Elevation::new(4.0, 10.0, 0.0, 0.34),
            level4: Elevation::new(8.0, 18.0, 0.0, 0.38),
        }
    }

    fn is_finite(&self) -> bool {
        self.level0.is_finite()
            && self.level1.is_finite()
            && self.level2.is_finite()
            && self.level3.is_finite()
            && self.level4.is_finite()
    }

    fn normalized(&self) -> Self {
        Self {
            level0: self.level0.normalized(),
            level1: self.level1.normalized(),
            level2: self.level2.normalized(),
            level3: self.level3.normalized(),
            level4: self.level4.normalized(),
        }
    }
}

impl Default for ElevationTokens {
    fn default() -> Self {
        Self::standard()
    }
}

/// Keyboard-focus indicator tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusTokens {
    /// Focus-ring color.
    pub ring_color: Color,
    /// Ring thickness in logical pixels.
    pub ring_width: f32,
    /// Distance between the control edge and the ring.
    pub ring_offset: f32,
    /// Minimum contrast ratio expected for the ring.
    pub minimum_contrast: f32,
}

impl FocusTokens {
    /// Returns the standard light-appearance focus indicator policy.
    pub const fn light() -> Self {
        Self {
            ring_color: Color::BLUE,
            ring_width: 2.0,
            ring_offset: 2.0,
            minimum_contrast: 3.0,
        }
    }

    /// Returns the standard dark-appearance focus indicator policy.
    pub const fn dark() -> Self {
        Self {
            ring_color: Color::YELLOW,
            ring_width: 2.0,
            ring_offset: 2.0,
            minimum_contrast: 3.0,
        }
    }

    /// Returns the high-contrast focus indicator policy.
    pub const fn high_contrast() -> Self {
        Self {
            ring_color: Color::BLACK,
            ring_width: 3.0,
            ring_offset: 2.0,
            minimum_contrast: 4.5,
        }
    }

    fn is_finite(&self) -> bool {
        self.ring_width.is_finite()
            && self.ring_offset.is_finite()
            && self.minimum_contrast.is_finite()
            && self.ring_width >= 0.0
            && self.ring_offset >= 0.0
            && (1.0..=21.0).contains(&self.minimum_contrast)
    }

    fn normalized(&self) -> Self {
        let defaults = Self::light();
        Self {
            ring_color: self.ring_color,
            ring_width: finite_non_negative(self.ring_width, defaults.ring_width),
            ring_offset: finite_non_negative(self.ring_offset, defaults.ring_offset),
            minimum_contrast: finite_clamped(
                self.minimum_contrast,
                defaults.minimum_contrast,
                1.0,
                21.0,
            ),
        }
    }
}

impl Default for FocusTokens {
    fn default() -> Self {
        Self::light()
    }
}

/// A renderer-neutral color/opacity state layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComponentState {
    /// The color used by the state layer.
    pub color: Color,
    /// Layer opacity in the normalized `0..=1` range.
    pub opacity: f32,
}

impl ComponentState {
    /// Creates a state layer, clamping invalid opacity to a safe value.
    pub fn new(color: Color, opacity: f32) -> Self {
        Self {
            color,
            opacity: normalized_opacity(opacity),
        }
    }

    /// Applies the state layer over `base`.
    #[inline]
    pub fn apply(self, base: Color) -> Color {
        apply_state_layer(base, self)
    }

    fn is_finite(&self) -> bool {
        self.opacity.is_finite() && (0.0..=1.0).contains(&self.opacity)
    }

    fn normalized(&self) -> Self {
        Self::new(self.color, self.opacity)
    }
}

impl Default for ComponentState {
    fn default() -> Self {
        Self::new(Color::Transparent, 0.0)
    }
}

/// Component interaction and semantic feedback states.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateTokens {
    /// Disabled content/surface treatment.
    pub disabled: ComponentState,
    /// Pointer-hover treatment.
    pub hover: ComponentState,
    /// Pressed or keyboard-activation treatment.
    pub pressed: ComponentState,
    /// Selected treatment.
    pub selected: ComponentState,
    /// Error treatment.
    pub error: ComponentState,
    /// Success treatment.
    pub success: ComponentState,
}

impl StateTokens {
    const fn light() -> Self {
        Self {
            disabled: ComponentState {
                color: Color::BLACK,
                opacity: 0.38,
            },
            hover: ComponentState {
                color: Color::RED,
                opacity: 0.08,
            },
            pressed: ComponentState {
                color: Color::RED,
                opacity: 0.12,
            },
            selected: ComponentState {
                color: Color::RED,
                opacity: 0.16,
            },
            error: ComponentState {
                color: Color::Rgba(186, 26, 26, 255),
                opacity: 1.0,
            },
            success: ComponentState {
                color: Color::Rgba(25, 120, 55, 255),
                opacity: 1.0,
            },
        }
    }

    const fn dark() -> Self {
        Self {
            disabled: ComponentState {
                color: Color::WHITE,
                opacity: 0.38,
            },
            hover: ComponentState {
                color: Color::Rgba(144, 202, 249, 255),
                opacity: 0.08,
            },
            pressed: ComponentState {
                color: Color::Rgba(144, 202, 249, 255),
                opacity: 0.12,
            },
            selected: ComponentState {
                color: Color::Rgba(144, 202, 249, 255),
                opacity: 0.16,
            },
            error: ComponentState {
                color: Color::Rgba(255, 180, 171, 255),
                opacity: 1.0,
            },
            success: ComponentState {
                color: Color::Rgba(125, 220, 145, 255),
                opacity: 1.0,
            },
        }
    }

    const fn high_contrast() -> Self {
        Self {
            disabled: ComponentState {
                color: Color::BLACK,
                opacity: 0.55,
            },
            hover: ComponentState {
                color: Color::BLUE,
                opacity: 0.18,
            },
            pressed: ComponentState {
                color: Color::BLUE,
                opacity: 0.28,
            },
            selected: ComponentState {
                color: Color::BLUE,
                opacity: 0.36,
            },
            error: ComponentState {
                color: Color::Rgba(160, 0, 0, 255),
                opacity: 1.0,
            },
            success: ComponentState {
                color: Color::Rgba(0, 100, 0, 255),
                opacity: 1.0,
            },
        }
    }

    fn is_finite(&self) -> bool {
        self.disabled.is_finite()
            && self.hover.is_finite()
            && self.pressed.is_finite()
            && self.selected.is_finite()
            && self.error.is_finite()
            && self.success.is_finite()
    }

    fn normalized(&self) -> Self {
        Self {
            disabled: self.disabled.normalized(),
            hover: self.hover.normalized(),
            pressed: self.pressed.normalized(),
            selected: self.selected.normalized(),
            error: self.error.normalized(),
            success: self.success.normalized(),
        }
    }
}

impl Default for StateTokens {
    fn default() -> Self {
        Self::light()
    }
}

/// Component-level aliases that keep controls on semantic token paths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlTokens {
    /// The focus-ring policy used by controls.
    pub focus_ring: FocusTokens,
    /// Minimum control height before density calculation.
    pub min_height: f32,
    /// Default control corner radius.
    pub radius: f32,
}

impl ControlTokens {
    const fn light() -> Self {
        Self {
            focus_ring: FocusTokens::light(),
            min_height: 40.0,
            radius: 8.0,
        }
    }

    const fn dark() -> Self {
        Self {
            focus_ring: FocusTokens::dark(),
            min_height: 40.0,
            radius: 8.0,
        }
    }

    const fn high_contrast() -> Self {
        Self {
            focus_ring: FocusTokens::high_contrast(),
            min_height: 44.0,
            radius: 8.0,
        }
    }

    fn is_finite(&self) -> bool {
        self.focus_ring.is_finite()
            && self.min_height.is_finite()
            && self.radius.is_finite()
            && self.min_height >= 0.0
            && self.radius >= 0.0
    }

    fn normalized(&self) -> Self {
        let defaults = Self::light();
        Self {
            focus_ring: self.focus_ring.normalized(),
            min_height: finite_non_negative(self.min_height, defaults.min_height),
            radius: finite_non_negative(self.radius, defaults.radius),
        }
    }
}

impl Default for ControlTokens {
    fn default() -> Self {
        Self::light()
    }
}

/// Motion duration roles and reduced-motion policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionTokens {
    /// Short feedback transition in milliseconds.
    pub fast_ms: f32,
    /// Standard component transition in milliseconds.
    pub standard_ms: f32,
    /// Long emphasis transition in milliseconds.
    pub slow_ms: f32,
    /// Whether motion should settle immediately.
    pub reduced_motion: bool,
}

impl MotionTokens {
    /// Returns the standard motion policy.
    pub const fn standard() -> Self {
        Self {
            fast_ms: 100.0,
            standard_ms: 200.0,
            slow_ms: 400.0,
            reduced_motion: false,
        }
    }

    /// Returns a reduced-motion policy preserving token identity but disabling
    /// transition durations.
    pub const fn reduced() -> Self {
        Self {
            fast_ms: 100.0,
            standard_ms: 200.0,
            slow_ms: 400.0,
            reduced_motion: true,
        }
    }

    /// Returns a duration role in milliseconds.
    pub const fn duration_ms(self, speed: MotionDuration) -> f32 {
        match speed {
            MotionDuration::Fast => self.fast_ms,
            MotionDuration::Standard => self.standard_ms,
            MotionDuration::Slow => self.slow_ms,
        }
    }

    /// Returns zero for reduced motion, otherwise the requested duration.
    pub fn effective_duration_ms(self, speed: MotionDuration) -> f32 {
        if self.reduced_motion {
            0.0
        } else {
            let duration = self.duration_ms(speed);
            if duration.is_finite() {
                duration.max(0.0)
            } else {
                0.0
            }
        }
    }

    fn is_finite(&self) -> bool {
        self.fast_ms.is_finite()
            && self.standard_ms.is_finite()
            && self.slow_ms.is_finite()
            && self.fast_ms >= 0.0
            && self.standard_ms >= 0.0
            && self.slow_ms >= 0.0
    }

    fn normalized(&self) -> Self {
        let defaults = Self::standard();
        Self {
            fast_ms: finite_clamped(self.fast_ms, defaults.fast_ms, 0.0, 60_000.0),
            standard_ms: finite_clamped(self.standard_ms, defaults.standard_ms, 0.0, 60_000.0),
            slow_ms: finite_clamped(self.slow_ms, defaults.slow_ms, 0.0, 60_000.0),
            reduced_motion: self.reduced_motion,
        }
    }
}

impl Default for MotionTokens {
    fn default() -> Self {
        Self::standard()
    }
}

impl Animatable for MotionTokens {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            fast_ms: self.fast_ms.lerp(&other.fast_ms, t),
            standard_ms: self.standard_ms.lerp(&other.standard_ms, t),
            slow_ms: self.slow_ms.lerp(&other.slow_ms, t),
            reduced_motion: if t < 0.5 {
                self.reduced_motion
            } else {
                other.reduced_motion
            },
        }
    }
}

/// Motion duration roles available from [`MotionTokens`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MotionDuration {
    /// Short feedback transition.
    Fast,
    /// Standard component transition.
    Standard,
    /// Long emphasis transition.
    Slow,
}

impl Default for MotionDuration {
    fn default() -> Self {
        Self::Standard
    }
}

macro_rules! impl_fieldwise_animatable {
    ($type:ty { $($field:ident),+ $(,)? }) => {
        impl Animatable for $type {
            fn lerp(&self, other: &Self, t: f32) -> Self {
                Self {
                    $($field: Animatable::lerp(&self.$field, &other.$field, t),)+
                }
            }
        }
    };
}

impl_fieldwise_animatable!(ColorTokens {
    primary,
    on_primary,
    background,
    on_background,
    surface,
    on_surface,
    outline,
    error,
    on_error,
    success,
    on_success,
});
impl_fieldwise_animatable!(TypographyTokens {
    display,
    headline,
    title,
    body,
    label,
});
impl_fieldwise_animatable!(TypographyStyle {
    font_size,
    line_height,
    font_weight,
    letter_spacing,
});
impl_fieldwise_animatable!(SpacingTokens {
    x_small,
    small,
    medium,
    large,
    x_large,
});
impl_fieldwise_animatable!(ShapeTokens {
    small,
    medium,
    large,
    pill,
});
impl_fieldwise_animatable!(DensityTokens {
    scale,
    minimum_target,
});
impl_fieldwise_animatable!(Elevation {
    offset_y,
    blur,
    spread,
    opacity,
});
impl_fieldwise_animatable!(ElevationTokens {
    level0,
    level1,
    level2,
    level3,
    level4,
});
impl_fieldwise_animatable!(FocusTokens {
    ring_color,
    ring_width,
    ring_offset,
    minimum_contrast,
});
impl_fieldwise_animatable!(ComponentState { color, opacity });
impl_fieldwise_animatable!(StateTokens {
    disabled,
    hover,
    pressed,
    selected,
    error,
    success,
});
impl_fieldwise_animatable!(ControlTokens {
    focus_ring,
    min_height,
    radius,
});

fn normalized_progress(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else if value.is_sign_negative() {
        0.0
    } else {
        1.0
    }
}

fn finite_non_negative(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        fallback
    }
}

fn finite_clamped(value: f32, fallback: f32, minimum: f32, maximum: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        fallback
    }
}

fn normalized_opacity(value: f32) -> f32 {
    finite_clamped(value, 0.0, 0.0, 1.0)
}

/// Calculates the WCAG relative luminance of an sRGB color.
///
/// Alpha is intentionally ignored here; callers that need compositing should
/// use [`contrast_ratio`], which resolves the foreground over the background
/// before measuring both colors.
pub fn relative_luminance(color: Color) -> f32 {
    fn linear(channel: u8) -> f32 {
        let channel = channel as f32 / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    let (red, green, blue, _) = color.to_rgba();
    0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
}

/// Calculates the WCAG contrast ratio between a foreground and background.
///
/// A translucent foreground is composited over the background first. The
/// result is finite and lies in the normal WCAG `1..=21` range for valid color
/// channels.
pub fn contrast_ratio(foreground: Color, background: Color) -> f32 {
    let foreground = composite(foreground, background);
    let foreground_luminance = relative_luminance(foreground);
    let background_luminance = relative_luminance(background);
    let lighter = foreground_luminance.max(background_luminance);
    let darker = foreground_luminance.min(background_luminance);
    ((lighter + 0.05) / (darker + 0.05)).clamp(1.0, 21.0)
}

/// Returns whether a foreground/background pair reaches `minimum_ratio`.
///
/// Invalid thresholds are rejected instead of silently weakening an
/// accessibility requirement.
pub fn meets_contrast(foreground: Color, background: Color, minimum_ratio: f32) -> bool {
    minimum_ratio.is_finite()
        && minimum_ratio >= 1.0
        && contrast_ratio(foreground, background) >= minimum_ratio
}

/// Applies a component state layer over a base color using source-over alpha
/// compositing. Opacity is clamped to `0..=1`, including when a caller built a
/// [`ComponentState`] with a non-finite public field.
pub fn apply_state_layer(base: Color, state: ComponentState) -> Color {
    let (_, _, _, base_alpha) = base.to_rgba();
    let (state_red, state_green, state_blue, state_alpha) = state.color.to_rgba();
    let (base_red, base_green, base_blue, _) = base.to_rgba();
    let state_alpha = normalized_opacity(state.opacity) * state_alpha as f32 / 255.0;
    let base_alpha = base_alpha as f32 / 255.0;
    let output_alpha = state_alpha + base_alpha * (1.0 - state_alpha);

    if output_alpha <= 0.0 {
        return Color::Transparent;
    }

    let channel = |state: u8, base: u8| {
        ((state as f32 * state_alpha + base as f32 * base_alpha * (1.0 - state_alpha))
            / output_alpha)
            .round()
            .clamp(0.0, 255.0) as u8
    };

    Color::Rgba(
        channel(state_red, base_red),
        channel(state_green, base_green),
        channel(state_blue, base_blue),
        (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

fn composite(foreground: Color, background: Color) -> Color {
    let (foreground_red, foreground_green, foreground_blue, foreground_alpha) =
        foreground.to_rgba();
    let (background_red, background_green, background_blue, background_alpha) =
        background.to_rgba();
    let foreground_alpha = foreground_alpha as f32 / 255.0;
    let background_alpha = background_alpha as f32 / 255.0;
    let output_alpha = foreground_alpha + background_alpha * (1.0 - foreground_alpha);

    if output_alpha <= 0.0 {
        return Color::Transparent;
    }

    let channel = |foreground: u8, background: u8| {
        ((foreground as f32 * foreground_alpha
            + background as f32 * background_alpha * (1.0 - foreground_alpha))
            / output_alpha)
            .round()
            .clamp(0.0, 255.0) as u8
    };

    Color::Rgba(
        channel(foreground_red, background_red),
        channel(foreground_green, background_green),
        channel(foreground_blue, background_blue),
        (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_tokens_have_complete_finite_defaults() {
        let tokens = ThemeTokens::light();

        assert_eq!(tokens.typography.body.font_size, 16.0);
        assert_eq!(tokens.spacing.medium, 16.0);
        assert_eq!(tokens.shape.medium, 8.0);
        assert_eq!(tokens.density.minimum_target, 44.0);
        assert!(tokens.is_finite());
    }

    #[test]
    fn tokens_can_be_supplied_to_the_existing_animated_theme_provider() {
        fn assert_theme<T: crate::Theme>() {}

        assert_theme::<ThemeTokens>();
    }

    #[test]
    fn variants_prefer_explicit_values_and_have_deterministic_fallbacks() {
        let light = ThemeTokens::light();
        let dark = ThemeTokens::dark();
        let high_contrast = ThemeTokens::high_contrast();
        let variants = ThemeTokenVariants::new(light)
            .dark(dark)
            .high_contrast(high_contrast);

        assert_eq!(variants.resolve(ThemeVariant::Light), light);
        assert_eq!(variants.resolve(ThemeVariant::Dark), dark);
        assert_eq!(variants.resolve(ThemeVariant::HighContrast), high_contrast);

        let fallback = ThemeTokenVariants::new(light);
        assert_eq!(fallback.resolve(ThemeVariant::Dark), light);
        assert_eq!(
            fallback.resolve(ThemeVariant::HighContrast),
            light.high_contrast_fallback()
        );
    }

    #[test]
    fn token_interpolation_clamps_progress_and_repairs_non_finite_values() {
        let mut begin = ThemeTokens::light();
        begin.spacing.medium = 8.0;
        let mut end = ThemeTokens::dark();
        end.spacing.medium = 24.0;
        end.motion.reduced_motion = true;

        let middle = begin.lerp(&end, 0.5);
        assert_eq!(middle.spacing.medium, 16.0);
        assert!(middle.colors.primary != begin.colors.primary);
        assert!(middle.motion.reduced_motion);

        assert_eq!(begin.lerp(&end, -1.0), begin);
        assert_eq!(begin.lerp(&end, 2.0), end);

        begin.spacing.medium = f32::NAN;
        begin.shape.large = f32::INFINITY;
        let repaired = begin.lerp(&end, f32::NAN);
        assert!(repaired.is_finite());
    }

    #[test]
    fn contrast_and_state_layers_preserve_accessible_color_invariants() {
        assert!((contrast_ratio(Color::WHITE, Color::BLACK) - 21.0).abs() < 0.001);
        assert!(meets_contrast(Color::WHITE, Color::BLACK, 4.5));
        assert!(!meets_contrast(Color::GRAY, Color::WHITE, 4.5));

        let base = Color::WHITE;
        assert_eq!(
            apply_state_layer(base, ComponentState::new(Color::BLACK, 0.0)),
            base
        );
        assert_eq!(
            apply_state_layer(base, ComponentState::new(Color::BLACK, 1.0)),
            Color::BLACK
        );
        assert_eq!(
            apply_state_layer(
                base,
                ComponentState {
                    color: Color::BLACK,
                    opacity: f32::NAN,
                },
            ),
            base
        );
    }

    #[test]
    fn density_enforces_minimum_targets_after_scaling() {
        let standard = DensityTokens::for_density(Density::Standard);
        assert_eq!(standard.target_size(12.0), 44.0);
        assert_eq!(standard.target_size(60.0), 60.0);

        let compact = DensityTokens::for_density(Density::Compact);
        assert_eq!(compact.target_size(30.0), 40.0);
        assert_eq!(compact.target_size(50.0), 45.0);
        assert_eq!(standard.target_size(f32::NAN), 44.0);
        assert!(standard.target_size(f32::MAX).is_finite());
    }

    #[test]
    fn reduced_motion_disables_effective_duration_without_changing_roles() {
        let standard = MotionTokens::standard();
        let reduced = MotionTokens::reduced();

        assert_eq!(
            standard.effective_duration_ms(MotionDuration::Standard),
            200.0
        );
        assert_eq!(reduced.duration_ms(MotionDuration::Standard), 200.0);
        assert_eq!(reduced.effective_duration_ms(MotionDuration::Standard), 0.0);
    }
}
