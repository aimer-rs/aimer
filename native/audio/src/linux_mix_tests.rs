use super::*;

fn assert_close(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 1e-6, "{actual:?} != {expected:?}");
    }
}

fn clip(frames: &[[f32; 2]]) -> Arc<VoiceShared> {
    let pcm: Pcm = frames.iter().flatten().copied().collect::<Vec<_>>().into();
    let duration = frames.len() as f64 / f64::from(OUT_RATE);
    VoiceShared::new(Feed::Clip { pcm, cursor: 0 }, duration)
}

fn queue() -> Arc<VoiceShared> {
    VoiceShared::new(
        Feed::Queue {
            queue: VecDeque::new(),
            ended: false,
            seek: None,
        },
        f64::NAN,
    )
}

fn push(voice: &VoiceShared, samples: &[f32]) {
    if let Feed::Queue { queue, .. } = &mut voice.lock().feed {
        queue.extend(samples);
    }
}

fn end(voice: &VoiceShared) {
    if let Feed::Queue { ended, .. } = &mut voice.lock().feed {
        *ended = true;
    }
}

#[test]
fn a_paused_voice_adds_nothing() {
    let voice = clip(&[[0.5, 0.5]; 4]);
    let mut out = [0.0; 8];
    mix_voice(&voice, &mut out);
    assert_eq!(out, [0.0; 8]);
    assert_eq!(voice.lock().frame, 0);
}

#[test]
fn a_clip_is_scaled_added_and_stops_at_its_end() {
    let voice = clip(&[[0.5, -0.5], [0.25, -0.25], [1.0, -1.0]]);
    voice.set_volume(0.5);
    voice.playing.store(true, Ordering::Relaxed);
    let mut out = [0.1; 8];
    mix_voice(&voice, &mut out);

    assert_close(&out, &[0.35, -0.15, 0.225, -0.025, 0.6, -0.4, 0.1, 0.1]);
    assert_eq!(voice.lock().frame, 3);
    assert!(!voice.playing.load(Ordering::Relaxed), "ran out, so it stopped");
}

#[test]
fn a_looping_clip_wraps_around_without_stopping() {
    let voice = clip(&[[1.0, 1.0], [2.0, 2.0]]);
    voice.lock().looping = true;
    voice.playing.store(true, Ordering::Relaxed);
    let mut out = [0.0; 10];
    mix_voice(&voice, &mut out);

    assert_eq!(out, [1.0, 1.0, 2.0, 2.0, 1.0, 1.0, 2.0, 2.0, 1.0, 1.0]);
    assert!(voice.playing.load(Ordering::Relaxed));
}

#[test]
fn an_empty_looping_clip_does_not_spin() {
    let voice = clip(&[]);
    voice.lock().looping = true;
    voice.playing.store(true, Ordering::Relaxed);
    let mut out = [0.0; 4];
    mix_voice(&voice, &mut out);
    assert!(!voice.playing.load(Ordering::Relaxed));
}

#[test]
fn a_queue_that_runs_dry_is_starved_until_its_decoder_reports_the_end() {
    let voice = queue();
    voice.playing.store(true, Ordering::Relaxed);
    push(&voice, &[0.5, 0.5]);
    let mut out = [0.0; 4];
    mix_voice(&voice, &mut out);

    assert_eq!(out, [0.5, 0.5, 0.0, 0.0]);
    assert!(voice.lock().starved);
    assert!(voice.playing.load(Ordering::Relaxed), "waiting for data is not the end");

    end(&voice);
    mix_voice(&voice, &mut out);
    assert!(!voice.lock().starved);
    assert!(!voice.playing.load(Ordering::Relaxed));
}

