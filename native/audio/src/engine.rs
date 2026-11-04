//! The audio engine: one thread that owns every platform voice.
//!
//! Handles never touch a voice. They write their intent into a [`Shared`]
//! snapshot and send a [`Command`]; the engine applies it to the voice it owns,
//! so voices need no `Send` bound and no lock. Slow work (reading a file,
//! downloading a URL) runs on a short-lived helper thread and comes back as
//! [`Command::Prepared`], so a slow source never stalls other sounds.
//!
//! While nothing plays the thread sleeps on the channel. While a track plays it
//! wakes every [`TICK`] to publish the playhead and to notice the end.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::backend::{Backend, Voice, VoiceData, is_http_url};
use crate::state::{Phase, Shared};
use crate::{AudioError, AudioLoops, AudioSource};

/// How often a playing track publishes its playhead.
pub(crate) const TICK: Duration = Duration::from_millis(30);
/// The most overlapping instances of one [`Sound`](crate::Sound); a further
/// trigger restarts the oldest.
pub(crate) const MAX_VOICES: usize = 8;

pub(crate) enum Command {
    NewTrack {
        id: u64,
        shared: Arc<Shared>,
    },
    NewSound {
        id: u64,
        shared: Arc<Shared>,
    },
    Load {
        id: u64,
        source: AudioSource,
    },
    /// A helper thread finished reading or downloading a source.
    Prepared {
        id: u64,
        generation: u64,
        result: Result<VoiceData, AudioError>,
    },
    Play {
        id: u64,
    },
    Pause {
        id: u64,
    },
    Stop {
        id: u64,
    },
    Seek {
        id: u64,
        seconds: f64,
    },
    SetVolume {
        id: u64,
        volume: f32,
    },
    SetLoops {
        id: u64,
        loops: AudioLoops,
    },
    Release {
        id: u64,
    },
}

/// A long-form player: one voice, with a playhead.
struct Track {
    shared: Arc<Shared>,
    /// Bumped by every load; a late [`Command::Prepared`] for an older load is
    /// dropped.
    generation: u64,
    /// What the current voice was opened from, kept to open a fresh voice when
    /// the track has to restart after reaching its end.
    data: Option<VoiceData>,
    voice: Option<Box<dyn Voice>>,
    /// The engine's own record of "should be producing sound", used to tell a
    /// pause from reaching the end.
    intent_play: bool,
    at_end: bool,
    volume: f32,
    /// Repeats after the first play; `-1` repeats forever.
    extra_loops: i64,
    /// Finite repeats still to come in the current run. Counted here rather
    /// than by the platform: `AVAudioPlayer` repeats a short clip (measured
    /// with 0.3 s) one time too few for a finite `numberOfLoops`. Only
    /// "forever" is delegated, which it loops gaplessly and correctly. Each
    /// repeat opens a fresh voice; see [`restart`].
    passes_left: i64,
    /// A seek asked for before the source finished opening.
    pending_seek: Option<f64>,
    /// Nothing left to report: the length is known and all of it is
    /// buffered. Until then even a paused track is ticked, because a stream,
    /// or a player that opens asynchronously, learns both over time.
    settled: bool,
}

impl Track {
    /// Whether the voice plays a URL while it downloads.
    fn streaming(&self) -> bool {
        matches!(self.data, Some(VoiceData::Stream(_)))
    }

    /// Restores the repeats of a fresh run.
    fn reset_passes(&mut self) {
        self.passes_left = passes(self.extra_loops, self.streaming());
    }
}

/// A short effect: a pool of voices over one shared copy of the data.
struct Sound {
    shared: Arc<Shared>,
    generation: u64,
    data: Option<VoiceData>,
    voices: Vec<Box<dyn Voice>>,
    next_steal: usize,
    volume: f32,
    /// `play` was called before the data arrived.
    play_when_ready: bool,
}

enum Entry {
    Track(Track),
    Sound(Sound),
}

pub(crate) struct Engine<B: Backend> {
    backend: B,
    tx: Sender<Command>,
    entries: HashMap<u64, Entry>,
}

impl<B: Backend> Engine<B> {
    pub(crate) fn new(backend: B, tx: Sender<Command>) -> Self {
        Self {
            backend,
            tx,
            entries: HashMap::new(),
        }
    }

