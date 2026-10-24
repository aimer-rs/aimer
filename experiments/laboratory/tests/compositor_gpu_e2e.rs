//! End-to-end GPU acceptance test for Aimer's retained compositor.
//!
//! This test deliberately crosses the framework seams, from the retained render
//! tree to the GPU:
//!
//! `RenderTree -> RenderFrame -> FramePacket -> Renderer -> GPU`
//!
//! Run it on a macOS machine with a visible, host GPU session:
//!
//! ```text
//! AIMER_REQUIRE_GPU_E2E=1 WGPU_BACKENDS=metal WGPU_POWER_PREF=high cargo test -p aimer_laboratory --features wgpu --test compositor_gpu_e2e -- --nocapture
//! ```

use std::sync::OnceLock;

use aimer::cupid::WgpuBackend;
use aimer::cupid::damage_region::DamageSet;
use aimer::cupid::draw_cmd_v2::{Rect as V2Rect, RenderNodeSpec, RenderTree};
use aimer::cupid::frame::{Frame, FramePacket, FrameRenderMetadata};
use aimer::cupid::renderer::Renderer;
use aimer_laboratory::v2_render::Canvas as V2Canvas;
use tokio::runtime::Runtime;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("create compositor test runtime")
    })
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = runtime()
        .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: true,
        }))
        .ok()?;
    let info = adapter.get_info();
    let (device, queue) = runtime()
        .block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("aimer compositor end-to-end test"),
            ..Default::default()
        }))
        .ok()?;
    Some((device, queue, info))
}

fn gpu_any() -> Option<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let request_adapter = |force_fallback_adapter, power_preference| {
        instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface: None,
            force_fallback_adapter,
            apply_limit_buckets: true,
        })
    };
    let adapter = runtime()
        .block_on(request_adapter(true, wgpu::PowerPreference::LowPower))
        .or_else(|_| {
            runtime().block_on(request_adapter(
                false,
                wgpu::PowerPreference::HighPerformance,
            ))
        })
        .ok()?;
    let info = adapter.get_info();
    let (device, queue) = runtime()
        .block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("aimer opacity-group compositor acceptance test"),
            ..Default::default()
        }))
        .ok()?;
    Some((device, queue, info))
}

fn read_target(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) -> Vec<u8> {
    read_target_sized(device, queue, target, WIDTH, HEIGHT)
}

fn read_target_sized(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let unpadded_bytes_per_row = width * 4;
    let bytes_per_row = unpadded_bytes_per_row.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("aimer compositor end-to-end readback"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("aimer compositor end-to-end readback encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
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
        result.expect("map compositor readback buffer");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for compositor readback");
    let mapped = slice
        .get_mapped_range()
        .expect("map compositor readback range");
    let mut pixels = vec![0; (unpadded_bytes_per_row * height) as usize];
    for row in 0..height as usize {
        let source_start = row * bytes_per_row as usize;
        let target_start = row * unpadded_bytes_per_row as usize;
        pixels[target_start..target_start + unpadded_bytes_per_row as usize]
            .copy_from_slice(&mapped[source_start..source_start + unpadded_bytes_per_row as usize]);
    }
    drop(mapped);
    readback.unmap();
    pixels
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    pixel_sized(pixels, WIDTH, x, y)
}

fn pixel_sized(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * width + x) * 4) as usize;
    pixels[offset..offset + 4].try_into().expect("RGBA pixel")
}

fn assert_pixel(pixels: &[u8], x: u32, y: u32, expected: [u8; 4]) {
    assert_pixel_sized(pixels, WIDTH, x, y, expected);
}

fn assert_pixel_sized(pixels: &[u8], width: u32, x: u32, y: u32, expected: [u8; 4]) {
    let actual = pixel_sized(pixels, width, x, y);
    for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert!(
            actual.abs_diff(expected) <= 2,
            "pixel ({x}, {y}) channel {channel}: expected {expected}, got {actual}"
        );
    }
}

