//! Highlighting a text a line at a time, so an edit costs the lines it touched.
//!
//! A highlighter that is handed the whole text styles all of it again for every
//! change, and for a long note that is the cost of the edit. Most highlighting is
//! line by line with a little state carried from one line to the next (inside a
//! fenced code block or not, inside a block comment or not), and for that the work
//! can be remembered: each line keeps what it was given, the ranges it produced and
//! the state it ended in. After an edit, the lines before the first change and
//! after the last are the same lines, and the lines in between are run again until
//! the state they end in is the one the old text had there, past which everything
//! would come out as it did.

use std::cell::RefCell;
use std::rc::Rc;

use aimer_text::StyledRange;

/// One line of the text as last highlighted.
struct Line<S> {
    hash: u64,
    len: usize,
    /// The state the line was highlighted in and the one it left behind.
    start: S,
    end: S,
    /// What it produced, relative to the start of the line.
    ranges: Rc<[StyledRange]>,
}

impl<S: Clone> Clone for Line<S> {
    fn clone(&self) -> Self {
        Self {
            hash: self.hash,
            len: self.len,
            start: self.start.clone(),
            end: self.end.clone(),
            ranges: Rc::clone(&self.ranges),
        }
    }
}

/// A highlighter of lines that remembers its last text.
pub(crate) struct LineHighlighter<S, F> {
    initial: S,
    line: F,
    previous: RefCell<Vec<Line<S>>>,
}

