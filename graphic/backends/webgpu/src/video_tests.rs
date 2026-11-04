//! Browser tests of the video import: a real `<video>` element, a real WebGPU
//! device, and the pixels read back from the texture the backend filled.
//!
//! A browser without WebGPU passes these vacuously.

use std::rc::Rc;

use aimer_cupid_gpu::web_video;
use js_sys::{Array, Promise, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use web_sys::{Blob, BlobPropertyBag, HtmlCanvasElement, HtmlVideoElement, Url};

use crate::backend::{
    BufferDescriptor, BufferUsage, Extent3d, GpuBackend, Origin3d, TexelCopyBufferInfo, TexelCopyBufferLayout,
    TexelCopyTextureInfo, TextureAspect,
};
use crate::pipeline::WebGpuViewTarget;
use crate::WebGpuBackend;

wasm_bindgen_test_configure!(run_in_browser);

/// 64 x 48, ten frames: red above, blue below.
const HALVES: &[u8] = include_bytes!("../fixtures/halves.webm");
/// `64 * 4` bytes per row, already a multiple of WebGPU's 256-byte alignment.
const ROW_BYTES: u32 = 256;

fn document() -> web_sys::Document {
    web_sys::window().expect("a window").document().expect("a document")
}

async fn backend() -> Option<WebGpuBackend> {
    let canvas: HtmlCanvasElement = document().create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(64);
    canvas.set_height(48);
    WebGpuBackend::new(canvas).await.ok()
}

/// A video element that has shown its first pictures.
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

#[wasm_bindgen_test]
async fn a_video_is_copied_into_a_texture_with_its_first_row_first() {
    let Some(backend) = backend().await else { return };
    let video = halves_video().await;
    let registration = web_video::register(Rc::new(video));

    let view = backend.import_video(registration.id(), 1, 64, 48).expect("the picture imports");
    let WebGpuViewTarget::Texture { _texture: texture, .. } = &view.0.target else {
        panic!("a video view is a texture view");
    };
    let buffer = backend.create_buffer(&BufferDescriptor {
        label: Some("readback".into()),
        size: u64::from(ROW_BYTES) * 48,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("readback");
    backend.copy_texture_to_buffer(
        &mut encoder,
        &TexelCopyTextureInfo { texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
        &TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(48),
            },
        },
        Extent3d { width: 64, height: 48, depth_or_array_layers: 1 },
    );
    backend.submit(encoder);
    let bytes = backend.read_buffer_async(&buffer, u64::from(ROW_BYTES) * 48).await.expect("readback");

    let texel = |x: usize, row: usize| {
        let at = row * ROW_BYTES as usize + x * 4;
        [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
    };
    let (top, bottom) = (texel(32, 5), texel(32, 42));
    // The codec is lossy, so the colours are only close.
    assert!(top[0] > 200 && top[2] < 60, "the top of the picture is red, got {top:?}");
    assert!(bottom[2] > 200 && bottom[0] < 60, "the bottom of the picture is blue, got {bottom:?}");
}

#[wasm_bindgen_test]
async fn a_picture_is_copied_once_per_serial_and_unknown_videos_import_nothing() {
    let Some(backend) = backend().await else { return };
    let video = halves_video().await;
    let registration = web_video::register(Rc::new(video));

    let texture_of = |view: &crate::WebGpuTextureView| match &view.0.target {
        WebGpuViewTarget::Texture { _texture, .. } => Rc::as_ptr(&_texture.0),
        WebGpuViewTarget::Surface(_) => panic!("a video view is a texture view"),
    };
    let first = backend.import_video(registration.id(), 1, 64, 48).expect("imports");
    let again = backend.import_video(registration.id(), 1, 64, 48).expect("imports");
    assert_eq!(texture_of(&first), texture_of(&again), "one texture per video");
    let resized = backend.import_video(registration.id(), 2, 32, 24).expect("imports");
    assert_ne!(texture_of(&first), texture_of(&resized), "a new size is a new texture");

    assert!(backend.import_video(u32::MAX, 1, 64, 48).is_none(), "no such element");
    assert!(backend.import_video(registration.id(), 1, 0, 0).is_none(), "no picture yet");
}
