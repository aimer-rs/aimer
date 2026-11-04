//! Linux: decoding with Symphonia.
//!
//! [`decode_all`] turns a sound's file into one [`Pcm`] clip. A track gets a
//! decoder thread ([`spawn`]) that keeps its queue about [`QUEUE_FRAMES`]
//! ahead of the playhead, serves seeks, and loops at the end when asked.

use std::io::{self, Cursor};
use std::sync::Arc;
use std::thread;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::{Error, SeekErrorKind};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase};

use crate::AudioError;
use crate::linux_mix::{CHANNELS, Feed, Pcm, QUEUE_FRAMES, Resampler, VoiceShared};

/// An opened file: its container reader and its audio track's decoder.
pub(crate) struct Source {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    rate: u32,
    channels: usize,
    /// Seconds, or `NaN` when the container does not say.
    pub duration: f64,
    /// Source frames still to drop after an accurate seek landed early.
    skip: usize,
}

/// What [`Source::next`] produced.
enum Next {
    /// One packet's audio, interleaved, at `rate` with `channels`.
    Audio { rate: u32, channels: usize },
    End,
}

fn decode_error(error: Error) -> AudioError {
    match error {
        Error::IoError(error) => AudioError::Io(error.to_string()),
        other => AudioError::Decode(other.to_string()),
    }
}

/// Opens `media` and finds its audio track. `extension` (such as `mp3`)
/// speeds up probing and helps with formats that have no magic bytes.
pub(crate) fn probe(
    media: Box<dyn MediaSource>,
    extension: Option<&str>,
) -> Result<Source, AudioError> {
    let mut hint = Hint::new();
    if let Some(extension) = extension {
        hint.with_extension(extension);
    }
    let stream = MediaSourceStream::new(media, Default::default());
    let reader = symphonia::default::get_probe()
        .probe(&hint, stream, FormatOptions::default(), MetadataOptions::default())
        .map_err(decode_error)?;
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or_else(|| AudioError::Decode("no audio track".into()))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or_else(|| AudioError::Decode("no audio codec parameters".into()))?;
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(decode_error)?;
    let rate = params.sample_rate.unwrap_or(crate::linux_mix::OUT_RATE);
    let channels = params.channels.as_ref().map_or(2, |channels| channels.count());
    let time_base = track.time_base;
    let duration = match (track.duration, time_base) {
        (Some(duration), Some(base)) => base.calc_duration(duration).map(|time| time.as_secs_f64()),
        _ => None,
    }
    .or_else(|| track.num_frames.map(|frames| frames as f64 / f64::from(rate)))
    .unwrap_or(f64::NAN);
    let track_id = track.id;
    Ok(Source {
        reader,
        decoder,
        track_id,
        time_base,
        rate,
        channels,
        duration,
        skip: 0,
    })
}

/// Opens in-memory encoded audio.
pub(crate) fn probe_bytes(bytes: Arc<[u8]>) -> Result<Source, AudioError> {
    probe(Box::new(Cursor::new(bytes)), None)
}

impl Source {
    /// Decodes the next packet of the audio track into `out` (cleared first).
    fn next(&mut self, out: &mut Vec<f32>) -> Result<Next, AudioError> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(Next::End),
                // A truncated file ends where its data does.
                Err(Error::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(Next::End);
                }
                // A chained stream changed format; play what was understood.
                Err(Error::ResetRequired) => return Ok(Next::End),
                Err(error) => return Err(decode_error(error)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let buffer = match self.decoder.decode(&packet) {
                Ok(buffer) => buffer,
                // One corrupt packet is skipped, as players do.
                Err(Error::DecodeError(_)) => continue,
                Err(error) => return Err(decode_error(error)),
            };
            let rate = buffer.spec().rate();
            let channels = buffer.spec().channels().count().max(1);
            out.clear();
            buffer.copy_to_vec_interleaved(out);
            if self.skip > 0 {
                let frames = (out.len() / channels).min(self.skip);
                out.drain(..frames * channels);
                self.skip -= frames;
            }
            self.rate = rate;
            self.channels = channels;
            return Ok(Next::Audio { rate, channels });
        }
    }

    /// Moves to `seconds`. Returns `false` when the position is past the end
    /// or the source cannot seek.
    fn seek(&mut self, seconds: f64) -> Result<bool, AudioError> {
        let time = Time::try_from_secs_f64(seconds.max(0.0)).unwrap_or_default();
        let seeked = match self.reader.seek(SeekMode::Accurate, SeekTo::Time {
            time,
            track_id: Some(self.track_id),
        }) {
            Ok(seeked) => seeked,
            Err(Error::SeekError(SeekErrorKind::OutOfRange | SeekErrorKind::Unseekable)) => {
                return Ok(false);
            }
            Err(error) => return Err(decode_error(error)),
        };
        self.decoder.reset();
        // An accurate seek may land on the packet before the target; the
        // frames between are decoded and dropped.
        self.skip = match self.time_base {
            Some(base) => {
                let required = base.calc_time_saturating(seeked.required_ts).as_secs_f64();
                let actual = base.calc_time_saturating(seeked.actual_ts).as_secs_f64();
                ((required - actual).max(0.0) * f64::from(self.rate)) as usize
            }
            None => 0,
        };
        Ok(true)
    }
}

