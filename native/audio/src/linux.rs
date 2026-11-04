//! Linux: Symphonia decodes, a mixer thread plays through ALSA.
//!
//! Linux has no system player to hand a file to, so this backend is a small
//! software player. One output thread owns the ALSA device and mixes every
//! playing voice into it; it closes nothing and burns nothing while idle, but
//! stops the device and sleeps. See [`crate::linux_mix`] for how voices are fed.
//!
//! The device is `default` (PipeWire or PulseAudio on a desktop), or whatever
//! `AIMER_AUDIO_ALSA_DEVICE` names.

use std::fs::File;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::AudioError;
use crate::backend::{Backend, Voice, VoiceData};
use crate::linux_alsa::Pcm;
use crate::linux_decode::{self, Source};
use crate::linux_mix::{CHANNELS, Feed, OUT_RATE, Pcm as Clip, VoiceShared, mix_voice, to_i16};
use crate::linux_net::{self, Download};

/// Frames mixed per write: 10 ms.
const PERIOD: usize = OUT_RATE as usize / 100;
/// Device buffering asked of ALSA.
const LATENCY_US: u32 = 40_000;
/// How far the mixer may run ahead of the wall clock. A real device blocks
/// long before this; a device that never blocks (ALSA's `null`) is paced by it.
const MAX_AHEAD: Duration = Duration::from_millis(60);

/// The ALSA device to open.
fn device() -> String {
    std::env::var("AIMER_AUDIO_ALSA_DEVICE").unwrap_or_else(|_| {
        // Tests play into ALSA's built-in `null` device: no hardware, no sound.
        if cfg!(test) { "null" } else { "default" }.into()
    })
}

/// The output thread's voice list.
struct Output {
    voices: Mutex<Vec<Arc<VoiceShared>>>,
    /// Wakes an idle output thread when a voice starts.
    wake: Condvar,
}

