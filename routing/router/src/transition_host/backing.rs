//! The solid layer behind the window of a transition.
//!
//! The window is the page that is growing out of an item: moved to the item's
//! place and revealing only as much of itself as the item is large. A page that
//! has not been given a background of its own is see-through, and whatever is
//! beneath it would show in the window. This layer is the page's missing
//! background.
//!
//! It is recorded once, as a rectangle the size of the host, and the host moves and
//! scales it onto the window with its presentation. A transition runs in
//! animation-only frames, which change nodes' transforms, opacities and clips and
//! leave their recorded paint alone, so a layer that had to be re-recorded for
//! each window would trail the page it backs. And there is no clip: the layer is
//! the window's shape, so there is nothing to go missing and leave a host-sized
//! rectangle over everything behind.

use std::cell::Cell;
use std::rc::Rc;

use aimer_canvas::{Canvas, Mat3};
use aimer_cupid::utilities::{Color as PaintColor, Rect};
use aimer_widget::base::{BuildContext, Color, ResolvedSize, Size, Vec2d};
use aimer_widget::{Drawable, EventElement, EventTreeRole, LayoutElement, Rebuildable, VisitorElement};

use crate::PaintPresentation;

/// Where the backing is in the host, in the host's logical pixels from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Window {
    /// `[x, y, width, height]`.
    pub rect: [f32; 4],
    /// The size of the host the window is in.
    pub host: (f32, f32),
}

/// The size of the host, as the host last published it for the layer to fill.
pub(super) type SharedHost = Rc<Cell<(f32, f32)>>;

/// The window of `presentation` in a host of `host` logical pixels.
///
/// A page is presented moved and scaled, and clipped to a part of itself: the
/// window is that part, moved and scaled with it, or the whole host when nothing
/// is revealed. `None` when there is no window: the presentation is not finite or
/// has no area, or the host has none. The opacity of the page is not part of it.
pub(super) fn window_of(presentation: &PaintPresentation, host: ResolvedSize) -> Option<Window> {
    if !presentation.is_finite() || !host.width.is_finite() || !host.height.is_finite()
        || host.width <= 0.0 || host.height <= 0.0 { return None; }
    let [[scale_x, ..], [_, scale_y, _], [x, y, _]] = presentation.transform.cols;
    let rect = match presentation.reveal {
        Some(reveal) => [x + scale_x * reveal.x, y + scale_y * reveal.y, scale_x * reveal.width, scale_y * reveal.height],
        None => [x, y, scale_x * host.width, scale_y * host.height],
    };
    (rect.iter().all(|value| value.is_finite()) && rect[2] > 0.0 && rect[3] > 0.0)
        .then_some(Window { rect, host: (host.width, host.height) })
}

/// How the host presents the backing: the layer, which is the size of the host, moved
/// and scaled onto the window of the page it backs, and always opaque, however much
/// of the page itself is showing. Hidden once the transition has `finished`, or when
/// there is no window to show.
///
/// The layer outlives the transition by up to a frame, until the frame that cleans up
/// after it; a finished transition has no window, and the layer must not go on showing
/// the last one.
pub(super) fn backing_presentation(finished: bool, followed: &PaintPresentation, host: ResolvedSize) -> PaintPresentation {
    const HIDDEN: PaintPresentation = PaintPresentation { opacity: 0.0, ..PaintPresentation::IDENTITY_OPAQUE };
    if finished { return HIDDEN; }
    let Some(Window { rect: [x, y, width, height], host: (host_width, host_height) }) = window_of(followed, host) else {
        return HIDDEN;
    };
    PaintPresentation {
        transform: Mat3::translate(x, y).mul(&Mat3::scale(width / host_width, height / host_height)),
        ..PaintPresentation::IDENTITY_OPAQUE
    }
}

/// The leaf that is the backing: a rectangle as large as the host, recorded when the
/// host's size changes and never for the window, which is the host's presentation of it.
pub(super) struct BackingLayer {
    color: Color,
    host: SharedHost,
    /// The size last recorded, `None` before the first recording.
    recorded: Cell<Option<(f32, f32)>>,
}

