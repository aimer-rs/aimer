//! The shaping of long texts, kept so an edit can start from it.
//!
//! A long note changes a character at a time, and shaping all of it for each one
//! costs in proportion to its length. The canvas keeps the shaping of the few long
//! texts it has seen last, and a text that is close to one of them is shaped from
//! it: see [`shape_text_styled_after_edit`](aimer_cupid_font::shape_text_styled_after_edit).
//! Short texts stay in the canvas's ordinary shaping cache.

use std::rc::Rc;

use aimer_cupid_font::font::{FontFamily, FontStyle, TextLanguage};
use aimer_cupid_font::shared_edge_bytes;
use aimer_cupid_font::text_layout::ShapedText;

/// From this many bytes a text is kept here and not in the ordinary cache.
pub(crate) const LARGE_TEXT_BYTES: usize = 2048;

/// How many long texts are kept. A few documents on screen at once, each its own
/// text area, are a normal working set; the oldest is dropped past this.
const CAPACITY: usize = 4;

/// What a shaping was made for, other than the text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShapeStyle {
    pub font_size_bits: u32,
    pub font_family: FontFamily,
    pub font_style: FontStyle,
    pub font_weight: u16,
    pub language: Option<TextLanguage>,
    pub font_revision: u64,
}

struct Entry {
    style: ShapeStyle,
    shaped: Rc<ShapedText>,
}

/// The long texts shaped last, the most recent at the end.
#[derive(Default)]
pub(crate) struct LargeShapings {
    entries: Vec<Entry>,
}

impl LargeShapings {
    /// The shaping of exactly `text` in `style`, marking it the most recent.
    pub(crate) fn exact(&mut self, text: &str, style: &ShapeStyle) -> Option<Rc<ShapedText>> {
        let at = self
            .entries
            .iter()
            .rposition(|entry| entry.style == *style && entry.shaped.text == text)?;
        let entry = self.entries.remove(at);
        let shaped = Rc::clone(&entry.shaped);
        self.entries.push(entry);
        Some(shaped)
    }

    /// Takes the shaping in `style` that shares the most with `text`, if it is at
    /// least half of it, so little enough changed to be worth starting from.
    ///
    /// The text is edited in place from here on, so it leaves the store; the
    /// shaping of `text` goes in with [`Self::insert`].
    pub(crate) fn take_similar(&mut self, text: &str, style: &ShapeStyle) -> Option<ShapedText> {
        let (at, shared) = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.style == *style)
            .map(|(at, entry)| (at, shared_edge_bytes(&entry.shaped.text, text)))
            .max_by_key(|&(_, shared)| shared)?;
        if shared * 2 < text.len() {
            return None;
        }
        let entry = self.entries.remove(at);
        // Nothing else holds it unless a caller is still using the last frame's
        // result, in which case it is copied and that caller keeps its own.
        Some(Rc::try_unwrap(entry.shaped).unwrap_or_else(|shared| (*shared).clone()))
    }

    /// Remembers `shaped`, the most recent, dropping the oldest past the capacity.
    pub(crate) fn insert(&mut self, style: ShapeStyle, shaped: Rc<ShapedText>) {
        if self.entries.len() >= CAPACITY {
            self.entries.remove(0);
        }
        self.entries.push(Entry { style, shaped });
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}
