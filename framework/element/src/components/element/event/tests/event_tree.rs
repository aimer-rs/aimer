use super::*;

struct CapturingElement {
    events: Cell<usize>,
}

impl VisitorElement for CapturingElement {
    fn debug_name(&self) -> &'static str {
        "CapturingElement"
    }
}

impl EventElement for CapturingElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.events.set(self.events.get() + 1);
        match event {
            ElementEvent::PointerDown(pointer) if pointer.id == 7 => {
                EventResult::consumed()
                    .with_pointer_capture(PointerKey::new(pointer.source, 7))
            }
            _ => EventResult::consumed(),
        }
    }
}

impl LayoutElement for CapturingElement {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some((Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }))
    }
}

impl Drawable for CapturingElement {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for CapturingElement {}

struct TreeElement {
    children: Vec<AnyElement>,
    events: Rc<Cell<usize>>,
    role: EventTreeRole,
    bounds: Option<Rc<Cell<Option<(Vec2d, Vec2d)>>>>,
}

impl VisitorElement for TreeElement {
    fn debug_name(&self) -> &'static str {
        "TreeElement"
    }
}

impl EventElement for TreeElement {
    fn event_tree_role(&self) -> EventTreeRole {
        self.role
    }

    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        self.events.set(self.events.get() + 1);
        EventResult::consumed()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }
}

impl LayoutElement for TreeElement {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.as_ref().and_then(|bounds| bounds.get())
    }
}

impl Drawable for TreeElement {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for TreeElement {}

struct EffectTreeElement {
    children: Vec<AnyElement>,
    result: EventResult,
}

impl VisitorElement for EffectTreeElement {
    fn debug_name(&self) -> &'static str {
        "EffectTreeElement"
    }
}

impl EventElement for EffectTreeElement {
    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        self.result
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }
}

impl LayoutElement for EffectTreeElement {}
impl Drawable for EffectTreeElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for EffectTreeElement {}

#[test]
fn dispatch_preserves_non_consuming_child_effects() {
    let element = EffectTreeElement {
        children: vec![EffectTreeElement {
            children: Vec::new(),
            result: EventResult::redraw(),
        }.boxed()],
        result: EventResult::ignored(),
    }.boxed();

    let result = dispatch_event(element.as_ref(), Vec2d { x: 5.0, y: 5.0 }, &ElementEvent::Cancel);

    assert!(!result.is_consumed());
    assert!(result.needs_redraw());
}

#[test]
fn captured_pointer_move_is_delivered_outside_element_bounds() {
    let events = Rc::new(Cell::new(0));
    let element = CapturingElement {
        events: Cell::new(0),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(
        element.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::touch(Vec2d { x: 5.0, y: 5.0 }, 7)),
    );
    let event = ElementEvent::PointerMove(PointerInfo::touch(Vec2d { x: 50.0, y: 50.0 }, 7));

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d { x: 50.0, y: 50.0 }, &event)
            .is_consumed()
    );
    let _ = events;
}

