use aimer_events::text_editing::TextEditingDelta;
use aimer_text::TextSelection;

use super::{TextEditingValue, TextRange};

pub(crate) fn adapt_native_delta(
    value: &TextEditingValue,
    delta: &TextEditingDelta,
) -> Option<TextEditingValue> {
    let replace_start = utf16_to_byte(value.text(), delta.replacement.start)?;
    let replace_end = utf16_to_byte(value.text(), delta.replacement.end)?;
    let mut text = String::with_capacity(
        value.text().len() - (replace_end - replace_start) + delta.replacement_text.len(),
    );
    text.push_str(&value.text()[..replace_start]);
    text.push_str(&delta.replacement_text);
    text.push_str(&value.text()[replace_end..]);

    let selection = TextSelection::new(
        utf16_to_byte(&text, delta.selection.start)?,
        utf16_to_byte(&text, delta.selection.end)?,
    );
    let composing = match delta.composing {
        Some(range) => Some(TextRange::new(
            utf16_to_byte(&text, range.start)?,
            utf16_to_byte(&text, range.end)?,
        )),
        None => None,
    };
    Some(TextEditingValue::new(text, selection, composing))
}

pub(crate) fn utf16_to_byte(text: &str, utf16_offset: usize) -> Option<usize> {
    let mut utf16 = 0;
    for (byte, ch) in text.char_indices() {
        if utf16 == utf16_offset {
            return Some(byte);
        }
        utf16 += ch.len_utf16();
        if utf16 > utf16_offset {
            return None;
        }
    }
    (utf16 == utf16_offset).then_some(text.len())
}

#[cfg(any(target_os = "ios", target_os = "android", test))]
pub(crate) fn byte_to_utf16(text: &str, byte_offset: usize) -> Option<usize> {
    text.is_char_boundary(byte_offset)
        .then(|| text[..byte_offset].encode_utf16().count())
}

#[cfg(test)]
mod tests {
    use aimer_events::text_editing::{NativeTextRange, TextEditingDelta};
    use aimer_text::TextSelection;

    use super::{adapt_native_delta, byte_to_utf16, utf16_to_byte};
    use crate::TextEditingValue;

    #[test]
    fn utf16_ranges_convert_without_splitting_surrogate_pairs() {
        assert_eq!(utf16_to_byte("A👩‍💻B", 1), Some(1));
        assert_eq!(utf16_to_byte("A👩‍💻B", 2), None);
        assert_eq!(utf16_to_byte("A👩‍💻B", 6), Some(12));
        assert_eq!(byte_to_utf16("A👩‍💻B", 12), Some(6));
        assert_eq!(byte_to_utf16("A👩‍💻B", 2), None);
    }

    #[test]
    fn native_selection_only_and_composing_deltas_preserve_unicode_text() {
        let value = TextEditingValue::with_text("A👩‍💻B");
        let delta = TextEditingDelta {
            session_id: 4,
            revision: 0,
            replacement: NativeTextRange::new(6, 6),
            replacement_text: String::new(),
            selection: NativeTextRange::new(1, 1),
            composing: Some(NativeTextRange::new(1, 6)),
        };

        let adapted = adapt_native_delta(&value, &delta).unwrap();

        assert_eq!(adapted.text(), value.text());
        assert_eq!(adapted.selection(), TextSelection::collapsed(1));
        assert_eq!(
            adapted
                .composing()
                .map(|range| (range.start(), range.end())),
            Some((1, 12))
        );
    }
}
