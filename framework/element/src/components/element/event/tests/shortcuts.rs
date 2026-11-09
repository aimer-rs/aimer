use aimer_shortcut::{ChordMods, Key, KeyChord, ShortcutScope};

use super::*;

/// An element that only carries a scope around its children.
struct ScopeElement {
    children: Vec<AnyElement>,
    scope: ShortcutScope,
}

impl VisitorElement for ScopeElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "ScopeElement"
    }
}

impl EventElement for ScopeElement {
    fn shortcut_scope(&self) -> Option<&ShortcutScope> {
        Some(&self.scope)
    }
}

impl LayoutElement for ScopeElement {}
impl Drawable for ScopeElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for ScopeElement {}

/// A focus target that consumes every key it is offered.
struct Greedy(FocusNode);

impl VisitorElement for Greedy {
    fn debug_name(&self) -> &'static str {
        "Greedy"
    }
}

impl EventElement for Greedy {
    fn focus_node(&self) -> Option<&FocusNode> {
        Some(&self.0)
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::KeyInput { .. } => EventResult::consumed(),
            _ => EventResult::ignored(),
        }
    }
}

impl LayoutElement for Greedy {}
impl Drawable for Greedy {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for Greedy {}

/// A focus target that ignores everything.
struct Passive(FocusNode);

impl VisitorElement for Passive {
    fn debug_name(&self) -> &'static str {
        "Passive"
    }
}

impl EventElement for Passive {
    fn focus_node(&self) -> Option<&FocusNode> {
        Some(&self.0)
    }
}

