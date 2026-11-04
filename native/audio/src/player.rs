use std::sync::Arc;
use std::time::Duration;

use crate::engine::{Command, clamp_seconds};
use crate::handle::Handle;
use crate::state::Phase;
use crate::{AudioError, AudioLoops, AudioSource, PlaybackState};

/// Plays one long track: music, a podcast, a voice message.
///
/// An `AudioPlayer` is a cheap handle; clones control the same track, so a
/// widget can keep one in its state and hand copies to its buttons. Dropping
/// the last clone stops the track.
///
/// Every control is safe to call from any thread, in any state, and never
/// blocks: loading and decoding happen off the calling thread. Controls that
/// make no sense yet (`play` before `load`) do nothing, and `play` while the
/// source is still loading starts it the moment it is ready.
///
/// `position` is published about every 30 ms while playing; read it from a
/// frame callback rather than expecting sample accuracy. For short effects that
/// must overlap and start instantly, use [`Sound`](crate::Sound) instead.
///
/// # Examples
///
/// ```no_run
/// use aimer_audio::{AudioLoops, AudioPlayer, AudioSource};
///
/// let player = AudioPlayer::from_source(AudioSource::bundled("assets/theme.mp3"));
/// player.set_loops(AudioLoops::Infinite);
/// player.set_volume(0.6);
/// player.play();
/// // ... later, from a button:
/// player.toggle();
/// ```
#[derive(Clone)]
pub struct AudioPlayer {
    handle: Arc<Handle>,
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioPlayer {
    /// Creates a player with nothing loaded.
    pub fn new() -> Self {
        Self {
            handle: Arc::new(Handle::new_track()),
        }
    }

    /// Creates a player and starts loading `source`.
    pub fn from_source(source: AudioSource) -> Self {
        let player = Self::new();
        player.load(source);
        player
    }

    /// Replaces the current track with `source`.
    ///
    /// Playback stops and the playhead resets; volume, mute and loops are kept.
    /// Call [`AudioPlayer::play`] afterwards to start it.
    pub fn load(&self, source: AudioSource) {
        self.handle.shared.lock().begin_load();
        self.handle.send(|id| Command::Load { id, source });
    }

    /// Starts, resumes, or (after the end) restarts playback.
    ///
    /// Does nothing before a source is loaded or after a failure.
    pub fn play(&self) {
        {
            let mut s = self.handle.shared.lock();
            if matches!(s.phase, Phase::Idle | Phase::Failed) {
                return;
            }
            s.want_play = true;
            s.started = true;
            if s.finished {
                s.finished = false;
                s.position = 0.0;
            }
        }
        self.handle.send(|id| Command::Play { id });
    }

    /// Pauses, keeping the playhead.
    pub fn pause(&self) {
        self.handle.shared.lock().want_play = false;
        self.handle.send(|id| Command::Pause { id });
    }

    /// Pauses when playing and plays otherwise.
    pub fn toggle(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Stops and rewinds to the start.
    pub fn stop(&self) {
        {
            let mut s = self.handle.shared.lock();
            s.want_play = false;
            s.started = false;
            s.finished = false;
            s.position = 0.0;
        }
        self.handle.send(|id| Command::Stop { id });
    }

    /// Moves the playhead, clamped to the track's length once that is known.
    ///
    /// Playing continues from the new position; a paused track stays paused.
    pub fn seek(&self, position: Duration) {
        let seconds = position.as_secs_f64();
        {
            let mut s = self.handle.shared.lock();
            s.position = match s.duration {
                Some(duration) => clamp_seconds(seconds, duration),
                None => seconds,
            };
            s.finished = false;
        }
        self.handle.send(|id| Command::Seek { id, seconds });
    }

    /// Sets the volume, `0.0` (silent) to `1.0` (full). Out-of-range values are
    /// clamped and `NaN` is ignored.
    pub fn set_volume(&self, volume: f32) {
        if volume.is_nan() {
            return;
        }
        let effective = {
            let mut s = self.handle.shared.lock();
            s.volume = volume.clamp(0.0, 1.0);
            s.effective_volume()
        };
        self.handle.send(|id| Command::SetVolume {
            id,
            volume: effective,
        });
    }

    /// Mutes or unmutes without forgetting the volume.
    pub fn set_muted(&self, muted: bool) {
        let effective = {
            let mut s = self.handle.shared.lock();
            s.muted = muted;
            s.effective_volume()
        };
        self.handle.send(|id| Command::SetVolume {
            id,
            volume: effective,
        });
    }

    /// Sets how many times the track plays; applies to the current and later
    /// tracks.
    pub fn set_loops(&self, loops: AudioLoops) {
        self.handle.shared.lock().loops = loops;
        self.handle.send(|id| Command::SetLoops { id, loops });
    }

    /// What the player is doing.
    pub fn state(&self) -> PlaybackState {
        self.handle.shared.lock().state()
    }

    /// Whether the player is, or is about to start, producing sound. A stream
    /// that is [buffering](Self::is_buffering) still counts: it resumes by
    /// itself.
    pub fn is_playing(&self) -> bool {
        matches!(self.state(), PlaybackState::Playing | PlaybackState::Buffering)
    }

    /// Whether playback is waiting for streamed data to arrive.
    pub fn is_buffering(&self) -> bool {
        self.state() == PlaybackState::Buffering
    }

    /// How far into the track audio is available without waiting for the
    /// network. For anything but an [`AudioSource::Stream`] that is the whole
    /// track as soon as it is loaded.
    pub fn buffered(&self) -> Duration {
        Duration::try_from_secs_f64(self.handle.shared.lock().buffered).unwrap_or_default()
    }

    /// The playhead.
    pub fn position(&self) -> Duration {
        Duration::try_from_secs_f64(self.handle.shared.lock().position).unwrap_or_default()
    }

    /// The track's length, once it is known. A stream reveals it after its
    /// first bytes arrive.
    pub fn duration(&self) -> Option<Duration> {
        let duration = self.handle.shared.lock().duration?;
        Duration::try_from_secs_f64(duration).ok()
    }

    /// Why loading or playback failed, when [`state`](Self::state) is
    /// [`PlaybackState::Failed`].
    pub fn error(&self) -> Option<AudioError> {
        self.handle.shared.lock().error.clone()
    }

    /// The volume set with [`set_volume`](Self::set_volume).
    pub fn volume(&self) -> f32 {
        self.handle.shared.lock().volume
    }

    /// Whether the player is muted.
    pub fn is_muted(&self) -> bool {
        self.handle.shared.lock().muted
    }

    /// The repeat setting.
    pub fn loops(&self) -> AudioLoops {
        self.handle.shared.lock().loops
    }
}
