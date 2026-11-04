//! The Linux backend against real clips and the real `ffmpeg`.
//!
//! They skip, loudly, where FFmpeg is not installed.

use std::path::PathBuf;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use aimer_cupid::external_texture::{ExternalTextureSource, NativeTexture};

use crate::backend::FrameSlot;
use crate::linux_ffmpeg::Programs;
use crate::{VideoError, VideoSource};

/// 64 x 48, ten pictures a second for one second, no sound: red above, blue
/// below.
fn halves() -> VideoSource {
    VideoSource::file(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/halves.webm"))
}

fn have_ffmpeg() -> bool {
    let present = Programs::from_environment().available();
    if !present {
        eprintln!("skipped: ffmpeg and ffprobe are not installed");
    }
    present
}

/// Polls until `condition` holds, for up to ten seconds.
fn until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// The RGBA of pixel (`x`, `y`) of the frame in `frames`.
fn pixel(frames: &FrameSlot, x: usize, y: usize) -> [u8; 4] {
    let frame = frames.current_frame().expect("a frame is stored");
    let NativeTexture::Rgba8 { data, .. } = frame.texture() else {
        panic!("software frames are pixels");
    };
    let at = (y * frame.width() as usize + x) * 4;
    // SAFETY: `data` is `width * height * 4` bytes kept alive by `frame`.
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr(), (frame.width() * frame.height() * 4) as usize) };
    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
}

#[test]
fn a_clip_reports_its_size_and_length_and_shows_its_first_picture_while_paused() {
    if !have_ffmpeg() {
        return;
    }
    let video = super::linux::open(&halves()).expect("opens");
    assert!(until(|| video.is_ready()), "the clip never opened");
    assert_eq!(video.size(), Some((64, 48)));
    assert!((video.duration() - 1.0).abs() < 0.2, "duration {}", video.duration());

    let frames = FrameSlot::default();
    assert!(until(|| video.take_frame(&frames)), "no first picture");
    let (top, bottom) = (pixel(&frames, 32, 5), pixel(&frames, 32, 42));
    assert!(top[0] > 200 && top[2] < 60, "the top is red, got {top:?}");
    assert!(bottom[2] > 200 && bottom[0] < 60, "the bottom is blue, got {bottom:?}");
    assert_eq!(top[3], 255);

    // Paused: nothing more is taken and the playhead stays.
    assert!(!video.take_frame(&frames));
    assert_eq!(video.position(), 0.0);
    assert!(!video.is_playing());
}

#[test]
fn playing_advances_the_playhead_shows_new_pictures_and_ends() {
    if !have_ffmpeg() {
        return;
    }
    let video = super::linux::open(&halves()).expect("opens");
    assert!(until(|| video.is_ready()));
    video.play();
    assert!(video.is_playing());

    let frames = FrameSlot::default();
    let mut shown = 0;
    assert!(
        until(|| {
            if video.take_frame(&frames) {
                shown += 1;
            }
            !video.is_playing()
        }),
        "the clip never ended"
    );
    // One second at ten pictures a second.
    assert!(shown >= 6, "only {shown} pictures were shown");
    assert!((video.position() - 1.0).abs() < 0.25, "ended at {}", video.position());
}

#[test]
fn pausing_freezes_the_playhead_and_resuming_continues_from_it() {
    if !have_ffmpeg() {
        return;
    }
    let video = super::linux::open(&halves()).expect("opens");
    assert!(until(|| video.is_ready()));
    let frames = FrameSlot::default();
    video.play();
    assert!(until(|| {
        video.take_frame(&frames);
        video.position() > 0.2
    }));
    video.pause();
    let frozen = video.position();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(video.position(), frozen, "the playhead moved while paused");
    assert!(!video.take_frame(&frames), "a picture was shown while paused");

    video.play();
    std::thread::sleep(Duration::from_millis(100));
    assert!(video.position() > frozen);
}

#[test]
fn seeking_moves_the_playhead_and_shows_the_picture_there() {
    if !have_ffmpeg() {
        return;
    }
    let video = super::linux::open(&halves()).expect("opens");
    assert!(until(|| video.is_ready()));
    let frames = FrameSlot::default();
    assert!(until(|| video.take_frame(&frames)));

    video.seek(0.6);
    assert!((video.position() - 0.6).abs() < 0.01, "at {}", video.position());
    assert!(until(|| video.take_frame(&frames)), "no picture after the seek");
    assert!(!video.is_playing());
}

