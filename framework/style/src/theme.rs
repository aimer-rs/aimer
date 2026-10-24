use aimer_animation::Animatable;
use aimer_color::prelude::Color;
use aimer_provider::{
    PortableProviderCodec, PortableProviderCodecError, ProviderContext, Snapshot,
};
use aimer_widget::Brightness;
use aimer_widget::base::BuildContext;
use aimer_widget::portable::__anteros::{
    THEME_DATA_VALUE_MAXIMUM_ENCODED_BYTES, THEME_DATA_VALUE_NAME, THEME_DATA_VALUE_VERSION,
    ValueSchemaMetadata, Version,
};
use std::any::type_name;

/// Semantic colors used by themed widgets.
///
/// Start with [`ThemeData::light`] or [`ThemeData::dark`], then replace
/// individual colors with the builder methods. The `on_*` colors are intended
/// for content drawn on top of the corresponding base color.
///
/// # Examples
///
/// ```
/// use aimer_color::prelude::Color;
/// use aimer_style::ThemeData;
///
/// let theme = ThemeData::light().primary_color(Color::RED)
///                               .on_primary_color(Color::WHITE);
///
/// assert_eq!(theme.primary_color, Color::RED);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeData {
    /// The primary accent color.
    pub primary_color: Color,
    /// The preferred content color on [`ThemeData::primary_color`].
    pub on_primary_color: Color,
    /// The color behind the main application content.
    pub background_color: Color,
    /// The preferred content color on [`ThemeData::background_color`].
    pub on_background_color: Color,
    /// The color of elevated or grouped surfaces.
    pub surface_color: Color,
    /// The preferred content color on [`ThemeData::surface_color`].
    pub on_surface_color: Color,
}

impl ThemeData {
    /// Creates the default light theme.
    pub const fn new() -> Self {
        Self::light()
    }

    /// Creates Aimer's built-in light theme.
    pub const fn light() -> Self {
        Self {
            primary_color: Color::RED,
            on_primary_color: Color::WHITE,
            background_color: Color::WHITE,
            on_background_color: Color::BLACK,
            surface_color: Color::WHITE,
            on_surface_color: Color::BLACK,
        }
    }

    /// Creates Aimer's built-in dark theme.
    pub const fn dark() -> Self {
        Self {
            primary_color: Color::Rgba(144, 202, 249, 255),
            on_primary_color: Color::BLACK,
            background_color: Color::Rgba(18, 18, 18, 255),
            on_background_color: Color::WHITE,
            surface_color: Color::Rgba(30, 30, 30, 255),
            on_surface_color: Color::WHITE,
        }
    }

    /// Creates the built-in theme matching an appearance.
    ///
    /// This is how the system appearance becomes a theme: the platform reports
    /// a [`Brightness`] and the application answers with the theme it draws in.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_style::{Brightness, ThemeData};
    ///
    /// assert_eq!(ThemeData::for_brightness(Brightness::Dark), ThemeData::dark());
    /// ```
    pub const fn for_brightness(brightness: Brightness) -> Self {
        match brightness {
            Brightness::Light => Self::light(),
            Brightness::Dark => Self::dark(),
        }
    }

    /// Sets the primary accent color.
    #[inline]
    pub fn primary_color(mut self, color: Color) -> Self {
        self.primary_color = color;
        self
    }

    /// Sets the preferred content color on the primary color.
    #[inline]
    pub fn on_primary_color(mut self, color: Color) -> Self {
        self.on_primary_color = color;
        self
    }

    /// Sets the main application background color.
    #[inline]
    pub fn background_color(mut self, color: Color) -> Self {
        self.background_color = color;
        self
    }

    /// Sets the preferred content color on the background color.
    #[inline]
    pub fn on_background_color(mut self, color: Color) -> Self {
        self.on_background_color = color;
        self
    }

    /// Sets the color of elevated or grouped surfaces.
    #[inline]
    pub fn surface_color(mut self, color: Color) -> Self {
        self.surface_color = color;
        self
    }

    /// Sets the preferred content color on the surface color.
    #[inline]
    pub fn on_surface_color(mut self, color: Color) -> Self {
        self.on_surface_color = color;
        self
    }

    /// Derives the semantic component tokens for this core-color theme.
    ///
    /// The returned value is independent and may be supplied to a separate
    /// [`crate::AnimatedTheme`] when a widget family needs semantic styling.
    /// Keeping this bridge additive preserves the existing six-color portable
    /// `ThemeData` codec while allowing token themes to evolve independently.
    #[inline]
    pub fn tokens(&self) -> crate::ThemeTokens {
        crate::ThemeTokens::from_core_colors(
            self.primary_color,
            self.on_primary_color,
            self.background_color,
            self.on_background_color,
            self.surface_color,
            self.on_surface_color,
        )
    }

