//! Translates a shortcut chord into the accelerator a native macOS menu item
//! shows and claims.

use aimer_shortcut::{ChordMods, Key, KeyChord};
use muda::accelerator::{Accelerator, Code, Modifiers};

/// The accelerator for `chord`, or `None` when the key has no native code.
pub(crate) fn accelerator_for(chord: &KeyChord) -> Option<Accelerator> {
    let mut modifiers = Modifiers::empty();
    for (flag, native) in [
        (ChordMods::CTRL, Modifiers::CONTROL),
        (ChordMods::ALT, Modifiers::ALT),
        (ChordMods::SHIFT, Modifiers::SHIFT),
        (ChordMods::META, Modifiers::META),
    ] {
        if chord.mods().contains(flag) {
            modifiers |= native;
        }
    }
    Some(Accelerator::new(modifiers, code_for(chord.key())?))
}

fn code_for(key: Key) -> Option<Code> {
    Some(match key {
        Key::Char(c) => return char_code(c),
        Key::Space => Code::Space,
        Key::Enter => Code::Enter,
        Key::Escape => Code::Escape,
        Key::Tab => Code::Tab,
        Key::Backspace => Code::Backspace,
        Key::Delete => Code::Delete,
        Key::ArrowUp => Code::ArrowUp,
        Key::ArrowDown => Code::ArrowDown,
        Key::ArrowLeft => Code::ArrowLeft,
        Key::ArrowRight => Code::ArrowRight,
        Key::Home => Code::Home,
        Key::End => Code::End,
        Key::PageUp => Code::PageUp,
        Key::PageDown => Code::PageDown,
        Key::F(n) => return function_code(n),
    })
}

fn char_code(c: char) -> Option<Code> {
    Some(match c {
        'a' => Code::KeyA,
        'b' => Code::KeyB,
        'c' => Code::KeyC,
        'd' => Code::KeyD,
        'e' => Code::KeyE,
        'f' => Code::KeyF,
        'g' => Code::KeyG,
        'h' => Code::KeyH,
        'i' => Code::KeyI,
        'j' => Code::KeyJ,
        'k' => Code::KeyK,
        'l' => Code::KeyL,
        'm' => Code::KeyM,
        'n' => Code::KeyN,
        'o' => Code::KeyO,
        'p' => Code::KeyP,
        'q' => Code::KeyQ,
        'r' => Code::KeyR,
        's' => Code::KeyS,
        't' => Code::KeyT,
        'u' => Code::KeyU,
        'v' => Code::KeyV,
        'w' => Code::KeyW,
        'x' => Code::KeyX,
        'y' => Code::KeyY,
        'z' => Code::KeyZ,
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        ',' => Code::Comma,
        '.' => Code::Period,
        '/' => Code::Slash,
        ';' => Code::Semicolon,
        '\'' => Code::Quote,
        '\\' => Code::Backslash,
        '[' => Code::BracketLeft,
        ']' => Code::BracketRight,
        '-' => Code::Minus,
        '=' => Code::Equal,
        '`' => Code::Backquote,
        _ => return None,
    })
}

fn function_code(n: u8) -> Option<Code> {
    Some(match n {
        1 => Code::F1,
        2 => Code::F2,
        3 => Code::F3,
        4 => Code::F4,
        5 => Code::F5,
        6 => Code::F6,
        7 => Code::F7,
        8 => Code::F8,
        9 => Code::F9,
        10 => Code::F10,
        11 => Code::F11,
        12 => Code::F12,
        13 => Code::F13,
        14 => Code::F14,
        15 => Code::F15,
        16 => Code::F16,
        17 => Code::F17,
        18 => Code::F18,
        19 => Code::F19,
        20 => Code::F20,
        21 => Code::F21,
        22 => Code::F22,
        23 => Code::F23,
        24 => Code::F24,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> KeyChord {
        KeyChord::parse_with_primary(text, ChordMods::META).expect(text)
    }

    #[test]
    fn modifiers_and_letter_translate() {
        assert_eq!(
            accelerator_for(&chord("CmdOrCtrl+Shift+K")),
            Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, Code::KeyK))
        );
    }

    #[test]
    fn a_bare_key_has_no_modifiers() {
        assert_eq!(accelerator_for(&chord("F5")), Some(Accelerator::new(Modifiers::empty(), Code::F5)));
    }

    #[test]
    fn named_keys_and_punctuation_translate() {
        assert_eq!(accelerator_for(&chord("Ctrl+Left")), Some(Accelerator::new(Modifiers::CONTROL, Code::ArrowLeft)));
        assert_eq!(accelerator_for(&chord("Cmd+,")), Some(Accelerator::new(Modifiers::META, Code::Comma)));
        assert_eq!(accelerator_for(&chord("Alt+F24")), Some(Accelerator::new(Modifiers::ALT, Code::F24)));
    }

    #[test]
    fn a_key_with_no_native_code_gives_none() {
        // `+` cannot be a key code; the layout decides which key types it.
        assert_eq!(accelerator_for(&chord("Cmd+Plus")), None);
    }
}
