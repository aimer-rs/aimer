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

/// How many styles' ASCII advances a rasterizer remembers at once.
///
/// A window rarely draws more than a handful of text styles at a time (body,
/// bold, italic, a few sizes); past this the oldest is dropped, which only
/// costs it resolving its characters again.
const MAX_REMEMBERED_STYLES: usize = 8;

/// What the ASCII advances of one text style are remembered under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AsciiAdvanceKey {
    family: FontFamily,
    /// Bits, not a float: sizes are compared exactly, as the advance cache
    /// that sits behind them compares them.
    size_bits: u32,
    weight: FontWeight,
    style: FontStyle,
}

impl AsciiAdvanceKey {
    #[inline]
    fn new(font_size: f32, family: FontFamily, weight: FontWeight, style: FontStyle) -> Self {
        Self {
            family,
            size_bits: font_size.to_bits(),
            weight,
            style,
        }
    }
}

/// The advances of the characters of one measured run.
///
/// Measuring text asks for the advance of every character, and resolving a
/// character to a glyph is the expensive part of that: the family's best face,
/// its glyph index, and the weight to address it at, all before the advance
/// cache is even consulted. Running prose uses a few dozen distinct characters
/// over hundreds of positions, so almost every one of those resolutions repeats
/// an answer the previous paragraph's measurement already found.
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
///
/// # Remembering across runs
///
/// [`RunAdvances::new`] forgets everything when the run ends. [`RunAdvances::begin`]
/// and [`RunAdvances::finish`] instead hand the table back to the rasterizer, so
/// the next run of the same style starts with every character the earlier ones
/// met. That is only done where the answer cannot depend on the run: the text
/// announced no script requirement and no kana, so the weight a glyph is
/// addressed at is the style's own, and only for characters the family's own
/// face covers, so which fallback lanes are loaded cannot matter. Anything else
/// keeps the run-local behaviour.
pub struct RunAdvances {
    ascii: [f32; 128],
    /// The style whose table this is, when it came from the rasterizer and may
    /// go back to it. Cleared the moment a character is answered by anything
    /// but the family's own face.
    shared: Option<(AsciiAdvanceKey, u64)>,
}

impl RunAdvances {
    /// An empty memory, for one run.
    #[inline]
    pub fn new() -> Self {
        Self {
            ascii: [f32::NAN; 128],
            shared: None,
        }
    }

