use super::*;
use crate::ChordMods;

fn chord(text: &str) -> KeyChord {
    KeyChord::parse_with_primary(text, ChordMods::CTRL).expect(text)
}

#[test]
fn a_bound_chord_resolves_to_its_action() {
    let mut map = Keymap::new();
    assert_eq!(map.bind(chord("Ctrl+S"), "save"), None);
    assert_eq!(map.get(&chord("ctrl+s")), Some(&"save"));
    assert_eq!(map.get(&chord("Ctrl+Shift+S")), None);
    assert_eq!(map.len(), 1);
}

#[test]
fn rebinding_a_chord_returns_the_action_it_displaced() {
    let mut map = Keymap::new();
    map.bind(chord("Ctrl+S"), 1);
    assert_eq!(map.bind(chord("Ctrl+S"), 2), Some(1));
    assert_eq!(map.get(&chord("Ctrl+S")), Some(&2));
    assert_eq!(map.len(), 1);
}

#[test]
fn unbinding_removes_only_that_chord() {
    let mut map = Keymap::new();
    map.bind(chord("Ctrl+S"), 1);
    map.bind(chord("Ctrl+O"), 2);
    assert_eq!(map.unbind(&chord("Ctrl+S")), Some(1));
    assert_eq!(map.unbind(&chord("Ctrl+S")), None);
    assert_eq!(map.get(&chord("Ctrl+O")), Some(&2));
    assert!(!map.is_empty());
}

#[test]
fn the_innermost_scope_wins_and_outer_scopes_fill_the_gaps() {
    let mut window = Keymap::new();
    window.bind(chord("Ctrl+S"), "window save");
    window.bind(chord("Ctrl+W"), "window close");
    let mut editor = Keymap::new();
    editor.bind(chord("Ctrl+S"), "editor save");

    // Innermost first, the order a focus path is walked in.
    let chain = [&editor, &window];
    assert_eq!(resolve(chain, &chord("Ctrl+S")), Some(&"editor save"));
    assert_eq!(resolve(chain, &chord("Ctrl+W")), Some(&"window close"));
    assert_eq!(resolve(chain, &chord("Ctrl+Q")), None);
}

#[test]
fn an_empty_chain_resolves_nothing() {
    assert_eq!(resolve(std::iter::empty::<&Keymap<u8>>(), &chord("Ctrl+S")), None);
}
