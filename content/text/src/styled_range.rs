use std::ops::Range;
use std::rc::Rc;

use aimer_style::TextStyle;
use unicode_segmentation::GraphemeCursor;

use crate::TextSource;
use crate::text_span::{ResolvedTextSpan, SpanStyle};

/// A byte range of a string together with the style it is drawn with.
///
/// Ranges let one run of text carry several looks without splitting it into
/// separate widgets: a code editor colours its tokens, a chat composer
/// highlights `@mentions`. The text itself is not changed, so editing, selection
/// and input-method behaviour keep treating it as one string.
///
/// `range` is a UTF-8 byte range into the string it styles. Ranges that are
/// out of bounds, empty, unsorted or overlapping are repaired rather than
/// rejected — see [`RawTextWidget`](crate::RawTextWidget) — so a highlighter
/// that is a little sloppy at the edges of the text still draws.
///
/// # Examples
///
/// ```
/// use aimer_text::{SpanStyle, StyledRange};
/// use aimer_widget::base::Color;
///
/// // Paints the first word of "let x = 1" as a keyword.
/// let keyword = StyledRange::new(0..3, SpanStyle::new().color(Color::Rgba(80, 120, 255, 255)));
/// assert_eq!(keyword.range, 0..3);
/// ```
#[derive(Clone, Debug)]
pub struct StyledRange {
    /// The styled UTF-8 byte range.
    pub range: Range<usize>,
    /// The overrides applied on top of the text's base style.
    pub style: SpanStyle,
}

impl StyledRange {
    /// Creates a styled range.
    #[inline]
    pub fn new(range: Range<usize>, style: SpanStyle) -> Self {
        Self { range, style }
    }
}

/// Moves `position` to the nearest grapheme boundary at or after it, clamped to
/// the text. A span edge inside a cluster would otherwise split a base letter
/// from its combining mark and shape the two halves separately.
fn snap_to_grapheme(text: &str, position: usize) -> usize {
    let len = text.len();
    if position >= len {
        return len;
    }
    if position == 0 {
        return 0;
    }
    // The cursor below requires a char boundary to start from.
    let mut position = position;
    while !text.is_char_boundary(position) {
        position += 1;
    }
    let bytes = text.as_bytes();
    // Two ASCII neighbours are a boundary unless they are the CR LF pair.
    if bytes[position - 1] < 0x80
        && bytes[position] < 0x80
        && !(bytes[position - 1] == b'\r' && bytes[position] == b'\n')
    {
        return position;
    }
    let mut cursor = GraphemeCursor::new(position, len, true);
    match cursor.is_boundary(text, 0) {
        Ok(true) => position,
        _ => match cursor.next_boundary(text, 0) {
            Ok(Some(next)) => next,
            _ => len,
        },
    }
}

/// Splits `text` into resolved spans: one per styled range, with the gaps
/// between them in `base`.
///
/// The result always covers `text` exactly once and in order, so laying it out
/// shapes the same characters a single run would. Edges are snapped to grapheme
/// boundaries, ranges are clamped to the text, empty ones are dropped, and where
/// ranges overlap the earlier-starting one keeps the shared bytes.
pub(crate) fn resolve_ranges(
    text: &TextSource,
    base: TextStyle,
    ranges: &[StyledRange],
) -> Vec<ResolvedTextSpan> {
    let source = text.as_str();
    let mut edges: Vec<(usize, usize, &SpanStyle)> = ranges
        .iter()
        .filter_map(|styled| {
            let start = snap_to_grapheme(source, styled.range.start);
            let end = snap_to_grapheme(source, styled.range.end);
            (start < end).then_some((start, end, &styled.style))
        })
        .collect();
    if edges.is_empty() {
        return vec![ResolvedTextSpan::plain_source(text.clone(), base)];
    }
    // Highlighters normally emit ranges in order, which makes this a no-op scan.
    if !edges.is_sorted_by_key(|edge| edge.0) {
        edges.sort_by_key(|edge| edge.0);
    }

    let mut spans = Vec::with_capacity(edges.len() * 2 + 1);
    let mut cursor = 0;
    for (start, end, style) in edges {
        // A later range never repaints bytes an earlier one already styled.
        let start = start.max(cursor);
        if end <= start {
            continue;
        }
        if start > cursor {
            spans.push(plain_slice(source, cursor..start, base));
        }
        spans.push(plain_slice(source, start..end, style.resolve(base)));
        cursor = end;
    }
    if cursor < source.len() {
        spans.push(plain_slice(source, cursor..source.len(), base));
    }
    spans
}