    /// A memory for one run that starts with what earlier runs of the same
    /// style learned, when that is exact.
    ///
    /// Call it after [`GlyphRasterizer::begin_script_run`], which decides
    /// whether it is, and end the run with [`RunAdvances::finish`]. Every
    /// [`RunAdvances::advance`] of the run must use the style given here.
    pub fn begin(
        rasterizer: &mut GlyphRasterizer,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> Self {
        let mut run = Self::new();
        let key = AsciiAdvanceKey::new(font_size, family, weight, style);
        if let Some((table, revision)) = rasterizer.take_ascii_advances(key) {
            run.ascii = table;
            run.shared = Some((key, revision));
        }
        run
    }

    /// Ends a run begun with [`RunAdvances::begin`], keeping what it learned for
    /// the next run of the same style. A no-op for a run that could not share.
    pub fn finish(self, rasterizer: &mut GlyphRasterizer) {
        if let Some((key, revision)) = self.shared {
            rasterizer.store_ascii_advances(key, revision, self.ascii);
        }
    }

    /// The advance of `codepoint` in the run's style.
    ///
    /// Always inlined: a remembered character is one array read, and in a debug
    /// build a call per character of every measured paragraph costs more than
    /// the read does. Only the first sight of a character leaves the caller.
    #[inline(always)]
    pub fn advance(
        &mut self,
        rasterizer: &mut GlyphRasterizer,
        codepoint: char,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> f32 {
        let index = codepoint as usize;
        if index >= self.ascii.len() {
            return rasterizer.advance_width_for_family(codepoint, font_size, family, weight, style);
        }
        // `NaN` marks a character not asked for yet; an advance is never `NaN`.
        if self.ascii[index].is_nan() {
            self.ascii[index] = self.resolve(rasterizer, codepoint, font_size, family, weight, style);
        }
        self.ascii[index]
    }

    #[cold]
    fn resolve(
        &mut self,
        rasterizer: &mut GlyphRasterizer,
        codepoint: char,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> f32 {
        // Checked here, once per distinct character, rather than in `advance`,
        // where it would cost every character of every run in a debug build.
        debug_assert!(
            self.shared.is_none_or(|(key, _)| {
                key == AsciiAdvanceKey::new(font_size, family, weight, style)
            }),
            "a run's advances are for the style it began with"
        );
        if self.shared.is_some() {
            if let Some(width) =
                rasterizer.family_face_advance(codepoint, font_size, family, weight, style)
            {
                return width;
            }
            // Answered by the fallback chain: this table is no longer one other
            // runs may start from.
            self.shared = None;
        }
        rasterizer.advance_width_for_family(codepoint, font_size, family, weight, style)
    }
}

impl GlyphRasterizer {
    /// The remembered ASCII advances of `key`, or an empty table for it, when
    /// the run announced by [`Self::begin_script_run`] may share them.
    ///
    /// Also the point where a changed font registry is noticed, which is what
    /// empties the remembered tables; the revision they were taken under comes
    /// back with the table.
    fn take_ascii_advances(&mut self, key: AsciiAdvanceKey) -> Option<([f32; 128], u64)> {
        self.refresh_registered_family_faces();
        // With no script requirement and no kana the weight a glyph is addressed
        // at is the style's own, whatever the surrounding text is.
        if !self.script_run.is_empty() || self.run_companion.is_some() {
            return None;
        }
        let table = match self.ascii_advances.iter().position(|(known, _)| *known == key) {
            Some(index) => self.ascii_advances.swap_remove(index).1,
            None => [f32::NAN; 128],
        };
        Some((table, self.registered_font_revision))
    }

    /// Keeps `table` for the next run of `key`'s style, unless the font
    /// registry changed while the run was measuring.
    fn store_ascii_advances(&mut self, key: AsciiAdvanceKey, revision: u64, table: [f32; 128]) {
        if revision != self.registered_font_revision {
            return;
        }
        if self.ascii_advances.len() >= MAX_REMEMBERED_STYLES {
            self.ascii_advances.remove(0);
        }
        self.ascii_advances.push((key, table));
    }

    /// The advance of `codepoint` when the family's own face covers it, else
    /// `None`. Needs the registered faces refreshed, as [`Self::take_ascii_advances`] does.
    fn family_face_advance(
        &mut self,
        codepoint: char,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> Option<f32> {
        #[cfg(test)]
        super::KEY_LOOKUPS.with(|count| count.set(count.get() + 1));
        let key = self.family_face_glyph_key(codepoint, font_size, family, weight, style)?;
        Some(self.advance_for_key(key, font_size))
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
    use crate::glyph_rasterizer::key_lookups;

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

    const SANS: FontFamily = FontFamily::SANS_SERIF;

    /// The width of `text` measured one character at a time, with no memory at
    /// all: what every memoized answer has to equal.
    fn direct_width(
        rasterizer: &mut GlyphRasterizer,
        text: &str,
        size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> f32 {
        rasterizer.begin_script_run(text, None);
        let width = text
            .chars()
            .map(|c| rasterizer.advance_width_for_family(c, size, family, weight, style))
            .sum();
        rasterizer.end_script_run();
        width
    }

    fn measure(
        rasterizer: &mut GlyphRasterizer,
        text: &str,
        size: f32,
        weight: FontWeight,
        style: FontStyle,
    ) -> f32 {
        rasterizer.measure_text_for_family(text, size, SANS, weight, style, None)
    }

    #[test]
    fn a_later_measure_resolves_no_ascii_character_the_first_one_resolved() {
        let mut rasterizer = GlyphRasterizer::new();
        let first = measure(&mut rasterizer, PROSE, 16.0, FontWeight::Normal, FontStyle::Normal);

        // Different text, same characters, same style: nothing is left to resolve.
        let reversed: String = PROSE.chars().rev().collect();
        let before = key_lookups();
        let second = measure(&mut rasterizer, &reversed, 16.0, FontWeight::Normal, FontStyle::Normal);
        let again = measure(&mut rasterizer, PROSE, 16.0, FontWeight::Normal, FontStyle::Normal);

        assert_eq!(key_lookups() - before, 0, "a known character was resolved again");
        assert_eq!(again, first);
        // Summed in another order, so equal only up to rounding.
        assert!(
            (second - first).abs() < 1e-2,
            "the same characters in another order are the same width: {second} vs {first}"
        );
    }

    #[test]
    fn a_remembered_advance_is_never_for_another_style_or_size() {
        let mut rasterizer = GlyphRasterizer::new();
        let styles = [
            (16.0, FontWeight::Normal, FontStyle::Normal),
            (16.0, FontWeight::Bold, FontStyle::Normal),
            (16.0, FontWeight::Normal, FontStyle::Italic),
            (17.5, FontWeight::Normal, FontStyle::Normal),
        ];
        // Interleaved on purpose: a table keyed too loosely answers the second
        // pass with the first style's widths.
        for _ in 0..2 {
            for (size, weight, style) in styles {
                let measured = measure(&mut rasterizer, PROSE, size, weight, style);
                let direct = direct_width(&mut rasterizer, PROSE, size, SANS, weight, style);
                assert_eq!(measured, direct, "{size} {weight:?} {style:?}");
            }
        }
        // And they are different styles, so the test above is not vacuous.
        let regular = measure(&mut rasterizer, PROSE, 16.0, FontWeight::Normal, FontStyle::Normal);
        let larger = measure(&mut rasterizer, PROSE, 17.5, FontWeight::Normal, FontStyle::Normal);
        assert!(larger > regular);
    }

    #[test]
    fn every_style_in_use_keeps_its_own_memory() {
        let mut rasterizer = GlyphRasterizer::new();
        let styles = [
            (FontWeight::Normal, FontStyle::Normal),
            (FontWeight::Bold, FontStyle::Normal),
            (FontWeight::Normal, FontStyle::Italic),
        ];
        for (weight, style) in styles {
            measure(&mut rasterizer, PROSE, 16.0, weight, style);
        }
        let before = key_lookups();
        for (weight, style) in styles {
            measure(&mut rasterizer, PROSE, 16.0, weight, style);
        }
        assert_eq!(key_lookups() - before, 0, "alternating styles evicted each other");
    }

    #[test]
    fn a_registered_font_makes_the_rasterizer_resolve_again() {
        let mut rasterizer = GlyphRasterizer::new();
        let before_registration =
            measure(&mut rasterizer, PROSE, 16.0, FontWeight::Normal, FontStyle::Normal);

        rasterizer
            .register_font_bytes(super::super::PRIMARY_FONT.to_vec())
            .expect("the bundled primary face is a valid font");

        let before = key_lookups();
        let after_registration =
            measure(&mut rasterizer, PROSE, 16.0, FontWeight::Normal, FontStyle::Normal);
        assert!(
            key_lookups() > before,
            "a registration changes what faces exist; remembered answers must not survive it"
        );
        let direct = direct_width(
            &mut rasterizer,
            PROSE,
            16.0,
            SANS,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        assert_eq!(after_registration, direct);
        assert_eq!(after_registration, before_registration);
    }

    #[test]
    fn only_a_bounded_number_of_styles_are_remembered() {
        let mut rasterizer = GlyphRasterizer::new();
        for step in 0..(3 * MAX_REMEMBERED_STYLES) {
            let size = 10.0 + step as f32;
            let measured = measure(&mut rasterizer, PROSE, size, FontWeight::Normal, FontStyle::Normal);
            let direct =
                direct_width(&mut rasterizer, PROSE, size, SANS, FontWeight::Normal, FontStyle::Normal);
            assert_eq!(measured, direct);
            assert!(rasterizer.ascii_advances.len() <= MAX_REMEMBERED_STYLES);
        }
        assert_eq!(rasterizer.ascii_advances.len(), MAX_REMEMBERED_STYLES);
    }

    #[test]
    fn a_run_with_kana_neither_reads_nor_poisons_the_memory() {
        let mut rasterizer = GlyphRasterizer::new();
        let plain = measure(&mut rasterizer, "abc", 16.0, FontWeight::Bold, FontStyle::Normal);

        // Kana name a companion face, which can change the weight an ASCII
        // letter is addressed at; this run must resolve for itself.
        let before = key_lookups();
        let mixed = "abc あ";
        let measured = measure(&mut rasterizer, mixed, 16.0, FontWeight::Bold, FontStyle::Normal);
        assert!(key_lookups() - before >= 4, "a, b, c and the kana are resolved in the run");
        let direct = direct_width(
            &mut rasterizer,
            mixed,
            16.0,
            SANS,
            FontWeight::Bold,
            FontStyle::Normal,
        );
        assert_eq!(measured, direct);

        // Plain text afterwards still gets exactly what it got before.
        let again = measure(&mut rasterizer, "abc", 16.0, FontWeight::Bold, FontStyle::Normal);
        assert_eq!(again, plain);
    }
}