impl BackingLayer {
    pub(super) fn new(color: Color, host: SharedHost) -> Self {
        Self { color, host, recorded: Cell::new(None) }
    }

    fn paint_color(&self) -> PaintColor {
        let rgba = self.color.as_u32();
        PaintColor::rgba8(
            ((rgba >> 16) & 0xff) as u8,
            ((rgba >> 8) & 0xff) as u8,
            (rgba & 0xff) as u8,
            ((rgba >> 24) & 0xff) as u8,
        )
    }
}

impl Rebuildable for BackingLayer {}

impl Drawable for BackingLayer {
    fn update(&self, _: &BuildContext) {}

    fn can_paint(&self, ctx: &BuildContext) -> bool { ctx.scale.is_finite() && ctx.scale > 0.0 }

    fn paint(&self, ctx: &BuildContext) {
        let canvas = Canvas::of(ctx);
        let (width, height) = self.host.get();
        if width > 0.0 && height > 0.0 {
            canvas.fill_rect_styled(
                Rect::new(0.0, 0.0, width, height),
                self.paint_color(),
                [0.0; 4],
                [0.0; 4],
                PaintColor::transparent(),
                [0.0; 4],
                PaintColor::transparent(),
            );
        }
        canvas.finish();
        self.recorded.set(Some((width, height)));
    }

    fn paint_needs_recording(&self, _: &BuildContext) -> bool {
        self.recorded.get() != Some(self.host.get())
    }

    fn paint_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        // The host publishes `(0, 0)` until it has a size, which is also the size recorded
        // for it: nothing is painted, so there is nothing to bound.
        let (width, height) = self.host.get();
        Some(ResolvedSize { width: width * ctx.scale, height: height * ctx.scale })
    }
}


impl EventElement for BackingLayer {
    fn event_tree_role(&self) -> EventTreeRole { EventTreeRole::Transparent }
}

impl VisitorElement for BackingLayer {
    fn debug_name(&self) -> &'static str { "NavigationBacking" }
}

impl LayoutElement for BackingLayer {
    fn is_layout_stable(&self) -> bool { true }
    fn size(&self) -> Option<Size> { None }
    fn pos(&self) -> Option<Vec2d> { None }
}

#[cfg(test)]
mod tests {
    use aimer_animation::ElementBounds;
    use aimer_canvas::Mat3;
    use aimer_widget::base::ResolvedSize;

    use super::*;
    use crate::PaintPresentation;

    const HOST: ResolvedSize = ResolvedSize { width: 400.0, height: 640.0 };

    fn presentation(transform: Mat3, reveal: Option<ElementBounds>, corner_radius: f32) -> PaintPresentation {
        PaintPresentation { transform, opacity: 0.3, corner_radius, reveal }
    }

    #[test]
    fn the_window_is_the_revealed_part_moved_to_where_the_page_is_shown() {
        let window = window_of(
            &presentation(Mat3::translate(10.0, 260.0), Some(ElementBounds::new(0.0, 0.0, 380.0, 160.0)), 12.0),
            HOST,
        ).unwrap();

        assert_eq!(window.rect, [10.0, 260.0, 380.0, 160.0]);
        assert_eq!(window.host, (400.0, 640.0));
    }

    #[test]
    fn a_page_scaled_to_its_window_is_the_whole_host_scaled() {
        // A stretched page has no reveal: the host, scaled and moved, is the window.
        let transform = Mat3::translate(20.0, 100.0).mul(&Mat3::scale(0.5, 0.25));
        let window = window_of(&presentation(transform, None, 0.0), HOST).unwrap();

        assert_eq!(window.rect, [20.0, 100.0, 200.0, 160.0]);
    }

    #[test]
    fn a_page_that_fills_the_host_is_a_window_as_large_as_it() {
        let window = window_of(&presentation(Mat3::identity(), None, 0.0), HOST).unwrap();

        assert_eq!(window.rect, [0.0, 0.0, 400.0, 640.0]);
    }

