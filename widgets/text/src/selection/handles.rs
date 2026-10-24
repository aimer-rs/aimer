//! The two knobs a finger drags to adjust a selection.
//!
//! A finger is far wider than a caret, so a touch selection that could only be
//! made by dragging across the glyphs would be impossible to correct: the very
//! hand adjusting it hides what it is adjusting. Every touch platform therefore
//! grows a knob at each end of the selection, offset away from the text so the
//! finger covers the knob instead of the characters.
//!
//! Everything here is pure geometry in absolute logical coordinates: given the
//! caret rectangle of an endpoint it says where the knob is drawn and whether a
//! press grabbed it. Nothing reads the clock, the canvas or the session, so the
//! layout is exercised with hand-written rectangles.

use aimer_attribute::Bounds;

/// Radius of the round knob, in logical pixels.
pub(crate) const HANDLE_RADIUS: f32 = 6.0;

/// Width of the bar drawn along the caret, in logical pixels.
pub(crate) const HANDLE_BAR_WIDTH: f32 = 2.0;

/// How far outside the knob a press still grabs it, in logical pixels.
///
/// A fingertip is about nine millimetres across and lands where it *looks*,
/// which is above where it touches, so the grabbable area is far larger than
/// the twelve-pixel dot it grabs.
pub(crate) const HANDLE_TOUCH_SLOP: f32 = 12.0;

/// Which end of the selection a knob belongs to.
///
/// The sides are in *document* order, not gesture order: [`HandleSide::Start`]
/// is always the earlier point in the text, whichever end the finger began
/// from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HandleSide {
    /// The knob at the beginning of the selection, drawn above its caret.
    Start,
    /// The knob at the end of the selection, drawn below its caret.
    End,
}

/// The circle a knob is painted as, and pressed on.
///
/// # Examples
///
/// ```ignore
/// let caret = Bounds::new(20.0, 40.0, 2.0, 16.0);
/// let knob = HandleCircle::of(caret, HandleSide::Start);
///
/// assert_eq!(knob.center_y, 40.0 - HANDLE_RADIUS);
/// assert!(knob.contains(21.0, 34.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HandleCircle {
    pub center_x: f32,
    pub center_y: f32,
    /// The caret the knob is attached to, which is drawn as the bar.
    pub caret: Bounds,
    pub side: HandleSide,
}

impl HandleCircle {
    /// Places the knob of `side` against the caret rectangle `caret`.
    ///
    /// The start knob sits above the line and the end knob below it, so the two
    /// never overlap on a one-word selection and the hand covering one does not
    /// cover the other.
    pub fn of(caret: Bounds, side: HandleSide) -> Self {
        let center_x = caret.x + caret.width * 0.5;
        let center_y = match side {
            HandleSide::Start => caret.y - HANDLE_RADIUS,
            HandleSide::End => caret.y + caret.height + HANDLE_RADIUS,
        };
        Self {
            center_x,
            center_y,
            caret,
            side,
        }
    }

    /// The square the knob is painted into.
    #[inline]
    pub fn circle_bounds(&self) -> Bounds {
        Bounds::new(
            self.center_x - HANDLE_RADIUS,
            self.center_y - HANDLE_RADIUS,
            HANDLE_RADIUS * 2.0,
            HANDLE_RADIUS * 2.0,
        )
    }

    /// The bar drawn along the caret, thickened so it reads as a stem rather
    /// than as a hairline.
    #[inline]
    pub fn bar_bounds(&self) -> Bounds {
        Bounds::new(
            self.center_x - HANDLE_BAR_WIDTH * 0.5,
            self.caret.y,
            HANDLE_BAR_WIDTH,
            self.caret.height,
        )
    }

    /// The rectangle a press must land in to grab this knob: the knob grown by
    /// [`HANDLE_TOUCH_SLOP`] and stretched to cover the bar, so a finger aiming
    /// at the stem still takes the handle rather than starting a new selection.
    ///
    /// This is the one place the grabbable area is described. A press is tested
    /// against it by [`Self::contains`], and the element that answers that press
    /// reports it as part of its own box — routing hit-tests, so a knob outside
    /// the box its region claims is a knob no press can ever reach. See
    /// [`grab_span`].
    #[inline]
    pub fn grab_bounds(&self) -> Bounds {
        let reach = HANDLE_RADIUS + HANDLE_TOUCH_SLOP;
        let top = self.caret.y.min(self.center_y) - HANDLE_TOUCH_SLOP;
        let bottom = (self.caret.y + self.caret.height).max(self.center_y) + HANDLE_TOUCH_SLOP;
        Bounds::new(self.center_x - reach, top, reach * 2.0, bottom - top)
    }

    /// Reports whether a press at `(x, y)` grabbed this knob.
    #[inline]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.grab_bounds().is_inside(x, y)
    }
}

/// The rectangle both knobs of a selection can be grabbed in.
///
/// A selection's furniture reaches past its glyphs — above the first line and
/// below the last — so the element that answers a press on a knob has to claim
/// this much. Purely the union of the two [`HandleCircle::grab_bounds`]: the
/// area between them belongs to the text, which is already claiming it.
#[inline]
pub(crate) fn grab_span(start: HandleCircle, end: HandleCircle) -> Bounds {
    let start = start.grab_bounds();
    let end = end.grab_bounds();
    let x = start.x.min(end.x);
    let y = start.y.min(end.y);
    let right = (start.x + start.width).max(end.x + end.width);
    let bottom = (start.y + start.height).max(end.y + end.height);
    Bounds::new(x, y, right - x, bottom - y)
}

