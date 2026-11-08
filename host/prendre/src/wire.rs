//! The text wire format a [`Command`] travels in: one command per line.
//!
//! ```text
//! move X Y                          down X Y [BUTTON]      up X Y [BUTTON]
//! leave                             scroll DX DY
//! key press|release|repeat MODS KEY text TEXT
//! ```
//!
//! `BUTTON` is `primary` (the default), `secondary`, `middle` or `other:N`.
//! `MODS` is `-` or `+`-joined `ctrl`, `shift`, `alt`, `meta`. `KEY` is one of
//! the [`NamedKey`] names in lowercase (`pageup`, `enter`, ...) or `other:NAME`.
//! `TEXT` and `NAME` run to the end of the line, so they may hold spaces; `\`,
//! newline and carriage return are written `\\`, `\n` and `\r`.
//!
//! Text keeps the protocol greppable and lets a shell script or `nc` drive an
//! app without a client library, and an input event is rare enough that
//! parsing a line costs nothing next to the frame it triggers.

use std::fmt::{self, Write as _};
use std::str::FromStr;

use aimer_events::element::{KeyAction, Modifiers, NamedKey};
use aimer_events::pointer::PointerButton;

use crate::Command;

/// Why a line could not be read as a [`Command`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The line held nothing but whitespace.
    Empty,
    /// The first word is not a command.
    UnknownCommand,
    /// A required argument, named here, is absent.
    Missing(&'static str),
    /// The named argument is present but unreadable, or not a finite number.
    Invalid(&'static str),
    /// The command takes fewer arguments than the line carries.
    TooMany,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty command"),
            Self::UnknownCommand => f.write_str("unknown command"),
            Self::Missing(what) => write!(f, "missing {what}"),
            Self::Invalid(what) => write!(f, "invalid {what}"),
            Self::TooMany => f.write_str("too many arguments"),
        }
    }
}

impl std::error::Error for ParseError {}

const KEYS: [(&str, NamedKey); 12] = [
    ("backspace", NamedKey::Backspace),
    ("delete", NamedKey::Delete),
    ("up", NamedKey::ArrowUp),
    ("down", NamedKey::ArrowDown),
    ("left", NamedKey::ArrowLeft),
    ("right", NamedKey::ArrowRight),
    ("pageup", NamedKey::PageUp),
    ("pagedown", NamedKey::PageDown),
    ("home", NamedKey::Home),
    ("end", NamedKey::End),
    ("enter", NamedKey::Enter),
    ("escape", NamedKey::Escape),
];

fn escape(text: &str, out: &mut impl fmt::Write) -> fmt::Result {
    for ch in text.chars() {
        match ch {
            '\\' => out.write_str("\\\\")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            ch => out.write_char(ch)?,
        }
    }
    Ok(())
}

fn unescape(text: &str, what: &'static str) -> Result<String, ParseError> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            _ => return Err(ParseError::Invalid(what)),
        }
    }
    Ok(out)
}

fn button_name(button: PointerButton, out: &mut fmt::Formatter<'_>) -> fmt::Result {
    match button {
        PointerButton::Primary => out.write_str("primary"),
        PointerButton::Secondary => out.write_str("secondary"),
        PointerButton::Middle => out.write_str("middle"),
        PointerButton::Other(code) => write!(out, "other:{code}"),
    }
}

fn parse_button(word: &str) -> Result<PointerButton, ParseError> {
    match word {
        "primary" => Ok(PointerButton::Primary),
        "secondary" => Ok(PointerButton::Secondary),
        "middle" => Ok(PointerButton::Middle),
        other => other
            .strip_prefix("other:")
            .and_then(|code| code.parse().ok())
            .map(PointerButton::Other)
            .ok_or(ParseError::Invalid("button")),
    }
}

fn parse_modifiers(word: &str) -> Result<Modifiers, ParseError> {
    let mut modifiers = Modifiers::default();
    if word == "-" {
        return Ok(modifiers);
    }
    for part in word.split('+') {
        match part {
            "ctrl" => modifiers.ctrl = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            "meta" => modifiers.meta = true,
            _ => return Err(ParseError::Invalid("modifiers")),
        }
    }
    Ok(modifiers)
}

