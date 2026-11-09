use aimer_events::element::{Modifiers, NamedKey};

use super::*;

const CTRL: ChordMods = ChordMods::CTRL;
const SHIFT: ChordMods = ChordMods::SHIFT;
const ALT: ChordMods = ChordMods::ALT;
const META: ChordMods = ChordMods::META;

fn parse(text: &str) -> KeyChord {
    KeyChord::parse_with_primary(text, META).expect(text)
}

#[test]
fn parses_modifiers_and_a_letter() {
    assert_eq!(
        parse("Cmd+Shift+K"),
        KeyChord::new(META | SHIFT, Key::Char('k'))
    );
}

#[test]
fn modifier_and_key_names_are_case_insensitive_and_aliased() {
    let expected = KeyChord::new(CTRL | ALT, Key::Delete);
    assert_eq!(parse("ctrl+option+del"), expected);
    assert_eq!(parse("CONTROL+Alt+Delete"), expected);
    assert_eq!(parse(" Ctrl + Alt + Delete "), expected);
}

#[test]
fn cmd_or_ctrl_follows_the_primary_modifier_of_the_platform() {
    assert_eq!(
        KeyChord::parse_with_primary("CmdOrCtrl+S", META).unwrap(),
        KeyChord::new(META, Key::Char('s'))
    );
    assert_eq!(
        KeyChord::parse_with_primary("CmdOrCtrl+S", CTRL).unwrap(),
        KeyChord::new(CTRL, Key::Char('s'))
    );
}

#[test]
fn parses_named_keys_function_keys_and_punctuation() {
    assert_eq!(parse("Escape"), KeyChord::new(ChordMods::NONE, Key::Escape));
    assert_eq!(parse("Cmd+,"), KeyChord::new(META, Key::Char(',')));
    assert_eq!(parse("Cmd+Plus"), KeyChord::new(META, Key::Char('+')));
    assert_eq!(parse("Alt+Left"), KeyChord::new(ALT, Key::ArrowLeft));
    assert_eq!(parse("F12"), KeyChord::new(ChordMods::NONE, Key::F(12)));
    assert_eq!(parse("Ctrl+Space"), KeyChord::new(CTRL, Key::Space));
}

#[test]
fn rejects_malformed_chords() {
    for bad in [
        "", "+", "Cmd+", "Cmd+Shift", "Cmd+K+L", "Cmd+Cmd+K", "Bogus+K", "F0", "F25", "Cmd+Nope",
        "K+Cmd",
    ] {
        assert!(
            KeyChord::parse_with_primary(bad, META).is_err(),
            "{bad:?} should not parse"
        );
    }
}

#[test]
fn display_round_trips_through_parse() {
    for text in ["Ctrl+Shift+K", "Alt+F4", "Meta+Ctrl+Alt+Shift+Left", "Space", "Ctrl+,"] {
        let chord = parse(text);
        assert_eq!(parse(&chord.to_string()), chord, "{text}");
    }
    // Canonical order is Ctrl, Alt, Shift, Meta whatever order was typed.
    assert_eq!(parse("meta+shift+k").to_string(), "Shift+Meta+K");
}

#[test]
fn macos_label_uses_symbols_in_apple_order() {
    assert_eq!(parse("Cmd+Shift+K").label_macos(), "⇧⌘K");
    assert_eq!(parse("Ctrl+Alt+Left").label_macos(), "⌃⌥←");
}

#[test]
fn a_key_event_becomes_the_chord_it_represents() {
    let mods = Modifiers { ctrl: false, shift: true, alt: false, meta: true };
    assert_eq!(
        KeyChord::from_event(&NamedKey::Other("k".into()), &mods),
        Some(KeyChord::new(META | SHIFT, Key::Char('k')))
    );
    assert_eq!(
        KeyChord::from_event(&NamedKey::Other("K".into()), &Modifiers::default()),
        Some(KeyChord::new(ChordMods::NONE, Key::Char('k')))
    );
    assert_eq!(
        KeyChord::from_event(&NamedKey::Other("F5".into()), &Modifiers::default()),
        Some(KeyChord::new(ChordMods::NONE, Key::F(5)))
    );
    assert_eq!(
        KeyChord::from_event(&NamedKey::ArrowUp, &Modifiers::default()),
        Some(KeyChord::new(ChordMods::NONE, Key::ArrowUp))
    );
}

#[test]
fn keys_that_are_not_shortcut_material_yield_no_chord() {
    let none = Modifiers::default();
    assert_eq!(KeyChord::from_event(&NamedKey::Other("Shift".into()), &none), None);
    assert_eq!(KeyChord::from_event(&NamedKey::Other("".into()), &none), None);
    assert_eq!(KeyChord::from_event(&NamedKey::Other("F0".into()), &none), None);
}
