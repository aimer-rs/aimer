#![doc = include_str!("../README.md")]

mod chord;
pub mod global;
mod global_keys;
mod keymap;
pub mod menu;
mod scope;

pub use chord::{ChordMods, Key, KeyChord, ParseChordError};
pub use keymap::{Keymap, resolve};
pub use scope::{Activation, ShortcutAction, ShortcutScope, scopes_alive};
