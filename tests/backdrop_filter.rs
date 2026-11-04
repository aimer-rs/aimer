#![cfg(feature = "wgpu")]

#[path = "backdrop_filter/fixtures.rs"]
mod fixtures;
#[path = "backdrop_filter/wide.rs"]
mod wide;
use fixtures::*;

use aimer::canvas::Canvas as RetainedCanvas;
use aimer_cupid::WgpuBackend;
use aimer_cupid::draw_cmd::{Rect, RenderPaintSource};
use aimer_cupid::frame::FramePacket;
use aimer_cupid::renderer::Renderer;
use aimer::{BackdropFilter, Decoration, ResolvedSize, Vec2d};
use aimer_utils::SyncFuture;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[test]
fn backdrop_filter_blends_thin_features_into_a_smooth_gaussian() {
    let (device, queue) = gpu().expect("a GPU is required for backdrop filter pixel validation");
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    for antialiasing in [
        aimer_cupid::AntiAlias::Analytic,
        aimer_cupid::AntiAlias::Msaa4x,
    ] {
        let mut renderer = Renderer::with_antialiasing(&backend, FORMAT, antialiasing);
        let (tree, node) = backdrop_tree();
        for scale in [1.0, 2.0] {
            paint_thin_cross(&tree, scale);
            for radius in [0.5, 1.0, 12.0, 48.0, 96.0] {
                let packet = filter_packet_scaled(
                    &tree,
                    node,
                    BackdropFilter::new().blur_radius(radius),
                    scale,
                );
                let pixels =
                    render_packet_and_read(&device, &queue, &backend, &mut renderer, &packet);
                assert_gaussian_cross(&pixels, radius, scale);
            }
        }
    }
}

#[test]
fn backdrop_filter_keeps_ordered_surfaces_correct_when_uniforms_and_intermediates_grow() {
    let (device, queue) = gpu().expect("a GPU is required for backdrop filter pixel validation");
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let (tree, node) = backdrop_tree();
    let root = background_node(&tree);
    let background = RetainedCanvas::of(&tree.context(root).unwrap());
    background.fill_rect(
        Rect::new(0.0, 0.0, FRAME_WIDTH as f32, FRAME_HEIGHT as f32),
        [128, 64, 32, 255],
    );
    background.finish();
    let packet = filter_packet(
        &tree,
        node,
        BackdropFilter::new().blur_radius(12.0).brightness(0.0),
    );
    render_packet_and_read(&device, &queue, &backend, &mut renderer, &packet);

    // More than sixteen requests grows both dynamic uniform buffers. Each
    // ordered capture reuses the same intermediate, with different origins.
    for index in 0..20 {
        let sibling = tree
            .add_child(
                root,
                Rect::new((FILTER_X + 96 + index * 48) as f32, 16.0, 16.0, 16.0),
            )
            .unwrap();
        tree.set_paint_source(sibling, RenderPaintSource::LocalV2)
            .unwrap();
        let canvas = RetainedCanvas::of(&tree.context(sibling).unwrap());
        BackdropFilter::new()
            .blur_radius(12.0)
            .brightness(0.5)
            .paint(
                &canvas,
                Vec2d::default(),
                ResolvedSize {
                    width: 16.0,
                    height: 16.0,
                },
                1.0,
            );
        canvas.finish();
    }
    // Grow the shared capture and intermediate without changing their format.
    let wide = tree
        .add_child(root, Rect::new(FILTER_X as f32, 48.0, 256.0, 8.0))
        .unwrap();
    tree.set_paint_source(wide, RenderPaintSource::LocalV2)
        .unwrap();
    let canvas = RetainedCanvas::of(&tree.context(wide).unwrap());
    BackdropFilter::new()
        .blur_radius(24.0)
        .brightness(0.0)
        .paint(
            &canvas,
            Vec2d::default(),
            ResolvedSize {
                width: 256.0,
                height: 8.0,
            },
            1.0,
        );
    canvas.finish();
    for _ in 0..2 {
        let pixels = render_packet_and_read(
            &device,
            &queue,
            &backend,
            &mut renderer,
            &packet_from_tree(&tree, 1.0),
        );
        for index in 0..20 {
            let actual = pixel(&pixels, FILTER_X + 104 + index * 48, 24);
            assert_eq!(actual[3], 255);
            assert!(
                actual[..3]
                    .iter()
                    .zip([64u8, 32, 16])
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 1),
                "ordered surface {index} lost its capture after buffer growth: {actual:?}"
            );
        }
        assert_eq!(pixel(&pixels, FILTER_X + 128, 52), [0, 0, 0, 255]);
    }
}

