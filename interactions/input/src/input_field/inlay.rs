//! Notes drawn inside a line of text, that are not part of it: the type of `x` in `let x = 1;`, which a language server knows and
//! the text does not say.
//!
//! An [`Inlay`] is a label that goes before the byte `offset` of the text. The text is not touched, so editing, undo, copy and
//! every offset the field gives are those of the text alone. What the field does is leave room: the label is as wide as the
//! space that is left after the character before `offset`, and is drawn in it.
//!
//! The room is left by the one thing that text layout already does for a gap: the letter spacing of a span.
//! So the gap is a span of the character before `offset`, with the label's width as its letter spacing. That is why an inlay
//! cannot be at the start of a line, where there is no character before it on the line. One at the end of a line needs no room,
//! as nothing follows it there: it is drawn after the last character, in the free space of the line.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aimer_cupid::font::FontFamily;
use aimer_text::{SpanStyle, StyledRange};
use aimer_text_buffer::TextSnapshot;
use aimer_widget::base::Color;

/// Source of the identities that tell two sets of inlays apart. Zero is reserved for "none".
static NEXT_INLAYS_ID: AtomicU64 = AtomicU64::new(1);

/// One label that goes in a line of text. See [the module](self).
///
/// # Examples
///
/// ```
/// use aimer_input::input::Inlay;
/// use aimer_widget::base::Color;
///
/// // The type of `x` in `let x = 1;`: after the `x`, which is the byte 5, in a pill of its own color.
/// let hint = Inlay::new(5, ": i32", Color::Rgba(190, 150, 170, 255))
///     .background(Color::Rgba(110, 70, 90, 255))
///     .padded(true, false);
/// assert_eq!(hint.offset(), 5);
/// assert!(hint.has_background());
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Inlay {
    offset: usize,
    label: Arc<str>,
    color: Color,
    background: Option<Color>,
    pad_left: bool,
    pad_right: bool,
}

impl Inlay {
    /// A label that goes before the byte `offset` of the text.
    pub fn new(offset: usize, label: impl Into<Arc<str>>, color: Color) -> Self {
        Self { offset, label: label.into(), color, background: None, pad_left: false, pad_right: false }
    }

    /// Whether the label is a little apart from the text before it and from the text after it.
    #[must_use]
    pub fn padded(mut self, left: bool, right: bool) -> Self {
        self.pad_left = left;
        self.pad_right = right;
        self
    }

    /// Draws the label in a pill of this color, rounded as [`Inlays::radius`] says, with [`Inlays::inset`] of room inside it on
    /// each side of the text.
    #[must_use]
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Whether the label is drawn in a pill.
    pub fn has_background(&self) -> bool {
        self.background.is_some()
    }

    /// The byte of the text the label goes before.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// What the label says.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The color of the label's text.
    pub fn color(&self) -> Color {
        self.color
    }

    /// The color of the pill the label is drawn in, if it has one.
    pub fn pill(&self) -> Option<Color> {
        self.background
    }

    /// Whether the label is a little apart from the text before it.
    pub fn pad_left(&self) -> bool {
        self.pad_left
    }

    /// Whether the label is a little apart from the text after it.
    pub fn pad_right(&self) -> bool {
        self.pad_right
    }
}

/// The labels of a text, in the order of the text, as the field takes them. A set is known by the value it was made as: make
/// one when the labels change, and keep it while they do not, so that the field does not lay its text out again for nothing.
#[derive(Clone, Debug)]
pub struct Inlays {
    id: u64,
    items: Rc<[Inlay]>,
    size: f32,
    family: Option<FontFamily>,
    pad: f32,
    radius: f32,
    inset: f32,
    text: Option<TextSnapshot>,
}

impl Inlays {
    /// How large the text of a label is, as a share of the field's own, unless it is said.
    pub const DEFAULT_SIZE: f32 = 0.85;
    /// How far a padded label is from the text, in logical pixels, on each side that is padded, unless it is said.
    pub const DEFAULT_PAD: f32 = 4.0;
    /// How round the corners of a pill are, in logical pixels, unless it is said.
    pub const DEFAULT_RADIUS: f32 = 4.0;
    /// How much room there is between the edge of a pill and its text, on each side, in logical pixels, unless it is said.
    pub const DEFAULT_INSET: f32 = 5.0;

