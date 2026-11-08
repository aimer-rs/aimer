//! Who is attached to the running app, and where their synthetic pointers are.
//!
//! A tool that drives the app — `aimer run`, an agent through its MCP — holds a
//! connection to it for as long as it is attached. `aimer_prendre`'s server
//! reports those connections here, from its own threads, and wakes the event
//! loop with [`AimerNativePlatformEvent::AttachmentChanged`]; the loop and the
//! debug overlay read the answer on their own thread. Nothing here is
//! application state: it is for tools that show the app is being driven.
//!
//! The same module carries the other half of "someone is driving this": a
//! [`SyntheticPointer`] says where an injected mouse or finger is, because an
//! injected event moves no real cursor and would otherwise leave nothing to see.
//!
//! [`AimerNativePlatformEvent::AttachmentChanged`]: crate::AimerNativePlatformEvent::AttachmentChanged

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use aimer_attribute::position::Vec2d;

static ATTACHED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Replaces the names of everyone attached, one per connection, and reports
/// whether that changed anything.
///
/// Callable from any thread. The caller wakes the event loop when this returns
/// `true`; an unchanged list asks for nothing, so a flapping connection does not
/// wake an idle app for no reason.
pub fn set_attached(names: Vec<String>) -> bool {
    let mut attached = ATTACHED.lock().expect("attachment mutex poisoned");
    if *attached == names {
        return false;
    }
    *attached = names;
    GENERATION.fetch_add(1, Ordering::Release);
    true
}

/// The names of everyone attached now, in the order they attached. Empty when
/// nobody is.
pub fn attached() -> Vec<String> {
    ATTACHED.lock().expect("attachment mutex poisoned").clone()
}

/// A number that changes whenever [`attached`] does, so a reader can tell
/// whether its copy is stale without comparing names.
#[inline]
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// What an injected pointer just did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerPhase {
    /// It moved, or hovered, without touching.
    Moved,
    /// A button went down, or a finger touched the surface.
    Pressed,
    /// The button came up, or the finger lifted.
    Released,
    /// It left the window, or the system cancelled the touch: nothing to show.
    Left,
}

/// An injected mouse or finger, reported just before the input it stands for so
/// the loop knows the input is not a person's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntheticPointer {
    /// A finger rather than the mouse.
    pub touch: bool,
    /// Which finger; `0` for the mouse.
    pub id: u64,
    /// What it just did.
    pub phase: MarkerPhase,
    /// Where, in logical window coordinates. Meaningless for
    /// [`MarkerPhase::Left`].
    pub pos: Vec2d,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The state is process-wide, and `cargo test` runs tests in one process.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn attaching_and_detaching_are_reported_in_order() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_attached(Vec::new());
        assert!(set_attached(names(&["aimer-cli"])));
        assert_eq!(attached(), ["aimer-cli"]);
        assert!(set_attached(names(&["aimer-cli", "mcp:claude"])));
        assert_eq!(attached(), ["aimer-cli", "mcp:claude"]);
        assert!(set_attached(Vec::new()));
        assert!(attached().is_empty());
    }

    #[test]
    fn an_unchanged_list_changes_nothing_and_wakes_nobody() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_attached(names(&["a"]));
        let before = generation();
        assert!(!set_attached(names(&["a"])), "same list: no change to report");
        assert_eq!(generation(), before, "and no new generation");
        assert!(set_attached(names(&["b"])));
        assert_ne!(generation(), before);
    }
}
