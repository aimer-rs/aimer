//! Global shortcuts: chords that fire while the application is in the
//! background, system-wide.
//!
//! Register one with [`GlobalShortcut::register`] and keep the returned guard
//! for as long as the shortcut should exist; dropping it releases the chord
//! back to the system. The action always runs on the UI thread, inside the
//! event loop, so it can touch widget state like any other callback.
//!
//! # Platforms
//!
//! | Platform | Mechanism |
//! |---|---|
//! | macOS | Carbon `RegisterEventHotKey` (needs no Accessibility permission) |
//! | Windows | `RegisterHotKey` on a dedicated message thread |
//! | Linux (X11) | `XGrabKey` on the root window, on a dedicated thread |
//! | Linux (Wayland), iOS, Android, web | [`GlobalShortcutError::Unsupported`] |
//!
//! Wayland has no API for an application to claim a system-wide key. Under
//! XWayland the X11 grab only sees keys pressed while an X11 window has focus.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::sync::OnceLock;

use crate::{ChordMods, Key, KeyChord, ShortcutAction};

#[cfg(target_os = "macos")]
#[path = "global_macos.rs"]
mod platform;
#[cfg(windows)]
#[path = "global_windows.rs"]
mod platform;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
#[path = "global_x11.rs"]
mod platform;
#[cfg(not(any(
    target_os = "macos",
    windows,
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
mod platform {
    use super::*;

    pub(super) fn backend() -> Box<dyn Backend> {
        Box::new(Unavailable)
    }
}

/// Identifies one registered global shortcut for as long as it is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlobalShortcutId(u32);

impl GlobalShortcutId {
    /// The id as a number, for crossing into a native API.
    #[inline]
    pub const fn as_raw(self) -> u32 {
        self.0
    }

    /// The id a native API handed back.
    #[inline]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// Why a global shortcut could not be registered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlobalShortcutError {
    /// This platform has no way for an application to claim a system-wide key.
    Unsupported,
    /// A system-wide key needs at least one modifier (or to be a function key):
    /// claiming a bare letter would stop it being typed anywhere.
    NeedsModifier,
    /// The operating system has no code for the chord's key.
    KeyNotSupported,
    /// This application already registered the chord.
    AlreadyRegistered,
    /// Another application owns the chord.
    Taken,
    /// The operating system failed in some other way.
    Platform(String),
}

impl fmt::Display for GlobalShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("global shortcuts are not supported on this platform"),
            Self::NeedsModifier => f.write_str("a global shortcut needs a modifier key"),
            Self::KeyNotSupported => f.write_str("the platform has no code for this key"),
            Self::AlreadyRegistered => f.write_str("this application already registered the chord"),
            Self::Taken => f.write_str("another application owns the chord"),
            Self::Platform(why) => write!(f, "the operating system refused the shortcut: {why}"),
        }
    }
}

impl std::error::Error for GlobalShortcutError {}

/// What a platform has to provide: claim a chord under an id, release it.
trait Backend {
    fn register(&mut self, id: GlobalShortcutId, chord: &KeyChord) -> Result<(), GlobalShortcutError>;
    fn unregister(&mut self, id: GlobalShortcutId);
}

/// The backend of a platform that has none, or whose connection failed.
// Used by every platform but macOS, whose backend cannot fail to start.
#[cfg_attr(target_os = "macos", allow(dead_code))]
struct Unavailable;

impl Backend for Unavailable {
    fn register(&mut self, _: GlobalShortcutId, _: &KeyChord) -> Result<(), GlobalShortcutError> {
        Err(GlobalShortcutError::Unsupported)
    }

    fn unregister(&mut self, _: GlobalShortcutId) {}
}

/// The registered shortcuts of one thread, and the operating-system claims
/// behind them.
struct Registry {
    backend: Box<dyn Backend>,
    shortcuts: HashMap<GlobalShortcutId, (KeyChord, ShortcutAction)>,
    next: u32,
}

impl Registry {
    fn new(backend: Box<dyn Backend>) -> Self {
        Self { backend, shortcuts: HashMap::new(), next: 1 }
    }

    fn register(&mut self, chord: KeyChord, action: ShortcutAction) -> Result<GlobalShortcutId, GlobalShortcutError> {
        if chord.mods() == ChordMods::NONE && !matches!(chord.key(), Key::F(_)) {
            return Err(GlobalShortcutError::NeedsModifier);
        }
        if self.shortcuts.values().any(|(held, _)| *held == chord) {
            return Err(GlobalShortcutError::AlreadyRegistered);
        }
        let id = GlobalShortcutId(self.next);
        self.backend.register(id, &chord)?;
        self.next = self.next.wrapping_add(1).max(1);
        self.shortcuts.insert(id, (chord, action));
        Ok(id)
    }

