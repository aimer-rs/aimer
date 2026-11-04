#![doc = include_str!("../README.md")]

mod assets;
mod backend;
mod captions;
mod captions_view;
mod controls;
mod controls_state;
mod frame_watch;
mod model;
mod player;
mod player_view;
mod rail;
mod track_geometry;
mod widget;
#[cfg(test)]
mod test_support;

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(all(test, target_arch = "wasm32"))]
mod web_tests;
#[cfg(not(any(target_os = "ios", target_os = "macos", target_arch = "wasm32", target_os = "windows")))]
mod unsupported;

pub use captions::{CaptionError, CaptionSource, CaptionTrack, Chapter, Chapters, Cue};
pub use model::{PlaybackState, VideoError, VideoLoops, VideoSource};
pub use player::VideoPlayer;
pub use player_view::VideoPlayerView;
pub use captions_view::{CaptionStyle, Captions};
pub use controls::VideoControls;
pub use widget::{Video, VideoFit};

#[cfg(any(target_os = "ios", target_os = "macos"))]
use apple::open as platform_open;
#[cfg(target_arch = "wasm32")]
use web::open as platform_open;
#[cfg(target_os = "windows")]
use self::windows::open as platform_open;
#[cfg(not(any(target_os = "ios", target_os = "macos", target_arch = "wasm32", target_os = "windows")))]
use unsupported::open as platform_open;

/// Whether this platform has a video backend: macOS, iOS, Windows and the web for now.
///
/// Where it returns `false`, every load fails with [`VideoError::Unsupported`]
/// and the controls do nothing, so call sites need no platform guard; use this
/// to hide video controls instead.
#[inline]
pub const fn is_supported() -> bool {
    cfg!(any(target_os = "ios", target_os = "macos", target_arch = "wasm32", target_os = "windows"))
}
