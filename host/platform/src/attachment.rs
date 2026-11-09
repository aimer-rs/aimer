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

/// Who is connected, and which of them are agents.
#[derive(Default)]
struct Connections {
    /// Everyone, in the order they connected.
    everyone: Vec<String>,
    /// The ones controlling the app, as opposed to supervising it.
    agents: Vec<String>,
}

static CONNECTIONS: Mutex<Connections> = Mutex::new(Connections { everyone: Vec::new(), agents: Vec::new() });
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Replaces the names of everyone connected and, of those, the agents; and
/// reports whether that changed anything.
///
/// Being connected is not controlling: the tool that launched the app holds a
/// connection for the whole run, and only an agent connection means someone is
/// remote-controlling the app. `agents` is the part of `everyone` that is.
///
/// Callable from any thread. The caller wakes the event loop when this returns
/// `true`; unchanged lists ask for nothing, so a flapping connection does not
/// wake an idle app for no reason.
pub fn set_connections(everyone: Vec<String>, agents: Vec<String>) -> bool {
    let mut connections = CONNECTIONS.lock().expect("attachment mutex poisoned");
    if connections.everyone == everyone && connections.agents == agents {
        return false;
    }
    *connections = Connections { everyone, agents };
    GENERATION.fetch_add(1, Ordering::Release);
    true
}

/// Replaces the names of everyone attached, taking every one of them to be an
/// agent, and reports whether that changed anything. See [`set_connections`].
pub fn set_attached(names: Vec<String>) -> bool {
    set_connections(names.clone(), names)
}

/// The names of everyone connected now, in the order they connected, launcher
/// and agents alike. Empty when nobody is.
pub fn attached() -> Vec<String> {
    CONNECTIONS.lock().expect("attachment mutex poisoned").everyone.clone()
}

/// The names of the connections that are remote-controlling the app, in the order
/// they connected. Empty when nobody is, even if the tool that launched the app
/// is connected: that is what the debug badge follows.
pub fn agents() -> Vec<String> {
    CONNECTIONS.lock().expect("attachment mutex poisoned").agents.clone()
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
    fn a_launcher_is_connected_but_not_an_agent() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_attached(Vec::new());
        assert!(set_connections(names(&["aimer-cli"]), Vec::new()));
        assert_eq!(attached(), ["aimer-cli"]);
        assert!(agents().is_empty(), "supervising is not controlling");

        assert!(set_connections(names(&["aimer-cli", "mcp:codex"]), names(&["mcp:codex"])));
        assert_eq!(agents(), ["mcp:codex"]);
        assert_eq!(attached(), ["aimer-cli", "mcp:codex"]);

        assert!(set_connections(names(&["aimer-cli"]), Vec::new()), "the agent leaving is a change");
        assert!(agents().is_empty());
    }

    #[test]
    fn a_change_in_only_the_agents_is_a_new_generation() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_connections(names(&["a", "b"]), Vec::new());
        let before = generation();
        assert!(!set_connections(names(&["a", "b"]), Vec::new()));
        assert_eq!(generation(), before);
        assert!(set_connections(names(&["a", "b"]), names(&["b"])), "same people, different roles");
        assert_ne!(generation(), before);
    }

    #[test]
    fn set_attached_means_everyone_is_an_agent() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_attached(names(&["x", "y"]));
        assert_eq!(agents(), ["x", "y"]);
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