fn parse_key(word: &str) -> Result<NamedKey, ParseError> {
    if let Some(name) = word.strip_prefix("other:") {
        let name = unescape(name, "key")?;
        return if name.is_empty() { Err(ParseError::Invalid("key")) } else { Ok(NamedKey::Other(name)) };
    }
    if word == "tab" {
        return Ok(NamedKey::Tab);
    }
    KEYS.iter()
        .find(|(name, _)| *name == word)
        .map(|(_, key)| key.clone())
        .ok_or(ParseError::Invalid("key"))
}

/// A finite `f32` argument; `NaN` and infinities would poison hit testing.
fn number(word: Option<&str>, what: &'static str) -> Result<f32, ParseError> {
    let value: f32 = word.ok_or(ParseError::Missing(what))?.parse().map_err(|_| ParseError::Invalid(what))?;
    if value.is_finite() { Ok(value) } else { Err(ParseError::Invalid(what)) }
}

fn no_more(mut words: std::str::SplitAsciiWhitespace<'_>) -> Result<(), ParseError> {
    if words.next().is_some() { Err(ParseError::TooMany) } else { Ok(()) }
}

impl fmt::Display for Command {
    /// Writes the command as one wire line without the trailing newline.
    ///
    /// A command holding a non-finite float is not representable: its line is
    /// rejected by [`Command::from_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PointerMove { x, y } => write!(f, "move {x} {y}"),
            Self::PointerDown { x, y, button } => {
                write!(f, "down {x} {y} ")?;
                button_name(*button, f)
            }
            Self::PointerUp { x, y, button } => {
                write!(f, "up {x} {y} ")?;
                button_name(*button, f)
            }
            Self::PointerLeave => f.write_str("leave"),
            Self::Scroll { dx, dy } => write!(f, "scroll {dx} {dy}"),
            Self::Key { key, action, modifiers } => {
                f.write_str(match action {
                    KeyAction::Pressed => "key press ",
                    KeyAction::Released => "key release ",
                    KeyAction::Repeat => "key repeat ",
                })?;
                let flags = [
                    (modifiers.ctrl, "ctrl"),
                    (modifiers.shift, "shift"),
                    (modifiers.alt, "alt"),
                    (modifiers.meta, "meta"),
                ];
                let mut any = false;
                for (_, name) in flags.iter().filter(|(on, _)| *on) {
                    if any {
                        f.write_char('+')?;
                    }
                    f.write_str(name)?;
                    any = true;
                }
                if !any {
                    f.write_char('-')?;
                }
                f.write_char(' ')?;
                match key {
                    NamedKey::Tab => f.write_str("tab"),
                    NamedKey::Other(name) => {
                        f.write_str("other:")?;
                        escape(name, f)
                    }
                    named => {
                        let name = KEYS.iter().find(|(_, k)| k == named).map(|(n, _)| *n);
                        f.write_str(name.expect("every non-Other, non-Tab key is in KEYS"))
                    }
                }
            }
            Self::Text(text) => {
                f.write_str("text ")?;
                escape(text, f)
            }
        }
    }
}

impl FromStr for Command {
    type Err = ParseError;

    /// Reads one wire line, without its trailing newline.
    fn from_str(line: &str) -> Result<Self, ParseError> {
        let (verb, rest) = match line.split_once(' ') {
            Some((verb, rest)) => (verb, Some(rest)),
            None => (line, None),
        };
        if verb.is_empty() && rest.is_none_or(|r| r.trim().is_empty()) {
            return Err(ParseError::Empty);
        }
        let mut words = rest.unwrap_or("").split_ascii_whitespace();
        match verb {
            "move" => {
                let (x, y) = (number(words.next(), "x")?, number(words.next(), "y")?);
                no_more(words)?;
                Ok(Self::PointerMove { x, y })
            }
            "down" | "up" => {
                let (x, y) = (number(words.next(), "x")?, number(words.next(), "y")?);
                let button = words.next().map_or(Ok(PointerButton::Primary), parse_button)?;
                no_more(words)?;
                Ok(if verb == "down" {
                    Self::PointerDown { x, y, button }
                } else {
                    Self::PointerUp { x, y, button }
                })
            }
            "leave" => no_more(words).map(|()| Self::PointerLeave),
            "scroll" => {
                let (dx, dy) = (number(words.next(), "dx")?, number(words.next(), "dy")?);
                no_more(words)?;
                Ok(Self::Scroll { dx, dy })
            }
            "key" => {
                // The key name runs to the end of the line, so split by hand.
                let mut parts = rest.unwrap_or("").splitn(3, ' ');
                let action = match parts.next().filter(|w| !w.is_empty()) {
                    Some("press") => KeyAction::Pressed,
                    Some("release") => KeyAction::Released,
                    Some("repeat") => KeyAction::Repeat,
                    Some(_) => return Err(ParseError::Invalid("action")),
                    None => return Err(ParseError::Missing("action")),
                };
                let modifiers = parse_modifiers(parts.next().ok_or(ParseError::Missing("modifiers"))?)?;
                let key = parse_key(parts.next().ok_or(ParseError::Missing("key"))?)?;
                Ok(Self::Key { key, action, modifiers })
            }
            "text" => Ok(Self::Text(unescape(rest.ok_or(ParseError::Missing("text"))?, "escape")?)),
            _ => Err(ParseError::UnknownCommand),
        }
    }
}

