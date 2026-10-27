//! Thread-local storage that does not run out of OS TLS keys.
//!
//! Rust's `aarch64-linux-android` target has no native `#[thread_local]`
//! support in std, so every `thread_local!` static is backed by its own
//! `pthread_key_create` key. Bionic gives a whole process only about 128–141
//! keys, and the Android runtime, libc++ and Skia already hold some of them.
//! A GUI framework that declares one `thread_local!` per subsystem exhausts the
//! rest, and std then aborts with `fatal runtime error: out of TLS keys`.
//!
//! [`thread_local!`] has the same grammar as [`std::thread_local!`]:
//!
//! - On Android every static declared through it is stored in a single
//!   per-thread table, so all of them together cost exactly one key.
//! - On every other target it is [`std::thread_local!`] itself, so there is no
//!   behavioural or performance change.
//!
//! # Differences on Android
//!
//! The table is dropped by the destructor of one std thread-local, so while a
//! thread is being torn down none of the shared statics can be reached from
//! another destructor. [`LocalKey::try_with`] reports that as an
//! [`AccessError`], and [`LocalKey::with`] panics, which is how std behaves
//! once a key has been destroyed. Values are dropped in reverse order of
//! initialisation.

#[cfg(not(target_os = "android"))]
pub use std::thread::{AccessError, LocalKey};
#[cfg(not(target_os = "android"))]
pub use std::thread_local;

#[cfg(target_os = "android")]
pub use shared::{AccessError, LocalKey};

#[doc(hidden)]
pub mod shared;

/// Declares thread-local statics that share one OS TLS key on Android.
///
/// Accepts the same input as [`std::thread_local!`], including `const { .. }`
/// initializers, visibility, attributes and several statics per invocation.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
///
/// aimer_tls::thread_local! {
///     static COUNTER: Cell<u32> = const { Cell::new(0) };
/// }
///
/// COUNTER.set(COUNTER.get() + 1);
/// assert_eq!(COUNTER.get(), 1);
/// ```
#[cfg(target_os = "android")]
#[macro_export]
macro_rules! thread_local {
    ($($tt:tt)*) => {
        $crate::shared_thread_local! { $($tt)* }
    };
}

/// Always uses the shared table, whatever the target.
///
/// [`thread_local!`] forwards here on Android. It is exported on every target
/// so the table can be tested on a development host.
#[doc(hidden)]
#[macro_export]
macro_rules! shared_thread_local {
    () => {};

    ($(#[$attr:meta])* $vis:vis static $name:ident : $t:ty = const $init:block $(; $($rest:tt)*)?) => {
        $crate::shared_thread_local!(@one $(#[$attr])* $vis $name: $t = const $init);
        $($crate::shared_thread_local!($($rest)*);)?
    };

    ($(#[$attr:meta])* $vis:vis static $name:ident : $t:ty = $init:expr $(; $($rest:tt)*)?) => {
        $crate::shared_thread_local!(@one $(#[$attr])* $vis $name: $t = $init);
        $($crate::shared_thread_local!($($rest)*);)?
    };

    (@one $(#[$attr:meta])* $vis:vis $name:ident : $t:ty = $init:expr) => {
        $(#[$attr])*
        $vis static $name: $crate::shared::LocalKey<$t> = {
            fn __init() -> $t {
                $init
            }
            $crate::shared::LocalKey::new(__init)
        };
    };
}
