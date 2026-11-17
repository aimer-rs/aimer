use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aimer_style::{TextAlign, TextStyle};
use aimer_text::{RawTextWidget, StyledRange};

use crate::editable_text::EditableGeometry;

/// Source of the identities that tell two highlighters apart. Zero is reserved
/// for "no highlighter", so a cache key can hold the id of an absent one.
static NEXT_HIGHLIGHTER_ID: AtomicU64 = AtomicU64::new(1);

/// Colours, backgrounds and decorations for ranges of a field's text.
///
/// A highlighter turns the text of a [`TextField`](crate::input::TextField) or
/// [`TextArea`](crate::TextArea) into a list of [`StyledRange`]s. It is how a
/// code editor colours its tokens or a chat composer marks `@mentions`, while
/// the text stays one plain string: editing, selection, undo and input methods
/// are untouched, and the caret keeps following the glyphs because a range may
/// only change how text looks, not how wide it is.
///
/// The function receives the text exactly as the field displays it — which
/// leaves out an input-method composition that has not been committed yet — and
/// returns byte ranges into that string. Ranges need not be sorted, may be
/// empty or run past the end, and may overlap; the earlier-starting range wins
/// the shared bytes. See [`StyledRange`].
///
/// The function runs again only when the displayed text changes, not when the
/// caret moves, so it may do real work such as tokenising. It must be pure: the
/// same text has to give the same ranges, or the field would show stale styling.
/// A highlighter that needs to know where the cursor is, to show markup only on
/// the line being edited, is made with [`TextHighlighter::with_selection`].
///
/// # Reuse the handle
///
/// A highlighter is identified by the value it was created as, and clones share
/// that identity. Create it once and clone it into each build; building a new
/// one on every rebuild makes the field treat its styling as changed and
/// recompute it every time.
///
/// # Weight and size
///
/// Ranges that change the font weight, size, family or spacing change how wide
/// the text is. The field lays the text out with them applied, so the caret and
/// selection stay correct, but the text reflows as the user types. Colour,
/// background and decoration never reflow.
///
/// A secure field shows bullets instead of its text and ignores its highlighter.
///
/// # Examples
///
/// ```
/// use aimer_input::input::{SpanStyle, StyledRange, TextHighlighter};
/// use aimer_widget::base::Color;
///
/// // Marks every `@name` in a message.
/// let mentions = TextHighlighter::new(|text| {
///     let style = SpanStyle::new().color(Color::Rgba(40, 110, 230, 255));
///     text.match_indices('@')
///         .map(|(start, _)| {
///             let end = text[start..]
///                 .find(char::is_whitespace)
///                 .map_or(text.len(), |length| start + length);
///             StyledRange::new(start..end, style)
///         })
///         .collect()
/// });
///
/// let ranges = mentions.highlight("hi @alice and @bob");
/// assert_eq!(ranges.len(), 2);
/// assert_eq!(ranges[0].range, 3..9);
/// ```
#[derive(Clone)]
pub struct TextHighlighter {
    id: u64,
    highlight: Rc<dyn Fn(&str, &HighlightContext) -> Vec<StyledRange>>,
    by_selection: bool,
}

impl TextHighlighter {
    /// Creates a highlighter from a function that styles ranges of a text.
    ///
    /// The function must be pure; see the [type documentation](TextHighlighter).
    #[inline]
    pub fn new<F>(highlight: F) -> Self
    where
        F: Fn(&str) -> Vec<StyledRange> + 'static,
    {
        Self {
            id: NEXT_HIGHLIGHTER_ID.fetch_add(1, Ordering::Relaxed),
            highlight: Rc::new(move |text, _| highlight(text)),
            by_selection: false,
        }
    }

