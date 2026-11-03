//! State shared between a handle (any thread) and the engine thread.
//!
//! Handles write the intent behind a control straight away (`play` sets
//! `want_play` before the engine hears about it), so reading the state right
//! after a call never shows the previous answer. The engine writes only what
//! it observes: the load result, the duration, the playhead, and the end.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::{AudioError, AudioLoops, PlaybackState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Idle,
    Loading,
    Loaded,
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub phase: Phase,
    pub error: Option<AudioError>,
    /// The caller wants sound: set by `play`, cleared by `pause`, `stop`, the
    /// end of the track, and failures.
    pub want_play: bool,
    /// Played at least once since the last load or stop.
    pub started: bool,
    pub finished: bool,
    /// Seconds.
    pub position: f64,
    /// Seconds, once the source is open.
    pub duration: Option<f64>,
    pub volume: f32,
    pub muted: bool,
    pub loops: AudioLoops,
}

impl Snapshot {
    fn new() -> Self {
        Self {
            phase: Phase::Idle,
            error: None,
            want_play: false,
            started: false,
            finished: false,
            position: 0.0,
            duration: None,
            volume: 1.0,
            muted: false,
            loops: AudioLoops::Once,
        }
    }

    pub fn state(&self) -> PlaybackState {
        match self.phase {
            Phase::Failed => PlaybackState::Failed,
            Phase::Idle => PlaybackState::Idle,
            _ if self.want_play => PlaybackState::Playing,
            Phase::Loading => PlaybackState::Loading,
            Phase::Loaded if self.finished => PlaybackState::Finished,
            Phase::Loaded if self.started => PlaybackState::Paused,
            Phase::Loaded => PlaybackState::Ready,
        }
    }

    /// The gain the backend should apply.
    pub fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    /// Forgets the previous source and starts loading a new one.
    pub fn begin_load(&mut self) {
        self.phase = Phase::Loading;
        self.error = None;
        self.want_play = false;
        self.started = false;
        self.finished = false;
        self.position = 0.0;
        self.duration = None;
    }

    pub fn fail(&mut self, error: AudioError) {
        self.phase = Phase::Failed;
        self.error = Some(error);
        self.want_play = false;
    }
}

pub(crate) struct Shared(Mutex<Snapshot>);

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Snapshot::new())))
    }

    /// Locks the snapshot. A poisoned lock is recovered: the snapshot is plain
    /// data, and a panic elsewhere must not silence audio controls forever.
    pub fn lock(&self) -> MutexGuard<'_, Snapshot> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_before_the_source_is_ready_reads_as_playing() {
        let mut s = Snapshot::new();
        s.begin_load();
        assert_eq!(s.state(), PlaybackState::Loading);
        s.want_play = true;
        assert_eq!(s.state(), PlaybackState::Playing);
    }

    #[test]
    fn a_loaded_snapshot_walks_ready_playing_paused_finished() {
        let mut s = Snapshot::new();
        s.phase = Phase::Loaded;
        assert_eq!(s.state(), PlaybackState::Ready);
        s.want_play = true;
        s.started = true;
        assert_eq!(s.state(), PlaybackState::Playing);
        s.want_play = false;
        assert_eq!(s.state(), PlaybackState::Paused);
        s.finished = true;
        assert_eq!(s.state(), PlaybackState::Finished);
    }

    #[test]
    fn failure_wins_over_every_other_flag_and_stops_playing() {
        let mut s = Snapshot::new();
        s.phase = Phase::Loaded;
        s.want_play = true;
        s.fail(AudioError::Unsupported);
        assert_eq!(s.state(), PlaybackState::Failed);
        assert!(!s.want_play);
    }

    #[test]
    fn muting_silences_without_losing_the_volume() {
        let mut s = Snapshot::new();
        s.volume = 0.4;
        s.muted = true;
        assert_eq!(s.effective_volume(), 0.0);
        s.muted = false;
        assert_eq!(s.effective_volume(), 0.4);
    }
}
