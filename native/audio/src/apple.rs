//! macOS and iOS: `AVAudioPlayer`.
//!
//! One `AVAudioPlayer` is one voice. Tracks open it from a file URL so the
//! system streams from disk; sounds open it from shared in-memory data.
//! `AVAudioPlayer` has no progressive-download mode, so a [`fetch`] of a URL
//! downloads the whole file first.
//!
//! Every object here is created and used on the engine thread only.
//!
//! [`fetch`]: Backend::fetch

use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_avf_audio::AVAudioPlayer;
use objc2_foundation::{NSData, NSError, NSString, NSURL};

use crate::AudioError;
use crate::backend::{Backend, Voice, VoiceData};

pub(crate) struct AppleBackend;

impl Backend for AppleBackend {
    fn new() -> Self {
        #[cfg(target_os = "ios")]
        configure_session();
        let mut backend = Self;
        // The first `AVAudioPlayer` of a process starts Core Audio, which
        // measured 0.5 to 3 s (initialisation plus `prepareToPlay`) in a debug
        // build on a Mac. Opening a clip of a few samples pays that here, on the
        // engine thread at start-up, instead of on the first real load.
        drop(backend.open(&VoiceData::Bytes(warm_up_clip().into())));
        backend
    }

    fn open(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        // SAFETY: `alloc` yields an uninitialised instance that the init call
        // consumes; the returned player is a retained, fully initialised object.
        let (player, data) = match data {
            VoiceData::Path(path) => {
                let path = NSString::from_str(&path.to_string_lossy());
                let url = NSURL::fileURLWithPath(&path);
                let player = unsafe {
                    AVAudioPlayer::initWithContentsOfURL_error(AVAudioPlayer::alloc(), &url)
                };
                (player, None)
            }
            VoiceData::Bytes(bytes) => {
                let data = NSData::with_bytes(bytes);
                let player =
                    unsafe { AVAudioPlayer::initWithData_error(AVAudioPlayer::alloc(), &data) };
                (player, Some(data))
            }
        };
        let player = player.map_err(|error| AudioError::Decode(describe(&error)))?;
        // Preload buffers so the first `play` starts without a hiccup. A
        // failure here is not fatal; `play` reports it.
        unsafe { player.prepareToPlay() };
        Ok(Box::new(AppleVoice {
            player,
            _data: data,
        }))
    }

    fn fetch(url: &str) -> Result<Vec<u8>, AudioError> {
        let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(AudioError::Network(format!("not an http(s) URL: {url}")));
        }
        let ns_url = NSURL::URLWithString(&NSString::from_str(url))
            .ok_or_else(|| AudioError::Network(format!("invalid URL: {url}")))?;
        NSData::dataWithContentsOfURL(&ns_url)
            .map(|data| data.to_vec())
            .ok_or_else(|| AudioError::Network(format!("download failed: {url}")))
    }
}

/// A few milliseconds of silence as a mono, 16-bit, 8 kHz WAV file.
fn warm_up_clip() -> Vec<u8> {
    const RATE: u32 = 8000;
    const SAMPLES: u32 = 80;
    const DATA_LEN: u32 = SAMPLES * 2;
    let mut wav = Vec::with_capacity(44 + DATA_LEN as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + DATA_LEN).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2_u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&DATA_LEN.to_le_bytes());
    wav.resize(44 + DATA_LEN as usize, 0);
    wav
}

/// Lets playback start with the silent switch on and keeps it going when other
/// audio would normally duck it; without a session iOS plays nothing audible
/// for an app that does not set one.
#[cfg(target_os = "ios")]
fn configure_session() {
    use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryPlayback};
    // SAFETY: the shared session is a process-wide singleton, and the category
    // constant is a valid, immutable `NSString`.
    unsafe {
        let session = AVAudioSession::sharedInstance();
        if let Some(category) = AVAudioSessionCategoryPlayback {
            let _ = session.setCategory_error(category);
        }
        let _ = session.setActive_error(true);
    }
}

fn describe(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

struct AppleVoice {
    player: Retained<AVAudioPlayer>,
    /// The player reads from this buffer for as long as it lives.
    _data: Option<Retained<NSData>>,
}

// SAFETY (all `unsafe` blocks below): `player` is a valid, initialised
// `AVAudioPlayer`, and the engine thread is the only one that ever touches it.
impl Voice for AppleVoice {
    fn duration(&self) -> f64 {
        unsafe { self.player.duration() }
    }

    fn position(&self) -> f64 {
        unsafe { self.player.currentTime() }
    }

    fn is_playing(&self) -> bool {
        unsafe { self.player.isPlaying() }
    }

    fn play(&mut self) -> Result<(), AudioError> {
        if unsafe { self.player.play() } {
            Ok(())
        } else {
            Err(AudioError::Playback("AVAudioPlayer refused to start".into()))
        }
    }

    fn pause(&mut self) {
        unsafe { self.player.pause() }
    }

    fn stop(&mut self) {
        // `pause` plus a rewind rather than `stop`: `stop` releases the
        // player's buffers, and the next `play` would pay for preparing again.
        unsafe {
            self.player.pause();
            self.player.setCurrentTime(0.0);
        }
    }

    fn seek(&mut self, seconds: f64) {
        unsafe { self.player.setCurrentTime(seconds) }
    }

    fn set_volume(&mut self, volume: f32) {
        unsafe { self.player.setVolume(volume) }
    }

    fn set_extra_loops(&mut self, extra: i64) {
        unsafe { self.player.setNumberOfLoops(extra as isize) }
    }
}
