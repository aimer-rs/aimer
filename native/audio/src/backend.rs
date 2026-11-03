//! The seam between the platform-independent engine and a platform's player.
//!
//! The engine owns every `Voice` on one thread, so voices need not be `Send`.
//! Tests drive the engine with a fake backend and need no audio hardware.

use std::path::PathBuf;
use std::sync::Arc;

use crate::AudioError;

/// Encoded audio, ready for a backend to open.
// Platforms without a backend never open the data, so never read it.
#[cfg_attr(not(any(target_os = "ios", target_os = "macos")), allow(dead_code))]
#[derive(Clone, Debug)]
pub(crate) enum VoiceData {
    /// A file the backend can stream from disk; used by tracks.
    Path(PathBuf),
    /// The whole encoded file, shared between the voices of one sound.
    Bytes(Arc<[u8]>),
}

/// One playable instance of loaded audio.
pub(crate) trait Voice {
    /// Length in seconds.
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
    /// Repeats after the first play; `-1` repeats forever.
    fn set_extra_loops(&mut self, extra: i64);
}

/// A platform's way of opening voices and downloading files.
pub(crate) trait Backend: 'static {
    /// Called once on the engine thread.
    fn new() -> Self
    where
        Self: Sized;

    /// Opens `data` as a paused voice. Runs on the engine thread.
    fn open(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError>;

    /// Downloads `url`. Runs on a short-lived helper thread, so it may block.
    fn fetch(url: &str) -> Result<Vec<u8>, AudioError>
    where
        Self: Sized;
}
