use std::time::Instant;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, Rgba, RgbaImage};

use super::*;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A 4x4 GIF with one solid frame per `(color, delay in ms)`.
fn gif(frames: &[([u8; 4], u32)], repeat: Repeat) -> Arc<[u8]> {
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut bytes);
        encoder.set_repeat(repeat).expect("repeat is written");
        for (color, delay_ms) in frames {
            let image = RgbaImage::from_pixel(4, 4, Rgba(*color));
            let delay = Delay::from_numer_denom_ms(*delay_ms, 1);
            encoder
                .encode_frame(Frame::from_parts(image, 0, 0, delay))
                .expect("frame is encoded");
        }
    }
    Arc::from(bytes)
}

/// Red for 100 ms, green for 200 ms, blue for 300 ms.
fn rgb_gif(repeat: Repeat) -> Arc<[u8]> {
    gif(&[(RED, 100), (GREEN, 200), (BLUE, 300)], repeat)
}

fn source(bytes: &Arc<[u8]>, loops: GifLoops, skip: usize) -> (FrameSource, Arc<ControllerState>) {
    let state = Arc::new(ControllerState::new(true));
    let source = FrameSource::new(Arc::clone(bytes), loops, u32::MAX, Arc::clone(&state), skip);
    (source, state)
}

/// 0, 1 or 2 for a frame whose pixels are mostly red, green or blue.
fn dominant(frame: &DecodedFrame) -> usize {
    let pixel = &frame.resource.rgba()[..3];
    (0..3).max_by_key(|channel| pixel[*channel]).unwrap()
}

fn drain(source: &mut FrameSource) -> Vec<DecodedFrame> {
    std::iter::from_fn(|| source.next()).collect()
}

fn indices(frames: &[DecodedFrame]) -> Vec<usize> {
    frames.iter().map(|frame| frame.index).collect()
}

fn player(bytes: &Arc<[u8]>, controller: &GifController, loops: GifLoops) -> GifPlayer {
    let options = GifOptions::new()
        .controller(controller.clone())
        .loops(loops);
    GifPlayer::new(
        Arc::clone(bytes),
        &options,
        u32::MAX,
        &WindowHandle::headless(Default::default(), 1.0),
    )
}

struct Clock {
    base: AnimInstant,
}

impl Clock {
    fn new() -> Self {
        Self {
            base: AnimInstant::now(),
        }
    }

    fn at(&self, milliseconds: u64) -> AnimInstant {
        self.base + Duration::from_millis(milliseconds)
    }
}

#[test]
fn recognizes_gif_signatures_only() {
    assert!(is_gif(b"GIF89a\x01\x00"));
    assert!(is_gif(b"GIF87a"));
    assert!(!is_gif(b"\x89PNG\r\n\x1a\n"));
    assert!(!is_gif(b"GIF"));
    assert!(!is_gif(b""));
}

#[test]
fn registry_remembers_gifs_and_ignores_other_bytes() {
    let bytes = rgb_gif(Repeat::Infinite);
    register("test:registry-roundtrip".into(), &bytes);
    register("test:registry-png".into(), b"\x89PNG\r\n\x1a\n");

    assert_eq!(
        lookup("test:registry-roundtrip").as_deref(),
        Some(&*bytes)
    );
    assert!(lookup("test:registry-png").is_none());
    assert!(lookup("test:registry-missing").is_none());
}

#[test]
fn registry_drops_the_oldest_entries_past_its_entry_limit() {
    let mut registry = Registry::new();
    for index in 0..REGISTRY_MAX_ENTRIES + 6 {
        registry.insert(format!("entry-{index}"), Arc::from(vec![0_u8; 8]));
    }

    assert_eq!(registry.entries.len(), REGISTRY_MAX_ENTRIES);
    assert!(registry.get("entry-0").is_none());
    assert!(registry.get("entry-5").is_none());
    assert!(registry.get("entry-6").is_some());
    assert!(registry.get(&format!("entry-{}", REGISTRY_MAX_ENTRIES + 5)).is_some());
}

#[test]
fn registry_drops_the_oldest_entries_past_its_byte_limit() {
    let mut registry = Registry::new();
    let chunk = REGISTRY_MAX_BYTES / 2 + 1;
    registry.insert("a".into(), Arc::from(vec![0_u8; chunk]));
    registry.insert("b".into(), Arc::from(vec![0_u8; chunk]));

    assert!(registry.get("a").is_none());
    assert!(registry.get("b").is_some());
    assert_eq!(registry.bytes, chunk);
}

