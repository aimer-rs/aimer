//! Key chords: a set of modifiers held together with one key.

use std::fmt;
use std::ops::BitOr;
use std::str::FromStr;

use aimer_events::element::{Modifiers, NamedKey};

/// The modifiers of a [`KeyChord`], as a bit set.
///
/// # Examples
///
/// ```
/// use aimer_shortcut::ChordMods;
///
/// let mods = ChordMods::CTRL | ChordMods::SHIFT;
/// assert!(mods.contains(ChordMods::SHIFT));
/// assert!(!mods.contains(ChordMods::ALT));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChordMods(u8);

impl ChordMods {
    /// No modifier held.
    pub const NONE: Self = Self(0);
    /// `Control`.
    pub const CTRL: Self = Self(1);
    /// `Alt`, which Apple keyboards label `Option`.
    pub const ALT: Self = Self(1 << 1);
    /// `Shift`.
    pub const SHIFT: Self = Self(1 << 2);
    /// The platform key: `Command` on macOS, `Windows`/`Super` elsewhere.
    pub const META: Self = Self(1 << 3);

    /// The modifier that conventional application shortcuts are built on:
    /// `Command` on macOS and `Control` everywhere else.
    #[inline]
    pub const fn primary() -> Self {
        if cfg!(target_os = "macos") { Self::META } else { Self::CTRL }
    }

    /// Whether every modifier of `other` is also held here.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no modifier is held.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Converts the modifier state a key event carries.
    #[inline]
    pub const fn from_event(modifiers: &Modifiers) -> Self {
        Self(
            modifiers.ctrl as u8
                | (modifiers.alt as u8) << 1
                | (modifiers.shift as u8) << 2
                | (modifiers.meta as u8) << 3,
        )
    }
}

impl BitOr for ChordMods {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// The key of a [`KeyChord`].
///
/// Letters are stored lowercase: whether `Shift` is held is the chord's
/// [`ChordMods`] business, never part of the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable key other than space: a letter, digit or punctuation mark.
    Char(char),
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    /// A function key, `F1` to [`Key::MAX_FUNCTION`].
    F(u8),
}

impl Key {
    /// The highest function key number a chord can name.
    pub const MAX_FUNCTION: u8 = 24;

    fn from_name(name: &str) -> Option<Self> {
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return (!c.is_control() && !c.is_whitespace() && c != '+')
                .then(|| Self::Char(c.to_lowercase().next().unwrap_or(c)));
        }
        let lower = name.to_ascii_lowercase();
        Some(match lower.as_str() {
            "plus" => Self::Char('+'),
            "space" => Self::Space,
            "enter" | "return" => Self::Enter,
            "esc" | "escape" => Self::Escape,
            "tab" => Self::Tab,
            "backspace" => Self::Backspace,
            "delete" | "del" => Self::Delete,
            "up" | "arrowup" => Self::ArrowUp,
            "down" | "arrowdown" => Self::ArrowDown,
            "left" | "arrowleft" => Self::ArrowLeft,
            "right" | "arrowright" => Self::ArrowRight,
            "home" => Self::Home,
            "end" => Self::End,
            "pageup" => Self::PageUp,
            "pagedown" => Self::PageDown,
            _ => return Self::function_key(&lower),
        })
    }

    fn function_key(lower: &str) -> Option<Self> {
        let number: u8 = lower.strip_prefix('f')?.parse().ok()?;
        (1..=Self::MAX_FUNCTION).contains(&number).then_some(Self::F(number))
    }

    fn from_named(key: &NamedKey) -> Option<Self> {
        Some(match key {
            NamedKey::Backspace => Self::Backspace,
            NamedKey::Delete => Self::Delete,
            NamedKey::ArrowUp => Self::ArrowUp,
            NamedKey::ArrowDown => Self::ArrowDown,
            NamedKey::ArrowLeft => Self::ArrowLeft,
            NamedKey::ArrowRight => Self::ArrowRight,
            NamedKey::PageUp => Self::PageUp,
            NamedKey::PageDown => Self::PageDown,
            NamedKey::Home => Self::Home,
            NamedKey::End => Self::End,
            NamedKey::Enter => Self::Enter,
            NamedKey::Escape => Self::Escape,
            NamedKey::Tab => Self::Tab,
            // The windowing layer reports a letter as one character and any
            // other named key by its `Debug` name (`F5`, `Space`, `Shift`).
            NamedKey::Other(name) => {
                let mut chars = name.chars();
                return match (chars.next(), chars.next()) {
                    (None, _) => None,
                    (Some(c), None) => (!c.is_control() && !c.is_whitespace())
                        .then(|| Self::Char(c.to_lowercase().next().unwrap_or(c))),
                    _ if name == "Space" => Some(Self::Space),
                    _ => Self::function_key(&name.to_ascii_lowercase()),
                };
            }
        })
    }
}

