use std::cell::RefCell;
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
    highlight: Rc<dyn Fn(&str) -> Vec<StyledRange>>,
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
            highlight: Rc::new(highlight),
        }
    }

    /// Runs the highlighter over `text`.
    #[inline]
    pub fn highlight(&self, text: &str) -> Vec<StyledRange> {
        (self.highlight)(text)
    }

    /// The identity shared by this highlighter and its clones.
    #[inline]
    pub(crate) fn id(&self) -> u64 {
        self.id
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
    ranges: Rc<[StyledRange]>,
}

impl HighlightCache {
    /// Returns the ranges for `text`, running `highlighter` only when it or the
    /// text differs from what the last call saw. `None` when there is no
    /// highlighter or it styled nothing, which is the plain-text fast path.
    pub(crate) fn resolve(
        &self,
        highlighter: Option<&TextHighlighter>,
        text: &Arc<str>,
    ) -> Option<Rc<[StyledRange]>> {
        let highlighter = highlighter?;
        if text.is_empty() {
            return None;
        }
        let mut cached = self.cached.borrow_mut();
        if let Some(hit) = cached.as_ref()
            && hit.highlighter == highlighter.id()
            && (Arc::ptr_eq(&hit.text, text) || hit.text == *text)
        {
            return (!hit.ranges.is_empty()).then(|| Rc::clone(&hit.ranges));
        }
        let ranges: Rc<[StyledRange]> = highlighter.highlight(text).into();
        *cached = Some(CachedHighlight {
            highlighter: highlighter.id(),
            text: Arc::clone(text),
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
}