    /// Creates a highlighter that styles the text a line at a time, remembering
    /// what it did, so an edit costs the lines it changed and not the whole text.
    ///
    /// `highlight_line` receives a line of the text, with its line feed if it has
    /// one, and the state the previous line left behind (`initial` for the first),
    /// and returns the ranges of the line, relative to its start, and the state it
    /// leaves for the next. A state is what carries from one line to the next:
    /// whether a fenced code block or a block comment is open, say. It must depend
    /// on nothing but those two arguments.
    ///
    /// After an edit the lines before the first change and after the last are not
    /// run again, and neither are the changed ones' neighbours, except that a line
    /// that now begins in another state is, and so is every line after it until
    /// the state is one the old text had there. Highlighting a note a line at a
    /// time is then as costly as the lines that changed, where a function of the
    /// whole text is as costly as the note.
    ///
    /// Create the highlighter once and clone it into each build, as with
    /// [`new`](Self::new): the lines it remembers are shared by its clones.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::input::{SpanStyle, StyledRange, TextHighlighter};
    /// use aimer_widget::base::Color;
    ///
    /// // Colours the lines of fenced blocks; the state is whether a block is open.
    /// let blocks = TextHighlighter::by_line(false, |line, in_block| {
    ///     let is_fence = line.starts_with("```");
    ///     let body = line.trim_end_matches('\n').len();
    ///     let ranges = if (*in_block || is_fence) && body > 0 {
    ///         vec![StyledRange::new(0..body, SpanStyle::new().color(Color::Rgba(60, 120, 60, 255)))]
    ///     } else {
    ///         Vec::new()
    ///     };
    ///     (ranges, *in_block != is_fence)
    /// });
    /// # let _ = blocks;
    /// ```
    pub fn by_line<S, F>(initial: S, highlight_line: F) -> Self
    where
        S: Clone + PartialEq + 'static,
        F: Fn(&str, &S) -> (Vec<StyledRange>, S) + 'static,
    {
        let lines = crate::input_field::line_highlight::LineHighlighter::new(initial, highlight_line);
        Self::new(move |text| lines.highlight(text))
    }

    /// Creates a highlighter that also knows where the cursor is.
    ///
    /// The function receives a [`HighlightContext`] next to the text, so it can
    /// style a line one way while the cursor is on it and another way while it
    /// is not. That is how a markdown editor shows `# Title` as a heading, and
    /// the `# ` itself only while the cursor is on the line.
    ///
    /// It runs again when the text changes and when the cursor moves to another
    /// *line*, or when the field gains or loses focus, since those change which
    /// lines are active. Moving within a line costs nothing. It must still be
    /// pure: the same text and context have to give the same ranges.
    ///
    /// Ranges that change width, such as one styled with
    /// [`SpanStyle::concealed`], reflow the text when the cursor reaches the
    /// line, and a click on collapsed text puts the cursor there before the
    /// line widens.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::input::{SpanStyle, StyledRange, TextHighlighter};
    ///
    /// // Hides the `# ` of a heading while the cursor is somewhere else.
    /// let headings = TextHighlighter::with_selection(|text, context| {
    ///     let end = text.find('\n').unwrap_or(text.len());
    ///     if text.starts_with("# ") && !context.is_active(&(0..end)) {
    ///         vec![StyledRange::new(0..2, SpanStyle::concealed())]
    ///     } else {
    ///         Vec::new()
    ///     }
    /// });
    ///
    /// assert_eq!(headings.highlight("# Hello").len(), 1, "no cursor, no active line");
    /// ```
    #[inline]
    pub fn with_selection<F>(highlight: F) -> Self
    where
        F: Fn(&str, &HighlightContext) -> Vec<StyledRange> + 'static,
    {
        Self {
            id: NEXT_HIGHLIGHTER_ID.fetch_add(1, Ordering::Relaxed),
            highlight: Rc::new(highlight),
            by_selection: true,
        }
    }

    /// Runs the highlighter over `text` with no cursor, as in a field that does
    /// not have focus.
    #[inline]
    pub fn highlight(&self, text: &str) -> Vec<StyledRange> {
        (self.highlight)(text, &HighlightContext::default())
    }

    /// Runs the highlighter over `text` for a cursor described by `context`.
    #[inline]
    pub(crate) fn highlight_in(&self, text: &str, context: &HighlightContext) -> Vec<StyledRange> {
        (self.highlight)(text, context)
    }

    /// Whether the ranges depend on the cursor, which is what decides if a move
    /// of the cursor can change them.
    #[inline]
    pub(crate) fn depends_on_selection(&self) -> bool {
        self.by_selection
    }

    /// The identity shared by this highlighter and its clones.
    #[inline]
    pub(crate) fn id(&self) -> u64 {
        self.id
    }
}

/// Where the cursor is, handed to a [`TextHighlighter::with_selection`] function.
///
/// Offsets are byte offsets into the text the function receives, like the ranges
/// it returns. A field without focus has no cursor: [`selection`](Self::selection)
/// and [`active_lines`](Self::active_lines) are `None` and nothing is active, so
/// everything shows its rendered form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HighlightContext {
    selection: Option<Range<usize>>,
    active_lines: Option<Range<usize>>,
}