/// Decodes all of `bytes` into one stereo clip at the output rate.
pub(crate) fn decode_all(bytes: Arc<[u8]>) -> Result<(Pcm, f64), AudioError> {
    let mut source = probe_bytes(bytes)?;
    let mut resampler = Resampler::new(source.rate, source.channels);
    let mut packet = Vec::new();
    let mut pcm = Vec::new();
    let mut format = (source.rate, source.channels);
    while let Next::Audio { rate, channels } = source.next(&mut packet)? {
        if (rate, channels) != format {
            format = (rate, channels);
            resampler = Resampler::new(rate, channels);
        }
        resampler.push(&packet, &mut pcm);
    }
    let duration = (pcm.len() / CHANNELS) as f64 / f64::from(crate::linux_mix::OUT_RATE);
    Ok((pcm.into(), duration))
}

/// What the decoder thread waits for.
enum Wake {
    Seek(f64),
    Room,
    Released,
}

/// Starts the decoder thread that fills `voice`'s queue. `open` runs on that
/// thread, so a stream's probing never blocks the engine.
pub(crate) fn spawn(
    voice: Arc<VoiceShared>,
    open: impl FnOnce() -> Result<Source, AudioError> + Send + 'static,
) -> Result<(), AudioError> {
    thread::Builder::new()
        .name("aimer-audio-decode".into())
        .spawn(move || run(&voice, open))
        .map(drop)
        .map_err(|error| AudioError::Io(error.to_string()))
}

fn run(voice: &VoiceShared, open: impl FnOnce() -> Result<Source, AudioError>) {
    let mut source = match open() {
        Ok(source) => source,
        Err(error) => return fail(voice, error),
    };
    voice.lock().duration = source.duration;
    let mut resampler = Resampler::new(source.rate, source.channels);
    let mut format = (source.rate, source.channels);
    let mut packet = Vec::new();
    let mut chunk = Vec::new();
    loop {
        match wait(voice) {
            Wake::Released => return,
            Wake::Seek(seconds) => {
                match source.seek(seconds) {
                    Ok(true) => {}
                    Ok(false) => set_ended(voice),
                    Err(error) => return fail(voice, error),
                }
                resampler.reset();
                continue;
            }
            Wake::Room => {}
        }
        match source.next(&mut packet) {
            Ok(Next::Audio { rate, channels }) => {
                if (rate, channels) != format {
                    format = (rate, channels);
                    resampler = Resampler::new(rate, channels);
                }
                chunk.clear();
                resampler.push(&packet, &mut chunk);
                let mut body = voice.lock();
                if let Feed::Queue { queue, seek: None, .. } = &mut body.feed {
                    queue.extend(&chunk);
                }
                // With a seek pending the chunk is stale; the loop serves it.
            }
            Ok(Next::End) => {
                if voice.lock().looping {
                    match source.seek(0.0) {
                        Ok(true) => {
                            resampler.reset();
                            continue;
                        }
                        Ok(false) => {}
                        Err(error) => return fail(voice, error),
                    }
                }
                set_ended(voice);
            }
            Err(error) => return fail(voice, error),
        }
    }
}

/// Blocks until there is room in the queue, a seek, or a release.
fn wait(voice: &VoiceShared) -> Wake {
    let mut body = voice.lock();
    loop {
        if body.released {
            return Wake::Released;
        }
        if let Feed::Queue { queue, ended, seek } = &mut body.feed {
            if let Some(seconds) = seek.take() {
                return Wake::Seek(seconds);
            }
            if !*ended && queue.len() < QUEUE_FRAMES * CHANNELS {
                return Wake::Room;
            }
        } else {
            return Wake::Released;
        }
        body = voice
            .wake
            .wait(body)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

fn set_ended(voice: &VoiceShared) {
    if let Feed::Queue { ended, .. } = &mut voice.lock().feed {
        *ended = true;
    }
}

fn fail(voice: &VoiceShared, error: AudioError) {
    let mut body = voice.lock();
    body.error = Some(error);
    if let Feed::Queue { ended, .. } = &mut body.feed {
        *ended = true;
    }
}

#[cfg(test)]
#[path = "linux_decode_tests.rs"]
mod tests;
