//! The text wire format a [`Command`] travels in: one command per line.
//!
//! ```text
//! move X Y [OPTS]                   down X Y [BUTTON] [OPTS]   up X Y [BUTTON] [OPTS]
//! glide X Y [MS] [OPTS]             settle                     leave
//! scroll DX DY [OPTS]
//! touch start|move|end ID X Y       touch cancel
//! pinch X Y DELTA PHASE             rotate X Y DELTA PHASE     pan X Y DX DY PHASE
//! smartzoom X Y
//! key press|release|repeat MODS KEY text TEXT                  preedit RANGE [TEXT]
//! soft-backspace                    back start|progress P|invoke|cancel
//! link URL                          menu undo|redo|cut|copy|paste|select-all
//! file-hover PATH                   file-hover-cancel          file-drop PATH
//! quit
//! ```
//!
//! `BUTTON` is `primary` (the default), `secondary`, `middle` or `other:N`.
//! `MODS` is `-` or `+`-joined `ctrl`, `shift`, `alt`, `meta`. `KEY` is one of
//! the [`NamedKey`] names in lowercase (`pageup`, `enter`, ...) or `other:NAME`.
//! `PHASE` is `start`, `move`, `end` or `cancel`. `RANGE` is `-` or the byte
//! range `START,END` of the active segment inside the preedit text.
//!
//! `MS` is how long a `glide` takes, from 1 to 3000 milliseconds.
//!
//! `OPTS` are `name=value` words after the positional arguments, in any order,
//! each at most once: `mods=MODS` on `move`, `glide`, `down`, `up` and `scroll`, and on
//! `scroll` also `kind=pixel|line` (default `pixel`) and `phase=PHASE` (default
//! `move`). A command's default is never written, so the shortest line is also
//! the one a person would type.
//!
//! `TEXT`, `NAME`, `URL` and `PATH` run to the end of the line, so they may hold
//! spaces; `\`, newline and carriage return are written `\\`, `\n` and `\r`.
//!
//! Text keeps the protocol greppable and lets a shell script or `nc` drive an
//! app without a client library, and an input event is rare enough that
//! parsing a line costs nothing next to the frame it triggers.

use std::fmt::{self, Write as _};
use std::str::FromStr;

use std::path::PathBuf;

use crate::model::{KeyAction, Modifiers, NamedKey, PointerButton, ScrollDeltaKind, TouchPhase};

use crate::Command;
use crate::command::{BackStep, EditShortcut, MAX_GLIDE_MILLIS};

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

/// The `name=value` words that follow a command's positional arguments.
#[derive(Default)]
struct Options {
    modifiers: Option<Modifiers>,
    kind: Option<ScrollDeltaKind>,
    phase: Option<TouchPhase>,
}

/// Which options a command accepts.
#[derive(Clone, Copy)]
enum Accepts {
    Modifiers,
    Scroll,
}

impl Options {
    fn parse<'a>(words: impl Iterator<Item = &'a str>, accepts: Accepts) -> Result<Self, ParseError> {
        let mut options = Self::default();
        for word in words {
            let (name, value) = word.split_once('=').ok_or(ParseError::Invalid("option"))?;
            let scroll = matches!(accepts, Accepts::Scroll);
            match name {
                "mods" if options.modifiers.is_none() => options.modifiers = Some(parse_modifiers(value)?),
                "kind" if scroll && options.kind.is_none() => options.kind = Some(parse_kind(value)?),
                "phase" if scroll && options.phase.is_none() => options.phase = Some(parse_phase(value)?),
                _ => return Err(ParseError::Invalid("option")),
            }
        }
        Ok(options)
    }

    fn modifiers(&mut self) -> Modifiers {
        self.modifiers.take().unwrap_or_default()
    }
}

fn parse_kind(word: &str) -> Result<ScrollDeltaKind, ParseError> {
    match word {
        "pixel" => Ok(ScrollDeltaKind::Pixel),
        "line" => Ok(ScrollDeltaKind::Line),
        _ => Err(ParseError::Invalid("kind")),
    }
}

