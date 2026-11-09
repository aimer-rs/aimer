//! The key and modifier codes each operating system's hotkey API speaks.
//!
//! Plain tables with no FFI in them, so every platform's table is compiled and
//! tested everywhere. Hotkeys are registered by *key position*, as the
//! operating systems define them, and not by the character a layout types.

// Every platform's table is compiled everywhere so each can be tested here, but
// a build uses only its own.
#![allow(dead_code)]

use crate::{ChordMods, Key};

/// macOS virtual key code (`kVK_*`) for `key`.
pub(crate) fn carbon_key(key: Key) -> Option<u32> {
    Some(match key {
        Key::Char(c) => match c {
            'a' => 0x00, 's' => 0x01, 'd' => 0x02, 'f' => 0x03, 'h' => 0x04, 'g' => 0x05,
            'z' => 0x06, 'x' => 0x07, 'c' => 0x08, 'v' => 0x09, 'b' => 0x0B, 'q' => 0x0C,
            'w' => 0x0D, 'e' => 0x0E, 'r' => 0x0F, 'y' => 0x10, 't' => 0x11,
            '1' => 0x12, '2' => 0x13, '3' => 0x14, '4' => 0x15, '6' => 0x16, '5' => 0x17,
            '=' => 0x18, '9' => 0x19, '7' => 0x1A, '-' => 0x1B, '8' => 0x1C, '0' => 0x1D,
            ']' => 0x1E, 'o' => 0x1F, 'u' => 0x20, '[' => 0x21, 'i' => 0x22, 'p' => 0x23,
            'l' => 0x25, 'j' => 0x26, '\'' => 0x27, 'k' => 0x28, ';' => 0x29, '\\' => 0x2A,
            ',' => 0x2B, '/' => 0x2C, 'n' => 0x2D, 'm' => 0x2E, '.' => 0x2F, '`' => 0x32,
            _ => return None,
        },
        Key::Enter => 0x24,
        Key::Tab => 0x30,
        Key::Space => 0x31,
        Key::Backspace => 0x33,
        Key::Escape => 0x35,
        Key::Delete => 0x75,
        Key::Home => 0x73,
        Key::End => 0x77,
        Key::PageUp => 0x74,
        Key::PageDown => 0x79,
        Key::ArrowLeft => 0x7B,
        Key::ArrowRight => 0x7C,
        Key::ArrowDown => 0x7D,
        Key::ArrowUp => 0x7E,
        Key::F(n) => match n {
            1 => 0x7A, 2 => 0x78, 3 => 0x63, 4 => 0x76, 5 => 0x60, 6 => 0x61, 7 => 0x62,
            8 => 0x64, 9 => 0x65, 10 => 0x6D, 11 => 0x67, 12 => 0x6F, 13 => 0x69,
            14 => 0x6B, 15 => 0x71, 16 => 0x6A, 17 => 0x40, 18 => 0x4F, 19 => 0x50,
            20 => 0x5A,
            _ => return None,
        },
    })
}

/// Carbon modifier mask (`cmdKey`, `shiftKey`, `optionKey`, `controlKey`).
pub(crate) fn carbon_mods(mods: ChordMods) -> u32 {
    let mut mask = 0;
    for (flag, bit) in [
        (ChordMods::META, 0x100),
        (ChordMods::SHIFT, 0x200),
        (ChordMods::ALT, 0x800),
        (ChordMods::CTRL, 0x1000),
    ] {
        if mods.contains(flag) {
            mask |= bit;
        }
    }
    mask
}

/// Windows virtual-key code (`VK_*`) for `key`.
pub(crate) fn windows_key(key: Key) -> Option<u32> {
    Some(match key {
        Key::Char(c) => match c {
            'a'..='z' => 0x41 + (c as u32 - 'a' as u32),
            '0'..='9' => 0x30 + (c as u32 - '0' as u32),
            ';' => 0xBA, '=' => 0xBB, ',' => 0xBC, '-' => 0xBD, '.' => 0xBE, '/' => 0xBF,
            '`' => 0xC0, '[' => 0xDB, '\\' => 0xDC, ']' => 0xDD, '\'' => 0xDE,
            _ => return None,
        },
        Key::Backspace => 0x08,
        Key::Tab => 0x09,
        Key::Enter => 0x0D,
        Key::Escape => 0x1B,
        Key::Space => 0x20,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::End => 0x23,
        Key::Home => 0x24,
        Key::ArrowLeft => 0x25,
        Key::ArrowUp => 0x26,
        Key::ArrowRight => 0x27,
        Key::ArrowDown => 0x28,
        Key::Delete => 0x2E,
        Key::F(n) => 0x6F + u32::from(n),
    })
}

