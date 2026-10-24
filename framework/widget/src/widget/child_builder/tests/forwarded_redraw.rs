//! A retained placement forwards a descendant's redraw; it must not also
//! mark the child it stands in for as having changed its own paint.

use aimer_events::element::{ScrollDeltaKind, TouchPhase};

use super::*;
use crate::EventDispatcher;
use crate::components::element::take_event_paint_invalidated;

/// A leaf that reacts to a scroll the way a scrollable does: it consumes the
/// event and asks for a redraw of its own paint.
struct RedrawLeaf;

impl VisitorElement for RedrawLeaf {
    fn debug_name(&self) -> &'static str { "RedrawLeaf" }
}
impl Rebuildable for RedrawLeaf {}
impl Drawable for RedrawLeaf {}
impl LayoutElement for RedrawLeaf {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some((Vec2d::ZERO, Vec2d { x: 200.0, y: 200.0 }))
    }
}
impl EventElement for RedrawLeaf {
    fn event_tree_role(&self) -> EventTreeRole { EventTreeRole::IndexedTarget }
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if matches!(event, ElementEvent::Scroll { .. }) {
            EventResult::consumed().with_redraw()
        } else {
            EventResult::ignored()
        }
    }
}

/// A container with no paint change of its own around one [`RedrawLeaf`].
struct Page {
    child: AnyElement,
}
impl VisitorElement for Page {
    fn debug_name(&self) -> &'static str { "Page" }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}
impl Rebuildable for Page {}
impl Drawable for Page {}
impl LayoutElement for Page {}
impl EventElement for Page {}

struct PageWidget {
    leaf_id: Rc<Cell<Option<ElementId>>>,
}
impl Widget for PageWidget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        let child = RedrawLeaf.boxed();
        self.leaf_id.set(child.element_id());
        Page { child }.boxed()
    }
    fn debug_name(&self) -> &'static str { "PageWidget" }
}
impl crate::PortableWidget for PageWidget {}

#[tokio::test]
async fn a_descendants_redraw_does_not_invalidate_the_retained_child_it_sits_in() {
    let ctx = context();
    let leaf_id = Rc::new(Cell::new(None));
    let placement = ChildBuilder::from_widget(PageWidget { leaf_id: leaf_id.clone() })
        .build(&ctx);
    let page_id = placement.element_id().expect("a placement reports its child's id");
    let leaf_id = leaf_id.get().expect("the leaf was built");
    assert_ne!(page_id, leaf_id);

    let frames = Rc::new(Cell::new(0));
    let previous = aimer_events::window::set_thread_redraw_requester({
        let frames = frames.clone();
        move || frames.set(frames.get() + 1)
    });
    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(
        placement.as_ref(),
        Vec2d { x: 50.0, y: 50.0 },
        &ElementEvent::Scroll {
            delta: Vec2d { x: 0.0, y: -20.0 },
            kind: ScrollDeltaKind::Pixel,
            phase: TouchPhase::Moved,
            is_direct_manipulation: true,
        },
    );
    aimer_events::window::restore_thread_redraw_requester(previous);

    assert!(take_event_paint_invalidated(leaf_id), "the leaf asked for the redraw");
    assert!(
        !take_event_paint_invalidated(page_id),
        "the page's own paint did not change, so its retained list must stay valid"
    );
    assert!(
        frames.get() >= 1,
        "the redraw still has to reach the frame loop"
    );
}
