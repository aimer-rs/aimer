//! The file drag the window is currently under.
//!
//! The platform reports a file drag *entering* the window, one event per file,
//! and then says nothing at all until it is dropped or leaves — while the user
//! goes on moving it, possibly across half a dozen drop zones. A region that
//! only ever heard about the entry lights up where the drag came in and stays
//! that way, which is what this tracker exists to prevent: it remembers the
//! batch in flight and the last place it was seen, so the windowing layer can
//! re-report the drag as [`ElementEvent::HoveredFileMoved`] for every position
//! it finds it at.
//!
//! [`ElementEvent::HoveredFileMoved`]: aimer_events::element::ElementEvent::HoveredFileMoved

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aimer_attribute::position::Vec2d;

/// How far the drag must travel before it is worth reporting again.
///
/// A sub-pixel step cannot change which element is under the cursor, and every
/// report costs a hit test through the tree.
const MIN_STEP: f32 = 1.0;

/// The batch of files being dragged over the window, if any.
///
/// Empty is the overwhelmingly common state, and costs one length check to
/// recognize.
#[derive(Debug, Default)]
pub(crate) struct FileDrag {
    /// Every path the platform has reported for this drag, in order.
    paths: Vec<PathBuf>,
    /// The batch as the events carry it, rebuilt only when `paths` changes so a
    /// move costs a reference-count bump rather than a copy of the batch.
    shared: Option<Arc<[PathBuf]>>,
    /// Where the drag was last reported.
    at: Vec2d,
    /// Whether the last report reached anything.
    ///
    /// A drag wandering over the background reaches nothing, and whoever lit up
    /// before has to be told — once, on the move that leaves it, not on every
    /// move that follows.
    answered: bool,
}

impl FileDrag {
    /// Creates a tracker with no drag in flight.
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            paths: Vec::new(),
            shared: None,
            at: Vec2d { x: 0.0, y: 0.0 },
            answered: false,
        }
    }

    /// Whether a file drag is over the window right now.
    #[inline]
    pub(crate) fn is_active(&self) -> bool {
        !self.paths.is_empty()
    }

    /// Records a file the platform has just announced, at `at`.
    ///
    /// A five-file drag announces five files and is still one drag, so the paths
    /// accumulate instead of replacing each other.
    pub(crate) fn enter(&mut self, path: &Path, at: Vec2d) {
        if !self.paths.iter().any(|known| known == path) {
            self.paths.push(path.to_path_buf());
            self.shared = None;
        }
        self.at = at;
        // The announcement itself was hit-tested, so a zone may well have lit
        // up; assuming it did is what makes the first move away from it report
        // the leave.
        self.answered = true;
    }

    /// The batch to report for a drag that has moved to `at`, or `None` if there
    /// is nothing in flight or it has not moved far enough to matter.
    pub(crate) fn moved_to(&mut self, at: Vec2d) -> Option<Arc<[PathBuf]>> {
        if !self.is_active() {
            return None;
        }
        if (at.x - self.at.x).abs() < MIN_STEP && (at.y - self.at.y).abs() < MIN_STEP {
            return None;
        }
        self.at = at;
        Some(self.batch())
    }

    /// Records whether the last report reached anything, and answers whether the
    /// drag has just left everything that was listening.
    pub(crate) fn note_answered(&mut self, answered: bool) -> bool {
        let left = self.answered && !answered;
        self.answered = answered;
        left
    }

    /// Forgets the drag: it was dropped, or it left the window.
    pub(crate) fn finish(&mut self) {
        self.paths.clear();
        self.shared = None;
        self.answered = false;
    }

    /// The batch as the events carry it.
    fn batch(&mut self) -> Arc<[PathBuf]> {
        self.shared
            .get_or_insert_with(|| Arc::from(self.paths.as_slice()))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> Vec2d {
        Vec2d { x, y }
    }

    #[test]
    fn nothing_is_reported_while_no_drag_is_in_flight() {
        let mut drag = FileDrag::new();

        assert!(!drag.is_active());
        assert!(drag.moved_to(at(10.0, 10.0)).is_none());
    }

    #[test]
    fn a_drag_that_travels_is_reported_with_its_whole_batch() {
        let mut drag = FileDrag::new();
        drag.enter(Path::new("/tmp/a.png"), at(10.0, 10.0));
        drag.enter(Path::new("/tmp/b.png"), at(10.0, 10.0));

        let batch = drag.moved_to(at(40.0, 10.0)).expect("the drag has moved");

        assert_eq!(
            &*batch,
            [PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.png")]
        );
    }

    /// The same file announced twice is one file: winit repeats a path when a
    /// drag re-enters the window without having been dropped.
    #[test]
    fn the_same_path_is_never_recorded_twice() {
        let mut drag = FileDrag::new();
        drag.enter(Path::new("/tmp/a.png"), at(10.0, 10.0));
        drag.enter(Path::new("/tmp/a.png"), at(12.0, 10.0));

        let batch = drag.moved_to(at(40.0, 10.0)).expect("the drag has moved");

        assert_eq!(&*batch, [PathBuf::from("/tmp/a.png")]);
    }

    #[test]
    fn a_step_too_small_to_change_anything_is_not_reported() {
        let mut drag = FileDrag::new();
        drag.enter(Path::new("/tmp/a.png"), at(10.0, 10.0));

        assert!(drag.moved_to(at(10.4, 10.4)).is_none());
        assert!(drag.moved_to(at(11.5, 10.0)).is_some());
    }

    #[test]
    fn a_finished_drag_reports_nothing_more() {
        let mut drag = FileDrag::new();
        drag.enter(Path::new("/tmp/a.png"), at(10.0, 10.0));
        drag.finish();

        assert!(!drag.is_active());
        assert!(drag.moved_to(at(60.0, 60.0)).is_none());
    }

    /// The leave is worth one broadcast, on the move that leaves — not on every
    /// move the drag then spends over the background.
    #[test]
    fn leaving_everything_is_answered_once() {
        let mut drag = FileDrag::new();
        drag.enter(Path::new("/tmp/a.png"), at(10.0, 10.0));

        assert!(drag.note_answered(false), "the leave must be reported");
        assert!(!drag.note_answered(false), "and reported only once");
        assert!(!drag.note_answered(true), "arriving is not a leave");
        assert!(drag.note_answered(false), "leaving again is");
    }
}