    /// Returns the built-in bounded codec used to carry `ThemeData` through a
    /// portable provider node.
    ///
    /// The wire order is the six public fields, each as RGBA bytes. The exact
    /// length and version are checked on decode so malformed guest payloads are
    /// rejected before native materialization.
    pub fn portable_codec() -> PortableProviderCodec<Self> {
        PortableProviderCodec::new(
            ValueSchemaMetadata::from_canonical_name(
                THEME_DATA_VALUE_NAME,
                THEME_DATA_VALUE_VERSION,
                THEME_DATA_VALUE_MAXIMUM_ENCODED_BYTES,
            ),
            encode_theme_data,
            decode_theme_data,
        )
    }
}

fn encode_theme_data(theme: &ThemeData) -> Result<Vec<u8>, PortableProviderCodecError> {
    let mut bytes = Vec::with_capacity(THEME_DATA_VALUE_MAXIMUM_ENCODED_BYTES as usize);
    for color in [
        theme.primary_color,
        theme.on_primary_color,
        theme.background_color,
        theme.on_background_color,
        theme.surface_color,
        theme.on_surface_color,
    ] {
        let (red, green, blue, alpha) = color.to_rgba();
        bytes.extend_from_slice(&[red, green, blue, alpha]);
    }
    Ok(bytes)
}

fn decode_theme_data(
    bytes: &[u8],
    version: Version,
) -> Result<ThemeData, PortableProviderCodecError> {
    if version != THEME_DATA_VALUE_VERSION {
        return Err(PortableProviderCodecError::new(format!(
            "unsupported ThemeData codec version {}.{}",
            version.major(),
            version.minor(),
        )));
    }
    if bytes.len() != THEME_DATA_VALUE_MAXIMUM_ENCODED_BYTES as usize {
        return Err(PortableProviderCodecError::new(format!(
            "ThemeData payload must be exactly {} bytes, got {}",
            THEME_DATA_VALUE_MAXIMUM_ENCODED_BYTES,
            bytes.len(),
        )));
    }
    let mut colors = [Color::Transparent; 6];
    for (color, bytes) in colors.iter_mut().zip(bytes.chunks_exact(4)) {
        *color = Color::Rgba(bytes[0], bytes[1], bytes[2], bytes[3]);
    }
    Ok(ThemeData {
        primary_color: colors[0],
        on_primary_color: colors[1],
        background_color: colors[2],
        on_background_color: colors[3],
        surface_color: colors[4],
        on_surface_color: colors[5],
    })
}

impl Animatable for ThemeData {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        if t <= 0.0 {
            return *self;
        }
        if t >= 1.0 {
            return *other;
        }
        Self {
            primary_color: self.primary_color.lerp(other.primary_color, t),
            on_primary_color: self.on_primary_color.lerp(other.on_primary_color, t),
            background_color: self.background_color.lerp(other.background_color, t),
            on_background_color: self.on_background_color.lerp(other.on_background_color, t),
            surface_color: self.surface_color.lerp(other.surface_color, t),
            on_surface_color: self.on_surface_color.lerp(other.on_surface_color, t),
        }
    }
}

impl Default for ThemeData {
    fn default() -> Self {
        Self::new()
    }
}

/// Accesses the nearest theme supplied by an [`crate::AnimatedTheme`] ancestor.
///
/// Use [`Theme::of`] while building themed widgets so they rebuild as the theme
/// animates. Use [`Theme::read`] when the caller only needs the current value
/// and should not subscribe to future changes.
///
/// A derived theme requires every field to implement [`Animatable`]:
///
/// ```compile_fail
/// use aimer_style::Theme;
///
/// #[derive(Clone, PartialEq, Theme)]
/// struct InvalidTheme {
///     label: String,
/// }
/// ```
pub trait Theme: Animatable + Clone + PartialEq + Sized + 'static {
    /// Returns the explicit codec used when this theme crosses a portable
    /// provider boundary.
    ///
    /// Derived or custom themes return `None` until their owner supplies a
    /// stable versioned codec. Native themes remain fully supported without
    /// one; a portable build reports the missing contract at the source site.
    #[inline]
    fn portable_codec() -> Option<PortableProviderCodec<Self>> {
        None
    }

    /// Returns the current theme and subscribes the building widget to theme
    /// changes.
    ///
    /// # Panics
    ///
    /// Panics when there is no [`crate::AnimatedTheme`] ancestor or when called
    /// outside a widget build.
    ///
    /// The panic is raised in the body of this `#[track_caller]` method rather
    /// than inside a closure: a closure is an untracked frame, so panicking
    /// there blames this file instead of the widget that asked for the theme.
    #[track_caller]
    fn of(context: &BuildContext) -> Snapshot<Self> {
        let Some(theme) = context.try_watch() else {
            panic!(
                "No provider for `{}` found in the current widget scope",
                type_name::<Self>()
            )
        };
        theme
    }

    /// Returns the current theme without subscribing the building widget to
    /// theme changes.
    ///
    /// # Panics
    ///
    /// Panics when there is no [`crate::AnimatedTheme`] ancestor.
    #[track_caller]
    fn read(context: &BuildContext) -> Snapshot<Self> {
        let Some(theme) = context.try_read() else {
            panic!(
                "No provider for `{}` found in the current widget scope",
                type_name::<Self>()
            )
        };
        theme
    }

    /// Returns a copy of the current theme without subscribing the building
    /// widget to changes.
    ///
    /// # Panics
    ///
    /// Panics when there is no provider for this theme type.
    #[track_caller]
    fn copied(context: &BuildContext) -> Self
    where
        Self: Copy,
    {
        let Some(theme) = context.try_copied::<Self>() else {
            panic!(
                "No provider for `{}` found in the current widget scope",
                type_name::<Self>()
            )
        };
        theme
    }
}

