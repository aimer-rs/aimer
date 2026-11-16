//! Widgets embedded between the lines of a text field.
//!
//! A [`TextEmbedder`] reads the text of a field and answers with the ranges to
//! replace by a widget. The field reserves a full-width box in the line for each
//! one, the way a tall inline image would, mounts the widget over it, and sizes
//! the box to the widget. The text itself is untouched, so editing, undo and
//! copy keep working on the source; an embed only changes what is drawn.
//!
//! An embedder sees where the cursor is, so it can embed a block while the
//! cursor is elsewhere and give the source back, for editing, when the cursor
//! enters it. See [`HighlightContext`].

use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use aimer_widget::{AnyWidget, Widget};

use crate::input_field::highlight::HighlightContext;

/// Source of the identities that tell two embedders apart. Zero is reserved for
/// "no embedder", so a cache key can hold the id of an absent one.
static NEXT_EMBEDDER_ID: AtomicU64 = AtomicU64::new(1);

/// How tall an embed's box is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EmbedHeight {
    /// The widget's own height for the width of the field, found by measuring it
    /// with an unbounded height. The default.
    Measured,
    /// A height in logical pixels.
    Fixed(f32),
}

/// One range of a field's text shown as a widget.
///
/// `key` is the identity of what is shown: an embed keeps its widget, and any
/// state in it, for as long as the embedder keeps answering with the same key,
/// and builds a new one when the key changes. Derive it from what the widget
/// displays, such as a hash of the source it renders.
///
/// # Examples
///
/// ```
/// use aimer_input::input::TextEmbed;
/// use aimer_widget::base::Color;
/// use aimer_container::SizedBox;
///
/// // A 40 px tall box in place of the first line.
/// let embed = TextEmbed::new(0..10, 1, || SizedBox::new().height(40)).fixed_height(40.0);
/// assert_eq!(embed.range(), 0..10);
/// ```
#[derive(Clone)]
pub struct TextEmbed {
    range: Range<usize>,
    key: u64,
    height: EmbedHeight,
    builder: Rc<dyn Fn() -> AnyWidget>,
}

impl TextEmbed {
    /// An embed of the bytes `range` of the text, drawn by the widget `builder`
    /// makes. The range should cover whole lines, since the widget takes the
    /// full width of the field.
    #[inline]
    pub fn new<W>(range: Range<usize>, key: u64, builder: impl Fn() -> W + 'static) -> Self
    where
        W: Widget + 'static,
    {
        Self {
            range,
            key,
            height: EmbedHeight::Measured,
            builder: Rc::new(move || builder().boxed()),
        }
    }

    /// Sets the box to a fixed height in logical pixels instead of measuring the
    /// widget.
    #[inline]
    #[must_use]
    pub fn fixed_height(mut self, height: f32) -> Self {
        self.height = EmbedHeight::Fixed(height);
        self
    }

    /// The replaced bytes of the text.
    #[inline]
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// The identity of what the embed shows.
    #[inline]
    pub fn key(&self) -> u64 {
        self.key
    }

    /// How the height of the box is decided.
    #[inline]
    pub fn height(&self) -> EmbedHeight {
        self.height
    }

    #[inline]
    pub(crate) fn build(&self) -> AnyWidget {
        (self.builder)()
    }
}

impl std::fmt::Debug for TextEmbed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextEmbed")
            .field("range", &self.range)
            .field("key", &self.key)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Decides which ranges of a field's text are shown as widgets.
///
/// The function receives the text exactly as the field displays it and a
/// [`HighlightContext`] with the cursor, and returns the embeds. It runs again
/// when the text changes and when the cursor moves to another line, like a
/// cursor-aware [`TextHighlighter`](crate::input::TextHighlighter), and it must
/// be pure. Ranges that overlap one another are not supported, and ranges that
/// overlap a highlighter's are drawn as embeds.
///
/// A highlighter and an embedder usually go together, since both read the same
/// parse; create them once and clone them into each build.
#[derive(Clone)]
pub struct TextEmbedder {
    id: u64,
    embeds: Rc<dyn Fn(&str, &HighlightContext) -> Vec<TextEmbed>>,
}

impl TextEmbedder {
    /// Creates an embedder from a function that answers with the embeds of a text.
    #[inline]
    pub fn new<F>(embeds: F) -> Self
    where
        F: Fn(&str, &HighlightContext) -> Vec<TextEmbed> + 'static,
    {
        Self {
            id: NEXT_EMBEDDER_ID.fetch_add(1, Ordering::Relaxed),
            embeds: Rc::new(embeds),
        }
    }

    /// Runs the embedder over `text` for a cursor described by `context`.
    #[inline]
    pub fn embeds(&self, text: &str, context: &HighlightContext) -> Vec<TextEmbed> {
        (self.embeds)(text, context)
    }

    /// The identity shared by this embedder and its clones.
    #[inline]
    pub(crate) fn id(&self) -> u64 {
        self.id
    }
}

impl std::fmt::Debug for TextEmbedder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextEmbedder")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The id a cache key holds for a field without an embedder.
#[inline]
pub(crate) fn embedder_id(embedder: Option<&TextEmbedder>) -> u64 {
    embedder.map_or(0, TextEmbedder::id)
}

#[cfg(test)]
mod tests {
    use aimer_container::SizedBox;

    use super::*;

    #[test]
    fn an_embed_keeps_its_range_key_and_height_rule() {
        let embed = TextEmbed::new(4..9, 7, || SizedBox::new().height(10));

        assert_eq!((embed.range(), embed.key(), embed.height()), (4..9, 7, EmbedHeight::Measured));
        assert_eq!(embed.fixed_height(30.0).height(), EmbedHeight::Fixed(30.0));
    }

    #[test]
    fn clones_share_an_identity_and_new_embedders_do_not() {
        let first = TextEmbedder::new(|_, _| Vec::new());
        let second = TextEmbedder::new(|_, _| Vec::new());

        assert_eq!(first.clone().id(), first.id());
        assert_ne!(first.id(), second.id());
        assert_ne!(first.id(), 0, "zero means no embedder");
        assert_eq!(embedder_id(None), 0);
    }

    #[test]
    fn the_embedder_sees_the_text_and_the_cursor() {
        let embedder = TextEmbedder::new(|text, context| {
            let end = text.find('\n').unwrap_or(text.len());
            if context.is_active(&(0..end)) {
                Vec::new()
            } else {
                vec![TextEmbed::new(0..end, 1, || SizedBox::new().height(10))]
            }
        });
        let text = "first\nsecond";

        assert_eq!(embedder.embeds(text, &HighlightContext::new(text, 8..8)).len(), 1);
        assert!(embedder.embeds(text, &HighlightContext::new(text, 2..2)).is_empty());
    }
}