impl HighlightContext {
    /// The context of a cursor or selection `selection` in `text`.
    ///
    /// A field builds this itself; it is public so a highlighter can be tested
    /// without one. A selection that is not on character boundaries of `text`
    /// panics, and one past its end is clamped.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::input::HighlightContext;
    ///
    /// let context = HighlightContext::new("one\ntwo", 5..5);
    /// assert_eq!(context.active_lines(), Some(4..7));
    /// assert!(context.is_active(&(4..7)));
    /// assert!(!context.is_active(&(0..3)));
    /// ```
    pub fn new(text: &str, selection: Range<usize>) -> Self {
        let start = selection.start.min(text.len());
        let end = selection.end.clamp(start, text.len());
        let first = text[..start].rfind('\n').map_or(0, |newline| newline + 1);
        let last = text[end..].find('\n').map_or(text.len(), |newline| end + newline);
        Self {
            selection: Some(start..end),
            active_lines: Some(first..last),
        }
    }

    /// The selected bytes, empty for a plain caret, or `None` without focus.
    #[inline]
    pub fn selection(&self) -> Option<Range<usize>> {
        self.selection.clone()
    }

    /// This context as it is for the part of the text that starts `start` bytes in: the
    /// selection and the active lines moved back by that much.
    ///
    /// A highlighter that styles a long text a piece at a time asks each piece which of its
    /// ranges the cursor is on, and the answer is the one the whole text would give. Lines
    /// that end before the piece starts are not active in it, and those that start after
    /// its end are active in none of its ranges, whatever `start` and the piece's length are.
    #[must_use]
    pub fn shifted_back(&self, start: usize) -> Self {
        let back = |range: &Range<usize>| range.start.saturating_sub(start)..range.end.saturating_sub(start);
        Self {
            selection: self.selection.as_ref().map(back),
            // Active lines that end before `start` are active nowhere in the piece.
            active_lines: self.active_lines.as_ref().filter(|lines| lines.end >= start).map(back),
        }
    }

    /// The whole lines the cursor or selection touches, from the start of the
    /// first to the end of the last, not counting the line break, or `None`
    /// without focus.
    #[inline]
    pub fn active_lines(&self) -> Option<Range<usize>> {
        self.active_lines.clone()
    }

    /// Whether any part of `range` lies on a line the cursor is on.
    ///
    /// This is the question a renderer asks to decide between a line's rendered
    /// form and its source.
    #[inline]
    pub fn is_active(&self, range: &Range<usize>) -> bool {
        self.active_lines
            .as_ref()
            .is_some_and(|lines| range.start <= lines.end && range.end >= lines.start)
    }
}

impl std::fmt::Debug for TextHighlighter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextHighlighter")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The id a cache key holds for a field without a highlighter.
#[inline]
pub(crate) fn highlighter_id(highlighter: Option<&TextHighlighter>) -> u64 {
    highlighter.map_or(0, TextHighlighter::id)
}

/// The ranges a field last computed, and what they were computed from.
///
/// The key is the highlighter and the displayed text itself, not the
/// controller's revision: a caret move advances the revision without touching
/// the text, and must not run the highlighter again.
#[derive(Default)]
pub(crate) struct HighlightCache {
    cached: RefCell<Option<CachedHighlight>>,
}

struct CachedHighlight {
    highlighter: u64,
    text: Arc<str>,
    /// The lines the ranges were computed for; `None` for a highlighter that
    /// does not look at the cursor, whose ranges stay valid wherever it is.
    active: Option<Range<usize>>,
    ranges: Rc<[StyledRange]>,
}

impl HighlightCache {
    /// Returns the ranges for `text`, running `highlighter` only when it or the
    /// text differs from what the last call saw. `None` when there is no
    /// highlighter or it styled nothing, which is the plain-text fast path.
    ///
    /// This asks as a field without focus would. See [`Self::resolve_in`].
    #[inline]
    pub(crate) fn resolve(
        &self,
        highlighter: Option<&TextHighlighter>,
        text: &Arc<str>,
    ) -> Option<Rc<[StyledRange]>> {
        self.resolve_in(highlighter, text, &HighlightContext::default())
    }

