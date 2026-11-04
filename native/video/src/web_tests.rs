//! Browser tests of the web backend: a real `<video>` element playing a
//! generated clip, and the frames it hands the renderer.

use aimer_cupid::external_texture::{ExternalTextureSource, NativeTexture};
use aimer_cupid_gpu::web_video;
use js_sys::{Array, Promise, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use web_sys::{Blob, BlobPropertyBag, Url};

use crate::backend::{FrameSlot, PlatformVideo};
use crate::{VideoError, VideoSource};

wasm_bindgen_test_configure!(run_in_browser);

/// 64 x 48, one second at ten frames a second: red above, blue below.
const HALVES: &[u8] = include_bytes!("../fixtures/halves.webm");

fn clip_url() -> String {
    let bytes = Uint8Array::from(HALVES);
    let options = BlobPropertyBag::new();
    options.set_type("video/webm");
    let blob = Blob::new_with_u8_array_sequence_and_options(&Array::of1(&bytes), &options).expect("blob");
    Url::create_object_url_with_blob(&blob).expect("object URL")
}

async fn sleep(milliseconds: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, milliseconds)
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}

/// Polls `condition` every few milliseconds, for up to five seconds.
async fn until(mut condition: impl FnMut() -> bool) -> bool {
    for _ in 0..250 {
        if condition() {
            return true;
        }
        sleep(20).await;
    }
    false
}

#[wasm_bindgen_test]
async fn a_playing_clip_reports_its_size_length_and_pictures() {
    let video = crate::web::open(&VideoSource::url(clip_url())).expect("opens");
    assert!(!video.is_playing(), "a new player is paused");
    video.play();
    assert!(video.is_playing());

    let frames = FrameSlot::default();
    assert!(until(|| video.take_frame(&frames)).await, "no picture arrived");
    assert_eq!(video.size(), Some((64, 48)));
    assert!((video.duration() - 1.0).abs() < 0.2, "duration {}", video.duration());

    let frame = frames.current_frame().expect("a frame is stored");
    assert_eq!((frame.width(), frame.height()), (64, 48));
    let NativeTexture::WebVideo { id, serial } = frame.texture() else {
        panic!("a web frame names its element");
    };
    assert!(web_video::lookup(id).is_some(), "the element is registered while a frame holds it");
    assert_eq!(serial, 1);

    // The next picture is a new serial; the same picture is not stored twice.
    assert!(until(|| video.take_frame(&frames)).await, "no second picture");
    let NativeTexture::WebVideo { serial: next, .. } = frames.current_frame().unwrap().texture() else {
        panic!("a web frame names its element");
    };
    assert!(next > serial);

    assert!(video.position() > 0.0, "the playhead moves");
    video.pause();
    assert!(!video.is_playing());
}

#[wasm_bindgen_test]
async fn a_frame_keeps_its_element_registered_after_the_player_is_gone() {
    let video = crate::web::open(&VideoSource::url(clip_url())).expect("opens");
    video.play();
    let frames = FrameSlot::default();
    assert!(until(|| video.take_frame(&frames)).await, "no picture arrived");
    let frame = frames.current_frame().unwrap();
    let NativeTexture::WebVideo { id, .. } = frame.texture() else { panic!("a web frame") };

    drop(video);
    assert!(web_video::lookup(id).is_some(), "a frame still in flight can be drawn");
    drop(frames);
    drop(frame);
    assert!(web_video::lookup(id).is_none(), "the id is released with the last frame");
}

#[wasm_bindgen_test]
async fn a_broken_source_reports_an_error() {
    let video = crate::web::open(&VideoSource::url("/no/such/video.mp4")).expect("opens");
    assert!(until(|| video.error().is_some()).await, "the failure never surfaced");
    assert!(matches!(video.error(), Some(VideoError::Decode(_) | VideoError::Network(_))));
}

#[wasm_bindgen_test]
fn host_files_are_unsupported() {
    assert!(matches!(
        crate::web::open(&VideoSource::file("/tmp/clip.mp4")),
        Err(VideoError::Unsupported)
    ));
}
