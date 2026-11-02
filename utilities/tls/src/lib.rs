#![doc = include_str!("../README.md")]
// The per-thread table key in `shared` is the one place that has to use std's
// macro; this crate has no other std thread-locals.
#![allow(clippy::disallowed_macros)]

#[doc(hidden)]
pub mod shared;

pub use shared::{AccessError, HotGuard, LocalKey, Scope, hot, hot_supported, scope};

/// Declares thread-local statics that share one OS TLS key.
///
/// Accepts the same input as [`std::thread_local!`], including `const { .. }`
/// initializers, visibility, attributes and several statics per invocation.
/// The trailing `;` after the last static is optional.
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
#[macro_export]
macro_rules! thread_local {
    ($($(#[$attr:meta])* $vis:vis static $name:ident : $t:ty = $init:expr);+ $(;)?) => {
        $(
            $(#[$attr])*
            $vis static $name: $crate::LocalKey<$t> = {
                fn __init() -> $t {
                    $init
                }
                $crate::shared::LocalKey::new(__init)
            };
        )+
    };
    () => {};
}