fn record_v2_fill(
    tree: &RenderTree,
    element: aimer::cupid::draw_cmd_v2::RenderNodeId,
    rect: V2Rect,
    color: [u8; 4],
) {
    let context = tree.context(element).expect("v2 element context");
    let canvas = V2Canvas::of(&context);
    canvas.fill_rect(rect, color);
    canvas.finish();
}

fn direct_v2_packet(tree: &RenderTree, damage: Vec<V2Rect>) -> FramePacket {
    FramePacket::from_v2_direct_with_frame(
        aimer::cupid::draw_cmd_v2::RenderFrame {
            damage,
            operations: tree.render_all(),
        },
        FrameRenderMetadata::new(1.0, 84, 1, 1, 1, DamageSet::new(WIDTH, HEIGHT)),
        Frame::new(aimer::cupid::draw_cmd::DrawList::new(), WIDTH, HEIGHT),
    )
    .expect("build a complete virtualized-scroll packet")
}

#[test]
fn direct_retained_packet_interleaves_clipped_local_v2_lists_on_metal() {
    let require_gpu = std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_some();
    let Some((device, queue, adapter_info)) = gpu() else {
        if require_gpu {
            panic!("Metal adapter unavailable; run this test in a host GPU session");
        }
        eprintln!("skipping: Metal adapter unavailable");
        return;
    };
    assert_eq!(adapter_info.backend, wgpu::Backend::Metal);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("aimer direct retained packet target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let tree = RenderTree::new();
    let root = tree
        .add_root(V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32))
        .unwrap();
    let legacy = tree
        .add_child(root, V2Rect::new(16.0, 16.0, 32.0, 32.0))
        .unwrap();
    // The child paints a 32x32 square but only its top half is inside its clip.
    tree.set_clip(legacy, Some(V2Rect::new(0.0, 0.0, 32.0, 16.0)))
        .unwrap();
    record_v2_fill(
        &tree,
        root,
        V2Rect::new(0.0, 0.0, 64.0, 64.0),
        [220, 20, 30, 255],
    );
    record_v2_fill(
        &tree,
        legacy,
        V2Rect::new(0.0, 0.0, 32.0, 32.0),
        [20, 210, 60, 255],
    );

    let packet = FramePacket::from_v2_direct_with_frame(
        aimer::cupid::draw_cmd_v2::RenderFrame {
            damage: vec![V2Rect::new(0.0, 0.0, 64.0, 64.0)],
            operations: tree.render_all(),
        },
        FrameRenderMetadata::new(1.0, 72, 1, 1, 1, DamageSet::new(WIDTH, HEIGHT)),
        Frame::new(aimer::cupid::draw_cmd::DrawList::new(), WIDTH, HEIGHT),
    )
    .expect("build a retained packet without flattening local v2 commands");
    assert!(packet.render_plan().unwrap().is_complete());
    assert!(packet.frame().draw_list.commands().is_empty());

    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    renderer.render_packet(&backend, &view, &packet, false);
    let pixels = read_target(&device, &queue, &target);
    assert_pixel(&pixels, 8, 8, [220, 20, 30, 255]);
    assert_pixel(&pixels, 24, 24, [20, 210, 60, 255]);
    assert_pixel(&pixels, 24, 40, [220, 20, 30, 255]);
    assert_pixel(&pixels, 56, 56, [220, 20, 30, 255]);

    let resized_target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("aimer resized direct retained target"),
        size: wgpu::Extent3d {
            width: WIDTH + 16,
            height: HEIGHT + 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let resized_view = resized_target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer.render_packet_at_size(
        &backend,
        &resized_view,
        &packet,
        WIDTH + 16,
        HEIGHT + 16,
        false,
    );
    assert!(renderer.compositor_stats().full_repaint);
    let resized_pixels = read_target(&device, &queue, &resized_target);
    assert_pixel(&resized_pixels, 8, 8, [220, 20, 30, 255]);
    assert_pixel(&resized_pixels, 24, 24, [20, 210, 60, 255]);
    assert_pixel(&resized_pixels, 24, 40, [220, 20, 30, 255]);
    assert_pixel(&resized_pixels, 56, 56, [220, 20, 30, 255]);
}

#[test]
fn retained_v2_packet_updates_only_its_damaged_pixels_on_metal() {
    let require_gpu = std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_some();
    let Some((device, queue, adapter_info)) = gpu() else {
        if require_gpu {
            panic!("Metal adapter unavailable; run this test in a host GPU session");
        }
        eprintln!("skipping: Metal adapter unavailable");
        return;
    };
    assert_eq!(adapter_info.backend, wgpu::Backend::Metal);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("aimer retained v2 frame target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let tree = RenderTree::new();
    let root = tree
        .add_root(V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32))
        .unwrap();
    let child = tree
        .add_child(root, V2Rect::new(16.0, 16.0, 32.0, 32.0))
        .unwrap();
    record_v2_fill(
        &tree,
        root,
        V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
        [220, 20, 30, 255],
    );
    record_v2_fill(
        &tree,
        child,
        V2Rect::new(0.0, 0.0, 32.0, 32.0),
        [20, 50, 220, 255],
    );

    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let metadata = |damage| FrameRenderMetadata::new(1.0, 42, 1, 1, 1, damage);
    let first_packet = FramePacket::from_v2_direct(
        aimer::cupid::draw_cmd_v2::RenderFrame {
            damage: vec![V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32)],
            operations: tree.render_all(),
        },
        metadata(DamageSet::new(WIDTH, HEIGHT)),
    )
    .expect("lower initial v2 render plan");
    assert!(first_packet.frame().draw_list.commands().is_empty());
    assert!(first_packet.render_plan().unwrap().is_complete());
    let _ = tree.take_damage();
    renderer.render_packet(&backend, &view, &first_packet, false);
    let first_pixels = read_target(&device, &queue, &target);
    assert_pixel(&first_pixels, 8, 8, [220, 20, 30, 255]);
    assert_pixel(&first_pixels, 32, 32, [20, 50, 220, 255]);

    record_v2_fill(
        &tree,
        child,
        V2Rect::new(0.0, 0.0, 32.0, 32.0),
        [20, 210, 60, 255],
    );
    let update_damage = tree.take_damage();
    assert_eq!(update_damage, vec![V2Rect::new(16.0, 16.0, 32.0, 32.0)]);
    let second_packet = FramePacket::from_v2_direct_with_frame(
        aimer::cupid::draw_cmd_v2::RenderFrame {
            damage: update_damage,
            operations: tree.render_all(),
        },
        metadata(DamageSet::new(WIDTH, HEIGHT)),
        Frame::new(aimer::cupid::draw_cmd::DrawList::new(), WIDTH, HEIGHT),
    )
    .expect("lower incremental v2 render plan");
    assert!(second_packet.frame().draw_list.commands().is_empty());
    assert!(second_packet.render_plan().unwrap().is_complete());
    assert!(second_packet.render_plan().unwrap().is_complete());
    renderer.render_packet(&backend, &view, &second_packet, false);
    let second_pixels = read_target(&device, &queue, &target);
    assert_pixel(&second_pixels, 8, 8, [220, 20, 30, 255]);
    assert_pixel(&second_pixels, 32, 32, [20, 210, 60, 255]);
    assert!(!renderer.compositor_stats().full_repaint);
}