    /// A set of labels. Those at the same byte keep the order they were given in.
    pub fn new(mut items: Vec<Inlay>) -> Self {
        items.sort_by_key(Inlay::offset);
        Self {
            id: NEXT_INLAYS_ID.fetch_add(1, Ordering::Relaxed),
            items: items.into(),
            size: Self::DEFAULT_SIZE,
            family: None,
            pad: Self::DEFAULT_PAD,
            radius: Self::DEFAULT_RADIUS,
            inset: Self::DEFAULT_INSET,
            text: None,
        }
    }

    /// How large the text of a label is, as a share of the field's own.
    #[must_use]
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self.id = NEXT_INLAYS_ID.fetch_add(1, Ordering::Relaxed);
        self
    }

    /// How round the corners of the pills are, in logical pixels. A radius larger than half the height of a pill is half of it.
    #[must_use]
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius.max(0.0);
        self.id = NEXT_INLAYS_ID.fetch_add(1, Ordering::Relaxed);
        self
    }

    /// How much room there is between the edge of a pill and its text, on each side, in logical pixels.
    #[must_use]
    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset.max(0.0);
        self.id = NEXT_INLAYS_ID.fetch_add(1, Ordering::Relaxed);
        self
    }

    /// The font of the labels, which is the field's own unless it is said.
    #[must_use]
    pub fn family(mut self, family: FontFamily) -> Self {
        self.family = Some(family);
        self.id = NEXT_INLAYS_ID.fetch_add(1, Ordering::Relaxed);
        self
    }

    /// Says which text the offsets of the labels are of. A field whose text is another one, because it was edited since, moves each
    /// label by the edit that makes one into the other: those after it by as much as it added or took away, and those that were
    /// inside what it replaced are left out. Without it the offsets are taken as they are of the text the field has.
    ///
    /// This is what keeps notes in their places while their text is typed in and the server has not said them again.
    #[must_use]
    pub fn of_text(mut self, text: TextSnapshot) -> Self {
        self.text = Some(text);
        self.id = NEXT_INLAYS_ID.fetch_add(1, Ordering::Relaxed);
        self
    }

    /// The labels, in the order of the text.
    pub fn items(&self) -> &[Inlay] {
        &self.items
    }

    /// How many labels there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// Whether `other` is this very set, or a copy of it: not one that was made from the same labels, which the field would lay its
    /// text out again for.
    pub fn same_as(&self, other: &Self) -> bool {
        self.id == other.id
    }

    /// The text the offsets are of, if it was said; see [`of_text`](Self::of_text).
    pub fn text(&self) -> Option<&TextSnapshot> {
        self.text.as_ref()
    }

    pub(crate) fn text_size(&self) -> f32 {
        self.size
    }

    pub(crate) fn text_family(&self) -> Option<FontFamily> {
        self.family
    }

    pub(crate) fn pad(&self) -> f32 {
        self.pad
    }

    pub(crate) fn corner_radius(&self) -> f32 {
        self.radius
    }

    pub(crate) fn text_inset(&self) -> f32 {
        self.inset
    }
}

/// Where the labels of one offset go, as the layout is to be told and the painter is to find them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Placed {
    /// The byte the labels go before, in the whole text.
    pub at: usize,
    /// The bytes of the character the gap is spread after (or for a space, the space itself). For labels at the end of
    /// a line, only the one before.
    pub span: Range<usize>,
    /// How wide the room is for all of the labels at `at`, in logical pixels.
    pub gap: f32,
    /// How wide the room of each of the labels is, in the order of `items`: they add up to `gap`.
    pub widths: Vec<f32>,
    /// Whether the labels are at the end of a line, where there is nothing after them to be moved, so that no room is left for them
    /// in the layout: they are drawn after the last character, and the line is as wide as it was.
    pub eol: bool,
    /// Whether the room is the word spacing of the space that follows `at`, and not the letter spacing between the character before it
    /// and the one after it: `span` is then the space alone. The character before is not in what is spread, so what is drawn under it,
    /// such as the underline of a name, ends where it does and does not go on under the room.
    pub word: bool,
    /// Which of the set's labels they are.
    pub items: Range<usize>,
}