    #[test]
    fn a_reveal_is_scaled_and_moved_with_the_page() {
        let transform = Mat3::translate(8.0, 16.0).mul(&Mat3::scale(2.0, 2.0));
        let window = window_of(&presentation(transform, Some(ElementBounds::new(1.0, 2.0, 30.0, 40.0)), 5.0), HOST).unwrap();

        assert_eq!(window.rect, [10.0, 20.0, 60.0, 80.0]);
    }

    #[test]
    fn nothing_is_painted_for_a_window_that_cannot_be_drawn() {
        let nowhere = [
            presentation(Mat3::translate(f32::NAN, 0.0), None, 0.0),
            presentation(Mat3::identity(), None, f32::NAN),
            presentation(Mat3::scale(0.0, 1.0), None, 0.0),
            presentation(Mat3::identity(), Some(ElementBounds::new(0.0, 0.0, 0.0, 10.0)), 0.0),
        ];
        for presentation in &nowhere {
            assert_eq!(window_of(presentation, HOST), None, "{presentation:?}");
        }
        assert_eq!(window_of(&presentation(Mat3::identity(), None, 0.0), ResolvedSize { width: 0.0, height: 100.0 }), None);
    }

    #[test]
    fn the_backing_is_the_host_moved_and_scaled_onto_the_window() {
        // The layer is recorded once, as large as the host, and every frame is only a
        // transform of it: that is all an animation-only frame can change.
        let shown = presentation(Mat3::translate(10.0, 260.0), Some(ElementBounds::new(0.0, 0.0, 380.0, 160.0)), 12.0);

        let backing = backing_presentation(false, &shown, HOST);

        assert_eq!(backing.transform.transform_point(0.0, 0.0), (10.0, 260.0));
        assert_eq!(backing.transform.transform_point(400.0, 640.0), (390.0, 420.0));
        assert_eq!(backing.opacity, 1.0, "opaque, whatever the page's own opacity");
        assert_eq!(backing.reveal, None, "and not clipped: there is nothing to clip");
        assert_eq!(backing.corner_radius, 0.0);
    }

    #[test]
    fn a_page_that_fills_the_host_has_the_backing_as_the_host() {
        let backing = backing_presentation(false, &presentation(Mat3::identity(), None, 0.0), HOST);

        assert_eq!(backing.transform, Mat3::identity());
        assert_eq!(backing.opacity, 1.0);
    }

    #[test]
    fn a_finished_transition_hides_the_backing() {
        // The layer is only dropped by the frame that cleans up after the transition,
        // which can be a frame away: until then it must show nothing, not the last
        // window it was given, which is a flat rectangle over the item the page closed into.
        let shown = presentation(Mat3::translate(0.0, 100.0), Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)), 12.0);

        assert_eq!(backing_presentation(false, &shown, HOST).opacity, 1.0);
        assert_eq!(backing_presentation(true, &shown, HOST).opacity, 0.0);
    }

    #[test]
    fn a_window_that_cannot_be_drawn_hides_the_backing() {
        for nowhere in [
            presentation(Mat3::translate(f32::NAN, 0.0), None, 0.0),
            presentation(Mat3::scale(0.0, 1.0), None, 0.0),
            presentation(Mat3::identity(), Some(ElementBounds::new(0.0, 0.0, 0.0, 10.0)), 0.0),
        ] {
            let backing = backing_presentation(false, &nowhere, HOST);
            assert_eq!(backing.opacity, 0.0, "{nowhere:?}");
            assert!(backing.is_finite(), "a presentation the host can apply: {backing:?}");
        }
    }

    #[test]
    fn the_opacity_of_the_page_does_not_reach_the_window() {
        // The backing is there so that nothing behind shows through, however much
        // of the page itself is showing.
        let window = window_of(&presentation(Mat3::identity(), None, 0.0), HOST);

        assert!(window.is_some());
    }
}
