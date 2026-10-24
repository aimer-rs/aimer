use std::sync::Arc;

use aimer_text::TextSelection;
use unicode_segmentation::UnicodeSegmentation;

/// A normalized half-open range of UTF-8 byte offsets.
///
/// A range does not validate itself against text. [`TextEditingValue`] clamps
/// both endpoints to extended-grapheme boundaries whenever a range becomes
/// part of an editing value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextRange {
    start: usize,
    end: usize,
}

impl TextRange {
    /// Creates a range, ordering its endpoints from least to greatest.
    #[inline]
    pub const fn new(first: usize, second: usize) -> Self {
        if first <= second {
            Self {
                start: first,
                end: second,
            }
        } else {
            Self {
                start: second,
                end: first,
            }
        }
    }

    /// Returns the inclusive start byte offset.
    #[inline]
    pub const fn start(self) -> usize {
        self.start
    }

    /// Returns the exclusive end byte offset.
    #[inline]
    pub const fn end(self) -> usize {
        self.end
    }

    /// Returns whether the range contains no text.
    #[inline]
    pub const fn is_collapsed(self) -> bool {
        self.start == self.end
    }
}

/// An immutable snapshot of editable text, selection, and IME composition.
///
/// Offsets are UTF-8 byte offsets. Construction clamps selection endpoints to
/// the preceding extended-grapheme boundary and expands composing endpoints to
/// cover complete graphemes. Consequently every stored offset is safe for
/// slicing and editing without splitting a user-perceived character.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEditingValue {
    text: Arc<str>,
    selection: TextSelection,
    composing: Option<TextRange>,
}

impl TextEditingValue {
    /// Creates a normalized editing snapshot.
    pub fn new(
        text: impl Into<String>,
        selection: TextSelection,
        composing: Option<TextRange>,
    ) -> Self {
        let text: Arc<str> = Arc::from(text.into());
        let selection = TextSelection::new(
            floor_grapheme_boundary(&text, selection.anchor()),
            floor_grapheme_boundary(&text, selection.focus()),
        );
        let composing = composing.map(|range| {
            TextRange::new(
                floor_grapheme_boundary(&text, range.start()),
                ceil_grapheme_boundary(&text, range.end()),
            )
        });
        Self {
            text,
            selection,
            composing,
        }
    }

    /// Creates a snapshot with its caret collapsed at the end of `text`.
    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self::new(text, TextSelection::collapsed(end), None)
    }

    /// Returns the snapshot's text.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the directional selection in UTF-8 byte offsets.
    #[inline]
    pub const fn selection(&self) -> TextSelection {
        self.selection
    }

    /// Returns the active composing range, if an input method owns one.
    #[inline]
    pub const fn composing(&self) -> Option<TextRange> {
        self.composing
    }

    pub(crate) fn with_selection(&self, selection: TextSelection) -> Self {
        Self {
            text: self.text.clone(),
            selection: TextSelection::new(
                floor_grapheme_boundary(&self.text, selection.anchor()),
                floor_grapheme_boundary(&self.text, selection.focus()),
            ),
            composing: self.composing,
        }
    }
}

impl Default for TextEditingValue {
    #[inline]
    fn default() -> Self {
        Self::with_text(String::new())
    }
}

fn floor_grapheme_boundary(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    if offset == text.len() {
        return offset;
    }
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index <= offset)
        .last()
        .unwrap_or(0)
}

fn ceil_grapheme_boundary(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index >= offset)
        .unwrap_or(text.len())
}
