//! A scope's bindings, the unit the widget tree activates.

use std::cell::Cell;
use std::rc::Rc;

use crate::{KeyChord, Keymap};

/// What runs when a chord is pressed.
pub type ShortcutAction = Rc<dyn Fn()>;

aimer_tls::thread_local! {
    static LIVE_SCOPES: Cell<usize> = const { Cell::new(0) };
}

/// Whether any [`ShortcutScope`] exists on this thread.
///
/// The dispatcher consults this before it builds chords or walks the tree, so
/// an application that never uses shortcuts pays one thread-local read per key
/// press and nothing else.
#[inline]
pub fn scopes_alive() -> bool {
    LIVE_SCOPES.with(|live| live.get() != 0)
}

/// Counts one live scope; the count drops when the scope does.
pub(crate) struct LiveToken;

impl LiveToken {
    pub(crate) fn new() -> Self {
        LIVE_SCOPES.with(|live| live.set(live.get() + 1));
        Self
    }
}

impl Drop for LiveToken {
    fn drop(&mut self) {
        LIVE_SCOPES.with(|live| live.set(live.get() - 1));
    }
}

/// The outcome of offering a chord to one scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    /// The scope does not bind the chord.
    Unbound,
    /// The chord is bound and its action ran.
    Ran,
    /// The chord is bound, but this was an auto-repeat the scope ignores. The
    /// key press is still the scope's, so it must not leak to anything else.
    Held,
}

impl Activation {
    /// Whether the scope claimed the key press.
    #[inline]
    pub const fn is_claimed(self) -> bool {
        !matches!(self, Self::Unbound)
    }
}

/// The shortcuts of one region of the application.
///
/// A scope is *focus-scoped* by default: it is consulted while keyboard focus
/// is inside the subtree that carries it. A [window](ShortcutScope::window)
/// scope is consulted whenever it is in the tree, wherever focus is.
///
/// # Examples
///
/// ```
/// use aimer_shortcut::{Activation, ShortcutScope};
///
/// let scope = ShortcutScope::new().bind("Ctrl+S".parse().unwrap(), || println!("saved"));
/// assert_eq!(scope.activate(&"Ctrl+S".parse().unwrap(), false), Activation::Ran);
/// ```
pub struct ShortcutScope {
    keymap: Keymap<ShortcutAction>,
    window: bool,
    repeat: bool,
    _live: LiveToken,
}

impl ShortcutScope {
    /// Creates an empty, focus-scoped scope that ignores auto-repeat.
    #[inline]
    pub fn new() -> Self {
        Self { keymap: Keymap::new(), window: false, repeat: false, _live: LiveToken::new() }
    }

    /// Binds `chord` to `action`; a later binding of the same chord wins.
    #[inline]
    pub fn bind(mut self, chord: KeyChord, action: impl Fn() + 'static) -> Self {
        self.keymap.bind(chord, Rc::new(action));
        self
    }

    /// Makes the scope active whenever it is in the tree, not only while focus
    /// is inside it.
    #[inline]
    pub fn window(mut self, window: bool) -> Self {
        self.window = window;
        self
    }

    /// Lets a held key run the action repeatedly. Off by default, because a
    /// shortcut that opens a dialog must not open forty of them.
    #[inline]
    pub fn repeat(mut self, repeat: bool) -> Self {
        self.repeat = repeat;
        self
    }

    /// Whether the scope is active regardless of focus.
    #[inline]
    pub fn is_window(&self) -> bool {
        self.window
    }

    /// The scope's bindings.
    #[inline]
    pub fn keymap(&self) -> &Keymap<ShortcutAction> {
        &self.keymap
    }

    /// Runs the action bound to `chord`, if any.
    ///
    /// `repeating` is whether this press is the keyboard's auto-repeat.
    pub fn activate(&self, chord: &KeyChord, repeating: bool) -> Activation {
        let Some(action) = self.keymap.get(chord) else {
            return Activation::Unbound;
        };
        if repeating && !self.repeat {
            return Activation::Held;
        }
        // Cloned so the action may rebuild the widget that owns this scope.
        let action = Rc::clone(action);
        action();
        Activation::Ran
    }
}

impl Default for ShortcutScope {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod scope_tests;
