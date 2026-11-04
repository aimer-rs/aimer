//! The seam between the platform-independent engine and a platform's player.
//!
//! The engine owns every `Voice` on one thread, so voices need not be `Send`.
//! Tests drive the engine with a fake backend and need no audio hardware.

use std::path::PathBuf;
use std::sync::Arc;

use crate::{AudioError, AudioSource};

/// Encoded audio, ready for a backend to open.
// Platforms without a backend never open the data, so never read it.
#[cfg_attr(not(any(target_os = "ios", target_os = "macos")), allow(dead_code))]
#[derive(Clone, Debug)]
pub(crate) enum VoiceData {
    /// A file the backend can stream from disk; used by tracks.
    Path(PathBuf),
    /// The whole encoded file, shared between the voices of one sound.
    Bytes(Arc<[u8]>),
    /// An `http(s)` URL the backend plays while it downloads; used by tracks.
    Stream(String),
    /// A bundled asset the backend opens itself, by its key, because it does
    /// not live on the file system (an Android APK).
    Asset(String),
}

/// One playable instance of loaded audio.
pub(crate) trait Voice {
    /// Length in seconds, or `NaN` while a stream has not revealed it yet.
    fn duration(&self) -> f64;
    /// Playhead in seconds.
    fn position(&self) -> f64;
    fn is_playing(&self) -> bool;
    /// Starts or resumes from the playhead.
    fn play(&mut self) -> Result<(), AudioError>;
    /// Stops producing sound and keeps the playhead.
    fn pause(&mut self);
    /// Stops producing sound and rewinds to the start.
    fn stop(&mut self);
    /// Moves the playhead; the engine has already clamped `seconds`.
    fn seek(&mut self, seconds: f64);
    /// Linear gain in `0.0..=1.0`.
    fn set_volume(&mut self, volume: f32);
    /// Repeats after the first play; `-1` repeats forever. Streams ignore it:
    /// the engine loops them by rewinding.
    fn set_extra_loops(&mut self, extra: i64);

    /// Whether playback is waiting for data. Only streams ever do.
    fn is_buffering(&self) -> bool {
        false
    }
    /// Seconds from the start that can play without waiting for the network.
    fn buffered(&self) -> f64 {
        self.duration()
    }
    /// Why the voice can no longer play, such as a dropped connection.
    fn error(&self) -> Option<AudioError> {
        None
    }
    /// Called on every engine tick before the voice is read, for players that
    /// apply deferred work (a seek asked for before they finished opening).
    fn poll(&mut self) {}
}

/// A platform's way of opening voices and downloading files.
pub(crate) trait Backend: 'static {
    /// Called once on the engine thread.
    fn new() -> Self
    where
        Self: Sized;

    /// Opens `data` as a paused voice for a track. Runs on the engine thread.
    fn open(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError>;

    /// Opens `data` as a paused voice for a sound: short, triggered often, and
    /// overlapping itself. Backends that decode themselves keep its samples
    /// across voices; the default opens it like a track.
    fn open_sound(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        self.open(data)
    }

    /// Prepares `source` without a helper thread, when that needs no slow
    /// work. `None` (the default) reads or downloads it on a helper thread.
    fn prepare_now(_source: &AudioSource, _want_bytes: bool) -> Option<Result<VoiceData, AudioError>>
    where
        Self: Sized,
    {
        None
    }

    /// Resolves a bundled asset that is not a plain file, on the helper
    /// thread. `None` (the default) looks it up on the file system.
    fn resolve_bundled(_key: &str, _want_bytes: bool) -> Option<Result<VoiceData, AudioError>>
    where
        Self: Sized,
    {
        None
    }

    /// Downloads `url`. Runs on a short-lived helper thread, so it may block.
    fn fetch(url: &str) -> Result<Vec<u8>, AudioError>
    where
        Self: Sized;
}

/// Whether `url` starts with `http://` or `https://`, in any letter case.
pub(crate) fn is_http_url(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}