    /// Whether any track is producing sound, or still learning its length or
    /// buffer, and needs ticking.
    pub(crate) fn needs_tick(&self) -> bool {
        self.entries.values().any(|entry| {
            matches!(entry, Entry::Track(t) if t.voice.is_some() && (t.intent_play || !t.settled))
        })
    }

    /// Runs until the process exits. The engine holds a sender of its own, so
    /// the channel never disconnects.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn run(&mut self, rx: Receiver<Command>) {
        loop {
            let first = if self.needs_tick() {
                match rx.recv_timeout(TICK) {
                    Ok(command) => Some(command),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match rx.recv() {
                    Ok(command) => Some(command),
                    Err(_) => return,
                }
            };
            if let Some(command) = first {
                self.handle(command);
                while let Ok(command) = rx.try_recv() {
                    self.handle(command);
                }
            }
            self.tick();
        }
    }

    pub(crate) fn handle(&mut self, command: Command) {
        match command {
            Command::NewTrack { id, shared } => {
                self.entries.insert(
                    id,
                    Entry::Track(Track {
                        shared,
                        generation: 0,
                        data: None,
                        voice: None,
                        intent_play: false,
                        at_end: false,
                        volume: 1.0,
                        extra_loops: 0,
                        passes_left: 0,
                        pending_seek: None,
                        settled: false,
                    }),
                );
            }
            Command::NewSound { id, shared } => {
                self.entries.insert(
                    id,
                    Entry::Sound(Sound {
                        shared,
                        generation: 0,
                        data: None,
                        voices: Vec::new(),
                        next_steal: 0,
                        volume: 1.0,
                        play_when_ready: false,
                    }),
                );
            }
            Command::Load { id, source } => self.load(id, source),
            Command::Prepared {
                id,
                generation,
                result,
            } => self.prepared(id, generation, result),
            Command::Play { id } => self.play(id),
            Command::Pause { id } => {
                if let Some(Entry::Track(track)) = self.entries.get_mut(&id) {
                    track.intent_play = false;
                    if let Some(voice) = track.voice.as_mut() {
                        voice.pause();
                    }
                }
            }
            Command::Stop { id } => match self.entries.get_mut(&id) {
                Some(Entry::Track(track)) => {
                    track.intent_play = false;
                    track.at_end = false;
                    track.pending_seek = None;
                    track.reset_passes();
                    if let Some(voice) = track.voice.as_mut() {
                        voice.stop();
                    }
                }
                Some(Entry::Sound(sound)) => {
                    sound.play_when_ready = false;
                    for voice in &mut sound.voices {
                        voice.stop();
                    }
                }
                None => {}
            },
            Command::Seek { id, seconds } => {
                if let Some(Entry::Track(track)) = self.entries.get_mut(&id) {
                    track.at_end = false;
                    match track.voice.as_mut() {
                        Some(voice) => {
                            let seconds = clamp_seconds(seconds, voice.duration());
                            voice.seek(seconds);
                        }
                        None => track.pending_seek = Some(seconds),
                    }
                }
            }
            Command::SetVolume { id, volume } => match self.entries.get_mut(&id) {
                Some(Entry::Track(track)) => {
                    track.volume = volume;
                    if let Some(voice) = track.voice.as_mut() {
                        voice.set_volume(volume);
                    }
                }
                Some(Entry::Sound(sound)) => {
                    sound.volume = volume;
                    for voice in &mut sound.voices {
                        voice.set_volume(volume);
                    }
                }
                None => {}
            },
            Command::SetLoops { id, loops } => {
                if let Some(Entry::Track(track)) = self.entries.get_mut(&id) {
                    track.extra_loops = loops.extra_plays();
                    track.reset_passes();
                    let native = native_loops(track.extra_loops, track.streaming());
                    if let Some(voice) = track.voice.as_mut() {
                        voice.set_extra_loops(native);
                    }
                }
            }
            Command::Release { id } => {
                self.entries.remove(&id);
            }
        }
    }

