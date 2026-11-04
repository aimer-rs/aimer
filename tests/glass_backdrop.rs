use aimer::canvas::{
    Canvas as RetainedCanvas, FrameCanvas, InnerCanvas, MaterialDrawRequest, MaterialKind,
};
use aimer_cupid::WgpuBackend;
use aimer_cupid::damage_region::DamageSet;
use aimer_cupid::draw_cmd::{Rect, RenderFrame, RenderNodeId, RenderPaintSource, RenderTree};
use aimer_cupid::frame::{FramePacket, FrameRenderMetadata};
use aimer_cupid::renderer::Renderer;
use aimer::widget::base::WindowHandle;
use aimer::{BuildContext, Color, Glass, ResolvedSize, SizedBox, Vec2d, Widget};
use aimer_utils::SyncFuture;

const FRAME_WIDTH: u32 = 2_176;
const FRAME_HEIGHT: u32 = 64;
const GLASS_X: u32 = 1_024;
const GLASS_SIZE: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .block()
        .ok()?;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("glass widget regression device"),
            ..Default::default()
        })
        .block()
        .ok()
}

fn glass_frame(blur_radius: f32) -> aimer_cupid::render_list::DrawList {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime");
    let inner = InnerCanvas::new();
    let canvas = FrameCanvas::new(&inner);
    let size = ResolvedSize {
        width: FRAME_WIDTH as f32,
        height: FRAME_HEIGHT as f32,
    };
    let context = BuildContext::new(
        canvas.clone(),
        size,
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(
            winit::dpi::PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
            1.0,
        ),
        runtime.handle().clone(),
    );

    canvas.fill_color_rect(
        Vec2d {
            x: GLASS_X as f32,
            y: 0.0,
        },
        ResolvedSize {
            width: GLASS_SIZE as f32 / 2.0,
            height: GLASS_SIZE as f32,
        },
        Color::Rgba(0, 0, 0, 255),
        [0.0; 4],
    );
    canvas.fill_color_rect(
        Vec2d {
            x: GLASS_X as f32 + GLASS_SIZE as f32 / 2.0,
            y: 0.0,
        },
        ResolvedSize {
            width: GLASS_SIZE as f32 / 2.0,
            height: GLASS_SIZE as f32,
        },
        Color::Rgba(255, 255, 255, 255),
        [0.0; 4],
    );

    let element = Glass::new()
        .tint(Color::Transparent)
        .opacity(0.0)
        .blur_radius(blur_radius)
        .border_width(0.0)
        .shadow_blur(0.0)
        .border_radius(0.0)
        .child(
            SizedBox::new()
                .width(GLASS_SIZE as f32)
                .height(GLASS_SIZE as f32),
        )
        .to_element(&context);
    element.layout(&context);
    canvas.save();
    canvas.translate(Vec2d {
        x: GLASS_X as f32,
        y: 0.0,
    });
    element.update(&context);
    canvas.restore();

    inner.take_draw_list()
}

fn tinted_glass_frame() -> aimer_cupid::render_list::DrawList {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime");
    let inner = InnerCanvas::new();
    let canvas = FrameCanvas::new(&inner);
    let size = ResolvedSize {
        width: FRAME_WIDTH as f32,
        height: FRAME_HEIGHT as f32,
    };
    let context = BuildContext::new(
        canvas.clone(),
        size,
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(
            winit::dpi::PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
            1.0,
        ),
        runtime.handle().clone(),
    );

    canvas.fill_color_rect(
        Vec2d {
            x: GLASS_X as f32,
            y: 0.0,
        },
        ResolvedSize {
            width: GLASS_SIZE as f32,
            height: GLASS_SIZE as f32,
        },
        Color::Rgba(32, 32, 32, 255),
        [0.0; 4],
    );

    let element = Glass::new()
        .tint(Color::Rgba(224, 24, 24, 255))
        .opacity(0.9)
        .blur_radius(16.0)
        .edge_lighting(0.0)
        .specular_highlight(0.0)
        .border_color(Color::Transparent)
        .border_width(0.0)
        .shadow_blur(0.0)
        .border_radius(0.0)
        .child(
            SizedBox::new()
                .width(GLASS_SIZE as f32)
                .height(GLASS_SIZE as f32),
        )
        .to_element(&context);
    element.layout(&context);
    canvas.save();
    canvas.translate(Vec2d {
        x: GLASS_X as f32,
        y: 0.0,
    });
    element.update(&context);
    canvas.restore();

    inner.take_draw_list()
}

