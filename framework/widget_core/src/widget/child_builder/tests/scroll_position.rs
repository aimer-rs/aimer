use super::*;
use aimer_events::element::{ScrollDeltaKind, TouchPhase};
use crate::EventDispatcher;

struct ScrollLeaf {
    start: Vec2d,
    events: Rc<Cell<usize>>,
}

impl VisitorElement for ScrollLeaf {
    fn debug_name(&self) -> &'static str { "ScrollLeaf" }
}
impl Rebuildable for ScrollLeaf {}
impl Drawable for ScrollLeaf {}
impl LayoutElement for ScrollLeaf {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some((self.start, Vec2d { x: self.start.x + 200.0, y: self.start.y + 200.0 }))
    }
}
impl EventElement for ScrollLeaf {
    fn event_tree_role(&self) -> EventTreeRole { EventTreeRole::IndexedTarget }
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if matches!(event, ElementEvent::Scroll { .. }) {
            self.events.set(self.events.get() + 1);
            EventResult::consumed()
        } else {
            EventResult::ignored()
        }
    }
}

struct ScrollPage {
    left: Rc<Cell<usize>>,
    right: Rc<Cell<usize>>,
}
impl Widget for ScrollPage {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        ScrollPageElement {
            children: [
                ScrollLeaf { start: Vec2d::ZERO, events: self.left }.boxed(),
                ScrollLeaf { start: Vec2d { x: 800.0, y: 300.0 }, events: self.right }.boxed(),
            ],
        }.boxed()
    }
}
impl crate::PortableWidget for ScrollPage {}

struct ScrollPageElement { children: [AnyElement; 2] }
impl VisitorElement for ScrollPageElement {
    fn debug_name(&self) -> &'static str { "ScrollPage" }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children { visitor(child.as_ref()); }
    }
}
impl Rebuildable for ScrollPageElement {}
impl Drawable for ScrollPageElement {}
impl LayoutElement for ScrollPageElement {}
impl EventElement for ScrollPageElement {}

#[tokio::test]
async fn retained_children_preserve_scroll_position_without_a_pointer_move() {
    let ctx = context();
    for depth in [0, 1, 3] {
        let left = Rc::new(Cell::new(0));
        let right = Rc::new(Cell::new(0));
        let mut page: AnyWidget = ScrollPage { left: left.clone(), right: right.clone() }.boxed();
        for _ in 0..depth { page = ChildBuilder::from_widget(page).boxed(); }
        let element = page.into_element(&ctx);
        let mut dispatcher = EventDispatcher::new();
        for (index, phase) in [TouchPhase::Started, TouchPhase::Moved, TouchPhase::Ended, TouchPhase::Cancelled].into_iter().enumerate() {
            let event = ElementEvent::Scroll {
                delta: Vec2d { x: 0.0, y: -20.0 }, kind: ScrollDeltaKind::Pixel,
                phase, is_direct_manipulation: true,
            };
            let result = dispatcher.dispatch(element.as_ref(), Vec2d { x: 900.0, y: 400.0 }, &event);
            assert!(result.is_consumed(), "scroll phase {phase:?} must reach the right target at depth {depth}");
            assert_eq!(right.get(), index + 1, "each scroll must reach the right target once at depth {depth}");
            assert_eq!(left.get(), 0, "scroll must not be forwarded to the origin at depth {depth}");
        }
        let _ = dispatcher.dispatch(element.as_ref(), Vec2d { x: 1100.0, y: 600.0 }, &ElementEvent::Scroll {
            delta: Vec2d { x: 0.0, y: -20.0 }, kind: ScrollDeltaKind::Pixel,
            phase: TouchPhase::Moved, is_direct_manipulation: true,
        });
        assert_eq!(right.get(), 4, "scroll outside the target must stay outside at depth {depth}");
        assert_eq!(left.get(), 0);
    }
}
