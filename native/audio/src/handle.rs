//! The reference-counted identity behind [`AudioPlayer`](crate::AudioPlayer)
//! and [`Sound`](crate::Sound): an engine id plus the shared snapshot.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::AudioError;
use crate::engine::{Command, send};
use crate::state::Shared;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Dropping the last clone releases the engine's voices, which stops playback.
pub(crate) struct Handle {
    pub id: u64,
    pub shared: Arc<Shared>,
}

impl Handle {
    pub fn new_track() -> Self {
        Self::create(|id, shared| Command::NewTrack { id, shared })
    }

    pub fn new_sound() -> Self {
        Self::create(|id, shared| Command::NewSound { id, shared })
    }

    fn create(make: impl FnOnce(u64, Arc<Shared>) -> Command) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let shared = Shared::new();
        let handle = Self {
            id,
            shared: shared.clone(),
        };
        if !send(make(id, shared)) {
            handle.fail_unavailable();
        }
        handle
    }

    /// Sends the command built from this handle's id; a dead engine marks the
    /// handle failed rather than leaving it loading forever.
    pub fn send(&self, make: impl FnOnce(u64) -> Command) {
        if !send(make(self.id)) {
            self.fail_unavailable();
        }
    }

    fn fail_unavailable(&self) {
        self.shared
            .lock()
            .fail(AudioError::Playback("the audio thread is not running".into()));
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = send(Command::Release { id: self.id });
    }
}