#[cfg(test)]
mod tests {
    use aimer_events::element::{KeyAction, Modifiers, NamedKey};
    use aimer_events::pointer::PointerButton;

    use super::*;

    fn roundtrip(command: Command) {
        let line = command.to_string();
        assert!(!line.contains('\n') && !line.contains('\r'), "one command is one line: {line:?}");
        assert_eq!(line.parse::<Command>(), Ok(command), "line was {line:?}");
    }

    #[test]
    fn every_command_survives_a_round_trip() {
        roundtrip(Command::PointerMove { x: 1.5, y: -2.25 });
        roundtrip(Command::PointerDown { x: 10.0, y: 20.0, button: PointerButton::Secondary });
        roundtrip(Command::PointerUp { x: 0.1, y: 0.2, button: PointerButton::Other(7) });
        roundtrip(Command::PointerUp { x: 0.0, y: 0.0, button: PointerButton::Middle });
        roundtrip(Command::PointerLeave);
        roundtrip(Command::Scroll { dx: 0.0, dy: -120.5 });
        roundtrip(Command::Key {
            key: NamedKey::PageDown,
            action: KeyAction::Repeat,
            modifiers: Modifiers { ctrl: true, shift: false, alt: true, meta: false },
        });
        roundtrip(Command::Key {
            key: NamedKey::Other("F5 key\\x".into()),
            action: KeyAction::Released,
            modifiers: Modifiers::default(),
        });
        roundtrip(Command::Text(String::new()));
        roundtrip(Command::Text("héllo 世界\nsecond\r line \\ done ".into()));
    }

    #[test]
    fn button_defaults_to_primary() {
        assert_eq!(
            "down 3 4".parse(),
            Ok(Command::PointerDown { x: 3.0, y: 4.0, button: PointerButton::Primary })
        );
    }

    #[test]
    fn malformed_lines_are_rejected_with_a_reason() {
        assert_eq!("".parse::<Command>(), Err(ParseError::Empty));
        assert_eq!("   ".parse::<Command>(), Err(ParseError::Empty));
        assert_eq!("teleport 1 2".parse::<Command>(), Err(ParseError::UnknownCommand));
        assert_eq!("move 1".parse::<Command>(), Err(ParseError::Missing("y")));
        assert_eq!("move".parse::<Command>(), Err(ParseError::Missing("x")));
        assert_eq!("move 1 2 3".parse::<Command>(), Err(ParseError::TooMany));
        assert_eq!("move one 2".parse::<Command>(), Err(ParseError::Invalid("x")));
        assert_eq!("down 1 2 left".parse::<Command>(), Err(ParseError::Invalid("button")));
        assert_eq!("key press - hyper".parse::<Command>(), Err(ParseError::Invalid("key")));
        assert_eq!("key tap - enter".parse::<Command>(), Err(ParseError::Invalid("action")));
        assert_eq!("key press ctrl+fn enter".parse::<Command>(), Err(ParseError::Invalid("modifiers")));
        assert_eq!("key press - other:".parse::<Command>(), Err(ParseError::Invalid("key")));
        assert_eq!("text bad\\q".parse::<Command>(), Err(ParseError::Invalid("escape")));
        assert_eq!("text trailing\\".parse::<Command>(), Err(ParseError::Invalid("escape")));
        assert_eq!("text".parse::<Command>(), Err(ParseError::Missing("text")));
        assert_eq!("leave now".parse::<Command>(), Err(ParseError::TooMany));
    }

    #[test]
    fn non_finite_numbers_never_reach_the_application() {
        for line in ["move NaN 1", "move 1 inf", "move -inf 0", "scroll 1 NaN", "down inf 1"] {
            assert!(line.parse::<Command>().is_err(), "{line} must be rejected");
        }
    }
}
