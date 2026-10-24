//! One retained layer of an indicator, presented from the controller each frame.

use std::cell::Cell;

use aimer_animation::{AnimInstant, PaintPresentation, PresentationClip};
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_canvas::Mat3;
use aimer_events::element::ElementEvent;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, LayoutElement, PortableWidget,
    Rebuildable, VisitorElement, Widget, request_isolated_animation_frame,
};

use crate::controller::{IndicatorController, IndicatorSample};
use crate::style::{AnyIndicatorStyle, IndicatorLayer, IndicatorStyle};
use crate::track::{AnyIndicatorTrack, IndicatorTrack};

/// Presents `child` as one layer of an indicator.
///
/// The child is converted to an element once. Each frame the layer samples the
/// controller and presents the child's render node at the indicator's position,
/// scaled and faded as the style says, so the animation never rebuilds or
/// re-records the child.
pub(crate) struct IndicatorLayerWidget<W> {
    pub(crate) controller: IndicatorController,
    pub(crate) track: AnyIndicatorTrack,
    pub(crate) style: AnyIndicatorStyle,
    pub(crate) layer: IndicatorLayer,
    pub(crate) child: W,
}

impl<W: Widget + 'static> Widget for IndicatorLayerWidget<W> {
    fn debug_name(&self) -> &'static str {
        "IndicatorLayer"
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        IndicatorLayerElement {
            child: self.child.to_element(ctx),
            controller: self.controller,
            track: self.track,
            style: self.style,
            layer: self.layer,
            clip: PresentationClip::default(),
            hidden: Cell::new(false),
        }
        .boxed()
    }
}

impl<W: Widget + 'static> PortableWidget for IndicatorLayerWidget<W> {}

/// Retained implementation behind [`IndicatorLayerWidget`].
struct IndicatorLayerElement {
    child: AnyElement,
    controller: IndicatorController,
    track: AnyIndicatorTrack,
    style: AnyIndicatorStyle,
    layer: IndicatorLayer,
    clip: PresentationClip,
    /// Whether this layer has clipped its child away because it is not visible.
    hidden: Cell<bool>,
}

// Safety: widget rendering and the retained element tree are single-threaded.
unsafe impl Send for IndicatorLayerElement {}
unsafe impl Sync for IndicatorLayerElement {}

impl IndicatorLayerElement {
    /// How the child is shown for `sample`. `size` is the child's logical size.
    fn presentation(
        &self,
        sample: IndicatorSample,
        size: ResolvedSize,
        outsets: [f32; 2],
    ) -> PaintPresentation {
        let shown = self.style.present(self.layer, sample.lift);
        let scale = if shown.scale.is_finite() && shown.scale > 0.0 { shown.scale } else { 1.0 };
        let opacity = if shown.opacity.is_finite() { shown.opacity.clamp(0.0, 1.0) } else { 0.0 };
        let presence = if sample.presence.is_finite() { sample.presence.clamp(0.0, 1.0) } else { 0.0 };
        let opacity = opacity * presence;
        let (x, y) = self.track.origin(sample.position);
        // Scale about the center of the child's layout box, then move it to the
        // slot. The child's render node starts at its outer paint corner, so that
        // center is `outsets` further in than half its size; a move needs no
        // such correction, only the pivot does.
        let transform = Mat3::translate(
            x + (size.width * 0.5 + outsets[0]) * (1.0 - scale),
            y + (size.height * 0.5 + outsets[1]) * (1.0 - scale),
        )
        .mul(&Mat3::scale(scale, scale));
        PaintPresentation { transform, opacity, ..PaintPresentation::IDENTITY_OPAQUE }
    }
}

impl Drawable for IndicatorLayerElement {
    fn update(&self, ctx: &BuildContext) {
        self.controller.tick(AnimInstant::now());
        let animating = self.controller.is_animating();
        self.child.update(ctx);
        if animating {
            aimer_events::window::request_animation_frame();
        }
    }

    #[inline]
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite() && ctx.scale > 0.0
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        let sample = self.controller.tick(AnimInstant::now());
        if self.controller.is_animating() {
            // Everything a frame of this animation changes is the presentation
            // set below, so the next frame may revisit this element alone.
            request_isolated_animation_frame(ctx);
        }
        if !sample.position.is_finite() || !sample.lift.is_finite() || !sample.presence.is_finite() {
            return false;
        }
        self.child.sync_paint_geometry(ctx);
        let size = self.child.computed_size(ctx);
        let size = ResolvedSize { width: size.width / ctx.scale, height: size.height / ctx.scale };
        // A child that paints beyond its layout box, such as a lens with a
        // shadow, has a render node that starts at its outsets, not at its
        // layout origin, which moves the pivot a scale turns about.
        let [left, top, _, _] = self
            .child
            .retained_v2_paint_outsets(ctx)
            .filter(|outsets| outsets.iter().all(|outset| outset.is_finite() && *outset >= 0.0))
            .unwrap_or([0.0; 4]);
        let presentation = self.presentation(sample, size, [left, top]);
        if !presentation.apply_to_child(ctx, 0, size, &self.clip) {
            return false;
        }
        // The renderer still draws a node at zero opacity, and for a lens that
        // means its backdrop pass. Clipping the child to nothing is what takes
        // it out of the render list, so a hidden layer costs nothing.
        let hidden = presentation.opacity <= 0.0;
        if self.hidden.get() == hidden {
            return true;
        }
        let clip = hidden.then_some((0.0, 0.0, 0.0, 0.0));
        let applied = ctx.set_local_v2_child_clip_at(0, clip, [0.0; 4]);
        if applied {
            self.hidden.set(hidden);
        }
        applied
    }

    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        // `sync_local_v2_state` has already asked for the next frame.
        self.child.update(ctx);
    }
}

impl VisitorElement for IndicatorLayerElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "IndicatorLayerElement"
    }
}

/// A layer is drawn over or under the content, never in the way of it, so it
/// takes no pointer input and no focus. It is still a structural child, which
/// reconciliation needs.
impl EventElement for IndicatorLayerElement {
    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn focus_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}
}

impl Rebuildable for IndicatorLayerElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl LayoutElement for IndicatorLayerElement {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
}

#[cfg(test)]
mod tests;
