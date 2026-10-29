#![doc = include_str!("../README.md")]

mod codec;
mod identity;
mod registry;
mod schema;

pub use codec::*;
pub use identity::*;
pub use registry::*;
pub use schema::*;
#[doc(hidden)]
pub use aimer_anteros as __anteros;

#[doc(hidden)]
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "illumos",
))]
pub use linkme as __linkme;