#[test]
fn nested_opacity_groups_composite_overlapping_children_and_clip_damage_on_available_gpu() {
    let require_gpu = std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_some();
    let Some((device, queue, adapter_info)) = gpu_any() else {
        if require_gpu {
            panic!("GPU adapter unavailable; run this test in a host GPU session");
        }
        eprintln!("skipping: GPU adapter unavailable");
        return;
    };
    println!("opacity-group backend: {:?}", adapter_info.backend);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("aimer nested opacity group target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let tree = RenderTree::new();
    let root = tree
        .add_root(V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32))
        .unwrap();
    tree.set_clip(root, Some(V2Rect::new(0.0, 0.0, 64.0, 64.0)))
        .unwrap();
    tree.set_opacity(root, 0.5).unwrap();
    let nested = tree
        .add_child(root, V2Rect::new(8.0, 8.0, 56.0, 56.0))
        .unwrap();
    tree.set_clip(nested, Some(V2Rect::new(0.0, 0.0, 40.0, 40.0)))
        .unwrap();
    tree.set_opacity(nested, 0.5).unwrap();
    let green = tree
        .add_child(nested, V2Rect::new(8.0, 8.0, 32.0, 32.0))
        .unwrap();
    let blue = tree
        .add_child(nested, V2Rect::new(24.0, 24.0, 40.0, 40.0))
        .unwrap();
    let edge_group = tree
        .add_child(root, V2Rect::new(48.0, 48.0, 32.0, 32.0))
        .unwrap();
    tree.set_clip(edge_group, Some(V2Rect::new(0.0, 0.0, 16.0, 16.0)))
        .unwrap();
    tree.set_opacity(edge_group, 0.5).unwrap();
    let yellow = tree
        .add_child(edge_group, V2Rect::new(0.0, 0.0, 32.0, 32.0))
        .unwrap();
    record_v2_fill(
        &tree,
        green,
        V2Rect::new(0.0, 0.0, 32.0, 32.0),
        [0, 255, 0, 255],
    );
    record_v2_fill(
        &tree,
        blue,
        V2Rect::new(0.0, 0.0, 40.0, 40.0),
        [0, 0, 255, 255],
    );
    record_v2_fill(
        &tree,
        yellow,
        V2Rect::new(0.0, 0.0, 32.0, 32.0),
        [255, 255, 0, 255],
    );

    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let metadata = |damage| FrameRenderMetadata::new(1.0, 93, 1, 1, 1, damage);
    let first_packet = FramePacket::from_v2_direct(
        aimer::cupid::draw_cmd_v2::RenderFrame {
            damage: vec![V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32)],
            operations: tree.render_all(),
        },
        metadata(DamageSet::new(WIDTH, HEIGHT)),
    )
    .expect("lower nested opacity groups");
    let first_plan = first_packet.render_plan().unwrap();
    assert!(first_plan.is_complete());
    let green_revision = first_plan.local_v2_revision(green).unwrap();
    let blue_revision = first_plan.local_v2_revision(blue).unwrap();
    let yellow_revision = first_plan.local_v2_revision(yellow).unwrap();
    renderer.render_packet(&backend, &view, &first_packet, false);
    let first_pixels = read_target(&device, &queue, &target);
    assert_pixel(&first_pixels, 20, 20, [0, 64, 0, 64]);
    assert_pixel(&first_pixels, 40, 40, [0, 0, 64, 64]);
    assert_pixel(&first_pixels, 50, 40, [0, 0, 0, 0]);
    assert_pixel(&first_pixels, 62, 62, [64, 64, 0, 64]);
    let _ = tree.take_damage();

    tree.set_opacity(nested, 0.25).unwrap();
    let update_damage = tree.take_damage();
    assert!(!update_damage.is_empty());
    assert!(update_damage.iter().all(|rect| {
        rect.x >= 0.0
            && rect.y >= 0.0
            && rect.x + rect.width <= 48.0
            && rect.y + rect.height <= 48.0
    }));
    let second_packet = FramePacket::from_v2_direct_with_frame(
        aimer::cupid::draw_cmd_v2::RenderFrame {
            damage: update_damage,
            operations: tree.render_all(),
        },
        metadata(DamageSet::new(WIDTH, HEIGHT)),
        Frame::new(aimer::cupid::draw_cmd::DrawList::new(), WIDTH, HEIGHT),
    )
    .expect("lower the opacity-only update");
    let second_plan = second_packet.render_plan().unwrap();
    assert_eq!(second_plan.local_v2_revision(green), Some(green_revision));
    assert_eq!(second_plan.local_v2_revision(blue), Some(blue_revision));
    assert_eq!(second_plan.local_v2_revision(yellow), Some(yellow_revision));
    renderer.render_packet(&backend, &view, &second_packet, false);
    let second_pixels = read_target(&device, &queue, &target);
    assert_pixel(&second_pixels, 20, 20, [0, 32, 0, 32]);
    assert_pixel(&second_pixels, 40, 40, [0, 0, 32, 32]);
    assert_pixel(&second_pixels, 50, 40, [0, 0, 0, 0]);
    assert_pixel(&second_pixels, 62, 62, [64, 64, 0, 64]);
    assert_eq!(
        pixel(&second_pixels, 2, 2),
        pixel(&first_pixels, 2, 2),
        "opacity damage leaves pixels outside the clipped group unchanged"
    );
    assert!(!renderer.compositor_stats().full_repaint);

    tree.set_opacity(edge_group, 0.25).unwrap();
    let edge_damage = tree.take_damage();
    assert!(
        edge_damage.iter().any(|rect| {
            (rect.x + rect.width - WIDTH as f32).abs() < f32::EPSILON
                && (rect.y + rect.height - HEIGHT as f32).abs() < f32::EPSILON
        }),
        "edge group damage reaches both target edges: {edge_damage:?}"
    );
    let edge_packet = FramePacket::from_v2_direct_with_frame(
        aimer::cupid::draw_cmd_v2::RenderFrame {
            damage: edge_damage,
            operations: tree.render_all(),
        },
        metadata(DamageSet::new(WIDTH, HEIGHT)),
        Frame::new(aimer::cupid::draw_cmd::DrawList::new(), WIDTH, HEIGHT),
    )
    .expect("lower opacity change at the target edge");
    let edge_plan = edge_packet.render_plan().unwrap();
    assert_eq!(edge_plan.local_v2_revision(green), Some(green_revision));
    assert_eq!(edge_plan.local_v2_revision(blue), Some(blue_revision));
    assert_eq!(edge_plan.local_v2_revision(yellow), Some(yellow_revision));
    renderer.render_packet(&backend, &view, &edge_packet, false);
    let edge_pixels = read_target(&device, &queue, &target);
    assert_pixel(&edge_pixels, 62, 62, [32, 32, 0, 32]);
    assert_pixel(&edge_pixels, 20, 20, [0, 32, 0, 32]);
    assert!(!renderer.compositor_stats().full_repaint);

    const RESIZED_WIDTH: u32 = 80;
    const RESIZED_HEIGHT: u32 = 80;
    let resized_target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("aimer resized cropped opacity group target"),
        size: wgpu::Extent3d {
            width: RESIZED_WIDTH,
            height: RESIZED_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let resized_view = resized_target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer.render_packet_at_size(
        &backend,
        &resized_view,
        &edge_packet,
        RESIZED_WIDTH,
        RESIZED_HEIGHT,
        false,
    );
    assert!(renderer.compositor_stats().full_repaint);
    let resized_old_bounds = read_target_sized(
        &device,
        &queue,
        &resized_target,
        RESIZED_WIDTH,
        RESIZED_HEIGHT,
    );
    assert_eq!(
        pixel_sized(&resized_old_bounds, RESIZED_WIDTH, 70, 70),
        [0, 0, 0, 0]
    );
    assert_pixel_sized(&resized_old_bounds, RESIZED_WIDTH, 62, 62, [32, 32, 0, 32]);

    tree.set_bounds(root, V2Rect::new(0.0, 0.0, 80.0, 80.0))
        .unwrap();
    tree.set_clip(root, Some(V2Rect::new(0.0, 0.0, 80.0, 80.0)))
        .unwrap();
    tree.set_bounds(edge_group, V2Rect::new(48.0, 48.0, 48.0, 48.0))
        .unwrap();
    tree.set_clip(edge_group, Some(V2Rect::new(0.0, 0.0, 32.0, 32.0)))
        .unwrap();
    let expanded_packet = FramePacket::from_v2_direct(
        aimer::cupid::draw_cmd_v2::RenderFrame {
            damage: vec![V2Rect::new(0.0, 0.0, 80.0, 80.0)],
            operations: tree.render_all(),
        },
        metadata(DamageSet::new(RESIZED_WIDTH, RESIZED_HEIGHT)),
    )
    .expect("lower resized opacity group bounds");
    renderer.render_packet(&backend, &resized_view, &expanded_packet, false);
    assert!(renderer.compositor_stats().full_repaint);
    let resized_expanded = read_target_sized(
        &device,
        &queue,
        &resized_target,
        RESIZED_WIDTH,
        RESIZED_HEIGHT,
    );
    assert_pixel_sized(&resized_expanded, RESIZED_WIDTH, 70, 70, [32, 32, 0, 32]);
    assert_pixel_sized(&resized_expanded, RESIZED_WIDTH, 79, 79, [32, 32, 0, 32]);
}