fn plain_slice(source: &str, range: Range<usize>, style: TextStyle) -> ResolvedTextSpan {
    ResolvedTextSpan::plain_source(TextSource::Shared(Rc::from(&source[range])), style)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(text: &str) -> TextSource {
        TextSource::Shared(Rc::from(text))
    }

    fn color(red: u8) -> SpanStyle {
        SpanStyle::new().color(aimer_widget::base::Color::Rgba(red, 0, 0, 255))
    }

    fn base() -> TextStyle {
        TextStyle::default()
    }

    /// The spans' texts joined, which must always be the original text.
    fn joined(spans: &[ResolvedTextSpan]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    fn pieces(spans: &[ResolvedTextSpan]) -> Vec<&str> {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// The red channel of a span's colour, or `None` when it is the base colour.
    fn red_of(span: &ResolvedTextSpan) -> Option<u8> {
        let rgba = span.style.color.to_rgba();
        (rgba != base().color.to_rgba()).then_some(rgba.0)
    }

    fn resolve(text: &str, ranges: &[StyledRange]) -> Vec<ResolvedTextSpan> {
        resolve_ranges(&shared(text), base(), ranges)
    }

    #[test]
    fn no_ranges_is_one_plain_span() {
        let spans = resolve("let x = 1", &[]);

        assert_eq!(pieces(&spans), ["let x = 1"]);
    }

    #[test]
    fn empty_text_still_yields_one_span() {
        let spans = resolve("", &[StyledRange::new(0..3, color(1))]);

        assert_eq!(spans.len(), 1);
        assert_eq!(joined(&spans), "");
    }

    #[test]
    fn a_range_splits_the_text_around_it() {
        let spans = resolve("let x = 1", &[StyledRange::new(0..3, color(1))]);

        assert_eq!(pieces(&spans), ["let", " x = 1"]);
        assert_eq!(red_of(&spans[0]), Some(1));
        assert_eq!(red_of(&spans[1]), None);
    }

    #[test]
    fn gaps_between_ranges_keep_the_base_style() {
        let spans = resolve(
            "let x = 1",
            &[
                StyledRange::new(0..3, color(1)),
                StyledRange::new(8..9, color(2)),
            ],
        );

        assert_eq!(pieces(&spans), ["let", " x = ", "1"]);
        assert_eq!(red_of(&spans[2]), Some(2));
        assert_eq!(red_of(&spans[1]), None);
    }

    #[test]
    fn a_range_covering_everything_is_one_span() {
        let spans = resolve("abc", &[StyledRange::new(0..3, color(7))]);

        assert_eq!(pieces(&spans), ["abc"]);
        assert_eq!(red_of(&spans[0]), Some(7));
    }

    #[test]
    fn unsorted_ranges_are_ordered() {
        let spans = resolve(
            "ab cd",
            &[
                StyledRange::new(3..5, color(2)),
                StyledRange::new(0..2, color(1)),
            ],
        );

        assert_eq!(pieces(&spans), ["ab", " ", "cd"]);
        assert_eq!(red_of(&spans[0]), Some(1));
        assert_eq!(red_of(&spans[2]), Some(2));
    }

    #[test]
    fn overlap_keeps_the_earlier_range_and_trims_the_later() {
        let spans = resolve(
            "abcdef",
            &[
                StyledRange::new(0..4, color(1)),
                StyledRange::new(2..6, color(2)),
            ],
        );

        assert_eq!(pieces(&spans), ["abcd", "ef"]);
        assert_eq!(red_of(&spans[0]), Some(1));
        assert_eq!(red_of(&spans[1]), Some(2));
    }

    #[test]
    fn a_range_nested_in_another_is_dropped() {
        let spans = resolve(
            "abcdef",
            &[
                StyledRange::new(0..6, color(1)),
                StyledRange::new(2..3, color(2)),
            ],
        );

        assert_eq!(pieces(&spans), ["abcdef"]);
        assert_eq!(red_of(&spans[0]), Some(1));
    }

    #[test]
    fn empty_and_inverted_ranges_are_ignored() {
        #[allow(clippy::reversed_empty_ranges)]
        let inverted = 4..2;
        let spans = resolve(
            "abcdef",
            &[
                StyledRange::new(3..3, color(1)),
                StyledRange::new(inverted, color(2)),
            ],
        );

        assert_eq!(pieces(&spans), ["abcdef"]);
        assert_eq!(red_of(&spans[0]), None);
    }

    #[test]
    fn out_of_bounds_ranges_are_clamped() {
        let spans = resolve(
            "abc",
            &[
                StyledRange::new(1..99, color(1)),
                StyledRange::new(50..60, color(2)),
            ],
        );

        assert_eq!(pieces(&spans), ["a", "bc"]);
        assert_eq!(red_of(&spans[1]), Some(1));
    }

    #[test]
    fn edges_inside_a_multibyte_char_move_to_the_next_boundary() {
        // `é` is two bytes: 1..3.
        let spans = resolve("aéb", &[StyledRange::new(0..2, color(1))]);

        assert_eq!(joined(&spans), "aéb");
        assert_eq!(pieces(&spans), ["aé", "b"]);
    }

    #[test]
    fn edges_inside_a_grapheme_cluster_move_to_its_end() {
        // `e` + combining acute accent is one cluster of three bytes.
        let text = "e\u{301}x";
        let spans = resolve(text, &[StyledRange::new(0..1, color(1))]);

        assert_eq!(joined(&spans), text);
        assert_eq!(pieces(&spans), ["e\u{301}", "x"]);
    }

    #[test]
    fn a_crlf_pair_is_never_split() {
        let spans = resolve("a\r\nb", &[StyledRange::new(0..2, color(1))]);

        assert_eq!(joined(&spans), "a\r\nb");
        assert_eq!(pieces(&spans), ["a\r\n", "b"]);
    }

    #[test]
    fn the_spans_always_cover_the_text_exactly() {
        let text = "fn main() { println!(\"héllo 👨‍👩‍👧\"); }\n";
        let ranges = (0..text.len())
            .step_by(3)
            .map(|start| StyledRange::new(start..start + 5, color(start as u8)))
            .collect::<Vec<_>>();

        let spans = resolve(text, &ranges);

        assert_eq!(joined(&spans), text);
        assert!(spans.iter().all(|span| !span.text.is_empty()));
    }
}
