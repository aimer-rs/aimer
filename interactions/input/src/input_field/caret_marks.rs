//! Boxes round what the caret is next to: a bracket and the one it matches, say.
//!
//! The text area asks, each time it is drawn with a caret in it, which ranges of the text are to be boxed, and draws a
//! rounded box behind the text round each. When the answer is not what it was, the boxes are marked for a moment: a line
//! that starts larger than the range and closes on it as it fades, which is how an editor shows where a bracket's other half is.

use std::ops::Range;
use std::rc::Rc;

use aimer_text_buffer::TextSnapshot;
use aimer_widget::base::Color;

/// What is boxed for the caret, decided by the app: see [`TextArea::caret_marks`](crate::TextArea::caret_marks).
///
/// # Examples
///
/// ```
/// use aimer_input::input::{CaretMark, CaretMarks};
/// use aimer_widget::base::Color;
///
/// // The character before the caret, when the caret is a cursor and there is one.
/// let marks = CaretMarks::new(|text, selection| {
///     if selection.start != selection.end || selection.start == 0 {
///         return Vec::new();
///     }
///     let start = (0..selection.start).rev().find(|&at| text.byte_at(at).is_some_and(|byte| byte & 0xC0 != 0x80)).unwrap_or(0);
///     vec![CaretMark::new(start..selection.start, Color::Rgba(255, 92, 150, 255))]
/// });
/// # let _ = marks;
/// ```
#[derive(Clone)]
pub struct CaretMarks {
    pub(crate) of_caret: Rc<dyn Fn(&TextSnapshot, Range<usize>) -> Vec<CaretMark>>,
}

impl CaretMarks {
    /// Marks for what `of_caret` says of the text, as a snapshot, and the selection in it in bytes, which is a cursor
    /// where it is empty. It is asked only while the field has the focus, and again only when the text or the selection is
    /// not what it was asked about, so it may look through the text as far as it needs to. It must be pure.
    ///
    /// Ranges past the end of the text are cut to it, and an empty range is not boxed.
    pub fn new(of_caret: impl Fn(&TextSnapshot, Range<usize>) -> Vec<CaretMark> + 'static) -> Self {
        Self { of_caret: Rc::new(of_caret) }
    }
}

/// A range to box, and the color of its box.
#[derive(Clone, Debug, PartialEq)]
pub struct CaretMark {
    pub(crate) range: Range<usize>,
    pub(crate) color: Color,
}

impl CaretMark {
    /// A box in `color` round the bytes in `range`, which are on character boundaries.
    pub fn new(range: Range<usize>, color: impl Into<Color>) -> Self {
        Self { range, color: color.into() }
    }
}
