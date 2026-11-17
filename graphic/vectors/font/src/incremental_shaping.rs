//! Shaping a text again after an edit, from the shaping of the text before it.
//!
//! A hard line (the text between two line feeds) is shaped on its own: what its
//! clusters are, and where a line may break inside it, depend on nothing outside
//! it. So when one character of a long note changes, the lines that did not change
//! keep the clusters they had, and only the lines around the edit need the shaper,
//! which is where nearly all of the time of shaping a note goes.

use crate::font::{FontFamily, FontStyle, FontWeight, TextLanguage};
use crate::glyph_rasterizer::GlyphRasterizer;
use crate::text_layout::{
    ShapedText, TextWritingMode, is_simple_ltr_text, shape_simple_ltr_text, simple_ltr_font,
};

/// Shapes `text` as [`shape_text_styled`](crate::text_layout::shape_text_styled)
/// would, reusing `previous`, the shaping of an earlier version of the text made
/// with the same `font_size`, `font_family`, `font_weight`, `font_style` and
/// `language`.
///
/// Only the hard lines that differ from `previous` are shaped; the clusters of the
/// others are moved, and the suffix's byte ranges shifted by how much the text
/// grew or shrank. The result is the one a full shaping gives.
///
/// Applies to the text the simple left-to-right path shapes, which is where the
/// lines are independent: printable ASCII and line feeds in the generic family.
/// For anything else, or when `previous` is not such a text, it gives `previous`
/// back untouched and the caller shapes in full.
#[allow(clippy::too_many_arguments)]
pub fn shape_text_styled_after_edit(
    rasterizer: &mut GlyphRasterizer,
    previous: ShapedText,
    text: &str,
    font_size: f32,
    font_family: FontFamily,
    font_weight: FontWeight,
    font_style: FontStyle,
    language: Option<TextLanguage>,
) -> Result<ShapedText, ShapedText> {
    let Some(font_id) = simple_ltr_font(rasterizer, text, font_family) else {
        return Err(previous);
    };
    // The previous clusters are one per byte, and each line apart from the others,
    // only if it was shaped by the same path.
    if previous.writing_mode != TextWritingMode::HorizontalTb
        || previous.font_size != font_size
        || previous.clusters.len() != previous.text.len()
        || !is_simple_ltr_text(&previous.text)
    {
        return Err(previous);
    }
    let (ascent, descent, line_gap) =
        rasterizer.line_metrics_for_family(font_size, font_family, font_weight, font_style);
    if (ascent, descent, line_gap) != (previous.ascent, previous.descent, previous.line_gap) {
        return Err(previous);
    }
    let _ = language; // ASCII reads no script requirement, whatever the language.

    let (start, old_end, new_end) = changed_lines(previous.text.as_bytes(), text.as_bytes());
    let delta = new_end as isize - old_end as isize;

    // The middle is whole lines of the new text, so it is shaped as the text of its
    // own: what it asks of the rasterizer is the same.
    rasterizer.begin_script_run(&text[start..new_end], None);
    let middle = shape_simple_ltr_text(
        rasterizer,
        &text[start..new_end],
        font_size,
        font_id,
        font_weight,
        ascent,
        descent,
        line_gap,
    );
    rasterizer.end_script_run();
    let mut middle = middle.clusters;
    for cluster in &mut middle {
        cluster.text_range = cluster.text_range.start + start..cluster.text_range.end + start;
    }
    // Shaped alone, the first cluster of the middle has nothing before it; in the
    // whole text a line that follows a line feed may be started at.
    if start > 0
        && let Some(first) = middle.first_mut()
        && first.base_codepoint != '\n'
    {
        first.can_break_before = true;
    }

    let middle_len = middle.len();
    let mut clusters = previous.clusters;
    clusters.splice(start..old_end, middle);
    for cluster in &mut clusters[start + middle_len..] {
        cluster.text_range = shifted(cluster.text_range.start, delta)..shifted(cluster.text_range.end, delta);
    }

    // The widest line, accumulated as full shaping does: cluster by cluster, reset at
    // every line feed.
    let mut max_line_width = 0.0_f32;
    let mut line_width = 0.0_f32;
    for cluster in &clusters {
        if cluster.base_codepoint == '\n' {
            line_width = 0.0;
        } else {
            line_width += cluster.width;
            max_line_width = max_line_width.max(line_width);
        }
    }

    Ok(ShapedText {
        text: text.to_string(),
        font_size,
        ascent,
        descent,
        line_gap,
        line_height: ascent - descent + line_gap,
        max_line_width,
        writing_mode: TextWritingMode::HorizontalTb,
        clusters,
    })
}