impl LayoutElement for Passive {}
impl Drawable for Passive {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for Passive {}

fn target(node: &FocusNode) -> AnyElement {
    Passive(node.clone()).boxed()
}

fn scope(children: Vec<AnyElement>, scope: ShortcutScope) -> AnyElement {
    ScopeElement { children, scope }.boxed()
}

fn ctrl(key: char) -> KeyChord {
    KeyChord::new(ChordMods::CTRL, Key::Char(key))
}

fn press(dispatcher: &mut EventDispatcher, root: &dyn Element, key: char, action: KeyAction) -> EventResult {
    dispatcher.dispatch(
        root,
        Vec2d::default(),
        &ElementEvent::KeyInput {
            key: NamedKey::Other(key.to_string()),
            action,
            modifiers: Modifiers { ctrl: true, ..Modifiers::default() },
        },
    )
}

fn focus(dispatcher: &mut EventDispatcher, root: &dyn Element, node: &FocusNode) {
    advance_element_tree_generation();
    node.request_focus();
    let _ = dispatcher.dispatch(root, Vec2d::default(), &ElementEvent::Cancel);
    assert!(node.has_focus());
}

fn counter() -> (Rc<Cell<u32>>, impl Fn() + 'static) {
    let count = Rc::new(Cell::new(0));
    let seen = Rc::clone(&count);
    (count, move || seen.set(seen.get() + 1))
}

#[test]
fn a_chord_bound_above_the_focused_element_runs_and_is_consumed() {
    let node = FocusNode::new();
    let (count, action) = counter();
    let root = scope(vec![target(&node)], ShortcutScope::new().bind(ctrl('k'), action));
    let mut dispatcher = EventDispatcher::new();
    focus(&mut dispatcher, root.as_ref(), &node);

    let result = press(&mut dispatcher, root.as_ref(), 'k', KeyAction::Pressed);

    assert!(result.is_consumed());
    assert_eq!(count.get(), 1);
}

#[test]
fn an_unbound_chord_is_left_for_everyone_else() {
    let node = FocusNode::new();
    let (count, action) = counter();
    let root = scope(vec![target(&node)], ShortcutScope::new().bind(ctrl('k'), action));
    let mut dispatcher = EventDispatcher::new();
    focus(&mut dispatcher, root.as_ref(), &node);

    assert!(!press(&mut dispatcher, root.as_ref(), 'j', KeyAction::Pressed).is_consumed());
    assert_eq!(count.get(), 0);
}

#[test]
fn the_innermost_scope_around_focus_wins_and_outer_scopes_fill_the_gaps() {
    let node = FocusNode::new();
    let (outer_k, outer_k_action) = counter();
    let (outer_o, outer_o_action) = counter();
    let (inner_k, inner_k_action) = counter();
    let inner = scope(vec![target(&node)], ShortcutScope::new().bind(ctrl('k'), inner_k_action));
    let root = scope(
        vec![inner],
        ShortcutScope::new().bind(ctrl('k'), outer_k_action).bind(ctrl('o'), outer_o_action),
    );
    let mut dispatcher = EventDispatcher::new();
    focus(&mut dispatcher, root.as_ref(), &node);

    press(&mut dispatcher, root.as_ref(), 'k', KeyAction::Pressed);
    press(&mut dispatcher, root.as_ref(), 'o', KeyAction::Pressed);

    assert_eq!((inner_k.get(), outer_k.get(), outer_o.get()), (1, 0, 1));
}

#[test]
fn a_scope_that_does_not_contain_focus_stays_silent_unless_it_is_window_wide() {
    let node = FocusNode::new();
    let elsewhere = FocusNode::new();
    let (local, local_action) = counter();
    let (wide, wide_action) = counter();
    let root = IdentityBranch(vec![
        target(&node),
        scope(vec![target(&elsewhere)], ShortcutScope::new().bind(ctrl('k'), local_action)),
        scope(vec![], ShortcutScope::new().window(true).bind(ctrl('w'), wide_action)),
    ])
    .boxed();
    let mut dispatcher = EventDispatcher::new();
    focus(&mut dispatcher, root.as_ref(), &node);

    assert!(!press(&mut dispatcher, root.as_ref(), 'k', KeyAction::Pressed).is_consumed());
    assert!(press(&mut dispatcher, root.as_ref(), 'w', KeyAction::Pressed).is_consumed());
    assert_eq!((local.get(), wide.get()), (0, 1));
}

#[test]
fn window_scopes_work_with_nothing_focused() {
    let (wide, wide_action) = counter();
    let root = scope(vec![], ShortcutScope::new().window(true).bind(ctrl('w'), wide_action));
    let mut dispatcher = EventDispatcher::new();
    advance_element_tree_generation();

    assert!(press(&mut dispatcher, root.as_ref(), 'w', KeyAction::Pressed).is_consumed());
    assert_eq!(wide.get(), 1);
}

#[test]
fn a_focused_element_that_consumes_the_key_keeps_it() {
    let node = FocusNode::new();
    let (count, action) = counter();
    let root = scope(vec![Greedy(node.clone()).boxed()], ShortcutScope::new().bind(ctrl('k'), action));
    let mut dispatcher = EventDispatcher::new();
    focus(&mut dispatcher, root.as_ref(), &node);

    assert!(press(&mut dispatcher, root.as_ref(), 'k', KeyAction::Pressed).is_consumed());
    assert_eq!(count.get(), 0);
}

#[test]
fn auto_repeat_is_swallowed_and_release_is_ignored() {
    let node = FocusNode::new();
    let (count, action) = counter();
    let root = scope(vec![target(&node)], ShortcutScope::new().bind(ctrl('k'), action));
    let mut dispatcher = EventDispatcher::new();
    focus(&mut dispatcher, root.as_ref(), &node);

    assert!(press(&mut dispatcher, root.as_ref(), 'k', KeyAction::Repeat).is_consumed());
    assert!(!press(&mut dispatcher, root.as_ref(), 'k', KeyAction::Released).is_consumed());
    assert_eq!(count.get(), 0);
}

#[test]
fn menu_chords_run_as_window_shortcuts_after_every_scope() {
    use aimer_shortcut::menu::{AppMenu, Menu, clear_app_menu, set_app_menu};

    let node = FocusNode::new();
    let menu_hits = Rc::new(Cell::new(0));
    let (save_hits, save_hit) = (Rc::clone(&menu_hits), Rc::clone(&menu_hits));
    let (scope_hits, scope_action) = counter();
    set_app_menu(AppMenu::new().menu(
        Menu::new("File")
            .item_with("Save", Some(ctrl('s')), move || save_hits.set(save_hits.get() + 1))
            .item_with("Open", Some(ctrl('o')), move || save_hit.set(save_hit.get() + 1)),
    ));
    let root = scope(vec![target(&node)], ShortcutScope::new().bind(ctrl('s'), scope_action));
    let mut dispatcher = EventDispatcher::new();
    focus(&mut dispatcher, root.as_ref(), &node);

    // The scope around focus binds Ctrl+S, so it beats the menu's item.
    press(&mut dispatcher, root.as_ref(), 's', KeyAction::Pressed);
    assert_eq!((scope_hits.get(), menu_hits.get()), (1, 0));

    // Ctrl+O is only the menu's.
    assert!(press(&mut dispatcher, root.as_ref(), 'o', KeyAction::Pressed).is_consumed());
    assert_eq!(menu_hits.get(), 1);
    clear_app_menu();
}
