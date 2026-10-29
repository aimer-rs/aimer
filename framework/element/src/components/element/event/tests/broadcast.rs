use super::*;

#[derive(Default)]
struct BroadcastTraversal {
    structural_visits: Cell<usize>,
    event_visits: Cell<usize>,
}

struct BroadcastProbe {
    label: usize,
    children: Vec<AnyElement>,
    traversal: Rc<BroadcastTraversal>,
    events: Rc<RefCell<Vec<usize>>>,
    role: EventTreeRole,
    result: Rc<Cell<EventResult>>,
    focus: Option<FocusNode>,
}

impl VisitorElement for BroadcastProbe {
    fn debug_name(&self) -> &'static str {
        "BroadcastProbe"
    }

    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }
}

impl EventElement for BroadcastProbe {
    fn event_tree_role(&self) -> EventTreeRole {
        self.role
    }

    fn focus_node(&self) -> Option<&FocusNode> {
        self.focus.as_ref()
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            self.traversal.structural_visits.set(self.traversal.structural_visits.get() + 1);
            visitor(child.as_ref());
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            self.traversal.event_visits.set(self.traversal.event_visits.get() + 1);
            visitor(child.as_ref());
        }
    }

    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        self.events.borrow_mut().push(self.label);
        self.result.get()
    }
}

impl LayoutElement for BroadcastProbe {
    fn is_layout_stable(&self) -> bool {
        true
    }
}
impl Drawable for BroadcastProbe {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for BroadcastProbe {}

fn broadcast_tree(
    count: usize,
    traversal: &Rc<BroadcastTraversal>,
    events: &Rc<RefCell<Vec<usize>>>,
) -> AnyElement {
    let children = (1..=count)
        .map(|label| BroadcastProbe {
            label,
            children: Vec::new(),
            traversal: traversal.clone(),
            events: events.clone(),
            role: EventTreeRole::IndexedTarget,
            result: Rc::new(Cell::new(EventResult::ignored())),
            focus: None,
        }.boxed())
        .collect();
    BroadcastProbe {
        label: 0,
        children,
        traversal: traversal.clone(),
        events: events.clone(),
        role: EventTreeRole::IndexedTarget,
        result: Rc::new(Cell::new(EventResult::ignored())),
        focus: None,
    }.boxed()
}

#[test]
fn standalone_broadcast_does_not_rescan_siblings_for_each_recipient() {
    let traversal = Rc::new(BroadcastTraversal::default());
    let events = Rc::new(RefCell::new(Vec::new()));
    let root = broadcast_tree(128, &traversal, &events);

    assert_eq!(broadcast_event(root.as_ref(), &ElementEvent::Cancel), EventResult::ignored());
    assert_eq!(events.borrow().len(), 129);
    assert!(
        traversal.structural_visits.get() <= 128,
        "one indexing walk should suffice; observed {} child visits",
        traversal.structural_visits.get(),
    );
}

#[test]
fn cached_broadcast_reuses_the_tree_across_event_frames() {
    let traversal = Rc::new(BroadcastTraversal::default());
    let events = Rc::new(RefCell::new(Vec::new()));
    let root = broadcast_tree(128, &traversal, &events);
    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::DragOver {
        pos: Vec2d::default(),
        source: PointerSource::Mouse,
        id: 0,
    });
    traversal.structural_visits.set(0);
    traversal.event_visits.set(0);
    events.borrow_mut().clear();

    for _ in 0..2 {
        begin_event_frame();
        assert_eq!(dispatcher.broadcast(root.as_ref(), &ElementEvent::Cancel), EventResult::ignored());
    }

    assert_eq!(events.borrow().len(), 258);
    assert_eq!(traversal.structural_visits.get(), 0, "an unchanged tree needs no structural walk");
    assert_eq!(traversal.event_visits.get(), 0, "an unchanged event tree needs no reconstruction");
}

#[test]
fn broadcast_keeps_postorder_and_combines_effects_without_applying_requests() {
    let traversal = Rc::new(BroadcastTraversal::default());
    let events = Rc::new(RefCell::new(Vec::new()));
    let pointer = PointerKey::new(PointerSource::Mouse, 0);
    let expected = EventResult::consumed().with_redraw()
        .with_pointer_capture(pointer).with_follow_up(FollowUp::DragOver);
    let focus = FocusNode::new();
    focus.request_focus();
    let leaf = |label, result, focus| BroadcastProbe {
        label,
        children: Vec::new(),
        traversal: traversal.clone(),
        events: events.clone(),
        role: EventTreeRole::IndexedTarget,
        result: Rc::new(Cell::new(result)),
        focus,
    };
    let mut branch = leaf(1, EventResult::consumed(), None);
    branch.role = EventTreeRole::Transparent;
    branch.children.push(leaf(3, EventResult::ignored(), None).boxed());
    let mut root = leaf(0, EventResult::ignored(), None);
    root.children = vec![branch.boxed(), leaf(2, expected, Some(focus.clone())).boxed()];
    let root = root.boxed();
    let mut dispatcher = EventDispatcher::new();

    assert_eq!(dispatcher.broadcast(root.as_ref(), &ElementEvent::Cancel), expected);
    assert_eq!(*events.borrow(), [2, 3, 0]);
    assert_eq!(dispatcher.capture_count(), 0);
    assert_eq!(dispatcher.focused(), None);
    assert!(!focus.has_focus());
}

