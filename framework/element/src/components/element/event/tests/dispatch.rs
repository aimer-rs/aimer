use super::*;

struct RoutedElement {
    children: Vec<AnyElement>,
    bounds: Option<(Vec2d, Vec2d)>,
    events: Rc<Cell<usize>>,
    capture_on_down: bool,
    release_on_move: bool,
}

impl VisitorElement for RoutedElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "RoutedElement"
    }
}

impl EventElement for RoutedElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.events.set(self.events.get() + 1);
        match event {
            ElementEvent::PointerDown(pointer) if self.capture_on_down => {
                EventResult::consumed()
                    .with_pointer_capture(PointerKey::new(pointer.source, pointer.id))
            }
            ElementEvent::PointerMove(pointer) if self.release_on_move => {
                EventResult::consumed()
                    .with_pointer_release(PointerKey::new(pointer.source, pointer.id))
            }
            _ => EventResult::consumed(),
        }
    }
}

impl LayoutElement for RoutedElement {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds
    }
}

impl Drawable for RoutedElement {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for RoutedElement {}

struct HitChainCacheLeaf {
    bounds: (Vec2d, Vec2d),
    events: Rc<Cell<usize>>,
    consume_moves: bool,
}

impl VisitorElement for HitChainCacheLeaf {
    fn visit_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn debug_name(&self) -> &'static str {
        "HitChainCacheLeaf"
    }
}

impl EventElement for HitChainCacheLeaf {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if matches!(event, ElementEvent::PointerMove(_)) {
            self.events.set(self.events.get() + 1);
        }
        if self.consume_moves && matches!(event, ElementEvent::PointerMove(_)) {
            EventResult::consumed()
        } else {
            EventResult::ignored()
        }
    }
}

impl LayoutElement for HitChainCacheLeaf {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(self.bounds)
    }
}

impl Drawable for HitChainCacheLeaf {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for HitChainCacheLeaf {}

struct HitChainCacheRoot {
    role: EventTreeRole,
    child: AnyElement,
    hit_tests: Rc<Cell<usize>>,
}

impl VisitorElement for HitChainCacheRoot {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "HitChainCacheRoot"
    }
}

impl EventElement for HitChainCacheRoot {
    fn event_tree_role(&self) -> EventTreeRole {
        self.role
    }

    fn hit_test_children_at<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.hit_tests.set(self.hit_tests.get() + 1);
        if contains(self.child.as_ref(), pos) {
            visitor(self.child.as_ref());
        }
    }
}

impl LayoutElement for HitChainCacheRoot {
    fn is_layout_stable(&self) -> bool {
        true
    }
}

impl Drawable for HitChainCacheRoot {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for HitChainCacheRoot {}

struct HitChainCacheForwarder {
    child: AnyElement,
    bounds: (Vec2d, Vec2d),
    events: Rc<Cell<usize>>,
}

impl VisitorElement for HitChainCacheForwarder {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "HitChainCacheForwarder"
    }
}

impl EventElement for HitChainCacheForwarder {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn on_event_with_context(
        &self,
        event: &ElementEvent,
        context: &mut EventDispatchContext<'_, '_>,
    ) -> EventResult {
        if matches!(event, ElementEvent::PointerMove(_)) {
            self.events.set(self.events.get() + 1);
        }
        let pos = match event {
            ElementEvent::PointerMove(info) => info.pos,
            _ => Vec2d::default(),
        };
        context.dispatch_child(self.child.as_ref(), pos, event)
    }
}

impl LayoutElement for HitChainCacheForwarder {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(self.bounds)
    }
}

impl Drawable for HitChainCacheForwarder {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for HitChainCacheForwarder {}

struct HoverProbe {
    bounds: (Vec2d, Vec2d),
    hovered: Rc<Cell<bool>>,
}

impl VisitorElement for HoverProbe {
    fn debug_name(&self) -> &'static str {
        "HoverProbe"
    }
}

impl EventElement for HoverProbe {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerMove(_) => {
                self.hovered.set(true);
                EventResult::ignored()
            }
            ElementEvent::PointerExited(_, _) => {
                self.hovered.set(false);
                EventResult::ignored()
            }
            _ => EventResult::ignored(),
        }
    }
}

impl LayoutElement for HoverProbe {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(self.bounds)
    }
}

impl Drawable for HoverProbe {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for HoverProbe {}

struct HoverProbeRoot {
    bounds: (Vec2d, Vec2d),
    children: Vec<AnyElement>,
}

impl VisitorElement for HoverProbeRoot {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "HoverProbeRoot"
    }
}

impl EventElement for HoverProbeRoot {
    fn hit_test_children_at<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        for child in &self.children {
            if contains(child.as_ref(), pos) {
                visitor(child.as_ref());
            }
        }
    }
}

impl LayoutElement for HoverProbeRoot {
    fn is_layout_stable(&self) -> bool {
        true
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(self.bounds)
    }
}

