// Moving the caret and deleting the way a text editor does: by word, by line, by page, to the
// ends of the text, with shift extending the selection.
//
// What a key does depends on which modifier is held and on the platform's habit: Option moves by
// word on macOS where Control does elsewhere, Command moves to the ends of the line or of the text
// there where Home and End do on the others. [`navigation_for`] is that table, a plain function of
// the key, the modifiers and the platform, so both habits are tested wherever the tests run. What
// the field does with the answer is `RawTextField::navigate`.

/// Whether this build follows the macOS habits for movement keys.
const PLATFORM_IS_MAC: bool = cfg!(any(target_os = "macos", target_os = "ios"));

/// How far a key moves the caret, or how far back a delete key reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    Left,
    Right,
    WordLeft,
    WordRight,
    /// The start of the visual line: where it wraps, or the hard line it starts.
    LineStart,
    /// The end of the visual line.
    LineEnd,
    DocumentStart,
    DocumentEnd,
    Up,
    Down,
    PageUp,
    PageDown,
}

/// What a key does to the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Navigation {
    /// Moves the caret, or the end of the selection when shift is held.
    Move(Reach),
    /// Deletes from the caret as far as `Reach` goes. A selection is deleted instead, as it is by
    /// a plain Backspace.
    Delete(Reach),
}

/// What `key` does with `modifiers` held, on a platform that follows the macOS habits when `mac`
/// is set; `None` for a key this does not give a meaning to (a plain Backspace is an ordinary
/// edit, and a letter is typed). Shift is not looked at: it extends whatever the key moves.
fn navigation_for(key: &NamedKey, modifiers: &Modifiers, mac: bool) -> Option<Navigation> {
    use Navigation::{Delete, Move};
    // Option on macOS, Control elsewhere: the key that makes a movement a word at a time.
    let by_word = if mac { modifiers.alt && !modifiers.meta } else { modifiers.ctrl && !modifiers.alt };
    // Command on macOS: to the ends of the line and of the text.
    let command = mac && modifiers.meta;
    Some(match key {
        NamedKey::ArrowLeft if command => Move(Reach::LineStart),
        NamedKey::ArrowLeft if by_word => Move(Reach::WordLeft),
        NamedKey::ArrowLeft => Move(Reach::Left),
        NamedKey::ArrowRight if command => Move(Reach::LineEnd),
        NamedKey::ArrowRight if by_word => Move(Reach::WordRight),
        NamedKey::ArrowRight => Move(Reach::Right),
        NamedKey::ArrowUp if command => Move(Reach::DocumentStart),
        NamedKey::ArrowUp => Move(Reach::Up),
        NamedKey::ArrowDown if command => Move(Reach::DocumentEnd),
        NamedKey::ArrowDown => Move(Reach::Down),
        // Home and End are the ends of the line; with Control or Command held, of the text.
        NamedKey::Home if modifiers.ctrl || modifiers.meta => Move(Reach::DocumentStart),
        NamedKey::Home => Move(Reach::LineStart),
        NamedKey::End if modifiers.ctrl || modifiers.meta => Move(Reach::DocumentEnd),
        NamedKey::End => Move(Reach::LineEnd),
        NamedKey::PageUp => Move(Reach::PageUp),
        NamedKey::PageDown => Move(Reach::PageDown),
        NamedKey::Backspace if command => Delete(Reach::LineStart),
        NamedKey::Backspace if by_word => Delete(Reach::WordLeft),
        NamedKey::Delete if command => Delete(Reach::LineEnd),
        NamedKey::Delete if by_word => Delete(Reach::WordRight),
        _ => return None,
    })
}

