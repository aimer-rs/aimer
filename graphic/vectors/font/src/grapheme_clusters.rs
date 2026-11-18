//! The grapheme clusters of a text, without asking the segmenter about every character.
//!
//! The segmenter decides each boundary by looking at the characters around it, which is the work
//! of a table search per character. Between two ASCII characters there is always a boundary (but
//! for a carriage return before a line feed), and nothing before it can matter to what follows:
//! no ASCII character extends, joins or pairs with another, and the state the rules carry from
//! one boundary to the next (regional indicators, emoji sequences) is empty after one. So text is
//! cut at those places, an ASCII character on its own is a cluster, and only the stretches
//! between are segmented. Text is mostly ASCII even where it is not only ASCII, and a note in
//! any script keeps its markup, numbers, spaces and names in it.

use unicode_segmentation::UnicodeSegmentation;

/// Whether there is a grapheme boundary between the bytes `before` and `after`, which are
/// adjacent in the text, found from those two bytes alone.
#[inline]
fn is_plain_boundary(before: u8, after: u8) -> bool {
    before < 0x80 && after < 0x80 && !(before == b'\r' && after == b'\n')
}

/// Calls `visit` with the start and the text of each grapheme cluster of `text`, in order: what
/// `text.grapheme_indices(true)` yields.
pub(crate) fn for_each_grapheme<'a>(text: &'a str, mut visit: impl FnMut(usize, &'a str)) {
    use aimer_simd::kernels::{find_byte, find_byte_at_least};

    /// A stretch of ASCII shorter than this is not worth a scan: text that keeps breaking off
    /// into non-ASCII characters is walked a character at a time from then on.
    const WORTH_A_SCAN: usize = 16;

    let bytes = text.as_bytes();
    let mut at = 0;
    let mut scanning = true;
    while at < bytes.len() {
        if scanning && bytes[at] < 0x80 {
            // A stretch of ASCII is found in one scan, and is clusters of one byte up to a
            // carriage return (which may pair with a line feed) or the last byte before a
            // character that is not ASCII (which may attach to it): those are left to what
            // follows.
            let ascii_end = find_byte_at_least(&bytes[at..], 0x80).map_or(bytes.len(), |found| at + found);
            scanning = ascii_end == bytes.len() || ascii_end - at >= WORTH_A_SCAN;
            let mut plain_end = if ascii_end < bytes.len() { ascii_end - 1 } else { ascii_end };
            if let Some(carriage_return) = find_byte(&bytes[at..plain_end], b'\r') {
                plain_end = at + carriage_return;
            }
            if plain_end > at {
                for start in at..plain_end {
                    visit(start, &text[start..start + 1]);
                }
                at = plain_end;
                continue;
            }
        }
        // A character that cannot join its neighbours is a cluster of its own.
        if bytes[at] < 0x80 && bytes.get(at + 1).is_none_or(|&next| is_plain_boundary(bytes[at], next)) {
            visit(at, &text[at..at + 1]);
            at += 1;
            continue;
        }
        // Otherwise the stretch up to the next place where the segmenter has nothing to decide.
        let mut end = at + 1;
        while end < bytes.len() && !is_plain_boundary(bytes[end - 1], bytes[end]) {
            end += 1;
        }
        for (offset, cluster) in text[at..end].grapheme_indices(true) {
            visit(at + offset, cluster);
        }
        at = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clusters(text: &str) -> Vec<(usize, &str)> {
        let mut found = Vec::new();
        for_each_grapheme(text, |start, cluster| found.push((start, cluster)));
        found
    }

    fn expected(text: &str) -> Vec<(usize, &str)> {
        text.grapheme_indices(true).collect()
    }

    #[test]
    fn plain_ascii_is_one_cluster_a_byte_and_the_empty_text_has_none() {
        assert_eq!(clusters("Hello, world!\n"), expected("Hello, world!\n"));
        assert!(clusters("").is_empty());
    }

    #[test]
    fn a_carriage_return_and_a_line_feed_are_one_cluster() {
        assert_eq!(clusters("a\r\nb\rc\n\r\n"), expected("a\r\nb\rc\n\r\n"));
        assert!(clusters("a\r\nb").contains(&(1, "\r\n")));
    }

    /// Characters that make clusters hard: marks that attach to what is before them, joiners,
    /// variation selectors, regional indicators that pair up, Hangul syllables built from jamo,
    /// Indic and Khmer signs, a prepended mark, and the ASCII and line breaks between.
    const PIECES: [&str; 28] = [
        "a", "b", " ", "\n", "\r", "\r\n", "1", "-", "e", "\u{301}", "\u{e9}", "\u{200d}", "\u{fe0f}", "\u{1f468}", "\u{1f469}",
        "\u{1f1e6}", "\u{1f1e7}", "\u{1f1e8}", "\u{1100}", "\u{1161}", "\u{11a8}", "\u{ac00}", "\u{915}", "\u{94d}", "\u{1780}",
        "\u{17d2}", "\u{600}", "\u{4f60}",
    ];

    #[test]
    fn text_of_every_hard_kind_is_cut_where_the_segmenter_cuts_it() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |below: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % below as u64) as usize
        };
        for round in 0..3000 {
            let count = next(40);
            let text: String = (0..count).map(|_| PIECES[next(PIECES.len())]).collect();
            assert_eq!(clusters(&text), expected(&text), "round {round}: {text:?}");
        }
    }

    #[test]
    fn an_ascii_character_followed_by_a_combining_mark_is_not_cut_from_it() {
        assert_eq!(clusters("xe\u{301}y"), expected("xe\u{301}y"));
        assert_eq!(clusters("e\u{301}"), vec![(0, "e\u{301}")]);
    }
}
