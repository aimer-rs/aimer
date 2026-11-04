#![doc = include_str!("../README.md")]

mod backend;
mod engine;
mod handle;
mod model;
mod player;
mod sound;
mod state;

#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple;
#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple_stream;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod linux_alsa;
#[cfg(target_os = "linux")]
mod linux_decode;
#[cfg(target_os = "linux")]
mod linux_mix;
#[cfg(target_os = "linux")]
mod linux_net;
mod sniff;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "macos",
    target_os = "linux",
    target_os = "windows",
    target_arch = "wasm32"
)))]
mod unsupported;

#[cfg(target_os = "android")]
use android::AndroidBackend as PlatformBackend;
#[cfg(any(target_os = "ios", target_os = "macos"))]
use apple::AppleBackend as PlatformBackend;
#[cfg(target_os = "linux")]
use linux::LinuxBackend as PlatformBackend;
#[cfg(target_arch = "wasm32")]
use web::WebBackend as PlatformBackend;
#[cfg(target_os = "windows")]
use windows::WindowsBackend as PlatformBackend;
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "macos",
    target_os = "linux",
    target_os = "windows",
    target_arch = "wasm32"
)))]
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

/// Whether this platform has an audio backend.
///
/// It is `true` on macOS, iOS, Android, Windows, Linux and the web. Where it
/// returns `false`, every load fails with [`AudioError::Unsupported`] and the
/// controls do nothing, so call sites need no platform guard; use this to hide
/// audio controls instead.
///
/// A supported platform can still fail to play at run time: a Linux machine
/// without ALSA, or a browser that refuses audio before the user has
/// interacted with the page. Those failures arrive as the player's
/// [`error`](AudioPlayer::error).
#[inline]
pub const fn is_supported() -> bool {
    cfg!(any(
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows",
        target_arch = "wasm32"
    ))
}

#[cfg(test)]
mod backend_tests;