/// Finds where the labels of `inlays` go in `text`, which is the bytes `base..` of the whole text, and how much room each place
/// needs, `measure` saying how wide a label is, in logical pixels. `edit` is what the whole text was changed by since the one the
/// offsets of the labels are of: the bytes that were replaced, and the bytes that replaced them. See [`Inlays::of_text`].
///
/// A label that cannot be placed is left out: one that is outside `text`, in the middle of a character, at the start of a line or
/// at the end of the text, or whose room would be in the room of the one before. The result is in the order of the text, and what
/// is in it does not overlap.
pub(crate) fn place(
    inlays: &Inlays,
    text: &str,
    base: usize,
    edit: Option<&(Range<usize>, Range<usize>)>,
    measure: &mut dyn FnMut(&Inlay) -> f32,
) -> Vec<Placed> {
    let items = inlays.items();
    // Without an edit the labels are where they say, and the ones of this text are found by bisection. With one they are
    // moved first, which keeps their order, and are all looked at.
    let (first, last) = match edit {
        None => (
            items.partition_point(|inlay| inlay.offset < base),
            items.partition_point(|inlay| inlay.offset <= base + text.len()),
        ),
        Some(_) => (0, items.len()),
    };
    let mut found: Vec<Placed> = Vec::new();
    let mut next = first;
    while next < last {
        let Some(at) = moved(items[next].offset, edit) else {
            next += 1;
            continue;
        };
        let group_start = next;
        next += 1;
        // The labels of one byte that follow each other: one that was left out between them ends the group.
        while next < last && moved(items[next].offset, edit) == Some(at) {
            next += 1;
        }
        if at < base || at > base + text.len() {
            continue;
        }
        let Some((span, spot)) = gap_span(text, at - base) else {
            continue;
        };
        // The room of the one before is not shared: the second is left out.
        if found.last().is_some_and(|before| span.start < before.span.end - base) {
            continue;
        }
        let widths: Vec<f32> = items[group_start..next].iter().map(&mut *measure).collect();
        let gap = widths.iter().sum();
        found.push(Placed { at, span: base + span.start..base + span.end, gap, widths, eol: spot == Spot::End, word: spot == Spot::Space, items: group_start..next });
    }
    found
}

/// Where the byte `offset` of the text before an edit is in the text after it, or none where what it was beside was replaced.
fn moved(offset: usize, edit: Option<&(Range<usize>, Range<usize>)>) -> Option<usize> {
    let Some((old, new)) = edit else {
        return Some(offset);
    };
    if offset >= old.end {
        Some(offset - old.len() + new.len())
    } else if offset > old.start {
        None
    } else {
        Some(offset)
    }
}

/// Where the room of a label is had from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spot {
    /// Between the character before and the one after, which the letter spacing of the two is spread over.
    Between,
    /// In the space that follows, whose word spacing it is: the character before is left out of it.
    Space,
    /// At the end of a line, where there is nothing to be moved and no room is asked for.
    End,
}

/// The bytes the room of a label at `at` is had from, if `at` is between two characters of a line, and where they are: the character
/// before it and the one after, a space after it alone, or at the end of a line the one before alone.
fn gap_span(text: &str, at: usize) -> Option<(Range<usize>, Spot)> {
    use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};
    if at == 0 || at > text.len() || !text.is_char_boundary(at) {
        return None;
    }
    let mut cursor = GraphemeCursor::new(at, text.len(), true);
    if !cursor.is_boundary(text, 0).ok()? {
        return None;
    }
    let before = text[..at].graphemes(true).next_back()?;
    // The start of a line has nothing before it on its line, and a line break is no character to put room after.
    if before.chars().all(|ch| matches!(ch, '\n' | '\r')) {
        return None;
    }
    let start = at - before.len();
    if at == text.len() {
        return Some((start..at, Spot::End));
    }
    let end = cursor.next_boundary(text, 0).ok()??;
    let after = &text[at..end];
    // What follows is a line break: it is not shaped with the text, so room cannot be had from it.
    if after.chars().all(|ch| matches!(ch, '\n' | '\r')) {
        return Some((start..at, Spot::End));
    }
    // A space takes word spacing whether or not it is the last of its span, which letter spacing is not: so the room is put in the
    // space, and the name before it is no part of what is spread, and keeps what is drawn under it to where it ends.
    if after.chars().all(char::is_whitespace) {
        return Some((at..end, Spot::Space));
    }
    Some((start..at, Spot::Between))
}

