//! The painted rail under the seek slider: chapter segments, and the part
//! already downloaded.

use std::cell::Cell;
use std::sync::Arc;
use std::time::Duration;

use aimer_attribute::size::ResolvedSize;
use aimer_canvas::Canvas;
use aimer_cupid::utilities::{Color as RenderColor, Rect};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventTreeRole, LayoutElement, Rebuildable,
    VisitorElement, Widget,
};

use crate::Chapters;
use crate::track_geometry::{segments, x_of};

/// One layer of the rail: the chapter segments in `color`, optionally with
/// the first `fill` of the video in `fill_color` on top.
#[derive(Clone)]
pub(crate) struct Rail {
    pub chapters: Arc<Chapters>,
    pub duration: Option<Duration>,
    /// The thumb's half width, so boundaries line up with the slider.
    pub inset: f32,
    pub height: f32,
    pub color: [u8; 4],
    pub fill: Option<(f32, [u8; 4])>,
}

impl Widget for Rail {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawRail {
            rail: self,
            recorded: Cell::new(None),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "VideoRail"
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Key {
    width: u32,
    scale: u32,
}

struct RawRail {
    rail: Rail,
    recorded: Cell<Option<Key>>,
}

impl RawRail {
    fn key(&self, ctx: &BuildContext) -> Option<(Key, f32)> {
        let scale = ctx.scale;
        let width = self.computed_size(ctx).width;
        (scale.is_finite() && scale > 0.0 && width.is_finite() && width >= 0.0).then(|| {
            (
                Key {
                    width: width.to_bits(),
                    scale: scale.to_bits(),
                },
                width / scale,
            )
        })
    }
}

fn fill(canvas: &Canvas, x: f32, width: f32, height: f32, color: [u8; 4]) {
    let radius = height / 2.0;
    canvas.fill_rect_styled(
        Rect::new(x, 0.0, width, height),
        RenderColor::rgba8(color[0], color[1], color[2], color[3]),
        [radius; 4],
        [0.0; 4],
        RenderColor::transparent(),
        [0.0; 4],
        RenderColor::transparent(),
    );
}

impl Drawable for RawRail {
    #[inline]
    fn can_paint(&self, ctx: &BuildContext) -> bool {
        self.key(ctx).is_some()
    }

    fn paint(&self, ctx: &BuildContext) {
        let Some((key, width)) = self.key(ctx) else {
            return;
        };
        let rail = &self.rail;
        let canvas = Canvas::of(ctx);
        let spans = segments(width, rail.inset, rail.duration, &rail.chapters);
        for span in &spans {
            fill(&canvas, span.x, span.width, rail.height, rail.color);
        }
        if let Some((fraction, color)) = rail.fill {
            let end = x_of(fraction, width, rail.inset);
            for span in spans.iter().filter_map(|span| span.clipped_to(end)) {
                fill(&canvas, span.x, span.width, rail.height, color);
            }
        }
        canvas.finish();
        self.recorded.set(Some(key));
    }

    #[inline]
    fn paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.recorded.get() != self.key(ctx).map(|(key, _)| key)
    }
}

impl EventElement for RawRail {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl VisitorElement for RawRail {
    fn debug_name(&self) -> &'static str {
        "VideoRail"
    }
}

impl Rebuildable for RawRail {}

impl LayoutElement for RawRail {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize {
            // The slider hands its slots its own size.
            width: ctx.parent_size.width.max(0.0),
            height: self.rail.height * ctx.scale,
        }
    }
}