impl Output {
    fn start() -> Result<Arc<Self>, String> {
        let pcm = Pcm::open(&device(), OUT_RATE, CHANNELS, LATENCY_US)?;
        let output = Arc::new(Self {
            voices: Mutex::new(Vec::new()),
            wake: Condvar::new(),
        });
        let shared = output.clone();
        thread::Builder::new()
            .name("aimer-audio-output".into())
            .spawn(move || shared.run(pcm))
            .map_err(|error| error.to_string())?;
        Ok(output)
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<VoiceShared>>> {
        self.voices.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn add(&self, voice: Arc<VoiceShared>) {
        self.lock().push(voice);
    }

    fn notify(&self) {
        // Taking the lock orders this after the thread's check, so a start
        // between its check and its wait is not missed.
        drop(self.lock());
        self.wake.notify_all();
    }

    fn run(&self, mut pcm: Pcm) {
        // Reused every period: the loop never allocates.
        let mut active: Vec<Arc<VoiceShared>> = Vec::with_capacity(16);
        let mut mix = vec![0.0_f32; PERIOD * CHANNELS];
        let mut samples = vec![0_i16; PERIOD * CHANNELS];
        let mut running = false;
        let mut started = Instant::now();
        let mut written: u64 = 0;
        loop {
            {
                let mut voices = self.lock();
                loop {
                    // The engine and any decoder thread hold the others.
                    voices.retain(|voice| Arc::strong_count(voice) > 1);
                    if voices.iter().any(|voice| voice.playing.load(Ordering::Acquire)) {
                        break;
                    }
                    if running {
                        pcm.stop();
                        running = false;
                    }
                    voices = self.wake.wait(voices).unwrap_or_else(PoisonError::into_inner);
                }
                active.clear();
                active.extend(voices.iter().cloned());
            }
            if !running {
                pcm.prepare();
                running = true;
                started = Instant::now();
                written = 0;
            }
            mix.fill(0.0);
            for voice in &active {
                mix_voice(voice, &mut mix);
            }
            to_i16(&mix, &mut samples);
            if let Err(error) = pcm.write(&samples) {
                eprintln!("aimer_audio: ALSA write failed: {error}");
                thread::sleep(Duration::from_millis(10));
            }
            written += PERIOD as u64;
            let played = Duration::from_secs_f64(written as f64 / f64::from(OUT_RATE));
            if let Some(ahead) = played.checked_sub(started.elapsed() + MAX_AHEAD) {
                thread::sleep(ahead);
            }
        }
    }
}

/// Encoded data, its decoded clip, and the clip's length in seconds.
type CachedClip = (Weak<[u8]>, Weak<[f32]>, f64);

pub(crate) struct LinuxBackend {
    output: Option<Arc<Output>>,
    /// Decoded sound clips, by the encoded data they came from; the entry dies
    /// with the last voice of the sound.
    clips: Vec<CachedClip>,
}

impl LinuxBackend {
    /// The output, opened on first use so an app that never plays audio never
    /// holds the device. A failure is not kept: the device may come back.
    fn output(&mut self) -> Result<Arc<Output>, AudioError> {
        if let Some(output) = &self.output {
            return Ok(output.clone());
        }
        let output = Output::start()
            .map_err(|error| AudioError::Playback(format!("no audio output: {error}")))?;
        self.output = Some(output.clone());
        Ok(output)
    }

    fn clip(&mut self, bytes: &Arc<[u8]>) -> Result<(Clip, f64), AudioError> {
        self.clips.retain(|(data, _, _)| data.strong_count() > 0);
        for (data, pcm, duration) in &self.clips {
            if data.upgrade().is_some_and(|data| Arc::ptr_eq(&data, bytes))
                && let Some(pcm) = pcm.upgrade()
            {
                return Ok((pcm, *duration));
            }
        }
        let (pcm, duration) = linux_decode::decode_all(bytes.clone())?;
        self.clips.push((Arc::downgrade(bytes), Arc::downgrade(&pcm), duration));
        Ok((pcm, duration))
    }

    fn track(
        &mut self,
        duration: f64,
        download: Option<Arc<Download>>,
        open: impl FnOnce() -> Result<Source, AudioError> + Send + 'static,
    ) -> Result<Box<dyn Voice>, AudioError> {
        let output = self.output()?;
        let feed = Feed::Queue {
            queue: Default::default(),
            ended: false,
            seek: None,
        };
        let shared = VoiceShared::new(feed, duration);
        linux_decode::spawn(shared.clone(), open)?;
        output.add(shared.clone());
        Ok(Box::new(LinuxVoice {
            shared,
            output,
            download,
        }))
    }
}

impl Backend for LinuxBackend {
    fn new() -> Self {
        Self {
            output: None,
            clips: Vec::new(),
        }
    }

    fn open(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        match data {
            VoiceData::Path(path) => {
                let file = File::open(path).map_err(|error| match error.kind() {
                    std::io::ErrorKind::NotFound => AudioError::NotFound(path.display().to_string()),
                    _ => AudioError::Io(error.to_string()),
                })?;
                // A local file is probed here, so a file that is not audio
                // fails its load like on every other platform.
                let source = linux_decode::probe(Box::new(file), extension(path.as_os_str().to_str()))?;
                let duration = source.duration;
                self.track(duration, None, move || Ok(source))
            }
            VoiceData::Bytes(bytes) => {
                let source = linux_decode::probe_bytes(bytes.clone())?;
                let duration = source.duration;
                self.track(duration, None, move || Ok(source))
            }
            VoiceData::Stream(url) => {
                let download = Download::start(url.clone());
                let reader = download.reader();
                let extension = extension(Some(url)).map(str::to_owned);
                self.track(f64::NAN, Some(download), move || {
                    linux_decode::probe(Box::new(reader), extension.as_deref())
                })
            }
            VoiceData::Asset(key) => Err(AudioError::NotFound(key.clone())),
        }
    }

    fn open_sound(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        let VoiceData::Bytes(bytes) = data else {
            return self.open(data);
        };
        let output = self.output()?;
        let (pcm, duration) = self.clip(bytes)?;
        let shared = VoiceShared::new(Feed::Clip { pcm, cursor: 0 }, duration);
        output.add(shared.clone());
        Ok(Box::new(LinuxVoice {
            shared,
            output,
            download: None,
        }))
    }

    fn fetch(url: &str) -> Result<Vec<u8>, AudioError> {
        linux_net::fetch(url)
    }
}

/// The file extension of a path or URL (ignoring a URL's query), as a hint for
/// probing.
fn extension(name: Option<&str>) -> Option<&str> {
    let name = name?;
    let name = name.split(['?', '#']).next().unwrap_or(name);
    let file = name.rsplit('/').next()?;
    let (_, extension) = file.rsplit_once('.')?;
    (!extension.is_empty()).then_some(extension)
}

struct LinuxVoice {
    shared: Arc<VoiceShared>,
    output: Arc<Output>,
    /// A stream's download, for its progress and its errors.
    download: Option<Arc<Download>>,
}

impl Drop for LinuxVoice {
    fn drop(&mut self) {
        self.shared.playing.store(false, Ordering::Release);
        self.shared.lock().released = true;
        self.shared.wake.notify_all();
        if let Some(download) = &self.download {
            download.cancel();
        }
    }
}

impl Voice for LinuxVoice {
    fn duration(&self) -> f64 {
        self.shared.lock().duration
    }

    fn position(&self) -> f64 {
        let body = self.shared.lock();
        let seconds = body.frame as f64 / f64::from(OUT_RATE);
        if body.looping && body.duration.is_finite() && body.duration > 0.0 {
            seconds % body.duration
        } else {
            seconds
        }
    }

    fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    fn play(&mut self) -> Result<(), AudioError> {
        self.shared.playing.store(true, Ordering::Release);
        self.output.notify();
        Ok(())
    }

    fn pause(&mut self) {
        self.shared.playing.store(false, Ordering::Release);
    }

    fn stop(&mut self) {
        self.shared.playing.store(false, Ordering::Release);
        self.shared.seek(0.0);
    }

    fn seek(&mut self, seconds: f64) {
        self.shared.seek(seconds);
    }

    fn set_volume(&mut self, volume: f32) {
        self.shared.set_volume(volume);
    }

    fn set_extra_loops(&mut self, extra: i64) {
        // Finite repeats are the engine's; only "forever" is native here.
        self.shared.lock().looping = extra < 0;
    }

    fn is_buffering(&self) -> bool {
        self.is_playing() && self.shared.lock().starved
    }

    fn buffered(&self) -> f64 {
        let duration = self.duration();
        let Some(download) = &self.download else {
            return duration;
        };
        match download.progress() {
            Some(progress) if duration.is_finite() => duration * progress,
            // Length unknown: all that is certain is what has played.
            _ => self.position(),
        }
    }

    fn error(&self) -> Option<AudioError> {
        if let Some(error) = self.download.as_ref().and_then(|download| download.error()) {
            return Some(AudioError::Network(error));
        }
        self.shared.lock().error.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::extension;

    #[test]
    fn the_probe_hint_comes_from_the_file_name_and_ignores_queries() {
        assert_eq!(extension(Some("assets/theme.mp3")), Some("mp3"));
        assert_eq!(extension(Some("https://x.test/a/b.ogg?token=1#t")), Some("ogg"));
        assert_eq!(extension(Some("https://x.test/stream")), None);
        assert_eq!(extension(Some("https://x.test.dir/file")), None);
        assert_eq!(extension(Some("trailing.")), None);
        assert_eq!(extension(None), None);
    }
}