#[test]
fn backdrop_filter_changes_colors_without_blur_or_glass_styling() {
    let (device, queue) = gpu().expect("a GPU is required for backdrop filter pixel validation");
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let (tree, node) = backdrop_tree();
    let packet = filter_packet(
        &tree,
        node,
        BackdropFilter::new().blur_radius(0.0).brightness(0.5),
    );
    let pixels = render_packet_and_read(&device, &queue, &backend, &mut renderer, &packet);
    assert_eq!(pixel(&pixels, FILTER_X + 8, 32), [0, 0, 0, 255]);
    let white = pixel(&pixels, FILTER_X + FILTER_SIZE - 8, 32);
    assert!(
        white[..3].iter().all(|channel| channel.abs_diff(128) <= 2),
        "brightness-only filter: {white:?}"
    );
    assert_eq!(
        pixel(&pixels, FILTER_X + FILTER_SIZE / 2 - 1, 32),
        [0, 0, 0, 255],
        "a color-only filter must preserve the sharp backdrop boundary"
    );
    assert_eq!(pixel(&pixels, FILTER_X - 1, 32), [0, 0, 0, 0]);
}

#[test]
fn backdrop_filter_preserves_transparent_and_translucent_backdrops() {
    let (device, queue) = gpu().expect("a GPU is required for backdrop filter pixel validation");
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let (tree, node) = backdrop_tree();
    let root = background_node(&tree);
    let canvas = RetainedCanvas::of(&tree.context(root).unwrap());
    canvas.fill_rect(
        Rect::new(FILTER_X as f32 + 32.0, 0.0, 32.0, 64.0),
        [255, 0, 0, 128],
    );
    canvas.finish();
    let packet = filter_packet(
        &tree,
        node,
        BackdropFilter::new().blur_radius(0.0).saturation(0.0),
    );
    let pixels = render_packet_and_read(&device, &queue, &backend, &mut renderer, &packet);
    assert_eq!(pixel(&pixels, FILTER_X + 8, 32), [0, 0, 0, 0]);
    let gray = pixel(&pixels, FILTER_X + 48, 32);
    assert_eq!(
        gray[3], 128,
        "the filter must preserve backdrop alpha: {gray:?}"
    );
    assert!(
        gray[..3].iter().all(|channel| channel.abs_diff(27) <= 2),
        "filter unpremultiplied colors: {gray:?}"
    );
}

#[test]
fn backdrop_filter_blur_radius_scales_with_device_pixels() {
    let (device, queue) = gpu().expect("a GPU is required for backdrop filter pixel validation");
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let (tree, node) = backdrop_tree();
    let packet = filter_packet_scaled(&tree, node, BackdropFilter::new(), 2.0);
    let pixels = render_packet_and_read(&device, &queue, &backend, &mut renderer, &packet);
    let softened = pixel(&pixels, (FILTER_X + 32) * 2 - 8, 32);
    assert!(
        softened[0] >= 20,
        "16 logical pixels of blur must reach 8 device pixels across this edge: {softened:?}"
    );
}

#[test]
fn backdrop_filter_resamples_changed_background_without_recording_its_commands() {
    let (device, queue) = gpu().expect("a GPU is required for backdrop filter pixel validation");
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let (tree, node) = backdrop_tree();
    paint_background(&tree, [128, 64, 32, 255]);
    let packet = filter_packet(
        &tree,
        node,
        BackdropFilter::new()
            .blur_radius(0.0)
            .contrast(2.0)
            .brightness(0.5),
    );
    let revision = tree.draw_list_revision(node).unwrap();
    let pixels = render_packet_and_read(&device, &queue, &backend, &mut renderer, &packet);
    let first = pixel(&pixels, FILTER_X + 32, 32);
    assert!(
        first[0].abs_diff(64) <= 1 && first[1] <= 1 && first[2] == 0,
        "contrast then brightness: {first:?}"
    );
    paint_background(&tree, [64, 128, 192, 255]);
    let pixels = render_packet_and_read(
        &device,
        &queue,
        &backend,
        &mut renderer,
        &packet_from_tree(&tree, 1.0),
    );
    let second = pixel(&pixels, FILTER_X + 32, 32);
    assert!(
        second[0] <= 1 && second[1].abs_diff(64) <= 1 && second[2].abs_diff(128) <= 1,
        "fresh backdrop: {second:?}"
    );
    assert_eq!(tree.draw_list_revision(node).unwrap(), revision);
}

