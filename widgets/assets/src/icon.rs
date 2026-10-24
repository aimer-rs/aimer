//! Theme- and direction-aware icon source contracts.

use super::AssetRef;

/// A glyph, vector, raster, or contextual icon source.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum IconSource {
    /// A glyph from a named font family.
    Glyph {
        /// Font family used to draw the glyph.
        family: String,
        /// Unicode scalar used by the family.
        glyph: char,
    },
    /// A vector asset delegated to the existing SVG asset loader.
    Vector(AssetRef),
    /// A raster asset delegated to the existing image asset loader.
    Raster(AssetRef),
    /// Chooses a source from the active light or dark theme.
    Theme {
        /// Source for a light theme.
        light: Box<IconSource>,
        /// Source for a dark theme.
        dark: Box<IconSource>,
    },
    /// Chooses a source from the active text direction.
    Directional {
        /// Source for left-to-right layouts.
        ltr: Box<IconSource>,
        /// Source for right-to-left layouts.
        rtl: Box<IconSource>,
    },
}

impl IconSource {
    /// Creates a font glyph source.
    pub fn glyph(family: impl Into<String>, glyph: char) -> Self {
        Self::Glyph {
            family: family.into(),
            glyph,
        }
    }

    /// Creates a vector source backed by an asset reference.
    pub fn vector(asset: AssetRef) -> Self {
        Self::Vector(asset)
    }

    /// Creates a raster source backed by an asset reference.
    pub fn raster(asset: AssetRef) -> Self {
        Self::Raster(asset)
    }

    /// Creates a light/dark theme source.
    pub fn themed(light: Self, dark: Self) -> Self {
        Self::Theme {
            light: Box::new(light),
            dark: Box::new(dark),
        }
    }

    /// Creates an LTR/RTL source.
    pub fn directional(ltr: Self, rtl: Self) -> Self {
        Self::Directional {
            ltr: Box::new(ltr),
            rtl: Box::new(rtl),
        }
    }

    fn resolve(&self, context: IconContext) -> Self {
        match self {
            Self::Theme { light, dark } => match context.theme {
                IconTheme::Light => light.resolve(context),
                IconTheme::Dark => dark.resolve(context),
            },
            Self::Directional { ltr, rtl } => match context.direction {
                IconDirection::Ltr => ltr.resolve(context),
                IconDirection::Rtl => rtl.resolve(context),
            },
            source => source.clone(),
        }
    }
}

/// A four-channel icon tint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IconTint {
    rgba: [u8; 4],
}

impl IconTint {
    /// Creates an RGBA tint.
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            rgba: [red, green, blue, alpha],
        }
    }

    /// Returns the RGBA channels.
    pub const fn channels(self) -> [u8; 4] {
        self.rgba
    }
}

/// The active visual theme for contextual icon resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconTheme {
    /// Light visual theme.
    Light,
    /// Dark visual theme.
    Dark,
}

/// The active layout direction for contextual icon resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconDirection {
    /// Left-to-right layout.
    Ltr,
    /// Right-to-left layout.
    Rtl,
}

/// Context used to resolve a contextual icon source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IconContext {
    theme: IconTheme,
    direction: IconDirection,
    high_contrast: bool,
}

impl IconContext {
    /// Creates a visual context.
    pub const fn new(theme: IconTheme, direction: IconDirection, high_contrast: bool) -> Self {
        Self {
            theme,
            direction,
            high_contrast,
        }
    }

    /// Returns the active theme.
    pub const fn theme(self) -> IconTheme {
        self.theme
    }

    /// Returns the active direction.
    pub const fn direction(self) -> IconDirection {
        self.direction
    }

    /// Returns whether high contrast is active.
    pub const fn high_contrast(self) -> bool {
        self.high_contrast
    }
}

/// A reason an icon builder rejected a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconError {
    /// The requested size was not finite and positive.
    InvalidSize,
}

/// An icon ready for a renderer adapter after contextual resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedIcon {
    source: IconSource,
    size: f32,
    tint: Option<IconTint>,
}

impl ResolvedIcon {
    /// Returns the selected source.
    pub fn source(&self) -> &IconSource {
        &self.source
    }

    /// Returns the validated logical size.
    pub const fn size(&self) -> f32 {
        self.size
    }

    /// Returns the selected tint.
    pub const fn tint(&self) -> Option<IconTint> {
        self.tint
    }
}

/// A reusable icon declaration with size and contrast policy.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    source: IconSource,
    size: f32,
    tint: Option<IconTint>,
    high_contrast_tint: Option<IconTint>,
}

impl Icon {
    /// Creates an icon with a 24 logical-pixel default size.
    pub fn new(source: IconSource) -> Self {
        Self {
            source,
            size: 24.0,
            tint: None,
            high_contrast_tint: None,
        }
    }

    /// Sets a finite, positive logical size.
    pub fn size(mut self, size: f32) -> Result<Self, IconError> {
        if !size.is_finite() || size <= 0.0 {
            return Err(IconError::InvalidSize);
        }
        self.size = size;
        Ok(self)
    }

    /// Sets the normal tint.
    pub const fn tint(mut self, tint: IconTint) -> Self {
        self.tint = Some(tint);
        self
    }

    /// Sets the tint used when high contrast is active.
    pub const fn high_contrast_tint(mut self, tint: IconTint) -> Self {
        self.high_contrast_tint = Some(tint);
        self
    }

    /// Returns the declared source before contextual resolution.
    pub fn source(&self) -> &IconSource {
        &self.source
    }

    /// Resolves theme, direction, and contrast policy into renderer data.
    pub fn resolve(&self, context: IconContext) -> ResolvedIcon {
        ResolvedIcon {
            source: self.source.resolve(context),
            size: self.size,
            tint: if context.high_contrast() {
                self.high_contrast_tint.or(self.tint)
            } else {
                self.tint
            },
        }
    }
}