    /// Publishes the playhead of every playing track and notices the ones that
    /// ran out.
    pub(crate) fn tick(&mut self) {
        let backend = &mut self.backend;
        for entry in self.entries.values_mut() {
            let Entry::Track(track) = entry else { continue };
            let Some(voice) = track.voice.as_mut() else {
                continue;
            };
            voice.poll();
            if let Some(error) = voice.error() {
                // A player that died (a dropped connection, a corrupt frame)
                // stops, and that must not read as reaching the end.
                track.intent_play = false;
                track.settled = true;
                track.voice = None;
                track.shared.lock().fail(error);
                continue;
            }
            if !track.settled || track.intent_play {
                observe(track);
            }
            if !track.intent_play {
                continue;
            }
            let Some(voice) = track.voice.as_ref() else {
                continue;
            };
            if voice.is_playing() {
                track.shared.lock().position = voice.position();
            } else if track.passes_left > 0 {
                // A finite repeat: start the next pass. The gap is the time
                // since the pass ended (up to one `TICK`) plus opening a voice.
                track.passes_left -= 1;
                match restart(backend, track) {
                    Ok(()) => track.shared.lock().position = 0.0,
                    Err(error) => {
                        track.intent_play = false;
                        track.shared.lock().fail(error);
                    }
                }
            } else {
                track.intent_play = false;
                track.at_end = true;
                let duration = voice.duration();
                let end = if duration.is_finite() { duration } else { voice.position() };
                let mut snapshot = track.shared.lock();
                if snapshot.want_play {
                    snapshot.want_play = false;
                    snapshot.finished = true;
                    snapshot.position = end;
                }
            }
        }
    }

    fn load(&mut self, id: u64, source: AudioSource) {
        let (generation, want_bytes, shared) = match self.entries.get_mut(&id) {
            Some(Entry::Track(track)) => {
                track.generation += 1;
                track.data = None;
                track.voice = None;
                track.intent_play = false;
                track.at_end = false;
                track.pending_seek = None;
                track.settled = false;
                track.reset_passes();
                (track.generation, false, track.shared.clone())
            }
            Some(Entry::Sound(sound)) => {
                sound.generation += 1;
                sound.data = None;
                sound.voices.clear();
                sound.next_steal = 0;
                sound.play_when_ready = false;
                (sound.generation, true, sound.shared.clone())
            }
            None => return,
        };
        // A backend that needs no slow work (the web hands the browser a URL)
        // prepares in place, which also serves platforms without threads.
        if let Some(result) = B::prepare_now(&source, want_bytes) {
            self.prepared(id, generation, result);
            return;
        }
        let tx = self.tx.clone();
        let spawned = thread::Builder::new()
            .name("aimer-audio-load".into())
            .spawn(move || {
                let result = prepare::<B>(&source, want_bytes);
                let _ = tx.send(Command::Prepared {
                    id,
                    generation,
                    result,
                });
            });
        if let Err(error) = spawned {
            shared.lock().fail(AudioError::Io(error.to_string()));
        }
    }

    fn prepared(&mut self, id: u64, generation: u64, result: Result<VoiceData, AudioError>) {
        let backend = &mut self.backend;
        match self.entries.get_mut(&id) {
            Some(Entry::Track(track)) if track.generation == generation => {
                match result.and_then(|data| backend.open(&data).map(|voice| (data, voice))) {
                    Ok((data, mut voice)) => {
                        track.data = Some(data);
                        let streaming = track.streaming();
                        track.reset_passes();
                        voice.set_volume(track.volume);
                        voice.set_extra_loops(native_loops(track.extra_loops, streaming));
                        let duration = voice.duration();
                        if let Some(seconds) = track.pending_seek.take() {
                            voice.seek(clamp_seconds(seconds, duration));
                        }
                        track.shared.lock().phase = Phase::Loaded;
                        track.voice = Some(voice);
                        observe(track);
                        // A `play` that arrived while loading was recorded in
                        // `intent_play` by `Command::Play`.
                        if track.intent_play {
                            start_track(backend, track);
                        }
                    }
                    Err(error) => {
                        track.intent_play = false;
                        track.shared.lock().fail(error);
                    }
                }
            }
            Some(Entry::Sound(sound)) if sound.generation == generation => {
                match result.and_then(|data| backend.open_sound(&data).map(|voice| (data, voice))) {
                    Ok((data, mut voice)) => {
                        voice.set_volume(sound.volume);
                        let duration = voice.duration();
                        sound.data = Some(data);
                        sound.voices.push(voice);
                        {
                            let mut snapshot = sound.shared.lock();
                            snapshot.phase = Phase::Loaded;
                            snapshot.duration = Some(duration);
                        }
                        if std::mem::take(&mut sound.play_when_ready) {
                            trigger(backend, sound);
                        }
                    }
                    Err(error) => {
                        sound.play_when_ready = false;
                        sound.shared.lock().fail(error);
                    }
                }
            }
            // Released, or superseded by a newer load.
            _ => {}
        }
    }

