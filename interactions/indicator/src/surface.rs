//! The pointer gesture of an interactive indicator.

use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerButton;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, InteractionBounds,
    InteractionDisagreement, InteractionSource, LayoutElement, PointerKey, Rebuildable,
    VisitorElement,
};

use crate::controller::IndicatorController;
use crate::track::{AnyIndicatorTrack, IndicatorTrack};

/// Which pointer is dragging, and the slot the indicator was resting on when it
/// went down, so that a drag that comes back there proposes nothing.
#[derive(Clone, Copy)]
struct Grab {
    key: PointerKey,
    /// `None` when the indicator was hidden: nothing was selected, so any slot
    /// the pointer lands on is a choice.
    rest: Option<usize>,
}

/// Owns the pointer gesture over an indicator's content.
///
/// It follows the pattern of the slider: the press is captured, so the moves
/// and the release that follow reach it wherever the pointer goes, even off the
/// content or out of the window. A gesture detector, which only hears moves
/// inside its own bounds, cannot carry a drag that far.
///
/// The content keeps receiving events as usual and is offered a press first, so
/// a button inside it still works; a press nothing takes begins the drag.
pub(crate) struct IndicatorSurfaceElement {
    child: AnyElement,
    controller: IndicatorController,
    track: AnyIndicatorTrack,
    on_select: Rc<dyn Fn(usize)>,
    /// Where the content is in the window, in logical pixels.
    bounds: InteractionBounds,
    grab: Cell<Option<Grab>>,
}

// Safety: widget rendering and the retained element tree are single-threaded.
unsafe impl Send for IndicatorSurfaceElement {}
unsafe impl Sync for IndicatorSurfaceElement {}

impl IndicatorSurfaceElement {
    pub(crate) fn new(
        child: AnyElement,
        controller: IndicatorController,
        track: AnyIndicatorTrack,
        on_select: Rc<dyn Fn(usize)>,
    ) -> Self {
        Self {
            child,
            controller,
            track,
            on_select,
            bounds: InteractionBounds::new(),
            grab: Cell::new(None),
        }
    }

    fn hit_test(&self, x: f32, y: f32) -> bool {
        self.bounds
            .get_bounds()
            .is_some_and(|bounds| bounds.width > 0.0 && bounds.height > 0.0)
            && self.bounds.is_inside(x, y)
    }

    /// The slot nearest to `position`, kept to the slots that exist.
    fn slot_near(&self, position: f32) -> usize {
        let last = self.track.slot_count().saturating_sub(1);
        if position.is_finite() { (position.round().max(0.0) as usize).min(last) } else { 0 }
    }

    fn press(&self, pointer: &aimer_events::pointer::PointerInfo) -> EventResult {
        let Some(bounds) = self.bounds.get_bounds() else {
            return EventResult::ignored();
        };
        let key = PointerKey::new(pointer.source, pointer.id);
        let rest = (!self.controller.is_hidden()).then(|| self.slot_near(self.controller.target()));
        // The indicator comes under the pointer, wherever it rested.
        let pressed = self.slot_near(
            self.track.nearest((pointer.pos.x - bounds.x, pointer.pos.y - bounds.y)),
        );
        self.grab.set(Some(Grab { key, rest }));
        self.controller.begin_drag(pressed as f32, (pointer.pos.x, pointer.pos.y));
        EventResult::consumed().with_pointer_capture(key).with_redraw()
    }

    fn release(&self, key: PointerKey, rest: Option<usize>, propose: bool) -> EventResult {
        self.grab.set(None);
        let landed = self.controller.end_drag(&self.track);
        if propose && Some(landed) != rest {
            (self.on_select)(landed);
        }
        EventResult::consumed().with_pointer_release(key).with_redraw()
    }
}

impl Drawable for IndicatorSurfaceElement {
    fn update(&self, ctx: &BuildContext) {
        self.save_bounds(ctx);
        self.child.update(ctx);
    }

    #[inline]
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite() && ctx.scale > 0.0
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.save_bounds(ctx);
        self.child.update(ctx);
    }

    /// Hit-tested against the size the content reports as its own.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.child.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(&self) -> Option<InteractionDisagreement> {
        self.bounds.disagreement("IndicatorSurface")
    }
}

impl IndicatorSurfaceElement {
    fn save_bounds(&self, ctx: &BuildContext) {
        let size = self.child.computed_size(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
    }
}

impl VisitorElement for IndicatorSurfaceElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "IndicatorSurfaceElement"
    }
}

impl EventElement for IndicatorSurfaceElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerDown(pointer)
                if pointer.button == PointerButton::Primary
                    && self.grab.get().is_none()
                    && self.hit_test(pointer.pos.x, pointer.pos.y) =>
            {
                self.press(pointer)
            }
            ElementEvent::PointerMove(pointer) => match self.grab.get() {
                Some(grab) if grab.key == PointerKey::new(pointer.source, pointer.id) => {
                    self.controller.drag_to((pointer.pos.x, pointer.pos.y), &self.track);
                    EventResult::consumed().with_redraw()
                }
                _ => EventResult::ignored(),
            },
            ElementEvent::PointerUp(pointer) => match self.grab.get() {
                Some(grab) if grab.key == PointerKey::new(pointer.source, pointer.id) => {
                    self.controller.drag_to((pointer.pos.x, pointer.pos.y), &self.track);
                    self.release(grab.key, grab.rest, true)
                }
                _ => EventResult::ignored(),
            },
            // The platform took the pointer away: nothing was chosen, so the
            // indicator goes back where it was.
            ElementEvent::Cancel => match self.grab.get() {
                Some(grab) => {
                    let result = self.release(grab.key, grab.rest, false);
                    match grab.rest {
                        Some(rest) => self.controller.select(rest as f32),
                        None => self.controller.set_hidden(true),
                    }
                    result
                }
                None => EventResult::ignored(),
            },
            _ => EventResult::ignored(),
        }
    }
}

impl Rebuildable for IndicatorSurfaceElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl LayoutElement for IndicatorSurfaceElement {
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

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}
