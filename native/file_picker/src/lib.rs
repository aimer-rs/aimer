#![doc = include_str!("../README.md")]

mod error;
mod filter;
mod future;
mod mime;
mod picked;
mod request;

#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "android", test))]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod android_wire;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple_types;
// Plain Rust, built on the host so its parsing is tested without a Linux box.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod file_uri;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_arch = "wasm32"
)))]
mod unsupported;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "android")]
use android as backend;
#[cfg(target_os = "ios")]
use ios as backend;
#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_arch = "wasm32"
)))]
use unsupported as backend;
#[cfg(target_arch = "wasm32")]
use web as backend;
#[cfg(target_os = "windows")]
use windows as backend;

pub use error::{PickError, PickResult};
pub use filter::FileFilter;
pub use future::PickFuture;
pub use mime::{extension_for_mime, mime_for_extension};
pub use picked::PickedFile;
pub use request::FilePicker;
