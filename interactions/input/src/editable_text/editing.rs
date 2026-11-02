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
    let Some(previous) = value.text()[..caret]
        .grapheme_indices(true)
        .next_back()
        .map(|(index, _)| index)
    else {
        return value.clone();
    };
    replace_range(value, previous..caret, "")
}

pub(crate) fn delete_forward(value: &TextEditingValue) -> TextEditingValue {
    let selection = value.selection().range();
    if !selection.is_empty() {
        return replace_range(value, selection, "");
    }
    let caret = selection.start;
    let Some(grapheme) = value.text()[caret..].graphemes(true).next() else {
        return value.clone();
    };
    replace_range(value, caret..caret + grapheme.len(), "")
}

pub(crate) fn move_left(value: &TextEditingValue, extend: bool) -> TextEditingValue {
    let selection = value.selection();
    let target = if !extend && !selection.is_collapsed() {
        selection.range().start
    } else {
        value.text()[..selection.focus()]
            .grapheme_indices(true)
            .next_back()
            .map_or(selection.focus(), |(index, _)| index)
    };
    moved_selection(value, target, extend)
}

pub(crate) fn move_right(value: &TextEditingValue, extend: bool) -> TextEditingValue {
    let selection = value.selection();
    let target = if !extend && !selection.is_collapsed() {
        selection.range().end
    } else {
        value.text()[selection.focus()..]
            .graphemes(true)
            .next()
            .map_or(selection.focus(), |grapheme| {
                selection.focus() + grapheme.len()
            })
    };
    moved_selection(value, target, extend)
}

pub(crate) fn update_composing(value: &TextEditingValue, preedit: &str) -> TextEditingValue {
    let range = value.composing().map_or_else(
        || value.selection().range(),
        |range| range.start()..range.end(),
    );
    let mut text = String::with_capacity(value.text().len() - range.len() + preedit.len());
    text.push_str(&value.text()[..range.start]);
    text.push_str(preedit);
    text.push_str(&value.text()[range.end..]);
    let end = range.start + preedit.len();
    TextEditingValue::new(
        text,
        TextSelection::collapsed(end),
        Some(TextRange::new(range.start, end)),
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
    let mut text = String::with_capacity(value.text().len() - range.len() + replacement.len());
    text.push_str(&value.text()[..range.start]);
    text.push_str(replacement);
    text.push_str(&value.text()[range.end..]);
    let caret = range.start + replacement.len();
    TextEditingValue::new(text, TextSelection::collapsed(caret), None)
}

fn limited_replacement(
    value: &TextEditingValue,
    range: std::ops::Range<usize>,
    replacement: &str,
    max_length: Option<usize>,
) -> String {
    let Some(max_length) = max_length else {
        return replacement.to_owned();
    };
    let selected = value.text()[range.clone()].graphemes(true).count();
    let retained = value
        .text()
        .graphemes(true)
        .count()
        .saturating_sub(selected);
    let available = max_length.saturating_sub(retained);
    replacement.graphemes(true).take(available).collect()
}
