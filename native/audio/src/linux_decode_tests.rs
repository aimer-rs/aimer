use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::*;
use crate::linux_mix::OUT_RATE;

/// A 16-bit PCM WAV file.
pub(crate) fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let block = u32::from(channels) * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * block).to_le_bytes());
    out.extend_from_slice(&(block as u16).to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// One second of a ramp at 48 kHz, stereo, left positive and right negative.
fn ramp() -> Vec<u8> {
    let samples: Vec<i16> = (0..OUT_RATE as i32)
        .flat_map(|i| {
            let value = (i % 1000) as i16 * 16;
            [value, -value]
        })
        .collect();
    wav(OUT_RATE, 2, &samples)
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn queued(voice: &VoiceShared) -> usize {
    match &voice.lock().feed {
        Feed::Queue { queue, .. } => queue.len() / CHANNELS,
        Feed::Clip { .. } => 0,
    }
}

fn ended(voice: &VoiceShared) -> bool {
    matches!(&voice.lock().feed, Feed::Queue { ended: true, .. })
}

fn queue_voice() -> Arc<VoiceShared> {
    VoiceShared::new(
        Feed::Queue {
            queue: VecDeque::new(),
            ended: false,
            seek: None,
        },
        f64::NAN,
    )
}

#[test]
fn a_wav_decodes_to_a_stereo_clip_at_the_output_rate() {
    let bytes = wav(8000, 1, &[i16::MAX / 2; 8000]);
    let (pcm, duration) = decode_all(bytes.into()).unwrap();
    assert!((duration - 1.0).abs() < 0.01, "{duration} s");
    let frames = pcm.len() / CHANNELS;
    assert!((frames as i64 - i64::from(OUT_RATE)).abs() < 10, "{frames} frames");
    assert!(pcm.iter().all(|sample| (sample - 0.5).abs() < 0.01));
}

#[test]
fn the_probe_reads_the_length_from_the_header() {
    let source = probe_bytes(ramp().into()).unwrap();
    assert!((source.duration - 1.0).abs() < 1e-6);
}

#[test]
fn bytes_that_are_not_audio_fail_to_decode() {
    let error = decode_all(b"definitely not audio".to_vec().into()).unwrap_err();
    assert!(matches!(error, AudioError::Decode(_)), "{error:?}");
}

#[test]
fn the_decoder_thread_fills_the_queue_only_up_to_its_limit() {
    let voice = queue_voice();
    let bytes: Arc<[u8]> = ramp().into();
    spawn(voice.clone(), move || probe_bytes(bytes)).unwrap();

    wait_until("the queue to fill", || queued(&voice) >= QUEUE_FRAMES);
    std::thread::sleep(Duration::from_millis(20));
    // At most one packet over the limit, never the whole second.
    assert!(queued(&voice) < QUEUE_FRAMES + 4096, "{} frames", queued(&voice));
    assert!(!ended(&voice));
    assert!((voice.lock().duration - 1.0).abs() < 1e-6);
    voice.lock().released = true;
    voice.wake.notify_all();
}

#[test]
fn draining_the_queue_lets_the_decoder_reach_the_end() {
    let voice = queue_voice();
    let bytes: Arc<[u8]> = ramp().into();
    spawn(voice.clone(), move || probe_bytes(bytes)).unwrap();
    let mut total = 0;
    wait_until("the end of the file", || {
        if let Feed::Queue { queue, .. } = &mut voice.lock().feed {
            total += queue.len() / CHANNELS;
            queue.clear();
        }
        voice.wake.notify_all();
        ended(&voice) && queued(&voice) == 0
    });
    assert!((total as i64 - i64::from(OUT_RATE)).abs() < 10, "{total} frames");
}

#[test]
fn a_seek_restarts_decoding_at_the_requested_sample() {
    let voice = queue_voice();
    let bytes: Arc<[u8]> = ramp().into();
    spawn(voice.clone(), move || probe_bytes(bytes)).unwrap();
    wait_until("the first audio", || queued(&voice) > 0);

    // Frame 24_500 of the ramp has the value 500 * 16.
    voice.seek(24_500.0 / f64::from(OUT_RATE));
    wait_until("audio after the seek", || queued(&voice) > 0);
    let first = match &voice.lock().feed {
        Feed::Queue { queue, .. } => [queue[0], queue[1]],
        Feed::Clip { .. } => unreachable!(),
    };
    let expected = 500.0 * 16.0 / 32768.0;
    assert!((first[0] - expected).abs() < 1e-3, "{first:?}");
    assert!((first[1] + expected).abs() < 1e-3, "{first:?}");
    voice.lock().released = true;
    voice.wake.notify_all();
}

#[test]
fn a_looping_track_keeps_decoding_past_its_end() {
    let voice = queue_voice();
    voice.lock().looping = true;
    let bytes: Arc<[u8]> = wav(OUT_RATE, 1, &[1000; 4800]).into(); // 0.1 s
    spawn(voice.clone(), move || probe_bytes(bytes)).unwrap();
    let mut total = 0;
    wait_until("three passes", || {
        if let Feed::Queue { queue, .. } = &mut voice.lock().feed {
            total += queue.len() / CHANNELS;
            queue.clear();
        }
        voice.wake.notify_all();
        total > 3 * 4800
    });
    assert!(!ended(&voice));
    voice.lock().released = true;
    voice.wake.notify_all();
}

#[test]
fn a_source_that_cannot_open_marks_the_voice_failed_and_ended() {
    let voice = queue_voice();
    spawn(voice.clone(), || Err(AudioError::Network("404".into()))).unwrap();
    wait_until("the failure", || voice.lock().error.is_some());
    assert!(ended(&voice));
}
