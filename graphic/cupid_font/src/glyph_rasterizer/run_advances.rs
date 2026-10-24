//! Advances of the characters of one measured run.
//!
//! Measuring text asks for the advance of every character, and resolving a
//! character to a glyph is the expensive part of that: the family's best face,
//! its glyph index, and the weight to address it at, all before the advance
//! cache is even consulted. Running prose uses a few dozen distinct characters
//! over hundreds of positions, so almost every one of those resolutions repeats
//! an answer the previous paragraph's measurement already found.

use super::GlyphRasterizer;
use crate::font::{FontFamily, FontStyle, FontWeight};

/// The advances of the characters measured so far in one run.
///
/// A run is one call to measure text: the style and size are fixed, and so is
/// the script run announced by [`GlyphRasterizer::begin_script_run`], which is
/// all that a character's glyph can depend on besides the character. An
/// advance remembered here is therefore the one the rasterizer would give again,
/// and measuring through a `RunAdvances` gives exactly the widths measuring
/// without one does. Create one per run: it must not outlive the script run or
/// be shared between styles.
///
/// Only ASCII is remembered. It is what repeats in a paragraph, and a table of
/// 128 entries costs less to build and ask than a hash map does to hold the
/// handful of characters a line uses.
pub struct RunAdvances {
    ascii: [f32; 128],
}

impl RunAdvances {
    /// An empty memory, for one run.
    #[inline]
    pub fn new() -> Self {
        Self {
            ascii: [f32::NAN; 128],
        }
    }

    /// The advance of `codepoint` in the run's style.
    #[inline]
    pub fn advance(
        &mut self,
        rasterizer: &mut GlyphRasterizer,
        codepoint: char,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> f32 {
        let Some(slot) = self.ascii.get_mut(codepoint as usize) else {
            return rasterizer.advance_width_for_family(codepoint, font_size, family, weight, style);
        };
        // `NaN` marks a character not asked for yet; an advance is never `NaN`.
        if slot.is_nan() {
            *slot = rasterizer.advance_width_for_family(codepoint, font_size, family, weight, style);
        }
        *slot
    }
}

impl Default for RunAdvances {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_pipeline::glyph_rasterizer::key_lookups;

    const PROSE: &str = "Paragraph 12: The compositor keeps this decorated scrolling row on the \
        live culled path because its container owns clipping and hit-test bounds. Text shaping \
        is cached per width, while stable paint islands elsewhere can be retained automatically.";

    fn styles() -> [(FontFamily, FontWeight, FontStyle); 3] {
        [
            (FontFamily::SANS_SERIF, FontWeight::Normal, FontStyle::Normal),
            (FontFamily::SANS_SERIF, FontWeight::Bold, FontStyle::Italic),
            (FontFamily::MONOSPACE, FontWeight::Normal, FontStyle::Normal),
        ]
    }

    #[test]
    fn a_run_reports_the_advances_the_rasterizer_does() {
        // ASCII, accents, ideographs, an emoji, a tab and a newline: the ones
        // that are remembered and the ones that are not.
        let text = format!("{PROSE} é ü ñ 中文字 日本語 😀 \t\n end");
        for (family, weight, style) in styles() {
            let mut rasterizer = GlyphRasterizer::new();
            let mut run = RunAdvances::new();
            rasterizer.begin_script_run(&text, None);
            let remembered: Vec<f32> = text
                .chars()
                .map(|c| run.advance(&mut rasterizer, c, 16.0, family, weight, style))
                .collect();
            let direct: Vec<f32> = text
                .chars()
                .map(|c| rasterizer.advance_width_for_family(c, 16.0, family, weight, style))
                .collect();
            rasterizer.end_script_run();
            assert_eq!(remembered, direct);
        }
    }

    #[test]
    fn a_run_resolves_each_ascii_character_once() {
        let mut rasterizer = GlyphRasterizer::new();
        let mut run = RunAdvances::new();
        let distinct = PROSE
            .chars()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        rasterizer.begin_script_run(PROSE, None);

        let before = key_lookups();
        for c in PROSE.chars() {
            run.advance(
                &mut rasterizer,
                c,
                16.0,
                FontFamily::SANS_SERIF,
                FontWeight::Normal,
                FontStyle::Normal,
            );
        }
        let lookups = key_lookups() - before;
        rasterizer.end_script_run();

        assert_eq!(
            lookups, distinct,
            "{lookups} glyph lookups for {} characters with {distinct} distinct ones",
            PROSE.chars().count()
        );
    }
}
