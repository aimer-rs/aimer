#![doc = include_str!("../README.md")]

// What both sides share: the commands, their wire format, and where an app listens.
pub mod actions;
mod command;
mod hello;
#[cfg(all(not(target_arch = "wasm32"), feature = "host"))]
mod motion;
#[cfg(all(not(target_arch = "wasm32"), feature = "host"))]
mod trackpad;
#[cfg(all(not(target_arch = "wasm32"), feature = "host"))]
mod typing;
pub mod env;
mod model;
#[cfg(all(not(target_arch = "wasm32"), any(feature = "client", feature = "host")))]
mod endpoint;
mod wire;

#[cfg(all(not(target_arch = "wasm32"), feature = "client"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "host"))]
mod platform;
#[cfg(all(not(target_arch = "wasm32"), feature = "host"))]
mod server;

#[cfg(all(not(target_arch = "wasm32"), feature = "client"))]
pub use client::{Client, Screenshot};
pub use command::{BackStep, Command, EditShortcut};
pub use hello::Role;
pub use model::{KeyAction, Modifiers, NamedKey, PointerButton, ScrollDeltaKind, TouchPhase};
#[cfg(all(not(target_arch = "wasm32"), any(feature = "client", feature = "host")))]
pub use endpoint::{Endpoint, InvalidEndpoint};
#[cfg(all(not(target_arch = "wasm32"), feature = "host"))]
pub use server::{Attached, Config, Server};
pub use wire::ParseError;
