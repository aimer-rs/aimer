#![doc = include_str!("../README.md")]

mod backend;
mod engine;
mod handle;
mod model;
mod player;
mod sound;
mod state;

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
mod unsupported;

#[cfg(any(target_os = "ios", target_os = "macos"))]
use apple::AppleBackend as PlatformBackend;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use unsupported::UnsupportedBackend as PlatformBackend;

pub use model::{AudioError, AudioLoops, AudioSource, PlaybackState};
pub use player::AudioPlayer;
pub use sound::Sound;

/// Starts the audio engine now, so the first sound plays without the platform's
/// one-time start-up delay.
///
/// The first audio of a process pays for starting the system's audio stack; on
/// macOS that measured 0.5 to 3 s in a debug build. The engine starts by itself
/// when the first [`AudioPlayer`] or [`Sound`] is created, so call this earlier
/// only if you want the cost paid at app start-up instead of at that point.
/// Returns immediately; the work happens on the audio thread.
#[inline]
pub fn warm_up() {
    engine::start();
}

/// Whether this platform can play audio.
///
/// Where it returns `false`, every load fails with [`AudioError::Unsupported`]
/// and the controls do nothing, so call sites need no platform guard; use this
/// to hide audio controls instead.
#[inline]
pub const fn is_supported() -> bool {
    cfg!(any(target_os = "ios", target_os = "macos"))
}

#[cfg(test)]
mod backend_tests;
