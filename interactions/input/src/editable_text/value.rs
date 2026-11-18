use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use aimer_text::TextSelection;
use aimer_text_buffer::TextSnapshot;

use super::graphemes::{self, GraphemeIndex};
use super::large_text::{LargeText, large_text_bytes};

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
    store: Store,
    selection: TextSelection,
    composing: Option<TextRange>,
}

/// The text of a value: shared and contiguous while it is short, and a rope once it is long.
///
/// An edit to a short text writes a copy of it, which costs the text and is the simplest thing that
/// works for the notes and fields that are almost every text. A long one cannot copy itself
/// for each key, so it is kept as a [`TextSnapshot`] (a tree of chunks) that an edit changes along
/// one path and shares the rest of. A value moves from one to the other as its text grows and
/// shrinks, with room between the two sizes so that typing at the edge does not move it back and forth.
#[derive(Clone)]
enum Store {
    Small(SmallText),
    Large(LargeText),
}

/// A short text: contiguous, with what is worked out from it kept once.
#[derive(Clone)]
struct SmallText {
    text: Arc<str>,
    /// Where the graphemes of `text` are, made the first time they are asked for and handed on
    /// to the value an edit makes. Text that is ASCII does not need one.
    graphemes: OnceLock<Arc<GraphemeIndex>>,
    /// Whether every grapheme of `text` is one byte, which needs no index: found once, by one
    /// scan of the text, and handed on to the values that share or edit it.
    one_byte: OnceLock<bool>,
    /// The text as a snapshot, for what reads a window of it, made when first asked for.
    snapshot: OnceLock<TextSnapshot>,
}

impl SmallText {
    fn new(text: Arc<str>) -> Self {
        Self { text, graphemes: OnceLock::new(), one_byte: OnceLock::new(), snapshot: OnceLock::new() }
    }

    fn is_one_byte(&self) -> bool {
        *self.one_byte.get_or_init(|| graphemes::is_one_byte_graphemes(&self.text))
    }

    fn index(&self) -> &GraphemeIndex {
        self.graphemes.get_or_init(|| Arc::new(GraphemeIndex::build(&self.text)))
    }
}

impl PartialEq for TextEditingValue {
    fn eq(&self, other: &Self) -> bool {
        self.selection == other.selection
            && self.composing == other.composing
            && self.same_text(other)
    }
}

impl Eq for TextEditingValue {}

impl std::fmt::Debug for TextEditingValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("TextEditingValue");
        match &self.store {
            Store::Small(small) => debug.field("text", &small.text),
            Store::Large(large) => debug.field("text", &format_args!("<{} bytes>", large.len())),
        };
        debug.field("selection", &self.selection).field("composing", &self.composing).finish()
    }
}

impl TextEditingValue {
    /// Creates a normalized editing snapshot.
    pub fn new(
        text: impl Into<String>,
        selection: TextSelection,
        composing: Option<TextRange>,
    ) -> Self {
        let text: String = text.into();
        let store = if text.len() >= large_text_bytes() {
            Store::Large(LargeText::new(&text))
        } else {
            Store::Small(SmallText::new(Arc::from(text)))
        };
        let mut value = Self { store, selection: TextSelection::collapsed(0), composing: None };
        value.selection = TextSelection::new(value.floor_to_boundary(selection.anchor()), value.floor_to_boundary(selection.focus()));
        value.composing = composing.map(|range| TextRange::new(value.floor_to_boundary(range.start()), value.ceil_to_boundary(range.end())));
        value
    }

    /// [`with_text`](Self::with_text) for a text that is only borrowed. A short one is copied, and a
    /// long one goes straight into the tree of chunks: no `String` is made of it first, which for
    /// a text of megabytes is a copy of all of it.
    pub fn with_str(text: &str) -> Self {
        let end = text.len();
        let store = if text.len() >= large_text_bytes() {
            Store::Large(LargeText::new(text))
        } else {
            Store::Small(SmallText::new(Arc::from(text)))
        };
        let mut value = Self { store, selection: TextSelection::collapsed(0), composing: None };
        value.selection = TextSelection::collapsed(value.floor_to_boundary(end));
        value
    }