#[inline]
fn hash_line(line: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut words = line.as_bytes().chunks_exact(8);
    for word in &mut words {
        let word = u64::from_le_bytes(word.try_into().expect("a chunk of eight bytes"));
        hash = (hash.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    let mut tail = 0u64;
    for (index, byte) in words.remainder().iter().enumerate() {
        tail |= u64::from(*byte) << (8 * index);
    }
    hash = (hash.rotate_left(5) ^ tail).wrapping_mul(0x517c_c1b7_2722_0a95);
    (hash.rotate_left(5) ^ line.len() as u64).wrapping_mul(0x517c_c1b7_2722_0a95)
}

impl<S, F> LineHighlighter<S, F>
where
    S: Clone + PartialEq,
    F: Fn(&str, &S) -> (Vec<StyledRange>, S),
{
    pub(crate) fn new(initial: S, line: F) -> Self {
        Self { initial, line, previous: RefCell::new(Vec::new()) }
    }

    /// The ranges of `text`, taken from the last text's lines where they are the
    /// same lines in the same state.
    pub(crate) fn highlight(&self, text: &str) -> Vec<StyledRange> {
        let lines = text.split_inclusive('\n').collect::<Vec<_>>();
        let hashes = lines.iter().map(|line| hash_line(line)).collect::<Vec<_>>();
        let old = std::mem::take(&mut *self.previous.borrow_mut());
        let same = |old: &Line<S>, at: usize| old.hash == hashes[at] && old.len == lines[at].len();

        let shorter = old.len().min(lines.len());
        let mut prefix = 0;
        while prefix < shorter && same(&old[prefix], prefix) {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < shorter - prefix
            && same(&old[old.len() - 1 - suffix], lines.len() - 1 - suffix)
        {
            suffix += 1;
        }

        let mut now: Vec<Line<S>> = Vec::with_capacity(lines.len());
        now.extend(old[..prefix].iter().cloned());
        let mut state = match prefix.checked_sub(1) {
            Some(last) => old[last].end.clone(),
            None => self.initial.clone(),
        };
        let mut run = |at: usize, state: &mut S, now: &mut Vec<Line<S>>| {
            let (ranges, end) = (self.line)(lines[at], state);
            now.push(Line {
                hash: hashes[at],
                len: lines[at].len(),
                start: state.clone(),
                end: end.clone(),
                ranges: ranges.into(),
            });
            *state = end;
        };

        let changed_end = lines.len() - suffix;
        for at in prefix..changed_end {
            run(at, &mut state, &mut now);
        }
        // The lines that were the same: they come out as they did unless the state
        // reaching them is not the one they had.
        let old_suffix = old.len() - suffix;
        for (k, old_line) in old[old_suffix..].iter().enumerate() {
            if old_line.start == state {
                now.extend(old[old_suffix + k..].iter().cloned());
                break;
            }
            run(changed_end + k, &mut state, &mut now);
        }
        debug_assert_eq!(now.len(), lines.len());

        let mut ranges = Vec::new();
        let mut offset = 0;
        for (line, entry) in lines.iter().zip(&now) {
            ranges.extend(entry.ranges.iter().map(|range| {
                StyledRange::new(range.range.start + offset..range.range.end + offset, range.style.clone())
            }));
            offset += line.len();
        }
        *self.previous.borrow_mut() = now;
        ranges
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_text::{SpanStyle, StyledRange};
    use aimer_widget::base::Color;

    use super::LineHighlighter;

    const RED: Color = Color::Rgba(200, 30, 30, 255);
    const BLUE: Color = Color::Rgba(30, 30, 200, 255);

    /// Fences toggle a block: every line inside one, and the fences, is red; a
    /// heading outside is blue. The state is whether a block is open.
    fn line(line: &str, in_block: &bool) -> (Vec<StyledRange>, bool) {
        let body = line.trim_end_matches('\n').len();
        let fence = line.starts_with("```");
        let open = *in_block != fence;
        let color = if *in_block || fence {
            Some(RED)
        } else if line.starts_with("# ") {
            Some(BLUE)
        } else {
            None
        };
        let ranges = color
            .filter(|_| body > 0)
            .map(|color| StyledRange::new(0..body, SpanStyle::new().color(color)))
            .into_iter()
            .collect();
        (ranges, open)
    }

    fn counting() -> (LineHighlighter<bool, impl Fn(&str, &bool) -> (Vec<StyledRange>, bool)>, Rc<Cell<usize>>) {
        let calls = Rc::new(Cell::new(0));
        let counted = Rc::clone(&calls);
        let highlighter = LineHighlighter::new(false, move |text: &str, state: &bool| {
            counted.set(counted.get() + 1);
            line(text, state)
        });
        (highlighter, calls)
    }

    fn fresh(text: &str) -> Vec<StyledRange> {
        LineHighlighter::new(false, line).highlight(text)
    }

    fn note() -> String {
        let mut text = String::new();
        for index in 0..60 {
            match index % 10 {
                0 => text.push_str(&format!("# Heading {index}\n")),
                3 => text.push_str("```\n"),
                6 => text.push_str("```\n"),
                _ => text.push_str(&format!("line {index} of the note\n")),
            }
        }
        text.push_str("no newline at the end");
        text
    }

    fn assert_same(got: &[StyledRange], want: &[StyledRange], what: &str) {
        assert_eq!(format!("{got:?}"), format!("{want:?}"), "{what}");
    }

    fn insert_at_line(text: &str, line: usize, with: &str) -> String {
        let at: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
        format!("{}{with}{}", &text[..at], &text[at..])
    }

    #[test]
    fn the_first_highlight_is_the_one_a_fresh_highlighter_gives() {
        let text = note();
        let (highlighter, calls) = counting();
        assert_same(&highlighter.highlight(&text), &fresh(&text), "first");
        assert_eq!(calls.get(), text.split_inclusive('\n').count(), "every line once");
    }

    #[test]
    fn an_edit_gives_what_highlighting_all_of_the_text_gives() {
        let text = note();
        let edits: Vec<(&str, String)> = vec![
            ("typed in a plain line", insert_at_line(&text, 20, "typed ")),
            ("typed in a heading", text.replacen("Heading 10", "Heading X10", 1)),
            ("a fence opened", insert_at_line(&text, 12, "```\n")),
            ("a fence closed", insert_at_line(&text, 4, "```\n")),
            ("a fence removed", text.replacen("```\n", "", 1)),
            ("a line inserted", insert_at_line(&text, 30, "a new line\n")),
            ("a line deleted", text.replacen("line 21 of the note\n", "", 1)),
            ("lines joined", text.replacen("line 21 of the note\nline 22", "line 21 of the noteline 22", 1)),
            ("a line split", text.replacen("line 21 of the note", "line 21 of\n the note", 1)),
            ("the last line edited", format!("{text}!")),
            ("a trailing newline", format!("{text}\n")),
            ("the first line edited", format!("x{text}")),
            ("everything replaced", "# one\nthe end\n".to_owned()),
            ("emptied", String::new()),
        ];
        for (what, edited) in edits {
            let (highlighter, _) = counting();
            let _ = highlighter.highlight(&text);
            assert_same(&highlighter.highlight(&edited), &fresh(&edited), what);
            // And back again, so the second edit starts from an edited cache.
            assert_same(&highlighter.highlight(&text), &fresh(&text), &format!("{what}, undone"));
        }
    }

    #[test]
    fn typing_in_a_line_highlights_that_line_alone() {
        let text = note();
        let (highlighter, calls) = counting();
        let _ = highlighter.highlight(&text);
        let before = calls.get();

        let _ = highlighter.highlight(&insert_at_line(&text, 20, "typed "));
        assert_eq!(calls.get() - before, 1);

        // Asked again with nothing changed, no line is highlighted.
        let before = calls.get();
        let _ = highlighter.highlight(&insert_at_line(&text, 20, "typed "));
        assert_eq!(calls.get(), before);
    }

    #[test]
    fn a_changed_state_runs_on_only_until_it_is_the_old_state_again() {
        let text = note();
        let (highlighter, calls) = counting();
        let _ = highlighter.highlight(&text);
        let before = calls.get();

        // A pair of fences is two toggles: the lines between them change and the state
        // after them is the one the old text had, so the lines after are not run.
        let edited = insert_at_line(&text, 8, "```
in the block
```
");
        assert_same(&highlighter.highlight(&edited), &fresh(&edited), "a block inserted");
        assert_eq!(calls.get() - before, 3, "the three lines that were inserted");
    }

    #[test]
    fn one_fence_changes_every_line_after_it_and_they_are_all_run() {
        let text = note();
        let (highlighter, calls) = counting();
        let _ = highlighter.highlight(&text);
        let before = calls.get();

        // One fence flips whether every later line is in a block, for good, so no
        // line after it can be taken from the old text.
        let edited = insert_at_line(&text, 12, "```
");
        assert_same(&highlighter.highlight(&edited), &fresh(&edited), "a fence inserted");
        let total = edited.split_inclusive('\n').count();
        assert_eq!(calls.get() - before, total - 12, "the new line and every line after it");
    }
}