#[test]
fn cancel_pointer_reaches_captured_element_outside_bounds() {
    let element = CapturingElement {
        events: Cell::new(0),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let pointer = PointerKey::new(PointerSource::Touch, 7);
    let _ = dispatcher.dispatch(
        element.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );

    assert!(
        dispatcher
            .cancel_pointer(element.as_ref(), pointer)
            .is_consumed()
    );
    assert_eq!(dispatcher.capture_count(), 0);
}

#[test]
fn cancel_pointer_without_capture_is_ignored() {
    let element = CapturingElement {
        events: Cell::new(0),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    assert_eq!(
        dispatcher.cancel_pointer(element.as_ref(), PointerKey::new(PointerSource::Touch, 8),),
        EventResult::ignored()
    );
}

#[test]
fn cancel_pointer_is_not_delivered_twice_to_captured_target() {
    let element = CapturingElement {
        events: Cell::new(0),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let pointer = PointerKey::new(PointerSource::Touch, 7);
    let _ = dispatcher.dispatch(
        element.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );

    assert!(
        dispatcher
            .cancel_pointer(element.as_ref(), pointer)
            .is_consumed()
    );
    assert_eq!(
        dispatcher.cancel_pointer(element.as_ref(), pointer),
        EventResult::ignored()
    );
}

#[test]
fn recursive_dispatch_helpers_reuse_inline_scratch_without_spilling() {
    let root_events = Rc::new(Cell::new(0));
    let transparent_events = Rc::new(Cell::new(0));
    let target_events = Rc::new(Cell::new(0));
    let element = TreeElement {
        children: vec![TreeElement {
            children: vec![TreeElement {
                children: Vec::new(),
                events: target_events.clone(),
                role: EventTreeRole::IndexedTarget,
                bounds: None,
            }
            .boxed()],
            events: transparent_events.clone(),
            role: EventTreeRole::Transparent,
            bounds: None,
        }
        .boxed()],
        events: root_events.clone(),
        role: EventTreeRole::IndexedTarget,
        bounds: None,
    };
    let event = ElementEvent::Cancel;
    let mut children = EventChildren::new();

    assert!(
        dispatch_event_inner(&element, Vec2d { x: 5.0, y: 5.0 }, &event, &mut children)
            .is_consumed()
    );
    assert!(children.is_empty());
    assert!(!children.spilled());

    assert!(broadcast_event_inner(&element, &event, &mut children).is_consumed());
    assert!(children.is_empty());
    assert!(!children.spilled());
    assert_eq!(root_events.get(), 1);
    assert_eq!(transparent_events.get(), 0);
    assert_eq!(target_events.get(), 2);
}

#[test]
fn indexed_dispatch_flattens_transparent_nodes_and_keeps_target_order() {
    let root_events = Rc::new(Cell::new(0));
    let transparent_events = Rc::new(Cell::new(0));
    let target_events = Rc::new(Cell::new(0));
    let target = TreeElement {
        children: Vec::new(),
        events: target_events.clone(),
        role: EventTreeRole::IndexedTarget,
        bounds: None,
    }
    .boxed();
    let transparent = TreeElement {
        children: vec![target],
        events: transparent_events.clone(),
        role: EventTreeRole::Transparent,
        bounds: None,
    }
    .boxed();
    let root = TreeElement {
        children: vec![transparent],
        events: root_events.clone(),
        role: EventTreeRole::IndexedTarget,
        bounds: None,
    }
    .boxed();

    let mut dispatcher = EventDispatcher::new();
    let result = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 1.0, y: 1.0 },
        &ElementEvent::DragOver {
            pos: Vec2d { x: 1.0, y: 1.0 },
            source: PointerSource::Touch,
            id: 1,
        },
    );

    assert!(result.is_consumed());
    assert_eq!(target_events.get(), 1);
    assert_eq!(transparent_events.get(), 0);
    assert_eq!(root_events.get(), 0);
}

struct IndexedHitTestBoundaryRoot {
    children: Vec<AnyElement>,
    selected_child: ElementId,
    alternate_child: Option<ElementId>,
}

impl VisitorElement for IndexedHitTestBoundaryRoot {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "IndexedHitTestBoundaryRoot"
    }
}

impl EventElement for IndexedHitTestBoundaryRoot {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedHitTestBoundary
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.visit_children(visitor);
    }

    fn hit_test_children_at<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        let selected_child = if pos.x >= 5.0 {
            self.alternate_child.unwrap_or(self.selected_child)
        } else {
            self.selected_child
        };
        for child in &self.children {
            if child.id() == selected_child {
                visitor(child.as_ref());
            }
        }
    }
}

impl LayoutElement for IndexedHitTestBoundaryRoot {}

impl Drawable for IndexedHitTestBoundaryRoot {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for IndexedHitTestBoundaryRoot {}

struct IndexedBoundaryRouteTarget {
    id: usize,
    events: Rc<RefCell<Vec<usize>>>,
}

impl VisitorElement for IndexedBoundaryRouteTarget {
    fn debug_name(&self) -> &'static str {
        "IndexedBoundaryRouteTarget"
    }
}

impl EventElement for IndexedBoundaryRouteTarget {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        self.events.borrow_mut().push(self.id);
        EventResult::ignored()
    }
}

impl LayoutElement for IndexedBoundaryRouteTarget {}