    fn unregister(&mut self, id: GlobalShortcutId) {
        if self.shortcuts.remove(&id).is_some() {
            self.backend.unregister(id);
        }
    }

    /// The action of `id`, cloned so it can run after the registry is released.
    fn action(&self, id: GlobalShortcutId) -> Option<ShortcutAction> {
        self.shortcuts.get(&id).map(|(_, action)| ShortcutAction::clone(action))
    }

    /// Runs the action of `id` at once; the real path goes through [`dispatch`].
    #[cfg(test)]
    fn fire(&self, id: GlobalShortcutId) -> bool {
        match self.action(id) {
            Some(action) => {
                action();
                true
            }
            None => false,
        }
    }
}

aimer_tls::thread_local! {
    static REGISTRY: RefCell<Option<Registry>> = const { RefCell::new(None) };
}

fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|slot| {
        let mut slot = slot.borrow_mut();
        f(slot.get_or_insert_with(|| Registry::new(platform::backend())))
    })
}

/// A held global shortcut. Dropping it releases the chord.
///
/// It stays on the thread that registered it, the UI thread.
///
/// # Examples
///
/// ```no_run
/// use aimer_shortcut::global::GlobalShortcut;
///
/// let toggle = GlobalShortcut::register("CmdOrCtrl+Shift+Space".parse().unwrap(), || {
///     println!("summoned from anywhere");
/// })?;
/// // Keep `toggle` alive; the shortcut ends when it is dropped.
/// # Ok::<(), aimer_shortcut::global::GlobalShortcutError>(())
/// ```
#[must_use = "the shortcut is released as soon as the guard is dropped"]
pub struct GlobalShortcut {
    id: GlobalShortcutId,
    chord: KeyChord,
    /// The registry is thread-local, so the guard must not leave its thread.
    _not_send: PhantomData<*const ()>,
}

impl GlobalShortcut {
    /// Claims `chord` system-wide and runs `action` whenever it is pressed.
    ///
    /// Call it on the UI thread.
    ///
    /// # Errors
    ///
    /// See [`GlobalShortcutError`].
    pub fn register(chord: KeyChord, action: impl Fn() + 'static) -> Result<Self, GlobalShortcutError> {
        let id = with_registry(|registry| registry.register(chord, std::rc::Rc::new(action)))?;
        Ok(Self { id, chord, _not_send: PhantomData })
    }

    /// The identity the platform reports when the shortcut fires.
    #[inline]
    pub fn id(&self) -> GlobalShortcutId {
        self.id
    }

    /// The chord this shortcut claims.
    #[inline]
    pub fn chord(&self) -> KeyChord {
        self.chord
    }
}

impl Drop for GlobalShortcut {
    fn drop(&mut self) {
        // Torn down while the thread is exiting, the registry may be gone
        // already, and so is the claim with it.
        let _ = REGISTRY.try_with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut()
                && let Some(registry) = slot.as_mut()
            {
                registry.unregister(self.id);
            }
        });
    }
}

/// Runs the action of the shortcut that fired. Returns whether there was one.
///
/// The platform layer calls this on the UI thread after a backend reported
/// the id through the [notifier](set_notifier).
pub fn dispatch(id: GlobalShortcutId) -> bool {
    // The registry is released before the action runs, so the action may
    // register or drop shortcuts itself.
    match with_registry(|registry| registry.action(id)) {
        Some(action) => {
            action();
            true
        }
        None => false,
    }
}

static NOTIFIER: OnceLock<fn(GlobalShortcutId)> = OnceLock::new();

/// Installs how a backend wakes the UI thread when a shortcut fires.
///
/// The backends hear the key press on threads of their own; `notify` must hand
/// the id to the UI thread (the platform layer posts an event to the event
/// loop), which then calls [`dispatch`]. Only the first call has an effect.
pub fn set_notifier(notify: fn(GlobalShortcutId)) {
    let _ = NOTIFIER.set(notify);
}

/// Reports that `id` fired. Called by the backends, from any thread.
fn notify(id: GlobalShortcutId) {
    if let Some(notify) = NOTIFIER.get() {
        notify(id);
    }
}

#[cfg(test)]
#[path = "global_tests.rs"]
mod global_tests;
