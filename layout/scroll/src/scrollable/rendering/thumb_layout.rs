//! How wide the scroll bar thumb is and where it sits across the bar strip,
//! and the fade that grows it when a finger takes hold.
//!
//! On a touch device the strip is wider than the thumb ever is at rest, so the
//! thumb has room to grow when it is held. The strip is transparent and the
//! resting thumb is anchored to the strip's outer edge with the same inset it
//! always had, so a bar nobody is holding looks exactly as it did before the
//! strip was widened.

use std::cell::Cell;

use aimer_attribute::dimension::Dimension;
use aimer_utils::AnimInstant;

use super::scroll_bar::{ScrollBar, ScrollBarPlacement};
use crate::scrollable::constants::{
    SCROLLBAR_THUMB_GRAB_WIDTH_DP, SCROLLBAR_TOUCH_REST_STRIP_DP, SCROLLBAR_TOUCH_STRIP_DP,
};

/// Whether this build runs on a phone or tablet, where a finger is the pointer.
pub(crate) const MOBILE: bool = cfg!(any(target_os = "android", target_os = "ios"));

/// Whether the bar's strip is the widened touch strip.
///
/// Only an unsized, floating bar on a touch device is widened: a bar the app
/// sized itself keeps the size it asked for, and an inline bar reserves its
/// strip's width out of the content's, which a wider strip would take away.
#[inline]
pub(crate) fn widened_strip(
    mobile: bool,
    scroll_bar: &ScrollBar,
    placement: ScrollBarPlacement,
) -> bool {
    mobile && matches!(scroll_bar.track.width, Dimension::Auto) && !placement.reserves_space()
}

/// The strip's width for a touch device's unsized bar, in physical pixels.
#[inline]
pub(crate) fn touch_strip_width(placement: ScrollBarPlacement, scale: f32) -> f32 {
    if placement.reserves_space() {
        SCROLLBAR_TOUCH_REST_STRIP_DP * scale
    } else {
        SCROLLBAR_TOUCH_STRIP_DP * scale
    }
}

/// The strip width the resting thumb is sized from: the widened strip is only
/// room to grow into, so the thumb at rest is measured against the strip it
/// had before.
#[inline]
pub(crate) fn rest_strip_width(widened: bool, track_width: f32, scale: f32) -> f32 {
    if widened {
        SCROLLBAR_TOUCH_REST_STRIP_DP * scale
    } else {
        track_width
    }
}

/// The thumb's width and its offset from the strip's start, across the bar.
///
/// `rest_width` is the width it has when nobody holds it and `grab_mix` how far
/// it has grown toward the held width, `0.0..=1.0`. The thumb grows inward: its
/// outer edge, the one against the edge of the viewport, stays where it was, so
/// it never reaches past the strip's end.
pub(crate) fn thumb_cross_layout(
    track_width: f32,
    rest_width: f32,
    grab_mix: f32,
    widened: bool,
    scale: f32,
) -> (f32, f32) {
    if grab_mix <= 0.0 && !widened {
        // Nothing to grow and nothing to anchor: centred, exactly as before.
        return (rest_width, (track_width - rest_width) / 2.0);
    }
    let inset = if widened {
        (rest_strip_width(true, track_width, scale) - rest_width).max(0.0) / 2.0
    } else {
        (track_width - rest_width) / 2.0
    };
    let outer_edge = track_width - inset;
    let held_width = (SCROLLBAR_THUMB_GRAB_WIDTH_DP * scale)
        .max(rest_width)
        .min(outer_edge.max(rest_width));
    let width = rest_width + (held_width - rest_width) * grab_mix.clamp(0.0, 1.0);
    (width, outer_edge - width)
}

/// The log line for one frame of the thumb growing or shrinking, or `None` when
/// nothing changed since the last line.
///
/// `mix` is how far the thumb has grown (`0.0` at rest, `1.0` held), `previous`
/// what it was when last reported, and `width_dp` its width now. Only changes are
/// worth a line: a bar nobody is holding would otherwise log every frame.
pub(crate) fn growth_log_line(
    grabbed: bool,
    mix: f32,
    previous: f32,
    width_dp: f32,
) -> Option<String> {
    let reached_an_end = mix >= 1.0 || mix <= 0.0;
    // A step too small to show in two decimals would print the same line twice.
    if mix == previous || (!reached_an_end && (mix - previous).abs() < 0.01) {
        return None;
    }
    let ending = if mix >= 1.0 {
        " - fully grown"
    } else if mix <= 0.0 {
        " - back at rest"
    } else {
        ""
    };
    Some(format!(
        "thumb {}: {mix:.2} of the way, {width_dp:.1} dp wide{ending}",
        if grabbed { "growing" } else { "shrinking" },
    ))
}

