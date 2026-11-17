//! Reading a long note a piece at a time.
//!
//! The live preview reads the note with a parser that takes the whole of it, and so each
//! key typed costs the parse of every block of the note, nearly all of them as they were. A
//! note is blocks, and a block is read by what is in it; so where a block is known to end, the
//! text can be cut, the pieces read apart, and what was read of a piece kept for as long as the
//! piece is the same. Typing then re-reads the piece it was typed in.
//!
//! Cutting is only safe where nothing after the cut can change what is before it. These are the
//! places this module cuts, and the reasons it does not cut anywhere it is less than sure:
//!
//! * at a blank line, before a line that starts in the first column and does not begin a list
//!   item. A list goes on across blank lines (a loose list), and a line indented under an item,
//!   or an indented code block, belongs to what is above it; neither starts in the first column
//!   after a blank line without being the start of something new;
//! * never inside a fenced code block, whose blank lines are code, nor inside an HTML block that
//!   a closing tag ends and not a blank line;
//! * never in a note with a link reference definition or a footnote definition, which another
//!   block can use from anywhere. Such a note is read whole, as it always was.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use aimer_text::StyledRange;
use markdown::mdast::Node;

/// The least text, in bytes, worth reading in pieces: below this the whole parse is quick.
pub(crate) const MIN_TEXT_BYTES: usize = 4096;


/// The least a piece holds, in bytes. Pieces of a few blocks keep the cache to a few hundred
/// entries for a long note, and a piece that small is a quick parse.
pub(crate) const MIN_SEGMENT_BYTES: usize = 1024;

/// Where `text` can be cut into pieces that read the same apart as together, as the byte
/// offsets at which each piece but the first starts; or `None` when it is to be read whole.
pub(crate) fn segment_cuts(text: &str) -> Option<Vec<usize>> {
    if text.len() < MIN_TEXT_BYTES {
        return None;
    }
    let mut cuts = Vec::new();
    let mut last_cut = 0;
    let mut fence: Option<(u8, usize)> = None;
    let mut html: Option<&'static str> = None;
    let mut previous_blank = false;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let blank = body.trim().is_empty();
        let inner = strip_containers(body);
        if let Some((mark, length)) = fence {
            if closes_fence(inner, mark, length) {
                fence = None;
            }
        } else if let Some(terminator) = html {
            if body.to_ascii_lowercase().contains(terminator) {
                html = None;
            }
        } else {
            if is_definition(body) {
                return None;
            }
            if previous_blank
                && !blank
                && at - last_cut >= MIN_SEGMENT_BYTES
                && starts_new_block(body)
            {
                cuts.push(at);
                last_cut = at;
            }
            if let Some(opened) = opens_fence(inner) {
                fence = Some(opened);
            } else if let Some(terminator) = html_terminator(body)
                && !body.to_ascii_lowercase().contains(terminator)
            {
                html = Some(terminator);
            }
        }
        previous_blank = blank;
        at += line.len();
    }
    (!cuts.is_empty()).then_some(cuts)
}

/// `line` without the container marks a block can sit in: leading spaces, quote marks and one
/// list marker, so that a fence in a quote or in a list item is found.
fn strip_containers(line: &str) -> &str {
    let mut rest = line;
    loop {
        let trimmed = rest.trim_start_matches([' ', '\t']);
        if let Some(after) = trimmed.strip_prefix('>') {
            rest = after;
            continue;
        }
        rest = trimmed;
        break;
    }
    let bytes = rest.as_bytes();
    let marker = match bytes.first() {
        Some(b'-' | b'*' | b'+') if matches!(bytes.get(1), Some(b' ' | b'\t')) => 1,
        Some(b'0'..=b'9') => {
            let digits = bytes.iter().take_while(|byte| byte.is_ascii_digit()).count();
            match (bytes.get(digits), bytes.get(digits + 1)) {
                (Some(b'.' | b')'), Some(b' ' | b'\t')) => digits + 1,
                _ => 0,
            }
        }
        _ => 0,
    };
    rest[marker..].trim_start_matches([' ', '\t'])
}