#[test]
fn broadcasting_a_release_does_not_change_existing_capture() {
    let pos = Vec2d { x: 5.0, y: 5.0 };
    let events = Rc::new(RefCell::new(Vec::new()));
    let pointer = PointerKey::new(PointerSource::Mouse, 0);
    let result = Rc::new(Cell::new(EventResult::consumed().with_pointer_capture(pointer)));
    let root = BroadcastProbe {
        label: 0,
        children: Vec::new(),
        traversal: Rc::new(BroadcastTraversal::default()),
        events: events.clone(),
        role: EventTreeRole::IndexedTarget,
        result: result.clone(),
        focus: None,
    }.boxed();
    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(root.as_ref(), pos, &ElementEvent::PointerDown(
        PointerInfo::mouse(pos, PointerButton::Primary),
    ));
    assert_eq!(dispatcher.captured_owner(pointer), Some(root.id()));

    result.set(EventResult::consumed().with_pointer_release(pointer));
    let result = dispatcher.broadcast(root.as_ref(), &ElementEvent::PointerMove(
        PointerInfo::mouse(pos, PointerButton::Primary),
    ));

    assert_eq!(result.capture_request(), CaptureRequest::Release(pointer));
    assert_eq!(dispatcher.captured_owner(pointer), Some(root.id()));
    assert_eq!(*events.borrow(), [0, 0]);
}

#[test]
fn cached_broadcast_handles_a_leaf_and_a_different_root() {
    let traversal = Rc::new(BroadcastTraversal::default());
    let events = Rc::new(RefCell::new(Vec::new()));
    let first = broadcast_tree(0, &traversal, &events);
    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.broadcast(first.as_ref(), &ElementEvent::Cancel);
    drop(first);
    let next = broadcast_tree(1, &traversal, &events);
    let _ = dispatcher.broadcast(next.as_ref(), &ElementEvent::Cancel);

    assert_eq!(*events.borrow(), [0, 1, 0]);
}

#[test]
fn cached_broadcast_ignores_a_tree_without_event_targets() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let root = BroadcastProbe {
        label: 0,
        children: Vec::new(),
        traversal: Rc::new(BroadcastTraversal::default()),
        events: events.clone(),
        role: EventTreeRole::Transparent,
        result: Rc::new(Cell::new(EventResult::consumed())),
        focus: None,
    }.boxed();
    let mut dispatcher = EventDispatcher::new();

    assert_eq!(dispatcher.broadcast(root.as_ref(), &ElementEvent::Cancel), EventResult::ignored());
    assert!(events.borrow().is_empty());
}

struct ChangingBroadcastRoot {
    first: AnyElement,
    next: AnyElement,
    use_next: Rc<Cell<bool>>,
}

impl VisitorElement for ChangingBroadcastRoot {
    fn debug_name(&self) -> &'static str {
        "ChangingBroadcastRoot"
    }

    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(if self.use_next.get() { self.next.as_ref() } else { self.first.as_ref() });
    }
}

impl EventElement for ChangingBroadcastRoot {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}
impl LayoutElement for ChangingBroadcastRoot {
    fn is_layout_stable(&self) -> bool {
        true
    }
}
impl Drawable for ChangingBroadcastRoot {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for ChangingBroadcastRoot {}

#[test]
fn cached_broadcast_refreshes_a_replaced_subtree_in_the_same_event_frame() {
    let traversal = Rc::new(BroadcastTraversal::default());
    let events = Rc::new(RefCell::new(Vec::new()));
    let use_next = Rc::new(Cell::new(false));
    let root = ChangingBroadcastRoot {
        first: broadcast_tree(1, &traversal, &events),
        next: broadcast_tree(2, &traversal, &events),
        use_next: use_next.clone(),
    }.boxed();
    let mut dispatcher = EventDispatcher::new();
    begin_event_frame();
    let _ = dispatcher.broadcast(root.as_ref(), &ElementEvent::Cancel);
    use_next.set(true);
    root.set_subtree_generation(root.subtree_generation() + 1);
    let _ = dispatcher.broadcast(root.as_ref(), &ElementEvent::Cancel);

    assert_eq!(*events.borrow(), [1, 0, 2, 1, 0]);
}
