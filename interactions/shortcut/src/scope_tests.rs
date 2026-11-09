use std::cell::Cell;
use std::rc::Rc;

use super::*;

fn chord(text: &str) -> KeyChord {
    KeyChord::parse_with_primary(text, crate::ChordMods::CTRL).expect(text)
}

fn counter() -> (Rc<Cell<u32>>, impl Fn() + 'static) {
    let count = Rc::new(Cell::new(0));
    let seen = Rc::clone(&count);
    (count, move || seen.set(seen.get() + 1))
}

#[test]
fn a_bound_chord_runs_its_action_once_per_press() {
    let (count, action) = counter();
    let scope = ShortcutScope::new().bind(chord("Ctrl+S"), action);

    assert_eq!(scope.activate(&chord("Ctrl+S"), false), Activation::Ran);
    assert_eq!(count.get(), 1);
    assert_eq!(scope.activate(&chord("Ctrl+O"), false), Activation::Unbound);
    assert_eq!(count.get(), 1);
}

#[test]
fn auto_repeat_is_swallowed_unless_the_scope_repeats() {
    let (count, action) = counter();
    let scope = ShortcutScope::new().bind(chord("Ctrl+S"), action);
    assert_eq!(scope.activate(&chord("Ctrl+S"), true), Activation::Held);
    assert_eq!(count.get(), 0);

    let (count, action) = counter();
    let scope = ShortcutScope::new().repeat(true).bind(chord("Ctrl+S"), action);
    assert_eq!(scope.activate(&chord("Ctrl+S"), true), Activation::Ran);
    assert_eq!(count.get(), 1);
}

#[test]
fn binding_a_chord_twice_keeps_the_last_action() {
    let (first, a) = counter();
    let (second, b) = counter();
    let scope = ShortcutScope::new().bind(chord("Ctrl+S"), a).bind(chord("Ctrl+S"), b);
    scope.activate(&chord("Ctrl+S"), false);
    assert_eq!((first.get(), second.get()), (0, 1));
}

#[test]
fn window_scopes_are_flagged() {
    assert!(!ShortcutScope::new().is_window());
    assert!(ShortcutScope::new().window(true).is_window());
}

#[test]
fn live_scopes_are_counted_so_unused_apps_pay_nothing() {
    assert!(!scopes_alive());
    let scope = ShortcutScope::new();
    assert!(scopes_alive());
    let other = ShortcutScope::new();
    drop(scope);
    assert!(scopes_alive());
    drop(other);
    assert!(!scopes_alive());
}
