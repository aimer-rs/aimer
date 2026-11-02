#![doc = include_str!("../README.md")]

mod feedback;
mod pattern;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "android", target_os = "ios", target_os = "macos")))]
mod unsupported;
// The Android effect model is plain Rust; it is built on the host in tests so
// its timeline logic is checked without a device.
#[cfg(any(target_os = "android", test))]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod waveform;

#[cfg(target_os = "android")]
use android as backend;
#[cfg(target_os = "ios")]
use ios as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(not(any(target_os = "android", target_os = "ios", target_os = "macos")))]
use unsupported as backend;

pub use feedback::Haptics;
pub use pattern::{HapticEvent, HapticPattern, ImpactStyle, NotificationStyle};
