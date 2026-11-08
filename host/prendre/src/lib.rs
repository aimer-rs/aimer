#![doc = include_str!("../README.md")]

#[cfg(not(target_arch = "wasm32"))]
mod client;
mod command;
#[cfg(not(target_arch = "wasm32"))]
mod endpoint;
#[cfg(not(target_arch = "wasm32"))]
mod server;
mod wire;

#[cfg(not(target_arch = "wasm32"))]
pub use client::Client;
pub use command::Command;
#[cfg(not(target_arch = "wasm32"))]
pub use endpoint::{Endpoint, InvalidEndpoint};
#[cfg(not(target_arch = "wasm32"))]
pub use server::{Config, Server};
pub use wire::ParseError;