impl Theme for ThemeData {
    fn portable_codec() -> Option<PortableProviderCodec<Self>> {
        Some(Self::portable_codec())
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use aimer_animation::Animatable;
    use aimer_utils::PanicSite;
    use aimer_widget::base::WindowHandle;

    use super::*;
    use crate::Theme as ThemeDerive;

    #[derive(Clone, Debug, PartialEq, ThemeDerive)]
    struct DirectTheme {
        value: f32,
    }

    fn theme(color: Color) -> ThemeData {
        ThemeData::new()
            .primary_color(color)
            .on_primary_color(color)
            .background_color(color)
            .on_background_color(color)
            .surface_color(color)
            .on_surface_color(color)
    }

    #[test]
    fn lerp_preserves_endpoints() {
        let begin = theme(Color::Rgba(10, 20, 30, 40));
        let end = theme(Color::Rgba(110, 120, 130, 140));

        assert_eq!(Animatable::lerp(&begin, &end, -0.5), begin);
        assert_eq!(Animatable::lerp(&begin, &end, 0.0), begin);
        assert_eq!(Animatable::lerp(&begin, &end, 1.0), end);
        assert_eq!(Animatable::lerp(&begin, &end, 1.5), end);
    }

    #[test]
    fn lerp_interpolates_every_semantic_color() {
        let begin = theme(Color::Rgba(0, 20, 40, 60));
        let end = theme(Color::Rgba(100, 120, 140, 160));
        let expected = theme(Color::Rgba(50, 70, 90, 110));

        assert_eq!(Animatable::lerp(&begin, &end, 0.5), expected);
    }

    #[test]
    fn theme_data_implements_theme_contract() {
        fn assert_theme<T: Theme>() {}

        assert_theme::<ThemeData>();
    }

    #[test]
    fn theme_data_bridges_core_colors_to_semantic_tokens() {
        let value = ThemeData::light()
            .primary_color(Color::Rgba(10, 20, 30, 255))
            .on_surface_color(Color::Rgba(220, 220, 220, 255));
        let tokens = value.tokens();

        assert_eq!(tokens.colors.primary, value.primary_color);
        assert_eq!(tokens.colors.on_surface, value.on_surface_color);
        assert_eq!(tokens.control.focus_ring.ring_color, value.primary_color);
        assert!(tokens.is_finite());
    }

    #[test]
    fn theme_data_portable_codec_rejects_wrong_version_and_length() {
        let codec = ThemeData::portable_codec();
        let wrong_version = codec.decode(&[0; 24], Version::new(2, 0));
        let wrong_length = codec.decode(&[0; 23], THEME_DATA_VALUE_VERSION);

        assert!(wrong_version.is_err());
        assert!(wrong_length.is_err());
    }

    #[test]
    fn derived_custom_theme_requires_an_explicit_portable_codec() {
        assert!(<DirectTheme as Theme>::portable_codec().is_none());
    }

    #[test]
    fn derive_works_for_direct_aimer_style_consumers() {
        let begin = DirectTheme { value: 2.0 };
        let end = DirectTheme { value: 6.0 };

        assert_eq!(begin.lerp(&end, 0.5), DirectTheme { value: 4.0 });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn context() -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        BuildContext::new(
            canvas,
            Default::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn a_missing_theme_provider_is_blamed_on_the_calling_line() {
        let context = context();

        let watch = PanicSite::watch();
        let expected_line = line!() + 2;
        let payload = catch_unwind(AssertUnwindSafe(|| {
            let _ = ThemeData::of(&context);
        }))
        .expect_err("a theme lookup without a provider should panic");
        let site = watch
            .take_site()
            .expect("the panic site should be recorded");
        let message = payload
            .downcast_ref::<String>()
            .expect("the diagnostic should be an owned message");

        assert_eq!(site.file(), file!());
        assert_eq!(site.line(), expected_line);
        assert_eq!(message.lines().count(), 1, "{message}");
        assert!(message.contains("ThemeData"), "{message}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn a_missing_theme_provider_highlights_the_calling_expression() {
        let context = context();

        let watch = PanicSite::watch();
        let rendered = catch_unwind(AssertUnwindSafe(|| {
            let _ = ThemeData::read(&context);
        }))
        .err()
        .and_then(|_| watch.take_site())
        .expect("the panic site should be recorded")
        .to_string();

        assert!(rendered.starts_with("at "), "{rendered}");
        assert!(rendered.contains("ThemeData::read(&context)"), "{rendered}");
        assert!(rendered.contains("^^^"), "{rendered}");
    }
}