/// A value easing between two ends at a fixed rate, advanced by the frames that
/// ask for it.
///
/// Time only counts toward the fade when the previous call left it unfinished.
/// A settled fade is not advanced again until something changes, so the gap
/// before the call that changes it is rest, not fade time; counting it would
/// jump straight to the destination. That call starts the fade, and the ones
/// that follow advance it. A fade that is turned around mid-way goes back from
/// where it is instead of restarting.
#[derive(Default)]
pub(crate) struct Fade {
    mix: Cell<f32>,
    at: Cell<Option<AnimInstant>>,
    fading_now: Cell<bool>,
}

impl Fade {
    /// Advances toward `on` (`1.0`) or off (`0.0`) over `duration_ms` and
    /// returns where it is.
    pub(crate) fn advance(&self, on: bool, now: AnimInstant, duration_ms: u64) -> f32 {
        let previous = self.at.replace(Some(now));
        let elapsed = previous
            .filter(|_| self.fading_now.get())
            .map_or(0.0, |previous| now.duration_since(previous).as_secs_f32());
        let step = elapsed / (duration_ms as f32 / 1000.0);
        let mix = self.mix.get();
        let mix = if on {
            (mix + step).min(1.0)
        } else {
            (mix - step).max(0.0)
        };
        self.mix.set(mix);
        self.fading_now.set(if on { mix < 1.0 } else { mix > 0.0 });
        mix
    }

    /// Where the fade is now, `0.0..=1.0`.
    pub(crate) fn value(&self) -> f32 {
        self.mix.get()
    }

    /// Whether the fade is still between its ends on its way to `on`, which only
    /// a frame can finish.
    pub(crate) fn unfinished(&self, on: bool) -> bool {
        let mix = self.mix.get();
        if on { mix < 1.0 } else { mix > 0.0 }
    }