impl Drawable for HoverProbeRoot {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for HoverProbeRoot {}

#[test]
fn moving_between_siblings_exits_the_previous_hover_target() {
    let first_hovered = Rc::new(Cell::new(false));
    let second_hovered = Rc::new(Cell::new(false));
    let root = HoverProbeRoot {
        bounds: (Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }),
        children: vec![
            HoverProbe {
                bounds: (Vec2d::default(), Vec2d { x: 40.0, y: 40.0 }),
                hovered: first_hovered.clone(),
            }
            .boxed(),
            HoverProbe {
                bounds: (
                    Vec2d { x: 60.0, y: 0.0 },
                    Vec2d { x: 100.0, y: 40.0 },
                ),
                hovered: second_hovered.clone(),
            }
            .boxed(),
        ],
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    let first = Vec2d { x: 20.0, y: 20.0 };
    let second = Vec2d { x: 80.0, y: 20.0 };
    let outside = Vec2d { x: 20.0, y: 80.0 };
    let move_pointer = |dispatcher: &mut EventDispatcher, pos| {
        let _ = dispatcher.dispatch(
            root.as_ref(),
            pos,
            &ElementEvent::PointerMove(PointerInfo::mouse(
                pos,
                PointerButton::Primary,
            )),
        );
    };

    move_pointer(&mut dispatcher, first);
    assert!(first_hovered.get());
    assert!(!second_hovered.get());

    move_pointer(&mut dispatcher, second);
    assert!(!first_hovered.get());
    assert!(second_hovered.get());

    move_pointer(&mut dispatcher, outside);
    assert!(!second_hovered.get());
}

/// A row that lives on while the element above it is replaced still owes its
/// exit. The dispatcher of a scrollable is rooted at the scrollable's child, and
/// that child is replaced whenever its content rebuilds; the rows inside are
/// carried over by identity, hover state included.
#[test]
fn a_hover_target_that_outlives_its_replaced_root_is_still_exited() {
    let hovered = Rc::new(Cell::new(false));
    let build_root = |hovered: &Rc<Cell<bool>>| {
        HoverProbeRoot {
            bounds: (Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }),
            children: vec![
                HoverProbe {
                    bounds: (Vec2d::default(), Vec2d { x: 40.0, y: 40.0 }),
                    hovered: hovered.clone(),
                }
                .boxed(),
            ],
        }
        .boxed()
    };
    let mut dispatcher = EventDispatcher::new();
    let move_pointer = |dispatcher: &mut EventDispatcher, root: &AnyElement, pos: Vec2d| {
        let _ = dispatcher.dispatch(
            root.as_ref(),
            pos,
            &ElementEvent::PointerMove(PointerInfo::mouse(pos, PointerButton::Primary)),
        );
    };

    let old_root = build_root(&hovered);
    move_pointer(&mut dispatcher, &old_root, Vec2d { x: 20.0, y: 20.0 });
    assert!(hovered.get());

    // The same row under a root that is a new element: the identities of the
    // row carry over, the identity of the root does not.
    let new_root = build_root(&hovered);
    let mut old_row = None;
    old_root.visit_children(&mut |child| old_row = child.element_id());
    let mut new_row_set = false;
    new_root.visit_children(&mut |child| {
        if let Some(id) = old_row {
            child.set_element_id(id);
            new_row_set = true;
        }
    });
    assert!(new_row_set);
    assert_ne!(old_root.element_id(), new_root.element_id());

    move_pointer(&mut dispatcher, &new_root, Vec2d { x: 80.0, y: 80.0 });
    assert!(!hovered.get(), "the row kept its hover after the pointer left it");
}

/// An explicit exit ends the pointer's hover wherever the pointer is when it
/// is sent. A container that stops offering moves to a nested dispatcher —
/// because the pointer left the content for something drawn over its edge —
/// says so with one, from a position that is nowhere near what was hovered.
#[test]
fn an_explicit_exit_releases_the_hover_whatever_its_position() {
    let hovered = Rc::new(Cell::new(false));
    let root = HoverProbeRoot {
        bounds: (Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }),
        children: vec![
            HoverProbe {
                bounds: (Vec2d::default(), Vec2d { x: 40.0, y: 40.0 }),
                hovered: hovered.clone(),
            }
            .boxed(),
        ],
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let on_probe = Vec2d { x: 20.0, y: 20.0 };
    let _ = dispatcher.dispatch(
        root.as_ref(),
        on_probe,
        &ElementEvent::PointerMove(PointerInfo::mouse(on_probe, PointerButton::Primary)),
    );
    assert!(hovered.get());
    assert!(dispatcher.is_hovering(PointerKey::new(PointerSource::Mouse, 0)));

    let away = Vec2d { x: 90.0, y: 90.0 };
    let _ = dispatcher.dispatch(root.as_ref(), away, &ElementEvent::PointerExited(PointerSource::Mouse, 0));

    assert!(!hovered.get(), "the probe stayed hovered after an explicit exit");
    assert!(!dispatcher.is_hovering(PointerKey::new(PointerSource::Mouse, 0)));
}