#[test]
fn backdrop_filter_obeys_clips_and_keeps_foreground_sharp() {
    let (device, queue) = gpu().expect("a GPU is required for backdrop filter pixel validation");
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let (tree, node) = backdrop_tree();
    // Cover the full Gaussian support so alpha diffusion at the background's
    // edge cannot mask a clipping or foreground-paint regression.
    let background = RetainedCanvas::of(&tree.context(background_node(&tree)).unwrap());
    background.fill_rect(
        Rect::new(FILTER_X as f32 - 16.0, 0.0, 96.0, 64.0),
        [40, 80, 160, 255],
    );
    background.finish();
    tree.set_clip(node, Some(Rect::new(0.0, 0.0, 32.0, 64.0)))
        .unwrap();
    let child = tree
        .add_child(node, Rect::new(12.0, 12.0, 8.0, 8.0))
        .unwrap();
    tree.set_paint_source(child, RenderPaintSource::LocalV2)
        .unwrap();
    let canvas = RetainedCanvas::of(&tree.context(child).unwrap());
    canvas.fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), [0, 255, 0, 255]);
    canvas.finish();
    let packet = filter_packet(&tree, node, BackdropFilter::new().brightness(0.0));
    let pixels = render_packet_and_read(&device, &queue, &backend, &mut renderer, &packet);
    assert_eq!(pixel(&pixels, FILTER_X + 8, 32), [0, 0, 0, 255]);
    assert_eq!(pixel(&pixels, FILTER_X + 48, 32), [40, 80, 160, 255]);
    assert_eq!(pixel(&pixels, FILTER_X + 16, 16), [0, 255, 0, 255]);
    assert_eq!(pixel(&pixels, FILTER_X + 21, 16), [0, 0, 0, 255]);
}

#[test]
fn backdrop_filter_budget_fallback_leaves_the_backdrop_unchanged() {
    let (device, queue) = gpu().expect("a GPU is required for backdrop filter pixel validation");
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let (tree, node) = backdrop_tree();
    let canvas = RetainedCanvas::of(&tree.context(node).unwrap());
    // The bounds fit the pixel budget exactly, but the blur margin adds
    // one row of capture and exceeds that budget. Exercise the shader's
    // unavailable-capture path without relying on the former 2,048-width cap.
    BackdropFilter::new().brightness(0.0).paint(
        &canvas,
        Vec2d {
            x: -(FILTER_X as f32),
            y: 0.0,
        },
        ResolvedSize {
            width: 4_096.0,
            height: 1_024.0,
        },
        1.0,
    );
    canvas.finish();
    let packet = FramePacket::from_v2_direct(
        aimer_cupid::draw_cmd::RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, 4096.0, 1025.0)],
            operations: tree.render_all(),
        },
        aimer_cupid::frame::FrameRenderMetadata::new(
            1.0,
            0,
            0,
            0,
            0,
            aimer_cupid::damage_region::DamageSet::new(4096, 1025),
        ),
    )
    .unwrap();
    let pixels = render_packet_and_read_sized(
        &device,
        &queue,
        &backend,
        &mut renderer,
        &packet,
        4096,
        1025,
    );
    assert_eq!(pixel(&pixels, FILTER_X + 8, 32), [0, 0, 0, 255]);
    assert_eq!(pixel(&pixels, FILTER_X + 48, 32), [255, 255, 255, 255]);
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .block()
        .ok()?;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("backdrop filter regression device"),
            ..Default::default()
        })
        .block()
        .ok()
}

#[test]
fn backdrop_filter_wide_ipad_toolbar() {
    let (device, queue) = gpu().expect("GPU for iPad-width backdrop validation");
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    for aa in [
        aimer_cupid::AntiAlias::Analytic,
        aimer_cupid::AntiAlias::Msaa4x,
    ] {
        let mut renderer = Renderer::with_antialiasing(&backend, FORMAT, aa);
        wide::wide_toolbar_blurs_and_refreshes(|packet| {
            render_packet_and_read_sized(
                &device,
                &queue,
                &backend,
                &mut renderer,
                packet,
                wide::WIDTH,
                wide::HEIGHT,
            )
        });
    }
}

fn render_packet_and_read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    backend: &WgpuBackend,
    renderer: &mut Renderer,
    packet: &FramePacket,
) -> Vec<u8> {
    render_packet_and_read_sized(
        device,
        queue,
        backend,
        renderer,
        packet,
        FRAME_WIDTH,
        FRAME_HEIGHT,
    )
}

fn render_packet_and_read_sized(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    backend: &WgpuBackend,
    renderer: &mut Renderer,
    packet: &FramePacket,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("backdrop filter regression target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    renderer.render_packet(backend, &view, packet, false);

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("backdrop filter regression readback"),
        size: (width * height * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| {
        result.expect("backdrop filter readback buffer to map");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("backdrop filter readback to finish");
    let pixels = slice
        .get_mapped_range()
        .expect("backdrop filter readback range")
        .to_vec();
    readback.unmap();
    pixels
}