fn render_and_read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    backend: &WgpuBackend,
    renderer: &mut Renderer,
    draw_list: &aimer_cupid::render_list::DrawList,
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glass widget regression target"),
        size: wgpu::Extent3d {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
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

    // Exercise the ordinary renderer entry point. Real presentation surfaces
    // are not guaranteed to support COPY_SRC, but Glass must still work.
    renderer.render(backend, &view, FRAME_WIDTH, FRAME_HEIGHT, false, draw_list);

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("glass widget regression readback"),
        size: (FRAME_WIDTH * FRAME_HEIGHT * 4) as u64,
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
                bytes_per_row: Some(FRAME_WIDTH * 4),
                rows_per_image: Some(FRAME_HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| {
        result.expect("glass readback buffer to map");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("glass readback to finish");
    let pixels = slice
        .get_mapped_range()
        .expect("glass readback range")
        .to_vec();
    readback.unmap();
    pixels
}

fn retained_v2_glass_tree() -> (RenderTree, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree
        .add_root(Rect::new(0.0, 0.0, FRAME_WIDTH as f32, FRAME_HEIGHT as f32))
        .unwrap();
    tree.set_paint_source(root, RenderPaintSource::LocalV2)
        .unwrap();
    let background = RetainedCanvas::of(&tree.context(root).unwrap());
    background.fill_rect(
        Rect::new(
            GLASS_X as f32,
            0.0,
            GLASS_SIZE as f32 / 2.0,
            GLASS_SIZE as f32,
        ),
        [0, 0, 0, 255],
    );
    background.fill_rect(
        Rect::new(
            GLASS_X as f32 + GLASS_SIZE as f32 / 2.0,
            0.0,
            GLASS_SIZE as f32 / 2.0,
            GLASS_SIZE as f32,
        ),
        [255, 255, 255, 255],
    );
    background.finish();

    let glass = tree
        .add_child(
            root,
            Rect::new(GLASS_X as f32, 0.0, GLASS_SIZE as f32, GLASS_SIZE as f32),
        )
        .unwrap();
    tree.set_paint_source(glass, RenderPaintSource::LocalV2)
        .unwrap();
    (tree, glass)
}

fn retained_v2_glass_packet(
    tree: &RenderTree,
    glass: RenderNodeId,
    blur_radius: f32,
) -> FramePacket {
    let mut request = MaterialDrawRequest::new(
        MaterialKind::Glass,
        [0.0, 0.0, GLASS_SIZE as f32, GLASS_SIZE as f32],
    );
    request.tint = [0.0; 4];
    request.opacity = 0.0;
    request.blur_radius = blur_radius;
    request.border_color = [0.0; 4];
    request.border_width = 0.0;
    request.corner_radii = [0.0; 4];
    request.shadow_color = [0.0; 4];
    request.shadow_blur = 0.0;
    let material_canvas = RetainedCanvas::of(&tree.context(glass).unwrap());
    material_canvas.draw_material(request);
    material_canvas.finish();

    let packet = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, FRAME_WIDTH as f32, FRAME_HEIGHT as f32)],
            operations: tree.render_all(),
        },
        FrameRenderMetadata::new(1.0, 0, 0, 0, 0, DamageSet::new(FRAME_WIDTH, FRAME_HEIGHT)),
    )
    .expect("complete retained v2 material frame");
    assert!(
        packet
            .render_plan()
            .is_some_and(|plan| plan.local_v2_revision(glass).is_some())
    );
    assert!(
        tree.draw_list_snapshot(glass)
            .unwrap()
            .commands
            .iter()
            .any(|command| matches!(
                command,
                aimer_cupid::draw_cmd::DrawCommand::DrawCustom { pipeline_name, data }
                    if pipeline_name.as_ref() == "aimer.material"
                        && data.len() == MaterialDrawRequest::PACKET_LEN
            ))
    );
    packet
}