/// Picks the knob a press grabbed, preferring the nearer one when the two
/// overlap on a very short selection.
pub(crate) fn handle_at(
    start: HandleCircle,
    end: HandleCircle,
    x: f32,
    y: f32,
) -> Option<HandleSide> {
    let hits_start = start.contains(x, y);
    let hits_end = end.contains(x, y);
    match (hits_start, hits_end) {
        (true, true) => {
            let to_start = squared_distance(start, x, y);
            let to_end = squared_distance(end, x, y);
            Some(if to_start <= to_end {
                HandleSide::Start
            } else {
                HandleSide::End
            })
        }
        (true, false) => Some(HandleSide::Start),
        (false, true) => Some(HandleSide::End),
        (false, false) => None,
    }
}

fn squared_distance(circle: HandleCircle, x: f32, y: f32) -> f32 {
    let dx = x - circle.center_x;
    let dy = y - circle.center_y;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret(x: f32, y: f32) -> Bounds {
        Bounds::new(x, y, 2.0, 16.0)
    }

    #[test]
    fn the_start_knob_sits_above_its_caret_and_the_end_knob_below() {
        let start = HandleCircle::of(caret(20.0, 40.0), HandleSide::Start);
        let end = HandleCircle::of(caret(80.0, 40.0), HandleSide::End);

        assert_eq!(start.center_y, 40.0 - HANDLE_RADIUS);
        assert_eq!(end.center_y, 40.0 + 16.0 + HANDLE_RADIUS);
    }

    #[test]
    fn the_knob_is_centred_on_the_caret() {
        let knob = HandleCircle::of(caret(20.0, 40.0), HandleSide::Start);

        assert_eq!(knob.center_x, 21.0);
        assert_eq!(knob.circle_bounds().x, 21.0 - HANDLE_RADIUS);
        assert_eq!(knob.bar_bounds().width, HANDLE_BAR_WIDTH);
        assert_eq!(knob.bar_bounds().height, 16.0);
    }

    #[test]
    fn the_grabbable_rectangle_is_the_area_a_press_is_tested_against() {
        let knob = HandleCircle::of(caret(20.0, 40.0), HandleSide::Start);
        let grab = knob.grab_bounds();

        assert!(
            grab.x < knob.circle_bounds().x,
            "a fingertip is wider than the dot it aims at"
        );
        assert!(
            grab.y < knob.center_y - HANDLE_RADIUS,
            "and reaches above it"
        );
        assert!(
            grab.y + grab.height >= knob.caret.y + knob.caret.height,
            "down to the foot of the bar"
        );
        for (x, y) in [
            (grab.x, grab.y),
            (grab.x + grab.width, grab.y + grab.height),
            (grab.x, grab.y + grab.height),
        ] {
            assert!(knob.contains(x, y), "every corner of it grabs the knob");
        }
        assert!(
            !knob.contains(grab.x - 1.0, knob.center_y),
            "and nothing outside does"
        );
    }

    #[test]
    fn the_grabbable_span_covers_both_knobs_of_a_selection() {
        let start = HandleCircle::of(caret(20.0, 40.0), HandleSide::Start);
        let end = HandleCircle::of(caret(80.0, 400.0), HandleSide::End);

        let span = grab_span(start, end);

        assert!(span.is_inside(start.center_x, start.center_y));
        assert!(span.is_inside(end.center_x, end.center_y));
        assert!(
            span.y < 40.0 - HANDLE_RADIUS,
            "the span reaches above the first line, where the start knob hangs"
        );
        assert!(
            span.y + span.height > 400.0 + 16.0 + HANDLE_RADIUS,
            "and below the last, where the end knob hangs"
        );
    }

    #[test]
    fn a_press_beside_the_knob_still_grabs_it() {
        let knob = HandleCircle::of(caret(20.0, 40.0), HandleSide::Start);

        assert!(knob.contains(21.0, 34.0), "on the knob");
        assert!(
            knob.contains(21.0 + HANDLE_RADIUS + 4.0, 34.0),
            "just beside"
        );
        assert!(knob.contains(21.0, 50.0), "on the bar");
    }

    #[test]
    fn a_press_far_from_the_knob_misses_it() {
        let knob = HandleCircle::of(caret(20.0, 40.0), HandleSide::Start);

        assert!(!knob.contains(200.0, 34.0));
        assert!(!knob.contains(21.0, 400.0));
    }

    #[test]
    fn overlapping_knobs_go_to_the_nearer_one() {
        // A one-character selection: both knobs are within reach of the middle.
        let start = HandleCircle::of(caret(20.0, 40.0), HandleSide::Start);
        let end = HandleCircle::of(caret(30.0, 40.0), HandleSide::End);

        assert_eq!(handle_at(start, end, 21.0, 33.0), Some(HandleSide::Start));
        assert_eq!(handle_at(start, end, 31.0, 63.0), Some(HandleSide::End));
    }

    #[test]
    fn a_press_on_neither_knob_grabs_nothing() {
        let start = HandleCircle::of(caret(20.0, 40.0), HandleSide::Start);
        let end = HandleCircle::of(caret(80.0, 40.0), HandleSide::End);

        assert_eq!(handle_at(start, end, 50.0, 200.0), None);
    }
}
