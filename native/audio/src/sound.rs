use std::sync::Arc;
use std::time::Duration;

use crate::engine::Command;
use crate::handle::Handle;
use crate::state::Phase;
use crate::{AudioError, AudioSource, PlaybackState};

/// A short effect that can overlap itself: a click, a chime, a tap.
///
/// The file is read and decoded once on load, then every [`play`](Sound::play)
/// starts a fresh instance from memory, so a quick sequence of taps layers
/// instead of cutting each other off. Up to eight instances sound at once; a
/// ninth restarts the oldest. For long tracks with a playhead, use
/// [`AudioPlayer`](crate::AudioPlayer).
///
/// A `Sound` is a cheap handle; clones trigger the same effect. Dropping the
/// last clone cuts off any instance still sounding, so keep it alive as long as
/// the effect may play, for example in a widget's state.
///
/// # Examples
///
/// ```no_run
/// use aimer_audio::{AudioSource, Sound};
///
/// let tap = Sound::from_source(AudioSource::bundled("assets/tap.wav"));
/// tap.set_volume(0.5);
/// tap.play(); // plays as soon as the file is ready
/// ```
#[derive(Clone)]
pub struct Sound {
    handle: Arc<Handle>,
}

impl Default for Sound {
    fn default() -> Self {
        Self::new()
    }
}

impl Sound {
    /// Creates a sound with nothing loaded.
    pub fn new() -> Self {
        Self {
            handle: Arc::new(Handle::new_sound()),
        }
    }

    /// Creates a sound and starts loading `source`.
    pub fn from_source(source: AudioSource) -> Self {
        let sound = Self::new();
        sound.load(source);
        sound
    }

    /// Replaces the effect with `source`, cutting off any instance sounding.
    pub fn load(&self, source: AudioSource) {
        self.handle.shared.lock().begin_load();
        self.handle.send(|id| Command::Load { id, source });
    }

    /// Starts one more instance. Called while the file is still loading, it
    /// plays once on arrival. Does nothing before a load or after a failure.
    pub fn play(&self) {
        if matches!(
            self.handle.shared.lock().phase,
            Phase::Idle | Phase::Failed
        ) {
            return;
        }
        self.handle.send(|id| Command::Play { id });
    }

    /// Cuts off every instance that is sounding.
    pub fn stop(&self) {
        self.handle.send(|id| Command::Stop { id });
    }

    /// Sets the volume for all instances, `0.0` to `1.0`. Out-of-range values
    /// are clamped and `NaN` is ignored.
    pub fn set_volume(&self, volume: f32) {
        if volume.is_nan() {
            return;
        }
        let volume = volume.clamp(0.0, 1.0);
        self.handle.shared.lock().volume = volume;
        self.handle.send(|id| Command::SetVolume { id, volume });
    }

    /// The volume set with [`set_volume`](Self::set_volume).
    pub fn volume(&self) -> f32 {
        self.handle.shared.lock().volume
    }

    /// `Idle`, `Loading`, `Ready` (loaded), or `Failed`. Individual instances
    /// are not tracked.
    pub fn state(&self) -> PlaybackState {
        self.handle.shared.lock().state()
    }

    /// The effect's length, once loaded.
    pub fn duration(&self) -> Option<Duration> {
        let duration = self.handle.shared.lock().duration?;
        Duration::try_from_secs_f64(duration).ok()
    }

    /// Why loading failed, when [`state`](Self::state) is
    /// [`PlaybackState::Failed`].
    pub fn error(&self) -> Option<AudioError> {
        self.handle.shared.lock().error.clone()
    }
}
