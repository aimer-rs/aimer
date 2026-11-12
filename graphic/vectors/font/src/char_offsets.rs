//! Walking the characters of a text together with their byte offsets.
//!
//! `str::char_indices` decodes UTF-8 one character at a time through several
//! generic layers, which a debug build runs as real calls. Most text measured is
//! ASCII, where a character is its byte and the offset is the position, so that
//! case is a plain loop over the bytes.

/// Calls `visit(offset, character)` for every character of `text`, exactly as
/// iterating `text.char_indices()` would.
///
/// Always inlined, which a debug build honours too: the per-character call of
/// `visit` is then a call of the closure and not of this function as well.
#[inline(always)]
pub fn for_each_char<F: FnMut(usize, char)>(text: &str, mut visit: F) {
    if text.is_ascii() {
        let bytes = text.as_bytes();
        let mut offset = 0;
        while offset < bytes.len() {
            visit(offset, char::from(bytes[offset]));
            offset += 1;
        }
    } else {
        for (offset, character) in text.char_indices() {
            visit(offset, character);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visited(text: &str) -> Vec<(usize, char)> {
        let mut seen = Vec::new();
        for_each_char(text, |offset, character| seen.push((offset, character)));
        seen
    }

    #[track_caller]
    fn assert_same(text: &str) {
        assert_eq!(visited(text), text.char_indices().collect::<Vec<_>>(), "{text:?}");
    }

    #[test]
    fn every_ascii_byte_alone_and_in_pairs_is_visited_like_char_indices() {
        for first in 0..128_u8 {
            assert_same(&char::from(first).to_string());
            for second in 0..128_u8 {
                assert_same(&format!("{}{}", char::from(first), char::from(second)));
            }
        }
    }

    #[test]
    fn text_beyond_ascii_keeps_byte_offsets() {
        for text in [
            "",
            "plain ascii, 123",
            "caf\u{e9} au lait",
            "日本語 and English",
            "emoji \u{1f600} then text",
            "a\u{0}b\n\tc",
            "\u{10ffff}",
        ] {
            assert_same(text);
        }
        // The offsets are bytes, not characters.
        assert_eq!(visited("é!"), vec![(0, 'é'), (2, '!')]);
    }
}
