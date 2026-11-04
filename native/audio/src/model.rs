//! The plain-data vocabulary of the audio crate: where a sound comes from, how
//! often it repeats, what it is doing, and why it failed.

use std::fmt;
use std::path::PathBuf;

/// Where the encoded audio comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioSource {
    /// A file registered in `Aimer.toml`, addressed by its project-relative
    /// path (the same key `AssetImage` uses). It is looked up in the working
    /// directory, then in the app bundle's resource directory.
    Bundled(String),
    /// A file on the host file system.
    File(PathBuf),
    /// An `http` or `https` URL. The whole file is downloaded before it plays,
    /// so seeking and looping are instant afterwards. For long remote audio use
    /// [`AudioSource::Stream`] instead.
    Url(String),
    /// An `http` or `https` URL played while it downloads.
    ///
    /// A [`Sound`](crate::Sound) cannot stream, so it downloads this source
    /// like [`AudioSource::Url`].
    Stream(String),
}

impl AudioSource {
    /// A bundled asset, by its project-relative path.
    #[inline]
    pub fn bundled(key: impl Into<String>) -> Self {
        Self::Bundled(key.into())
    }

    /// A file on the host file system.
    #[inline]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }

    /// A remote file, downloaded completely before it plays.
    #[inline]
    pub fn url(url: impl Into<String>) -> Self {
        Self::Url(url.into())
    }

    /// A remote file, played as soon as enough of it has arrived.
    #[inline]
    pub fn stream(url: impl Into<String>) -> Self {
        Self::Stream(url.into())
    }
}

/// How many times a track plays before it finishes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioLoops {
    /// Play once.
    #[default]
    Once,
    /// Play this many times in total. `Count(0)` and `Count(1)` play once.
    Count(u32),
    /// Repeat until paused or stopped.
    Infinite,
}

impl AudioLoops {
    /// The number of repeats after the first play; `-1` means forever.
    pub(crate) fn extra_plays(self) -> i64 {
        match self {
            Self::Once => 0,
            Self::Count(total) => i64::from(total.saturating_sub(1)),
            Self::Infinite => -1,
        }
    }
}

/// What a player or sound is doing right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    /// Nothing has been loaded.
    Idle,
    /// The source is being fetched or opened.
    Loading,
    /// Loaded, and not started yet (or stopped).
    Ready,
    /// Producing sound. A track asked to play while still loading counts as
    /// playing: it starts the moment it is ready.
    Playing,
    /// Asked to play, but waiting for streamed data to arrive. It resumes by
    /// itself, so it counts as playing for [`AudioPlayer::is_playing`](crate::AudioPlayer::is_playing).
    Buffering,
    /// Paused partway through.
    Paused,
    /// Reached the end and is not repeating.
    Finished,
    /// Loading or playback failed; see `error()`.
    Failed,
}

/// Why audio could not be loaded or played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// This platform has no audio backend yet.
    Unsupported,
    /// The bundled asset or file does not exist.
    NotFound(String),
    /// The file could not be read.
    Io(String),
    /// The download failed.
    Network(String),
    /// The data is not audio the platform can decode.
    Decode(String),
    /// The platform refused to start playback.
    Playback(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("audio playback is not supported on this platform"),
            Self::NotFound(what) => write!(f, "audio source not found: {what}"),
            Self::Io(why) => write!(f, "could not read audio: {why}"),
            Self::Network(why) => write!(f, "could not download audio: {why}"),
            Self::Decode(why) => write!(f, "could not decode audio: {why}"),
            Self::Playback(why) => write!(f, "could not play audio: {why}"),
        }
    }
}

impl std::error::Error for AudioError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loops_map_to_extra_plays_with_forever_as_minus_one() {
        assert_eq!(AudioLoops::Once.extra_plays(), 0);
        assert_eq!(AudioLoops::Count(0).extra_plays(), 0);
        assert_eq!(AudioLoops::Count(1).extra_plays(), 0);
        assert_eq!(AudioLoops::Count(3).extra_plays(), 2);
        assert_eq!(AudioLoops::Infinite.extra_plays(), -1);
    }

    #[test]
    fn a_huge_count_does_not_overflow() {
        assert_eq!(AudioLoops::Count(u32::MAX).extra_plays(), i64::from(u32::MAX) - 1);
    }
}
