//! The plain-data vocabulary of video playback.

use std::fmt;
use std::path::PathBuf;

/// Where the video comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoSource {
    /// A file registered in `Aimer.toml`, by its project-relative path (the
    /// key `AssetImage` uses). Looked up in the working directory, then in the
    /// app bundle's resources.
    Bundled(String),
    /// A file on the host file system.
    File(PathBuf),
    /// An `http` or `https` URL, played while it downloads. An HLS playlist
    /// (`.m3u8`) streams adaptively where the platform supports it.
    Url(String),
}

impl VideoSource {
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

    /// A remote file or HLS playlist, streamed.
    #[inline]
    pub fn url(url: impl Into<String>) -> Self {
        Self::Url(url.into())
    }
}

/// How many times a video plays before it finishes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoLoops {
    /// Play once.
    #[default]
    Once,
    /// Play this many times in total. `Count(0)` and `Count(1)` play once.
    Count(u32),
    /// Repeat until paused or stopped.
    Infinite,
}

impl VideoLoops {
    /// Repeats after the first pass; `u64::MAX` stands for forever.
    pub(crate) fn repeats(self) -> u64 {
        match self {
            Self::Once => 0,
            Self::Count(total) => u64::from(total.saturating_sub(1)),
            Self::Infinite => u64::MAX,
        }
    }
}

/// What a player is doing right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    /// Nothing has been loaded.
    Idle,
    /// The source is being opened: its length and size are not known yet.
    Loading,
    /// Loaded, and not started yet (or stopped).
    Ready,
    /// Playing.
    Playing,
    /// Asked to play, but waiting for data. It resumes by itself, so it counts
    /// as playing for [`VideoPlayer::is_playing`](crate::VideoPlayer::is_playing).
    Buffering,
    /// Paused partway through.
    Paused,
    /// Reached the end and is not repeating.
    Finished,
    /// Loading or playback failed; see `error()`.
    Failed,
}

/// Why a video could not be loaded or played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoError {
    /// This platform has no video backend yet.
    Unsupported,
    /// The bundled asset or file does not exist.
    NotFound(String),
    /// The download or stream failed.
    Network(String),
    /// The data is not video the platform can decode.
    Decode(String),
    /// The platform refused to play.
    Playback(String),
}

impl fmt::Display for VideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("video playback is not supported on this platform"),
            Self::NotFound(what) => write!(f, "video source not found: {what}"),
            Self::Network(why) => write!(f, "could not stream video: {why}"),
            Self::Decode(why) => write!(f, "could not decode video: {why}"),
            Self::Playback(why) => write!(f, "could not play video: {why}"),
        }
    }
}

impl std::error::Error for VideoError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loops_map_to_repeats_after_the_first_pass() {
        assert_eq!(VideoLoops::Once.repeats(), 0);
        assert_eq!(VideoLoops::Count(0).repeats(), 0);
        assert_eq!(VideoLoops::Count(1).repeats(), 0);
        assert_eq!(VideoLoops::Count(3).repeats(), 2);
        assert_eq!(VideoLoops::Infinite.repeats(), u64::MAX);
    }
}
