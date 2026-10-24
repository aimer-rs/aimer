//! Persistence for Aimer apps, with one API on every platform.
//!
//! Windows, Apple, Android and the web each have their own answer to "where does
//! an app keep its data", and their own idioms for settings and documents. This
//! crate puts one API over them, built on [`aimer_storage`]'s file and
//! IndexedDB adapters, so app code is written once:
//!
//! * [`AppStorage`] resolves the platform's *locations*: persistent data,
//!   roaming data, cache, and scratch space ([`StorageLocation`]).
//! * [`Preferences`] are small named settings, read and written synchronously
//!   and saved in the background: `UserDefaults`, `SharedPreferences`,
//!   `LocalSettings` and `localStorage`, in one.
//! * [`Document`] is a whole app model saved as one atomic, versioned record,
//!   with migration from older layouts.
//! * [`Persisted`] binds a document to a provider, so the widget tree reads and
//!   updates state that is restored at launch and saved as it changes.
//! * [`StorageValue`] is the encoding that all of it uses, implemented for the
//!   usual types and for your own with [`impl_storage_value!`]. It is owned by
//!   this crate, with no serializer in the public API.
//!
//! # Getting started
//!
//! ```no_run
//! use aimer_persist::{AppStorage, Persisted, impl_storage_value};
//!
//! #[derive(Clone)]
//! struct Settings { volume: f32, name: String }
//! impl_storage_value!(Settings { volume, name });
//!
//! # async fn startup() -> Result<(), aimer_persist::PersistError> {
//! let app = AppStorage::new("com.example.player")?;
//!
//! // Small settings, read synchronously afterwards.
//! let prefs = app.preferences().await?;
//! let volume = prefs.preference("volume", 0.8f32);
//! volume.set(&0.5);                       // saved in the background
//!
//! // App state shared through the widget tree, restored at launch.
//! let settings = Persisted::load(app.document("settings"), || Settings {
//!     volume: 0.8,
//!     name: String::new(),
//! })
//! .await?;
//! let handle = settings.handle();          // give this to a `Provider`
//! # let _ = handle;
//!
//! // Optional: wait until everything is on disk. Background, quit and
//! // terminate are handled for you.
//! prefs.flush().await?;
//! settings.flush().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Not losing data
//!
//! Changes are saved in the background, within a frame or two. When the app goes
//! to the background, loses focus on mobile, quits, or receives `SIGTERM` or
//! Ctrl-C, the event loop reports it and every pending save is finished before the
//! app carries on, waiting up to two seconds. A panic on any thread saves too,
//! from the panic hook ([`set_flush_on_panic`])
//! ([`set_lifecycle_flush_timeout`]). Saves are atomic, so even a kill the
//! process cannot observe (`SIGKILL`, an out-of-memory kill, a native crash)
//! leaves the previous complete state, never a torn one; only the last few
//! milliseconds of changes can be lost then, since nothing can run during one.
//!
//! The panic save needs `aimer_venus::install_panic_hook()`, which the event loop
//! calls at startup; an app with its own loop must call it after installing its
//! Venus runtime.
//!
//! Everything asynchronous is awaited by a Venus task, with a Venus runtime
//! installed, as in `aimer_storage`.

#![deny(missing_docs)]

mod app;
mod document;
mod error;
mod persisted;
mod preferences;
mod value;
mod write_behind;

pub use aimer_storage::StorageLocation;

pub use crate::app::AppStorage;
pub use crate::document::Document;
pub use crate::error::{PersistError, PersistFuture};
pub use crate::persisted::Persisted;
pub use crate::preferences::{Preference, Preferences, Subscription};
pub use crate::value::{DecodeError, Decoder, Encoder, StorageValue, from_bytes, to_bytes};
pub use crate::write_behind::{FlushOutcome, flush_all_blocking};

/// Whether any [`Preferences`] or [`Persisted`] on this thread has a change that
/// has not yet been handed to storage.
pub fn has_unsaved_changes() -> bool {
    write_behind::any_unsaved()
}

/// Enables or disables saving pending changes when the app panics. The default
/// is enabled.
///
/// A panic ends a release build on the spot, so the panic hook (installed by the
/// event loop) is the last chance to save, and it runs wherever the panic did:
/// a panic on a worker thread still saves state held on the UI thread. The
/// saved state may be part of what went wrong, and an app that reloads it and
/// panics again would crash on every launch; an app that cannot rule that out
/// can pass `false`.
pub fn set_flush_on_panic(enabled: bool) {
    write_behind::set_flush_on_panic(enabled);
}

/// Sets how long the event loop may block the UI thread, when the app goes to
/// the background or exits, finishing pending saves. The default is two seconds.
///
/// Keep it short: mobile operating systems terminate an app whose main thread
/// stalls for several seconds, which would lose the very data being saved.
pub fn set_lifecycle_flush_timeout(timeout: std::time::Duration) {
    write_behind::set_lifecycle_timeout(timeout);
}
