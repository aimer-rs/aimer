//! Whole gestures, in the words an agent or a person already uses for them.
//!
//! A [`Command`] is one event; a click is three of them, a drag is a dozen. These
//! build the sequence so a caller says `click(x, y)` or `key("ctrl+shift+a")` and
//! gets what a real mouse or keyboard would have produced. The vocabulary is
//! the one computer-use tools share — click, drag, scroll by direction and
//! amount, `key` with a `ctrl+a` combo, `type` — plus the touch gestures a
//! phone needs: tap, swipe and pinch.
//!
//! Positions are logical pixels in window coordinates, as in [`Command`]. Every
//! builder checks its numbers and returns an [`ActionError`] instead of a
//! sequence the app would mistranslate.
//!
//! # Examples
//!
//! ```
//! use aimer_prendre::actions;
//! use aimer_prendre::PointerButton;
//!
//! let lines: Vec<String> = actions::click(120.0, 48.0, PointerButton::Primary, 1)?
//!     .iter()
//!     .map(ToString::to_string)
//!     .collect();
//! assert_eq!(lines, ["glide 120 48", "down 120 48 primary", "up 120 48 primary"]);
//! # Ok::<(), actions::ActionError>(())
//! ```

use std::fmt;
use std::str::FromStr;

use crate::Command;
use crate::model::{KeyAction, Modifiers, NamedKey, PointerButton, ScrollDeltaKind, TouchPhase};

/// How many moves a drag or swipe is cut into. Few enough to be quick, enough
/// that a widget watching the pointer sees a path and not a jump.
const PATH_STEPS: u32 = 8;
/// How many steps a pinch is cut into.
const PINCH_STEPS: u32 = 5;
/// The most wheel steps one scroll may ask for; beyond it is a typo.
const MAX_SCROLL_AMOUNT: f32 = 10_000.0;
/// The finger every touch gesture uses.
const FINGER: u64 = 1;

/// Why an action could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionError(String);

impl ActionError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ActionError {}

/// The way a scroll reveals more content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Reveal what is above.
    Up,
    /// Reveal what is below.
    Down,
    /// Reveal what is to the left.
    Left,
    /// Reveal what is to the right.
    Right,
}

impl FromStr for Direction {
    type Err = ActionError;

    fn from_str(text: &str) -> Result<Self, ActionError> {
        match text.to_ascii_lowercase().as_str() {
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            _ => Err(ActionError::new("a direction is up, down, left or right")),
        }
    }
}

fn finite(values: &[f32], what: &str) -> Result<(), ActionError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(ActionError::new(format!("{what} must be finite numbers")))
    }
}