/// Whether a piece of text between word boundaries is a word, and not the spaces or the
/// punctuation between words.
fn is_wordlike(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

/// Where the word edge nearest `at` is, going forward (the end of the word the caret is in or the
/// next one) or back (the start of the word it is in or the previous one), as a byte offset of
/// `text`; `None` when there is no word that way.
///
/// Words are those of Unicode's word segmentation, so `don't` and `foo_bar` are one word each, and
/// each ideograph is a word of its own. Spaces, punctuation and line breaks are not words: they are
/// passed over, which is what a caret does between two words.
fn word_edge(text: &str, at: usize, forward: bool) -> Option<usize> {
    use unicode_segmentation::UnicodeSegmentation;
    if forward {
        text.split_word_bound_indices()
            .find(|(start, segment)| start + segment.len() > at && is_wordlike(segment))
            .map(|(start, segment)| start + segment.len())
    } else {
        text.split_word_bound_indices()
            .rev()
            .find(|(start, segment)| *start < at && is_wordlike(segment))
            .map(|(start, _)| start)
    }
}

/// The grapheme offset of the start or end of the visual line the caret `current` is on.
///
/// A caret at the join of two lines that wrap into each other is drawn at the start of the second,
/// so it is on that one. The end of a line that wraps stops before the spaces it wraps after, where
/// the last word ends: `trailing_spaces` says how many a line has. `None` when no line holds the
/// caret.
fn line_edge(
    lines: &[VisualLine],
    current: usize,
    end: bool,
    trailing_spaces: impl Fn(&VisualLine) -> usize,
) -> Option<usize> {
    let index = lines.iter().rposition(|line| current >= line.grapheme_start && current <= line.grapheme_end)?;
    let line = &lines[index];
    if !end {
        return Some(line.grapheme_start);
    }
    let wraps = lines.get(index + 1).is_some_and(|next| next.grapheme_start == line.grapheme_end);
    let spaces = if wraps { trailing_spaces(line).min(line.grapheme_end - line.grapheme_start) } else { 0 };
    // A line of nothing but spaces keeps them: there is no word to stop after.
    Some(if spaces == line.grapheme_end - line.grapheme_start { line.grapheme_end } else { line.grapheme_end - spaces })
}

/// How far either way, in bytes, a word is looked for from the caret: a word is not longer than
/// this, and a text of megabytes is not read out to find one.
const WORD_REACH: usize = 4096;

impl RawTextField {
    /// Does what `navigation` says, extending the selection when `extend` is set. Returns whether
    /// the key was taken: a delete in a read-only field is not, and a page key in a field that
    /// has no page of its own (the one its parent scrolls) is left for the parent.
    fn navigate(&self, navigation: Navigation, extend: bool) -> bool {
        match navigation {
            Navigation::Move(reach) => self.move_caret(reach, extend),
            Navigation::Delete(reach) => self.delete_by(reach),
        }
    }

    fn move_caret(&self, reach: Reach, extend: bool) -> bool {
        // A secure field shows bullets, not words, and has no lines: a word is the line.
        let reach = match reach {
            Reach::WordLeft if self.input_type.is_obscured() => Reach::LineStart,
            Reach::WordRight if self.input_type.is_obscured() => Reach::LineEnd,
            other => other,
        };
        match reach {
            Reach::Left => self.move_left(extend),
            Reach::Right => self.move_right(extend),
            Reach::Up => self.move_vertical(-1, extend),
            Reach::Down => self.move_vertical(1, extend),
            Reach::PageUp | Reach::PageDown => {
                // A field that scrolls inside itself has a page: the lines it shows, less one so
                // the last stays in view. Any other is as tall as its text and its parent scrolls
                // it, so the key is the parent's.
                let Some(shown) = self.max_lines.filter(|shown| *shown > 1) else {
                    return false;
                };
                let rows = (shown - 1) as isize;
                self.move_vertical(if reach == Reach::PageUp { -rows } else { rows }, extend);
            }
            Reach::DocumentStart => self.move_caret_to(0, extend),
            Reach::DocumentEnd => self.move_caret_to(self.controller.grapheme_count(), extend),
            Reach::WordLeft | Reach::WordRight | Reach::LineStart | Reach::LineEnd => {
                let target = self.reach_target(reach);
                self.move_caret_to(target, extend);
            }
        }
        self.reveal_caret.set(true);
        true
    }

    /// Puts the caret at `target`, and when `extend` is set leaves the selection's anchor where
    /// it was, or at the caret if nothing was selected.
    fn move_caret_to(&self, target: usize, extend: bool) {
        let focus = self.cursor.offset();
        let anchor = if extend { self.cursor.selection_anchor().unwrap_or(focus) } else { target };
        self.controller.set_selection_graphemes(anchor, target);
        self.sync_cursor_from_controller();
    }

    /// The grapheme offset a word or line reach goes to from the caret.
    fn reach_target(&self, reach: Reach) -> usize {
        let focus = self.cursor.offset();
        match reach {
            Reach::WordLeft | Reach::WordRight => self.word_target(focus, reach == Reach::WordRight),
            Reach::LineStart | Reach::LineEnd => self.line_target(focus, reach == Reach::LineEnd),
            // The others are not asked for here.
            _ => focus,
        }
    }

    fn word_target(&self, focus: usize, forward: bool) -> usize {
        let value = self.controller.value();
        let byte = value.byte_of_grapheme(focus);
        let from = value.floor_to_boundary(byte.saturating_sub(WORD_REACH));
        let to = value.ceil_to_boundary((byte + WORD_REACH).min(value.len()));
        let window = value.slice(from..to);
        match word_edge(&window, byte - from, forward) {
            Some(edge) => value.grapheme_of_byte(from + edge),
            // No word within reach: as far as was looked, which the next press carries on from.
            None => value.grapheme_of_byte(if forward { to } else { from }),
        }
    }

    fn line_target(&self, focus: usize, end: bool) -> usize {
        let value = self.controller.value();
        let spaces = |line: &VisualLine| {
            let text = value.slice(value.byte_of_grapheme(line.grapheme_start)..value.byte_of_grapheme(line.grapheme_end));
            text.chars().rev().take_while(|c| *c == ' ' || *c == '\t').count()
        };
        let found = match self.geometry_cache.latest() {
            Some(geometry) => line_edge(&geometry.visual_lines, focus, end, spaces),
            None => {
                let lines = wrap_visual_lines(&self.display_text(), f32::INFINITY, |_| 0.0);
                line_edge(&lines, focus, end, spaces)
            }
        };
        // No visual line holds the caret (it is outside the lines laid out): the edge of the hard
        // line it is on, which the text itself says.
        found.unwrap_or_else(|| self.hard_line_edge(focus, end))
    }

    /// The grapheme offset of the start or end of the hard line (between line feeds) that holds
    /// `focus`, read from the text's line counts and not from a layout. The end is before the line
    /// break.
    fn hard_line_edge(&self, focus: usize, end: bool) -> usize {
        let value = self.controller.value();
        let text = value.snapshot();
        let line = text.line_range(text.byte_to_line(value.byte_of_grapheme(focus)));
        let mut edge = if end { line.end } else { line.start };
        if end && edge > line.start && text.byte_at(edge - 1) == Some(b'\n') {
            edge -= 1;
            if edge > line.start && text.byte_at(edge - 1) == Some(b'\r') {
                edge -= 1;
            }
        }
        value.grapheme_of_byte(edge)
    }

    fn delete_by(&self, reach: Reach) -> bool {
        if self.read_only {
            return false;
        }
        let forward = matches!(reach, Reach::WordRight | Reach::LineEnd);
        // With a selection the key deletes that, as a plain Backspace or Delete does.
        if self.cursor.selection_range().is_some() {
            if forward {
                self.delete_forward();
            } else {
                self.delete_backward();
            }
            return true;
        }
        let focus = self.cursor.offset();
        let target = self.reach_target(reach);
        if target == focus {
            return true;
        }
        let (from, to) = if forward { (focus, target) } else { (target, focus) };
        if self.controller.replace_selection_graphemes(from, to, "", None) {
            self.sync_cursor_from_controller();
            self.on_changed.call_with(|| self.controller.text());
        }
        true
    }
}
