use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use aimer_cupid::font::TextLanguage;
use aimer_text::TextSelection;
use aimer_text_buffer::TextSnapshot;
use unicode_segmentation::UnicodeSegmentation;

use super::TextEditingValue;
use super::editing;

struct ControllerState {
    value: TextEditingValue,
    revision: u64,
    attachments: Rc<[AttachmentEntry]>,
    undo: Vec<TextEditingValue>,
    redo: Vec<TextEditingValue>,
    composition_origin: Option<TextEditingValue>,
}

type AttachmentCallback = Rc<dyn Fn(&TextEditingValue, u64)>;
type AttachmentEntry = (u64, AttachmentCallback);

struct ControllerCore {
    state: RefCell<ControllerState>,
    next_attachment: Cell<u64>,
    input_language: Cell<Option<TextLanguage>>,
}

/// Owns a UI-thread-local editable value and its transaction revision.
///
/// Clones refer to the same controller state. [`value`](Self::value) returns an
/// immutable snapshot whose contents remain unchanged after later controller
/// updates. The controller intentionally uses [`Rc`] rather than thread-safe
/// interior mutability because Aimer widget state belongs to the UI thread.
#[derive(Clone)]
pub struct TextEditingController {
    core: Rc<ControllerCore>,
}

pub(crate) struct ControllerAttachment {
    core: Weak<ControllerCore>,
    id: u64,
}

/// Keeps a [`TextEditingController::listen`] callback registered: dropping it stops the calls.
#[must_use = "dropping the listener stops it"]
pub struct ControllerListener {
    _attachment: ControllerAttachment,
}

impl TextEditingController {
    const MAX_UNDO_DEPTH: usize = 200;
    /// Creates an empty controller with a collapsed selection.
    #[inline]
    pub fn new() -> Self {
        Self::with_value(TextEditingValue::default())
    }

    /// Creates a controller whose caret starts at the end of `text`.
    #[inline]
    pub fn with_text(text: impl Into<String>) -> Self {
        Self::with_value(TextEditingValue::with_text(text))
    }

    /// [`with_text`](Self::with_text) for a text that is only borrowed: a long one is put straight
    /// into the controller's tree of chunks, with no `String` made of it first.
    #[inline]
    pub fn with_str(text: &str) -> Self {
        Self::with_value(TextEditingValue::with_str(text))
    }

    fn with_value(value: TextEditingValue) -> Self {
        Self {
            core: Rc::new(ControllerCore {
                state: RefCell::new(ControllerState {
                    value,
                    revision: 0,
                    attachments: Rc::from([]),
                    undo: Vec::new(),
                    redo: Vec::new(),
                    composition_origin: None,
                }),
                next_attachment: Cell::new(0),
                input_language: Cell::new(None),
            }),
        }
    }

    /// Returns the language of the keyboard this text was typed with.
    ///
    /// Chinese, Japanese and Korean share the Han ideographs but not their
    /// shapes: the same code point is drawn differently by a Chinese face
    /// (PingFang) and a Japanese one (Hiragino). Text alone cannot decide
    /// between them — a run whose every character also exists in Japanese is
    /// indistinguishable from Japanese — so a Chinese field renders in a
    /// Japanese face until a simplified-only character lands in it and makes
    /// the whole field jump to another typeface mid-sentence. The keyboard the
    /// user typed with is the missing evidence, and this is where a field
    /// keeps it, to hand to the renderer as the script hint of every run it
    /// draws.
    ///
    /// The value lives on the controller rather than on the element because
    /// the controller is the retained side of a text field: it survives
    /// widget rebuilds, and it keeps the language while the field is
    /// unfocused, when no keyboard exists to ask.
    ///
    /// Platforms that do not report a keyboard language answer `None`, which
    /// leaves the renderer with its content-only heuristic.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextEditingController;
    ///
    /// let controller = TextEditingController::with_text("\u{4f60}\u{597d}");
    /// assert_eq!(controller.input_language(), None);
    /// ```
    #[inline]
    pub fn input_language(&self) -> Option<TextLanguage> {
        self.core.input_language.get()
    }

