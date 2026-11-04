//! Windows: WinRT `Windows.Media.Playback.MediaPlayer`.
//!
//! One `MediaPlayer` is one voice. It plays files, in-memory data and `http(s)`
//! URLs (progressively) alike, decoding with Media Foundation. A player opens
//! its source asynchronously, so its length arrives on a later engine tick, a
//! seek asked for while it opens waits in [`Voice::poll`], and a failure to
//! decode surfaces through [`Voice::error`] rather than failing the load.
//!
//! Players are created on the engine thread, which joins the multithreaded
//! apartment, and are only used there. Each opts out of the system media
//! controls: an app that wants them integrates them itself.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::Core::MediaSource;
use windows::Media::Playback::{
    IMediaPlaybackSource, MediaPlaybackSession, MediaPlaybackState, MediaPlayer,
    MediaPlayerFailedEventArgs,
};
use windows::Storage::StorageFile;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
use windows::Web::Http::HttpClient;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{HSTRING, IInspectable, Interface, Ref};

use crate::AudioError;
use crate::backend::{Backend, Voice, VoiceData};
use crate::sniff::audio_mime;

/// `TimeSpan` ticks per second (100 ns each).
const TICKS: f64 = 10_000_000.0;

fn describe(error: windows::core::Error) -> String {
    error.message().to_string()
}

fn playback(error: windows::core::Error) -> AudioError {
    AudioError::Playback(describe(error))
}

pub(crate) struct WindowsBackend;

impl Backend for WindowsBackend {
    fn new() -> Self {
        // SAFETY: called once, on the engine thread that owns every player.
        // Failing means the thread already joined an apartment, which is fine.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        Self
    }

    fn open(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        let source = match data {
            VoiceData::Path(path) => from_file(path)?,
            VoiceData::Bytes(bytes) => from_bytes(bytes)?,
            VoiceData::Stream(url) => {
                let uri = Uri::CreateUri(&HSTRING::from(url.as_str()))
                    .map_err(|error| AudioError::Network(describe(error)))?;
                MediaSource::CreateFromUri(&uri).map_err(playback)?
            }
            VoiceData::Asset(key) => return Err(AudioError::NotFound(key.clone())),
        };
        WindowsVoice::new(&source).map(|voice| Box::new(voice) as Box<dyn Voice>)
    }

    fn fetch(url: &str) -> Result<Vec<u8>, AudioError> {
        let network = |error: windows::core::Error| AudioError::Network(describe(error));
        // `fetch` runs on a helper thread; it joins the apartment as well.
        // SAFETY: as in `new`; an apartment already joined is fine.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let uri = Uri::CreateUri(&HSTRING::from(url)).map_err(network)?;
        let buffer = HttpClient::new()
            .and_then(|client| client.GetBufferAsync(&uri))
            .and_then(|operation| operation.join())
            .map_err(network)?;
        let reader = DataReader::FromBuffer(&buffer).map_err(network)?;
        let mut bytes = vec![0; reader.UnconsumedBufferLength().map_err(network)? as usize];
        reader.ReadBytes(&mut bytes).map_err(network)?;
        Ok(bytes)
    }
}

fn from_file(path: &Path) -> Result<MediaSource, AudioError> {
    // `StorageFile` wants an absolute path with backslashes.
    let absolute = std::fs::canonicalize(path).map_err(|error| AudioError::Io(error.to_string()))?;
    let text = absolute.to_string_lossy();
    // `canonicalize` yields a verbatim `\\?\` path, which WinRT rejects.
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(text))
        .and_then(|operation| operation.join())
        .map_err(|error| AudioError::Io(describe(error)))?;
    MediaSource::CreateFromStorageFile(&file).map_err(playback)
}

fn from_bytes(bytes: &[u8]) -> Result<MediaSource, AudioError> {
    let stream = InMemoryRandomAccessStream::new().map_err(playback)?;
    let buffer = CryptographicBuffer::CreateFromByteArray(bytes).map_err(playback)?;
    stream
        .WriteAsync(&buffer)
        .and_then(|operation| operation.join())
        .map_err(playback)?;
    stream.Seek(0).map_err(playback)?;
    let mime = HSTRING::from(audio_mime(bytes).unwrap_or(""));
    MediaSource::CreateFromStream(&stream, &mime).map_err(playback)
}

/// What the player's event handlers report, from a system thread.
#[derive(Default)]
struct Events {
    ended: AtomicBool,
    error: Mutex<Option<String>>,
}

struct WindowsVoice {
    player: MediaPlayer,
    session: MediaPlaybackSession,
    events: Arc<Events>,
    ended_token: i64,
    failed_token: i64,
    /// Whether the engine last asked this voice to play. The player's own
    /// state lags a `Play` call, so it cannot tell a start from the end.
    intent: bool,
    /// A seek asked for before the source opened; applied by `poll`.
    pending_seek: Option<f64>,
}

