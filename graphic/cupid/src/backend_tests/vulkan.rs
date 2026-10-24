//! Vulkan backend tests that drive the real renderer.
//!
//! They live beside the renderer rather than in `aimer_cupid_vulkan` because
//! they need draw lists and frames, and the backend crate must not depend on
//! either.

use aimer_cupid_vulkan::*;
use crate::backend::*;
use ash::vk;
use crate::damage_region::DamageSet;
use crate::draw_cmd::DrawList;
use crate::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderFrame, RenderNodeId, RenderTree};
use crate::frame::{Frame, FramePacket, FrameRenderMetadata};
use crate::renderer::RendererImpl;
use crate::utilities::{Color, Rect, Vec2d};
use std::sync::Arc;

#[test]
fn generic_renderer_draws_and_reads_back_with_native_vulkan() {
    const SIZE: u32 = 32;
    let Ok(backend) = VulkanBackend::new_headless() else {
        eprintln!("skipping: no Vulkan graphics device is available");
        return;
    };
    let mut renderer =
        RendererImpl::<VulkanBackend>::new(&backend, VulkanBackend::rgba8_unorm_format());
    let mut draw = DrawList::new();
    draw.fill_rect(
        Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
        Color::red(),
        [0.0; 4],
        [0.0; 4],
        Color::transparent(),
    );
    let target = backend.create_texture(&TextureDescriptor {
        label: Some("Cupid Vulkan readback test target".to_string()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: VulkanBackend::rgba8_unorm_format(),
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "Cupid Vulkan test view");
    renderer.render(&backend, &view, SIZE, SIZE, false, &draw);

    let pixels = read_target(&backend, &target, SIZE, SIZE);
    let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    let colored_pixels = pixels
        .chunks_exact(4)
        .filter(|pixel| pixel.iter().any(|channel| *channel != 0))
        .count();
    assert_eq!(
        &pixels[center..center + 4],
        &[255, 0, 0, 255],
        "non-clear pixel count: {colored_pixels}; corner: {:?}",
        &pixels[..4]
    );
}

#[test]
fn cropped_opacity_groups_composite_nested_damage_at_target_edges_on_native_vulkan() {
    const SIZE: u32 = 80;
    let Ok(backend) = VulkanBackend::new_headless() else {
        eprintln!("skipping: no Vulkan graphics device is available");
        return;
    };
    let mut renderer =
        RendererImpl::<VulkanBackend>::new(&backend, VulkanBackend::rgba8_unorm_format());
    let target = create_target(&backend, SIZE, SIZE);
    let view = backend.create_texture_view(&target, "Cupid Vulkan opacity-group target");
    let tree = RenderTree::new();
    let root = tree
        .add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32))
        .unwrap();
    tree.set_clip(root, Some(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)))
        .unwrap();
    tree.set_opacity(root, 0.5).unwrap();
    let nested = tree
        .add_child(root, Rect::new(8.0, 8.0, 56.0, 56.0))
        .unwrap();
    tree.set_clip(nested, Some(Rect::new(0.0, 0.0, 40.0, 40.0)))
        .unwrap();
    tree.set_opacity(nested, 0.5).unwrap();
    let green = tree
        .add_child(nested, Rect::new(8.0, 8.0, 32.0, 32.0))
        .unwrap();
    let edge_group = tree
        .add_child(root, Rect::new(48.0, 48.0, 32.0, 32.0))
        .unwrap();
    tree.set_clip(edge_group, Some(Rect::new(0.0, 0.0, 32.0, 32.0)))
        .unwrap();
    tree.set_opacity(edge_group, 0.5).unwrap();
    let yellow = tree
        .add_child(edge_group, Rect::new(0.0, 0.0, 32.0, 32.0))
        .unwrap();
    record_v2_fill(
        &tree,
        green,
        Rect::new(0.0, 0.0, 32.0, 32.0),
        Color::rgba8(0, 255, 0, 255),
    );
    record_v2_fill(
        &tree,
        yellow,
        Rect::new(0.0, 0.0, 32.0, 32.0),
        Color::rgba8(255, 255, 0, 255),
    );

    let metadata = || FrameRenderMetadata::new(1.0, 401, 1, 1, 1, DamageSet::new(SIZE, SIZE));
    let first_packet = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)],
            operations: tree.render_all(),
        },
        metadata(),
    )
    .expect("lower Vulkan opacity groups");
    renderer.render_packet(&backend, &view, &first_packet, false);
    let first_pixels = read_target(&backend, &target, SIZE, SIZE);
    assert_pixel_near(&first_pixels, SIZE, 20, 20, [0, 64, 0, 64]);
    assert_pixel_near(&first_pixels, SIZE, 70, 70, [64, 64, 0, 64]);
    assert_eq!(pixel(&first_pixels, SIZE, 2, 2), [0, 0, 0, 0]);
    let _ = tree.take_damage();

    tree.set_opacity(edge_group, 0.25).unwrap();
    let edge_damage = tree.take_damage();
    assert!(
        edge_damage.iter().any(|rect| {
            (rect.x + rect.width - SIZE as f32).abs() < f32::EPSILON
                && (rect.y + rect.height - SIZE as f32).abs() < f32::EPSILON
        }),
        "edge-group damage reaches both target edges: {edge_damage:?}"
    );
    let update_packet = FramePacket::from_v2_direct_with_frame(
        RenderFrame {
            damage: edge_damage,
            operations: tree.render_all(),
        },
        metadata(),
        Frame::new(DrawList::new(), SIZE, SIZE),
    )
    .expect("lower Vulkan edge-opacity update");
    renderer.render_packet(&backend, &view, &update_packet, false);
    let updated_pixels = read_target(&backend, &target, SIZE, SIZE);
    assert_pixel_near(&updated_pixels, SIZE, 70, 70, [32, 32, 0, 32]);
    assert_pixel_near(&updated_pixels, SIZE, 20, 20, [0, 64, 0, 64]);
    assert_eq!(pixel(&updated_pixels, SIZE, 2, 2), [0, 0, 0, 0]);
}