    /// Records the language of the keyboard that produced the latest edit.
    ///
    /// `tag` is an IETF language tag as the platform reports it — `"zh-Hans"`,
    /// `"ja-JP"` — or `None` when the platform has nothing to say. Only a tag
    /// naming one of the Han-sharing languages
    /// ([`TextLanguage::from_tag`]) is remembered; everything else, `None`
    /// included, leaves the previously learned language in place.
    ///
    /// The asymmetry is deliberate. A field is typed into with several
    /// keyboards — the user switches to the Latin one for a name, a URL or a
    /// digit and back again — and each switch would otherwise erase the only
    /// evidence of which Han face the text wants. A Latin keyboard writes no
    /// ideographs, so it can never contradict the Chinese, Japanese or Korean
    /// keyboard that wrote them; forgetting on its account would make the
    /// field flip typeface for typing a full stop. The one thing that does
    /// overrule the memory is another CJK keyboard, which is the user saying
    /// the field is now in that language.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextEditingController;
    ///
    /// let controller = TextEditingController::new();
    /// controller.learn_input_language_tag(Some("zh-Hans"));
    /// controller.learn_input_language_tag(Some("en-US"));
    /// assert!(controller.input_language().is_some());
    /// ```
    #[inline]
    pub fn learn_input_language_tag(&self, tag: Option<&str>) {
        if let Some(language) = tag.and_then(TextLanguage::from_tag) {
            self.core.input_language.set(Some(language));
        }
    }

    /// Returns the current immutable editing snapshot.
    #[inline]
    pub fn value(&self) -> TextEditingValue {
        self.core.state.borrow().value.clone()
    }

    /// Replaces the complete editing value as one transaction.
    pub fn set_value(&self, value: TextEditingValue) {
        let previous = self.value();
        self.core.state.borrow_mut().composition_origin = None;
        self.apply_value(value, Some(previous));
    }

    fn apply_value(&self, value: TextEditingValue, history: Option<TextEditingValue>) -> bool {
        let (revision, attachments) = {
            let mut state = self.core.state.borrow_mut();
            if state.value == value {
                return false;
            }
            if let Some(previous) = history {
                state.undo.push(previous);
                if state.undo.len() > Self::MAX_UNDO_DEPTH {
                    state.undo.remove(0);
                }
                state.redo.clear();
            }
            state.value = value;
            state.revision = state
                .revision
                .checked_add(1)
                .expect("exhausted all text editing revisions");
            let attachments = state.attachments.clone();
            (state.revision, attachments)
        };
        let value = self.value();
        for (_, attachment) in attachments.iter() {
            attachment(&value, revision);
        }
        true
    }

    pub(crate) fn replace_selection_graphemes(
        &self,
        anchor: usize,
        focus: usize,
        replacement: &str,
        max_length: Option<usize>,
    ) -> bool {
        let value = self.value();
        let selected = value.with_selection(TextSelection::new(
            value.byte_of_grapheme(anchor),
            value.byte_of_grapheme(focus),
        ));
        self.apply_value(
            editing::replace_selection(&selected, replacement, max_length),
            Some(value),
        )
    }

    /// Where the caret is, as the one-based line and the one-based column (in graphemes, from the
    /// start of the line) a status bar shows: `32:23`. For a selection it is where the selection
    /// ends, the end the caret is at.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextEditingController;
    ///
    /// let controller = TextEditingController::with_text("one\ntwo");
    /// assert_eq!(controller.cursor_position(), (2, 4), "the caret is at the end of the second line");
    /// ```
    pub fn cursor_position(&self) -> (usize, usize) {
        let value = self.value();
        let (line, column) = value.line_and_column(value.selection().focus());
        (line + 1, column + 1)
    }

    /// The text as a snapshot, which never changes whatever is typed after: see
    /// [`TextEditingValue::snapshot`].
    pub fn snapshot(&self) -> TextSnapshot {
        self.value().snapshot()
    }

    /// How many lines the text has: one more than its line feeds.
    pub fn line_count(&self) -> usize {
        self.value().line_count()
    }

    /// [`replace_selection_graphemes`](Self::replace_selection_graphemes) with a replacement that is
    /// a snapshot, shared and not copied. There is no length limit to apply to one: the caller gives
    /// a text that is to go in whole.
    pub(crate) fn replace_selection_graphemes_snapshot(
        &self,
        anchor: usize,
        focus: usize,
        with: &TextSnapshot,
    ) -> bool {
        let value = self.value();
        let selected = value.with_selection(TextSelection::new(
            value.byte_of_grapheme(anchor),
            value.byte_of_grapheme(focus),
        ));
        let range = selected.selection().range();
        self.apply_value(selected.splice_snapshot(range, with, TextSelection::collapsed), Some(value))
    }

    pub(crate) fn delete_backward_graphemes(&self, anchor: usize, focus: usize) -> bool {
        self.edit_from_graphemes(anchor, focus, editing::delete_backward, true)
    }

    pub(crate) fn delete_forward_graphemes(&self, anchor: usize, focus: usize) -> bool {
        self.edit_from_graphemes(anchor, focus, editing::delete_forward, true)
    }

    pub(crate) fn move_left_graphemes(&self, anchor: usize, focus: usize, extend: bool) -> bool {
        self.edit_from_graphemes(
            anchor,
            focus,
            |value| editing::move_left(value, extend),
            false,
        )
    }

    pub(crate) fn move_right_graphemes(&self, anchor: usize, focus: usize, extend: bool) -> bool {
        self.edit_from_graphemes(
            anchor,
            focus,
            |value| editing::move_right(value, extend),
            false,
        )
    }

