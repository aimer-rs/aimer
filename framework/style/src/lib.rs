extern crate self as aimer_style;

mod animated_theme;
mod style;
mod theme;
mod theme_mode;
mod tokens;

pub use aimer_macro::Theme;
pub use aimer_widget::Brightness;
pub use animated_theme::AnimatedTheme;
pub use theme::{Theme, ThemeData};
pub use theme_mode::{ThemeMode, ThemeSelection};
pub use tokens::{
    ColorTokens, ComponentState, ControlTokens, Density, DensityTokens, Elevation, ElevationTokens,
    FocusTokens, MotionDuration, MotionTokens, ShapeTokens, SpacingScale, SpacingTokens,
    StateTokens, ThemeTokenVariants, ThemeTokens, ThemeVariant, TypographyStyle, TypographyTokens,
    apply_state_layer, contrast_ratio, meets_contrast, relative_luminance,
};

#[doc(hidden)]
pub mod __private {
    pub use aimer_animation::Animatable;
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