/// How many bytes `old` and `new` share at their two ends, which is what
/// [`shape_text_styled_after_edit`] gets to keep: the more of `new`, the better
/// `old` is to start from.
pub fn shared_edge_bytes(old: &str, new: &str) -> usize {
    let prefix = common_prefix(old.as_bytes(), new.as_bytes());
    prefix + common_suffix(&old.as_bytes()[prefix..], &new.as_bytes()[prefix..])
}

#[inline]
fn shifted(offset: usize, delta: isize) -> usize {
    offset.checked_add_signed(delta).expect("a shifted offset stays inside the text")
}

/// The whole lines that differ between `old` and `new`: where they start, where
/// they end in `old`, and where they end in `new`. Everything before the start and
/// after the ends is the same in both, and the start and the ends are at the
/// beginnings of lines (or the end of the text).
fn changed_lines(old: &[u8], new: &[u8]) -> (usize, usize, usize) {
    let prefix = common_prefix(old, new);
    // Back to the start of the line the first difference is on.
    let start = new[..prefix].iter().rposition(|&byte| byte == b'\n').map_or(0, |at| at + 1);
    let suffix = common_suffix(&old[start..], &new[start..]);
    // Forward to the start of the next line, so what follows is whole lines.
    let suffix_start = new.len() - suffix;
    let end_new = new[suffix_start..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(new.len(), |at| suffix_start + at + 1);
    (start, old.len() - (new.len() - end_new), end_new)
}

/// How many bytes `a` and `b` start with in common.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    // Blocks first: comparing slices is one memcmp, where a byte loop is slow
    // enough in a debug build to be felt on a note of several hundred kilobytes.
    const BLOCK: usize = 256;
    let limit = a.len().min(b.len());
    let mut at = 0;
    while at + BLOCK <= limit && a[at..at + BLOCK] == b[at..at + BLOCK] {
        at += BLOCK;
    }
    while at < limit && a[at] == b[at] {
        at += 1;
    }
    at
}