    /// Like [`Self::resolve`], for a cursor described by `context`. A
    /// highlighter that depends on the cursor also runs again when the active
    /// lines differ from the last call's; one that does not never looks at it.
    pub(crate) fn resolve_in(
        &self,
        highlighter: Option<&TextHighlighter>,
        text: &Arc<str>,
        context: &HighlightContext,
    ) -> Option<Rc<[StyledRange]>> {
        let highlighter = highlighter?;
        if text.is_empty() {
            return None;
        }
        let active = highlighter
            .depends_on_selection()
            .then(|| context.active_lines())
            .flatten();
        let mut cached = self.cached.borrow_mut();
        if let Some(hit) = cached.as_ref()
            && hit.highlighter == highlighter.id()
            && hit.active == active
            && (Arc::ptr_eq(&hit.text, text) || hit.text == *text)
        {
            return (!hit.ranges.is_empty()).then(|| Rc::clone(&hit.ranges));
        }
        let ranges: Rc<[StyledRange]> = highlighter.highlight_in(text, context).into();
        *cached = Some(CachedHighlight {
            highlighter: highlighter.id(),
            text: Arc::clone(text),
            active,
            ranges: Rc::clone(&ranges),
        });
        (!ranges.is_empty()).then_some(ranges)
    }

    /// Hands the cache of a field being replaced to its successor, so a rebuild
    /// that leaves the text alone does not run the highlighter again.
    pub(crate) fn adopt(&self, old: &Self) {
        *self.cached.borrow_mut() = old.cached.borrow_mut().take();
    }
}

/// The widget that lays out and paints a field's highlighted text, kept for as
/// long as the geometry, style and alignment it was built for are current.
///
/// A frame asks for that widget several times — to check it can paint, to
/// measure its outsets, to record it — and a fresh widget starts with an empty
/// layout cache, so each ask would shape the whole paragraph again. Plain text
/// never needs this: its single run is measured by the canvas, not prepared as
/// a paragraph.
#[derive(Default)]
pub(crate) struct RetainedDisplay {
    retained: RefCell<Option<Retained>>,
}

struct Retained {
    // Held strongly so the pointer comparison below cannot be fooled by a new
    // geometry allocated at the address of a dropped one.
    geometry: Rc<EditableGeometry>,
    style: TextStyle,
    align: TextAlign,
    widget: Rc<RawTextWidget>,
}

