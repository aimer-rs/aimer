//! Where the seek rail's pieces go: platform-free, in the rail's own logical
//! coordinates.
//!
//! The rail sits under a `Slider`, whose thumb moves linearly across the
//! width, so everything here is on the same linear scale: chapters are
//! segments of the rail with a small gap carved out at each boundary, and the
//! played part ends exactly under the thumb.

use std::time::Duration;

use crate::Chapters;

/// The gap carved at each chapter boundary, in logical pixels.
pub(crate) const CHAPTER_GAP: f32 = 3.0;

/// One horizontal piece of the rail: `x` and `width` in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Span {
    pub x: f32,
    pub width: f32,
}

impl Span {
    /// The part of this span left of `x`.
    pub(crate) fn clipped_to(self, x: f32) -> Option<Span> {
        let width = (x - self.x).min(self.width);
        (width > 0.0).then_some(Span { x: self.x, width })
    }
}

/// The fraction of the video at `time`, `0.0` for an unknown length.
pub(crate) fn fraction(time: Duration, duration: Option<Duration>) -> f32 {
    match duration {
        Some(total) if !total.is_zero() => (time.as_secs_f64() / total.as_secs_f64()).clamp(0.0, 1.0) as f32,
        _ => 0.0,
    }
}

/// Where `fraction` of the video lies on a rail `width` wide whose thumb
/// centre is kept `inset` in from each end, as the slider's is.
pub(crate) fn x_of(fraction: f32, width: f32, inset: f32) -> f32 {
    let inset = inset.clamp(0.0, width.max(0.0) / 2.0);
    inset + fraction.clamp(0.0, 1.0) * (width - inset * 2.0).max(0.0)
}

/// The rail's segments on a rail `width` wide: one per chapter, or one for a
/// video without chapters (or of unknown length). Boundaries use the same
/// scale as the slider's thumb ([`x_of`]); the ends reach the rail's edges.
pub(crate) fn segments(width: f32, inset: f32, duration: Option<Duration>, chapters: &Chapters) -> Vec<Span> {
    let width = width.max(0.0);
    let spans = duration.map(|total| chapters.spans(total)).unwrap_or_default();
    if spans.len() < 2 {
        return vec![Span { x: 0.0, width }];
    }
    let half = CHAPTER_GAP / 2.0;
    let last = spans.len() - 1;
    spans
        .iter()
        .enumerate()
        .filter_map(|(index, (start, end))| {
            let left = if index == 0 { 0.0 } else { x_of(*start as f32, width, inset) + half };
            let right = if index == last { width } else { x_of(*end as f32, width, inset) - half };
            // A chapter narrower than its gaps is drawn as nothing.
            (right > left).then_some(Span { x: left, width: right - left })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Chapter;

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn fractions_clamp_and_an_unknown_length_reads_as_the_start() {
        assert_eq!(fraction(secs(5), Some(secs(10))), 0.5);
        assert_eq!(fraction(secs(50), Some(secs(10))), 1.0);
        assert_eq!(fraction(secs(5), None), 0.0);
        assert_eq!(fraction(secs(5), Some(Duration::ZERO)), 0.0);
    }

    #[test]
    fn without_chapters_or_a_length_the_rail_is_one_segment() {
        assert_eq!(segments(200.0, 0.0, Some(secs(10)), &Chapters::default()), vec![Span { x: 0.0, width: 200.0 }]);
        let chapters = Chapters::new(vec![Chapter::new(secs(0), "A"), Chapter::new(secs(5), "B")]);
        assert_eq!(segments(200.0, 0.0, None, &chapters), vec![Span { x: 0.0, width: 200.0 }]);
    }

    #[test]
    fn chapters_split_the_rail_on_its_linear_scale_with_a_gap_at_each_boundary() {
        let chapters = Chapters::new(vec![
            Chapter::new(secs(0), "A"),
            Chapter::new(secs(5), "B"),
            Chapter::new(secs(8), "C"),
        ]);
        let rail = segments(200.0, 0.0, Some(secs(10)), &chapters);
        assert_eq!(
            rail,
            vec![
                Span { x: 0.0, width: 98.5 },
                Span { x: 101.5, width: 57.0 },
                Span { x: 161.5, width: 38.5 },
            ]
        );
    }

    #[test]
    fn a_chapter_narrower_than_its_gaps_is_dropped_and_nothing_is_negative() {
        let chapters = Chapters::new((0..10).map(|i| Chapter::new(secs(i), "c")).collect());
        let rail = segments(20.0, 0.0, Some(secs(10)), &chapters);
        assert!(rail.iter().all(|span| span.width > 0.0));
        assert!(rail.len() < 10);
    }

    #[test]
    fn boundaries_follow_the_thumb_scale_when_the_thumb_is_inset() {
        // A 12 px thumb keeps its centre 6 px in from each end of a 212 px
        // rail, so the half-way boundary is at 6 + 0.5 * 200.
        assert_eq!(x_of(0.0, 212.0, 6.0), 6.0);
        assert_eq!(x_of(0.5, 212.0, 6.0), 106.0);
        assert_eq!(x_of(1.0, 212.0, 6.0), 206.0);
        let chapters = Chapters::new(vec![Chapter::new(secs(0), "A"), Chapter::new(secs(5), "B")]);
        let rail = segments(212.0, 6.0, Some(secs(10)), &chapters);
        assert_eq!(rail, vec![Span { x: 0.0, width: 104.5 }, Span { x: 107.5, width: 104.5 }]);
    }

    #[test]
    fn an_inset_wider_than_the_rail_collapses_to_its_middle() {
        assert_eq!(x_of(0.3, 10.0, 20.0), 5.0);
    }

    #[test]
    fn a_span_clips_to_the_played_point() {
        let span = Span { x: 10.0, width: 50.0 };
        assert_eq!(span.clipped_to(5.0), None);
        assert_eq!(span.clipped_to(30.0), Some(Span { x: 10.0, width: 20.0 }));
        assert_eq!(span.clipped_to(99.0), Some(span));
    }
}