impl Drawable for IndexedBoundaryRouteTarget {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for IndexedBoundaryRouteTarget {}

#[test]
fn indexed_hit_test_boundary_filters_positions_but_not_broadcasts() {
    let hits = Rc::new(RefCell::new(Vec::new()));
    let first_target = IndexedBoundaryRouteTarget {
        id: 1,
        events: hits.clone(),
    }
    .boxed();
    let third_target = IndexedBoundaryRouteTarget {
        id: 3,
        events: hits.clone(),
    }
    .boxed();
    let first_branch = TreeElement {
        children: vec![first_target, third_target],
        events: Rc::new(Cell::new(0)),
        role: EventTreeRole::Transparent,
        bounds: None,
    }
    .boxed();
    let selected_child = first_branch.id();
    let second_target = IndexedBoundaryRouteTarget {
        id: 2,
        events: hits.clone(),
    }
    .boxed();
    let second_branch = TreeElement {
        children: vec![second_target],
        events: Rc::new(Cell::new(0)),
        role: EventTreeRole::Transparent,
        bounds: None,
    }
    .boxed();
    let root = IndexedHitTestBoundaryRoot {
        children: vec![first_branch, second_branch],
        selected_child,
        alternate_child: None,
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    dispatcher.synchronize_paths(root.as_ref());
    assert!(!dispatcher.event_tree.elements().is_empty());

    let event = ElementEvent::DragOver {
        pos: Vec2d { x: 1.0, y: 1.0 },
        source: PointerSource::Mouse,
        id: 1,
    };
    assert_eq!(
        dispatcher.dispatch(root.as_ref(), Vec2d { x: 1.0, y: 1.0 }, &event),
        EventResult::ignored()
    );
    assert_eq!(*hits.borrow(), [3, 1]);

    hits.borrow_mut().clear();
    assert_eq!(broadcast_event(root.as_ref(), &ElementEvent::Cancel), EventResult::ignored());
    assert_eq!(*hits.borrow(), [2, 3, 1]);
}

struct IndexedBoundaryHoverTarget {
    id: usize,
    pointer_moves: Rc<RefCell<Vec<usize>>>,
}

impl VisitorElement for IndexedBoundaryHoverTarget {
    fn debug_name(&self) -> &'static str {
        "IndexedBoundaryHoverTarget"
    }
}

impl EventElement for IndexedBoundaryHoverTarget {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if matches!(event, ElementEvent::PointerMove(_)) {
            self.pointer_moves.borrow_mut().push(self.id);
        }
        EventResult::ignored()
    }
}

impl LayoutElement for IndexedBoundaryHoverTarget {}

impl Drawable for IndexedBoundaryHoverTarget {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for IndexedBoundaryHoverTarget {}

#[test]
fn indexed_hit_test_boundary_rechecks_pointer_moves_instead_of_replaying_a_stale_chain() {
    let pointer_moves = Rc::new(RefCell::new(Vec::new()));
    let first_target = IndexedBoundaryHoverTarget {
        id: 1,
        pointer_moves: pointer_moves.clone(),
    }
    .boxed();
    let first_branch = TreeElement {
        children: vec![first_target],
        events: Rc::new(Cell::new(0)),
        role: EventTreeRole::Transparent,
        bounds: None,
    }
    .boxed();
    let first_branch_id = first_branch.id();
    let second_target = IndexedBoundaryHoverTarget {
        id: 2,
        pointer_moves: pointer_moves.clone(),
    }
    .boxed();
    let second_branch = TreeElement {
        children: vec![second_target],
        events: Rc::new(Cell::new(0)),
        role: EventTreeRole::Transparent,
        bounds: None,
    }
    .boxed();
    let second_branch_id = second_branch.id();
    let root = IndexedHitTestBoundaryRoot {
        children: vec![first_branch, second_branch],
        selected_child: first_branch_id,
        alternate_child: Some(second_branch_id),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    for pos in [Vec2d { x: 1.0, y: 1.0 }, Vec2d { x: 10.0, y: 1.0 }] {
        let event = ElementEvent::PointerMove(PointerInfo::mouse(
            pos,
            PointerButton::Primary,
        ));
        let _ = dispatcher.dispatch(root.as_ref(), pos, &event);
    }

    assert_eq!(*pointer_moves.borrow(), [1, 2]);
}

struct DynamicBoundsTarget {
    visits: Rc<Cell<usize>>,
}

impl VisitorElement for DynamicBoundsTarget {
    fn debug_name(&self) -> &'static str {
        "DynamicBoundsTarget"
    }
}

impl EventElement for DynamicBoundsTarget {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        self.visits.set(self.visits.get() + 1);
        EventResult::ignored()
    }
}

impl LayoutElement for DynamicBoundsTarget {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some((Vec2d::ZERO, Vec2d { x: 10.0, y: 10.0 }))
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        None
    }
}