#[test]
fn virtualized_scroll_partial_damage_preserves_pixels_outside_viewport_on_metal() {
    let require_gpu = std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_some();
    let Some((device, queue, adapter_info)) = gpu() else {
        if require_gpu {
            panic!("Metal adapter unavailable; run this test in a host GPU session");
        }
        eprintln!("skipping: Metal adapter unavailable");
        return;
    };
    assert_eq!(adapter_info.backend, wgpu::Backend::Metal);

    const VIEWPORT_X: f32 = 16.0;
    const VIEWPORT_Y: f32 = 16.0;
    const VIEWPORT_SIZE: f32 = 32.0;
    const ROW_HEIGHT: f32 = 16.0;
    const ROW_WIDTH: f32 = 48.0;
    const BACKGROUND: [u8; 4] = [220, 20, 30, 255];
    const ROW_ZERO: [u8; 4] = [20, 50, 220, 255];
    const ROW_ONE: [u8; 4] = [220, 200, 20, 255];
    const ROW_TWO: [u8; 4] = [20, 210, 60, 255];

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("aimer virtualized scroll damage target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let tree = RenderTree::new();
    let viewport = V2Rect::new(0.0, 0.0, VIEWPORT_SIZE, VIEWPORT_SIZE);
    let first_ids = tree
        .sync_structure_with_clips(
            &[
                RenderNodeSpec::new(
                    None,
                    None,
                    V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
                ),
                RenderNodeSpec::new(
                    None,
                    Some(0),
                    V2Rect::new(VIEWPORT_X, VIEWPORT_Y, VIEWPORT_SIZE, VIEWPORT_SIZE),
                ),
                RenderNodeSpec::new(
                    None,
                    Some(1),
                    V2Rect::new(0.0, 0.0, ROW_WIDTH, ROW_HEIGHT * 4.0),
                ),
                RenderNodeSpec::new(None, Some(2), V2Rect::new(0.0, 0.0, ROW_WIDTH, ROW_HEIGHT)),
                RenderNodeSpec::new(
                    None,
                    Some(2),
                    V2Rect::new(0.0, ROW_HEIGHT, ROW_WIDTH, ROW_HEIGHT),
                ),
            ],
            &[None, Some(viewport), None, None, None],
        )
        .expect("build the initial clipped row window");
    record_v2_fill(
        &tree,
        first_ids[0],
        V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
        BACKGROUND,
    );
    record_v2_fill(
        &tree,
        first_ids[3],
        V2Rect::new(0.0, 0.0, ROW_WIDTH, ROW_HEIGHT),
        ROW_ZERO,
    );
    record_v2_fill(
        &tree,
        first_ids[4],
        V2Rect::new(0.0, 0.0, ROW_WIDTH, ROW_HEIGHT),
        ROW_ONE,
    );

    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let first_packet = direct_v2_packet(
        &tree,
        vec![V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32)],
    );
    assert!(first_packet.render_plan().unwrap().is_complete());
    renderer.render_packet(&backend, &view, &first_packet, false);
    let first_pixels = read_target(&device, &queue, &target);
    assert_pixel(&first_pixels, 8, 8, BACKGROUND);
    assert_pixel(&first_pixels, 20, 20, ROW_ZERO);
    assert_pixel(&first_pixels, 20, 36, ROW_ONE);
    assert_pixel(&first_pixels, 47, 20, ROW_ZERO);
    assert_pixel(&first_pixels, 48, 20, BACKGROUND);
    let _ = tree.take_damage();

    let scrolled_ids = tree
        .sync_structure_with_clips(
            &[
                RenderNodeSpec::new(
                    Some(first_ids[0]),
                    None,
                    V2Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
                ),
                RenderNodeSpec::new(
                    Some(first_ids[1]),
                    Some(0),
                    V2Rect::new(VIEWPORT_X, VIEWPORT_Y, VIEWPORT_SIZE, VIEWPORT_SIZE),
                ),
                RenderNodeSpec::new(
                    Some(first_ids[2]),
                    Some(1),
                    V2Rect::new(0.0, -ROW_HEIGHT, ROW_WIDTH, ROW_HEIGHT * 4.0),
                ),
                RenderNodeSpec::new(
                    Some(first_ids[4]),
                    Some(2),
                    V2Rect::new(0.0, ROW_HEIGHT, ROW_WIDTH, ROW_HEIGHT),
                ),
                RenderNodeSpec::new(
                    None,
                    Some(2),
                    V2Rect::new(0.0, ROW_HEIGHT * 2.0, ROW_WIDTH, ROW_HEIGHT),
                ),
            ],
            &[None, Some(viewport), None, None, None],
        )
        .expect("advance the virtualized row window by one row");
    let new_row = scrolled_ids[4];
    record_v2_fill(
        &tree,
        new_row,
        V2Rect::new(0.0, 0.0, ROW_WIDTH, ROW_HEIGHT),
        ROW_TWO,
    );

    let update_damage = tree.take_damage();
    assert_eq!(
        update_damage,
        vec![V2Rect::new(
            VIEWPORT_X,
            VIEWPORT_Y,
            VIEWPORT_SIZE,
            VIEWPORT_SIZE
        )],
        "the row-window update damages only the clipped viewport"
    );
    let second_packet = direct_v2_packet(&tree, update_damage);
    assert_eq!(
        second_packet.metadata().damage().regions(),
        &[aimer::cupid::damage_region::DamageRect::new(16, 16, 32, 32)]
    );
    let plan = second_packet.render_plan().unwrap();
    assert!(plan.is_complete());
    assert_eq!(plan.local_v2_revision(new_row), Some(1));
    assert_eq!(plan.local_v2_revision(scrolled_ids[3]), Some(1));

    renderer.render_packet(&backend, &view, &second_packet, false);
    let second_pixels = read_target(&device, &queue, &target);
    let stats = renderer.compositor_stats();
    assert!(!stats.full_repaint);
    assert_eq!(stats.damage_regions, 1);
    assert_eq!(stats.damaged_pixels, 32 * 32);
    assert_pixel(&second_pixels, 8, 8, BACKGROUND);
    assert_pixel(&second_pixels, 20, 20, ROW_ONE);
    assert_pixel(&second_pixels, 20, 36, ROW_TWO);
    assert_pixel(&second_pixels, 47, 20, ROW_ONE);
    assert_pixel(&second_pixels, 48, 20, BACKGROUND);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if !(16..48).contains(&x) || !(16..48).contains(&y) {
                assert_eq!(
                    pixel(&second_pixels, x, y),
                    pixel(&first_pixels, x, y),
                    "scrolling changed the pixel outside its viewport at ({x}, {y})"
                );
            }
        }
    }
}