/// The mark and length of the fence `inner` opens: three or more backticks or tildes. A backtick
/// fence's info string holds no backtick, or it is code in a line and not a fence.
fn opens_fence(inner: &str) -> Option<(u8, usize)> {
    let mark = *inner.as_bytes().first().filter(|byte| matches!(byte, b'`' | b'~'))?;
    let length = inner.bytes().take_while(|byte| *byte == mark).count();
    if length < 3 || (mark == b'`' && inner[length..].contains('`')) {
        return None;
    }
    Some((mark, length))
}

/// Whether `inner` closes a fence of `length` of `mark`: at least as many, and nothing after.
fn closes_fence(inner: &str, mark: u8, length: usize) -> bool {
    let run = inner.bytes().take_while(|byte| *byte == mark).count();
    run >= length && inner[run..].trim().is_empty()
}

/// Whether `line` is a link reference or a footnote definition, or something that looks like one:
/// a line that starts with a bracketed label and a colon. A false positive only reads the note
/// whole.
fn is_definition(line: &str) -> bool {
    let line = line.trim_start_matches([' ', '\t']);
    line.starts_with('[') && line.contains("]:")
}

/// Whether `line`, after a blank line, starts something new: the first column, and not an item
/// of a list that could be going on.
fn starts_new_block(line: &str) -> bool {
    let bytes = line.as_bytes();
    match bytes.first() {
        None | Some(b' ' | b'\t') => false,
        Some(b'-' | b'*' | b'+') => !matches!(bytes.get(1), None | Some(b' ' | b'\t')),
        Some(b'0'..=b'9') => {
            let digits = bytes.iter().take_while(|byte| byte.is_ascii_digit()).count();
            !(matches!(bytes.get(digits), Some(b'.' | b')'))
                && matches!(bytes.get(digits + 1), None | Some(b' ' | b'\t')))
        }
        Some(_) => true,
    }
}

/// What ends the HTML block `line` opens, for the kinds of block that a blank line does not end.
fn html_terminator(line: &str) -> Option<&'static str> {
    let line = line.trim_start_matches(' ');
    if !line.starts_with('<') {
        return None;
    }
    let lower = line.to_ascii_lowercase();
    let starts = |prefix: &str| lower.starts_with(prefix);
    if starts("<!--") {
        Some("-->")
    } else if starts("<?") {
        Some("?>")
    } else if starts("<![cdata[") {
        Some("]]>")
    } else if starts("<!") && lower.as_bytes().get(2).is_some_and(u8::is_ascii_alphabetic) {
        Some(">")
    } else if starts("<script") {
        Some("</script>")
    } else if starts("<pre") {
        Some("</pre>")
    } else if starts("<style") {
        Some("</style>")
    } else if starts("<textarea") {
        Some("</textarea>")
    } else {
        None
    }
}

/// What was read of one piece of a note.
pub(crate) struct Piece {
    /// The tree the parser made of the piece, in the piece's own offsets.
    pub root: Rc<Node>,
    /// The ranges painted from it, by which part of it the cursor is on (`None` for none): the
    /// same tree is painted again only when the cursor comes to it or leaves it.
    painted: RefCell<Vec<(Option<(usize, usize)>, Rc<Vec<StyledRange>>)>>,
}

impl Piece {
    /// The styled ranges of the piece for the active part `active` of it (a pair of offsets in the
    /// piece), painted once for each.
    pub(crate) fn ranges(
        &self,
        active: Option<(usize, usize)>,
        paint: impl FnOnce(&Node) -> Vec<StyledRange>,
    ) -> Rc<Vec<StyledRange>> {
        if let Some((_, ranges)) = self.painted.borrow().iter().find(|(known, _)| *known == active) {
            return Rc::clone(ranges);
        }
        let ranges = Rc::new(paint(&self.root));
        let mut painted = self.painted.borrow_mut();
        // The cursor is in one piece at a time, so there are two or three of these at most.
        if painted.len() >= 4 {
            painted.remove(0);
        }
        painted.push((active, Rc::clone(&ranges)));
        ranges
    }
}