#[cfg(feature = "frame-stats")]
#[test]
fn uncaptured_pointer_moves_reuse_the_last_hit_chain() {
    let hit_tests = Rc::new(Cell::new(0));
    let leaf_events = Rc::new(Cell::new(0));
    let root = HitChainCacheRoot {
        role: EventTreeRole::IndexedTarget,
        child: HitChainCacheLeaf {
            bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            events: leaf_events.clone(),
            consume_moves: false,
        }
        .boxed(),
        hit_tests: hit_tests.clone(),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let pos = Vec2d { x: 5.0, y: 5.0 };

    reset_routed_event_visit_count();
    let _ = dispatcher.dispatch(
        root.as_ref(),
        pos,
        &ElementEvent::PointerMove(PointerInfo::mouse(pos, PointerButton::Primary)),
    );
    let first_visits = take_routed_event_visit_count();
    let first_hit_tests = hit_tests.get();

    reset_routed_event_visit_count();
    let _ = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 6.0, y: 6.0 },
        &ElementEvent::PointerMove(PointerInfo::mouse(
            Vec2d { x: 6.0, y: 6.0 },
            PointerButton::Primary,
        )),
    );
    let second_visits = take_routed_event_visit_count();

    assert_eq!(hit_tests.get(), first_hit_tests);
    assert_eq!(first_visits, 2);
    assert_eq!(second_visits, first_visits);
    assert_eq!(leaf_events.get(), 2);
}

#[test]
fn overlapping_hover_chain_membership_is_linear() {
    const CHILD_COUNT: usize = 64;
    let bounds = (Vec2d::default(), Vec2d { x: 100.0, y: 100.0 });
    let hovered: Vec<_> = (0..CHILD_COUNT).map(|_| Rc::new(Cell::new(false))).collect();
    let children = hovered
        .iter()
        .map(|hovered| {
            HoverProbe {
                bounds,
                hovered: hovered.clone(),
            }
            .boxed()
        })
        .collect();
    let root = HoverProbeRoot { bounds, children }.boxed();
    let mut dispatcher = EventDispatcher::new();
    let first = Vec2d { x: 10.0, y: 10.0 };
    let second = Vec2d { x: 20.0, y: 20.0 };

    reset_hover_membership_check_count();
    for pos in [first, second] {
        let _ = dispatcher.dispatch(
            root.as_ref(),
            pos,
            &ElementEvent::PointerMove(PointerInfo::mouse(
                pos,
                PointerButton::Primary,
            )),
        );
    }

    assert!(hovered.iter().all(|hovered| hovered.get()));
    let checks = take_hover_membership_check_count();
    assert!(
        checks <= (CHILD_COUNT as u64 + 1) * 2,
        "hover membership checks must remain linear, observed {checks}"
    );
}