fn render_packet_and_read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    backend: &WgpuBackend,
    renderer: &mut Renderer,
    packet: &FramePacket,
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("retained glass regression target"),
        size: wgpu::Extent3d {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
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
        label: Some("retained glass regression readback"),
        size: (FRAME_WIDTH * FRAME_HEIGHT * 4) as u64,
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
                bytes_per_row: Some(FRAME_WIDTH * 4),
                rows_per_image: Some(FRAME_HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| {
        result.expect("retained glass readback buffer to map");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("retained glass readback to finish");
    let pixels = slice
        .get_mapped_range()
        .expect("retained glass readback range")
        .to_vec();
    readback.unmap();
    pixels
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * FRAME_WIDTH + x) * 4) as usize;
    pixels[offset..offset + 4].try_into().expect("RGBA pixel")
}

#[test]
fn glass_widget_blurs_a_backdrop_without_a_copyable_surface() {
    let Some((device, queue)) = gpu() else {
        eprintln!("skipping: no GPU adapter available");
        return;
    };
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);

    let sharp = render_and_read(&device, &queue, &backend, &mut renderer, &glass_frame(0.0));
    let frosted = render_and_read(&device, &queue, &backend, &mut renderer, &glass_frame(64.0));
    let sample_x = GLASS_X + GLASS_SIZE / 2;
    let sharp_edge = pixel(&sharp, sample_x, GLASS_SIZE / 2);
    let frosted_edge = pixel(&frosted, sample_x, GLASS_SIZE / 2);
    let difference = sharp_edge[..3]
        .iter()
        .zip(&frosted_edge[..3])
        .map(|(left, right)| left.abs_diff(*right))
        .map(u16::from)
        .sum::<u16>();

    assert!(
        difference > 24,
        "Glass should visibly change the backdrop edge without surface COPY_SRC: sharp={sharp_edge:?}, frosted={frosted_edge:?}"
    );
}

#[test]
fn configured_glass_tint_reaches_the_edge_when_lighting_is_disabled() {
    let Some((device, queue)) = gpu() else {
        eprintln!("skipping: no GPU adapter available");
        return;
    };
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let pixels = render_and_read(
        &device,
        &queue,
        &backend,
        &mut renderer,
        &tinted_glass_frame(),
    );
    let edge = pixel(&pixels, GLASS_X + 1, GLASS_SIZE / 2);

    assert!(
        edge[0] > edge[1].saturating_add(40) && edge[0] > edge[2].saturating_add(40),
        "the configured red tint should reach the pane edge without a fixed cyan bloom: edge={edge:?}"
    );
}

#[test]
fn retained_v2_glass_custom_command_samples_the_backdrop() {
    let Some((device, queue)) = gpu() else {
        eprintln!("skipping: no GPU adapter available");
        return;
    };
    let backend = WgpuBackend::new(device.clone(), queue.clone());
    let mut renderer = Renderer::new(&backend, FORMAT);
    let (tree, glass) = retained_v2_glass_tree();
    let sharp_packet = retained_v2_glass_packet(&tree, glass, 0.0);
    let sharp_revision = sharp_packet
        .render_plan()
        .and_then(|plan| plan.local_v2_revision(glass))
        .unwrap();
    let sharp = render_packet_and_read(&device, &queue, &backend, &mut renderer, &sharp_packet);
    let frosted_packet = retained_v2_glass_packet(&tree, glass, 64.0);
    assert!(
        frosted_packet
            .render_plan()
            .and_then(|plan| plan.local_v2_revision(glass))
            .is_some_and(|revision| revision > sharp_revision)
    );
    let frosted = render_packet_and_read(&device, &queue, &backend, &mut renderer, &frosted_packet);
    assert_eq!(pixel(&sharp, GLASS_X + 8, GLASS_SIZE / 2), [0, 0, 0, 255]);
    assert_eq!(
        pixel(&sharp, GLASS_X + GLASS_SIZE - 8, GLASS_SIZE / 2),
        [255, 255, 255, 255]
    );
    let sample_x = GLASS_X + GLASS_SIZE / 2;
    let sharp_edge = pixel(&sharp, sample_x, GLASS_SIZE / 2);
    let frosted_edge = pixel(&frosted, sample_x, GLASS_SIZE / 2);
    let difference = sharp_edge[..3]
        .iter()
        .zip(&frosted_edge[..3])
        .map(|(left, right)| left.abs_diff(*right))
        .map(u16::from)
        .sum::<u16>();

    assert!(
        difference > 24,
        "retained Glass should blur its V2 sibling backdrop: sharp={sharp_edge:?}, frosted={frosted_edge:?}"
    );
}
