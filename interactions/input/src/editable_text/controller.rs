use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use aimer_cupid::font::TextLanguage;
use aimer_text::TextSelection;
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
            grapheme_byte_offset(value.text(), anchor),
            grapheme_byte_offset(value.text(), focus),
        ));
        self.apply_value(
            editing::replace_selection(&selected, replacement, max_length),
            Some(value),
        )
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
                grapheme_byte_offset(current.text(), anchor),
                grapheme_byte_offset(current.text(), focus),
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
                .or_else(|| (current.text() != value.text()).then_some(current))
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
        self.value().text().to_owned()
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
        self.value().text().graphemes(true).count()
    }

    pub(crate) fn selection_graphemes(&self) -> (usize, usize) {
        let value = self.value();
        (
            value.text()[..value.selection().anchor()]
                .graphemes(true)
                .count(),
            value.text()[..value.selection().focus()]
                .graphemes(true)
                .count(),
        )
    }

    pub(crate) fn set_selection_graphemes(&self, anchor: usize, focus: usize) -> bool {
        let value = self.value();
        let selection = TextSelection::new(
            grapheme_byte_offset(value.text(), anchor),
            grapheme_byte_offset(value.text(), focus),
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
            grapheme_byte_offset(value.text(), anchor),
            grapheme_byte_offset(value.text(), focus),
        ));
        let history = record_history.then_some(value);
        self.apply_value(edit(&selected), history)
    }

    pub(crate) fn get_range(&self, start: usize, end: usize) -> String {
        let value = self.value();
        let byte_start = grapheme_byte_offset(value.text(), start);
        let byte_end = grapheme_byte_offset(value.text(), start.max(end));
        value.text()[byte_start..byte_end].to_owned()
    }

    #[cfg(test)]
    pub(crate) fn delete_grapheme(&self, offset: usize) -> String {
        self.delete_range(offset, offset.saturating_add(1))
    }

    #[cfg(test)]
    pub(crate) fn delete_range(&self, start: usize, end: usize) -> String {
        let value = self.value();
        let byte_start = grapheme_byte_offset(value.text(), start);
        let byte_end = grapheme_byte_offset(value.text(), start.max(end));
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
        let byte_offset = grapheme_byte_offset(value.text(), offset);
        let selected = value.with_selection(TextSelection::collapsed(byte_offset));
        self.apply_value(
            editing::replace_selection(&selected, text, None),
            Some(value),
        );
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

fn grapheme_byte_offset(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .nth(offset)
        .map_or(text.len(), |(byte, _)| byte)
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
            let mut state = core.state.borrow_mut();
            state.attachments = state
                .attachments
                .iter()
                .filter(|(id, _)| *id != self.id)
                .cloned()
                .collect::<Vec<_>>()
                .into();
        }
    }
}

impl Default for TextEditingController {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