#[test]
fn cached_hit_chain_falls_back_when_the_pointer_leaves_the_chain() {
    let hit_tests = Rc::new(Cell::new(0));
    let leaf_events = Rc::new(Cell::new(0));
    let root = HitChainCacheRoot {
        role: EventTreeRole::IndexedHitTestBoundary,
        child: HitChainCacheLeaf {
            bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            events: leaf_events.clone(),
            consume_moves: false,
        }
        .boxed(),
        hit_tests: hit_tests.clone(),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let inside = Vec2d { x: 5.0, y: 5.0 };

    let _ = dispatcher.dispatch(
        root.as_ref(),
        inside,
        &ElementEvent::PointerMove(PointerInfo::mouse(inside, PointerButton::Primary)),
    );
    let first_hit_tests = hit_tests.get();

    let outside = Vec2d { x: 50.0, y: 50.0 };
    let _ = dispatcher.dispatch(
        root.as_ref(),
        outside,
        &ElementEvent::PointerMove(PointerInfo::mouse(
            outside,
            PointerButton::Primary,
        )),
    );
    let outside_hit_tests = hit_tests.get();

    let _ = dispatcher.dispatch(
        root.as_ref(),
        inside,
        &ElementEvent::PointerMove(PointerInfo::mouse(inside, PointerButton::Primary)),
    );

    assert!(outside_hit_tests > first_hit_tests);
    assert!(hit_tests.get() > outside_hit_tests);
    assert_eq!(leaf_events.get(), 2);
}

#[test]
fn cached_hit_chain_is_invalidated_by_a_subtree_generation_change() {
    let hit_tests = Rc::new(Cell::new(0));
    let leaf_events = Rc::new(Cell::new(0));
    let root = HitChainCacheRoot {
        role: EventTreeRole::IndexedHitTestBoundary,
        child: HitChainCacheLeaf {
            bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            events: leaf_events.clone(),
            consume_moves: false,
        }
        .boxed(),
        hit_tests: hit_tests.clone(),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let pos = Vec2d { x: 5.0, y: 5.0 };

    let _ = dispatcher.dispatch(
        root.as_ref(),
        pos,
        &ElementEvent::PointerMove(PointerInfo::mouse(pos, PointerButton::Primary)),
    );
    let first_hit_tests = hit_tests.get();

    let generation = root.subtree_generation();
    root.set_subtree_generation(generation.wrapping_add(1));
    let _ = dispatcher.dispatch(
        root.as_ref(),
        pos,
        &ElementEvent::PointerMove(PointerInfo::mouse(pos, PointerButton::Primary)),
    );

    assert!(hit_tests.get() > first_hit_tests);
    assert_eq!(leaf_events.get(), 2);
}

#[test]
fn consuming_pointer_moves_are_not_cached() {
    let hit_tests = Rc::new(Cell::new(0));
    let leaf_events = Rc::new(Cell::new(0));
    let root = HitChainCacheRoot {
        role: EventTreeRole::IndexedHitTestBoundary,
        child: HitChainCacheLeaf {
            bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            events: leaf_events.clone(),
            consume_moves: true,
        }
        .boxed(),
        hit_tests: hit_tests.clone(),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let first_pos = Vec2d { x: 5.0, y: 5.0 };
    let second_pos = Vec2d { x: 6.0, y: 6.0 };

    let _ = dispatcher.dispatch(
        root.as_ref(),
        first_pos,
        &ElementEvent::PointerMove(PointerInfo::mouse(
            first_pos,
            PointerButton::Primary,
        )),
    );
    let first_hit_tests = hit_tests.get();

    let _ = dispatcher.dispatch(
        root.as_ref(),
        second_pos,
        &ElementEvent::PointerMove(PointerInfo::mouse(
            second_pos,
            PointerButton::Primary,
        )),
    );

    assert!(hit_tests.get() > first_hit_tests);
    assert_eq!(leaf_events.get(), 2);
}

#[test]
fn cached_hit_chain_replays_a_forwarding_boundary_once() {
    let hit_tests = Rc::new(Cell::new(0));
    let forwarder_events = Rc::new(Cell::new(0));
    let leaf_events = Rc::new(Cell::new(0));
    let root = HitChainCacheRoot {
        role: EventTreeRole::IndexedTarget,
        child: HitChainCacheForwarder {
            child: HitChainCacheLeaf {
                bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
                events: leaf_events.clone(),
                consume_moves: false,
            }
            .boxed(),
            bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            events: forwarder_events.clone(),
        }
        .boxed(),
        hit_tests: hit_tests.clone(),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let first_pos = Vec2d { x: 5.0, y: 5.0 };
    let second_pos = Vec2d { x: 6.0, y: 6.0 };

    let _ = dispatcher.dispatch(
        root.as_ref(),
        first_pos,
        &ElementEvent::PointerMove(PointerInfo::mouse(
            first_pos,
            PointerButton::Primary,
        )),
    );
    let first_hit_tests = hit_tests.get();
    assert!(dispatcher.hit_chain_cache.is_some(), "the fixed indexed route is cacheable");

    let _ = dispatcher.dispatch(
        root.as_ref(),
        second_pos,
        &ElementEvent::PointerMove(PointerInfo::mouse(
            second_pos,
            PointerButton::Primary,
        )),
    );

    assert_eq!(hit_tests.get(), first_hit_tests);
    assert_eq!(forwarder_events.get(), 2);
    assert_eq!(leaf_events.get(), 2);
}

pub(super) fn routed_leaf(
    bounds: (Vec2d, Vec2d),
    events: Rc<Cell<usize>>,
    capture_on_down: bool,
    release_on_move: bool,
) -> AnyElement {
    RoutedElement {
        children: Vec::new(),
        bounds: Some(bounds),
        events,
        capture_on_down,
        release_on_move,
    }
    .boxed()
}

struct ForwardOnlyHitTestElement {
    child: AnyElement,
}

impl VisitorElement for ForwardOnlyHitTestElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "ForwardOnlyHitTestElement"
    }
}

impl EventElement for ForwardOnlyHitTestElement {
    fn hit_test_children_at<'a>(
        &'a self,
        _pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        visitor(self.child.as_ref());
    }

    fn hit_test_children_reversed<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {
        panic!("routed dispatch should use the shared forward hit-test scratch");
    }
}

impl LayoutElement for ForwardOnlyHitTestElement {}

impl Drawable for ForwardOnlyHitTestElement {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for ForwardOnlyHitTestElement {}

#[test]
fn routed_dispatch_uses_position_aware_forward_scratch() {
    let events = Rc::new(Cell::new(0));
    let root = ForwardOnlyHitTestElement {
        child: routed_leaf(
            (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            events.clone(),
            false,
            false,
        ),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    let result = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::mouse(
            Vec2d { x: 5.0, y: 5.0 },
            PointerButton::Primary,
        )),
    );

    assert!(result.is_consumed());
    assert_eq!(events.get(), 1);
}

#[test]
fn routed_hit_testing_does_not_descend_through_an_outside_parent() {
    let child_events = Rc::new(Cell::new(0));
    let child = routed_leaf(
        (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
        child_events.clone(),
        false,
        false,
    );
    let parent = RoutedElement {
        children: vec![child],
        bounds: Some((
            Vec2d { x: 20.0, y: 20.0 },
            Vec2d { x: 30.0, y: 30.0 },
        )),
        events: Rc::new(Cell::new(0)),
        capture_on_down: false,
        release_on_move: false,
    }
    .boxed();
    let root = RoutedIdentityBranch(vec![parent]).boxed();
    let pos = Vec2d { x: 5.0, y: 5.0 };

    let _ = EventDispatcher::new().dispatch(
        root.as_ref(),
        pos,
        &ElementEvent::PointerDown(PointerInfo::mouse(pos, PointerButton::Primary)),
    );

    assert_eq!(child_events.get(), 0);
}

/// An element that carries a drag: it takes the pointer on press and then
/// asks for the drag to be routed to whoever is underneath.
struct DragCarrier {
    bounds: (Vec2d, Vec2d),
    follows_up: bool,
}

impl VisitorElement for DragCarrier {
    fn visit_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn debug_name(&self) -> &'static str {
        "DragCarrier"
    }
}

impl EventElement for DragCarrier {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerDown(pointer) => EventResult::consumed()
                .with_pointer_capture(PointerKey::new(pointer.source, pointer.id)),
            ElementEvent::PointerMove(_) if self.follows_up => {
                EventResult::consumed().with_follow_up(FollowUp::DragOver)
            }
            ElementEvent::PointerUp(_) if self.follows_up => {
                EventResult::consumed().with_follow_up(FollowUp::DragDrop)
            }
            _ => EventResult::consumed(),
        }
    }
}