fn parse_phase(word: &str) -> Result<TouchPhase, ParseError> {
    match word {
        "start" => Ok(TouchPhase::Started),
        "move" => Ok(TouchPhase::Moved),
        "end" => Ok(TouchPhase::Ended),
        "cancel" => Ok(TouchPhase::Cancelled),
        _ => Err(ParseError::Invalid("phase")),
    }
}

fn phase_name(phase: TouchPhase) -> &'static str {
    match phase {
        TouchPhase::Started => "start",
        TouchPhase::Moved => "move",
        TouchPhase::Ended => "end",
        TouchPhase::Cancelled => "cancel",
    }
}

fn parse_cursor(word: &str, text: &str) -> Result<Option<(usize, usize)>, ParseError> {
    if word == "-" {
        return Ok(None);
    }
    let invalid = ParseError::Invalid("cursor");
    let (start, end) = word.split_once(',').ok_or(invalid.clone())?;
    let start: usize = start.parse().map_err(|_| invalid.clone())?;
    let end: usize = end.parse().map_err(|_| invalid.clone())?;
    // The range is sliced out of the text downstream, so it must be one.
    if start <= end && end <= text.len() && text.is_char_boundary(start) && text.is_char_boundary(end) {
        Ok(Some((start, end)))
    } else {
        Err(invalid)
    }
}

/// An unescaped, non-empty argument that runs to the end of the line.
fn rest_of_line(rest: Option<&str>, what: &'static str) -> Result<String, ParseError> {
    let text = unescape(rest.ok_or(ParseError::Missing(what))?, what)?;
    if text.is_empty() { Err(ParseError::Invalid(what)) } else { Ok(text) }
}

/// Writes the modifiers as `ctrl+shift`, and reports whether any were held.
fn write_modifiers(modifiers: &Modifiers, f: &mut fmt::Formatter<'_>) -> Result<bool, fmt::Error> {
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
    Ok(any)
}

/// Writes ` mods=...` when any modifier is held.
fn write_mods_option(modifiers: &Modifiers, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if *modifiers != Modifiers::default() {
        f.write_str(" mods=")?;
        write_modifiers(modifiers, f)?;
    }
    Ok(())
}