/// Why a chord could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseChordError {
    /// The text was empty.
    Empty,
    /// A segment was neither a modifier nor a key: carries the segment.
    UnknownKey(String),
    /// The chord names no key, only modifiers.
    MissingKey,
    /// A modifier was repeated.
    DuplicateModifier(String),
    /// A key was followed by more segments; the key must come last.
    KeyNotLast(String),
}

impl fmt::Display for ParseChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty key chord"),
            Self::UnknownKey(name) => write!(f, "unknown key or modifier `{name}`"),
            Self::MissingKey => f.write_str("key chord has modifiers but no key"),
            Self::DuplicateModifier(name) => write!(f, "modifier `{name}` appears twice"),
            Self::KeyNotLast(name) => write!(f, "key `{name}` must be the last part of a chord"),
        }
    }
}

impl std::error::Error for ParseChordError {}

/// One key pressed together with a set of modifiers: `Cmd+Shift+K`, `F5`.
///
/// # Examples
///
/// ```
/// use aimer_shortcut::{ChordMods, Key, KeyChord};
///
/// let save = KeyChord::new(ChordMods::CTRL, Key::Char('s'));
/// assert_eq!(save, "Ctrl+S".parse().unwrap());
/// assert_eq!(save.to_string(), "Ctrl+S");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    mods: ChordMods,
    key: Key,
}

impl KeyChord {
    /// Creates a chord.
    #[inline]
    pub const fn new(mods: ChordMods, key: Key) -> Self {
        Self { mods, key }
    }

    /// The modifiers that must be held.
    #[inline]
    pub const fn mods(&self) -> ChordMods {
        self.mods
    }

    /// The key that completes the chord.
    #[inline]
    pub const fn key(&self) -> Key {
        self.key
    }

