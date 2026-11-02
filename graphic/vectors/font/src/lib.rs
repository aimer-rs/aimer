//! Font parsing, shaping, fallback and glyph rasterization for Cupid.
//!
//! Nothing here touches the GPU: the crate turns font bytes and text into
//! shaped runs and glyph coverage, and `aimer_cupid` uploads the results.
//! `aimer_cupid` re-exports [`font`], [`glyph_rasterizer`] and [`text_layout`]
//! under their historical paths.

// The SVG-in-OpenType glyph path and the rasterizer name the SVG types through
// this alias.
pub(crate) use aimer_cupid_svg as svg;

#[doc(hidden)]
pub mod aimer_font;
#[cfg(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
))]
#[doc(hidden)]
pub mod apple_fonts;
#[cfg(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
))]
#[doc(hidden)]
pub mod core_text_raster;
pub mod font;
#[doc(hidden)]
pub mod font_resolver;
#[doc(hidden)]
pub mod glyph_metrics;
pub mod glyph_rasterizer;
#[doc(hidden)]
pub mod system_fallback;
pub mod text_layout;
#[doc(hidden)]
pub mod unicode_script;

#[cfg(test)]
mod phase0_baseline;
#[cfg(test)]
mod phase6_verification;
