//! Turns physical keys into the key names shortcut chords are written with.

use winit::keyboard::{KeyCode, PhysicalKey};

/// The name a chord uses for a printable physical key, or `None` for a key a
/// chord is not written with here (named keys such as arrows and function keys
/// travel their own path).
///
/// The *physical* key is used, as the editing shortcuts already do, so a
/// shortcut stays on the same key under any keyboard layout and never depends
/// on what a held `Alt` or `Shift` would have typed.
pub(crate) fn chord_key_name(key: PhysicalKey) -> Option<&'static str> {
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    Some(match code {
        KeyCode::KeyA => "a",
        KeyCode::KeyB => "b",
        KeyCode::KeyC => "c",
        KeyCode::KeyD => "d",
        KeyCode::KeyE => "e",
        KeyCode::KeyF => "f",
        KeyCode::KeyG => "g",
        KeyCode::KeyH => "h",
        KeyCode::KeyI => "i",
        KeyCode::KeyJ => "j",
        KeyCode::KeyK => "k",
        KeyCode::KeyL => "l",
        KeyCode::KeyM => "m",
        KeyCode::KeyN => "n",
        KeyCode::KeyO => "o",
        KeyCode::KeyP => "p",
        KeyCode::KeyQ => "q",
        KeyCode::KeyR => "r",
        KeyCode::KeyS => "s",
        KeyCode::KeyT => "t",
        KeyCode::KeyU => "u",
        KeyCode::KeyV => "v",
        KeyCode::KeyW => "w",
        KeyCode::KeyX => "x",
        KeyCode::KeyY => "y",
        KeyCode::KeyZ => "z",
        KeyCode::Digit0 => "0",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        KeyCode::Digit5 => "5",
        KeyCode::Digit6 => "6",
        KeyCode::Digit7 => "7",
        KeyCode::Digit8 => "8",
        KeyCode::Digit9 => "9",
        KeyCode::Comma => ",",
        KeyCode::Period => ".",
        KeyCode::Slash => "/",
        KeyCode::Semicolon => ";",
        KeyCode::Quote => "'",
        KeyCode::Backslash => "\\",
        KeyCode::BracketLeft => "[",
        KeyCode::BracketRight => "]",
        KeyCode::Minus => "-",
        KeyCode::Equal => "=",
        KeyCode::Backquote => "`",
        KeyCode::Space => "Space",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use aimer_shortcut::{ChordMods, Key, KeyChord};
    use aimer_events::element::{Modifiers, NamedKey};
    use winit::keyboard::NativeKeyCode;

    use super::*;

    #[test]
    fn letters_digits_and_punctuation_have_names() {
        assert_eq!(chord_key_name(PhysicalKey::Code(KeyCode::KeyK)), Some("k"));
        assert_eq!(chord_key_name(PhysicalKey::Code(KeyCode::Digit7)), Some("7"));
        assert_eq!(chord_key_name(PhysicalKey::Code(KeyCode::Comma)), Some(","));
    }

    #[test]
    fn keys_without_a_chord_name_have_none() {
        assert_eq!(chord_key_name(PhysicalKey::Code(KeyCode::F5)), None);
        assert_eq!(chord_key_name(PhysicalKey::Code(KeyCode::ShiftLeft)), None);
        assert_eq!(chord_key_name(PhysicalKey::Unidentified(NativeKeyCode::Unidentified)), None);
    }

    #[test]
    fn every_name_becomes_a_chord_key() {
        for code in [KeyCode::KeyA, KeyCode::Digit0, KeyCode::Slash, KeyCode::Backquote, KeyCode::Space] {
            let name = chord_key_name(PhysicalKey::Code(code)).expect("named");
            let chord = KeyChord::from_event(
                &NamedKey::Other(name.to_owned()),
                &Modifiers { ctrl: true, ..Modifiers::default() },
            )
            .unwrap_or_else(|| panic!("{name:?} must form a chord"));
            assert_eq!(chord.mods(), ChordMods::CTRL);
            assert!(matches!(chord.key(), Key::Char(_) | Key::Space));
        }
    }
}