impl LayoutElement for DragCarrier {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(self.bounds)
    }
}

impl Drawable for DragCarrier {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for DragCarrier {}

/// An element that counts the drags routed onto it.
struct DragReceiver {
    bounds: (Vec2d, Vec2d),
    overs: Rc<Cell<usize>>,
    drops: Rc<Cell<usize>>,
}

impl VisitorElement for DragReceiver {
    fn visit_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn debug_name(&self) -> &'static str {
        "DragReceiver"
    }
}

impl EventElement for DragReceiver {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::DragOver { .. } => {
                self.overs.set(self.overs.get() + 1);
                EventResult::consumed()
            }
            ElementEvent::DragDrop { .. } => {
                self.drops.set(self.drops.get() + 1);
                EventResult::consumed()
            }
            _ => EventResult::ignored(),
        }
    }
}

impl LayoutElement for DragReceiver {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(self.bounds)
    }
}

impl Drawable for DragReceiver {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for DragReceiver {}

/// A carrier on the left, a receiver on the right, side by side and not
/// overlapping — the shape of a card being dragged out of one column and
/// onto another.
fn drag_tree(
    follows_up: bool,
    overs: Rc<Cell<usize>>,
    drops: Rc<Cell<usize>>,
) -> (AnyElement, Vec2d, Vec2d) {
    let carrier = DragCarrier {
        bounds: (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
        follows_up,
    }
    .boxed();
    let receiver = DragReceiver {
        bounds: (Vec2d { x: 20.0, y: 0.0 }, Vec2d { x: 30.0, y: 10.0 }),
        overs,
        drops,
    }
    .boxed();
    let root = RoutedIdentityBranch(vec![carrier, receiver]).boxed();

    (
        root,
        Vec2d { x: 5.0, y: 5.0 },
        Vec2d { x: 25.0, y: 5.0 },
    )
}

/// A parent that is transparent to hit testing: it has no bounds of its own
/// and does not consume, so a routed pass reaches both of its children.
struct RoutedIdentityBranch(Vec<AnyElement>);

impl VisitorElement for RoutedIdentityBranch {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.0 {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "RoutedIdentityBranch"
    }
}

impl EventElement for RoutedIdentityBranch {}

impl LayoutElement for RoutedIdentityBranch {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        None
    }
}

impl Drawable for RoutedIdentityBranch {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for RoutedIdentityBranch {}

/// The element carrying a drag owns the pointer, so it is the only element
/// that hears the move — which is exactly why the drag has to be routed
/// separately to whoever is underneath.
#[test]
fn a_captured_drag_reaches_the_element_under_the_pointer() {
    let overs = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let (root, source, over_receiver) = drag_tree(true, overs.clone(), drops.clone());
    let mut dispatcher = EventDispatcher::new();

    let _ = dispatcher.dispatch(
        root.as_ref(),
        source,
        &ElementEvent::PointerDown(PointerInfo::mouse(source, PointerButton::Primary)),
    );
    let _ = dispatcher.dispatch(
        root.as_ref(),
        over_receiver,
        &ElementEvent::PointerMove(PointerInfo::mouse(over_receiver, PointerButton::Primary)),
    );

    assert_eq!(overs.get(), 1, "the receiver was not told about the drag");
    assert_eq!(drops.get(), 0);
}

/// A drop is the end of the drag: it is delivered once, and the pointer
/// goes back to routing normally.
#[test]
fn a_drop_is_delivered_once_and_releases_the_capture() {
    let overs = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let (root, source, over_receiver) = drag_tree(true, overs.clone(), drops.clone());
    let pointer = PointerKey::new(PointerSource::Mouse, 0);
    let mut dispatcher = EventDispatcher::new();

    let _ = dispatcher.dispatch(
        root.as_ref(),
        source,
        &ElementEvent::PointerDown(PointerInfo::mouse(source, PointerButton::Primary)),
    );
    assert!(dispatcher.is_captured(pointer));

    let _ = dispatcher.dispatch(
        root.as_ref(),
        over_receiver,
        &ElementEvent::PointerUp(PointerInfo::mouse(over_receiver, PointerButton::Primary)),
    );

    assert_eq!(drops.get(), 1);
    assert!(!dispatcher.is_captured(pointer));
}

/// The cost of drag support for an application that never drags: a captured
/// move that asks for nothing traverses nothing.
#[test]
fn a_capture_that_asks_for_no_follow_up_traverses_nothing() {
    let overs = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let (root, source, over_receiver) = drag_tree(false, overs.clone(), drops.clone());
    let mut dispatcher = EventDispatcher::new();

    let _ = dispatcher.dispatch(
        root.as_ref(),
        source,
        &ElementEvent::PointerDown(PointerInfo::mouse(source, PointerButton::Primary)),
    );
    let _ = dispatcher.dispatch(
        root.as_ref(),
        over_receiver,
        &ElementEvent::PointerMove(PointerInfo::mouse(over_receiver, PointerButton::Primary)),
    );

    assert_eq!(overs.get(), 0);
}

#[test]
fn focus_directed_text_reaches_elements_the_pointer_is_not_over() {
    let node = FocusNode::new();
    let events = Rc::new(Cell::new(0));
    let root = FocusKeyElement {
        node: node.clone(),
        bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
        key_events: events.clone(),
        consume_keys: Rc::new(Cell::new(true)),
    }
    .boxed();
    let outside = Vec2d {
        x: f32::MIN,
        y: f32::MIN,
    };

    let mut dispatcher = EventDispatcher::new();
    node.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), outside, &ElementEvent::Cancel);
    let result = dispatcher.dispatch(
        root.as_ref(),
        outside,
        &ElementEvent::TextInput {
            text: "你好".into(),
            action: KeyAction::Pressed,
            modifiers: Modifiers::default(),
        },
    );