    /// Parses `text` with `CmdOrCtrl` resolving to `primary`.
    ///
    /// Parts are separated by `+`, modifiers come first and the key last, and
    /// names are case-insensitive. Modifiers: `Ctrl`/`Control`, `Alt`/`Option`,
    /// `Shift`, `Cmd`/`Command`/`Meta`/`Super`/`Win`, and `CmdOrCtrl`/`Mod` for
    /// the platform's [primary](ChordMods::primary) modifier. A literal `+`
    /// key is written `Plus`.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseChordError`] for an empty chord, an unknown name, a
    /// repeated modifier, or a chord with no key or with parts after its key.
    pub fn parse_with_primary(text: &str, primary: ChordMods) -> Result<Self, ParseChordError> {
        if text.trim().is_empty() {
            return Err(ParseChordError::Empty);
        }
        let mut mods = ChordMods::NONE;
        let mut key: Option<(Key, &str)> = None;
        for part in text.split('+').map(str::trim) {
            if let Some((_, name)) = key {
                return Err(ParseChordError::KeyNotLast(name.to_owned()));
            }
            let modifier = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => Some(ChordMods::CTRL),
                "alt" | "option" | "opt" => Some(ChordMods::ALT),
                "shift" => Some(ChordMods::SHIFT),
                "cmd" | "command" | "meta" | "super" | "win" => Some(ChordMods::META),
                "cmdorctrl" | "commandorcontrol" | "mod" => Some(primary),
                _ => None,
            };
            match modifier {
                Some(modifier) => {
                    if mods.contains(modifier) {
                        return Err(ParseChordError::DuplicateModifier(part.to_owned()));
                    }
                    mods = mods | modifier;
                }
                None => {
                    let parsed = Key::from_name(part)
                        .ok_or_else(|| ParseChordError::UnknownKey(part.to_owned()))?;
                    key = Some((parsed, part));
                }
            }
        }
        key.map(|(key, _)| Self { mods, key }).ok_or(ParseChordError::MissingKey)
    }

    /// Parses `text`, resolving `CmdOrCtrl` for the platform this runs on.
    ///
    /// # Errors
    ///
    /// See [`KeyChord::parse_with_primary`].
    #[inline]
    pub fn parse(text: &str) -> Result<Self, ParseChordError> {
        Self::parse_with_primary(text, ChordMods::primary())
    }

    /// The chord a key event stands for, or `None` for a key that cannot be
    /// part of a shortcut (a bare modifier, an unnamed key).
    pub fn from_event(key: &NamedKey, modifiers: &Modifiers) -> Option<Self> {
        Some(Self { mods: ChordMods::from_event(modifiers), key: Key::from_named(key)? })
    }

    /// The chord as macOS menus write it: `⇧⌘K`.
    pub fn label_macos(&self) -> String {
        let mut label = String::new();
        for (flag, symbol) in [
            (ChordMods::CTRL, '⌃'),
            (ChordMods::ALT, '⌥'),
            (ChordMods::SHIFT, '⇧'),
            (ChordMods::META, '⌘'),
        ] {
            if self.mods.contains(flag) {
                label.push(symbol);
            }
        }
        match self.key {
            Key::Char(c) => label.extend(c.to_uppercase()),
            Key::ArrowUp => label.push('↑'),
            Key::ArrowDown => label.push('↓'),
            Key::ArrowLeft => label.push('←'),
            Key::ArrowRight => label.push('→'),
            Key::Enter => label.push('↩'),
            Key::Escape => label.push('⎋'),
            Key::Tab => label.push('⇥'),
            Key::Backspace => label.push('⌫'),
            Key::Delete => label.push('⌦'),
            other => label.push_str(&other.to_string()),
        }
        label
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Char('+') => f.write_str("Plus"),
            Self::Char(c) => write!(f, "{}", c.to_uppercase()),
            Self::Space => f.write_str("Space"),
            Self::Enter => f.write_str("Enter"),
            Self::Escape => f.write_str("Escape"),
            Self::Tab => f.write_str("Tab"),
            Self::Backspace => f.write_str("Backspace"),
            Self::Delete => f.write_str("Delete"),
            Self::ArrowUp => f.write_str("Up"),
            Self::ArrowDown => f.write_str("Down"),
            Self::ArrowLeft => f.write_str("Left"),
            Self::ArrowRight => f.write_str("Right"),
            Self::Home => f.write_str("Home"),
            Self::End => f.write_str("End"),
            Self::PageUp => f.write_str("PageUp"),
            Self::PageDown => f.write_str("PageDown"),
            Self::F(n) => write!(f, "F{n}"),
        }
    }
}

/// Writes the canonical, platform-neutral form: `Ctrl+Alt+Shift+Meta+Key`.
/// It parses back to the same chord.
impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (flag, name) in [
            (ChordMods::CTRL, "Ctrl"),
            (ChordMods::ALT, "Alt"),
            (ChordMods::SHIFT, "Shift"),
            (ChordMods::META, "Meta"),
        ] {
            if self.mods.contains(flag) {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

impl FromStr for KeyChord {
    type Err = ParseChordError;

    #[inline]
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

#[cfg(test)]
#[path = "chord_tests.rs"]
mod chord_tests;