    fn play(&mut self, id: u64) {
        let backend = &mut self.backend;
        match self.entries.get_mut(&id) {
            Some(Entry::Track(track)) => {
                track.intent_play = true;
                if track.voice.is_some() {
                    start_track(backend, track);
                }
            }
            Some(Entry::Sound(sound)) => trigger(backend, sound),
            None => {}
        }
    }
}

/// Starts or resumes a loaded track, restarting it if it had run out.
fn start_track<B: Backend>(backend: &mut B, track: &mut Track) {
    let started = if track.at_end {
        track.at_end = false;
        track.reset_passes();
        restart(backend, track)
    } else {
        match track.voice.as_mut() {
            Some(voice) => voice.play(),
            None => return,
        }
    };
    if let Err(error) = started {
        track.intent_play = false;
        track.shared.lock().fail(error);
    }
}

/// Replaces a track's voice with a fresh one from the same data and plays it.
///
/// A platform player that has run to its end is not reliably reusable:
/// `AVAudioPlayer` ignores `play` after a natural end, and a pause-first
/// workaround worked only some of the time when probed. A fresh player always
/// starts, so a track that must play again gets one.
fn restart<B: Backend>(backend: &mut B, track: &mut Track) -> Result<(), AudioError> {
    if track.streaming()
        && let Some(voice) = track.voice.as_mut()
    {
        // A stream's data is already buffered by the player, so rewind it
        // instead of opening a new connection.
        voice.seek(0.0);
        return voice.play();
    }
    let data = track
        .data
        .clone()
        .ok_or_else(|| AudioError::Playback("no audio is loaded".into()))?;
    let mut voice = backend.open(&data)?;
    voice.set_volume(track.volume);
    voice.set_extra_loops(native_loops(track.extra_loops, false));
    let result = voice.play();
    track.voice = Some(voice);
    result
}

/// Plays one more instance of a sound, reusing an idle voice, growing the pool
/// up to [`MAX_VOICES`], and then restarting the oldest.
fn trigger<B: Backend>(backend: &mut B, sound: &mut Sound) {
    let Some(data) = sound.data.clone() else {
        // Not loaded yet: remember one trigger and play it on arrival.
        sound.play_when_ready = true;
        return;
    };
    if let Some(idle) = sound.voices.iter().position(|voice| !voice.is_playing()) {
        replay(&mut sound.voices[idle], &sound.shared);
        return;
    }
    if sound.voices.len() < MAX_VOICES {
        match backend.open_sound(&data) {
            Ok(mut voice) => {
                voice.set_volume(sound.volume);
                replay(&mut voice, &sound.shared);
                sound.voices.push(voice);
            }
            Err(error) => sound.shared.lock().fail(error),
        }
        return;
    }
    let oldest = sound.next_steal % sound.voices.len();
    sound.next_steal = oldest + 1;
    replay(&mut sound.voices[oldest], &sound.shared);
}

fn replay(voice: &mut Box<dyn Voice>, shared: &Shared) {
    voice.stop();
    if let Err(error) = voice.play() {
        shared.lock().fail(error);
    }
}

/// The repeat count handed to the platform: only "forever" (`-1`) is native,
/// and not for a stream; every other repeat is driven by [`Engine::tick`].
fn native_loops(extra: i64, streaming: bool) -> i64 {
    if extra < 0 && !streaming { -1 } else { 0 }
}

/// How many repeats [`Engine::tick`] drives itself. "Forever" is the
/// platform's job except for a stream, which the engine rewinds without end.
fn passes(extra: i64, streaming: bool) -> i64 {
    match (extra < 0, streaming) {
        (true, true) => i64::MAX,
        (true, false) => 0,
        (false, _) => extra,
    }
}

/// Publishes what a track learns over time: its length, how much of it has
/// arrived, and whether it is waiting for more.
fn observe(track: &mut Track) {
    let Some(voice) = track.voice.as_ref() else {
        return;
    };
    let duration = voice.duration();
    let reported = voice.buffered();
    // A player that cannot tell (`NaN`) has certainly buffered what it played.
    let buffered = if reported.is_nan() { voice.position() } else { reported };
    {
        let mut snapshot = track.shared.lock();
        if duration.is_finite() {
            snapshot.duration = Some(duration);
        }
        snapshot.buffered = buffered;
        snapshot.buffering = voice.is_buffering();
    }
    // Once the length is known, an unknown buffer will not become known.
    track.settled = duration.is_finite() && (reported.is_nan() || reported >= duration);
}