struct Entry {
    text: Box<str>,
    piece: Rc<Piece>,
    used: u64,
}

/// The pieces of the notes a preview read lately, by what is in them.
///
/// A piece is found by its text, so the piece after an edit is the one before it wherever it moved
/// to: an edit above it, or a line inserted, does not make it new.
pub(crate) struct SegmentCache {
    entries: HashMap<u64, Entry>,
    bytes: usize,
    clock: u64,
}

/// How much text the cache keeps the readings of: the trees are many times this.
const BUDGET_BYTES: usize = 4 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    /// How many pieces were parsed, rather than found, for tests.
    static PARSED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn pieces_parsed() -> usize {
    PARSED.with(std::cell::Cell::get)
}

#[inline]
fn hash_text(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut words = text.as_bytes().chunks_exact(8);
    for word in &mut words {
        let word = u64::from_le_bytes(word.try_into().expect("a chunk of eight bytes"));
        hash = (hash.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    let mut tail = 0u64;
    for (index, byte) in words.remainder().iter().enumerate() {
        tail |= u64::from(*byte) << (8 * index);
    }
    hash = (hash.rotate_left(5) ^ tail).wrapping_mul(0x517c_c1b7_2722_0a95);
    (hash.rotate_left(5) ^ text.len() as u64).wrapping_mul(0x517c_c1b7_2722_0a95)
}

impl SegmentCache {
    pub(crate) fn new() -> Self {
        Self { entries: HashMap::new(), bytes: 0, clock: 0 }
    }

    /// The piece for `text`, parsed with `parse` when it has not been read lately.
    pub(crate) fn piece(&mut self, text: &str, parse: impl FnOnce(&str) -> Option<Node>) -> Option<Rc<Piece>> {
        self.clock += 1;
        let key = hash_text(text);
        if let Some(entry) = self.entries.get_mut(&key)
            && *entry.text == *text
        {
            entry.used = self.clock;
            return Some(Rc::clone(&entry.piece));
        }
        #[cfg(test)]
        PARSED.with(|count| count.set(count.get() + 1));
        let piece = Rc::new(Piece { root: Rc::new(parse(text)?), painted: RefCell::new(Vec::new()) });
        if self.bytes + text.len() > BUDGET_BYTES {
            // The least recently used quarter goes; a note larger than the budget keeps what it
            // reads last, which is what the next edit reads again.
            let mut stamps = self.entries.values().map(|entry| entry.used).collect::<Vec<_>>();
            stamps.sort_unstable();
            if let Some(&oldest) = stamps.get(stamps.len() / 4) {
                self.entries.retain(|_, entry| entry.used > oldest);
                self.bytes = self.entries.values().map(|entry| entry.text.len()).sum();
            }
        }
        if let Some(replaced) = self.entries.insert(key, Entry { text: text.into(), piece: Rc::clone(&piece), used: self.clock }) {
            self.bytes -= replaced.text.len();
        }
        self.bytes += text.len();
        Some(piece)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document of `blocks` paragraphs, each long enough to be a piece of its own.
    fn paragraphs(blocks: usize) -> String {
        (0..blocks).map(|index| format!("{}\n\n", format!("paragraph {index} ").repeat(60))).collect()
    }

    fn cut_lines(text: &str) -> Vec<usize> {
        segment_cuts(text)
            .unwrap_or_default()
            .iter()
            .map(|&cut| text[..cut].matches('\n').count())
            .collect()
    }

    #[test]
    fn a_note_is_cut_at_the_start_of_a_block_after_a_blank_line() {
        let text = paragraphs(10);
        let cuts = segment_cuts(&text).expect("a long note of paragraphs is cut");
        // Paragraphs of about 800 bytes: a piece takes two to reach its least size.
        assert!(cuts.len() >= 3, "{} cuts", cuts.len());
        for cut in cuts {
            assert!(text[..cut].ends_with("\n\n"), "a cut follows a blank line");
            assert!(text[cut..].starts_with("paragraph"), "and starts a block");
        }
    }

    #[test]
    fn short_notes_and_notes_without_a_place_to_cut_are_read_whole() {
        assert_eq!(segment_cuts("# title\n\nbody\n\nmore"), None, "too short to be worth it");
        assert_eq!(segment_cuts(&"one long line without a break ".repeat(300)), None);
    }

    #[test]
    fn a_definition_anywhere_makes_the_note_read_whole() {
        let mut text = paragraphs(10);
        assert!(segment_cuts(&text).is_some());
        text.push_str("[label]: https://example.com\n");
        assert_eq!(segment_cuts(&text), None);
        let mut text = paragraphs(10);
        text.push_str("[^1]: a footnote\n");
        assert_eq!(segment_cuts(&text), None);
        // Inside a fence it is code, not a definition.
        let mut text = paragraphs(10);
        text.push_str("```\n[label]: not a definition\n```\n");
        assert!(segment_cuts(&text).is_some());
    }

    #[test]
    fn a_fenced_block_is_never_cut() {
        let code = format!("```rust\n{}\n\n{}\n```\n\n", "let a = 1;".repeat(120), "let b = 2;".repeat(120));
        let text = format!("{}{code}{}", paragraphs(4), paragraphs(4));
        let start = text.find("```rust").unwrap();
        let end = start + code.len();
        for cut in segment_cuts(&text).unwrap() {
            assert!(!(start < cut && cut < end), "a cut at {cut} is inside the fence {start}..{end}");
        }
        // Also a fence in a list item and in a quote, which a blank line inside does not end.
        for lead in ["- ", "> ", "1. "] {
            let code = format!("{lead}```\n{}\n\n{}\n{lead}```\n\n", "x".repeat(600), "y".repeat(600));
            let text = format!("{}{code}{}", paragraphs(3), paragraphs(3));
            let start = text.find("```").unwrap();
            let end = text.rfind("```").unwrap();
            for cut in segment_cuts(&text).unwrap() {
                assert!(!(start < cut && cut < end), "lead {lead:?}: a cut at {cut} is inside the fence");
            }
        }
    }

    #[test]
    fn an_html_block_that_a_closing_tag_ends_is_never_cut() {
        for (open, close) in [("<!--", "-->"), ("<script>", "</script>"), ("<pre>", "</pre>"), ("<style>", "</style>")] {
            let block = format!("{open}\n{}\n\n{}\n{close}\n\n", "a".repeat(600), "b".repeat(600));
            let text = format!("{}{block}{}", paragraphs(3), paragraphs(3));
            let start = text.find(open).unwrap();
            let end = text.find(close).unwrap();
            for cut in segment_cuts(&text).unwrap() {
                assert!(!(start < cut && cut < end), "{open}: a cut at {cut} is inside the block");
            }
        }
    }

    #[test]
    fn a_list_that_goes_on_across_blank_lines_is_one_piece() {
        // A loose list: its items are apart by blank lines, and its item is indented text.
        let item = |index: usize| format!("- item {index} {}\n\n  continued {}\n\n", "word ".repeat(120), "more ".repeat(60));
        let numbered = |index: usize| format!("{index}. item {}\n\n", "word ".repeat(200));
        for (what, make) in [("bullets", &item as &dyn Fn(usize) -> String), ("numbers", &numbered)] {
            let list: String = (1..=8).map(make).collect();
            let text = format!("{}{list}{}", paragraphs(2), paragraphs(3));
            let start = text.find(if what == "bullets" { "- item 1" } else { "1. item" }).unwrap();
            let end = start + list.len();
            for cut in segment_cuts(&text).unwrap() {
                assert!(!(start < cut && cut < end), "{what}: a cut at {cut} is inside the list {start}..{end}");
            }
        }
    }

    #[test]
    fn an_indented_line_after_a_blank_line_stays_with_what_it_follows() {
        let text = format!("{}    indented code {}\n\n{}", paragraphs(3), "x".repeat(900), paragraphs(3));
        let at = text.find("    indented").unwrap();
        assert!(!segment_cuts(&text).unwrap().contains(&at));
        let _ = cut_lines(&text);
    }
}