impl WindowsVoice {
    fn new(source: &MediaSource) -> Result<Self, AudioError> {
        let player = MediaPlayer::new().map_err(playback)?;
        player.SetAutoPlay(false).map_err(playback)?;
        // Effects and in-app music must not take over the media keys.
        player
            .CommandManager()
            .and_then(|manager| manager.SetIsEnabled(false))
            .map_err(playback)?;
        let events = Arc::new(Events::default());
        let ended = events.clone();
        let ended_token = player
            .MediaEnded(&TypedEventHandler::new(
                move |_: Ref<MediaPlayer>, _: Ref<IInspectable>| {
                    ended.ended.store(true, Ordering::Release);
                    Ok(())
                },
            ))
            .map_err(playback)?;
        let failed = events.clone();
        let failed_token = player
            .MediaFailed(&TypedEventHandler::new(
                move |_: Ref<MediaPlayer>, args: Ref<MediaPlayerFailedEventArgs>| {
                    let message = args
                        .as_ref()
                        .and_then(|args| args.ErrorMessage().ok())
                        .map(|message| message.to_string())
                        .filter(|message| !message.is_empty())
                        .unwrap_or_else(|| "the media could not be played".into());
                    *failed.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
                    Ok(())
                },
            ))
            .map_err(playback)?;
        let playable: IMediaPlaybackSource = source.cast().map_err(playback)?;
        player.SetSource(&playable).map_err(playback)?;
        let session = player.PlaybackSession().map_err(playback)?;
        Ok(Self {
            player,
            session,
            events,
            ended_token,
            failed_token,
            intent: false,
            pending_seek: None,
        })
    }

    fn state(&self) -> MediaPlaybackState {
        self.session.PlaybackState().unwrap_or(MediaPlaybackState::None)
    }

    fn opened(&self) -> bool {
        !matches!(self.state(), MediaPlaybackState::None | MediaPlaybackState::Opening)
    }

    fn set_position(&self, seconds: f64) {
        let ticks = (seconds.max(0.0) * TICKS) as i64;
        let _ = self.session.SetPosition(TimeSpan { Duration: ticks });
    }
}

impl Drop for WindowsVoice {
    fn drop(&mut self) {
        let _ = self.player.RemoveMediaEnded(self.ended_token);
        let _ = self.player.RemoveMediaFailed(self.failed_token);
        let _ = self.player.Close();
    }
}

impl Voice for WindowsVoice {
    fn duration(&self) -> f64 {
        if !self.opened() {
            return f64::NAN;
        }
        match self.session.NaturalDuration() {
            // A live stream reports no length.
            Ok(span) if span.Duration > 0 => span.Duration as f64 / TICKS,
            _ => f64::NAN,
        }
    }

    fn position(&self) -> f64 {
        if let Some(seconds) = self.pending_seek {
            return seconds;
        }
        self.session
            .Position()
            .map_or(0.0, |span| (span.Duration as f64 / TICKS).max(0.0))
    }

    fn is_playing(&self) -> bool {
        self.intent && !self.events.ended.load(Ordering::Acquire)
    }

    fn play(&mut self) -> Result<(), AudioError> {
        self.events.ended.store(false, Ordering::Release);
        self.intent = true;
        // Before the source opens the player starts as soon as it is ready.
        self.player.Play().map_err(playback)
    }

    fn pause(&mut self) {
        self.intent = false;
        let _ = self.player.Pause();
    }

    fn stop(&mut self) {
        self.pause();
        self.seek(0.0);
    }

    fn seek(&mut self, seconds: f64) {
        self.events.ended.store(false, Ordering::Release);
        if self.opened() {
            self.set_position(seconds);
        } else {
            self.pending_seek = Some(seconds);
        }
    }

    fn set_volume(&mut self, volume: f32) {
        let _ = self.player.SetVolume(f64::from(volume));
    }

    fn set_extra_loops(&mut self, extra: i64) {
        let _ = self.player.SetIsLoopingEnabled(extra < 0);
    }

    fn is_buffering(&self) -> bool {
        self.intent
            && matches!(self.state(), MediaPlaybackState::Buffering | MediaPlaybackState::Opening)
    }

    fn buffered(&self) -> f64 {
        let duration = self.duration();
        if !duration.is_finite() {
            return self.position();
        }
        // A local file reports 1.0 at once; a stream counts up.
        duration * self.session.DownloadProgress().unwrap_or(0.0).clamp(0.0, 1.0)
    }

    fn error(&self) -> Option<AudioError> {
        let error = self.events.error.lock().unwrap_or_else(PoisonError::into_inner);
        error.clone().map(AudioError::Decode)
    }

    fn poll(&mut self) {
        if self.pending_seek.is_some() && self.opened() {
            let seconds = self.pending_seek.take().unwrap_or(0.0);
            self.set_position(seconds);
        }
    }
}