impl fmt::Display for Command {
    /// Writes the command as one wire line without the trailing newline.
    ///
    /// A command holding a non-finite float is not representable: its line is
    /// rejected by [`Command::from_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PointerMove { x, y, modifiers } => {
                write!(f, "move {x} {y}")?;
                write_mods_option(modifiers, f)
            }
            Self::PointerDown { x, y, button, modifiers } => {
                write!(f, "down {x} {y} ")?;
                button_name(*button, f)?;
                write_mods_option(modifiers, f)
            }
            Self::PointerUp { x, y, button, modifiers } => {
                write!(f, "up {x} {y} ")?;
                button_name(*button, f)?;
                write_mods_option(modifiers, f)
            }
            Self::PointerGlide { x, y, millis, modifiers } => {
                write!(f, "glide {x} {y}")?;
                if let Some(millis) = millis {
                    write!(f, " {millis}")?;
                }
                write_mods_option(modifiers, f)
            }
            Self::Settle => f.write_str("settle"),
            Self::PointerLeave => f.write_str("leave"),
            Self::Scroll { dx, dy, kind, phase, modifiers } => {
                write!(f, "scroll {dx} {dy}")?;
                if *kind == ScrollDeltaKind::Line {
                    f.write_str(" kind=line")?;
                }
                if *phase != TouchPhase::Moved {
                    write!(f, " phase={}", phase_name(*phase))?;
                }
                write_mods_option(modifiers, f)
            }
            Self::Touch { phase, id, x, y } => write!(f, "touch {} {id} {x} {y}", phase_name(*phase)),
            Self::TouchCancel => f.write_str("touch cancel"),
            Self::Pinch { x, y, delta, phase } => write!(f, "pinch {x} {y} {delta} {}", phase_name(*phase)),
            Self::Rotate { x, y, delta, phase } => write!(f, "rotate {x} {y} {delta} {}", phase_name(*phase)),
            Self::Pan { x, y, dx, dy, phase } => write!(f, "pan {x} {y} {dx} {dy} {}", phase_name(*phase)),
            Self::SmartZoom { x, y } => write!(f, "smartzoom {x} {y}"),
            Self::Key { key, action, modifiers } => {
                f.write_str(match action {
                    KeyAction::Pressed => "key press ",
                    KeyAction::Released => "key release ",
                    KeyAction::Repeat => "key repeat ",
                })?;
                if !write_modifiers(modifiers, f)? {
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
            Self::Preedit { text, cursor } => {
                match cursor {
                    Some((start, end)) => write!(f, "preedit {start},{end}")?,
                    None => f.write_str("preedit -")?,
                }
                if !text.is_empty() {
                    f.write_char(' ')?;
                    escape(text, f)?;
                }
                Ok(())
            }
            Self::SoftBackspace => f.write_str("soft-backspace"),
            Self::Back(BackStep::Started) => f.write_str("back start"),
            Self::Back(BackStep::Progressed(p)) => write!(f, "back progress {p}"),
            Self::Back(BackStep::Invoked) => f.write_str("back invoke"),
            Self::Back(BackStep::Cancelled) => f.write_str("back cancel"),
            Self::Link(url) => {
                f.write_str("link ")?;
                escape(url, f)
            }
            Self::Menu(shortcut) => {
                let name = EditShortcut::ALL.iter().find(|(_, s)| s == shortcut).map(|(n, _)| *n);
                write!(f, "menu {}", name.expect("every shortcut is in ALL"))
            }
            Self::FileHover(path) => {
                f.write_str("file-hover ")?;
                escape(&path.to_string_lossy(), f)
            }
            Self::FileHoverCancel => f.write_str("file-hover-cancel"),
            Self::FileDrop(path) => {
                f.write_str("file-drop ")?;
                escape(&path.to_string_lossy(), f)
            }
            Self::Quit => f.write_str("quit"),
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
                let mut options = Options::parse(words, Accepts::Modifiers)?;
                Ok(Self::PointerMove { x, y, modifiers: options.modifiers() })
            }
            "down" | "up" => {
                let (x, y) = (number(words.next(), "x")?, number(words.next(), "y")?);
                let mut words = words.peekable();
                let button = match words.next_if(|w| !w.contains('=')) {
                    Some(word) => parse_button(word)?,
                    None => PointerButton::Primary,
                };
                let mut options = Options::parse(words, Accepts::Modifiers)?;
                let modifiers = options.modifiers();
                Ok(if verb == "down" {
                    Self::PointerDown { x, y, button, modifiers }
                } else {
                    Self::PointerUp { x, y, button, modifiers }
                })
            }
            "glide" => {
                let (x, y) = (number(words.next(), "x")?, number(words.next(), "y")?);
                let mut words = words.peekable();
                let millis = words
                    .next_if(|w| !w.contains('='))
                    .map(|w| {
                        w.parse::<u32>()
                            .ok()
                            .filter(|ms| (1..=MAX_GLIDE_MILLIS).contains(ms))
                            .ok_or(ParseError::Invalid("duration"))
                    })
                    .transpose()?;
                let mut options = Options::parse(words, Accepts::Modifiers)?;
                Ok(Self::PointerGlide { x, y, millis, modifiers: options.modifiers() })
            }
            "settle" => no_more(words).map(|()| Self::Settle),
            "leave" => no_more(words).map(|()| Self::PointerLeave),
            "scroll" => {
                let (dx, dy) = (number(words.next(), "dx")?, number(words.next(), "dy")?);
                let mut options = Options::parse(words, Accepts::Scroll)?;
                Ok(Self::Scroll {
                    dx,
                    dy,
                    kind: options.kind.unwrap_or(ScrollDeltaKind::Pixel),
                    phase: options.phase.unwrap_or(TouchPhase::Moved),
                    modifiers: options.modifiers(),
                })
            }
            "touch" => {
                let phase = parse_phase(words.next().ok_or(ParseError::Missing("phase"))?)?;
                match phase {
                    TouchPhase::Cancelled => no_more(words).map(|()| Self::TouchCancel),
                    phase => {
                        let id = words
                            .next()
                            .ok_or(ParseError::Missing("id"))?
                            .parse()
                            .map_err(|_| ParseError::Invalid("id"))?;
                        let (x, y) = (number(words.next(), "x")?, number(words.next(), "y")?);
                        no_more(words)?;
                        Ok(Self::Touch { phase, id, x, y })
                    }
                }
            }
            "pinch" | "rotate" => {
                let (x, y) = (number(words.next(), "x")?, number(words.next(), "y")?);
                let delta = number(words.next(), "delta")?;
                let phase = parse_phase(words.next().ok_or(ParseError::Missing("phase"))?)?;
                no_more(words)?;
                Ok(if verb == "pinch" {
                    Self::Pinch { x, y, delta, phase }
                } else {
                    Self::Rotate { x, y, delta, phase }
                })
            }
            "pan" => {
                let (x, y) = (number(words.next(), "x")?, number(words.next(), "y")?);
                let (dx, dy) = (number(words.next(), "dx")?, number(words.next(), "dy")?);
                let phase = parse_phase(words.next().ok_or(ParseError::Missing("phase"))?)?;
                no_more(words)?;
                Ok(Self::Pan { x, y, dx, dy, phase })
            }
            "smartzoom" => {
                let (x, y) = (number(words.next(), "x")?, number(words.next(), "y")?);
                no_more(words)?;
                Ok(Self::SmartZoom { x, y })
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
            "preedit" => {
                let (range, text) = match rest.ok_or(ParseError::Missing("cursor"))?.split_once(' ') {
                    Some((range, text)) => (range, text),
                    None => (rest.unwrap_or(""), ""),
                };
                let text = unescape(text, "escape")?;
                let cursor = parse_cursor(range, &text)?;
                Ok(Self::Preedit { text, cursor })
            }
            "soft-backspace" => no_more(words).map(|()| Self::SoftBackspace),
            "back" => {
                let signal = match words.next().ok_or(ParseError::Missing("signal"))? {
                    "start" => BackStep::Started,
                    "progress" => BackStep::Progressed({
                        let progress = number(words.next(), "progress")?;
                        if !(0.0..=1.0).contains(&progress) {
                            return Err(ParseError::Invalid("progress"));
                        }
                        progress
                    }),
                    "invoke" => BackStep::Invoked,
                    "cancel" => BackStep::Cancelled,
                    _ => return Err(ParseError::Invalid("signal")),
                };
                no_more(words)?;
                Ok(Self::Back(signal))
            }
            "link" => rest_of_line(rest, "url").map(Self::Link),
            "menu" => {
                let name = words.next().ok_or(ParseError::Missing("shortcut"))?;
                let shortcut = EditShortcut::ALL
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, s)| *s)
                    .ok_or(ParseError::Invalid("shortcut"))?;
                no_more(words)?;
                Ok(Self::Menu(shortcut))
            }
            "file-hover" => rest_of_line(rest, "path").map(|p| Self::FileHover(PathBuf::from(p))),
            "file-drop" => rest_of_line(rest, "path").map(|p| Self::FileDrop(PathBuf::from(p))),
            "file-hover-cancel" => no_more(words).map(|()| Self::FileHoverCancel),
            "quit" => no_more(words).map(|()| Self::Quit),
            _ => Err(ParseError::UnknownCommand),
        }
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