    pub(crate) fn update_composing_graphemes(
        &self,
        anchor: usize,
        focus: usize,
        preedit: &str,
    ) -> bool {
        if preedit.is_empty() {
            return self.cancel_composing();
        }
        let current = self.value();
        let selected = if current.composing().is_some() {
            current.clone()
        } else {
            current.with_selection(TextSelection::new(
                current.byte_of_grapheme(anchor),
                current.byte_of_grapheme(focus),
            ))
        };
        {
            let mut state = self.core.state.borrow_mut();
            if state.composition_origin.is_none() {
                state.composition_origin = Some(selected.clone());
            }
        }
        self.apply_value(editing::update_composing(&selected, preedit), None)
    }

    pub(crate) fn commit_composing(&self, committed: &str, max_length: Option<usize>) -> bool {
        let origin = self.core.state.borrow_mut().composition_origin.take();
        let origin = origin.unwrap_or_else(|| self.value());
        let value = editing::replace_selection(&origin, committed, max_length);
        self.apply_value(value, Some(origin))
    }

    pub(crate) fn cancel_composing(&self) -> bool {
        let origin = self.core.state.borrow_mut().composition_origin.take();
        origin.is_some_and(|origin| self.apply_value(origin, None))
    }

    pub(crate) fn apply_native_value(&self, value: TextEditingValue) -> bool {
        let current = self.value();
        let history = if value.composing().is_some() {
            let mut state = self.core.state.borrow_mut();
            if state.composition_origin.is_none() {
                state.composition_origin = Some(current);
            }
            None
        } else {
            self.core
                .state
                .borrow_mut()
                .composition_origin
                .take()
                .or_else(|| (!current.same_text(&value)).then_some(current))
        };
        self.apply_value(value, history)
    }

    /// Restores the complete editing value before the most recent transaction.
    pub fn undo(&self) -> bool {
        let Some((value, revision, attachments)) = self.restore_history(true) else {
            return false;
        };
        notify_attachments(&attachments, &value, revision);
        true
    }

    /// Reapplies the complete editing value most recently restored by undo.
    pub fn redo(&self) -> bool {
        let Some((value, revision, attachments)) = self.restore_history(false) else {
            return false;
        };
        notify_attachments(&attachments, &value, revision);
        true
    }

    /// Replaces the text while preserving and safely clamping the selection.
    pub fn set_text(&self, text: impl Into<String>) {
        let selection = self.value().selection();
        self.set_value(TextEditingValue::new(text, selection, None));
    }

    /// Replaces the UTF-8 byte `range` of the text with `replacement` as one undoable edit, and puts
    /// the caret after the replacement. A code editor completes the word before the caret this way.
    ///
    /// The range is clamped to the text and widened to whole graphemes, so it never splits a
    /// character. It costs the edit, not the text: a long text is edited along one path of its tree.
    /// Returns whether the text or the caret changed.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextEditingController;
    ///
    /// let controller = TextEditingController::with_text("let x = pu;");
    /// controller.replace_range(8..10, "pub");
    /// assert_eq!(controller.value().text(), "let x = pub;");
    /// ```
    pub fn replace_range(&self, range: std::ops::Range<usize>, replacement: &str) -> bool {
        let value = self.value();
        let start = value.floor_to_boundary(range.start.min(value.len()));
        let end = value.ceil_to_boundary(range.end.min(value.len())).max(start);
        let edited = value.splice(start..end, replacement, TextSelection::collapsed, None);
        self.core.state.borrow_mut().composition_origin = None;
        self.apply_value(edited, Some(value))
    }

