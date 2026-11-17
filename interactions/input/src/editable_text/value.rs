use std::sync::{Arc, OnceLock};

use aimer_text::TextSelection;

use super::graphemes::{self, GraphemeIndex};

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
#[derive(Clone)]
pub struct TextEditingValue {
    text: Arc<str>,
    selection: TextSelection,
    composing: Option<TextRange>,
    /// Where the graphemes of `text` are, made the first time they are asked for and handed on
    /// to the value an edit makes. Text that is ASCII does not need one.
    graphemes: OnceLock<Arc<GraphemeIndex>>,
    /// Whether every grapheme of `text` is one byte, which needs no index: found once, by one
    /// scan of the text, and handed on to the values that share or edit it.
    one_byte: OnceLock<bool>,
}

impl PartialEq for TextEditingValue {
    fn eq(&self, other: &Self) -> bool {
        self.selection == other.selection && self.composing == other.composing && self.text == other.text
    }
}

impl Eq for TextEditingValue {}

impl std::fmt::Debug for TextEditingValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextEditingValue")
            .field("text", &self.text)
            .field("selection", &self.selection)
            .field("composing", &self.composing)
            .finish()
    }
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
            graphemes::floor_boundary(&text, selection.anchor()),
            graphemes::floor_boundary(&text, selection.focus()),
        );
        let composing = composing.map(|range| {
            TextRange::new(
                graphemes::floor_boundary(&text, range.start()),
                graphemes::ceil_boundary(&text, range.end()),
            )
        });
        Self {
            text,
            selection,
            composing,
            graphemes: OnceLock::new(),
            one_byte: OnceLock::new(),
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

    /// The snapshot's text, shared: a copy of the handle and not of the text.
    #[inline]
    pub(crate) fn text_arc(&self) -> &Arc<str> {
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
                self.floor_to_boundary(selection.anchor()),
                self.floor_to_boundary(selection.focus()),
            ),
            composing: self.composing,
            graphemes: self.graphemes.clone(),
            one_byte: self.one_byte.clone(),
        }
    }

    /// Whether every grapheme of the text is one byte.
    fn is_one_byte(&self) -> bool {
        *self.one_byte.get_or_init(|| graphemes::is_one_byte_graphemes(&self.text))
    }

    /// The index of the graphemes of the text, made now if it has not been.
    fn index(&self) -> &GraphemeIndex {
        self.graphemes.get_or_init(|| Arc::new(GraphemeIndex::build(&self.text)))
    }

    /// How many graphemes the text has.
    pub(crate) fn grapheme_count(&self) -> usize {
        if self.is_one_byte() {
            self.text.len()
        } else {
            self.index().count()
        }
    }

    /// The byte offset of the grapheme `offset`; the end of the text for one past the last.
    pub(crate) fn byte_of_grapheme(&self, offset: usize) -> usize {
        if self.is_one_byte() {
            offset.min(self.text.len())
        } else {
            self.index().byte_of(&self.text, offset)
        }
    }

    /// How many graphemes come before the byte offset `byte`, which is on a boundary.
    pub(crate) fn grapheme_of_byte(&self, byte: usize) -> usize {
        if self.is_one_byte() {
            byte.min(self.text.len())
        } else {
            self.index().grapheme_of(&self.text, byte)
        }
    }

    /// The greatest grapheme boundary at or before `offset`.
    pub(crate) fn floor_to_boundary(&self, offset: usize) -> usize {
        if self.is_one_byte() {
            offset.min(self.text.len())
        } else {
            self.index().floor(&self.text, offset)
        }
    }

    /// The value whose text is this one's with the bytes `range` replaced by `replacement`, and
    /// whose selection and composing range are as given (made safe on the new text).
    ///
    /// The new text is written once, into the buffer it keeps, and the graphemes of this text are
    /// handed on to it, so an edit costs the one copy of the text and what the edit touches.
    pub(crate) fn splice(
        &self,
        range: std::ops::Range<usize>,
        replacement: &str,
        selection: impl FnOnce(usize) -> TextSelection,
        composing: Option<TextRange>,
    ) -> Self {
        let text = concat_text(&[&self.text[..range.start], replacement, &self.text[range.end..]]);
        let end = range.start + replacement.len();
        let value = Self {
            text,
            selection: TextSelection::collapsed(0),
            composing: None,
            graphemes: OnceLock::new(),
            one_byte: OnceLock::new(),
        };
        // Text that was one byte a grapheme stays so, with one-byte text put in it.
        if self.is_one_byte() && graphemes::is_one_byte_graphemes(replacement) {
            let _ = value.one_byte.set(true);
        }
        let mut value = value.with_graphemes_from(self, range, replacement.len());
        let selection = selection(end);
        value.selection = TextSelection::new(
            value.floor_to_boundary(selection.anchor()),
            value.floor_to_boundary(selection.focus()),
        );
        value.composing = composing.map(|range| {
            TextRange::new(
                value.floor_to_boundary(range.start()),
                value.ceil_to_boundary(range.end()),
            )
        });
        value
    }

    /// The least grapheme boundary at or after `offset`.
    pub(crate) fn ceil_to_boundary(&self, offset: usize) -> usize {
        if self.is_one_byte() {
            offset.min(self.text.len())
        } else {
            self.index().ceil(&self.text, offset)
        }
    }

    /// The boundary before the one at `byte`, or `byte` at the start of the text.
    pub(crate) fn previous_boundary(&self, byte: usize) -> usize {
        if byte == 0 {
            0
        } else if self.is_one_byte() {
            byte - 1
        } else {
            let before = self.index().grapheme_of(&self.text, byte);
            self.index().byte_of(&self.text, before.saturating_sub(1))
        }
    }

    /// The boundary after the one at `byte`, or `byte` at the end of the text.
    pub(crate) fn next_boundary(&self, byte: usize) -> usize {
        if byte >= self.text.len() {
            self.text.len()
        } else if self.is_one_byte() {
            byte + 1
        } else {
            let before = self.index().grapheme_of(&self.text, byte);
            self.index().byte_of(&self.text, before + 1)
        }
    }

    /// Hands the index of the graphemes of `previous`'s text on to this value, whose text is
    /// `previous`'s with the bytes `replaced` of it taken out and `inserted` bytes put in their
    /// place: the graphemes are not walked again from the start.
    pub(crate) fn with_graphemes_from(self, previous: &Self, replaced: std::ops::Range<usize>, inserted: usize) -> Self {
        if !self.is_one_byte() {
            let index = previous.index().spliced(&self.text, replaced, inserted);
            let _ = self.graphemes.set(Arc::new(index));
        }
        self
    }
}

impl Default for TextEditingValue {
    #[inline]
    fn default() -> Self {
        Self::with_text(String::new())
    }
}

/// `parts` joined into one text, written once into the buffer the text keeps: `Arc::from` a
/// `String` would copy it a second time.
fn concat_text(parts: &[&str]) -> Arc<str> {
    let length = parts.iter().map(|part| part.len()).sum();
    let mut bytes = Arc::<[u8]>::new_uninit_slice(length);
    let buffer = Arc::get_mut(&mut bytes).expect("a new Arc has no other owner").as_mut_ptr().cast::<u8>();
    let mut at = 0;
    for part in parts {
        // SAFETY: `at + part.len()` never passes `length`, the sum of the parts, and the buffer
        // is new, so it overlaps no part. (A byte loop would be slow in a debug build.)
        unsafe { std::ptr::copy_nonoverlapping(part.as_ptr(), buffer.add(at), part.len()) };
        at += part.len();
    }
    // SAFETY: every one of the `length` bytes was written above, and they are the bytes of
    // `str`s laid end to end, which are UTF-8.
    unsafe {
        let bytes = bytes.assume_init();
        Arc::from_raw(Arc::into_raw(bytes) as *const str)
    }
}
