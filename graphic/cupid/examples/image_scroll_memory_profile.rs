//! Logical GPU image-cache usage during a long scrolling workload.
//!
//! The profile renders a moving window of unique reconstructible images into
//! an offscreen target. It reports the peak cache size during the scroll and
//! the settled size after the final viewport has remained visible for longer
//! than the idle-eviction grace period.
//!
//! Run with:
//!
//! ```text
//! cargo run -p aimer_cupid --example image_scroll_memory_profile --release
//! ```

use std::hint::black_box;

use aimer_cupid::backend::wgpu::WgpuBackend;
use aimer_cupid::draw_cmd::DrawList;
use aimer_cupid::renderer::{Renderer, RendererMemoryStats};
use aimer_cupid::utilities::Rect;
use aimer_utils::SyncFuture;

const SURFACE_WIDTH: u32 = 512;
const SURFACE_HEIGHT: u32 = 512;
const IMAGE_WIDTH: u32 = 256;
const IMAGE_HEIGHT: u32 = 256;
const VISIBLE_IMAGES: usize = 4;
const SCROLL_FRAMES: usize = 180;
const SETTLE_FRAMES: usize = 121;
const IMAGE_BYTES: usize = IMAGE_WIDTH as usize * IMAGE_HEIGHT as usize * 4;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .block()
        .ok()?;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("image scroll memory profile device"),
            ..Default::default()
        })
        .block()
        .ok()
}

/// Generates a deterministic image whose sampled bytes identify its position
/// in the scroll. `DrawList::load_image` hashes the beginning and end of the
/// payload, so both regions carry the same identifier.
fn image_data(index: usize) -> Vec<u8> {
    let value = index as u32;
    let mut data = vec![0; IMAGE_BYTES];
    for pixel in data.chunks_exact_mut(4) {
        pixel[0] = (value & 0xff) as u8;
        pixel[1] = ((value >> 8) & 0xff) as u8;
        pixel[2] = ((value >> 16) & 0xff) as u8;
        pixel[3] = 0xff;
    }
    data
}

fn render_viewport(
    renderer: &mut Renderer<WgpuBackend>,
    backend: &WgpuBackend,
    view: &wgpu::TextureView,
    first_image: usize,
) -> RendererMemoryStats {
    let mut draw = DrawList::new();
    let image_width = SURFACE_WIDTH as f32 / VISIBLE_IMAGES as f32;
    for slot in 0..VISIBLE_IMAGES {
        let index = first_image + slot;
        let bytes = image_data(index);
        let texture_id = draw.load_image(&bytes, IMAGE_WIDTH, IMAGE_HEIGHT);
        draw.draw_image(
            Rect::new(
                slot as f32 * image_width,
                0.0,
                image_width,
                SURFACE_HEIGHT as f32,
            ),
            texture_id,
        );
    }
    renderer.render(backend, view, SURFACE_WIDTH, SURFACE_HEIGHT, false, &draw);
    drop(draw);
    black_box(renderer.memory_stats())
}

fn main() {
    let Some((device, queue)) = gpu() else {
        eprintln!("skipping: no GPU adapter available");
        return;
    };

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("image scroll memory profile target"),
        size: wgpu::Extent3d {
            width: SURFACE_WIDTH,
            height: SURFACE_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, wgpu::TextureFormat::Rgba8Unorm);

    let mut peak = RendererMemoryStats::default();
    for frame in 0..SCROLL_FRAMES {
        let stats = render_viewport(&mut renderer, &backend, &view, frame * VISIBLE_IMAGES);
        peak.image_texture_count = peak.image_texture_count.max(stats.image_texture_count);
        peak.image_texture_bytes = peak.image_texture_bytes.max(stats.image_texture_bytes);
    }

    let final_viewport = (SCROLL_FRAMES - 1) * VISIBLE_IMAGES;
    let mut settled = RendererMemoryStats::default();
    for _ in 0..SETTLE_FRAMES {
        settled = render_viewport(&mut renderer, &backend, &view, final_viewport);
    }
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device to finish the image profile");

    println!(
        "image scroll memory profile: {SCROLL_FRAMES} scroll frames, {SETTLE_FRAMES} settle frames, {VISIBLE_IMAGES} visible images, {IMAGE_WIDTH}x{IMAGE_HEIGHT} RGBA8"
    );
    println!(
        "peak logical image cache:     {} textures, {} bytes",
        peak.image_texture_count, peak.image_texture_bytes
    );
    println!(
        "settled logical image cache:  {} textures, {} bytes",
        settled.image_texture_count, settled.image_texture_bytes
    );
}