    /// Creates a snapshot with its caret collapsed at the end of `text`.
    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self::new(text, TextSelection::collapsed(end), None)
    }

    /// Whether the two values hold the same text, whatever their selections: a long one is
    /// compared by the chunks the two share, and not by reading both out.
    pub fn same_text(&self, other: &Self) -> bool {
        match (&self.store, &other.store) {
            (Store::Small(a), Store::Small(b)) => a.text == b.text,
            (Store::Large(a), Store::Large(b)) => a.snapshot().diff(b.snapshot()).is_none(),
            _ => self.len() == other.len() && *self.text() == *other.text(),
        }
    }

    /// Returns the snapshot's text: borrowed when the text is short, and a copy of all of it
    /// when it is long. A reader that wants a part of a long text asks for that part with
    /// [`slice`](Self::slice).
    #[inline]
    pub fn text(&self) -> Cow<'_, str> {
        match &self.store {
            Store::Small(small) => Cow::Borrowed(&small.text),
            // Sized once: a string grown by `to_string` ends up to twice the text, and copies it as it grows.
            Store::Large(large) => Cow::Owned(large.snapshot().slice(0..large.snapshot().len()).into_owned()),
        }
    }

    /// A short text, shared: a copy of the handle and not of the text. `None` for a long one.
    #[inline]
    pub(crate) fn text_arc(&self) -> Option<&Arc<str>> {
        match &self.store {
            Store::Small(small) => Some(&small.text),
            Store::Large(_) => None,
        }
    }

    /// The text in `range` (byte offsets on character boundaries): borrowed from a short text, and
    /// from a long one when the range lies in one chunk of it.
    pub fn slice(&self, range: std::ops::Range<usize>) -> Cow<'_, str> {
        match &self.store {
            Store::Small(small) => Cow::Borrowed(&small.text[range]),
            Store::Large(large) => large.snapshot().slice(range),
        }
    }

    /// How many bytes the text has.
    #[inline]
    pub fn len(&self) -> usize {
        match &self.store {
            Store::Small(small) => small.text.len(),
            Store::Large(large) => large.len(),
        }
    }

    /// Whether the text has no bytes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many lines the text has: one more than its line feeds. A long text answers from its
    /// tree, and a short one counts them with a vector scan.
    pub fn line_count(&self) -> usize {
        match &self.store {
            Store::Small(small) => aimer_simd::kernels::count_byte(small.text.as_bytes(), b'\n') + 1,
            Store::Large(large) => large.snapshot().line_count(),
        }
    }

    /// The zero-based line and the zero-based column, in graphemes, of the byte offset `byte`.
    ///
    /// A column is counted from the start of the line, so it is how many graphemes come before the
    /// byte on its line, which is what an editor's status bar shows (plus one).
    pub fn line_and_column(&self, byte: usize) -> (usize, usize) {
        let byte = self.floor_to_boundary(byte.min(self.len()));
        let (line, start) = match &self.store {
            Store::Small(small) => {
                let before = &small.text.as_bytes()[..byte];
                let line = aimer_simd::kernels::count_byte(before, b'\n');
                let start = before.iter().rposition(|b| *b == b'\n').map_or(0, |at| at + 1);
                (line, start)
            }
            Store::Large(large) => {
                let snapshot = large.snapshot();
                let line = snapshot.byte_to_line(byte);
                (line, snapshot.line_range(line).start)
            }
        };
        let text = self.slice(start..byte);
        let column = if text.is_ascii() { text.len() } else { unicode_segmentation::UnicodeSegmentation::graphemes(&*text, true).count() };
        (line, column)
    }

    /// Writes the text to `writer` a piece at a time: a long text is written from the chunks it is
    /// in, and no `String` is made of it. This is how a file is saved.
    ///
    /// # Errors
    ///
    /// Whatever `writer` returns.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextEditingValue;
    ///
    /// let value = TextEditingValue::with_str("one\ntwo");
    /// let mut out = Vec::new();
    /// value.write_to(&mut out)?;
    /// assert_eq!(out, b"one\ntwo");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn write_to(&self, writer: &mut impl std::io::Write) -> std::io::Result<()> {
        for piece in self.pieces(0..self.len()) {
            writer.write_all(piece.as_bytes())?;
        }
        Ok(())
    }

    /// The text as a snapshot: O(1) for a long text, and a copy of a short one, which is made
    /// once and kept. A reader that wants a part of the text (a parser, a search) reads it from
    /// the snapshot without making a `String` of all of it.
    pub fn snapshot(&self) -> TextSnapshot {
        match &self.store {
            Store::Small(small) => small.snapshot.get_or_init(|| TextSnapshot::from_text(&small.text)).clone(),
            Store::Large(large) => large.snapshot().clone(),
        }
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
            store: self.store.clone(),
            selection: TextSelection::new(
                self.floor_to_boundary(selection.anchor()),
                self.floor_to_boundary(selection.focus()),
            ),
            composing: self.composing,
        }
    }

    /// How many graphemes the text has.
    pub(crate) fn grapheme_count(&self) -> usize {
        match &self.store {
            Store::Small(small) if small.is_one_byte() => small.text.len(),
            Store::Small(small) => small.index().count(),
            Store::Large(large) => large.grapheme_count(),
        }
    }

    /// The byte offset of the grapheme `offset`; the end of the text for one past the last.
    pub(crate) fn byte_of_grapheme(&self, offset: usize) -> usize {
        match &self.store {
            Store::Small(small) if small.is_one_byte() => offset.min(small.text.len()),
            Store::Small(small) => small.index().byte_of(&small.text, offset),
            Store::Large(large) => large.byte_of_grapheme(offset),
        }
    }

    /// How many graphemes come before the byte offset `byte`, which is on a boundary.
    pub(crate) fn grapheme_of_byte(&self, byte: usize) -> usize {
        match &self.store {
            Store::Small(small) if small.is_one_byte() => byte.min(small.text.len()),
            Store::Small(small) => small.index().grapheme_of(&small.text, byte),
            Store::Large(large) => large.grapheme_of_byte(byte),
        }
    }

    /// The greatest grapheme boundary at or before `offset`.
    pub(crate) fn floor_to_boundary(&self, offset: usize) -> usize {
        match &self.store {
            Store::Small(small) if small.is_one_byte() => offset.min(small.text.len()),
            Store::Small(small) => small.index().floor(&small.text, offset),
            Store::Large(large) => large.snapshot().floor_grapheme_boundary(offset),
        }
    }

    /// The least grapheme boundary at or after `offset`.
    pub(crate) fn ceil_to_boundary(&self, offset: usize) -> usize {
        match &self.store {
            Store::Small(small) if small.is_one_byte() => offset.min(small.text.len()),
            Store::Small(small) => small.index().ceil(&small.text, offset),
            Store::Large(large) => large.snapshot().ceil_grapheme_boundary(offset),
        }
    }

    /// The boundary before the one at `byte`, or `byte` at the start of the text.
    pub(crate) fn previous_boundary(&self, byte: usize) -> usize {
        if byte == 0 {
            return 0;
        }
        match &self.store {
            Store::Small(small) if small.is_one_byte() => byte - 1,
            Store::Small(small) => {
                let before = small.index().grapheme_of(&small.text, byte);
                small.index().byte_of(&small.text, before.saturating_sub(1))
            }
            Store::Large(large) => large.snapshot().prev_grapheme_boundary(byte),
        }
    }

    /// The boundary after the one at `byte`, or `byte` at the end of the text.
    pub(crate) fn next_boundary(&self, byte: usize) -> usize {
        if byte >= self.len() {
            return self.len();
        }
        match &self.store {
            Store::Small(small) if small.is_one_byte() => byte + 1,
            Store::Small(small) => {
                let before = small.index().grapheme_of(&small.text, byte);
                small.index().byte_of(&small.text, before + 1)
            }
            Store::Large(large) => large.snapshot().next_grapheme_boundary(byte),
        }
    }

    /// The value whose text is this one's with the bytes `range` replaced by `replacement`, and
    /// whose selection and composing range are as given (made safe on the new text).
    ///
    /// A short text is written once into the buffer the new one keeps, with the graphemes of this one
    /// handed on to it, so an edit costs the one copy of the text and what the edit touches. A long
    /// text is edited along one path of its tree and shares the rest.
    pub(crate) fn splice(
        &self,
        range: std::ops::Range<usize>,
        replacement: &str,
        selection: impl FnOnce(usize) -> TextSelection,
        composing: Option<TextRange>,
    ) -> Self {
        let end = range.start + replacement.len();
        let store = match &self.store {
            Store::Small(small) => {
                let new_len = small.text.len() - range.len() + replacement.len();
                if new_len >= large_text_bytes() {
                    // The replacement goes into the chunks once: joining the three parts into a
                    // string first would hold the new text twice as it is made.
                    let snapshot = TextSnapshot::from_text(&small.text).with_replaced(range.clone(), replacement);
                    Store::Large(LargeText::from_snapshot(snapshot))
                } else {
                    let text = concat_text(&[&small.text[..range.start], replacement, &small.text[range.end..]]);
                    let next = SmallText::new(text);
                    // Text that was one byte a grapheme stays so, with one-byte text put in it.
                    if small.is_one_byte() && graphemes::is_one_byte_graphemes(replacement) {
                        let _ = next.one_byte.set(true);
                    }
                    if !next.is_one_byte() {
                        let index = small.index().spliced(&next.text, range.clone(), replacement.len());
                        let _ = next.graphemes.set(Arc::new(index));
                    }
                    Store::Small(next)
                }
            }
            Store::Large(large) => {
                let new_len = large.len() - range.len() + replacement.len();
                if new_len < large_text_bytes() / 2 {
                    let mut text = String::with_capacity(new_len);
                    text.push_str(&large.snapshot().slice(0..range.start));
                    text.push_str(replacement);
                    text.push_str(&large.snapshot().slice(range.end..large.len()));
                    Store::Small(SmallText::new(Arc::from(text)))
                } else {
                    Store::Large(large.spliced(range, replacement))
                }
            }
        };
        Self::edited(store, end, selection, composing)
    }

    /// [`splice`](Self::splice) with a replacement that is already a snapshot. A long result shares
    /// the chunks of `with` and of this text, so pasting a text of megabytes copies none of it; a
    /// short one is made as `splice` makes it.
    pub(crate) fn splice_snapshot(
        &self,
        range: std::ops::Range<usize>,
        with: &TextSnapshot,
        selection: impl FnOnce(usize) -> TextSelection,
    ) -> Self {
        let new_len = self.len() - range.len() + with.len();
        let store = match &self.store {
            Store::Small(small) if new_len >= large_text_bytes() => {
                let text = TextSnapshot::from_text(&small.text).with_replaced_snapshot(range.clone(), with);
                Store::Large(LargeText::from_snapshot(text))
            }
            Store::Large(large) if new_len >= large_text_bytes() / 2 => Store::Large(large.spliced_snapshot(range.clone(), with)),
            _ => return self.splice(range, &with.slice(0..with.len()), selection, None),
        };
        Self::edited(store, range.start + with.len(), selection, None)
    }

    /// The value with `store` as its text, the selection `selection` makes of the end of what was put
    /// in, and `composing`, all on boundaries of the new text.
    fn edited(
        store: Store,
        end: usize,
        selection: impl FnOnce(usize) -> TextSelection,
        composing: Option<TextRange>,
    ) -> Self {
        let mut value = Self { store, selection: TextSelection::collapsed(0), composing: None };
        let selection = selection(end);
        value.selection = TextSelection::new(
            value.floor_to_boundary(selection.anchor()),
            value.floor_to_boundary(selection.focus()),
        );
        value.composing = composing.map(|range| {
            TextRange::new(value.floor_to_boundary(range.start()), value.ceil_to_boundary(range.end()))
        });
        value
    }

    /// The pieces of the text in `range`, in order, each borrowed: the whole range for a short
    /// text, and the chunks it spans for a long one. Nothing is copied.
    pub(crate) fn pieces(&self, range: std::ops::Range<usize>) -> Vec<&str> {
        match &self.store {
            Store::Small(small) => vec![&small.text[range]],
            Store::Large(large) => large.snapshot().chunks(range).collect(),
        }
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

#[cfg(test)]
mod with_str_tests {
    use super::*;

    #[test]
    fn a_borrowed_text_makes_the_value_an_owned_one_does_with_the_caret_at_its_end() {
        for text in ["", "short", "caf\u{e9} \u{1f468}\u{200d}\u{1f469}", &"long text. ".repeat(900)] {
            let borrowed = TextEditingValue::with_str(text);
            let owned = TextEditingValue::with_text(text);
            assert_eq!(borrowed, owned, "{} bytes", text.len());
            assert_eq!(borrowed.selection().range(), text.len()..text.len());
            assert_eq!(*borrowed.text(), *text);
        }
    }
}