#[test]
fn registry_replaces_an_entry_without_double_counting_its_bytes() {
    let mut registry = Registry::new();
    registry.insert("a".into(), Arc::from(vec![0_u8; 100]));
    registry.insert("a".into(), Arc::from(vec![0_u8; 40]));

    assert_eq!(registry.entries.len(), 1);
    assert_eq!(registry.bytes, 40);
}

#[test]
fn registry_keys_do_not_collide_across_source_kinds() {
    let file = key_file(std::path::Path::new("a.gif"));
    let asset = key_asset("a.gif");
    let network = key_network("a.gif");

    assert!(file != asset && asset != network && file != network);
}

#[test]
fn frames_arrive_in_order_with_their_delays() {
    let bytes = rgb_gif(Repeat::Infinite);
    let (mut source, state) = source(&bytes, GifLoops::ONCE, 0);

    let frames = drain(&mut source);

    assert_eq!(indices(&frames), [0, 1, 2]);
    assert_eq!(
        frames.iter().map(dominant).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        frames.iter().map(|frame| frame.delay).collect::<Vec<_>>(),
        [
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ]
    );
    assert_eq!(state.frame_count.load(Ordering::Acquire), 3);
    assert_eq!((frames[0].resource.width(), frames[0].resource.height()), (4, 4));
}

#[test]
fn a_requested_loop_count_overrides_the_file() {
    let bytes = rgb_gif(Repeat::Infinite);
    let twice = GifLoops::Count(NonZeroU32::new(2).unwrap());
    let (mut source, _) = source(&bytes, twice, 0);

    assert_eq!(indices(&drain(&mut source)), [0, 1, 2, 0, 1, 2]);
}

#[test]
fn the_files_loop_count_counts_repeats_after_the_first_play() {
    let bytes = rgb_gif(Repeat::Finite(1));
    let (mut source, _) = source(&bytes, GifLoops::FromFile, 0);

    assert_eq!(indices(&drain(&mut source)), [0, 1, 2, 0, 1, 2]);
}

#[test]
fn infinite_loops_never_run_out() {
    let bytes = rgb_gif(Repeat::Finite(1));
    let (mut source, _) = source(&bytes, GifLoops::Infinite, 0);

    let frames: Vec<_> = std::iter::from_fn(|| source.next()).take(10).collect();

    assert_eq!(frames.len(), 10);
    assert_eq!(frames[9].index, 0);
}

#[test]
fn skipping_starts_at_the_requested_frame_and_the_next_loop_starts_over() {
    let bytes = rgb_gif(Repeat::Infinite);
    let twice = GifLoops::Count(NonZeroU32::new(2).unwrap());
    let (mut source, _) = source(&bytes, twice, 2);

    let frames = drain(&mut source);

    assert_eq!(indices(&frames), [2, 0, 1, 2]);
    assert_eq!(dominant(&frames[0]), 2);
}

#[test]
fn skipping_past_the_end_lands_on_the_last_frame() {
    let bytes = rgb_gif(Repeat::Infinite);
    let (mut source, state) = source(&bytes, GifLoops::ONCE, 99);

    let frames = drain(&mut source);

    assert_eq!(indices(&frames), [2]);
    assert_eq!(state.frame_count.load(Ordering::Acquire), 3);
}

#[test]
fn bytes_that_are_not_a_gif_end_at_once() {
    let bytes: Arc<[u8]> = Arc::from(&b"not a gif at all"[..]);
    let (mut source, state) = source(&bytes, GifLoops::Infinite, 0);

    assert!(source.next().is_none());
    assert!(source.next().is_none());
    assert_eq!(state.frame_count.load(Ordering::Acquire), 0);
}