    /// Replaces several ranges of bytes at once, as one edit that one undo takes back, and puts the
    /// caret after the replacement of the `caret_after`-th range of them in the order they have in the
    /// text, wherever the others have moved it. The ranges are given in any order and must not overlap
    /// (they may touch); each is made whole characters, and one past the end is the end. Returns
    /// whether the text changed: nothing does when they overlap or there are none.
    ///
    /// This is for what a language server asks for when an item is completed: the word, and the line
    /// that imports it somewhere else.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextEditingController;
    ///
    /// let controller = TextEditingController::with_text("let m = Ma;");
    /// controller.replace_ranges(&[(8..10, "Map"), (0..0, "use a::Map;\n")], 1);
    /// assert_eq!(controller.value().text(), "use a::Map;\nlet m = Map;");
    /// assert!(controller.undo());
    /// assert_eq!(controller.value().text(), "let m = Ma;");
    /// ```
    pub fn replace_ranges(&self, edits: &[(std::ops::Range<usize>, &str)], caret_after: usize) -> bool {
        let value = self.value();
        let mut list: Vec<(usize, usize, &str)> = edits
            .iter()
            .map(|(range, text)| {
                let start = value.floor_to_boundary(range.start.min(value.len()));
                let end = value.ceil_to_boundary(range.end.min(value.len())).max(start);
                (start, end, *text)
            })
            .collect();
        if list.is_empty() {
            return false;
        }
        list.sort_by_key(|&(start, end, _)| (start, end));
        if list.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return false;
        }
        let focus = caret_after.min(list.len() - 1);
        // Where the caret is once the ranges before it are replaced: each moves it by how much longer or
        // shorter its replacement is.
        let moved: isize = list[..focus].iter().map(|&(start, end, text)| text.len() as isize - (end - start) as isize).sum();
        let caret = (list[focus].0 + list[focus].2.len()).checked_add_signed(moved).expect("a caret is not before the start of the text");
        let mut edited = value.clone();
        // From the end, so that a range is not moved by the ones after it.
        for &(start, end, text) in list.iter().rev() {
            edited = edited.splice(start..end, text, TextSelection::collapsed, None);
        }
        let edited = edited.with_selection(TextSelection::collapsed(caret));
        self.core.state.borrow_mut().composition_origin = None;
        self.apply_value(edited, Some(value))
    }

    /// Removes all text and collapses the selection at zero.
    #[inline]
    pub fn clear(&self) {
        self.set_value(TextEditingValue::default());
    }

    /// Returns the monotonically increasing transaction revision.
    #[inline]
    pub fn revision(&self) -> u64 {
        self.core.state.borrow().revision
    }

    /// Advances the revision without changing the value.
    ///
    /// The revision orders the snapshots a field pushes to a platform text
    /// editor against the deltas the editor reports back. An input that is
    /// consumed without producing a transaction — the Return key submitting a
    /// single-line field, or a delta that describes the state the controller
    /// already holds — still has to outrank the revision the editor advanced
    /// speculatively, otherwise the snapshot that re-anchors the editor is
    /// discarded as stale.
    pub(crate) fn bump_revision(&self) {
        let (value, revision, attachments) = {
            let mut state = self.core.state.borrow_mut();
            state.revision = state
                .revision
                .checked_add(1)
                .expect("exhausted all text editing revisions");
            (
                state.value.clone(),
                state.revision,
                state.attachments.clone(),
            )
        };
        notify_attachments(&attachments, &value, revision);
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn with_initial(text: impl Into<String>) -> Self {
        Self::with_text(text)
    }

    #[inline]
    pub(crate) fn text(&self) -> String {
        self.value().text().into_owned()
    }

    /// Whether the text is empty, without a copy of it to ask.
    #[inline]
    pub(crate) fn is_text_empty(&self) -> bool {
        self.core.state.borrow().value.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn take(&self) -> String {
        let text = self.text();
        self.clear();
        text
    }

    #[cfg(test)]
    pub(crate) unsafe fn insert_char(&self, ch: impl Into<char>, offset: usize) {
        let mut encoded = [0; 4];
        self.insert_str(ch.into().encode_utf8(&mut encoded), offset);
    }

    pub(crate) fn grapheme_count(&self) -> usize {
        self.value().grapheme_count()
    }

    pub(crate) fn selection_graphemes(&self) -> (usize, usize) {
        let value = self.value();
        (
            value.grapheme_of_byte(value.selection().anchor()),
            value.grapheme_of_byte(value.selection().focus()),
        )
    }

    /// Selects from the grapheme offset `anchor` to `focus`, which is a caret where they are the
    /// same. Offsets past the end of the text are the end. Returns whether the selection changed.
    /// The caret of a controller made with text is at its end; this is how a file is opened at its
    /// start.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextEditingController;
    ///
    /// let controller = TextEditingController::with_text("one\ntwo");
    /// controller.select(0, 0);
    /// assert_eq!(controller.cursor_position(), (1, 1));
    /// controller.select(0, 5);
    /// assert_eq!(controller.cursor_position(), (2, 2));
    /// ```
    pub fn select(&self, anchor: usize, focus: usize) -> bool {
        self.set_selection_graphemes(anchor, focus)
    }

    /// Selects from the byte offset `anchor` to `focus`, which is a caret where they are the same. An
    /// offset past the end is the end, and one inside a character or a grapheme (a letter and its
    /// mark are one) is moved back to the start of it, so a selection never cuts either. Returns
    /// whether the selection changed. It is not an edit, and the undo history is as it was.
    ///
    /// This is [`select`](Self::select) for a caller that has byte offsets of the text, such as the
    /// ranges a compiler or a language server reports, and not offsets of graphemes.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextEditingController;
    ///
    /// let controller = TextEditingController::with_text("caf\u{e9}\nbar");
    /// // "bar" starts at byte 6: "é" is two bytes.
    /// controller.select_bytes(6, 9);
    /// assert_eq!(controller.cursor_position(), (2, 4));
    /// ```
    pub fn select_bytes(&self, anchor: usize, focus: usize) -> bool {
        let value = self.value();
        let len = value.len();
        // `with_selection` brings each end back to a grapheme boundary.
        let selection = TextSelection::new(anchor.min(len), focus.min(len));
        self.apply_value(value.with_selection(selection), None)
    }

    pub(crate) fn set_selection_graphemes(&self, anchor: usize, focus: usize) -> bool {
        let value = self.value();
        let selection = TextSelection::new(
            value.byte_of_grapheme(anchor),
            value.byte_of_grapheme(focus),
        );
        self.apply_value(value.with_selection(selection), None)
    }

    fn edit_from_graphemes(
        &self,
        anchor: usize,
        focus: usize,
        edit: impl FnOnce(&TextEditingValue) -> TextEditingValue,
        record_history: bool,
    ) -> bool {
        let value = self.value();
        let selected = value.with_selection(TextSelection::new(
            value.byte_of_grapheme(anchor),
            value.byte_of_grapheme(focus),
        ));
        let history = record_history.then_some(value);
        self.apply_value(edit(&selected), history)
    }

    pub(crate) fn get_range(&self, start: usize, end: usize) -> String {
        let value = self.value();
        let byte_start = value.byte_of_grapheme(start);
        let byte_end = value.byte_of_grapheme(start.max(end));
        // The range alone: reading the whole text out to cut a part of it would copy a long one.
        value.slice(byte_start..byte_end).into_owned()
    }

    #[cfg(test)]
    pub(crate) fn delete_grapheme(&self, offset: usize) -> String {
        self.delete_range(offset, offset.saturating_add(1))
    }

    #[cfg(test)]
    pub(crate) fn delete_range(&self, start: usize, end: usize) -> String {
        let value = self.value();
        let byte_start = value.byte_of_grapheme(start);
        let byte_end = value.byte_of_grapheme(start.max(end));
        let removed = value.text()[byte_start..byte_end].to_owned();
        if byte_start == byte_end {
            return removed;
        }
        let selected = value.with_selection(TextSelection::new(byte_start, byte_end));
        self.apply_value(editing::replace_selection(&selected, "", None), Some(value));
        removed
    }

    #[cfg(test)]
    pub(crate) fn insert_str(&self, text: &str, offset: usize) {
        let value = self.value();
        let byte_offset = value.byte_of_grapheme(offset);
        let selected = value.with_selection(TextSelection::collapsed(byte_offset));
        self.apply_value(
            editing::replace_selection(&selected, text, None),
            Some(value),
        );
    }

    /// Calls `callback` with the new value after every change to the controller: an edit, a move of the
    /// caret or the selection, an undo, a text set from outside. It is how a status bar follows the
    /// caret, or a view follows the text, without being the field that edits it.
    ///
    /// The call is made after the change is complete, so the callback may read the controller and
    /// change it again; keep it quick, since it runs on each keystroke. The returned guard keeps the
    /// callback registered for as long as it is held.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use aimer_input::TextEditingController;
    ///
    /// let controller = TextEditingController::new();
    /// let changes = Rc::new(Cell::new(0));
    /// let counted = Rc::clone(&changes);
    /// let _listener = controller.listen(move |_value| counted.set(counted.get() + 1));
    ///
    /// controller.set_text("hello");
    /// assert_eq!(changes.get(), 1);
    /// ```
    pub fn listen(&self, callback: impl Fn(&TextEditingValue) + 'static) -> ControllerListener {
        ControllerListener { _attachment: self.attach(move |value, _revision| callback(value)) }
    }

    pub(crate) fn attach(
        &self,
        callback: impl Fn(&TextEditingValue, u64) + 'static,
    ) -> ControllerAttachment {
        let id = self
            .core
            .next_attachment
            .get()
            .checked_add(1)
            .expect("exhausted all text editor attachment identities");
        self.core.next_attachment.set(id);
        let mut state = self.core.state.borrow_mut();
        let mut attachments = Vec::with_capacity(state.attachments.len() + 1);
        attachments.extend(state.attachments.iter().cloned());
        attachments.push((id, Rc::new(callback)));
        state.attachments = attachments.into();
        drop(state);
        ControllerAttachment {
            core: Rc::downgrade(&self.core),
            id,
        }
    }

    fn restore_history(
        &self,
        undo: bool,
    ) -> Option<(TextEditingValue, u64, Rc<[AttachmentEntry]>)> {
        let mut state = self.core.state.borrow_mut();
        let restored = if undo {
            state.undo.pop()
        } else {
            state.redo.pop()
        }?;
        let current = std::mem::replace(&mut state.value, restored);
        if undo {
            state.redo.push(current);
        } else {
            state.undo.push(current);
        }
        state.revision = state
            .revision
            .checked_add(1)
            .expect("exhausted all text editing revisions");
        Some((
            state.value.clone(),
            state.revision,
            state.attachments.clone(),
        ))
    }
}

fn notify_attachments(
    attachments: &[(u64, AttachmentCallback)],
    value: &TextEditingValue,
    revision: u64,
) {
    for (_, attachment) in attachments {
        attachment(value, revision);
    }
}

#[cfg(test)]
mod tests {
    use aimer_cupid::font::TextLanguage;

    use super::TextEditingController;

    #[test]
    fn several_ranges_are_replaced_at_once_as_one_edit_that_one_undo_takes_back() {
        // A word is completed and the line that imports it is added at the top, as a server asks.
        let controller = TextEditingController::with_text("fn main() {\n    HashM\n}\n");
        let at = controller.value().text().find("HashM").unwrap();
        let word = at..at + 5;
        assert!(controller.replace_ranges(&[(word, "HashMap"), (0..0, "use std::collections::HashMap;\n\n")], 1));
        assert_eq!(
            controller.value().text(),
            "use std::collections::HashMap;\n\nfn main() {\n    HashMap\n}\n",
            "the later range was replaced first, so the earlier one was not moved"
        );
        let end = controller.value().text().find("HashMap\n}").unwrap() + "HashMap".len();
        assert_eq!(controller.value().selection().focus(), end, "the caret is after the word, wherever the other edit put it");
        assert_eq!(controller.value().selection().anchor(), end);
        assert!(controller.undo());
        assert_eq!(controller.value().text(), "fn main() {\n    HashM\n}\n", "both edits are taken back by one undo");
        assert!(!controller.undo());
    }

    #[test]
    fn ranges_that_overlap_or_none_at_all_change_nothing() {
        let controller = TextEditingController::with_text("abcdef");
        assert!(!controller.replace_ranges(&[(1..4, "x"), (3..5, "y")], 0), "they overlap");
        assert!(!controller.replace_ranges(&[], 0), "no edit is no change");
        assert_eq!(controller.value().text(), "abcdef");
        assert!(!controller.undo(), "and nothing was recorded");
        // Ranges that only touch are not overlapping, and are given in any order.
        assert!(controller.replace_ranges(&[(3..6, "Z"), (0..3, "Y")], 1));
        assert_eq!(controller.value().text(), "YZ");
        assert_eq!(controller.value().selection().focus(), 2, "after the replacement of the range that is second in the text");
    }

    #[test]
    fn ranges_that_are_in_a_character_or_past_the_end_are_made_whole() {
        let controller = TextEditingController::with_text("\u{e9}b");
        // Byte 1 is inside the accent, which is two bytes.
        assert!(controller.replace_ranges(&[(1..3, "X"), (50..60, "!")], 0));
        assert_eq!(controller.value().text(), "X!", "the accent whole, and the end where past it was said");
    }

    #[test]
    fn a_selection_by_bytes_puts_the_caret_where_the_byte_is_and_counts_in_bytes_not_characters() {
        // "é" is two bytes, "你" three, "😀" four.
        let text = "a\u{e9}\u{4f60}\u{1f600}z\nline two";
        let controller = TextEditingController::with_text(text);
        let at = |needle: &str| text.find(needle).expect("in the text");

        assert!(controller.select_bytes(at("z"), at("z")));
        assert_eq!(controller.cursor_position(), (1, 5), "after four characters on the first line");
        assert_eq!(controller.value().selection().anchor(), at("z"));
        assert_eq!(controller.value().selection().focus(), at("z"));

        assert!(controller.select_bytes(at("line"), at("two")));
        assert_eq!(controller.cursor_position(), (2, 6), "the end of the selection is the caret");
        let selection = controller.value().selection();
        assert_eq!((selection.anchor(), selection.focus()), (at("line"), at("two")));

        // The same place again is not a change.
        assert!(!controller.select_bytes(at("line"), at("two")));
    }

    #[test]
    fn a_byte_that_is_inside_a_character_or_a_grapheme_is_moved_to_the_start_of_it() {
        // "e" and U+0301 are one grapheme: a caret does not go between them. 😀 is four bytes.
        let text = "e\u{301}x\u{1f600}y";
        let controller = TextEditingController::with_text(text);
        controller.select_bytes(1, 1);
        assert_eq!(controller.value().selection().focus(), 0, "between a letter and its mark is before the letter");
        controller.select_bytes(5, 5);
        assert_eq!(controller.value().selection().focus(), 4, "inside the emoji is before it");
        controller.select_bytes(4, 8);
        let selection = controller.value().selection();
        assert_eq!((selection.anchor(), selection.focus()), (4, 8), "whole characters are as they are");
    }

    #[test]
    fn a_byte_past_the_end_is_the_end_and_empty_text_has_only_one_place() {
        let controller = TextEditingController::with_text("abc");
        controller.select_bytes(0, 0);
        assert!(controller.select_bytes(99, 1000));
        assert_eq!((controller.value().selection().anchor(), controller.value().selection().focus()), (3, 3));
        let empty = TextEditingController::new();
        assert!(!empty.select_bytes(5, 9), "the caret of empty text is already where there is to go");
        assert_eq!(empty.value().selection().focus(), 0);
    }

    #[test]
    fn a_selection_by_bytes_is_not_an_edit_and_leaves_the_undo_history_alone() {
        let controller = TextEditingController::with_text("hello");
        controller.set_text("hello world");
        assert!(controller.undo(), "the edit is there to be undone");
        assert!(controller.redo());
        controller.select_bytes(2, 2);
        controller.select_bytes(5, 5);
        // What an undo undoes is still the edit, and not the moves of the caret.
        assert!(controller.undo());
        assert_eq!(controller.value().text(), "hello");
        assert!(!controller.undo(), "nothing else was recorded");
    }

    #[test]
    fn a_range_of_bytes_is_replaced_as_one_edit_the_caret_goes_after() {
        let controller = TextEditingController::with_text("let x = pu;\n");
        assert!(controller.replace_range(8..10, "pub"));
        assert_eq!(controller.value().text(), "let x = pub;\n");
        assert_eq!(controller.value().selection().range(), 11..11, "the caret is after the replacement");
        assert!(!controller.replace_range(11..11, ""), "the same text with the caret where it is is not an edit");
        assert!(controller.undo(), "one undo takes it back");
        assert_eq!(controller.value().text(), "let x = pu;\n");
    }

    #[test]
    fn a_range_is_clamped_to_the_text_and_to_whole_characters() {
        let controller = TextEditingController::with_text("caf\u{e9}!");
        // The range ends inside the two bytes of the accent: it takes the whole character.
        assert!(controller.replace_range(1..4, "-"));
        assert_eq!(controller.value().text(), "c-!");
        assert!(controller.replace_range(1..100, ""), "past the end is the end");
        assert_eq!(controller.value().text(), "c");
    }

    #[test]
    fn controller_starts_without_an_input_language() {
        let controller = TextEditingController::with_text("你好");

        assert_eq!(controller.input_language(), None);
    }

    #[test]
    fn a_chinese_keyboard_tag_is_learned() {
        let controller = TextEditingController::new();

        controller.learn_input_language_tag(Some("zh-Hans"));

        assert_eq!(controller.input_language(), Some(TextLanguage::Chinese));
    }

    #[test]
    fn a_latin_keyboard_tag_learns_nothing_and_erases_nothing() {
        let controller = TextEditingController::new();

        controller.learn_input_language_tag(Some("en-US"));
        assert_eq!(controller.input_language(), None);

        controller.learn_input_language_tag(Some("zh-Hans"));
        controller.learn_input_language_tag(Some("en-US"));
        assert_eq!(controller.input_language(), Some(TextLanguage::Chinese));
    }

    #[test]
    fn an_absent_tag_keeps_the_learned_language() {
        let controller = TextEditingController::new();

        controller.learn_input_language_tag(Some("ja-JP"));
        controller.learn_input_language_tag(None);

        assert_eq!(controller.input_language(), Some(TextLanguage::Japanese));
    }

    #[test]
    fn the_learned_language_is_shared_by_every_clone() {
        let controller = TextEditingController::new();
        let rebuilt = controller.clone();

        controller.learn_input_language_tag(Some("ko-KR"));

        assert_eq!(rebuilt.input_language(), Some(TextLanguage::Korean));
    }

    /// A listener's callback may own what owns another listener of the same controller: closing a
    /// document whose widgets listen to it drops the last callback, which drops the document, which
    /// drops its own listener. That must not happen while the controller's state is borrowed.
    #[test]
    fn dropping_a_listener_whose_callback_owns_another_listener_does_not_reenter_the_controller() {
        let controller = TextEditingController::with_text("a");
        let inner = controller.listen(|_| {});
        let outer = controller.listen({
            let keep = inner;
            move |_| {
                let _ = &keep;
            }
        });
        drop(outer);
        // The controller still works, and the inner listener went with the outer one.
        controller.set_text("b");
        assert_eq!(controller.value().text(), "b");
    }

    #[test]
    fn editing_the_text_never_forgets_the_language() {
        let controller = TextEditingController::with_text("你好");
        controller.learn_input_language_tag(Some("zh-Hant"));

        controller.set_text("");

        assert_eq!(controller.input_language(), Some(TextLanguage::Chinese));
    }
}

impl Drop for ControllerAttachment {
    fn drop(&mut self) {
        if let Some(core) = self.core.upgrade() {
            let replaced = {
                let mut state = core.state.borrow_mut();
                let kept: Rc<[AttachmentEntry]> = state
                    .attachments
                    .iter()
                    .filter(|(id, _)| *id != self.id)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into();
                std::mem::replace(&mut state.attachments, kept)
            };
            // The old list held the last reference to this attachment's callback. The callback may own
            // something that owns another listener of this controller (a document, say), whose drop
            // takes the state again, so it is released after the borrow above has ended.
            drop(replaced);
        }
    }
}

impl Default for TextEditingController {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod oracle_tests {
    use unicode_segmentation::UnicodeSegmentation;

    use super::TextEditingController;

    /// Edits made through the controller, which finds graphemes by an index it hands from edit to
    /// edit, against the same edits made on a plain string by walking it.
    #[test]
    fn edits_agree_with_a_plain_string_walked_from_its_start() {
        const PIECES: [&str; 12] = [
            "a", "b", " ", "\n", "\r\n", "e", "\u{301}", "\u{e9}", "\u{4f60}", "\u{1f468}\u{200d}\u{1f469}", "\u{1f1e6}", "\u{1f1e7}",
        ];
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |below: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % below as u64) as usize
        };
        for round in 0..40 {
            let start: String = (0..200 + next(300)).map(|_| PIECES[next(PIECES.len())]).collect();
            let controller = TextEditingController::with_text(start.clone());
            let mut oracle = start;
            for step in 0..60 {
                let count = oracle.graphemes(true).count();
                assert_eq!(controller.grapheme_count(), count, "round {round} step {step}: count");
                let at = next(count + 1);
                let byte = oracle.grapheme_indices(true).nth(at).map_or(oracle.len(), |(byte, _)| byte);
                match next(3) {
                    0 => {
                        let piece = PIECES[next(PIECES.len())];
                        controller.replace_selection_graphemes(at, at, piece, None);
                        oracle.insert_str(byte, piece);
                    }
                    1 if at > 0 => {
                        controller.delete_backward_graphemes(at, at);
                        let before = oracle.grapheme_indices(true).nth(at - 1).map(|(b, _)| b).unwrap();
                        oracle.replace_range(before..byte, "");
                    }
                    _ if at < count => {
                        controller.delete_forward_graphemes(at, at);
                        let after = oracle.grapheme_indices(true).nth(at + 1).map_or(oracle.len(), |(b, _)| b);
                        oracle.replace_range(byte..after, "");
                    }
                    _ => {}
                }
                assert_eq!(controller.text(), oracle, "round {round} step {step}");
                let (anchor, focus) = controller.selection_graphemes();
                assert_eq!(anchor, focus);
                let caret = controller.value().selection().focus();
                assert_eq!(oracle[..caret].graphemes(true).count(), focus, "round {round} step {step}: caret");
            }
        }
    }
}