impl Drawable for DynamicBoundsTarget {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for DynamicBoundsTarget {}

#[test]
fn event_tree_keeps_dynamic_bounds_as_candidates_but_checks_live_bounds() {
    let visits = Rc::new(Cell::new(0));
    let root = DynamicBoundsTarget {
        visits: visits.clone(),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    dispatcher.synchronize_paths(root.as_ref());

    assert!(!dispatcher.event_tree.elements().is_empty());
    assert_eq!(dispatcher.event_tree.hit_test(50.0, 50.0).count(), 1);

    let pos = Vec2d { x: 50.0, y: 50.0 };
    let result = dispatcher.dispatch(
        root.as_ref(),
        pos,
        &ElementEvent::PointerMove(PointerInfo::mouse(pos, PointerButton::Primary)),
    );
    assert_eq!(result, EventResult::ignored());
    assert_eq!(visits.get(), 0);
}

#[test]
fn indexed_bounds_rebuild_after_layout_invalidation() {
    let target_events = Rc::new(Cell::new(0));
    let bounds = Rc::new(Cell::new(Some((
        Vec2d { x: 0.0, y: 0.0 },
        Vec2d { x: 10.0, y: 10.0 },
    ))));
    let target = TreeElement {
        children: Vec::new(),
        events: target_events.clone(),
        role: EventTreeRole::IndexedTarget,
        bounds: Some(bounds.clone()),
    }
    .boxed();
    let root = TreeElement {
        children: vec![target],
        events: Rc::new(Cell::new(0)),
        role: EventTreeRole::Transparent,
        bounds: None,
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let event = ElementEvent::DragOver {
        pos: Vec2d { x: 5.0, y: 5.0 },
        source: PointerSource::Touch,
        id: 2,
    };

    assert!(dispatcher.dispatch(root.as_ref(), Vec2d { x: 5.0, y: 5.0 }, &event).is_consumed());
    bounds.set(Some((
        Vec2d { x: 40.0, y: 40.0 },
        Vec2d { x: 50.0, y: 50.0 },
    )));
    advance_layout_invalidation_generation();
    assert!(!dispatcher.dispatch(root.as_ref(), Vec2d { x: 5.0, y: 5.0 }, &event).is_consumed());
    let moved_event = ElementEvent::DragOver {
        pos: Vec2d { x: 45.0, y: 45.0 },
        source: PointerSource::Touch,
        id: 2,
    };
    assert!(dispatcher.dispatch(root.as_ref(), Vec2d { x: 45.0, y: 45.0 }, &moved_event).is_consumed());
    assert_eq!(target_events.get(), 2);
}

#[test]
fn indexed_event_tree_rebuilds_after_a_subtree_replacement() {
    let first_events = Rc::new(Cell::new(0));
    let next_events = Rc::new(Cell::new(0));
    let first = TreeElement {
        children: Vec::new(),
        events: first_events.clone(),
        role: EventTreeRole::IndexedTarget,
        bounds: None,
    }
    .boxed();
    let next = TreeElement {
        children: Vec::new(),
        events: next_events.clone(),
        role: EventTreeRole::IndexedTarget,
        bounds: None,
    }
    .boxed();
    let root = SwitchingEventRoot {
        first,
        next,
        use_next: Cell::new(false),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let event = ElementEvent::DragOver {
        pos: Vec2d { x: 5.0, y: 5.0 },
        source: PointerSource::Touch,
        id: 3,
    };

    assert!(dispatcher.dispatch(root.as_ref(), Vec2d { x: 5.0, y: 5.0 }, &event).is_consumed());
    root.as_ref()
        .option_any()
        .expect("switching root remains downcastable")
        .downcast_ref::<SwitchingEventRoot>()
        .expect("the indexed test root keeps its concrete type")
        .use_next
        .set(true);
    advance_element_tree_generation();

    assert!(dispatcher.dispatch(root.as_ref(), Vec2d { x: 5.0, y: 5.0 }, &event).is_consumed());
    assert_eq!(first_events.get(), 1);
    assert_eq!(next_events.get(), 1);
}

struct SwitchingEventRoot {
    first: AnyElement,
    next: AnyElement,
    use_next: Cell<bool>,
}

impl VisitorElement for SwitchingEventRoot {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        let child = if self.use_next.get() {
            &self.next
        } else {
            &self.first
        };
        visitor(child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "SwitchingEventRoot"
    }
}

impl EventElement for SwitchingEventRoot {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.visit_children(visitor);
    }
}

impl LayoutElement for SwitchingEventRoot {}

impl Drawable for SwitchingEventRoot {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for SwitchingEventRoot {
    fn option_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
