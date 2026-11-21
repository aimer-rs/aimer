//! Lines drawn under ranges of the text: the red one under an error and the yellow one under a warning, which a language server
//! points at.
//!
//! An [`Underline`] is a range of bytes and a color. The text is not touched. The line is straight, a little under the text, with
//! rounded ends, and it goes under a range that is on more than one line a line at a time.

use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use aimer_text_buffer::TextSnapshot;
use aimer_widget::base::Color;

/// Source of the identities that tell two sets of underlines apart. Zero is reserved for "none".
static NEXT_UNDERLINES_ID: AtomicU64 = AtomicU64::new(1);

/// One range of the text to draw a line under, and the color of the line.
///
/// # Examples
///
/// ```
/// use aimer_input::input::Underline;
/// use aimer_widget::base::Color;
///
/// // The name `count` of `let count = 1;`, as an error.
/// let error = Underline::new(4..9, Color::Rgba(235, 64, 64, 255));
/// assert_eq!(error.range(), 4..9);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Underline {
    range: Range<usize>,
    color: Color,
}

impl Underline {
    /// A line in `color` under the bytes `range`, which are on character boundaries.
    pub fn new(range: Range<usize>, color: Color) -> Self {
        Self { range, color }
    }

    /// The bytes the line is under.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// The color of the line.
    pub fn color(&self) -> Color {
        self.color
    }
}

/// The lines under a text, as the field takes them. A set is known by the value it was made as: make one when the lines change, and
/// keep it while they do not, so that the field does not draw them again for nothing.
#[derive(Clone, Debug)]
pub struct Underlines {
    id: u64,
    items: Rc<[Underline]>,
    thickness: f32,
    radius: f32,
    text: Option<TextSnapshot>,
}

impl Underlines {
    /// How thick a line is, in logical pixels, unless it is said.
    pub const DEFAULT_THICKNESS: f32 = 2.0;
    /// How round the ends of a line are, in logical pixels, unless it is said.
    pub const DEFAULT_RADIUS: f32 = 1.0;

    /// A set of lines. They are drawn in the order they are given, so that one that is given later is over one that is under the same
    /// bytes: the worst of the problems of a place is given last.
    pub fn new(items: Vec<Underline>) -> Self {
        Self {
            id: NEXT_UNDERLINES_ID.fetch_add(1, Ordering::Relaxed),
            items: items.into(),
            thickness: Self::DEFAULT_THICKNESS,
            radius: Self::DEFAULT_RADIUS,
            text: None,
        }
    }

    /// How thick the lines are, in logical pixels.
    #[must_use]
    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness.max(0.0);
        self.id = NEXT_UNDERLINES_ID.fetch_add(1, Ordering::Relaxed);
        self
    }

    /// How round the ends of the lines are, in logical pixels. A radius larger than half the thickness is half of it.
    #[must_use]
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius.max(0.0);
        self.id = NEXT_UNDERLINES_ID.fetch_add(1, Ordering::Relaxed);
        self
    }

    /// Says which text the ranges are of. A field whose text is another one, because it was edited since, moves each line by the edit
    /// that makes one into the other: those after it by as much as it added or took away, those it was typed in the middle of by the
    /// same, and those that it only partly took away are left out. Without it the ranges are taken as they are of the text the field has.
    #[must_use]
    pub fn of_text(mut self, text: TextSnapshot) -> Self {
        self.text = Some(text);
        self.id = NEXT_UNDERLINES_ID.fetch_add(1, Ordering::Relaxed);
        self
    }

    /// The lines, in the order they are drawn.
    pub fn items(&self) -> &[Underline] {
        &self.items
    }

    /// How many lines there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether `other` is this very set, or a copy of it.
    pub fn same_as(&self, other: &Self) -> bool {
        self.id == other.id
    }

    /// The text the ranges are of, if it was said; see [`of_text`](Self::of_text).
    pub fn text(&self) -> Option<&TextSnapshot> {
        self.text.as_ref()
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn line_thickness(&self) -> f32 {
        self.thickness
    }

    pub(crate) fn corner_radius(&self) -> f32 {
        self.radius
    }
}

