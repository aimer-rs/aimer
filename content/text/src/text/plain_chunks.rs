//! Drawing a long plain text as requests of a few hard lines each.
//!
//! The renderer sets a text request aside as a whole when its box is off screen, and otherwise
//! looks at every glyph of it, every frame, whether the glyph is on screen or not. A long text
//! drawn as one request is therefore all looked at while only a screenful of it shows, as the
//! window of a long note does (a few screens of text, so that scrolling a little changes
//! nothing). Cut into requests of a few lines each, the ones off screen cost nothing, and a
//! window that moves shapes only the chunks that came into it.
//!
//! Hard lines wrap, measure and stack independently of each other, so a request of whole hard
//! lines is laid out as those lines are in the whole text, and is placed under the ones before
//! it by their heights. The cuts depend on what the lines say and not on how many come before
//! them, so a window that has moved by some lines still cuts the lines it shares with the
//! window before it into the same chunks, and the renderer already has those shaped.

use std::ops::Range;

/// Texts shorter than this are drawn as one request: the cuts are not worth their cost, and a
/// short text is all on screen anyway.
pub(super) const MIN_BYTES: usize = 4096;

/// On average one hard line in this many ends a chunk.
const CUT_MODULUS: u64 = 8;

/// No chunk holds more hard lines than this, whatever the hashes say, so a run of lines that
/// never cut cannot make a chunk as large as the text.
const MAX_CHUNK_LINES: usize = 24;

/// A chunk of the text: the part to draw, and whether the line feed that ends it (which is
/// not drawn, and belongs to no line of the chunk) follows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Chunk {
    /// The text of the chunk's hard lines, without the line feed that ends the last of them
    /// (except for the last chunk of the text, which keeps all of its text).
    pub text: Range<usize>,
}

fn hash_line(line: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in line {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    // The low bits of this hash are the weakest: cut on the high ones.
    hash >> 32
}

/// The chunks to draw `text` in, or `None` when it is to be drawn as one request: when it is
/// short, or has a carriage return (whose line feed would be cut off from it), or is only one
/// chunk.
pub(super) fn plan(text: &str) -> Option<Vec<Chunk>> {
    if text.len() < MIN_BYTES || text.as_bytes().contains(&b'\r') {
        return None;
    }
    let bytes = text.as_bytes();
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut lines = 0;
    let mut at = 0;
    while at < bytes.len() {
        let end = aimer_simd::kernels::find_byte(&bytes[at..], b'\n').map_or(bytes.len(), |found| at + found);
        lines += 1;
        let line_feed = end < bytes.len();
        let cut = line_feed && (hash_line(&bytes[at..end]) % CUT_MODULUS == 0 || lines >= MAX_CHUNK_LINES);
        if cut {
            // The line feed ends the chunk and is not part of what is drawn; the next chunk
            // begins after it.
            chunks.push(Chunk { text: start..end });
            start = end + 1;
            lines = 0;
        }
        at = end + 1;
    }
    // What is left, down to the end of the text, which may end in a line feed of its own: that
    // makes an empty last line, as it does in the whole text.
    if start <= bytes.len() {
        chunks.push(Chunk { text: start..bytes.len() });
    }
    (chunks.len() >= 2).then_some(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(lines: usize) -> String {
        (0..lines).map(|index| format!("line number {index} of a note that is long enough to matter\n")).collect()
    }

    /// The chunks give back the text when put together with the line feeds between them.
    #[test]
    fn chunks_put_back_together_are_the_text() {
        for text in [note(200), note(200) + "no line feed at the end", format!("\n\n{}\n\n", note(100))] {
            let chunks = plan(&text).expect("a long text is cut");
            let joined = chunks.iter().map(|chunk| &text[chunk.text.clone()]).collect::<Vec<_>>().join("\n");
            assert_eq!(joined, text);
            assert!(chunks.windows(2).all(|pair| pair[0].text.end < pair[1].text.start), "in order, with a line feed between");
        }
    }

    /// What cutting a long text into chunks costs, for a 90 KB note of mostly short lines.
    ///
    /// ```text
    /// cargo nextest run -p aimer_text --run-ignored only -E 'test(bench_planning)' --no-capture
    /// ```
    #[test]
    #[ignore = "a timing, not a check"]
    fn bench_planning() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        let text = note(1100);
        let mut best = Duration::MAX;
        for _ in 0..100 {
            let started = Instant::now();
            black_box(plan(black_box(&text)));
            best = best.min(started.elapsed());
        }
        println!("plan {:>6} bytes, {:>4} lines   {best:>10.2?}", text.len(), text.matches('\n').count());
    }

    #[test]
    fn no_chunk_has_more_lines_than_the_cap() {
        let text = "x\n".repeat(3000);
        for chunk in plan(&text).unwrap() {
            assert!(text[chunk.text].matches('\n').count() < MAX_CHUNK_LINES);
        }
    }

    #[test]
    fn a_short_text_a_carriage_return_and_one_chunk_are_drawn_whole() {
        assert!(plan("short\ntext").is_none());
        assert!(plan(&note(200).replace('\n', "\r\n")).is_none());
        assert!(plan(&"a".repeat(MIN_BYTES + 1)).is_none(), "a text with no line feed is one chunk");
    }

    /// A window that has moved by some lines cuts the lines it shares with the one before it
    /// the same way, so the chunks the renderer already has are found again.
    #[test]
    fn a_text_that_begins_some_lines_later_cuts_the_shared_lines_the_same_way() {
        let text = note(300);
        let whole = plan(&text).unwrap();
        let skipped = text.match_indices('\n').nth(36).unwrap().0 + 1;
        let later = &text[skipped..];
        let moved = plan(later).unwrap();
        let from_whole: Vec<&str> = whole.iter().map(|chunk| &text[chunk.text.clone()]).collect();
        let shared = moved.iter().skip(1).filter(|chunk| from_whole.contains(&&later[chunk.text.clone()])).count();
        assert!(shared + 3 >= moved.len(), "{shared} of {} chunks are the same", moved.len());
    }
}
