pub(crate) mod cursor;
pub(crate) mod handles;
mod participant;
pub(crate) mod selectable;
pub(crate) mod session;
pub(crate) mod touch_hold;
pub(crate) mod ui;

use std::ops::Range;
use std::rc::Rc;

use aimer_attribute::Bounds;
use aimer_widget::PointerKey;

use crate::selection::session::SelectionSlot;

pub use participant::SelectionParticipant;

/// One painted text range used by a custom selection participant.
///
/// `source_range` uses byte offsets, matching the selection engine's text
/// snapshots. The bounds are absolute logical window coordinates, so a
/// participant can live below translated or scrolled elements without teaching
/// the selection engine about its canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionRegion {
    source_range: Range<usize>,
    bounds: Bounds,
}

impl SelectionRegion {
    /// Creates a selectable range occupying `bounds`.
    #[inline]
    pub const fn new(source_range: Range<usize>, bounds: Bounds) -> Self {
        Self {
            source_range,
            bounds,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextHitRegion {
    pub source_range: Range<usize>,
    pub bounds: Bounds,
}

impl From<SelectionRegion> for TextHitRegion {
    #[inline]
    fn from(region: SelectionRegion) -> Self {
        Self::new(region.source_range, region.bounds)
    }
}

impl TextHitRegion {
    pub const fn new(source_range: Range<usize>, bounds: Bounds) -> Self {
        Self {
            source_range,
            bounds,
        }
    }
}

pub(crate) fn text_offset_at(regions: &[TextHitRegion], x: f32, y: f32) -> Option<usize> {
    let region = regions.iter().min_by(|left, right| {
        vertical_distance(left.bounds, y)
            .total_cmp(&vertical_distance(right.bounds, y))
            .then_with(|| {
                distance_squared(left.bounds, x, y).total_cmp(&distance_squared(right.bounds, x, y))
            })
    })?;
    let midpoint = region.bounds.x + region.bounds.width / 2.0;
    Some(if x < midpoint {
        region.source_range.start
    } else {
        region.source_range.end
    })
}

fn vertical_distance(bounds: Bounds, y: f32) -> f32 {
    if y < bounds.y {
        bounds.y - y
    } else if y > bounds.y + bounds.height {
        y - (bounds.y + bounds.height)
    } else {
        0.0
    }
}

fn distance_squared(bounds: Bounds, x: f32, y: f32) -> f32 {
    let dx = if x < bounds.x {
        bounds.x - x
    } else if x > bounds.x + bounds.width {
        x - (bounds.x + bounds.width)
    } else {
        0.0
    };
    let dy = if y < bounds.y {
        bounds.y - y
    } else if y > bounds.y + bounds.height {
        y - (bounds.y + bounds.height)
    } else {
        0.0
    };
    dx * dx + dy * dy
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSelection {
    anchor: usize,
    focus: usize,
}

impl TextSelection {
    pub const fn new(anchor: usize, focus: usize) -> Self {
        Self { anchor, focus }
    }

    pub const fn collapsed(offset: usize) -> Self {
        Self::new(offset, offset)
    }

    pub const fn anchor(self) -> usize {
        self.anchor
    }

    pub const fn focus(self) -> usize {
        self.focus
    }

    #[inline]
    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.focus)..self.anchor.max(self.focus)
    }

    pub const fn is_collapsed(self) -> bool {
        self.anchor == self.focus
    }

    #[inline]
    pub fn selected_text(self, text: &str) -> Option<&str> {
        text.get(self.range())
    }
}

/// One end of a selection: a participant plus a UTF-8 offset inside it.
#[derive(Clone)]
pub(crate) struct SelectionPoint {
    pub slot: Rc<SelectionSlot>,
    pub offset: usize,
}

impl SelectionPoint {
    /// Creates a point at `offset` inside `slot`.
    #[inline]
    pub const fn new(slot: Rc<SelectionSlot>, offset: usize) -> Self {
        Self { slot, offset }
    }

    /// Reports whether both points address the same offset of the same
    /// participant.
    #[inline]
    pub fn is_same(&self, other: &Self) -> bool {
        self.offset == other.offset && Rc::ptr_eq(&self.slot, &other.slot)
    }
}

/// A selection spanning from an anchor to a focus, each in its own participant.
///
/// The pair keeps the gesture's direction: `anchor` is where the pointer went
/// down and `focus` is where it currently is.
#[derive(Clone)]
pub(crate) struct PointSelection {
    pub anchor: SelectionPoint,
    pub focus: SelectionPoint,
}

impl PointSelection {
    /// Creates an empty selection at `point`.
    #[inline]
    pub fn collapsed(point: SelectionPoint) -> Self {
        Self {
            anchor: point.clone(),
            focus: point,
        }
    }

    /// Reports whether the selection covers nothing.
    #[inline]
    pub fn is_collapsed(&self) -> bool {
        self.anchor.is_same(&self.focus)
    }
}

/// Pointer-driven selection bookkeeping shared by every session.
///
/// The state owns exactly one gesture at a time: only the pointer that began a
/// drag may extend or end it, and cancelling restores the selection from before
/// that pointer went down.
#[derive(Default)]
pub(crate) struct SelectionState {
    selection: Option<PointSelection>,
    selection_before_gesture: Option<Option<PointSelection>>,
    active_pointer: Option<PointerKey>,
    dragged: bool,
}

impl SelectionState {
    /// Collapses the selection at `point` and takes ownership of `pointer`.
    pub fn begin(&mut self, point: SelectionPoint, pointer: PointerKey) {
        self.selection_before_gesture = Some(self.selection.take());
        self.selection = Some(PointSelection::collapsed(point));
        self.active_pointer = Some(pointer);
        self.dragged = false;
    }

    /// Moves the focus to `point`, ignoring pointers that did not begin the
    /// gesture.
    pub fn update(&mut self, point: SelectionPoint, pointer: PointerKey) -> bool {
        if self.active_pointer != Some(pointer) {
            return false;
        }
        let Some(selection) = &mut self.selection else {
            return false;
        };
        selection.focus = point;
        self.dragged |= !selection.is_collapsed();
        true
    }

    /// Commits the gesture owned by `pointer`.
    pub fn end(&mut self, pointer: PointerKey) -> bool {
        if self.active_pointer != Some(pointer) {
            return false;
        }
        self.active_pointer = None;
        self.selection_before_gesture = None;
        true
    }

    /// Restores the selection from before the current gesture.
    pub fn cancel(&mut self) {
        if let Some(selection) = self.selection_before_gesture.take() {
            self.selection = selection;
        }
        self.active_pointer = None;
        self.dragged = false;
    }

    /// Drops the selection and any gesture in progress.
    pub fn clear(&mut self) {
        self.selection = None;
        self.selection_before_gesture = None;
        self.active_pointer = None;
        self.dragged = false;
    }

    /// Replaces the selection outside of any gesture.
    pub fn set(&mut self, selection: PointSelection) {
        self.selection = Some(selection);
        self.selection_before_gesture = None;
        self.active_pointer = None;
        self.dragged = false;
    }

    /// Replaces the selection without touching the gesture in progress.
    ///
    /// Used when a participant's text changed under a live selection.
    pub fn replace_selection(&mut self, selection: PointSelection) {
        self.selection = Some(selection);
    }

    /// The current selection, if there is one.
    pub fn selection(&self) -> Option<&PointSelection> {
        self.selection.as_ref()
    }

    /// The pointer that began the current gesture.
    pub const fn active_pointer(&self) -> Option<PointerKey> {
        self.active_pointer
    }

    /// Reports whether the current gesture ever covered more than one offset.
    pub const fn was_dragged(&self) -> bool {
        self.dragged
    }
}

#[cfg(test)]
mod tests {
    use aimer_attribute::Bounds;

    use super::{TextHitRegion, TextSelection, text_offset_at};

    #[test]
    fn reversed_selection_normalizes_without_losing_direction() {
        let selection = TextSelection::new(8, 2);

        assert_eq!(selection.anchor(), 8);
        assert_eq!(selection.focus(), 2);
        assert_eq!(selection.range(), 2..8);
        assert!(!selection.is_collapsed());
    }

    #[test]
    fn collapsed_selection_has_an_empty_range() {
        let selection = TextSelection::collapsed(4);

        assert_eq!(selection.range(), 4..4);
        assert!(selection.is_collapsed());
    }

    #[test]
    fn selected_text_preserves_unicode_and_line_breaks() {
        let text = "Aé\n👩‍💻Z";
        let selection = TextSelection::new(1, text.len() - 1);

        assert_eq!(selection.selected_text(text), Some("é\n👩‍💻"));
    }

    #[test]
    fn selected_text_rejects_non_utf8_boundaries() {
        let selection = TextSelection::new(1, 2);

        assert_eq!(selection.selected_text("é"), None);
    }

    #[test]
    fn hit_testing_chooses_the_nearest_grapheme_boundary() {
        let regions = vec![
            TextHitRegion::new(0..2, Bounds::new(10.0, 20.0, 10.0, 12.0)),
            TextHitRegion::new(2..3, Bounds::new(20.0, 20.0, 10.0, 12.0)),
        ];

        assert_eq!(text_offset_at(&regions, 14.0, 25.0), Some(0));
        assert_eq!(text_offset_at(&regions, 16.0, 25.0), Some(2));
        assert_eq!(text_offset_at(&regions, 26.0, 25.0), Some(3));
    }

    #[test]
    fn hit_testing_clamps_outside_a_line_to_its_nearest_edge() {
        let regions = vec![
            TextHitRegion::new(0..1, Bounds::new(10.0, 20.0, 10.0, 12.0)),
            TextHitRegion::new(1..2, Bounds::new(20.0, 20.0, 10.0, 12.0)),
        ];

        assert_eq!(text_offset_at(&regions, -100.0, 25.0), Some(0));
        assert_eq!(text_offset_at(&regions, 100.0, 25.0), Some(2));
    }

    #[test]
    fn hit_testing_below_short_final_line_reaches_end_of_text() {
        let regions = vec![
            TextHitRegion::new(0..1, Bounds::new(10.0, 20.0, 100.0, 10.0)),
            TextHitRegion::new(1..2, Bounds::new(10.0, 30.0, 10.0, 10.0)),
        ];

        assert_eq!(text_offset_at(&regions, 200.0, 50.0), Some(2));
    }

    #[test]
    fn hit_testing_above_short_first_line_reaches_start_of_text() {
        let regions = vec![
            TextHitRegion::new(0..1, Bounds::new(100.0, 20.0, 10.0, 10.0)),
            TextHitRegion::new(1..2, Bounds::new(10.0, 30.0, 100.0, 10.0)),
        ];

        assert_eq!(text_offset_at(&regions, -100.0, 10.0), Some(0));
    }
}
