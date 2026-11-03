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
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use crate::backend::{Backend, Voice, VoiceData};
use crate::state::{Phase, Shared};
use crate::{AudioError, AudioLoops, AudioSource};

/// How often a playing track publishes its playhead.
const TICK: Duration = Duration::from_millis(30);
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

    /// Whether any track is producing sound and needs ticking.
    pub(crate) fn needs_tick(&self) -> bool {
        self.entries
            .values()
            .any(|entry| matches!(entry, Entry::Track(t) if t.intent_play && t.voice.is_some()))
    }

    /// Runs until the process exits. The engine holds a sender of its own, so
    /// the channel never disconnects.
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
                    track.passes_left = track.extra_loops.max(0);
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
                    track.passes_left = track.extra_loops.max(0);
                    if let Some(voice) = track.voice.as_mut() {
                        voice.set_extra_loops(native_loops(track.extra_loops));
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
                let mut snapshot = track.shared.lock();
                if snapshot.want_play {
                    snapshot.want_play = false;
                    snapshot.finished = true;
                    snapshot.position = duration;
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
                track.passes_left = track.extra_loops.max(0);
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
                        voice.set_volume(track.volume);
                        voice.set_extra_loops(native_loops(track.extra_loops));
                        let duration = voice.duration();
                        if let Some(seconds) = track.pending_seek.take() {
                            voice.seek(clamp_seconds(seconds, duration));
                        }
                        {
                            let mut snapshot = track.shared.lock();
                            snapshot.phase = Phase::Loaded;
                            snapshot.duration = Some(duration);
                        }
                        track.data = Some(data);
                        track.voice = Some(voice);
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
                match result.and_then(|data| backend.open(&data).map(|voice| (data, voice))) {
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
        track.passes_left = track.extra_loops.max(0);
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
    let data = track
        .data
        .clone()
        .ok_or_else(|| AudioError::Playback("no audio is loaded".into()))?;
    let mut voice = backend.open(&data)?;
    voice.set_volume(track.volume);
    voice.set_extra_loops(native_loops(track.extra_loops));
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
        match backend.open(&data) {
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

/// The repeat count handed to the platform: only "forever" (`-1`) is native;
/// finite repeats are driven by [`Engine::tick`].
fn native_loops(extra: i64) -> i64 {
    if extra < 0 { -1 } else { 0 }
}

/// Clamps a requested playhead into `0..=duration`; a NaN becomes the start.
pub(crate) fn clamp_seconds(seconds: f64, duration: f64) -> f64 {
    if seconds.is_nan() {
        return 0.0;
    }
    seconds.clamp(0.0, duration.max(0.0))
}

/// Reads or downloads `source` on a helper thread.
///
/// Tracks open a file by path so the platform can stream it from disk; sounds
/// (`want_bytes`) read the whole file once so every voice shares it.
fn prepare<B: Backend>(source: &AudioSource, want_bytes: bool) -> Result<VoiceData, AudioError> {
    match source {
        AudioSource::Url(url) => B::fetch(url).map(|bytes| VoiceData::Bytes(bytes.into())),
        AudioSource::File(path) => from_path(path.clone(), want_bytes),
        AudioSource::Bundled(key) => {
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
pub(crate) fn start() {
    sender();
}

/// Sends `command` to the engine thread, starting it on first use. Returns
/// `false` if the engine thread could not be started.
pub(crate) fn send(command: Command) -> bool {
    sender().send(command).is_ok()
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
