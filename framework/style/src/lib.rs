extern crate self as aimer_style;

mod animated_theme;
mod style;
mod theme;
mod theme_controller;
mod theme_mode;
mod tokens;

pub use aimer_macro::Theme;
pub use aimer_widget::Brightness;
pub use animated_theme::AnimatedTheme;
pub use theme::{Theme, ThemeData};
pub use theme_controller::{ThemeController, ThemeSnapshot};
pub use theme_mode::{ThemeMode, ThemeSelection};
pub use tokens::{
    ColorTokens, ComponentState, ControlTokens, Density, DensityTokens, Elevation, ElevationTokens,
    FocusTokens, MotionDuration, MotionTokens, ShapeTokens, SpacingScale, SpacingTokens,
    StateTokens, ThemeTokenVariants, ThemeTokens, ThemeVariant, TypographyStyle, TypographyTokens,
    apply_state_layer, contrast_ratio, meets_contrast, relative_luminance,
};

#[doc(hidden)]
pub mod __private {
    use std::hash::Hash;

    pub use aimer_animation::Animatable;
    pub use aimer_widget::{ContentHash, ContentHasher, HashContent};

    /// What `#[derive(Theme)]` hashes a field through.
    ///
    /// Which of the three traits below answers is picked by method resolution on
    /// `(&&Probe(&field))`, so a field is hashed by its [`HashContent`] when it
    /// has one, by [`Hash`] when it has that instead, and otherwise marks the
    /// theme as unhashable. This is autoref-based specialization: it resolves at
    /// the field's concrete type, so a field of a generic type parameter is
    /// always unhashable.
    pub struct Probe<'a, T: ?Sized>(pub &'a T);

    /// A field with [`HashContent`]: highest priority.
    pub trait ViaContent {
        fn hash_field(&self, hasher: &mut ContentHasher);
    }
    impl<T: HashContent + ?Sized> ViaContent for &Probe<'_, T> {
        #[inline]
        fn hash_field(&self, hasher: &mut ContentHasher) {
            self.0.hash_content(hasher);
        }
    }

    /// A field with [`Hash`] only.
    pub trait ViaHash {
        fn hash_field(&self, hasher: &mut ContentHasher);
    }
    impl<T: Hash + ?Sized> ViaHash for &&Probe<'_, T> {
        #[inline]
        fn hash_field(&self, hasher: &mut ContentHasher) {
            self.0.hash(hasher);
        }
    }

    /// A field with neither: the theme has no hash.
    pub trait ViaNothing {
        fn hash_field(&self, hasher: &mut ContentHasher);
    }
    impl<T: ?Sized> ViaNothing for Probe<'_, T> {
        #[inline]
        fn hash_field(&self, hasher: &mut ContentHasher) {
            hasher.mark_unavailable();
        }
    }
}

// layout export
pub use aimer_attribute::BoxConstraint;
pub use style::alignment::{ColumnAlignment, RowAlignment};
// Compatibility exports; decoration types are owned by aimer_decoration.
pub use aimer_decoration::{
    BorderRadius, BorderSlice, BorderStyle, BoxBorder, BoxDecoration, BoxOutline, BoxShadow,
    ShadowSide, Stroke,
};
pub use style::box_fit::BoxFit;
pub use style::layout_spacing::{LayoutSpacing, Spacing};
// text export
pub use style::text_style::*;
