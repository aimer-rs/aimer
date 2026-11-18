use aimer_text::TextSelection;
use unicode_segmentation::UnicodeSegmentation;

use super::{TextEditingValue, TextRange};

pub(crate) fn replace_selection(
    value: &TextEditingValue,
    replacement: &str,
    max_length: Option<usize>,
) -> TextEditingValue {
    let range = value.selection().range();
    let replacement = limited_replacement(value, range.clone(), replacement, max_length);
    replace_range(value, range, &replacement)
}

pub(crate) fn delete_backward(value: &TextEditingValue) -> TextEditingValue {
    let selection = value.selection().range();
    if !selection.is_empty() {
        return replace_range(value, selection, "");
    }
    let caret = selection.start;
    if caret == 0 {
        return value.clone();
    }
    replace_range(value, value.previous_boundary(caret)..caret, "")
}

pub(crate) fn delete_forward(value: &TextEditingValue) -> TextEditingValue {
    let selection = value.selection().range();
    if !selection.is_empty() {
        return replace_range(value, selection, "");
    }
    let caret = selection.start;
    if caret >= value.len() {
        return value.clone();
    }
    replace_range(value, caret..value.next_boundary(caret), "")
}

pub(crate) fn move_left(value: &TextEditingValue, extend: bool) -> TextEditingValue {
    let selection = value.selection();
    let target = if !extend && !selection.is_collapsed() {
        selection.range().start
    } else {
        value.previous_boundary(selection.focus())
    };
    moved_selection(value, target, extend)
}

pub(crate) fn move_right(value: &TextEditingValue, extend: bool) -> TextEditingValue {
    let selection = value.selection();
    let target = if !extend && !selection.is_collapsed() {
        selection.range().end
    } else {
        value.next_boundary(selection.focus())
    };
    moved_selection(value, target, extend)
}

pub(crate) fn update_composing(value: &TextEditingValue, preedit: &str) -> TextEditingValue {
    let range = value.composing().map_or_else(
        || value.selection().range(),
        |range| range.start()..range.end(),
    );
    let start = range.start;
    let end = start + preedit.len();
    value.splice(
        range,
        preedit,
        |_| TextSelection::collapsed(end),
        Some(TextRange::new(start, end)),
    )
}

fn moved_selection(value: &TextEditingValue, target: usize, extend: bool) -> TextEditingValue {
    let selection = if extend {
        TextSelection::new(value.selection().anchor(), target)
    } else {
        TextSelection::collapsed(target)
    };
    value.with_selection(selection)
}

fn replace_range(
    value: &TextEditingValue,
    range: std::ops::Range<usize>,
    replacement: &str,
) -> TextEditingValue {
    value.splice(range, replacement, TextSelection::collapsed, None)
}

fn limited_replacement<'a>(
    value: &TextEditingValue,
    range: std::ops::Range<usize>,
    replacement: &'a str,
    max_length: Option<usize>,
) -> std::borrow::Cow<'a, str> {
    let Some(max_length) = max_length else {
        // Not copied: a paste can be megabytes.
        return std::borrow::Cow::Borrowed(replacement);
    };
    let selected = value.grapheme_of_byte(range.end) - value.grapheme_of_byte(range.start);
    let retained = value.grapheme_count().saturating_sub(selected);
    let available = max_length.saturating_sub(retained);
    std::borrow::Cow::Owned(replacement.graphemes(true).take(available).collect())
}