    assert!(result.is_consumed());
    assert_eq!(events.get(), 1);
}

#[test]
fn named_keys_stay_positional() {
    let events = Rc::new(Cell::new(0));
    let leaf = routed_leaf(
        (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
        events.clone(),
        false,
        false,
    );
    let root = RoutedElement {
        children: vec![leaf],
        bounds: Some((Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 })),
        events: Rc::new(Cell::new(0)),
        capture_on_down: false,
        release_on_move: false,
    }
    .boxed();

    let result = EventDispatcher::new().dispatch(
        root.as_ref(),
        Vec2d { x: 50.0, y: 50.0 },
        &ElementEvent::KeyInput {
            key: NamedKey::ArrowDown,
            action: KeyAction::Pressed,
            modifiers: Modifiers::default(),
        },
    );

    assert!(!result.is_consumed());
    assert_eq!(events.get(), 0);
}

#[test]
fn focus_directed_delivery_stops_at_the_first_consumer() {
    let first_node = FocusNode::new();
    let first_events = Rc::new(Cell::new(0));
    let second_events = Rc::new(Cell::new(0));
    let root = IdentityBranch(vec![
        FocusKeyElement {
            node: first_node.clone(),
            bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            key_events: first_events.clone(),
            consume_keys: Rc::new(Cell::new(true)),
        }
        .boxed(),
        FocusKeyElement {
            node: FocusNode::new(),
            bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            key_events: second_events.clone(),
            consume_keys: Rc::new(Cell::new(true)),
        }
        .boxed(),
    ])
    .boxed();

    let mut dispatcher = EventDispatcher::new();
    first_node.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    let _ = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::ImePreedit {
            text: "ni".into(),
            cursor: None,
        },
    );

    assert_eq!(first_events.get(), 1);
    assert_eq!(second_events.get(), 0);
}