#[test]
fn a_seek_before_the_clip_has_opened_is_applied_when_it_does() {
    if !have_ffmpeg() {
        return;
    }
    let video = super::linux::open(&halves()).expect("opens");
    video.seek(0.5);
    assert!((video.position() - 0.5).abs() < 0.2 || video.position() == 0.0);
    assert!(until(|| video.is_ready()));
    assert!((video.position() - 0.5).abs() < 0.01, "at {}", video.position());
}

#[test]
fn a_missing_file_a_bad_url_and_a_non_video_are_reported() {
    if !have_ffmpeg() {
        return;
    }
    assert!(matches!(
        super::linux::open(&VideoSource::file("/no/such/clip.mp4")),
        Err(VideoError::NotFound(_))
    ));
    assert!(matches!(
        super::linux::open(&VideoSource::url("ftp://example.com/clip.mp4")),
        Err(VideoError::Network(_))
    ));
    let text = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let video = super::linux::open(&VideoSource::file(text)).expect("opens, then fails");
    assert!(until(|| video.error().is_some()), "no error for a non-video");
    assert!(matches!(video.error(), Some(VideoError::Decode(_))));
}

#[test]
fn a_clip_with_sound_plays_on_whether_or_not_there_is_an_audio_device() {
    if !have_ffmpeg() {
        return;
    }
    // 640 x 360, H.264 with an AAC track, six seconds. A machine with no sound
    // card (a build server) still plays the picture, on the wall clock.
    let clip = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/jaime/assets/test_pattern.mp4");
    let video = super::linux::open(&VideoSource::file(clip)).expect("opens");
    assert!(until(|| video.is_ready()));
    assert_eq!(video.size(), Some((640, 360)));
    assert!((video.duration() - 6.0).abs() < 0.2);

    let frames = FrameSlot::default();
    video.set_volume(0.0);
    video.play();
    assert!(until(|| {
        video.take_frame(&frames);
        video.position() > 0.5
    }));
    assert!(video.error().is_none(), "{:?}", video.error());
    assert!(video.is_playing());
    video.pause();
}

/// A stand-in `ffmpeg` that fails whenever it is asked to decode on the GPU,
/// as one without a working device does, and otherwise runs the real one.
fn ffmpeg_without_a_gpu(directory: &std::path::Path) -> Programs {
    let script = directory.join("ffmpeg");
    let log = directory.join("log");
    // Only decoding runs (`-i`) are logged, not the `-version` availability check.
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ndecode=0\nhardware=0\nfor argument in \"$@\"; do\n  [ \"$argument\" = \"-i\" ] && decode=1\n  [ \"$argument\" = \"-hwaccel\" ] && hardware=1\ndone\nif [ $hardware = 1 ]; then\n  echo hardware >> {log}\n  echo 'Failed setup for format vaapi: hwaccel initialisation returned error' >&2\n  exit 1\nfi\n[ $decode = 1 ] && echo software >> {log}\nexec ffmpeg \"$@\"\n",
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    Programs {
        ffmpeg: script.into_os_string(),
        ffprobe: "ffprobe".into(),
    }
}

#[test]
fn a_failing_gpu_decoder_falls_back_to_software_once_and_the_clip_plays() {
    if !have_ffmpeg() {
        return;
    }
    let directory = std::env::temp_dir().join(format!("aimer-video-hwfail-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let programs = ffmpeg_without_a_gpu(&directory);
    let video = super::linux::open_with(&halves(), programs).expect("opens");
    assert!(until(|| video.is_ready()));
    video.play();
    let frames = FrameSlot::default();
    let mut shown = 0;
    assert!(
        until(|| {
            if video.take_frame(&frames) {
                shown += 1;
            }
            !video.is_playing()
        }),
        "the clip never ended"
    );
    assert!(video.error().is_none(), "{:?}", video.error());
    assert!(shown >= 6, "only {shown} pictures were shown");

    // One failed attempt with the GPU, then software for good.
    let log = std::fs::read_to_string(directory.join("log")).unwrap();
    let attempts: Vec<&str> = log.lines().collect();
    assert_eq!(attempts.first(), Some(&"hardware"), "{attempts:?}");
    assert_eq!(attempts.iter().filter(|line| **line == "hardware").count(), 1, "{attempts:?}");
    assert!(attempts.contains(&"software"), "{attempts:?}");
    std::fs::remove_dir_all(&directory).ok();
}
