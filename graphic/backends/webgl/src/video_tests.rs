//! Browser tests of the video import: a real `<video>` element, a real WebGL2
//! context, and the pixels read back from the texture the backend filled.

use std::rc::Rc;

use aimer_cupid_gpu::web_video;
use js_sys::{Array, Promise, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use web_sys::{Blob, BlobPropertyBag, HtmlCanvasElement, HtmlVideoElement, Url, WebGl2RenderingContext as Gl};

use crate::pipeline::WebGl2ViewTarget;
use crate::{WebGl2Backend, WebGl2TextureView};

wasm_bindgen_test_configure!(run_in_browser);

/// 64 x 48, ten frames: red above, blue below.
const HALVES: &[u8] = include_bytes!("../fixtures/halves.webm");

fn document() -> web_sys::Document {
    web_sys::window().expect("a window").document().expect("a document")
}

/// A canvas-backed backend, or `None` where the browser has no WebGL2.
fn backend() -> Option<WebGl2Backend> {
    let canvas: HtmlCanvasElement = document().create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(64);
    canvas.set_height(48);
    WebGl2Backend::new(canvas).ok()
}

/// A video element playing `HALVES`, resolved once its first picture is ready.
async fn halves_video() -> HtmlVideoElement {
    let bytes = Uint8Array::from(HALVES);
    let options = BlobPropertyBag::new();
    options.set_type("video/webm");
    let blob = Blob::new_with_u8_array_sequence_and_options(&Array::of1(&bytes), &options).expect("blob");
    let url = Url::create_object_url_with_blob(&blob).expect("object URL");
    let video: HtmlVideoElement = document().create_element("video").unwrap().dyn_into().unwrap();
    video.set_muted(true);
    let ready = Promise::new(&mut |resolve, _| {
        let _ = video.add_event_listener_with_callback("loadeddata", &resolve);
    });
    video.set_src(&url);
    video.load();
    JsFuture::from(ready).await.expect("the video loads");
    // Chromium produces a paused video's first picture lazily: play a little.
    let played = Promise::new(&mut |resolve, _| {
        let _ = video.add_event_listener_with_callback("timeupdate", &resolve);
    });
    JsFuture::from(video.play().expect("play")).await.expect("plays");
    JsFuture::from(played).await.expect("time advances");
    video.pause().expect("pause");
    video
}

/// The RGBA of texel (`x`, `row`) of the view's texture; row 0 is the first
/// row uploaded.
fn texel(backend: &WebGl2Backend, view: &WebGl2TextureView, x: i32, row: i32) -> [u8; 4] {
    let WebGl2ViewTarget::Texture { texture, .. } = &view.0.target else {
        panic!("a video view is a texture view");
    };
    let gl = &backend.shared.gl;
    let framebuffer = gl.create_framebuffer().expect("framebuffer");
    gl.bind_framebuffer(Gl::FRAMEBUFFER, Some(&framebuffer));
    gl.framebuffer_texture_2d(Gl::FRAMEBUFFER, Gl::COLOR_ATTACHMENT0, Gl::TEXTURE_2D, Some(&texture.0.object), 0);
    assert_eq!(gl.check_framebuffer_status(Gl::FRAMEBUFFER), Gl::FRAMEBUFFER_COMPLETE);
    let mut pixel = [0_u8; 4];
    gl.read_pixels_with_opt_u8_array(x, row, 1, 1, Gl::RGBA, Gl::UNSIGNED_BYTE, Some(&mut pixel))
        .expect("read_pixels");
    gl.bind_framebuffer(Gl::FRAMEBUFFER, None);
    gl.delete_framebuffer(Some(&framebuffer));
    pixel
}

#[wasm_bindgen_test]
async fn a_video_is_copied_into_a_texture_with_its_first_row_first() {
    let Some(backend) = backend() else { return };
    let video = halves_video().await;
    let registration = web_video::register(Rc::new(video));

    let probe = web_video::lookup(registration.id()).unwrap().downcast::<HtmlVideoElement>().unwrap();
    wasm_bindgen_test::console_log!(
        "ready {} size {}x{} t {} err {:?}",
        probe.ready_state(),
        probe.video_width(),
        probe.video_height(),
        probe.current_time(),
        probe.error().map(|e| e.message())
    );
    let view = backend.import_video(registration.id(), 1, 64, 48).expect("the picture imports");
    let gl_error = backend.shared.gl.get_error();
    wasm_bindgen_test::console_log!("gl error {gl_error}");
    let top = texel(&backend, &view, 32, 5);
    let bottom = texel(&backend, &view, 32, 42);
    // The codec is lossy, so the colours are only close.
    assert!(top[0] > 200 && top[2] < 60, "the top of the picture is red, got {top:?}");
    assert!(bottom[2] > 200 && bottom[0] < 60, "the bottom of the picture is blue, got {bottom:?}");
}

#[wasm_bindgen_test]
async fn a_picture_is_copied_once_per_serial() {
    let Some(backend) = backend() else { return };
    let video = halves_video().await;
    let registration = web_video::register(Rc::new(video.clone()));

    let first = backend.import_video(registration.id(), 1, 64, 48).expect("imports");
    let again = backend.import_video(registration.id(), 1, 64, 48).expect("imports");
    // The same serial reuses the element's texture.
    let (WebGl2ViewTarget::Texture { texture: a, .. }, WebGl2ViewTarget::Texture { texture: b, .. }) =
        (&first.0.target, &again.0.target)
    else {
        panic!("texture views");
    };
    assert!(Rc::ptr_eq(&a.0, &b.0), "one texture per video");
    // A new serial copies again, and a new size makes a new texture.
    assert!(backend.import_video(registration.id(), 2, 64, 48).is_some());
    let resized = backend.import_video(registration.id(), 3, 32, 24).expect("imports");
    let WebGl2ViewTarget::Texture { texture: c, .. } = &resized.0.target else { panic!("texture view") };
    assert!(!Rc::ptr_eq(&a.0, &c.0), "a new size is a new texture");
}

#[wasm_bindgen_test]
fn unknown_or_empty_videos_import_nothing() {
    let Some(backend) = backend() else { return };
    assert!(backend.import_video(u32::MAX, 1, 64, 48).is_none(), "no such element");
    let registration = web_video::register(Rc::new(7_u32));
    assert!(backend.import_video(registration.id(), 1, 64, 48).is_none(), "not a video element");
    assert!(backend.import_video(registration.id(), 1, 0, 0).is_none(), "no picture yet");
}
