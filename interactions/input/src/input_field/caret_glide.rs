//! The smooth motion of a caret from one insertion point to the next.

use std::time::Duration;

use aimer_animation::AnimInstant;

/// A caret rectangle in the field's logical coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CaretRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl CaretRect {
    #[inline]
    fn lerp(self, to: Self, t: f32) -> Self {
        let mix = |from: f32, to: f32| from + (to - from) * t;
        Self {
            x: mix(self.x, to.x),
            y: mix(self.y, to.y),
            width: mix(self.width, to.width),
            height: mix(self.height, to.height),
        }
    }
}

/// Moves a caret to where the cursor went instead of jumping there.
///
/// The field publishes where its caret *is*; a glide decides where it is
/// *drawn*. When the cursor moves to another insertion point, the drawn caret
/// travels there over [`CaretGlide::DURATION`] with a decelerating curve, and a
/// cursor that moves again meanwhile redirects the caret from wherever it has
/// got to, so typing quickly keeps one continuous motion rather than a series
/// of restarts.
///
/// Only a *cursor* move glides. A caret whose rectangle changes while the
/// cursor stays put is following its text, which scrolled or reflowed, and a
/// caret that lagged behind the glyphs it stands between would look broken, so
/// it snaps. The first caret after a [`reset`](Self::reset) appears in place for
/// the same reason.
///
/// The clock is a parameter, so the motion is deterministic under test.
#[derive(Debug, Default)]
pub(crate) struct CaretGlide {
    from: CaretRect,
    target: CaretRect,
    started: Option<AnimInstant>,
    offset: usize,
    placed: bool,
}

impl CaretGlide {
    /// How long a caret takes to reach its new position. Short enough that it
    /// never trails fast typing, long enough to read as motion.
    pub(crate) const DURATION: Duration = Duration::from_millis(120);

    /// Forgets where the caret was, so the next one appears where it belongs.
    #[inline]
    pub(crate) fn reset(&mut self) {
        self.placed = false;
        self.started = None;
    }

    /// Takes the caret's published rectangle and the cursor offset it belongs
    /// to, and returns the rectangle to draw at `now` and whether the caret is
    /// still on its way, which is when the next frame is needed.
    pub(crate) fn advance(
        &mut self,
        target: CaretRect,
        offset: usize,
        now: AnimInstant,
    ) -> (CaretRect, bool) {
        if !self.placed {
            self.snap(target);
            self.placed = true;
        } else if target != self.target {
            if offset != self.offset {
                self.from = self.at(now);
                self.started = Some(now);
            } else {
                self.snap(target);
            }
        }
        self.target = target;
        self.offset = offset;
        (self.at(now), self.is_moving(now))
    }

    #[inline]
    fn snap(&mut self, target: CaretRect) {
        self.from = target;
        self.target = target;
        self.started = None;
    }

    #[inline]
    fn progress(&self, now: AnimInstant) -> Option<f32> {
        let started = self.started?;
        let t = now.duration_since(started).as_secs_f32() / Self::DURATION.as_secs_f32();
        (t < 1.0).then_some(t.max(0.0))
    }

    #[inline]
    fn is_moving(&self, now: AnimInstant) -> bool {
        self.progress(now).is_some()
    }

