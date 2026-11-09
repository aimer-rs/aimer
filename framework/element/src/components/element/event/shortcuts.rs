//! Offers key chords to the shortcut scopes of the tree.

use aimer_events::element::{ElementEvent, KeyAction};
use aimer_shortcut::{KeyChord, ShortcutScope, scopes_alive};
use smallvec::SmallVec;

use super::*;

type ScopeStack<'a> = SmallVec<[&'a ShortcutScope; 4]>;

/// Collects what one walk of the tree has to say about shortcuts: the scopes
/// around the focused element, and every window-wide scope.
struct ScopeWalk<'a> {
    /// The scopes of the elements between the root and where the walk is.
    path: ScopeStack<'a>,
    /// A copy of `path` taken at the focused element, outermost first.
    around_focus: Option<ScopeStack<'a>>,
    /// Window-wide scopes, in tree order.
    window: ScopeStack<'a>,
}

impl<'a> ScopeWalk<'a> {
    fn visit(&mut self, element: &'a dyn Element) {
        let scope = element.shortcut_scope();
        if let Some(scope) = scope {
            self.path.push(scope);
            if scope.is_window() {
                self.window.push(scope);
            }
        }
        if self.around_focus.is_none()
            && element.focus_node().is_some_and(|node| node.has_focus())
        {
            self.around_focus = Some(self.path.clone());
        }
        for child in structural_children(element) {
            self.visit(child);
        }
        if scope.is_some() {
            self.path.pop();
        }
    }
}

/// Runs the shortcut `event` stands for, if it stands for one.
///
/// Scopes around the focused element are tried innermost first, then the
/// window-wide ones. The first scope that binds the chord claims the key press,
/// so a binding never also reaches the element under the pointer.
///
/// Costs one thread-local read when no scope exists, and nothing is allocated
/// before a chord has been formed from the event.
pub(super) fn dispatch_shortcut(root: &dyn Element, event: &ElementEvent) -> EventResult {
    if !scopes_alive() {
        return EventResult::ignored();
    }
    let ElementEvent::KeyInput { key, action, modifiers } = event else {
        return EventResult::ignored();
    };
    let repeating = match action {
        KeyAction::Pressed => false,
        KeyAction::Repeat => true,
        KeyAction::Released => return EventResult::ignored(),
    };
    let Some(chord) = KeyChord::from_event(key, modifiers) else {
        return EventResult::ignored();
    };

    let mut walk = ScopeWalk { path: SmallVec::new(), around_focus: None, window: SmallVec::new() };
    walk.visit(root);

    let focused = walk.around_focus.iter().flat_map(|scopes| scopes.iter().rev());
    for scope in focused.chain(walk.window.iter()) {
        if scope.activate(&chord, repeating).is_claimed() {
            return EventResult::consumed();
        }
    }
    // The application menu comes last: a native menu bar has already claimed
    // its accelerators before they get here, and where there is none these are
    // the menu's shortcuts.
    if aimer_shortcut::menu::activate_chord(&chord, repeating).is_claimed() {
        return EventResult::consumed();
    }
    EventResult::ignored()
}