/// How many bytes `a` and `b` end with in common.
fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 256;
    let limit = a.len().min(b.len());
    let mut len = 0;
    while len + BLOCK <= limit && a[a.len() - len - BLOCK..a.len() - len] == b[b.len() - len - BLOCK..b.len() - len] {
        len += BLOCK;
    }
    while len < limit && a[a.len() - 1 - len] == b[b.len() - 1 - len] {
        len += 1;
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_layout::shape_text_styled;

    const WEIGHT: FontWeight = FontWeight::Normal;

    fn full(rasterizer: &mut GlyphRasterizer, text: &str) -> ShapedText {
        shape_text_styled(
            rasterizer,
            text,
            16.0,
            FontFamily::SANS_SERIF,
            WEIGHT,
            FontStyle::Normal,
            None,
        )
    }

    fn after_edit(
        rasterizer: &mut GlyphRasterizer,
        previous: ShapedText,
        text: &str,
    ) -> Result<ShapedText, ShapedText> {
        shape_text_styled_after_edit(
            rasterizer,
            previous,
            text,
            16.0,
            FontFamily::SANS_SERIF,
            WEIGHT,
            FontStyle::Normal,
            None,
        )
    }

    /// Everything a full shaping gives, compared field by field.
    fn assert_same(spliced: &ShapedText, expected: &ShapedText, what: &str) {
        assert_eq!(spliced.text, expected.text, "{what}: text");
        assert_eq!(spliced.font_size, expected.font_size, "{what}: font size");
        assert_eq!(spliced.ascent, expected.ascent, "{what}: ascent");
        assert_eq!(spliced.descent, expected.descent, "{what}: descent");
        assert_eq!(spliced.line_gap, expected.line_gap, "{what}: line gap");
        assert_eq!(spliced.line_height, expected.line_height, "{what}: line height");
        assert_eq!(spliced.max_line_width, expected.max_line_width, "{what}: widest line");
        assert_eq!(spliced.writing_mode, expected.writing_mode, "{what}: writing mode");
        assert_eq!(spliced.clusters.len(), expected.clusters.len(), "{what}: cluster count");
        for (index, (got, want)) in spliced.clusters.iter().zip(&expected.clusters).enumerate() {
            assert_eq!(got.text, want.text, "{what}: cluster {index} text");
            assert_eq!(got.text_range, want.text_range, "{what}: cluster {index} range");
            assert_eq!(got.level, want.level, "{what}: cluster {index} level");
            assert_eq!(got.base_codepoint, want.base_codepoint, "{what}: cluster {index} char");
            assert_eq!(got.glyphs, want.glyphs, "{what}: cluster {index} glyphs");
            assert_eq!(got.width, want.width, "{what}: cluster {index} width");
            assert_eq!(
                got.can_break_before, want.can_break_before,
                "{what}: cluster {index} break opportunity"
            );
        }
    }

    fn note() -> String {
        (0..30)
            .map(|index| match index % 5 {
                0 => format!("## Heading number {index}\n"),
                1 => format!("A paragraph of prose, line {index}, with enough words to be a line.\n"),
                2 => "\n".to_owned(),
                3 => format!("- item {index} with AV To kerning pairs\n"),
                _ => format!("last of the group {index}"),
            })
            .collect::<String>()
            + "\nthe end"
    }

    /// The edits of a text area: typing, deleting, line breaks made and joined,
    /// and the first and last lines, which have a neighbour on one side only.
    fn edits() -> Vec<(&'static str, String)> {
        let old = note();
        let middle = old.len() / 2;
        let line_end = old[middle..].find('\n').map(|at| middle + at).expect("a line follows");
        let mut cases = Vec::new();
        let mut typed = old.clone();
        typed.insert(middle, 'x');
        cases.push(("a character typed mid-line", typed));
        let mut deleted = old.clone();
        deleted.remove(middle);
        cases.push(("a character deleted", deleted));
        let mut split = old.clone();
        split.insert(middle, '\n');
        cases.push(("a line broken in two", split));
        let mut joined = old.clone();
        joined.remove(line_end);
        cases.push(("two lines joined", joined));
        cases.push(("a character at the very start", format!("x{old}")));
        cases.push(("a character at the very end", format!("{old}x")));
        cases.push(("the last character removed", old[..old.len() - 1].to_owned()));
        cases.push(("a new line at the end", format!("{old}\n")));
        cases.push(("a blank line at the start", format!("\n{old}")));
        let mut several = old.clone();
        several.replace_range(middle..middle + 150, "replaced\n\nacross\nlines");
        cases.push(("several lines replaced", several));
        let mut same_length = old.clone();
        same_length.replace_range(middle..middle + 1, "#");
        cases.push(("a character replaced", same_length));
        cases.push(("nothing changed", old.clone()));
        cases.push(("everything replaced", "something else entirely\nover two lines".to_owned()));
        cases.push(("emptied", String::new()));
        cases
    }

    #[test]
    fn splicing_gives_what_a_full_shaping_gives() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let old = note();
        for (what, new) in edits() {
            let previous = full(&mut rasterizer, &old);
            let expected = full(&mut rasterizer, &new);
            let spliced = after_edit(&mut rasterizer, previous, &new)
                .unwrap_or_else(|_| panic!("{what}: the edit was not spliced"));
            assert_same(&spliced, &expected, what);
        }
    }

    #[test]
    fn splicing_from_an_empty_text_gives_what_a_full_shaping_gives() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let new = note();
        let previous = full(&mut rasterizer, "");
        let expected = full(&mut rasterizer, &new);
        let spliced = after_edit(&mut rasterizer, previous, &new).ok().expect("spliced");
        assert_same(&spliced, &expected, "from empty");
    }

    #[test]
    fn only_the_lines_around_the_edit_are_shaped() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let old = note();
        let mut new = old.clone();
        new.insert(old.len() / 2, 'x');
        let previous = full(&mut rasterizer, &old);
        let lines = old.matches('\n').count() + 1;

        rasterizer.reset_shape_call_count();
        let _ = full(&mut rasterizer, &new);
        let full_calls = rasterizer.shape_call_count();
        assert!(full_calls >= lines - 8, "a full shaping shapes every line: {full_calls}");

        rasterizer.reset_shape_call_count();
        after_edit(&mut rasterizer, previous, &new).ok().expect("spliced");
        assert_eq!(rasterizer.shape_call_count(), 1, "one line changed");
    }

    #[test]
    fn text_the_simple_path_does_not_shape_is_declined() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let old = note();
        let previous = full(&mut rasterizer, &old);
        let accented = format!("{old} caf\u{e9}");
        let previous = after_edit(&mut rasterizer, previous, &accented)
            .err()
            .expect("an accent leaves the simple path");
        assert_eq!(previous.text, old, "the previous shaping is handed back untouched");

        // The reverse: a previous text that was not simple.
        let previous = full(&mut rasterizer, &accented);
        assert!(after_edit(&mut rasterizer, previous, &old).is_err());
    }
}