/// Clamps a requested playhead into `0..=duration`; a NaN becomes the start.
/// A duration that is not known yet (`NaN`) only bounds the playhead below.
pub(crate) fn clamp_seconds(seconds: f64, duration: f64) -> f64 {
    if seconds.is_nan() {
        return 0.0;
    }
    if duration.is_nan() {
        return seconds.clamp(0.0, f64::MAX);
    }
    seconds.clamp(0.0, duration.max(0.0))
}

/// Reads or downloads `source` on a helper thread.
///
/// Tracks open a file by path so the platform can stream it from disk; sounds
/// (`want_bytes`) read the whole file once so every voice shares it.
fn prepare<B: Backend>(source: &AudioSource, want_bytes: bool) -> Result<VoiceData, AudioError> {
    match source {
        AudioSource::Stream(url) if !want_bytes => {
            if is_http_url(url) {
                Ok(VoiceData::Stream(url.clone()))
            } else {
                Err(AudioError::Network(format!("not an http(s) URL: {url}")))
            }
        }
        // A sound needs every byte to overlap itself, so it cannot stream.
        AudioSource::Url(url) | AudioSource::Stream(url) => {
            B::fetch(url).map(|bytes| VoiceData::Bytes(bytes.into()))
        }
        AudioSource::File(path) => from_path(path.clone(), want_bytes),
        AudioSource::Bundled(key) => {
            if let Some(resolved) = B::resolve_bundled(key, want_bytes) {
                return resolved;
            }
            let exe = std::env::current_exe().ok();
            let path = bundled_candidates(key, exe.as_deref())
                .into_iter()
                .find(|candidate| candidate.is_file())
                .ok_or_else(|| AudioError::NotFound(key.clone()))?;
            from_path(path, want_bytes)
        }
    }
}

fn from_path(path: PathBuf, want_bytes: bool) -> Result<VoiceData, AudioError> {
    if !path.is_file() {
        return Err(AudioError::NotFound(path.display().to_string()));
    }
    if want_bytes {
        let bytes = std::fs::read(&path).map_err(|e| AudioError::Io(e.to_string()))?;
        Ok(VoiceData::Bytes(bytes.into()))
    } else {
        Ok(VoiceData::Path(path))
    }
}

/// Where a bundled asset may live, in lookup order: the working directory
/// (development runs), the macOS bundle's `Resources`, then next to the
/// executable (iOS).
///
/// This mirrors the lookup `aimer_assets` uses for bundled images, so one
/// `Aimer.toml` entry resolves the same way for every kind of asset.
pub(crate) fn bundled_candidates(key: &str, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(key)];
    if let Some(dir) = exe.and_then(Path::parent) {
        if let Some(contents) = dir.parent() {
            candidates.push(contents.join("Resources").join(key));
        }
        candidates.push(dir.join(key));
    }
    candidates
}

/// The process-wide engine thread's inbox, starting the thread on first use.
#[cfg(not(target_arch = "wasm32"))]
fn sender() -> &'static Sender<Command> {
    static SENDER: OnceLock<Sender<Command>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let engine_tx = tx.clone();
        // If the spawn fails the closure, and with it the receiver, is
        // dropped, so every later `send` reports failure.
        let _ = thread::Builder::new()
            .name("aimer-audio".into())
            .spawn(move || {
                // Creating the backend warms the platform's audio system up,
                // so commands queue behind it instead of the first load.
                let backend = crate::PlatformBackend::new();
                Engine::new(backend, engine_tx).run(rx);
            });
        tx
    })
}

/// Starts the engine thread now rather than on the first handle.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn start() {
    sender();
}

/// Sends `command` to the engine thread, starting it on first use. Returns
/// `false` if the engine thread could not be started.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn send(command: Command) -> bool {
    sender().send(command).is_ok()
}

// The web has no threads: the engine runs on the page's thread instead.
#[cfg(target_arch = "wasm32")]
#[path = "engine_web.rs"]
mod web;
#[cfg(target_arch = "wasm32")]
pub(crate) use web::{send, start};

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
