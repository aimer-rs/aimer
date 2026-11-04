#![doc = include_str!("../README.md")]

mod backend;
mod model;
mod player;
mod widget;
#[cfg(test)]
mod test_support;

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
mod unsupported;

pub use model::{PlaybackState, VideoError, VideoLoops, VideoSource};
pub use player::VideoPlayer;
pub use widget::{Video, VideoFit};

#[cfg(any(target_os = "ios", target_os = "macos"))]
use apple::open as platform_open;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use unsupported::open as platform_open;

/// Whether this platform has a video backend: macOS and iOS for now.
///
/// Where it returns `false`, every load fails with [`VideoError::Unsupported`]
/// and the controls do nothing, so call sites need no platform guard; use this
/// to hide video controls instead.
#[inline]
pub const fn is_supported() -> bool {
    cfg!(any(target_os = "ios", target_os = "macos"))
}