#[test]
fn a_queue_reads_across_its_ring_boundary() {
    let voice = queue();
    voice.playing.store(true, Ordering::Relaxed);
    if let Feed::Queue { queue, .. } = &mut voice.lock().feed {
        // Force the ring to wrap: fill, drain the front, refill.
        queue.extend([9.0; 6]);
        queue.drain(..4);
        queue.extend([1.0, 2.0, 3.0, 4.0]);
    }
    let mut out = [0.0; 6];
    mix_voice(&voice, &mut out);
    assert_eq!(out, [9.0, 9.0, 1.0, 2.0, 3.0, 4.0]);
    assert_eq!(voice.lock().frame, 3);
}

#[test]
fn seeking_a_queue_drops_stale_audio_and_asks_the_decoder() {
    let voice = queue();
    push(&voice, &[1.0; 8]);
    end(&voice);
    voice.seek(2.0);

    let body = voice.lock();
    let Feed::Queue { queue, ended, seek } = &body.feed else {
        unreachable!()
    };
    assert!(queue.is_empty());
    assert!(!ended);
    assert_eq!(*seek, Some(2.0));
    assert_eq!(body.frame, 2 * u64::from(OUT_RATE));
}

#[test]
fn seeking_a_clip_moves_its_cursor_and_clamps_to_the_end() {
    let voice = clip(&[[0.0, 0.0]; 48]);
    voice.seek(0.5 / 1000.0); // 24 frames
    assert_eq!(voice.lock().frame, 24);
    voice.seek(10.0);
    assert_eq!(voice.lock().frame, 48);
    voice.seek(-1.0);
    assert_eq!(voice.lock().frame, 0);
}

#[test]
fn samples_clip_when_converted_to_16_bit() {
    let mut out = [0_i16; 4];
    to_i16(&[0.0, 1.0, -2.0, 0.5], &mut out);
    assert_eq!(out, [0, i16::MAX, -i16::MAX, i16::MAX / 2]);
}

#[test]
fn audio_at_the_output_rate_only_gains_a_second_channel() {
    let mut resampler = Resampler::new(OUT_RATE, 1);
    let mut out = Vec::new();
    resampler.push(&[0.1, 0.2], &mut out);
    assert_eq!(out, [0.1, 0.1, 0.2, 0.2]);

    let mut resampler = Resampler::new(OUT_RATE, 6);
    out.clear();
    resampler.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &mut out);
    assert_eq!(out, [1.0, 2.0], "surround keeps its front pair");
}

#[test]
fn upsampling_interpolates_between_input_frames() {
    let mut resampler = Resampler::new(OUT_RATE / 2, 2);
    let mut out = Vec::new();
    resampler.push(&[0.0, 0.0, 1.0, -1.0, 0.0, 0.0], &mut out);
    // Frames at input positions 0, 0.5, 1, 1.5; position 2 needs the next chunk.
    assert_close(&out, &[0.0, 0.0, 0.5, -0.5, 1.0, -1.0, 0.5, -0.5]);
}

#[test]
fn a_stream_resampled_in_chunks_matches_one_resampled_whole() {
    let input: Vec<f32> = (0..300).map(|i| (i as f32 * 0.37).sin()).collect();
    let mut whole = Vec::new();
    Resampler::new(44_100, 1).push(&input, &mut whole);

    let mut chunked = Vec::new();
    let mut resampler = Resampler::new(44_100, 1);
    for chunk in input.chunks(37) {
        resampler.push(chunk, &mut chunked);
    }
    // The whole run stops one interpolation short of its end, like a chunk.
    assert!(chunked.len() >= whole.len());
    for (a, b) in whole.iter().zip(&chunked) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }
}

#[test]
fn downsampling_produces_the_right_number_of_frames() {
    let input = vec![0.0_f32; 96_000]; // one second at 96 kHz, mono
    let mut out = Vec::new();
    Resampler::new(96_000, 1).push(&input, &mut out);
    let frames = out.len() / CHANNELS;
    assert!((frames as i64 - i64::from(OUT_RATE)).abs() <= 1, "{frames} frames");
}