impl RetainedDisplay {
    /// Returns the retained widget when it was built for exactly this
    /// `geometry`, `style` and `align`; otherwise builds one with `build` and
    /// keeps it.
    pub(crate) fn resolve(
        &self,
        geometry: &Rc<EditableGeometry>,
        style: &TextStyle,
        align: TextAlign,
        build: impl FnOnce() -> RawTextWidget,
    ) -> Rc<RawTextWidget> {
        let mut retained = self.retained.borrow_mut();
        if let Some(hit) = retained.as_ref()
            && Rc::ptr_eq(&hit.geometry, geometry)
            && hit.style == *style
            && hit.align == align
        {
            return Rc::clone(&hit.widget);
        }
        let widget = Rc::new(build());
        *retained = Some(Retained {
            geometry: Rc::clone(geometry),
            style: *style,
            align,
            widget: Rc::clone(&widget),
        });
        widget
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use aimer_text::SpanStyle;

    use super::*;

    /// A highlighter that counts how often it runs and styles `needle`.
    fn counting(needle: &'static str) -> (TextHighlighter, Rc<Cell<u32>>) {
        let runs = Rc::new(Cell::new(0));
        let counted = Rc::clone(&runs);
        let highlighter = TextHighlighter::new(move |text| {
            counted.set(counted.get() + 1);
            text.find(needle)
                .map(|start| StyledRange::new(start..start + needle.len(), SpanStyle::new()))
                .into_iter()
                .collect()
        });
        (highlighter, runs)
    }

    #[test]
    fn a_line_highlighter_resolves_through_the_cache_a_line_at_a_time() {
        let calls = Rc::new(Cell::new(0));
        let counted = Rc::clone(&calls);
        let highlighter = TextHighlighter::by_line((), move |line: &str, _: &()| {
            counted.set(counted.get() + 1);
            let ranges = line
                .find("TODO")
                .map(|start| StyledRange::new(start..start + 4, SpanStyle::new()))
                .into_iter()
                .collect();
            (ranges, ())
        });
        let cache = HighlightCache::default();
        let first = text("one\nTODO two\nthree\n");
        let ranges = cache.resolve(Some(&highlighter), &first).expect("one range");
        assert_eq!(ranges[0].range, 4..8);
        assert_eq!(calls.get(), 3);

        // A character typed in the last line: only that line is run, and the range
        // of the line before it is where it was.
        let edited = text("one\nTODO two\nthree!\n");
        let ranges = cache.resolve(Some(&highlighter), &edited).expect("one range");
        assert_eq!(ranges[0].range, 4..8);
        assert_eq!(calls.get(), 4);

        // A line added above moves the range, and runs only the new line.
        let moved = text("zero\none\nTODO two\nthree!\n");
        let ranges = cache.resolve(Some(&highlighter), &moved).expect("one range");
        assert_eq!(ranges[0].range, 9..13);
        assert_eq!(calls.get(), 5);
    }

    #[test]
    fn a_context_shifted_back_answers_for_a_piece_as_the_whole_would() {
        let text = "one\ntwo\nthree\nfour\n";
        // The cursor on `three`, which is bytes 8..13.
        let context = HighlightContext::new(text, 10..10);
        assert_eq!(context.active_lines(), Some(8..13));

        // A piece that starts at `three`: its own first line is the active one.
        let piece = context.shifted_back(8);
        assert_eq!(piece.active_lines(), Some(0..5));
        assert!(piece.is_active(&(0..5)));
        // A piece that starts after the active line has nothing active in it...
        let after = context.shifted_back(14);
        assert_eq!(after.active_lines(), None);
        // ...and one that ends before it is asked about ranges that do not reach it.
        let before = context.shifted_back(0);
        assert!(!before.is_active(&(0..3)) && before.is_active(&(8..13)));
        // The line that ends exactly where the piece starts touches it, as it would in the whole.
        let touching = context.shifted_back(13);
        assert_eq!(touching.active_lines(), Some(0..0));
        assert!(touching.is_active(&(0..4)));
    }

    fn text(value: &str) -> Arc<str> {
        Arc::from(value)
    }

    #[test]
    fn clones_share_an_identity_and_new_highlighters_do_not() {
        let (first, _) = counting("a");
        let (second, _) = counting("a");

        assert_eq!(first.clone().id(), first.id());
        assert_ne!(first.id(), second.id());
        assert_ne!(first.id(), 0, "zero means no highlighter");
        assert_eq!(highlighter_id(None), 0);
        assert_eq!(highlighter_id(Some(&first)), first.id());
    }

    #[test]
    fn the_same_text_runs_the_highlighter_once() {
        let (highlighter, runs) = counting("let");
        let cache = HighlightCache::default();
        let source = text("let x = 1");

        let first = cache.resolve(Some(&highlighter), &source).unwrap();
        // A different `Arc` holding equal text is a hit too: the controller
        // hands the field a fresh string after every caret move.
        let second = cache.resolve(Some(&highlighter), &text("let x = 1")).unwrap();

        assert_eq!(runs.get(), 1);
        assert!(Rc::ptr_eq(&first, &second));
    }

    #[test]
    fn changed_text_runs_the_highlighter_again() {
        let (highlighter, runs) = counting("let");
        let cache = HighlightCache::default();

        cache.resolve(Some(&highlighter), &text("let x"));
        let changed = cache.resolve(Some(&highlighter), &text("let xy")).unwrap();

        assert_eq!(runs.get(), 2);
        assert_eq!(changed[0].range, 0..3);
    }

    #[test]
    fn a_different_highlighter_runs_even_on_the_same_text() {
        let (first, first_runs) = counting("let");
        let (second, second_runs) = counting("x");
        let cache = HighlightCache::default();
        let source = text("let x");

        cache.resolve(Some(&first), &source);
        let swapped = cache.resolve(Some(&second), &source).unwrap();

        assert_eq!((first_runs.get(), second_runs.get()), (1, 1));
        assert_eq!(swapped[0].range, 4..5);
    }

    #[test]
    fn no_highlighter_or_nothing_styled_is_the_plain_path() {
        let (highlighter, runs) = counting("zzz");
        let cache = HighlightCache::default();

        assert!(cache.resolve(None, &text("let x")).is_none());
        assert!(cache.resolve(Some(&highlighter), &text("let x")).is_none());
        // The empty answer is cached as well.
        assert!(cache.resolve(Some(&highlighter), &text("let x")).is_none());
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn empty_text_is_never_highlighted() {
        let (highlighter, runs) = counting("a");
        let cache = HighlightCache::default();

        assert!(cache.resolve(Some(&highlighter), &text("")).is_none());
        assert_eq!(runs.get(), 0);
    }

    #[test]
    fn an_adopted_cache_spares_the_rebuild_a_highlight() {
        let (highlighter, runs) = counting("let");
        let old = HighlightCache::default();
        old.resolve(Some(&highlighter), &text("let x"));

        let new = HighlightCache::default();
        new.adopt(&old);
        let kept = new.resolve(Some(&highlighter), &text("let x"));

        assert!(kept.is_some());
        assert_eq!(runs.get(), 1);
    }

    /// A selection-aware highlighter that counts its runs.
    fn counting_by_selection() -> (TextHighlighter, Rc<Cell<u32>>) {
        let runs = Rc::new(Cell::new(0));
        let counted = Rc::clone(&runs);
        let highlighter = TextHighlighter::with_selection(move |text, context| {
            counted.set(counted.get() + 1);
            // Styles the first line, but only while the cursor is elsewhere.
            let end = text.find('\n').unwrap_or(text.len());
            if context.is_active(&(0..end)) {
                Vec::new()
            } else {
                vec![StyledRange::new(0..end, SpanStyle::new())]
            }
        });
        (highlighter, runs)
    }

    #[test]
    fn the_active_lines_are_the_whole_lines_a_selection_touches() {
        const TEXT: &str = "one\ntwo\nthree";

        assert_eq!(HighlightContext::new(TEXT, 5..5).active_lines(), Some(4..7));
        assert_eq!(HighlightContext::new(TEXT, 3..3).active_lines(), Some(0..3), "the end of a line is on it");
        assert_eq!(HighlightContext::new(TEXT, 4..4).active_lines(), Some(4..7), "and the start of the next is on that one");
        assert_eq!(HighlightContext::new(TEXT, 5..9).active_lines(), Some(4..13));
        assert_eq!(HighlightContext::new(TEXT, 5..9).selection(), Some(5..9));
        assert_eq!(HighlightContext::new("", 0..0).active_lines(), Some(0..0));
    }

    #[test]
    fn a_range_is_active_when_it_shares_a_line_with_the_cursor() {
        let context = HighlightContext::new("one\ntwo\nthree", 5..5);

        assert!(context.is_active(&(4..7)));
        assert!(context.is_active(&(5..6)));
        assert!(!context.is_active(&(0..3)));
        assert!(!context.is_active(&(8..13)));
        assert!(!HighlightContext::default().is_active(&(0..3)), "an unfocused field has no active line");
    }

    #[test]
    fn a_selection_aware_highlighter_runs_again_only_when_the_active_lines_change() {
        let (highlighter, runs) = counting_by_selection();
        let cache = HighlightCache::default();
        let source = text("title\nbody text");

        let on_body = cache.resolve_in(Some(&highlighter), &source, &HighlightContext::new(&source, 8..8));
        assert!(on_body.is_some(), "the title is styled while the cursor is on the body");

        cache.resolve_in(Some(&highlighter), &source, &HighlightContext::new(&source, 11..11));
        assert_eq!(runs.get(), 1, "moving within a line asks nothing new of it");

        let on_title = cache.resolve_in(Some(&highlighter), &source, &HighlightContext::new(&source, 2..2));
        assert_eq!(runs.get(), 2);
        assert!(on_title.is_none(), "the cursor on the title takes its styling off");

        let unfocused = cache.resolve_in(Some(&highlighter), &source, &HighlightContext::default());
        assert_eq!(runs.get(), 3, "losing focus changes what is active too");
        assert!(unfocused.is_some());
    }

    #[test]
    fn a_plain_highlighter_ignores_where_the_cursor_is() {
        let (highlighter, runs) = counting("let");
        let cache = HighlightCache::default();
        let source = text("let x\nlet y");

        cache.resolve_in(Some(&highlighter), &source, &HighlightContext::new(&source, 1..1));
        cache.resolve_in(Some(&highlighter), &source, &HighlightContext::new(&source, 8..8));
        cache.resolve_in(Some(&highlighter), &source, &HighlightContext::default());

        assert_eq!(runs.get(), 1);
    }
}