/// Where the range `range` of the text before an edit is in the text after it: `edit` is the bytes that were replaced and the bytes
/// that replaced them. A range after the edit moves by what it added or took away, one before it stays, one the edit is inside of
/// grows or shrinks by it, and one that the edit is only partly in has nothing left to be under.
pub(crate) fn moved(range: &Range<usize>, edit: Option<&(Range<usize>, Range<usize>)>) -> Option<Range<usize>> {
    let Some((old, new)) = edit else {
        return Some(range.clone());
    };
    let shift = |at: usize| at + new.len() - old.len();
    if range.end <= old.start {
        Some(range.clone())
    } else if range.start >= old.end {
        Some(shift(range.start)..shift(range.end))
    } else if range.start <= old.start && old.end <= range.end {
        let end = shift(range.end);
        (range.start < end).then_some(range.start..end)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::Rgba(235, 64, 64, 255);

    fn edit(old: Range<usize>, new: Range<usize>) -> Option<(Range<usize>, Range<usize>)> {
        Some((old, new))
    }

    #[test]
    fn a_range_with_no_edit_is_where_it_is() {
        assert_eq!(moved(&(4..9), None), Some(4..9));
    }

    #[test]
    fn a_range_after_an_edit_moves_by_what_it_added_or_took_away_and_one_before_it_stays() {
        // Three bytes put in at 2, and four taken out of 2..6.
        assert_eq!(moved(&(4..9), edit(2..2, 2..5).as_ref()), Some(7..12));
        assert_eq!(moved(&(6..9), edit(2..6, 2..2).as_ref()), Some(2..5));
        assert_eq!(moved(&(0..2), edit(2..6, 2..2).as_ref()), Some(0..2), "it ends where the edit begins");
        assert_eq!(moved(&(0..1), edit(2..2, 2..5).as_ref()), Some(0..1));
    }

    #[test]
    fn what_is_put_in_beside_a_range_is_not_in_it() {
        assert_eq!(moved(&(4..9), edit(9..9, 9..12).as_ref()), Some(4..9), "after its end");
        assert_eq!(moved(&(4..9), edit(4..4, 4..6).as_ref()), Some(6..11), "before its start: it moves");
    }

    #[test]
    fn a_range_that_is_typed_in_grows_and_shrinks_with_what_is_typed() {
        assert_eq!(moved(&(4..9), edit(6..6, 6..8).as_ref()), Some(4..11));
        assert_eq!(moved(&(4..9), edit(6..8, 6..6).as_ref()), Some(4..7));
        assert_eq!(moved(&(4..9), edit(4..9, 4..6).as_ref()), Some(4..6), "all of it replaced is a range of what replaced it");
        assert_eq!(moved(&(4..9), edit(4..9, 4..4).as_ref()), None, "and all of it taken out has nothing left");
    }

    #[test]
    fn a_range_that_is_partly_taken_away_is_gone() {
        assert_eq!(moved(&(4..9), edit(2..6, 2..3).as_ref()), None);
        assert_eq!(moved(&(4..9), edit(7..12, 7..8).as_ref()), None);
    }

    #[test]
    fn a_set_keeps_its_order_and_is_its_own_until_it_is_changed() {
        let set = Underlines::new(vec![Underline::new(4..9, RED), Underline::new(0..2, RED)]);
        assert_eq!(set.items().iter().map(Underline::range).collect::<Vec<_>>(), [4..9, 0..2], "the order they are drawn in");
        assert_eq!(set.len(), 2);
        assert!(!set.is_empty());
        assert!(set.same_as(&set.clone()));
        assert!(!set.same_as(&Underlines::new(vec![Underline::new(4..9, RED), Underline::new(0..2, RED)])));
        let thicker = set.clone().thickness(3.0);
        assert!(!thicker.same_as(&set), "a change of how they are drawn is a new set");
        assert_eq!((thicker.line_thickness(), thicker.corner_radius()), (3.0, Underlines::DEFAULT_RADIUS));
        assert_eq!(set.clone().radius(-1.0).corner_radius(), 0.0, "never negative");
        assert!(set.text().is_none());
        assert!(set.of_text(TextSnapshot::from_text("abc")).text().is_some());
    }
}