    /// Puts the fade back at rest, off.
    pub(crate) fn reset(&self) {
        self.mix.set(0.0);
        self.at.set(None);
        self.fading_now.set(false);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const SCALE: f32 = 3.0;

    fn strip() -> f32 {
        SCROLLBAR_TOUCH_STRIP_DP * SCALE
    }

    fn rest() -> f32 {
        // What an unsized thumb measures against the old, narrow strip.
        (SCROLLBAR_TOUCH_REST_STRIP_DP * SCALE * 0.6).max(4.0)
    }

    #[test]
    fn a_desktop_bar_at_rest_is_centred_in_its_track_as_ever() {
        let (width, offset) = thumb_cross_layout(36.0, 21.6, 0.0, false, 1.0);
        assert_eq!(width, 21.6);
        assert_eq!(offset, (36.0 - 21.6) / 2.0);
    }

    #[test]
    fn the_widened_strip_leaves_the_resting_thumb_where_the_narrow_one_put_it() {
        let narrow_strip = SCROLLBAR_TOUCH_REST_STRIP_DP * SCALE;
        let (_, centred) = thumb_cross_layout(narrow_strip, rest(), 0.0, false, SCALE);
        let from_the_edge = narrow_strip - (centred + rest());

        let (width, offset) = thumb_cross_layout(strip(), rest(), 0.0, true, SCALE);

        assert_eq!(width, rest());
        let widened_from_the_edge = strip() - (offset + width);
        assert!(
            (widened_from_the_edge - from_the_edge).abs() < 1e-4,
            "the gap to the viewport edge is unchanged: {widened_from_the_edge} vs {from_the_edge}"
        );
    }

    #[test]
    fn a_held_thumb_grows_inward_and_keeps_its_outer_edge() {
        let (rest_width, rest_offset) = thumb_cross_layout(strip(), rest(), 0.0, true, SCALE);
        let (held_width, held_offset) = thumb_cross_layout(strip(), rest(), 1.0, true, SCALE);

        assert_eq!(held_width, SCROLLBAR_THUMB_GRAB_WIDTH_DP * SCALE);
        assert!(held_width > rest_width);
        assert!(
            ((rest_offset + rest_width) - (held_offset + held_width)).abs() < 1e-4,
            "the edge against the viewport does not move"
        );
        assert!(held_offset >= 0.0, "and it stays inside the strip");
    }

    #[test]
    fn growth_is_proportional_to_the_mix() {
        let (rest_width, _) = thumb_cross_layout(strip(), rest(), 0.0, true, SCALE);
        let (full, _) = thumb_cross_layout(strip(), rest(), 1.0, true, SCALE);
        let (half, _) = thumb_cross_layout(strip(), rest(), 0.5, true, SCALE);
        assert!((half - (rest_width + full) / 2.0).abs() < 1e-4);
    }

    #[test]
    fn a_thumb_never_shrinks_below_its_resting_width_or_outgrows_the_strip() {
        // A resting thumb already wider than the held width keeps its width.
        let (width, _) = thumb_cross_layout(strip(), 40.0, 1.0, true, SCALE);
        assert_eq!(width, 40.0);
        // A strip too narrow to hold the held width caps it.
        let (width, offset) = thumb_cross_layout(10.0, 4.0, 1.0, false, SCALE);
        assert!(offset >= 0.0 && width <= 10.0, "width {width}, offset {offset}");
    }

    #[test]
    fn only_an_unsized_floating_bar_on_a_touch_device_is_widened() {
        let unsized_bar = ScrollBar::default();
        assert!(widened_strip(true, &unsized_bar, ScrollBarPlacement::Floating));
        assert!(!widened_strip(false, &unsized_bar, ScrollBarPlacement::Floating));
        assert!(!widened_strip(true, &unsized_bar, ScrollBarPlacement::Inline));

        let mut sized = ScrollBar::default();
        sized.track.width = Dimension::Px(10.0);
        assert!(!widened_strip(true, &sized, ScrollBarPlacement::Floating));
    }

    #[test]
    fn growth_is_logged_only_while_it_changes() {
        assert_eq!(growth_log_line(true, 0.0, 0.0, 3.6), None, "a resting thumb says nothing");
        assert_eq!(growth_log_line(true, 1.0, 1.0, 8.0), None, "nor does a grown one");

        assert_eq!(growth_log_line(true, 0.004, 0.0, 3.6), None, "too small a step to say anything");
        assert!(growth_log_line(true, 1.0, 0.999, 8.0).is_some(), "but arriving always does");
        assert!(growth_log_line(false, 0.0, 0.003, 3.6).is_some());

        let growing = growth_log_line(true, 0.4, 0.0, 5.5).unwrap();
        assert!(growing.contains("growing") && growing.contains("0.40") && growing.contains("5.5 dp"), "{growing}");
        let grown = growth_log_line(true, 1.0, 0.8, 8.0).unwrap();
        assert!(grown.contains("fully grown"), "{grown}");
        let shrinking = growth_log_line(false, 0.6, 1.0, 6.5).unwrap();
        assert!(shrinking.contains("shrinking"), "{shrinking}");
        let rested = growth_log_line(false, 0.0, 0.2, 3.6).unwrap();
        assert!(rested.contains("back at rest"), "{rested}");
    }

    #[test]
    fn the_fade_moves_at_a_fixed_rate_and_turns_around_where_it_is() {
        let fade = Fade::default();
        let t0 = AnimInstant::now();
        let ms = |n| Duration::from_millis(n);
        assert_eq!(fade.advance(false, t0, 100), 0.0);
        assert_eq!(fade.advance(true, t0, 100), 0.0, "it begins at nothing");
        let half = fade.advance(true, t0 + ms(50), 100);
        assert!((half - 0.5).abs() < 1e-4);
        let back = fade.advance(false, t0 + ms(75), 100);
        assert!((back - 0.25).abs() < 1e-4, "it returns from where it was: {back}");
        assert!(fade.unfinished(false), "still on its way back");
        assert!(fade.unfinished(true), "or on its way out again");
        assert!(!Fade::default().unfinished(false), "a fade at rest has nothing to finish");
    }
}