#[test]
fn captured_dispatch_visits_only_the_saved_path() {
    let target_events = Rc::new(Cell::new(0));
    let unrelated_events = Rc::new(Cell::new(0));
    let target = routed_leaf(
        (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
        target_events.clone(),
        true,
        false,
    );
    let target_id = target.id();
    let mut children = Vec::new();
    for _ in 0..128 {
        children.push(routed_leaf(
            (Vec2d { x: 20.0, y: 20.0 }, Vec2d { x: 30.0, y: 30.0 }),
            unrelated_events.clone(),
            false,
            false,
        ));
    }
    children.push(target);
    let root = RoutedElement {
        children,
        bounds: None,
        events: Rc::new(Cell::new(0)),
        capture_on_down: false,
        release_on_move: false,
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let touch = PointerKey::new(PointerSource::Touch, 9);

    let _ = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            touch.source,
            touch.id,
            PointerButton::Primary,
        )),
    );
    target_events.set(0);
    unrelated_events.set(0);

    let _ = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 500.0, y: 500.0 },
        &ElementEvent::PointerMove(PointerInfo::new(
            Vec2d { x: 500.0, y: 500.0 },
            touch.source,
            touch.id,
            PointerButton::Primary,
        )),
    );

    assert_eq!(dispatcher.captured_owner(touch), Some(target_id));
    assert_eq!(target_events.get(), 1);
    assert_eq!(unrelated_events.get(), 0);
}

#[test]
fn equal_mouse_and_touch_ids_capture_independently() {
    let mouse_events = Rc::new(Cell::new(0));
    let touch_events = Rc::new(Cell::new(0));
    let mouse = routed_leaf(
        (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
        mouse_events,
        true,
        false,
    );
    let mouse_id = mouse.id();
    let touch = routed_leaf(
        (Vec2d { x: 20.0, y: 20.0 }, Vec2d { x: 30.0, y: 30.0 }),
        touch_events,
        true,
        false,
    );
    let touch_id = touch.id();
    let root = RoutedElement {
        children: vec![mouse, touch],
        bounds: None,
        events: Rc::new(Cell::new(0)),
        capture_on_down: false,
        release_on_move: false,
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let mouse_pointer = PointerKey::new(PointerSource::Mouse, 0);
    let touch_pointer = PointerKey::new(PointerSource::Touch, 0);

    let _ = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            mouse_pointer.source,
            mouse_pointer.id,
            PointerButton::Primary,
        )),
    );
    let _ = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 25.0, y: 25.0 },
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 25.0, y: 25.0 },
            touch_pointer.source,
            touch_pointer.id,
            PointerButton::Primary,
        )),
    );

    assert_eq!(dispatcher.captured_owner(mouse_pointer), Some(mouse_id));
    assert_eq!(dispatcher.captured_owner(touch_pointer), Some(touch_id));
}