/// `RegisterHotKey` modifier flags (`MOD_ALT`, `MOD_CONTROL`, `MOD_SHIFT`, `MOD_WIN`).
pub(crate) fn windows_mods(mods: ChordMods) -> u32 {
    let mut mask = 0;
    for (flag, bit) in [
        (ChordMods::ALT, 0x1),
        (ChordMods::CTRL, 0x2),
        (ChordMods::SHIFT, 0x4),
        (ChordMods::META, 0x8),
    ] {
        if mods.contains(flag) {
            mask |= bit;
        }
    }
    mask
}

/// X11 keysym for `key`.
pub(crate) fn x11_keysym(key: Key) -> Option<u32> {
    Some(match key {
        // Latin-1 keysyms equal their ASCII code, lowercase for letters.
        Key::Char(c) if c.is_ascii_graphic() => c as u32,
        Key::Char(_) => return None,
        Key::Space => 0x20,
        Key::Backspace => 0xFF08,
        Key::Tab => 0xFF09,
        Key::Enter => 0xFF0D,
        Key::Escape => 0xFF1B,
        Key::Home => 0xFF50,
        Key::ArrowLeft => 0xFF51,
        Key::ArrowUp => 0xFF52,
        Key::ArrowRight => 0xFF53,
        Key::ArrowDown => 0xFF54,
        Key::PageUp => 0xFF55,
        Key::PageDown => 0xFF56,
        Key::End => 0xFF57,
        Key::Delete => 0xFFFF,
        Key::F(n) => 0xFFBD + u32::from(n),
    })
}

/// X11 modifier mask (`ShiftMask`, `ControlMask`, `Mod1Mask`, `Mod4Mask`).
pub(crate) fn x11_mods(mods: ChordMods) -> u32 {
    let mut mask = 0;
    for (flag, bit) in [
        (ChordMods::SHIFT, 0x1),
        (ChordMods::CTRL, 0x4),
        (ChordMods::ALT, 0x8),
        (ChordMods::META, 0x40),
    ] {
        if mods.contains(flag) {
            mask |= bit;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carbon_codes_match_the_apple_headers() {
        assert_eq!(carbon_key(Key::Char('a')), Some(0x00));
        assert_eq!(carbon_key(Key::Char('k')), Some(0x28));
        assert_eq!(carbon_key(Key::Space), Some(0x31));
        assert_eq!(carbon_key(Key::F(5)), Some(0x60));
        assert_eq!(carbon_key(Key::F(21)), None);
        assert_eq!(carbon_key(Key::Char('+')), None);
        assert_eq!(carbon_mods(ChordMods::META | ChordMods::SHIFT), 0x300);
        assert_eq!(carbon_mods(ChordMods::CTRL | ChordMods::ALT), 0x1800);
    }

    #[test]
    fn every_letter_and_digit_has_a_distinct_carbon_code() {
        let mut seen = std::collections::HashSet::new();
        for c in ('a'..='z').chain('0'..='9') {
            let code = carbon_key(Key::Char(c)).unwrap_or_else(|| panic!("{c}"));
            assert!(seen.insert(code), "{c} repeats code {code:#x}");
        }
    }

    #[test]
    fn windows_codes_match_the_sdk() {
        assert_eq!(windows_key(Key::Char('a')), Some(0x41));
        assert_eq!(windows_key(Key::Char('z')), Some(0x5A));
        assert_eq!(windows_key(Key::Char('0')), Some(0x30));
        assert_eq!(windows_key(Key::Char(',')), Some(0xBC));
        assert_eq!(windows_key(Key::F(1)), Some(0x70));
        assert_eq!(windows_key(Key::F(24)), Some(0x87));
        assert_eq!(windows_key(Key::ArrowLeft), Some(0x25));
        assert_eq!(windows_key(Key::Char('+')), None);
        assert_eq!(windows_mods(ChordMods::CTRL | ChordMods::SHIFT), 0x6);
        assert_eq!(windows_mods(ChordMods::META | ChordMods::ALT), 0x9);
    }

    #[test]
    fn x11_keysyms_match_keysymdef() {
        assert_eq!(x11_keysym(Key::Char('a')), Some(0x61));
        assert_eq!(x11_keysym(Key::Char('1')), Some(0x31));
        assert_eq!(x11_keysym(Key::F(1)), Some(0xFFBE));
        assert_eq!(x11_keysym(Key::F(12)), Some(0xFFC9));
        assert_eq!(x11_keysym(Key::Delete), Some(0xFFFF));
        assert_eq!(x11_keysym(Key::Char('é')), None);
        assert_eq!(x11_mods(ChordMods::CTRL | ChordMods::ALT | ChordMods::META), 0x4 | 0x8 | 0x40);
    }
}
