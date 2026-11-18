use aimer_sum_tree::Bias;

use crate::buffer::EditDelta;

/// A name for a position that follows the text it is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnchorId(u64);

/// Positions in a text that stay with the text as it is edited: a caret, the ends of a selection,
/// the start of an embedded widget.
///
/// Each carries a [`Bias`]: whether it sticks to the text before it (left) or after it (right) when
/// text is put in exactly where it is. [`apply`](Self::apply) moves every anchor by an edit, as
/// [`EditDelta::transform`] does, in time proportional to the number of anchors (a handful: a
/// caret and a selection).
///
/// # Examples
///
/// ```
/// use aimer_text_buffer::{Anchors, Bias, TextBuffer};
///
/// let mut buffer = TextBuffer::from_text("hello world");
/// let mut anchors = Anchors::new();
/// let world = anchors.insert(6, Bias::Right);
/// let delta = buffer.replace(0..0, "oh, ");
/// anchors.apply(&delta);
/// assert_eq!(anchors.get(world), Some(10));
/// ```
#[derive(Clone, Default, Debug)]
pub struct Anchors {
    next: u64,
    entries: Vec<(AnchorId, usize, Bias)>,
}

impl Anchors {
    /// No anchors.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an anchor at `offset` and returns its name.
    pub fn insert(&mut self, offset: usize, bias: Bias) -> AnchorId {
        let id = AnchorId(self.next);
        self.next += 1;
        self.entries.push((id, offset, bias));
        id
    }

    /// Takes an anchor away; returns where it was, if it was there.
    pub fn remove(&mut self, id: AnchorId) -> Option<usize> {
        let at = self.entries.iter().position(|entry| entry.0 == id)?;
        Some(self.entries.swap_remove(at).1)
    }

    /// Where the anchor is now.
    pub fn get(&self, id: AnchorId) -> Option<usize> {
        self.entries.iter().find(|entry| entry.0 == id).map(|entry| entry.1)
    }

    /// Moves the anchor to `offset`, keeping its bias.
    pub fn set(&mut self, id: AnchorId, offset: usize) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.0 == id) {
            entry.1 = offset;
        }
    }

    /// How many anchors there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Moves every anchor by an edit.
    pub fn apply(&mut self, delta: &EditDelta) {
        for entry in &mut self.entries {
            entry.1 = delta.transform(entry.1, entry.2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextBuffer;

    struct Random(u64);

    impl Random {
        fn below(&mut self, below: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % below as u64) as usize
        }
    }

    /// Anchors on marker characters, edits that never touch the markers, and the markers are
    /// still where the anchors say: before the edit nothing moves, after it everything moves by
    /// what was added.
    #[test]
    fn anchors_stay_on_the_text_they_were_put_on_through_random_edits() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        for round in 0..60 {
            let mut buffer = TextBuffer::from_text("aaaa");
            // "aaaa<1>bbbb<2>cccc<3>" with every marker a character of its own.
            let mut text = String::from("aaaa");
            let mut anchors = Anchors::new();
            let mut markers = Vec::new();
            for marker in ['\u{1}', '\u{2}', '\u{3}', '\u{4}'] {
                let at = text.len();
                text.push(marker);
                text.push_str("bbbb");
                buffer.replace(at..at, &format!("{marker}bbbb"));
                // An anchor on the marker sticks to it from either side: right to what is after, left to what is before.
                let on = anchors.insert(at, Bias::Right);
                let after = anchors.insert(at + 1, Bias::Left);
                markers.push((marker, on, after));
            }
            for edit in 0..25 {
                let marker_at: Vec<usize> = markers.iter().map(|(_, id, _)| anchors.get(*id).expect("anchored")).collect();
                // An edit between two markers, or before the first or after the last.
                let gap = random.below(markers.len() + 1);
                let low = if gap == 0 { 0 } else { marker_at[gap - 1] + 1 };
                let high = if gap == markers.len() { buffer.text().len() } else { marker_at[gap] };
                let boundaries: Vec<usize> = (low..=high).filter(|&at| buffer.text().is_char_boundary(at)).collect();
                let a = boundaries[random.below(boundaries.len())];
                let b = boundaries[random.below(boundaries.len())];
                let inserted = ["", "x", "yy\n", "\u{e9}\u{4f60}"][random.below(4)];
                let delta = buffer.replace(a.min(b)..a.max(b), inserted);
                anchors.apply(&delta);
                for (marker, id, after) in &markers {
                    let at = anchors.get(*id).expect("anchored");
                    assert_eq!(buffer.text().slice(at..at + 1), marker.to_string(), "round {round} edit {edit}: anchor on {marker:?}");
                    assert_eq!(anchors.get(*after), Some(at + 1), "round {round} edit {edit}: the anchor after {marker:?}");
                }
            }
        }
    }

    #[test]
    fn an_anchor_is_added_moved_and_removed() {
        let mut anchors = Anchors::new();
        assert!(anchors.is_empty());
        let a = anchors.insert(3, Bias::Left);
        let b = anchors.insert(5, Bias::Right);
        assert_eq!((anchors.get(a), anchors.get(b), anchors.len()), (Some(3), Some(5), 2));
        anchors.set(a, 9);
        assert_eq!(anchors.get(a), Some(9));
        assert_eq!(anchors.remove(a), Some(9));
        assert_eq!((anchors.get(a), anchors.remove(a), anchors.len()), (None, None, 1));
    }
}
