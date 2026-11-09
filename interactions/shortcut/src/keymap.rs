//! Chord-to-action tables and the scope chain that resolves them.

use std::collections::HashMap;

use crate::KeyChord;

/// The bindings of one scope: at most one action per chord.
///
/// A scope is a window, a panel or a dialog; see [`resolve`] for how nested
/// scopes combine.
///
/// # Examples
///
/// ```
/// use aimer_shortcut::{Keymap, KeyChord};
///
/// let mut map = Keymap::new();
/// map.bind("Ctrl+S".parse().unwrap(), "save");
/// assert_eq!(map.get(&"Ctrl+S".parse().unwrap()), Some(&"save"));
/// ```
#[derive(Clone, Debug)]
pub struct Keymap<A> {
    bindings: HashMap<KeyChord, A>,
}

impl<A> Default for Keymap<A> {
    #[inline]
    fn default() -> Self {
        Self { bindings: HashMap::new() }
    }
}

impl<A> Keymap<A> {
    /// Creates an empty keymap.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds `chord` to `action`, returning the action it displaced, if any.
    #[inline]
    pub fn bind(&mut self, chord: KeyChord, action: A) -> Option<A> {
        self.bindings.insert(chord, action)
    }

    /// Removes the binding of `chord`, returning its action.
    #[inline]
    pub fn unbind(&mut self, chord: &KeyChord) -> Option<A> {
        self.bindings.remove(chord)
    }

    /// The action bound to `chord` in this scope alone.
    #[inline]
    pub fn get(&self, chord: &KeyChord) -> Option<&A> {
        self.bindings.get(chord)
    }

    /// The number of bound chords.
    #[inline]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether nothing is bound.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Every binding, in no particular order.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&KeyChord, &A)> {
        self.bindings.iter()
    }
}

/// Finds the action for `chord` in a chain of scopes given innermost first.
///
/// The first scope that binds the chord wins, so an inner scope overrides an
/// outer one and a chord it does not bind falls through to the outer scopes.
/// This allocates nothing, which matters because it runs on every key press.
///
/// # Examples
///
/// ```
/// use aimer_shortcut::{resolve, Keymap};
///
/// let (mut window, mut editor) = (Keymap::new(), Keymap::new());
/// window.bind("Ctrl+S".parse().unwrap(), "save all");
/// editor.bind("Ctrl+S".parse().unwrap(), "save file");
///
/// let chord = "Ctrl+S".parse().unwrap();
/// assert_eq!(resolve([&editor, &window], &chord), Some(&"save file"));
/// ```
pub fn resolve<'a, A: 'a>(
    chain: impl IntoIterator<Item = &'a Keymap<A>>,
    chord: &KeyChord,
) -> Option<&'a A> {
    chain.into_iter().find_map(|scope| scope.get(chord))
}

#[cfg(test)]
#[path = "keymap_tests.rs"]
mod keymap_tests;
