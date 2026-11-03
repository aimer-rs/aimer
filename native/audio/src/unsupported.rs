//! Platforms without an audio backend: every load fails with
//! [`AudioError::Unsupported`], so call sites need no platform guard.

use crate::AudioError;
use crate::backend::{Backend, Voice, VoiceData};

pub(crate) struct UnsupportedBackend;

impl Backend for UnsupportedBackend {
    fn new() -> Self {
        Self
    }

    fn open(&mut self, _data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        Err(AudioError::Unsupported)
    }

    fn fetch(_url: &str) -> Result<Vec<u8>, AudioError> {
        Err(AudioError::Unsupported)
    }
}
