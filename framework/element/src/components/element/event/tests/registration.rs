//! Which elements the event tree registers: the ones that handle events.
//!
//! An element joins the event tree by saying so, through its
//! [`EventTreeRole`]. The default is [`EventTreeRole::Transparent`], so a
//! wrapper, a leaf that only paints, or a widget that only describes itself to
//! assistive technology costs routing nothing, and an element that does handle
//! events declares the role that fits it.

use super::*;

/// An element that overrides nothing about events: the shape of a wrapper or a
/// painting leaf.
struct PlainParent {
    children: Vec<AnyElement>,
}

impl VisitorElement for PlainParent {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }
}

impl Element for PlainParent {
    fn name(&self) -> &'static str {
        "PlainParent"
    }
}

impl EventElement for PlainParent {}

impl LayoutElement for PlainParent {}

impl Drawable for PlainParent {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for PlainParent {}

/// An element that handles events and declares it.
struct Handler {
    events: Rc<Cell<usize>>,
}

impl VisitorElement for Handler {}

impl Element for Handler {
    fn name(&self) -> &'static str {
        "Handler"
    }
}

impl EventElement for Handler {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        self.events.set(self.events.get() + 1);
        EventResult::consumed()
    }
}

impl LayoutElement for Handler {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some((Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }))
    }
}

impl Drawable for Handler {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for Handler {}

fn press_at(dispatcher: &mut EventDispatcher, root: &dyn Element, x: f32, y: f32) -> EventResult {
    let pos = Vec2d { x, y };
    dispatcher.dispatch(
        root,
        pos,
        &ElementEvent::PointerDown(PointerInfo::mouse(pos, PointerButton::Primary)),
    )
}

#[test]
fn the_default_role_registers_nothing() {
    let plain = PlainParent { children: Vec::new() };

    assert_eq!(plain.event_tree_role(), EventTreeRole::Transparent);
}

#[test]
fn an_element_that_handles_nothing_is_not_an_event_target() {
    let events = Rc::new(Cell::new(0));
    let root = PlainParent {
        children: vec![
            PlainParent {
                children: vec![Handler { events: events.clone() }.boxed()],
            }
            .boxed(),
        ],
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    let result = press_at(&mut dispatcher, root.as_ref(), 5.0, 5.0);

    assert!(result.is_consumed(), "the handler below the wrappers still gets the press");
    assert_eq!(events.get(), 1);
    assert_eq!(
        dispatcher.event_tree.elements().len(),
        1,
        "only the element that handles events is registered, not its two wrappers",
    );
    assert_eq!(dispatcher.event_tree.roots().len(), 1);
}

#[test]
fn siblings_that_handle_nothing_do_not_displace_the_ones_that_do() {
    let (left, right) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let root = PlainParent {
        children: vec![
            Handler { events: left.clone() }.boxed(),
            PlainParent { children: Vec::new() }.boxed(),
            Handler { events: right.clone() }.boxed(),
        ],
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    let _ = press_at(&mut dispatcher, root.as_ref(), 5.0, 5.0);

    assert_eq!(dispatcher.event_tree.elements().len(), 2);
    // Both handlers overlap at this point and paint order puts the last on top.
    assert_eq!((left.get(), right.get()), (0, 1));
}