/// What each of the controller's operations costs on a long text, which is what typing in a long
/// note asks of it, one key at a time.
///
/// ```text
/// cargo nextest run -p aimer_input --run-ignored only -E 'test(bench_the_controller)' --no-capture
/// ```
#[cfg(test)]
mod bench_tests {
    use std::time::{Duration, Instant};

    use super::TextEditingController;

    fn timed(runs: usize, mut run: impl FnMut()) -> Duration {
        let started = Instant::now();
        for _ in 0..runs {
            run();
        }
        started.elapsed() / runs as u32
    }

    #[test]
    #[ignore = "a timing, not a check"]
    fn bench_the_controller() {
        let latin = "The quick brown fox jumps over the lazy dog. ".repeat(7000);
        let mixed = "Привет мир, 你好世界 \u{1780}\u{17D2}\u{1794}\u{17BB}\u{1798} e\u{301} caf\u{e9} \u{1f468}\u{200d}\u{1f469}. ".repeat(3500);
        for (name, text) in [("latin", latin), ("mixed", mixed)] {
            let controller = TextEditingController::with_text(text.clone());
            let graphemes = controller.grapheme_count();
            let middle = graphemes / 2;
            let kilobytes = text.len() / 1000;
            let line = |what: &str, time: Duration| println!("{kilobytes:>4} KB {name:<6} {what:<28} {time:>10.2?}");

            line("type a character", timed(20, || {
                controller.replace_selection_graphemes(middle, middle, "x", None);
            }));
            line("backspace", timed(20, || {
                controller.delete_backward_graphemes(middle + 1, middle + 1);
            }));
            line("move the caret", timed(20, || {
                controller.set_selection_graphemes(middle, middle);
                controller.set_selection_graphemes(middle + 1, middle + 1);
            }) / 2);
            line("selection_graphemes", timed(20, || {
                let _ = controller.selection_graphemes();
            }));
            line("grapheme_count", timed(20, || {
                let _ = controller.grapheme_count();
            }));
            line("text() copy", timed(20, || {
                let _ = controller.text();
            }));
        }
    }
}