fn lerp(from: (f32, f32), to: (f32, f32), step: u32) -> (f32, f32) {
    // The last step is the target itself, not a float that nearly is.
    if step == PATH_STEPS {
        return to;
    }
    let t = step as f32 / PATH_STEPS as f32;
    (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
}

/// The pointer glides to `(x, y)`, crossing what lies between as a mouse would,
/// so a hover starts and ends on the way and a viewer can follow it. A raw,
/// instant `move` is a command of its own.
pub fn mouse_move(x: f32, y: f32) -> Result<Vec<Command>, ActionError> {
    finite(&[x, y], "a position")?;
    Ok(vec![Command::PointerGlide { x, y, millis: None, modifiers: Modifiers::default() }])
}

/// Glides to `(x, y)` and clicks `button` there `count` times (one to three).
///
/// The press waits for the glide to arrive, so it lands on the point and does not
/// cut the glide short.
pub fn click(x: f32, y: f32, button: PointerButton, count: u8) -> Result<Vec<Command>, ActionError> {
    finite(&[x, y], "a position")?;
    if !(1..=3).contains(&count) {
        return Err(ActionError::new("a click is one to three presses"));
    }
    let modifiers = Modifiers::default();
    let mut commands = vec![Command::PointerGlide { x, y, millis: None, modifiers }];
    for _ in 0..count {
        commands.push(Command::PointerDown { x, y, button, modifiers });
        commands.push(Command::PointerUp { x, y, button, modifiers });
    }
    Ok(commands)
}

/// Glides to `from`, presses, glides to `to` holding the button, and releases
/// there. Each press and release waits for the glide before it to arrive.
pub fn drag(from: (f32, f32), to: (f32, f32)) -> Result<Vec<Command>, ActionError> {
    finite(&[from.0, from.1, to.0, to.1], "a position")?;
    let (modifiers, button) = (Modifiers::default(), PointerButton::Primary);
    Ok(vec![
        Command::PointerGlide { x: from.0, y: from.1, millis: None, modifiers },
        Command::PointerDown { x: from.0, y: from.1, button, modifiers },
        Command::PointerGlide { x: to.0, y: to.1, millis: None, modifiers },
        Command::PointerUp { x: to.0, y: to.1, button, modifiers },
    ])
}

/// Scrolls `amount` wheel steps so that `direction` is revealed: scrolling down
/// moves the content up.
///
/// With `at`, the mouse glides there first and the scroll waits for it to arrive.
/// Without, it scrolls wherever the mouse already is: the position the app last
/// had for it, injected or real.
pub fn scroll(at: Option<(f32, f32)>, direction: Direction, amount: f32) -> Result<Vec<Command>, ActionError> {
    finite(&[amount], "an amount")?;
    if let Some((x, y)) = at {
        finite(&[x, y], "a position")?;
    }
    if amount <= 0.0 || amount > MAX_SCROLL_AMOUNT {
        return Err(ActionError::new(format!("an amount is above 0 and at most {MAX_SCROLL_AMOUNT}")));
    }
    // The content moves the other way: the offset of a list scrolled down is negative.
    let (dx, dy) = match direction {
        Direction::Up => (0.0, amount),
        Direction::Down => (0.0, -amount),
        Direction::Left => (amount, 0.0),
        Direction::Right => (-amount, 0.0),
    };
    let modifiers = Modifiers::default();
    let scroll = Command::Scroll { dx, dy, kind: ScrollDeltaKind::Line, phase: TouchPhase::Moved, modifiers };
    Ok(match at {
        Some((x, y)) => vec![Command::PointerGlide { x, y, millis: None, modifiers }, Command::Settle, scroll],
        None => vec![scroll],
    })
}

/// A finger touches `(x, y)` and lifts.
pub fn tap(x: f32, y: f32) -> Result<Vec<Command>, ActionError> {
    finite(&[x, y], "a position")?;
    Ok(vec![
        Command::Touch { phase: TouchPhase::Started, id: FINGER, x, y },
        Command::Touch { phase: TouchPhase::Ended, id: FINGER, x, y },
    ])
}

/// A finger touches `from`, slides to `to`, and lifts.
pub fn swipe(from: (f32, f32), to: (f32, f32)) -> Result<Vec<Command>, ActionError> {
    finite(&[from.0, from.1, to.0, to.1], "a position")?;
    let mut commands = vec![Command::Touch { phase: TouchPhase::Started, id: FINGER, x: from.0, y: from.1 }];
    for step in 1..=PATH_STEPS {
        let (x, y) = lerp(from, to, step);
        commands.push(Command::Touch { phase: TouchPhase::Moved, id: FINGER, x, y });
    }
    commands.push(Command::Touch { phase: TouchPhase::Ended, id: FINGER, x: to.0, y: to.1 });
    Ok(commands)
}

/// A pinch anchored at `(x, y)` that multiplies the scale by `scale`: above `1`
/// zooms in, below `1` zooms out.
pub fn pinch(x: f32, y: f32, scale: f32) -> Result<Vec<Command>, ActionError> {
    finite(&[x, y, scale], "a position and a scale")?;
    if scale <= 0.0 {
        return Err(ActionError::new("a scale is above 0"));
    }
    let step = (scale - 1.0) / PINCH_STEPS as f32;
    let at = |delta, phase| Command::Pinch { x, y, delta, phase };
    let mut commands = vec![at(0.0, TouchPhase::Started)];
    commands.extend((0..PINCH_STEPS).map(|_| at(step, TouchPhase::Moved)));
    commands.push(at(0.0, TouchPhase::Ended));
    Ok(commands)
}

/// Types `text` as one edit, as an input method would commit it.
pub fn type_text(text: &str) -> Command {
    Command::Text(text.to_owned())
}

fn modifier(name: &str, modifiers: &mut Modifiers) -> bool {
    match name {
        "ctrl" | "control" => modifiers.ctrl = true,
        "shift" => modifiers.shift = true,
        "alt" | "option" | "opt" => modifiers.alt = true,
        "meta" | "cmd" | "command" | "super" | "win" | "windows" => modifiers.meta = true,
        _ => return false,
    }
    true
}

fn named(name: &str) -> Option<NamedKey> {
    Some(match name {
        "enter" | "return" => NamedKey::Enter,
        "escape" | "esc" => NamedKey::Escape,
        "tab" => NamedKey::Tab,
        "backspace" | "back_space" => NamedKey::Backspace,
        "delete" | "del" => NamedKey::Delete,
        "up" | "arrowup" | "arrow_up" => NamedKey::ArrowUp,
        "down" | "arrowdown" | "arrow_down" => NamedKey::ArrowDown,
        "left" | "arrowleft" | "arrow_left" => NamedKey::ArrowLeft,
        "right" | "arrowright" | "arrow_right" => NamedKey::ArrowRight,
        "pageup" | "page_up" | "pgup" => NamedKey::PageUp,
        "pagedown" | "page_down" | "pgdn" => NamedKey::PageDown,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        _ => return None,
    })
}

/// Presses and releases a key or a combination such as `ctrl+shift+a`,
/// `Return`, `cmd+c` or `F5`.
///
/// Modifiers come first, joined with `+`: `ctrl`, `shift`, `alt` (or `option`),
/// `meta` (or `cmd`, `command`, `super`). A character typed with no `ctrl`,
/// `meta` or `alt` held is text, as it is on a real keyboard, so `a` and `space`
/// are [`type_text`]; with one of them held it is a shortcut, so `cmd+c` is the
/// key `c` with `meta`. Other names are passed on as the platform names them.
pub fn key(spec: &str) -> Result<Vec<Command>, ActionError> {
    let spec = spec.trim();
    let mut parts: Vec<&str> = spec.split('+').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(ActionError::new("a key is `NAME` or `MODIFIER+MODIFIER+NAME`"));
    }
    let name = parts.pop().expect("split yields at least one part");
    let mut modifiers = Modifiers::default();
    for part in parts {
        if !modifier(&part.to_ascii_lowercase(), &mut modifiers) {
            return Err(ActionError::new(format!("`{part}` is not a modifier: ctrl, shift, alt or meta")));
        }
    }
    let lower = name.to_ascii_lowercase();
    if modifier(&lower, &mut Modifiers::default()) {
        return Err(ActionError::new("a combination needs a key after its modifiers"));
    }
    let chord = modifiers.ctrl || modifiers.meta || modifiers.alt;

    let key = if let Some(key) = named(&lower) {
        key
    } else if lower == "space" {
        if chord { NamedKey::Other("Space".into()) } else { return Ok(vec![type_text(" ")]) }
    } else if name.chars().count() == 1 {
        if chord {
            NamedKey::Other(lower)
        } else if modifiers.shift {
            return Ok(vec![type_text(&name.to_uppercase())]);
        } else {
            return Ok(vec![type_text(name)]);
        }
    } else if lower.strip_prefix('f').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
        NamedKey::Other(name.to_ascii_uppercase())
    } else {
        NamedKey::Other(name.to_owned())
    };
    let event = |action| Command::Key { key: key.clone(), action, modifiers };
    Ok(vec![event(KeyAction::Pressed), event(KeyAction::Released)])
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