#[test]
fn a_header_declaring_a_gigantic_canvas_is_refused_before_any_allocation() {
    // "GIF89a", a 60000 x 60000 logical screen, no color table, a trailer.
    let mut header = b"GIF89a".to_vec();
    header.extend_from_slice(&60_000_u16.to_le_bytes());
    header.extend_from_slice(&60_000_u16.to_le_bytes());
    header.extend_from_slice(&[0, 0, 0, 0x3B]);
    let bytes: Arc<[u8]> = Arc::from(header);

    let started = Instant::now();
    assert!(open(&bytes).is_none());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn loaders_register_gifs_under_the_keys_their_source_looks_up() {
    use std::path::PathBuf;

    use super::super::source::{ImageSource, decode_registering};
    use crate::ImageProvider;

    let bytes = rgb_gif(Repeat::Infinite);
    let path = PathBuf::from("gif-tests/registered-through-the-loader.gif");
    let url = "https://example.invalid/gif-tests/registered.gif";
    let asset = "gif-tests/registered-asset.gif";

    assert!(ImageSource::File(path.clone()).animation_bytes().is_none());
    decode_registering(key_file(&path), &bytes).expect("gif decodes");
    decode_registering(key_network(url), &bytes).expect("gif decodes");
    decode_registering(key_asset(asset), &bytes).expect("gif decodes");

    for source in [
        ImageSource::File(path),
        ImageSource::Network(url.to_owned()),
        ImageSource::NetworkWithHeaders(url.to_owned(), Default::default()),
        ImageSource::Asset(asset.to_owned()),
    ] {
        assert_eq!(source.animation_bytes().as_deref(), Some(&*bytes), "{source:?}");
    }
    assert!(ImageSource::Id(3).animation_bytes().is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn still_images_are_not_registered_as_animations() {
    use std::path::PathBuf;

    use super::super::source::{ImageSource, decode_registering};
    use crate::ImageProvider;

    let mut png = Vec::new();
    RgbaImage::from_pixel(2, 2, Rgba(RED))
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("png encodes");
    let path = PathBuf::from("gif-tests/a-still-image.png");

    decode_registering(key_file(&path), &png).expect("png decodes");

    assert!(ImageSource::File(path).animation_bytes().is_none());
}

#[test]
fn zero_and_tiny_delays_become_the_browser_default() {
    assert_eq!(effective_delay(Duration::ZERO), DEFAULT_FRAME_DELAY);
    assert_eq!(effective_delay(Duration::from_millis(10)), DEFAULT_FRAME_DELAY);
    assert_eq!(
        effective_delay(Duration::from_millis(20)),
        Duration::from_millis(20)
    );
}

#[test]
fn large_frames_are_scaled_down_to_the_upload_limit() {
    let image = RgbaImage::from_pixel(64, 32, Rgba(RED));

    let resource = frame_resource(image, 16).expect("frame becomes a resource");

    assert_eq!((resource.width(), resource.height()), (16, 8));
    assert_eq!(
        (resource.intrinsic_width(), resource.intrinsic_height()),
        (64, 32),
        "layout keeps seeing the GIF's own size"
    );
}

#[test]
fn frames_with_different_pixels_get_different_textures() {
    let bytes = rgb_gif(Repeat::Infinite);
    let (mut source, _) = source(&bytes, GifLoops::ONCE, 0);

    let frames = drain(&mut source);

    assert_ne!(frames[0].resource.texture_id(), frames[1].resource.texture_id());
    assert_ne!(frames[1].resource.revision(), frames[2].resource.revision());
}

#[test]
fn the_player_shows_the_first_frame_then_advances_as_delays_pass() {
    let bytes = rgb_gif(Repeat::Infinite);
    let controller = GifController::new();
    let mut player = player(&bytes, &controller, GifLoops::Infinite);
    let clock = Clock::new();

    let first = player.tick(clock.at(0));
    assert!(first.changed && first.wants_frame);
    assert_eq!(controller.current_frame(), 0);

    assert!(!player.tick(clock.at(99)).changed);
    assert_eq!(controller.current_frame(), 0);

    assert!(player.tick(clock.at(100)).changed);
    assert_eq!(controller.current_frame(), 1);

    assert!(!player.tick(clock.at(299)).changed);
    assert!(player.tick(clock.at(300)).changed);
    assert_eq!(controller.current_frame(), 2);
}

#[test]
fn the_player_wraps_around_to_the_first_frame_when_looping() {
    let bytes = rgb_gif(Repeat::Infinite);
    let controller = GifController::new();
    let mut player = player(&bytes, &controller, GifLoops::Infinite);
    let clock = Clock::new();

    player.tick(clock.at(0));
    player.tick(clock.at(100));
    player.tick(clock.at(300));
    let wrapped = player.tick(clock.at(600));

    assert!(wrapped.changed && wrapped.wants_frame);
    assert_eq!(controller.current_frame(), 0);
    assert_eq!(controller.frame_count(), Some(3));
    assert!(!controller.is_finished());
}

#[test]
fn a_long_stall_does_not_replay_the_frames_it_skipped() {
    let bytes = rgb_gif(Repeat::Infinite);
    let controller = GifController::new();
    let mut player = player(&bytes, &controller, GifLoops::Infinite);
    let clock = Clock::new();

    player.tick(clock.at(0));
    let resumed = player.tick(clock.at(60_000));

    assert!(resumed.changed);
    assert!(
        controller.current_frame() <= FRAME_WINDOW,
        "a stall catches up by a bounded number of frames per tick"
    );
}

#[test]
fn a_paused_player_holds_its_frame_and_stops_asking_for_frames() {
    let bytes = rgb_gif(Repeat::Infinite);
    let controller = GifController::new();
    let mut player = player(&bytes, &controller, GifLoops::Infinite);
    let clock = Clock::new();

    player.tick(clock.at(0));
    controller.pause();
    let paused = player.tick(clock.at(500));

    assert!(!paused.changed && !paused.wants_frame);
    assert_eq!(controller.current_frame(), 0);

    controller.play();
    let resumed = player.tick(clock.at(550));
    assert!(!resumed.changed && resumed.wants_frame);
    assert!(
        player.tick(clock.at(600)).changed,
        "paused time does not count towards the frame's delay"
    );
    assert_eq!(controller.current_frame(), 1);
}

#[test]
fn a_controller_that_starts_paused_shows_the_first_frame_and_waits() {
    let bytes = rgb_gif(Repeat::Infinite);
    let controller = GifController::paused();
    let mut player = player(&bytes, &controller, GifLoops::Infinite);
    let clock = Clock::new();

    let first = player.tick(clock.at(0));
    assert!(first.changed && !first.wants_frame);
    assert!(player.resource().is_some());

    assert!(!player.tick(clock.at(5_000)).changed);
    assert_eq!(controller.current_frame(), 0);
}

#[test]
fn seeking_jumps_to_the_frame_even_while_paused() {
    let bytes = rgb_gif(Repeat::Infinite);
    let controller = GifController::paused();
    let mut player = player(&bytes, &controller, GifLoops::Infinite);
    let clock = Clock::new();
    player.tick(clock.at(0));

    controller.seek_to_frame(2);
    let sought = player.tick(clock.at(10));

    assert!(sought.changed && !sought.wants_frame);
    assert_eq!(controller.current_frame(), 2);
    assert_eq!(
        dominant(&DecodedFrame {
            index: 2,
            delay: Duration::ZERO,
            resource: Arc::clone(player.resource().unwrap()),
        }),
        2
    );
}

#[test]
fn seeking_past_the_end_selects_the_last_frame() {
    let bytes = rgb_gif(Repeat::Infinite);
    let controller = GifController::new();
    let mut player = player(&bytes, &controller, GifLoops::Infinite);
    let clock = Clock::new();
    player.tick(clock.at(0));

    controller.seek_to_frame(usize::MAX);
    player.tick(clock.at(10));

    assert_eq!(controller.current_frame(), 2);
    assert_eq!(controller.frame_count(), Some(3));
}

#[test]
fn a_player_finishes_on_the_last_frame_when_loops_run_out() {
    let bytes = rgb_gif(Repeat::Infinite);
    let controller = GifController::new();
    let mut player = player(&bytes, &controller, GifLoops::ONCE);
    let clock = Clock::new();

    player.tick(clock.at(0));
    player.tick(clock.at(100));
    player.tick(clock.at(300));
    let done = player.tick(clock.at(600));

    assert!(!done.changed && !done.wants_frame);
    assert!(controller.is_finished());
    assert_eq!(controller.current_frame(), 2);
    assert!(player.resource().is_some(), "the last frame stays up");
    assert!(!player.tick(clock.at(5_000)).wants_frame);
}

#[test]
fn playing_a_finished_gif_starts_it_over() {
    let bytes = rgb_gif(Repeat::Infinite);
    let controller = GifController::new();
    let mut player = player(&bytes, &controller, GifLoops::ONCE);
    let clock = Clock::new();
    player.tick(clock.at(0));
    player.tick(clock.at(100));
    player.tick(clock.at(300));
    player.tick(clock.at(600));
    assert!(controller.is_finished());

    controller.play();
    let restarted = player.tick(clock.at(700));

    assert!(restarted.changed && restarted.wants_frame);
    assert_eq!(controller.current_frame(), 0);
    assert!(!controller.is_finished());
}

#[test]
fn a_single_frame_gif_finishes_after_its_delay_without_looping() {
    let bytes = gif(&[(RED, 100)], Repeat::Infinite);
    let controller = GifController::new();
    let mut player = player(&bytes, &controller, GifLoops::ONCE);
    let clock = Clock::new();

    assert!(player.tick(clock.at(0)).changed);
    let after = player.tick(clock.at(100));

    assert!(!after.changed && !after.wants_frame);
    assert!(controller.is_finished());
}

#[test]
fn undecodable_bytes_leave_a_finished_player_with_no_frame() {
    let bytes: Arc<[u8]> = Arc::from(&b"GIF89a-but-truncated"[..]);
    let controller = GifController::new();
    let mut player = player(&bytes, &controller, GifLoops::Infinite);
    let clock = Clock::new();

    let tick = player.tick(clock.at(0));

    assert!(!tick.changed && !tick.wants_frame);
    assert!(player.resource().is_none());
}

#[test]
fn the_controller_reports_and_toggles_playback() {
    let controller = GifController::paused();
    assert!(!controller.is_playing());

    controller.toggle();
    assert!(controller.is_playing());
    controller.toggle();
    assert!(!controller.is_playing());

    controller.restart();
    assert!(controller.is_playing());
    assert_eq!(controller.frame_count(), None);
    assert!(!controller.is_finished());
}

#[test]
fn clones_of_a_controller_share_state() {
    let controller = GifController::new();
    let clone = controller.clone();

    clone.pause();

    assert!(!controller.is_playing());
}

#[test]
fn the_decoder_thread_delivers_every_frame_and_then_ends() {
    let bytes = rgb_gif(Repeat::Infinite);
    let state = Arc::new(ControllerState::new(true));
    let Some(Feed::Thread(receiver)) = spawn_feed(bytes, GifLoops::ONCE, u32::MAX, state, 0)
    else {
        panic!("the decoder thread starts");
    };

    let timeout = Duration::from_secs(10);
    let first = receiver.recv_timeout(timeout).expect("first frame");
    let second = receiver.recv_timeout(timeout).expect("second frame");
    let third = receiver.recv_timeout(timeout).expect("third frame");

    assert_eq!([first.index, second.index, third.index], [0, 1, 2]);
    assert!(receiver.recv_timeout(timeout).is_err());
}

#[test]
fn the_decoder_thread_stops_decoding_ahead_of_a_full_window() {
    let frames: Vec<_> = (0..FRAME_WINDOW + 4).map(|_| (RED, 100)).collect();
    let bytes = gif(&frames, Repeat::Infinite);
    let state = Arc::new(ControllerState::new(true));
    let Some(Feed::Thread(receiver)) = spawn_feed(
        bytes,
        GifLoops::Infinite,
        u32::MAX,
        Arc::clone(&state),
        0,
    ) else {
        panic!("the decoder thread starts");
    };

    let deadline = Instant::now() + Duration::from_secs(10);
    while receiver.len() < FRAME_WINDOW && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(100));

    assert_eq!(receiver.len(), FRAME_WINDOW);
    assert_eq!(
        state.frame_count.load(Ordering::Acquire),
        0,
        "a blocked decoder has not read the file to its end"
    );
}

#[test]
fn dropping_the_feed_lets_the_decoder_thread_end() {
    let bytes = rgb_gif(Repeat::Infinite);
    let state = Arc::new(ControllerState::new(true));
    let feed = spawn_feed(
        bytes,
        GifLoops::Infinite,
        u32::MAX,
        Arc::clone(&state),
        0,
    );
    // The thread holds one clone of the state; it ends, and releases it, once
    // the receiver is gone and its blocked send fails.
    assert!(Arc::strong_count(&state) >= 2);

    drop(feed);

    let deadline = Instant::now() + Duration::from_secs(10);
    while Arc::strong_count(&state) > 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(Arc::strong_count(&state), 1);
}