    /// The rectangle to draw at `now`.
    fn at(&self, now: AnimInstant) -> CaretRect {
        match self.progress(now) {
            Some(t) => {
                let eased = 1.0 - (1.0 - t).powi(3);
                self.from.lerp(self.target, eased)
            }
            None => self.target,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> CaretRect {
        CaretRect {
            x,
            y,
            width: 1.5,
            height: 18.0,
        }
    }

    #[test]
    fn the_first_caret_appears_where_it_belongs() {
        let mut glide = CaretGlide::default();
        let now = AnimInstant::now();

        let (shown, moving) = glide.advance(at(40.0, 10.0), 3, now);

        assert_eq!(shown, at(40.0, 10.0));
        assert!(!moving);
    }

    #[test]
    fn a_moved_cursor_glides_to_its_new_position() {
        let mut glide = CaretGlide::default();
        let start = AnimInstant::now();
        glide.advance(at(0.0, 0.0), 0, start);

        let (first, moving) = glide.advance(at(100.0, 0.0), 1, start);
        assert_eq!(first, at(0.0, 0.0), "it leaves from where it was");
        assert!(moving);

        let mut last = 0.0;
        for step in 1..9 {
            let (shown, moving) =
                glide.advance(at(100.0, 0.0), 1, start + CaretGlide::DURATION * step / 10);
            assert!(shown.x > last, "it keeps advancing: {} after {last}", shown.x);
            assert!(shown.x < 100.0 && moving, "and is still on its way at step {step}");
            last = shown.x;
        }

        let (arrived, moving) = glide.advance(at(100.0, 0.0), 1, start + CaretGlide::DURATION);
        assert_eq!(arrived, at(100.0, 0.0));
        assert!(!moving, "it stops asking for frames once it has arrived");
    }

    #[test]
    fn a_glide_decelerates_into_its_target() {
        let mut glide = CaretGlide::default();
        let start = AnimInstant::now();
        glide.advance(at(0.0, 0.0), 0, start);
        glide.advance(at(100.0, 0.0), 1, start);

        let half = glide
            .advance(at(100.0, 0.0), 1, start + CaretGlide::DURATION / 2)
            .0
            .x;

        assert!(half > 50.0, "an ease-out covers most of the way early: {half}");
    }

    #[test]
    fn a_cursor_that_moves_again_redirects_from_where_the_caret_is() {
        let mut glide = CaretGlide::default();
        let start = AnimInstant::now();
        glide.advance(at(0.0, 0.0), 0, start);
        glide.advance(at(100.0, 0.0), 1, start);
        let midway = start + CaretGlide::DURATION / 2;
        let before = glide.advance(at(100.0, 0.0), 1, midway).0;

        let after = glide.advance(at(140.0, 0.0), 2, midway).0;

        assert_eq!(after, before, "the caret does not jump when it is redirected");
    }

    #[test]
    fn a_caret_that_moves_with_its_text_snaps() {
        let mut glide = CaretGlide::default();
        let start = AnimInstant::now();
        glide.advance(at(40.0, 100.0), 7, start);

        // The same cursor offset, but the field scrolled under it.
        let (shown, moving) = glide.advance(at(40.0, 60.0), 7, start);

        assert_eq!(shown, at(40.0, 60.0));
        assert!(!moving);
    }

    #[test]
    fn a_caret_that_comes_back_after_a_reset_appears_in_place() {
        let mut glide = CaretGlide::default();
        let start = AnimInstant::now();
        glide.advance(at(0.0, 0.0), 0, start);
        glide.reset();

        let (shown, moving) = glide.advance(at(200.0, 50.0), 9, start);

        assert_eq!(shown, at(200.0, 50.0));
        assert!(!moving);
    }

    #[test]
    fn a_caret_that_stays_put_never_asks_for_a_frame() {
        let mut glide = CaretGlide::default();
        let start = AnimInstant::now();
        glide.advance(at(10.0, 10.0), 1, start);

        for step in 0..5u32 {
            let (_, moving) = glide.advance(at(10.0, 10.0), 1, start + Duration::from_millis(16) * step);
            assert!(!moving);
        }
    }

    #[test]
    fn the_caret_height_glides_with_it_between_lines_of_different_size() {
        let mut glide = CaretGlide::default();
        let start = AnimInstant::now();
        let body = CaretRect { height: 18.0, ..at(0.0, 40.0) };
        let title = CaretRect { height: 26.0, ..at(0.0, 0.0) };
        glide.advance(body, 20, start);
        glide.advance(title, 5, start);

        let (shown, _) = glide.advance(title, 5, start + CaretGlide::DURATION / 2);

        assert!(shown.height > 18.0 && shown.height < 26.0, "{}", shown.height);
    }
}