fn record_v2_fill(tree: &RenderTree, node: RenderNodeId, rect: Rect, color: Color) {
    tree.context(node)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![V2DrawCommand::FillRect {
            rect,
            color,
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        }])
        .unwrap();
}

fn pixel(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * width + x) * 4) as usize;
    data[offset..offset + 4].try_into().expect("RGBA pixel")
}

fn assert_pixel_near(data: &[u8], width: u32, x: u32, y: u32, expected: [u8; 4]) {
    let actual = pixel(data, width, x, y);
    for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert!(
            actual.abs_diff(expected) <= 2,
            "pixel ({x}, {y}) channel {channel}: expected {expected}, got {actual}"
        );
    }
}

#[test]
fn generic_renderer_uploads_and_draws_image_and_text() {
    const WIDTH: u32 = 96;
    const HEIGHT: u32 = 64;
    let Ok(backend) = VulkanBackend::new_headless() else {
        eprintln!("skipping: no Vulkan graphics device is available");
        return;
    };
    let mut renderer =
        RendererImpl::<VulkanBackend>::new(&backend, VulkanBackend::rgba8_unorm_format());
    let mut draw = DrawList::new();
    let blue = [0, 0, 255, 255];
    let texture = draw.load_image(&blue.repeat(4), 2, 2);
    draw.draw_image(Rect::new(0.0, 0.0, 24.0, 24.0), texture);
    draw.draw_text(
        Vec2d::new(32.0, 40.0),
        Arc::from("A"),
        22.0,
        Color::white(),
        400,
    );
    let svg_scene = Arc::new(crate::svg::SvgScene {
        viewport: crate::svg::SvgViewport {
            width: 10.0,
            height: 10.0,
        },
        nodes: Arc::new([crate::svg::SvgNode {
            node_id: crate::svg::SvgNodeId(0),
            svg_id: None,
            classes: Arc::new([]),
            element: crate::svg::SvgElementKind::Path,
            is_definition: false,
            definition_owner: None,
            clip_path: None,
            mask: None,
            filter: None,
            parent: None,
            children: Arc::new([]),
            transform: crate::svg::SvgTransform::default(),
            opacity: 1.0,
            geometry: Some(0),
            fill_rule: crate::svg::SvgFillRule::NonZero,
            fill: Some(crate::svg::SvgFill {
                color: crate::svg::SvgColor::rgba8(255, 0, 0, 255),
                rule: crate::svg::SvgFillRule::NonZero,
            }),
            stroke: None,
            fill_paint: None,
            stroke_paint: None,
            paint_order: crate::svg::SvgPaintOrder::FillAndStroke,
            visible: true,
        }]),
        geometries: Arc::new([crate::svg::SvgGeometry {
            commands: Arc::new([
                crate::svg::SvgPathCommand::MoveTo { x: 0.0, y: 0.0 },
                crate::svg::SvgPathCommand::LineTo { x: 10.0, y: 0.0 },
                crate::svg::SvgPathCommand::LineTo { x: 10.0, y: 10.0 },
                crate::svg::SvgPathCommand::LineTo { x: 0.0, y: 10.0 },
                crate::svg::SvgPathCommand::Close,
            ]),
        }]),
        resources: Arc::new(crate::svg::SvgResourceGraph::empty()),
    });
    draw.draw_svg(svg_scene, Rect::new(64.0, 0.0, 24.0, 24.0), Arc::new([]));
    let target = create_target(&backend, WIDTH, HEIGHT);
    let view = backend.create_texture_view(&target, "Cupid Vulkan image and text target");
    renderer.render(&backend, &view, WIDTH, HEIGHT, false, &draw);
    let pixels = read_target(&backend, &target, WIDTH, HEIGHT);
    let image_center = ((12 * WIDTH + 12) * 4) as usize;
    assert_eq!(&pixels[image_center..image_center + 4], &[0, 0, 255, 255]);
    let svg_center = ((12 * WIDTH + 76) * 4) as usize;
    assert_eq!(&pixels[svg_center..svg_center + 4], &[255, 0, 0, 255]);
    let text_has_pixels = (30..HEIGHT).any(|y| {
        (28..WIDTH).any(|x| {
            let offset = ((y * WIDTH + x) * 4) as usize;
            pixels[offset + 3] != 0
        })
    });
    assert!(text_has_pixels, "text pipeline produced no covered pixels");
}