/// What `ranges` say with the room for each of `placed` in them. `text` is what the ranges are of, and starts at the byte `base`
/// of the whole text.
///
/// The character before `at` takes the style it has and spreads the room after it. The ranges are taken as the
/// text takes them, with the one that starts earlier keeping the bytes that two share. `scale` is how many physical pixels a
/// logical one is: the room is logical, and the letter spacing that makes it is physical.
pub(crate) fn with_gaps(
    ranges: Option<Rc<[StyledRange]>>,
    placed: &[Placed],
    base: usize,
    scale: f32,
) -> Option<Rc<[StyledRange]>> {
    // What is at the end of a line has no room in the layout.
    let placed: Vec<&Placed> = placed.iter().filter(|place| !place.eol).collect();
    if placed.is_empty() {
        return ranges;
    }
    // The ranges as the text takes them: in order, with no byte in two.
    let mut edges: Vec<(usize, usize, &SpanStyle)> = ranges
        .iter()
        .flat_map(|ranges| ranges.iter())
        .filter(|styled| styled.range.start < styled.range.end)
        .map(|styled| (styled.range.start, styled.range.end, &styled.style))
        .collect();
    if !edges.is_sorted_by_key(|edge| edge.0) {
        edges.sort_by_key(|edge| edge.0);
    }
    let mut ranges: Vec<(usize, usize, &SpanStyle)> = Vec::with_capacity(edges.len());
    let mut cursor = 0;
    for (start, end, style) in edges {
        let start = start.max(cursor);
        if end > start {
            ranges.push((start, end, style));
            cursor = end;
        }
    }
    let gaps: Vec<(Range<usize>, f32, bool)> = placed
        .into_iter()
        .map(|place| (place.span.start - base..place.span.end - base, place.gap, place.word))
        .collect();

    let mut merged: Vec<StyledRange> = Vec::with_capacity(ranges.len() + gaps.len() * 2);
    for &(start, end, style) in &ranges {
        // The first gap that ends after the range starts: the ones before it are not in it.
        let mut from = start;
        let first = gaps.partition_point(|(gap, ..)| gap.end <= start);
        for (gap, ..) in &gaps[first..] {
            if gap.start >= end {
                break;
            }
            if from < gap.start {
                merged.push(StyledRange::new(from..gap.start, style.clone()));
            }
            from = from.max(gap.end);
        }
        if from < end {
            merged.push(StyledRange::new(from..end, style.clone()));
        }
    }
    for (gap, width, word) in &gaps {
        let covering = ranges.partition_point(|(start, ..)| *start <= gap.start);
        let mut style = covering
            .checked_sub(1)
            .map(|index| ranges[index])
            .filter(|(_, end, _)| *end > gap.start)
            .map_or_else(SpanStyle::new, |(.., style)| style.clone());
        if *word {
            style.word_spacing = Some(*width * scale);
        } else {
            style.letter_spacing = Some(*width * scale);
        }
        merged.push(StyledRange::new(gap.clone(), style));
    }
    Some(merged.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREY: Color = Color::Rgba(120, 124, 135, 255);

    fn hint(offset: usize, label: &str) -> Inlay {
        Inlay::new(offset, label, GREY)
    }

    /// Every label is as wide as it has characters, and a padded side adds the pad.
    fn width(inlay: &Inlay) -> f32 {
        inlay.label().chars().count() as f32 + if inlay.pad_left() { 4.0 } else { 0.0 } + if inlay.pad_right() { 4.0 } else { 0.0 }
    }

    fn placed(inlays: &[Inlay], text: &str, base: usize) -> Vec<Placed> {
        place(&Inlays::new(inlays.to_vec()), text, base, None, &mut width)
    }

    /// Where the hints go in `text`, which they were made for a text that `edit` made into.
    fn moved(inlays: &[Inlay], text: &str, edit: (Range<usize>, Range<usize>)) -> Vec<(usize, Range<usize>)> {
        place(&Inlays::new(inlays.to_vec()), text, 0, Some(&edit), &mut width)
            .into_iter()
            .map(|place| (place.at, place.items))
            .collect()
    }

    /// Where the hints go in the whole of `text`, a snapshot, as the field asks.
    fn place_in(inlays: &Inlays, text: &TextSnapshot, measure: &mut dyn FnMut(&Inlay) -> f32) -> Vec<Placed> {
        let edit = inlays.text().and_then(|made_for| made_for.diff(text));
        place(inlays, &text.slice(0..text.len()), 0, edit.as_ref(), measure)
    }

    fn red() -> SpanStyle {
        SpanStyle::new().color(Color::Rgba(255, 0, 0, 255))
    }

    fn blue() -> SpanStyle {
        SpanStyle::new().color(Color::Rgba(0, 0, 255, 255))
    }

    #[test]
    fn a_label_goes_between_the_character_before_it_and_the_one_after() {
        let found = placed(&[hint(5, ": i32")], "let x = 1", 0);
        // After `(`, before `1`: neither is a space, so the room is spread after `(`.
        let found = placed(&[hint(4, ": i32")], "foo(1)", 0);
        assert_eq!(found, vec![Placed { at: 4, span: 3..4, gap: 5.0, widths: vec![5.0], eol: false, word: false, items: 0..1 }]);
    }

    #[test]
    fn a_label_before_a_space_has_its_room_in_the_space_and_leaves_the_character_before_it_alone() {
        let found = placed(&[hint(5, ": i32")], "let x = 1", 0);
        assert_eq!(found, vec![Placed { at: 5, span: 5..6, gap: 5.0, widths: vec![5.0], eol: false, word: true, items: 0..1 }]);
        // A tab is a space, and a no-break one too; a line break is not, and is the end of a line.
        assert!(placed(&[hint(5, "x")], "let x\t= 1", 0)[0].word);
        assert!(placed(&[hint(5, "x")], "let x\u{a0}= 1", 0)[0].word);
        assert!(!placed(&[hint(5, "x")], "let x\n= 1", 0)[0].word);
        assert!(placed(&[hint(5, "x")], "let x\n= 1", 0)[0].eol);
    }

    #[test]
    fn the_labels_of_one_byte_share_the_room_and_keep_their_order() {
        let found = placed(&[hint(5, "bb"), hint(5, "c").padded(true, true)], "let x = 1", 0);
        assert_eq!(found, vec![Placed { at: 5, span: 5..6, gap: 2.0 + 9.0, widths: vec![2.0, 9.0], eol: false, word: true, items: 0..2 }]);
    }

    #[test]
    fn the_labels_are_found_in_the_order_of_the_text_whatever_the_order_they_were_given_in() {
        let found = placed(&[hint(7, "b"), hint(2, "a")], "let x = 1", 0);
        assert_eq!(found.iter().map(|place| place.at).collect::<Vec<_>>(), [2, 7]);
        assert_eq!(found[0].items, 0..1);
        assert_eq!(found[1].items, 1..2);
    }

    #[test]
    fn a_label_at_the_end_of_a_line_is_after_its_last_character_and_takes_no_room() {
        // Nothing follows it on its line, so there is nothing to make room from: a line break is not shaped with the text.
        let end = |at, span: Range<usize>| Placed { at, span, gap: 1.0, widths: vec![1.0], eol: true, word: false, items: 0..1 };
        assert_eq!(placed(&[hint(2, "x")], "ab\ncd", 0), vec![end(2, 1..2)]);
        assert_eq!(placed(&[hint(2, "x")], "ab\r\ncd", 0), vec![end(2, 1..2)]);
        assert_eq!(placed(&[hint(2, "x")], "ab", 0), vec![end(2, 1..2)], "the end of the text is the end of a line");
        assert_eq!(placed(&[hint(2, "x")], "ab\n", 0), vec![end(2, 1..2)]);
    }

    #[test]
    fn the_room_of_a_label_at_the_end_of_a_line_is_not_asked_of_the_layout() {
        let end = Placed { at: 2, span: 1..2, gap: 5.0, widths: vec![5.0], eol: true, word: false, items: 0..1 };
        let ranges: Rc<[StyledRange]> = vec![range(0, 9, red())].into();
        let found = with_gaps(Some(ranges.clone()), &[end.clone()], 0, 1.0);
        assert_eq!(found.as_deref(), Some(&*ranges), "what was styled is as it was");
        assert_eq!(with_gaps(None, &[end], 0, 1.0), None);
    }

    #[test]
    fn what_has_no_character_on_each_side_on_its_line_is_left_out() {
        assert!(placed(&[hint(0, "x")], "ab", 0).is_empty(), "the start of the text");
        assert!(placed(&[hint(3, "x")], "ab\ncd", 0).is_empty(), "the start of a line");
        assert!(placed(&[hint(9, "x")], "ab", 0).is_empty(), "past the end");
        assert!(placed(&[hint(2, "x")], "ab\n\ncd", 0).iter().all(|place| place.at != 3), "the start of an empty line");
        assert!(placed(&[hint(3, "x")], "ab\r\ncd", 0).is_empty(), "in the middle of a line break");
    }

    #[test]
    fn what_is_in_the_middle_of_a_character_is_left_out() {
        // `é` is two bytes, and `e` with an accent is a character of two code points.
        assert!(placed(&[hint(1, "x")], "é = 1", 0).is_empty(), "inside a character");
        assert!(placed(&[hint(2, "x")], "e\u{301} = 1", 0).is_empty(), "inside a cluster");
        assert_eq!(placed(&[hint(2, "x")], "é = 1", 0), vec![Placed { at: 2, span: 2..3, gap: 1.0, widths: vec![1.0], eol: false, word: true, items: 0..1 }]);
        assert_eq!(placed(&[hint(3, "x")], "e\u{301} = 1", 0), vec![Placed { at: 3, span: 3..4, gap: 1.0, widths: vec![1.0], eol: false, word: true, items: 0..1 }]);
    }

    #[test]
    fn a_label_whose_room_would_be_in_the_room_of_the_one_before_is_left_out() {
        let found = placed(&[hint(5, "a"), hint(6, "b"), hint(8, "c")], "let x = 1", 0);
        assert_eq!(found.iter().map(|place| place.at).collect::<Vec<_>>(), [5, 8], "the one at 6 is next to the one at 5");
        assert_eq!(found[1].items, 2..3);
    }

    #[test]
    fn a_window_of_the_text_is_placed_in_the_offsets_of_the_whole() {
        let found = placed(&[hint(50, "before"), hint(105, "in"), hint(500, "after")], "let x = 1", 100);
        assert_eq!(found, vec![Placed { at: 105, span: 105..106, gap: 2.0, widths: vec![2.0], eol: false, word: true, items: 1..2 }]);
        // The label at the very start of the window is at the start of a line only when the window starts one.
        assert!(placed(&[hint(100, "x")], "let x = 1", 100).is_empty());
    }

    #[test]
    fn a_label_in_a_pill_says_so_and_the_pills_are_as_round_as_the_set_says() {
        assert!(!hint(1, "x").has_background());
        assert_eq!(hint(1, "x").background(GREY).pill(), Some(GREY));
        let set = Inlays::new(vec![]).radius(7.0).inset(3.0);
        assert_eq!((set.corner_radius(), set.text_inset()), (7.0, 3.0));
        assert_eq!(Inlays::new(vec![]).radius(-1.0).corner_radius(), 0.0, "never negative");
        assert_ne!(Inlays::new(vec![]).id(), Inlays::new(vec![]).id(), "each set is its own");
    }

    #[test]
    fn what_comes_after_an_edit_moves_with_it_and_what_comes_before_stays() {
        // `let x = 1;` became `let abc x = 1;`: three bytes and a space were put in at 4, and the hint was at 5.
        let text = "let abcd x = 1;";
        let hints = [hint(5, "a"), hint(9, "b")];
        assert_eq!(moved(&hints, text, (4..4, 4..9)), vec![(10, 0..1), (14, 1..2)], "both are after it");
        assert_eq!(moved(&[hint(3, "a")], text, (4..4, 4..9)), vec![(3, 0..1)], "before it");
        // Text that was taken out: `let abcd x = 1;` was `let x = 1;`.
        assert_eq!(moved(&[hint(10, "a")], "let x = 1;", (4..9, 4..4)), vec![(5, 0..1)]);
    }

    #[test]
    fn a_hint_in_the_text_that_was_replaced_is_gone_and_one_at_its_edges_is_kept_by_the_side_it_is_on() {
        let text = "let abcdef = 1;";
        // `let x = 1;` has `x` replaced by `abcdef`: the hint at 5 is the end of what was replaced, and moves with what follows.
        assert_eq!(moved(&[hint(5, "a")], text, (4..5, 4..10)), vec![(10, 0..1)]);
        // A hint in the middle of what was replaced has nothing left to be beside.
        assert!(moved(&[hint(6, "a")], text, (4..8, 4..10)).is_empty());
        // The start of what was replaced stays.
        assert_eq!(moved(&[hint(4, "a")], text, (4..8, 4..10)), vec![(4, 0..1)]);
        // Something put in at the byte of a hint goes before it: the hint follows what is typed.
        assert_eq!(moved(&[hint(5, "a")], "let xy = 1;", (5..5, 5..6)), vec![(6, 0..1)]);
    }

    #[test]
    fn hints_made_for_a_text_are_moved_by_the_difference_of_the_text_they_are_laid_in_from_it() {
        let before = TextSnapshot::from_text("let x = 1;\nlet y = 2;");
        let hints = Inlays::new(vec![hint(5, "a"), hint(16, "b")]).of_text(before.clone());
        let after = TextSnapshot::from_text("let xs = 1;\nlet y = 2;");
        let found = place_in(&hints, &after, &mut width);
        assert_eq!(found.iter().map(|place| place.at).collect::<Vec<_>>(), [6, 17], "an `s` was put in at 5");
        let same = place_in(&hints, &before, &mut width);
        assert_eq!(same.iter().map(|place| place.at).collect::<Vec<_>>(), [5, 16], "and the text they were made for is as it was");
        let unknown = Inlays::new(vec![hint(5, "a")]);
        assert_eq!(place_in(&unknown, &after, &mut width).first().map(|place| place.at), Some(5), "no text, no moving");
    }

    #[test]
    fn no_labels_and_no_text_place_nothing() {
        assert!(placed(&[], "let x = 1", 0).is_empty());
        assert!(placed(&[hint(1, "x")], "", 0).is_empty());
    }

    fn range(from: usize, to: usize, style: SpanStyle) -> StyledRange {
        StyledRange::new(from..to, style)
    }

    fn gap_of(from: usize, to: usize, gap: f32) -> Placed {
        Placed { at: to, span: from..to, gap, widths: vec![gap], eol: false, word: false, items: 0..1 }
    }

    fn summary(ranges: &Option<Rc<[StyledRange]>>) -> Vec<(Range<usize>, Option<Color>, Option<f32>)> {
        let mut found: Vec<_> = ranges
            .iter()
            .flat_map(|ranges| ranges.iter())
            .map(|styled| (styled.range.clone(), styled.style.color, styled.style.letter_spacing))
            .collect();
        found.sort_by_key(|(range, ..)| range.start);
        found
    }

    #[test]
    fn the_room_is_a_range_of_the_two_characters_even_where_nothing_was_styled() {
        let found = with_gaps(None, &[gap_of(4, 5, 5.0)], 0, 1.0);
        assert_eq!(summary(&found), vec![(4..5, None, Some(5.0))]);
        assert_eq!(with_gaps(None, &[], 0, 1.0), None, "no room, and nothing styled, is nothing");
    }

    #[test]
    fn the_room_takes_the_style_of_the_character_before_it_and_cuts_what_it_is_in() {
        let ranges: Rc<[StyledRange]> = vec![range(0, 9, red())].into();
        let found = with_gaps(Some(ranges), &[gap_of(4, 5, 5.0)], 0, 1.0);
        let red = red().color;
        assert_eq!(summary(&found), vec![(0..4, red, None), (4..5, red, Some(5.0)), (5..9, red, None)]);
    }

    #[test]
    fn the_character_after_keeps_its_own_style_and_the_room_takes_the_style_of_the_one_before() {
        let ranges: Rc<[StyledRange]> = vec![range(0, 3, red()), range(4, 5, blue()), range(5, 9, red())].into();
        let found = with_gaps(Some(ranges), &[gap_of(4, 5, 5.0)], 0, 1.0);
        let (red, blue) = (red().color, blue().color);
        assert_eq!(summary(&found), vec![(0..3, red, None), (4..5, blue, Some(5.0)), (5..9, red, None)]);
    }

    #[test]
    fn a_range_that_starts_earlier_keeps_the_bytes_two_share() {
        let ranges: Rc<[StyledRange]> = vec![range(2, 8, blue()), range(0, 5, red())].into();
        let found = with_gaps(Some(ranges), &[gap_of(3, 4, 2.0)], 0, 1.0);
        let (red, blue) = (red().color, blue().color);
        // The red one starts first, so it has the bytes up to 5 and the blue one what is after.
        assert_eq!(summary(&found), vec![(0..3, red, None), (3..4, red, Some(2.0)), (4..5, red, None), (5..8, blue, None)]);
    }

    #[test]
    fn a_window_of_the_text_has_its_ranges_in_its_own_offsets_and_the_room_is_found_there() {
        let ranges: Rc<[StyledRange]> = vec![range(0, 9, red())].into();
        let found = with_gaps(Some(ranges), &[gap_of(104, 105, 5.0)], 100, 1.0);
        let red = red().color;
        assert_eq!(summary(&found), vec![(0..4, red, None), (4..5, red, Some(5.0)), (5..9, red, None)]);
    }

    fn word_gap_of(at: usize, gap: f32) -> Placed {
        Placed { at, span: at..at + 1, gap, widths: vec![gap], eol: false, word: true, items: 0..1 }
    }

    #[test]
    fn the_room_in_a_space_is_the_word_spacing_of_the_space_alone_in_the_style_the_space_has() {
        // A name that is underlined, and the space after it that is not part of the name.
        let underlined = SpanStyle::new().color(Color::Rgba(255, 0, 0, 255)).text_decoration(aimer_style::TextDecoration::Underline);
        let ranges: Rc<[StyledRange]> = vec![StyledRange::new(4..5, underlined.clone())].into();
        let found = with_gaps(Some(ranges), &[word_gap_of(5, 6.0)], 0, 2.0).expect("styled");
        let mut found: Vec<_> = found.iter().cloned().collect();
        found.sort_by_key(|run| run.range.start);
        assert_eq!(found.len(), 2, "the name, which is as it was, and the space");
        assert_eq!((found[0].range.clone(), found[0].style.clone()), (4..5, underlined), "the name is not touched");
        assert_eq!(found[1].range, 5..6);
        assert_eq!(found[1].style.word_spacing, Some(12.0), "in physical pixels");
        assert_eq!(found[1].style.letter_spacing, None);
        assert_eq!(found[1].style.text_decoration, None, "and it is not underlined with the name");
    }

    #[test]
    fn the_room_in_a_space_that_is_in_a_range_keeps_the_style_of_that_range() {
        let ranges: Rc<[StyledRange]> = vec![StyledRange::new(0..9, red())].into();
        let found = with_gaps(Some(ranges), &[word_gap_of(5, 3.0)], 0, 1.0);
        let red = red().color;
        assert_eq!(summary(&found), vec![(0..5, red, None), (5..6, red, None), (6..9, red, None)]);
        let spaced = found.unwrap().iter().find(|run| run.range == (5..6)).cloned().unwrap();
        assert_eq!(spaced.style.word_spacing, Some(3.0));
    }

    #[test]
    fn the_room_is_in_physical_pixels_at_the_scale_given() {
        let found = with_gaps(None, &[gap_of(4, 5, 5.0)], 0, 2.0);
        assert_eq!(summary(&found), vec![(4..5, None, Some(10.0))]);
    }

    #[test]
    fn two_places_are_each_cut_out() {
        let ranges: Rc<[StyledRange]> = vec![range(0, 12, red())].into();
        let found = with_gaps(Some(ranges), &[gap_of(2, 3, 1.0), gap_of(8, 9, 2.0)], 0, 1.0);
        let red = red().color;
        assert_eq!(
            summary(&found),
            vec![(0..2, red, None), (2..3, red, Some(1.0)), (3..8, red, None), (8..9, red, Some(2.0)), (9..12, red, None)]
        );
    }
}
