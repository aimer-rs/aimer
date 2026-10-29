use super::*;

struct FocusTestElement {
    node: FocusNode,
    lifecycle: Rc<RefCell<Vec<&'static str>>>,
    pub(super) autofocus: bool,
}

impl VisitorElement for FocusTestElement {
    fn debug_name(&self) -> &'static str {
        "FocusTestElement"
    }
}

impl EventElement for FocusTestElement {
    fn focus_node(&self) -> Option<&FocusNode> {
        Some(&self.node)
    }

    fn autofocus(&self) -> bool {
        self.autofocus
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::FocusGained => self.lifecycle.borrow_mut().push("focus"),
            ElementEvent::FocusLost => self.lifecycle.borrow_mut().push("blur"),
            _ => {}
        }
        EventResult::ignored()
    }
}

impl LayoutElement for FocusTestElement {}
impl Drawable for FocusTestElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for FocusTestElement {}

#[test]
fn imperative_focus_is_exclusive_and_emits_each_lifecycle_event_once() {
    let first_node = FocusNode::new();
    let second_node = FocusNode::new();
    let first_lifecycle = Rc::new(RefCell::new(Vec::new()));
    let second_lifecycle = Rc::new(RefCell::new(Vec::new()));
    let root = IdentityBranch(vec![
        FocusTestElement {
            node: first_node.clone(),
            lifecycle: first_lifecycle.clone(),
            autofocus: false,
        }
        .boxed(),
        FocusTestElement {
            node: second_node.clone(),
            lifecycle: second_lifecycle.clone(),
            autofocus: false,
        }
        .boxed(),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    first_node.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    second_node.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

    assert!(!first_node.has_focus());
    assert!(second_node.has_focus());
    assert_eq!(&*first_lifecycle.borrow(), &["focus", "blur"]);
    assert_eq!(&*second_lifecycle.borrow(), &["focus"]);
}

#[test]
fn removing_the_focused_element_clears_focus_and_emits_one_blur() {
    let node = FocusNode::new();
    let lifecycle = Rc::new(RefCell::new(Vec::new()));
    let old = FocusTestElement {
        node: node.clone(),
        lifecycle: lifecycle.clone(),
        autofocus: false,
    }
    .boxed();
    let replacement = ReplacementLeaf.boxed();
    let mut dispatcher = EventDispatcher::new();
    node.request_focus();
    let _ = dispatcher.dispatch(old.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

    reconcile_generated_tree(old.as_ref(), replacement.as_ref());

    assert!(!node.has_focus());
    assert_eq!(&*lifecycle.borrow(), &["focus", "blur"]);
}

#[test]
fn first_autofocus_node_in_tree_order_wins_conflicts() {
    let first_node = FocusNode::new();
    let second_node = FocusNode::new();
    let first_lifecycle = Rc::new(RefCell::new(Vec::new()));
    let second_lifecycle = Rc::new(RefCell::new(Vec::new()));
    let root = IdentityBranch(vec![
        FocusTestElement {
            node: first_node.clone(),
            lifecycle: first_lifecycle.clone(),
            autofocus: true,
        }
        .boxed(),
        FocusTestElement {
            node: second_node.clone(),
            lifecycle: second_lifecycle.clone(),
            autofocus: true,
        }
        .boxed(),
    ])
    .boxed();

    let _ = EventDispatcher::new().dispatch(
        root.as_ref(),
        Vec2d::default(),
        &ElementEvent::Cancel,
    );

    assert!(first_node.has_focus());
    assert!(!second_node.has_focus());
    assert_eq!(&*first_lifecycle.borrow(), &["focus"]);
    assert!(second_lifecycle.borrow().is_empty());
}

#[test]
fn pointer_down_outside_focusable_elements_blurs_the_owner_once() {
    let node = FocusNode::new();
    let lifecycle = Rc::new(RefCell::new(Vec::new()));
    let outside_events = Rc::new(Cell::new(0));
    let root = IdentityBranch(vec![
        FocusTestElement {
            node: node.clone(),
            lifecycle: lifecycle.clone(),
            autofocus: false,
        }
        .boxed(),
        routed_leaf(
            (
                Vec2d { x: 20.0, y: 20.0 },
                Vec2d { x: 30.0, y: 30.0 },
            ),
            outside_events,
            false,
            false,
        ),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    node.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

    let _ = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 25.0, y: 25.0 },
        &ElementEvent::PointerDown(PointerInfo::mouse(
            Vec2d { x: 25.0, y: 25.0 },
            PointerButton::Primary,
        )),
    );

    assert!(!node.has_focus());
    assert_eq!(&*lifecycle.borrow(), &["focus", "blur"]);
}

#[test]
fn tab_and_shift_tab_traverse_focusable_elements_in_structural_order() {
    let first_node = FocusNode::new();
    let second_node = FocusNode::new();
    let first_lifecycle = Rc::new(RefCell::new(Vec::new()));
    let second_lifecycle = Rc::new(RefCell::new(Vec::new()));
    let root = IdentityBranch(vec![
        FocusTestElement {
            node: first_node.clone(),
            lifecycle: first_lifecycle,
            autofocus: false,
        }
        .boxed(),
        FocusTestElement {
            node: second_node.clone(),
            lifecycle: second_lifecycle,
            autofocus: false,
        }
        .boxed(),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    first_node.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

    let forward = dispatcher.dispatch(
        root.as_ref(),
        Vec2d::default(),
        &ElementEvent::KeyInput {
            key: NamedKey::Tab,
            action: KeyAction::Pressed,
            modifiers: Modifiers::default(),
        },
    );
    let backward = dispatcher.dispatch(
        root.as_ref(),
        Vec2d::default(),
        &ElementEvent::KeyInput {
            key: NamedKey::Tab,
            action: KeyAction::Pressed,
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        },
    );

    assert!(forward.is_consumed());
    assert!(backward.is_consumed());
    assert!(first_node.has_focus());
    assert!(!second_node.has_focus());
}

struct FocusScopeElement {
    children: Vec<AnyElement>,
    traps: Cell<bool>,
}

impl VisitorElement for FocusScopeElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "FocusScopeElement"
    }
}

impl EventElement for FocusScopeElement {
    fn traps_focus(&self) -> bool {
        self.traps.get()
    }
}

impl LayoutElement for FocusScopeElement {}
impl Drawable for FocusScopeElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for FocusScopeElement {
    fn option_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

fn focusable(node: &FocusNode) -> AnyElement {
    FocusTestElement {
        node: node.clone(),
        lifecycle: Rc::new(RefCell::new(Vec::new())),
        autofocus: false,
    }
    .boxed()
}

fn tab(dispatcher: &mut EventDispatcher, root: &dyn Element) -> EventResult {
    dispatcher.dispatch(
        root,
        Vec2d::default(),
        &ElementEvent::KeyInput {
            key: NamedKey::Tab,
            action: KeyAction::Pressed,
            modifiers: Modifiers::default(),
        },
    )
}

#[test]
fn tab_inside_a_trapping_scope_never_leaves_it() {
    let outside = FocusNode::new();
    let first = FocusNode::new();
    let second = FocusNode::new();
    let root = IdentityBranch(vec![
        focusable(&outside),
        FocusScopeElement {
            children: vec![focusable(&first), focusable(&second)],
            traps: Cell::new(true),
        }
        .boxed(),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    advance_element_tree_generation();

    first.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    assert!(first.has_focus());

    assert!(tab(&mut dispatcher, root.as_ref()).is_consumed());
    assert!(second.has_focus());

    // Wrapping at the end of the scope returns to its first target rather
    // than escaping into the tree behind it.
    assert!(tab(&mut dispatcher, root.as_ref()).is_consumed());
    assert!(first.has_focus());
    assert!(!outside.has_focus());
}

#[test]
fn a_trapping_scope_takes_focus_away_from_the_tree_behind_it() {
    let outside = FocusNode::new();
    let inside = FocusNode::new();
    let scope = FocusScopeElement {
        children: vec![focusable(&inside)],
        traps: Cell::new(false),
    }
    .boxed();
    let root = IdentityBranch(vec![focusable(&outside), scope]).boxed();
    let mut dispatcher = EventDispatcher::new();
    advance_element_tree_generation();

    outside.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    assert!(outside.has_focus());

    set_scope_traps(root.as_ref(), true);
    advance_element_tree_generation();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

    assert!(!outside.has_focus());
    assert_eq!(dispatcher.focused(), None);

    // Only the scope is reachable now.
    assert!(tab(&mut dispatcher, root.as_ref()).is_consumed());
    assert!(inside.has_focus());
}

#[test]
fn leaving_a_trapping_scope_restores_the_focus_it_displaced() {
    let outside = FocusNode::new();
    let inside = FocusNode::new();
    let outside_lifecycle = Rc::new(RefCell::new(Vec::new()));
    let root = IdentityBranch(vec![
        FocusTestElement {
            node: outside.clone(),
            lifecycle: outside_lifecycle.clone(),
            autofocus: false,
        }
        .boxed(),
        FocusScopeElement {
            children: vec![focusable(&inside)],
            traps: Cell::new(false),
        }
        .boxed(),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    advance_element_tree_generation();

    outside.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    assert!(outside.has_focus());

    set_scope_traps(root.as_ref(), true);
    advance_element_tree_generation();
    inside.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    assert!(inside.has_focus());

    set_scope_traps(root.as_ref(), false);
    advance_element_tree_generation();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

    assert!(outside.has_focus());
    assert!(!inside.has_focus());
    assert_eq!(&*outside_lifecycle.borrow(), &["focus", "blur", "focus"]);
}

#[test]
fn a_trap_elsewhere_suspends_the_tree_and_releases_tab_to_it() {
    let node = FocusNode::new();
    let lifecycle = Rc::new(RefCell::new(Vec::new()));
    let root = IdentityBranch(vec![
        FocusTestElement {
            node: node.clone(),
            lifecycle: lifecycle.clone(),
            autofocus: false,
        }
        .boxed(),
        focusable(&FocusNode::new()),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    advance_element_tree_generation();

    node.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    assert!(node.has_focus());

    let trap = FocusTrap::acquire();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

    assert!(!node.has_focus());
    assert_eq!(dispatcher.focused(), None);
    assert!(
        !tab(&mut dispatcher, root.as_ref()).is_consumed(),
        "a suspended tree must pass Tab on to the trapping region"
    );
    assert_eq!(dispatcher.focused(), None);

    drop(trap);
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

    assert!(node.has_focus());
    assert_eq!(&*lifecycle.borrow(), &["focus", "blur", "focus"]);
}

#[test]
fn text_reaches_a_nested_dispatch_root_while_the_tree_is_suspended() {
    let node = FocusNode::new();
    let events = Rc::new(Cell::new(0));
    let outside_events = Rc::new(Cell::new(0));
    let root = IdentityBranch(vec![
        FocusKeyElement {
            node: node.clone(),
            bounds: (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
            key_events: events.clone(),
            consume_keys: Rc::new(Cell::new(true)),
        }
        .boxed(),
        routed_leaf(
            (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
            outside_events.clone(),
            false,
            false,
        ),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    advance_element_tree_generation();
    node.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

    let text = ElementEvent::TextInput {
        text: "a".to_string(),
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    };
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &text);
    assert_eq!(events.get(), 1);
    assert_eq!(outside_events.get(), 0);

    let _trap = FocusTrap::acquire();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &text);

    assert_eq!(events.get(), 1, "the suspended field must not receive text");
    assert_eq!(
        outside_events.get(),
        1,
        "text must be routed so an overlay host can hand it to its own root"
    );
}

#[test]
fn a_press_outside_a_trapping_scope_does_not_take_focus_out_of_it() {
    let inside = FocusNode::new();
    let outside = FocusNode::new();
    let root = IdentityBranch(vec![
        PressableFocusElement {
            node: outside.clone(),
            bounds: (Vec2d { x: 20.0, y: 20.0 }, Vec2d { x: 30.0, y: 30.0 }),
        }
        .boxed(),
        FocusScopeElement {
            children: vec![
                PressableFocusElement {
                    node: inside.clone(),
                    bounds: (Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 10.0, y: 10.0 }),
                }
                .boxed(),
            ],
            traps: Cell::new(true),
        }
        .boxed(),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    advance_element_tree_generation();

    inside.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    assert!(inside.has_focus());

    let pos = Vec2d { x: 25.0, y: 25.0 };
    let _ = dispatcher.dispatch(
        root.as_ref(),
        pos,
        &ElementEvent::PointerDown(PointerInfo::mouse(pos, PointerButton::Primary)),
    );

    assert!(!outside.has_focus(), "a press escaped the trapping scope");
    assert!(inside.has_focus());

    // A press on nothing focusable at all leaves the scope's owner alone
    // instead of blurring it from outside.
    let empty = Vec2d { x: 50.0, y: 50.0 };
    let _ = dispatcher.dispatch(
        root.as_ref(),
        empty,
        &ElementEvent::PointerDown(PointerInfo::mouse(empty, PointerButton::Primary)),
    );

    assert!(inside.has_focus());
}

struct PressableFocusElement {
    node: FocusNode,
    bounds: (Vec2d, Vec2d),
}

impl VisitorElement for PressableFocusElement {
    fn debug_name(&self) -> &'static str {
        "PressableFocusElement"
    }
}

impl EventElement for PressableFocusElement {
    fn focus_node(&self) -> Option<&FocusNode> {
        Some(&self.node)
    }
}

impl LayoutElement for PressableFocusElement {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(self.bounds)
    }
}

impl Drawable for PressableFocusElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for PressableFocusElement {}

/// Flips the trapping flag of the scope in `root`, as a rebuild would.
fn set_scope_traps(root: &dyn Element, traps: bool) {
    let mut found = false;
    root.visit_children(&mut |child| {
        if let Some(scope) = child
            .option_any()
            .and_then(|any| any.downcast_ref::<FocusScopeElement>())
        {
            scope.traps.set(traps);
            found = true;
        }
    });
    assert!(found, "the tree under test has no focus scope");
}

pub(super) struct FocusKeyElement {
    pub(super) node: FocusNode,
    pub(super) bounds: (Vec2d, Vec2d),
    pub(super) key_events: Rc<Cell<usize>>,
    pub(super) consume_keys: Rc<Cell<bool>>,
}

impl VisitorElement for FocusKeyElement {
    fn debug_name(&self) -> &'static str {
        "FocusKeyElement"
    }
}

impl EventElement for FocusKeyElement {
    fn focus_node(&self) -> Option<&FocusNode> {
        Some(&self.node)
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if matches!(
            event,
            ElementEvent::KeyInput { .. }
                | ElementEvent::CharInput { .. }
                | ElementEvent::TextInput { .. }
                | ElementEvent::ImePreedit { .. }
        ) {
            self.key_events.set(self.key_events.get() + 1);
            return self.consume_keys.get().into();
        }
        EventResult::ignored()
    }
}

impl LayoutElement for FocusKeyElement {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(self.bounds)
    }
}
impl Drawable for FocusKeyElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for FocusKeyElement {}

#[test]
fn named_keys_target_focus_first_then_fall_back_when_ignored() {
    let first_node = FocusNode::new();
    let first_events = Rc::new(Cell::new(0));
    let second_events = Rc::new(Cell::new(0));
    let consume_first = Rc::new(Cell::new(true));
    let root = IdentityBranch(vec![
        FocusKeyElement {
            node: first_node.clone(),
            bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            key_events: first_events.clone(),
            consume_keys: consume_first.clone(),
        }
        .boxed(),
        FocusKeyElement {
            node: FocusNode::new(),
            bounds: (
                Vec2d { x: 20.0, y: 20.0 },
                Vec2d { x: 30.0, y: 30.0 },
            ),
            key_events: second_events.clone(),
            consume_keys: Rc::new(Cell::new(true)),
        }
        .boxed(),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    first_node.request_focus();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    let arrow = ElementEvent::KeyInput {
        key: NamedKey::ArrowLeft,
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    };

    let consumed_by_focus = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 25.0, y: 25.0 },
        &arrow,
    );
    consume_first.set(false);
    let consumed_by_fallback = dispatcher.dispatch(
        root.as_ref(),
        Vec2d { x: 25.0, y: 25.0 },
        &arrow,
    );

    assert!(consumed_by_focus.is_consumed());
    assert!(consumed_by_fallback.is_consumed());
    assert_eq!(first_events.get(), 2);
    assert_eq!(second_events.get(), 1);
}

/// An element that hands focus to somebody else when pressed, the way a
/// button focuses the field it belongs to. It is not focusable itself.
struct FocusRequestingElement {
    requests: FocusNode,
    bounds: (Vec2d, Vec2d),
}

impl VisitorElement for FocusRequestingElement {
    fn debug_name(&self) -> &'static str {
        "FocusRequestingElement"
    }
}

impl EventElement for FocusRequestingElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if matches!(event, ElementEvent::PointerDown(_)) {
            self.requests.request_focus();
            return EventResult::consumed();
        }
        EventResult::ignored()
    }
}

impl LayoutElement for FocusRequestingElement {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(self.bounds)
    }
}
impl Drawable for FocusRequestingElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for FocusRequestingElement {}

/// A request made while an event is delivered is granted by that event.
///
/// Focus is resolved before an event is routed, so a handler's request
/// arrives too late for that pass. Leaving it there would mean waiting for
/// whatever input comes next: a mouse that moves a pixel hides the delay,
/// but a finger that taps and lifts sends nothing more, so the field a
/// button focused would only be focused by the *next* tap.
#[test]
fn focus_requested_while_an_event_is_delivered_is_granted_by_that_event() {
    let target = FocusNode::new();
    let button = Vec2d { x: 25.0, y: 25.0 };
    let root = IdentityBranch(vec![
        FocusKeyElement {
            node: target.clone(),
            bounds: (Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }),
            key_events: Rc::new(Cell::new(0)),
            consume_keys: Rc::new(Cell::new(true)),
        }
        .boxed(),
        FocusRequestingElement {
            requests: target.clone(),
            bounds: (Vec2d { x: 20.0, y: 20.0 }, Vec2d { x: 30.0, y: 30.0 }),
        }
        .boxed(),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    let _ = dispatcher.dispatch(
        root.as_ref(),
        button,
        &ElementEvent::PointerDown(PointerInfo::mouse(button, PointerButton::Primary)),
    );

    assert!(
        target.has_focus(),
        "the press dropped focus and the handler asked for it back, so the \
         event must not end before that is settled"
    );
}