#[test]
fn material_pipeline_renders_through_native_vulkan() {
    use crate::custom_pipeline::CustomPipelineGeneric;
    use crate::pipeline::material::{MaterialKind, MaterialPipeline, MaterialRequest};
    use std::any::Any;

    const SIZE: u32 = 32;
    let Ok(backend) = VulkanBackend::new_headless() else {
        eprintln!("skipping: no Vulkan graphics device is available");
        return;
    };
    let mut pipeline = MaterialPipeline::new(
        &backend,
        VulkanBackend::rgba8_unorm_format(),
        crate::AntiAlias::Analytic,
    );
    let request =
        MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, SIZE as f32, SIZE as f32]);
    pipeline.begin_frame();
    pipeline.prepare_command(&request as &(dyn Any + Send));
    let context = crate::custom_pipeline::RenderContextGeneric {
        backend: &backend,
        width: SIZE,
        height: SIZE,
        is_srgb: false,
        format: VulkanBackend::rgba8_unorm_format(),
        sample_count: 1,
        source_texture: None,
    };
    pipeline.prepare(&context);
    let target = create_target(&backend, SIZE, SIZE);
    let view = backend.create_texture_view(&target, "Cupid Vulkan material target");
    let mut encoder = backend.create_command_encoder("Cupid Vulkan material render");
    {
        let attachments = [RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear([0.0, 0.0, 0.0, 1.0]),
                store: StoreOp::Store,
            },
        }];
        let mut pass = backend.begin_render_pass(
            &mut encoder,
            &RenderPassDescriptor {
                label: Some("Cupid Vulkan material pass".to_string()),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
            },
        );
        pipeline.render_command(Some(0), &mut pass);
    }
    backend.submit(encoder);
    let pixels = read_target(&backend, &target, SIZE, SIZE);
    let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    assert_ne!(&pixels[center..center + 4], &[0, 0, 0, 255]);
}

fn create_target(backend: &VulkanBackend, width: u32, height: u32) -> VulkanTexture {
    backend.create_texture(&TextureDescriptor {
        label: Some("Cupid Vulkan readback test target".to_string()),
        size: (width, height, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: VulkanBackend::rgba8_unorm_format(),
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    })
}

fn read_target(
    backend: &VulkanBackend,
    target: &VulkanTexture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let row_bytes = width * 4;
    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("Cupid Vulkan readback test buffer".to_string()),
        size: (row_bytes * height) as u64,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("Cupid Vulkan readback test");
    backend.copy_texture_to_buffer(
        &mut encoder,
        &TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(height),
            },
        },
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);
    backend.read_buffer(&readback, (row_bytes * height) as u64)
}