#[test]
fn explicit_release_request_clears_capture() {
    let events = Rc::new(Cell::new(0));
    let target = routed_leaf(
        (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
        events,
        true,
        true,
    );
    let pointer = PointerKey::new(PointerSource::Mouse, 0);
    let mut dispatcher = EventDispatcher::new();

    let _ = dispatcher.dispatch(
        target.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );
    let _ = dispatcher.dispatch(
        target.as_ref(),
        Vec2d { x: 50.0, y: 50.0 },
        &ElementEvent::PointerMove(PointerInfo::new(
            Vec2d { x: 50.0, y: 50.0 },
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );

    assert_eq!(dispatcher.captured_owner(pointer), None);
}

#[test]
fn compatible_rebuild_preserves_active_capture() {
    let old_target_events = Rc::new(Cell::new(0));
    let old = RoutedElement {
        children: vec![routed_leaf(
            (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
            old_target_events,
            true,
            false,
        )],
        bounds: None,
        events: Rc::new(Cell::new(0)),
        capture_on_down: false,
        release_on_move: false,
    }
    .boxed();
    let pointer = PointerKey::new(PointerSource::Touch, 3);
    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(
        old.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );
    let owner = dispatcher.captured_owner(pointer);

    let rebuilt_target_events = Rc::new(Cell::new(0));
    let rebuilt = RoutedElement {
        children: vec![routed_leaf(
            (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
            rebuilt_target_events.clone(),
            true,
            false,
        )],
        bounds: None,
        events: Rc::new(Cell::new(0)),
        capture_on_down: false,
        release_on_move: false,
    }
    .boxed();
    reconcile_generated_tree(old.as_ref(), rebuilt.as_ref());

    let _ = dispatcher.dispatch(
        rebuilt.as_ref(),
        Vec2d { x: 50.0, y: 50.0 },
        &ElementEvent::PointerMove(PointerInfo::new(
            Vec2d { x: 50.0, y: 50.0 },
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );

    assert_eq!(dispatcher.captured_owner(pointer), owner);
    assert_eq!(rebuilt_target_events.get(), 1);
}

struct CaptureChainElement {
    child: Option<AnyElement>,
    pointer: PointerKey,
}

impl VisitorElement for CaptureChainElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if let Some(child) = &self.child {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "CaptureChainElement"
    }
}

impl EventElement for CaptureChainElement {
    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored().with_pointer_capture(self.pointer)
    }
}

impl LayoutElement for CaptureChainElement {}
impl Drawable for CaptureChainElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for CaptureChainElement {}

#[test]
fn deepest_capture_request_wins() {
    let pointer = PointerKey::new(PointerSource::Touch, 6);
    let child = CaptureChainElement {
        child: None,
        pointer,
    }
    .boxed();
    let child_id = child.id();
    let root = CaptureChainElement {
        child: Some(child),
        pointer,
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    let _ = dispatcher.dispatch(
        root.as_ref(),
        Vec2d::default(),
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d::default(),
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );

    assert_eq!(dispatcher.captured_owner(pointer), Some(child_id));
}

#[test]
fn pointer_up_and_cancel_release_captures() {
    let events = Rc::new(Cell::new(0));
    let target = routed_leaf(
        (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
        events,
        true,
        false,
    );
    let root = RoutedElement {
        children: vec![target],
        bounds: None,
        events: Rc::new(Cell::new(0)),
        capture_on_down: false,
        release_on_move: false,
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let mouse = PointerKey::new(PointerSource::Mouse, 0);
    let touch = PointerKey::new(PointerSource::Touch, 7);

    for pointer in [mouse, touch] {
        let _ = dispatcher.dispatch(
            root.as_ref(),
            Vec2d { x: 5.0, y: 5.0 },
            &ElementEvent::PointerDown(PointerInfo::new(
                Vec2d { x: 5.0, y: 5.0 },
                pointer.source,
                pointer.id,
                PointerButton::Primary,
            )),
        );
    }
    let _ = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 50.0, y: 50.0 },
        &ElementEvent::PointerUp(PointerInfo::new(
            Vec2d { x: 50.0, y: 50.0 },
            mouse.source,
            mouse.id,
            PointerButton::Primary,
        )),
    );
    assert_eq!(dispatcher.captured_owner(mouse), None);
    assert!(dispatcher.captured_owner(touch).is_some());

    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    assert_eq!(dispatcher.capture_count(), 0);
}

#[test]
fn removed_owner_clears_capture_without_falling_back() {
    let old_events = Rc::new(Cell::new(0));
    let old = routed_leaf(
        (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
        old_events,
        true,
        false,
    );
    let mut dispatcher = EventDispatcher::new();
    let pointer = PointerKey::new(PointerSource::Touch, 5);
    let _ = dispatcher.dispatch(
        old.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );

    let replacement = ReplacementLeaf.boxed();
    reconcile_generated_tree(old.as_ref(), replacement.as_ref());
    let result = dispatcher.dispatch(
        replacement.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerMove(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );

    assert_eq!(dispatcher.captured_owner(pointer), None);
    assert_eq!(result, EventResult::ignored());
}

#[test]
fn invalid_saved_path_clears_capture_without_falling_back() {
    let events = Rc::new(Cell::new(0));
    let target = routed_leaf(
        (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
        events.clone(),
        true,
        false,
    );
    let pointer = PointerKey::new(PointerSource::Touch, 11);
    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(
        target.as_ref(),
        Vec2d { x: 5.0, y: 5.0 },
        &ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        )),
    );
    let owner = dispatcher
        .captured_owner(pointer)
        .expect("pointer must be captured after down");
    dispatcher
        .path_indices
        .insert(owner, usize::MAX);
    events.set(0);

    let move_event = ElementEvent::PointerMove(PointerInfo::new(
        Vec2d { x: 5.0, y: 5.0 },
        pointer.source,
        pointer.id,
        PointerButton::Primary,
    ));
    let result = dispatcher.dispatch_captured(target.as_ref(), pointer, &move_event);

    assert_eq!(result, EventResult::ignored());
    assert_eq!(dispatcher.captured_owner(pointer), None);
    assert_eq!(events.get(), 0);
}

/// Hit testing sees a position and nothing else, so an element that must treat a
/// finger differently from a cursor (an edge swipe) reads which one spoke last.
/// It has to be known *before* the event is routed, hence the dispatcher noting
/// it on the way in.
#[test]
fn the_dispatcher_notes_which_kind_of_pointer_spoke_last() {
    use aimer_events::pointer::{PointerSource, last_pointer_source};

    let root = RoutedElement {
        children: Vec::new(),
        bounds: Some((Vec2d::default(), Vec2d { x: 100.0, y: 100.0 })),
        events: Rc::new(Cell::new(0)),
        capture_on_down: false,
        release_on_move: false,
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    let pos = Vec2d { x: 10.0, y: 10.0 };
    let mouse = ElementEvent::PointerMove(PointerInfo::mouse(pos, PointerButton::Primary));
    let touch = ElementEvent::PointerDown(PointerInfo::new(pos, PointerSource::Touch, 1, PointerButton::Primary));

    let _ = dispatcher.dispatch(root.as_ref(), pos, &mouse);
    assert_eq!(last_pointer_source(), PointerSource::Mouse);
    let _ = dispatcher.dispatch(root.as_ref(), pos, &touch);
    assert_eq!(last_pointer_source(), PointerSource::Touch);
    let _ = dispatcher.dispatch(root.as_ref(), pos, &mouse);
    assert_eq!(last_pointer_source(), PointerSource::Mouse);
}
