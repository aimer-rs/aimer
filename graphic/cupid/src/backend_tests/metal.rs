//! Metal backend tests that drive the real renderer.
//!
//! They live beside the renderer rather than in `aimer_metal` because
//! they need draw lists, frames and the rect pipeline's instance layout, and the
//! backend crate must not depend on any of that.

use std::mem::{offset_of, size_of};

use aimer_metal::*;
use crate::backend::*;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::*;
use objc2_quartz_core::CAMetalLayer;
use crate::backend::{
    BindGroupEntry, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    BufferBindingType, BufferUsage, ColorWriteMask, Operations, PrimitiveState,
    RenderPassColorAttachment, RenderPassDescriptor, TexelCopyBufferLayout, TextureAspect,
    VertexAttribute, VertexState,
};
use crate::damage_region::DamageSet;
use crate::draw_cmd::DrawList;
use crate::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderFrame, RenderTree};
use crate::frame::{Frame, FramePacket, FrameRenderMetadata};
use crate::rect_pipeline::RectInstance;
use crate::renderer::RendererImpl;
use crate::utilities::Rgba8;
use crate::utilities::{Color, Rect};

const SIZE: u32 = 64;
const FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA8Unorm;
const ROW_BYTES: u32 = SIZE * 4;
const FRAME_BYTES: u64 = (ROW_BYTES * SIZE) as u64;

#[test]
fn surface_uses_command_buffer_compatible_presentation() {
    let Some(backend) = MetalBackend::new() else {
        return;
    };
    let layer = CAMetalLayer::new();
    layer.setPresentsWithTransaction(true);

    let surface = MetalSurface::new(&backend, layer, MTLPixelFormat::BGRA8Unorm_sRGB);

    assert!(!surface.layer().presentsWithTransaction());
}

/// A non-clipped, unbordered, unshadowed rect filled with `color`.
fn instance(x: f32, y: f32, width: f32, height: f32, color: Rgba8) -> RectInstance {
    RectInstance {
        position: [x, y],
        size: [width, height],
        color,
        border_radius: [0.0; 4],
        border_width: [0.0; 4],
        border_color: Rgba8::TRANSPARENT,
        outline_width: [0.0; 4],
        outline_color: Rgba8::TRANSPARENT,
        // A negative width tells the shader that no clip is active.
        clip_rect: [-1.0, -1.0, -1.0, -1.0],
        clip_border_radius: [0.0; 4],
        shadow_params: [0.0; 4],
        shadow_color: Rgba8::TRANSPARENT,
        shadow_flags: [0.0; 4],
    }
}

/// The vertex attributes of [`RectInstance`], read from the struct itself.
///
/// Offsets come from `offset_of!` rather than literals so this cannot drift
/// from the layout the CPU side actually writes.
fn rect_attributes() -> Vec<VertexAttribute> {
    let float2 = VertexFormat::Float32x2;
    let float4 = VertexFormat::Float32x4;
    let unorm4 = VertexFormat::Unorm8x4;
    let field = |offset: usize, format: VertexFormat, location: u32| VertexAttribute {
        format,
        offset: offset as u64,
        shader_location: location,
    };
    vec![
        field(offset_of!(RectInstance, position), float2, 0),
        field(offset_of!(RectInstance, size), float2, 1),
        field(offset_of!(RectInstance, color), unorm4, 2),
        field(offset_of!(RectInstance, border_radius), float4, 3),
        field(offset_of!(RectInstance, border_width), float4, 4),
        field(offset_of!(RectInstance, border_color), unorm4, 5),
        field(offset_of!(RectInstance, outline_width), float4, 6),
        field(offset_of!(RectInstance, outline_color), unorm4, 7),
        field(offset_of!(RectInstance, clip_rect), float4, 8),
        field(offset_of!(RectInstance, clip_border_radius), float4, 9),
        field(offset_of!(RectInstance, shadow_params), float4, 10),
        field(offset_of!(RectInstance, shadow_color), unorm4, 11),
        field(offset_of!(RectInstance, shadow_flags), float4, 12),
    ]
}

fn pixel(data: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = (y as usize * ROW_BYTES as usize) + (x as usize * 4);
    [
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]
}

fn pixel_with_stride(data: &[u8], row_bytes: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = (y as usize * row_bytes as usize) + (x as usize * 4);
    [
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]
}

fn assert_pixel_near(data: &[u8], row_bytes: u32, x: u32, y: u32, expected: [u8; 4]) {
    let actual = pixel_with_stride(data, row_bytes, x, y);
    for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert!(
            actual.abs_diff(expected) <= 2,
            "pixel ({x}, {y}) channel {channel}: expected {expected}, got {actual}"
        );
    }
}

fn record_v2_fill(
    tree: &RenderTree,
    node: crate::draw_cmd_v2::RenderNodeId,
    rect: Rect,
    color: Color,
) {
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

/// Construction compiles every built-in pipeline shader for Metal.
#[test]
fn generic_renderer_initializes_all_builtin_pipelines_on_metal() {
    let Some(backend) = MetalBackend::new() else {
        eprintln!("skipping: this host exposes no Metal device");
        return;
    };

    let _renderer = RendererImpl::<MetalBackend>::new(&backend, FORMAT);
}

/// Draw the non-rectangle built-ins through their Metal pipelines and
/// verify image, multiple text requests, SVG, material and composite pixels.
#[test]
fn generic_renderer_renders_all_builtin_pipelines_on_metal() {
    let Some(backend) = MetalBackend::new() else {
        eprintln!("skipping: this host exposes no Metal device");
        return;
    };

    use std::sync::Arc;

    use crate::pipeline::material::{MATERIAL_PIPELINE_NAME, MaterialKind, MaterialRequest};
    use crate::svg::{
        SvgElementKind, SvgFill, SvgFillRule, SvgGeometry, SvgNode, SvgNodeId, SvgPaintOrder,
        SvgPathCommand, SvgScene, SvgTransform, SvgViewport,
    };
    use crate::utilities::Vec2d;

    let mut renderer = RendererImpl::<MetalBackend>::new(&backend, FORMAT);
    let mut draw = DrawList::new();
    let blue = [0, 0, 255, 255];
    let texture_id = draw.load_image(&[blue; 4].concat(), 2, 2);
    draw.draw_image(Rect::new(0.0, 0.0, 24.0, 24.0), texture_id);
    draw.draw_text(
        Vec2d::new(32.0, 24.0),
        Arc::from("A"),
        18.0,
        Color::white(),
        400,
    );
    draw.draw_text(
        Vec2d::new(2.0, 36.0),
        Arc::from("B"),
        18.0,
        Color::white(),
        400,
    );
    draw.draw_text_decoration(Rect::new(8.0, 56.0, 48.0, 4.0), Color::white(), 0, 2.0, 8.0);

    let scene = Arc::new(SvgScene {
        viewport: SvgViewport {
            width: 10.0,
            height: 10.0,
        },
        nodes: Arc::new([
            SvgNode {
                node_id: SvgNodeId(0),
                svg_id: None,
                classes: Arc::new([]),
                element: SvgElementKind::Path,
                is_definition: false,
                definition_owner: None,
                clip_path: None,
                mask: None,
                filter: None,
                parent: None,
                children: Arc::new([]),
                transform: SvgTransform::default(),
                opacity: 1.0,
                geometry: Some(0),
                fill_rule: SvgFillRule::NonZero,
                fill: Some(SvgFill {
                    color: crate::svg::SvgColor::rgba8(255, 0, 0, 255),
                    rule: SvgFillRule::NonZero,
                }),
                stroke: None,
                fill_paint: None,
                stroke_paint: None,
                paint_order: SvgPaintOrder::FillAndStroke,
                visible: true,
            },
            SvgNode {
                node_id: SvgNodeId(1),
                svg_id: None,
                classes: Arc::new([]),
                element: SvgElementKind::Path,
                is_definition: false,
                definition_owner: None,
                clip_path: None,
                mask: None,
                filter: None,
                parent: None,
                children: Arc::new([]),
                transform: SvgTransform {
                    tx: 5.0,
                    ..SvgTransform::default()
                },
                opacity: 1.0,
                geometry: Some(0),
                fill_rule: SvgFillRule::NonZero,
                fill: Some(SvgFill {
                    color: crate::svg::SvgColor::rgba8(0, 255, 0, 255),
                    rule: SvgFillRule::NonZero,
                }),
                stroke: None,
                fill_paint: None,
                stroke_paint: None,
                paint_order: SvgPaintOrder::FillAndStroke,
                visible: true,
            },
        ]),
        geometries: Arc::new([SvgGeometry {
            commands: Arc::new([
                SvgPathCommand::MoveTo { x: 0.0, y: 0.0 },
                SvgPathCommand::LineTo { x: 10.0, y: 0.0 },
                SvgPathCommand::LineTo { x: 10.0, y: 10.0 },
                SvgPathCommand::LineTo { x: 0.0, y: 10.0 },
                SvgPathCommand::Close,
            ]),
        }]),
        resources: Arc::new(crate::svg::SvgResourceGraph::empty()),
    });
    draw.draw_svg(scene, Rect::new(32.0, 0.0, 20.0, 20.0), Arc::from([]));
    draw.draw_custom(
        MATERIAL_PIPELINE_NAME,
        MaterialRequest::new(MaterialKind::Glass, [24.0, 32.0, 32.0, 20.0]),
    );

    let target = backend.create_texture(&TextureDescriptor {
        label: Some("all Metal built-ins target".to_string()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FORMAT,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "all Metal built-ins view");
    renderer.render(&backend, &view, SIZE, SIZE, false, &draw);

    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("all Metal built-ins readback".to_string()),
        size: FRAME_BYTES,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("all Metal built-ins readback");
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
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(SIZE),
            },
        },
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);

    let pixels = backend.read_buffer(&readback, FRAME_BYTES);
    assert_eq!(pixel(&pixels, 12, 12), [0, 0, 255, 255], "image");
    assert_eq!(pixel(&pixels, 35, 10), [255, 0, 0, 255], "first SVG path");
    assert_eq!(pixel(&pixels, 45, 10), [0, 255, 0, 255], "second SVG path");
    assert_eq!(
        pixel(&pixels, 55, 10),
        [0, 255, 0, 255],
        "second SVG path transform"
    );
    assert_eq!(pixel(&pixels, 32, 58), [255, 255, 255, 255], "decoration");
    assert!(
        (32..SIZE).any(|x| {
            (16..32).any(|y| {
                let [red, green, blue, alpha] = pixel(&pixels, x, y);
                alpha > 0 && red > 0 && green > 0 && blue > 0
            })
        }),
        "text pipeline should render antialiased glyph pixels"
    );
    assert!(
        (2..20).any(|x| (24..36).any(|y| pixel(&pixels, x, y)[3] > 0)),
        "a later text request should draw from its own instance range"
    );
    assert!(pixel(&pixels, 40, 40)[3] > 0, "material + frame composite");
}

/// The analytic antialiasing fringe carries a per-vertex coverage that the
/// Metal SVG shader must multiply into the fragment. Ignoring it draws the
/// fringe at full opacity: edges grow by a pixel and stay hard, which shows
/// as sawtooth corners on curved icons.
#[test]
fn svg_edge_fringe_is_partially_covered_on_metal() {
    let Some(backend) = MetalBackend::new() else {
        eprintln!("skipping: this host exposes no Metal device");
        return;
    };

    use std::sync::Arc;

    use crate::svg::{
        SvgElementKind, SvgFill, SvgFillRule, SvgGeometry, SvgNode, SvgNodeId, SvgPaintOrder,
        SvgPathCommand, SvgScene, SvgTransform, SvgViewport,
    };

    let mut renderer = RendererImpl::<MetalBackend>::new(&backend, FORMAT);
    let mut draw = DrawList::new();
    // The right edge sits a quarter pixel into pixel column 10, so the
    // fringe quad covers that column's centre at a coverage of 0.75.
    let scene = Arc::new(SvgScene {
        viewport: SvgViewport {
            width: 16.0,
            height: 16.0,
        },
        nodes: Arc::new([SvgNode {
            node_id: SvgNodeId(0),
            svg_id: None,
            classes: Arc::new([]),
            element: SvgElementKind::Path,
            is_definition: false,
            definition_owner: None,
            clip_path: None,
            mask: None,
            filter: None,
            parent: None,
            children: Arc::new([]),
            transform: SvgTransform::default(),
            opacity: 1.0,
            geometry: Some(0),
            fill_rule: SvgFillRule::NonZero,
            fill: Some(SvgFill {
                color: crate::svg::SvgColor::rgba8(255, 0, 0, 255),
                rule: SvgFillRule::NonZero,
            }),
            stroke: None,
            fill_paint: None,
            stroke_paint: None,
            paint_order: SvgPaintOrder::FillAndStroke,
            visible: true,
        }]),
        geometries: Arc::new([SvgGeometry {
            commands: Arc::new([
                SvgPathCommand::MoveTo { x: 2.0, y: 2.0 },
                SvgPathCommand::LineTo { x: 10.25, y: 2.0 },
                SvgPathCommand::LineTo { x: 10.25, y: 14.0 },
                SvgPathCommand::LineTo { x: 2.0, y: 14.0 },
                SvgPathCommand::Close,
            ]),
        }]),
        resources: Arc::new(crate::svg::SvgResourceGraph::empty()),
    });
    draw.draw_svg(scene, Rect::new(0.0, 0.0, 16.0, 16.0), Arc::from([]));

    let target = backend.create_texture(&TextureDescriptor {
        label: Some("Metal SVG fringe target".to_string()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FORMAT,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "Metal SVG fringe view");
    renderer.render(&backend, &view, SIZE, SIZE, false, &draw);

    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("Metal SVG fringe readback".to_string()),
        size: FRAME_BYTES,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("Metal SVG fringe readback");
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
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(SIZE),
            },
        },
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);

    let pixels = backend.read_buffer(&readback, FRAME_BYTES);
    assert_eq!(pixel(&pixels, 6, 8), [255, 0, 0, 255], "interior is opaque");
    let fringe = pixel(&pixels, 10, 8)[3];
    assert!(
        (100..=230).contains(&fringe),
        "fringe pixel should be partially covered, got alpha {fringe}"
    );
    assert_eq!(pixel(&pixels, 12, 8)[3], 0, "beyond the fringe is empty");
}

/// Render a DrawList through the generic renderer on Metal and verify the
/// rect's pixels through a buffer readback.
#[test]
fn generic_renderer_renders_rect_through_metal() {
    let Some(backend) = MetalBackend::new() else {
        eprintln!("skipping: this host exposes no Metal device");
        return;
    };

    let mut renderer = RendererImpl::<MetalBackend>::new_rect_only(&backend, FORMAT);
    let mut draw_list = DrawList::new();
    draw_list.fill_rect(
        Rect::new(8.0, 8.0, 48.0, 48.0),
        Color::red(),
        [0.0; 4],
        [0.0; 4],
        Color::transparent(),
    );

    let target = backend.create_texture(&TextureDescriptor {
        label: Some("generic Metal renderer target".to_string()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FORMAT,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "generic Metal renderer view");
    renderer.render(&backend, &view, SIZE, SIZE, false, &draw_list);

    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("generic Metal renderer readback".to_string()),
        size: FRAME_BYTES,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("generic Metal renderer readback");
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
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(SIZE),
            },
        },
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);

    let pixels = backend.read_buffer(&readback, FRAME_BYTES);
    assert_eq!(pixel(&pixels, 32, 32), [255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 1, 1), [0, 0, 0, 0]);
}

/// Render two rects on a transparent 64x64 target and read the pixels back.
///
/// The placement is deliberate. Red occupies the upper band and green the
/// lower one, with a gap between them and margins at the edges, so a single
/// pass proves four things at once:
///
/// * a filled pixel anywhere means the vertex descriptor, the argument
///   indices and the MSL all agree;
/// * *which* band is red rules out a vertical flip, which a full-cover rect
///   would hide completely;
/// * the clear leaving pixels transparent proves the load action reached the
///   attachment;
/// * a green pixel in the last rows means the readback row pitch is right,
///   which a centre-only check cannot tell.
#[test]
fn two_bands_render_and_read_back_through_metal() {
    let Some(backend) = MetalBackend::new() else {
        eprintln!("skipping: this host exposes no Metal device");
        return;
    };

    // A shader that fails to compile here is the most likely first failure
    // of a hand-written MSL port, so carry the compiler's own words.
    let msl = include_str!("../../../backends/metal/shaders/metal/rect.metal");
    let module = backend
        .try_create_shader_module(msl.as_bytes(), "rect shader")
        .expect("rect.metal must compile as MSL");

    let viewport_buffer = backend.create_buffer(&BufferDescriptor {
        label: Some("metal test viewport".to_string()),
        size: 16,
        usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
    });
    let layout = backend.create_bind_group_layout(&[BindGroupLayoutEntry {
        binding: 0,
        visibility: vec![ShaderStage::Vertex, ShaderStage::Fragment],
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }]);
    let bind_group = backend.create_bind_group(
        &layout,
        &[BindGroupEntry {
            binding: 0,
            resource: BindingResource::Buffer(&viewport_buffer),
        }],
    );
    let pipeline_layout = backend.create_pipeline_layout(&[&layout]);

    let attributes = rect_attributes();
    let vertex_buffers = [Some(VertexBufferLayout {
        array_stride: size_of::<RectInstance>() as u64,
        step_mode: VertexStepMode::Instance,
        attributes: &attributes,
    })];
    let targets = [Some(ColorTargetState {
        format: FORMAT,
        blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: ColorWriteMask::ALL,
    })];

    let pipeline = backend
        .try_create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("metal test rect".to_string()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &module,
                entry_point: "vs_main",
                buffers: &vertex_buffers,
            },
            fragment: Some(crate::backend::FragmentState {
                module: &module,
                entry_point: "fs_main",
                targets: &targets,
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: crate::backend::MultisampleState::default(),
        })
        .expect("the rect pipeline must build on Metal");

    // `surface_is_srgb` stays 0.0 to match the non-sRGB reference target the
    // wgpu tests render into.
    backend.write_buffer(
        &viewport_buffer,
        0,
        bytemuck::cast_slice(&[SIZE as f32, SIZE as f32, 0.0f32, 0.0f32]),
    );

    let red = Rgba8::new(255, 0, 0, 255);
    let green = Rgba8::new(0, 255, 0, 255);
    let instances = [
        instance(4.0, 4.0, 56.0, 24.0, red),
        instance(4.0, 36.0, 56.0, 24.0, green),
    ];
    let instance_buffer = backend.create_buffer(&BufferDescriptor {
        label: Some("metal test instances".to_string()),
        size: (instances.len() * size_of::<RectInstance>()) as u64,
        usage: vec![BufferUsage::Vertex],
    });
    backend.write_buffer(&instance_buffer, 0, bytemuck::cast_slice(&instances));

    let target = backend.create_texture(&TextureDescriptor {
        label: Some("metal test target".to_string()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FORMAT,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "metal test view");

    let mut encoder = backend.create_command_encoder("metal test frame");
    {
        let mut pass = backend.begin_render_pass(
            &mut encoder,
            &RenderPassDescriptor {
                label: Some("metal test pass".to_string()),
                color_attachments: &[RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear([0.0, 0.0, 0.0, 0.0]),
                        store: StoreOp::Store,
                    },
                }],
                depth_stencil_attachment: None,
            },
        );
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, &instance_buffer, 0);
        // Six vertices generated in the shader, one instance per band.
        pass.draw(0..6, 0..2);
    }

    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("metal test readback".to_string()),
        size: FRAME_BYTES,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
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
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(SIZE),
            },
        },
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);

    let pixels = backend.read_buffer(&readback, FRAME_BYTES);
    assert_eq!(pixels.len(), FRAME_BYTES as usize);

    // Upper band: the red instance.
    assert_eq!(pixel(&pixels, 32, 16), [255, 0, 0, 255], "upper band");
    assert_eq!(pixel(&pixels, 8, 8), [255, 0, 0, 255], "upper band corner");
    // Lower band: the green instance. Getting these two the wrong way round
    // is what a silent vertical flip would look like.
    assert_eq!(pixel(&pixels, 32, 48), [0, 255, 0, 255], "lower band");
    assert_eq!(
        pixel(&pixels, 8, 56),
        [0, 255, 0, 255],
        "lower band, last rows: a wrong readback pitch lands here"
    );
    // Gap and margins stayed cleared.
    assert_eq!(pixel(&pixels, 32, 32), [0, 0, 0, 0], "gap between bands");
    assert_eq!(pixel(&pixels, 1, 1), [0, 0, 0, 0], "outside both bands");
}

#[test]
fn queued_texture_upload_honors_offset_and_padded_rows() {
    let Some(backend) = MetalBackend::new() else {
        eprintln!("skipping: this host exposes no Metal device");
        return;
    };

    const WIDTH: u32 = 5;
    const HEIGHT: u32 = 3;
    const SOURCE_STRIDE: usize = 8;
    const SOURCE_OFFSET: usize = 2;
    const READBACK_STRIDE: u32 = 256;
    let mut source = vec![0xee; SOURCE_OFFSET + SOURCE_STRIDE * HEIGHT as usize];
    for row in 0..HEIGHT as usize {
        for column in 0..WIDTH as usize {
            source[SOURCE_OFFSET + row * SOURCE_STRIDE + column] =
                (row * 16 + column + 1) as u8;
        }
    }

    let texture = backend.create_texture(&TextureDescriptor {
        label: Some("offset texture upload target".to_string()),
        size: (WIDTH, HEIGHT, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: MTLPixelFormat::R8Unorm,
        usage: vec![
            TextureUsage::TextureBinding,
            TextureUsage::CopyDst,
            TextureUsage::CopySrc,
        ],
    });
    backend.write_texture(&WriteTextureDescriptor {
        texture: &texture,
        mip_level: 0,
        origin: Origin3d::ZERO,
        aspect: TextureAspect::All,
        data: &source,
        buffer_layout: TexelCopyBufferLayout {
            offset: SOURCE_OFFSET as u64,
            bytes_per_row: Some(SOURCE_STRIDE as u32),
            rows_per_image: Some(HEIGHT),
        },
        extent: Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    });

    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("offset texture upload readback".to_string()),
        size: (READBACK_STRIDE * HEIGHT) as u64,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("offset texture upload readback");
    backend.copy_texture_to_buffer(
        &mut encoder,
        &TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(READBACK_STRIDE),
                rows_per_image: Some(HEIGHT),
            },
        },
        Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);

    let pixels = backend.read_buffer(&readback, (READBACK_STRIDE * HEIGHT) as u64);
    for row in 0..HEIGHT as usize {
        let start = row * READBACK_STRIDE as usize;
        let expected = (0..WIDTH as usize)
            .map(|column| (row * 16 + column + 1) as u8)
            .collect::<Vec<_>>();
        assert_eq!(&pixels[start..start + WIDTH as usize], expected);
    }
}

/// The `Unorm8x4` colors must arrive normalized, not as raw 0..255 bytes.
///
/// This is the specific thing that would break if the vertex descriptor were
/// dropped in favour of reading `RectInstance` as a plain MSL struct: Metal's
/// own alignment for `float4` does not reproduce the Rust layout, and pulling
/// bytes would hand the shader 255.0 instead of 1.0.
#[test]
fn unorm_vertex_attributes_reach_the_shader_normalized() {
    let Some(backend) = MetalBackend::new() else {
        eprintln!("skipping: this host exposes no Metal device");
        return;
    };
    let msl = include_str!("../../../backends/metal/shaders/metal/rect.metal");
    let module = backend
        .try_create_shader_module(msl.as_bytes(), "rect shader")
        .expect("rect.metal must compile as MSL");

    // Half-intensity grey: 128/255 of full.
    let grey = Rgba8::new(128, 128, 128, 255);
    let viewport_buffer = backend.create_buffer(&BufferDescriptor {
        label: Some("unorm viewport".to_string()),
        size: 16,
        usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
    });
    backend.write_buffer(
        &viewport_buffer,
        0,
        bytemuck::cast_slice(&[SIZE as f32, SIZE as f32, 0.0f32, 0.0f32]),
    );
    let layout = backend.create_bind_group_layout(&[BindGroupLayoutEntry {
        binding: 0,
        visibility: vec![ShaderStage::Vertex, ShaderStage::Fragment],
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }]);
    let bind_group = backend.create_bind_group(
        &layout,
        &[BindGroupEntry {
            binding: 0,
            resource: BindingResource::Buffer(&viewport_buffer),
        }],
    );
    let pipeline_layout = backend.create_pipeline_layout(&[&layout]);
    let attributes = rect_attributes();
    let vertex_buffers = [Some(VertexBufferLayout {
        array_stride: size_of::<RectInstance>() as u64,
        step_mode: VertexStepMode::Instance,
        attributes: &attributes,
    })];
    let targets = [Some(ColorTargetState {
        format: FORMAT,
        blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: ColorWriteMask::ALL,
    })];
    let pipeline = backend
        .try_create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("unorm rect".to_string()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &module,
                entry_point: "vs_main",
                buffers: &vertex_buffers,
            },
            fragment: Some(crate::backend::FragmentState {
                module: &module,
                entry_point: "fs_main",
                targets: &targets,
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: crate::backend::MultisampleState::default(),
        })
        .expect("the rect pipeline must build on Metal");

    let instances = [instance(0.0, 0.0, SIZE as f32, SIZE as f32, grey)];
    let instance_buffer = backend.create_buffer(&BufferDescriptor {
        label: Some("unorm instances".to_string()),
        size: (size_of::<RectInstance>()) as u64,
        usage: vec![BufferUsage::Vertex],
    });
    backend.write_buffer(&instance_buffer, 0, bytemuck::cast_slice(&instances));
    let target = backend.create_texture(&TextureDescriptor {
        label: Some("unorm target".to_string()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FORMAT,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "unorm view");

    let mut encoder = backend.create_command_encoder("unorm frame");
    {
        let mut pass = backend.begin_render_pass(
            &mut encoder,
            &RenderPassDescriptor {
                label: Some("unorm pass".to_string()),
                color_attachments: &[RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear([0.0, 0.0, 0.0, 0.0]),
                        store: StoreOp::Store,
                    },
                }],
                depth_stencil_attachment: None,
            },
        );
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, &instance_buffer, 0);
        pass.draw(0..6, 0..1);
    }
    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("unorm readback".to_string()),
        size: FRAME_BYTES,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
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
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(SIZE),
            },
        },
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);
    let pixels = backend.read_buffer(&readback, FRAME_BYTES);

    let centre = pixel(&pixels, 32, 32);
    assert_eq!(centre[3], 255, "alpha must stay opaque: {centre:?}");
    assert!(
        (118..=138).contains(&centre[0]),
        "128/255 unorm should land near mid grey after the sRGB round trip, got {centre:?}"
    );
    assert_eq!(centre[0], centre[1]);
    assert_eq!(centre[1], centre[2]);
}

#[test]
fn cropped_opacity_groups_render_with_msaa_through_native_metal() {
    let Some(backend) = MetalBackend::new() else {
        eprintln!("skipping: this host exposes no Metal device");
        return;
    };

    const TARGET_SIZE: u32 = 80;
    let format = <MetalBackend as GpuBackend>::rgba8_unorm_format();
    let target = backend.create_texture(&TextureDescriptor {
        label: Some("native Metal cropped opacity target".to_string()),
        size: (TARGET_SIZE, TARGET_SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "native Metal cropped opacity view");
    let mut renderer = RendererImpl::<MetalBackend>::with_antialiasing(
        &backend,
        format,
        crate::AntiAlias::Msaa4x,
    );

    let tree = RenderTree::new();
    let root = tree
        .add_root(Rect::new(0.0, 0.0, TARGET_SIZE as f32, TARGET_SIZE as f32))
        .unwrap();
    tree.set_clip(
        root,
        Some(Rect::new(0.0, 0.0, TARGET_SIZE as f32, TARGET_SIZE as f32)),
    )
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
    let blue = tree
        .add_child(nested, Rect::new(24.0, 24.0, 40.0, 40.0))
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
        blue,
        Rect::new(0.0, 0.0, 40.0, 40.0),
        Color::rgba8(0, 0, 255, 255),
    );
    record_v2_fill(
        &tree,
        yellow,
        Rect::new(0.0, 0.0, 32.0, 32.0),
        Color::rgba8(255, 255, 0, 255),
    );

    let metadata = || {
        FrameRenderMetadata::new(1.0, 301, 1, 1, 1, DamageSet::new(TARGET_SIZE, TARGET_SIZE))
    };
    let first_packet = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, TARGET_SIZE as f32, TARGET_SIZE as f32)],
            operations: tree.render_all(),
        },
        metadata(),
    )
    .expect("lower native Metal opacity groups");
    let first_plan = first_packet.render_plan().unwrap();
    let revisions = [green, blue, yellow].map(|node| {
        first_plan
            .local_v2_revision(node)
            .expect("retained child list")
    });
    renderer.render_packet(&backend, &view, &first_packet, false);

    let readback_row_bytes = (TARGET_SIZE * 4).div_ceil(256) * 256;
    let readback_size = u64::from(readback_row_bytes) * u64::from(TARGET_SIZE);
    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("native Metal cropped opacity readback".to_string()),
        size: readback_size,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let read_pixels = || {
        let mut encoder =
            backend.create_command_encoder("native Metal cropped opacity readback");
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
                    bytes_per_row: Some(readback_row_bytes),
                    rows_per_image: Some(TARGET_SIZE),
                },
            },
            Extent3d {
                width: TARGET_SIZE,
                height: TARGET_SIZE,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);
        backend.read_buffer(&readback, readback_size)
    };

    let first_pixels = read_pixels();
    assert_pixel_near(&first_pixels, readback_row_bytes, 20, 20, [0, 64, 0, 64]);
    assert_pixel_near(&first_pixels, readback_row_bytes, 40, 40, [0, 0, 64, 64]);
    assert_pixel_near(&first_pixels, readback_row_bytes, 70, 70, [64, 64, 0, 64]);
    let _ = tree.take_damage();

    tree.set_opacity(nested, 0.25).unwrap();
    let update_packet = FramePacket::from_v2_direct_with_frame(
        RenderFrame {
            damage: tree.take_damage(),
            operations: tree.render_all(),
        },
        metadata(),
        Frame::new(DrawList::new(), TARGET_SIZE, TARGET_SIZE),
    )
    .expect("lower native Metal MSAA opacity update");
    for (node, revision) in [green, blue, yellow].into_iter().zip(revisions) {
        assert_eq!(
            update_packet.render_plan().unwrap().local_v2_revision(node),
            Some(revision)
        );
    }
    renderer.render_packet(&backend, &view, &update_packet, false);
    let updated_pixels = read_pixels();
    assert_pixel_near(&updated_pixels, readback_row_bytes, 20, 20, [0, 32, 0, 32]);
    assert_pixel_near(&updated_pixels, readback_row_bytes, 40, 40, [0, 0, 32, 32]);
    assert_pixel_near(&updated_pixels, readback_row_bytes, 70, 70, [64, 64, 0, 64]);
    assert_eq!(
        pixel_with_stride(&updated_pixels, readback_row_bytes, 2, 2),
        [0, 0, 0, 0]
    );
}

/// Renders a capsule lens over a backdrop whose red channel is twice the column
/// (0..=254 across the 128-pixel frame), with the bulge at full strength and no
/// zoom or refraction, so a lens pixel's red reads back as twice the column it
/// sampled. Returns the frame, or `None` without a Metal device.
fn render_bulge_lens(bulge_radius: f32) -> Option<Vec<u8>> {
    render_lens(|request| request.bulge_radius = bulge_radius)
}

/// [`render_bulge_lens`]'s scene, with `configure` applied to the lens request
/// after the bulge setup, so a test can change the shape, style or bevel.
fn render_lens(
    configure: impl FnOnce(&mut crate::pipeline::material::MaterialRequest),
) -> Option<Vec<u8>> {
    use std::sync::Arc;

    use crate::pipeline::material::{
        MATERIAL_PIPELINE_NAME, MagnificationStyle, MaterialKind, MaterialRequest,
    };

    const FRAME: u32 = 128;
    const FRAME_ROW_BYTES: u32 = FRAME * 4;
    let Some(backend) = MetalBackend::new() else {
        eprintln!("skipping: this host exposes no Metal device");
        return None;
    };
    let mut renderer = RendererImpl::<MetalBackend>::new(&backend, FORMAT);
    let tree = RenderTree::new();
    let root = tree
        .add_root(Rect::new(0.0, 0.0, FRAME as f32, FRAME as f32))
        .unwrap();
    let lens = tree.add_child(root, Rect::new(8.0, 24.0, 112.0, 48.0)).unwrap();
    let columns = (0..FRAME)
        .map(|x| V2DrawCommand::FillRect {
            rect: Rect::new(x as f32, 0.0, 1.0, FRAME as f32),
            color: Color::rgba8((x * 2) as u8, 0, 0, 255),
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        })
        .collect();
    tree.context(root).unwrap().begin_recording().unwrap().commit(columns).unwrap();
    let mut request = MaterialRequest::new(MaterialKind::Magnification, [0.0, 0.0, 112.0, 48.0]);
    request.magnification_style = MagnificationStyle::UNIFORM | MagnificationStyle::BULGE;
    request.magnification = 1.0;
    request.bulge_strength = 1.0;
    request.bevel_radius = 0.0;
    request.corner_radii = [24.0; 4];
    request.opacity = 1.0;
    request.tint = [0.0; 4];
    request.border_width = 0.0;
    request.shadow_color = [0.0; 4];
    request.shadow_blur = 0.0;
    request.elevation = 0.0;
    request.edge_lighting = 0.0;
    request.specular_highlight = 0.0;
    request.distortion_strength = 0.0;
    request.chromatic_aberration = 0.0;
    configure(&mut request);
    tree.context(lens)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![V2DrawCommand::DrawCustom {
            pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
            data: Arc::from(request.encode_for_test()),
        }])
        .unwrap();

    let target = backend.create_texture(&TextureDescriptor {
        label: Some("Metal bulge lens target".to_string()),
        size: (FRAME, FRAME, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FORMAT,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "Metal bulge lens view");
    let packet = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, FRAME as f32, FRAME as f32)],
            operations: tree.render_all(),
        },
        FrameRenderMetadata::new(1.0, 601, 1, 1, 1, DamageSet::new(FRAME, FRAME)),
    )
    .expect("lower the bulge lens scene");
    renderer.render_packet(&backend, &view, &packet, false);
    let size = u64::from(FRAME_ROW_BYTES) * u64::from(FRAME);
    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("Metal bulge lens readback".to_string()),
        size,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("Metal bulge lens readback");
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
                bytes_per_row: Some(FRAME_ROW_BYTES),
                rows_per_image: Some(FRAME),
            },
        },
        Extent3d { width: FRAME, height: FRAME, depth_or_array_layers: 1 },
    );
    backend.submit(encoder);
    Some(backend.read_buffer(&readback, size))
}

/// The bulge's shoulder radius reshapes the swell the lens draws. On the lens's
/// center row, 40 pixels right of the middle (inside the capsule's end cap), the
/// bell has swelled only a little, so the lens shows content further out; the
/// slab, still on its shoulder, has swelled more and shows content nearer the
/// middle. The middle itself, where nothing is displaced, is the same either way.
#[test]
fn bulge_shoulder_radius_reshapes_the_swell_on_metal() {
    let (Some(bell), Some(slab)) = (render_bulge_lens(0.0), render_bulge_lens(24.0)) else {
        return;
    };
    let red = |frame: &[u8], x: u32| pixel_with_stride(frame, 128 * 4, x, 48)[0];
    // Bell: 64 + 40.5 / 1.2275 = column 97; slab: 64 + 40.5 / 1.935 = column 85.
    for (frame, label, expected) in [(&bell, "bell", 194.0), (&slab, "slab", 170.0)] {
        let actual = f32::from(red(frame, 104));
        assert!((actual - expected).abs() <= 8.0, "{label}: red {actual}, expected {expected}");
    }
    assert!(red(&bell, 104) > red(&slab, 104) + 12, "the slab swells more there");
    assert!(red(&bell, 64).abs_diff(red(&slab, 64)) <= 3, "the middle is not displaced");
}

/// With a radius narrower than half the height the lens has a plateau: an
/// inset copy of the capsule where the swell is flat, inside a ring, as wide as
/// the radius, where it rolls off. On the center row the plateau spans columns
/// 8+8 to 120-8 and the swell there is a constant 2x, so the lens shows the
/// column halfway between the middle and the pixel; in the ring it is less.
#[test]
fn bulge_plateau_is_flat_inside_a_ring_of_the_radius_on_metal() {
    let Some(frame) = render_bulge_lens(8.0) else { return };
    let red = |x: u32| f32::from(pixel_with_stride(&frame, 128 * 4, x, 48)[0]);
    // On the plateau a pixel dx right of the middle (64) samples 64 + dx / 2.
    for (x, expected) in [(80u32, 144.5), (88, 152.5), (104, 168.5), (110, 174.5)] {
        assert!((red(x) - expected).abs() <= 6.0, "plateau x {x}: red {}, expected {expected}", red(x));
    }
    // 1.5 px inside the right edge: rise 0.81, swell 1.58x, column 98.
    assert!((red(118) - 197.0).abs() <= 8.0, "ring: red {}", red(118));
    // The plateau is linear, so equal steps give equal red steps.
    let (a, b) = (red(88) - red(80), red(104) - red(96));
    assert!((a - b).abs() <= 3.0, "plateau steps {a} and {b}");
}

/// An explicit bevel width reaches past the corner radius. The derived width
/// stops at the smallest corner radius (4 here), so 6.5 px in from the left
/// edge, on the center row, nothing bends. An explicit width of 16 is measured
/// against an inner contour that keeps the corner radius, so the same pixel is
/// inside the band and the lens shows displaced content.
#[test]
fn explicit_bevel_width_is_not_capped_by_the_corner_radius_on_metal() {
    use crate::pipeline::material::MagnificationStyle;
    let lens = |width: f32| {
        render_lens(|request| {
            request.magnification_style = MagnificationStyle::UNIFORM | MagnificationStyle::REFRACTION;
            request.bulge_strength = 0.0;
            request.bevel_radius = 24.0;
            request.corner_radii = [4.0; 4];
            request.bevel_width = width;
        })
    };
    let (Some(derived), Some(explicit)) = (lens(0.0), lens(16.0)) else { return };
    let red = |frame: &[u8], x: u32| f32::from(pixel_with_stride(frame, 128 * 4, x, 48)[0]);
    // The gradient reads twice the column, and column 14.5 is undisplaced.
    assert!((red(&derived, 14) - 29.0).abs() <= 4.0, "derived: red {}", red(&derived, 14));
    assert!(
        (red(&explicit, 14) - red(&derived, 14)).abs() >= 8.0,
        "explicit: red {} against derived {}",
        red(&explicit, 14),
        red(&derived, 14)
    );
    // Deep inside either, nothing bends.
    assert!((red(&explicit, 64) - red(&derived, 64)).abs() <= 3.0);
}

/// The radial pull displaces smoothly across the whole lens: nothing at the
/// center, growing toward the rim, always toward the middle. On the center row of
/// a capsule with 20 px of thickness the pull at a pixel with inset t across the
/// 24 px band is 20 * (1 - t / 24)^2, so 1.5 px from the right edge it is about
/// 17.6 px and the lens shows content that far inward; at the middle it is zero.
#[test]
fn radial_pull_grows_from_nothing_at_the_middle_to_the_thickness_at_the_rim_on_metal() {
    use crate::pipeline::material::MagnificationStyle;
    let Some(frame) = render_lens(|request| {
        request.magnification_style = MagnificationStyle::UNIFORM | MagnificationStyle::RADIAL;
        request.bulge_strength = 0.0;
        request.bevel_radius = 20.0;
    }) else {
        return;
    };
    let red = |x: u32| f32::from(pixel_with_stride(&frame, 128 * 4, x, 48)[0]);
    // Undisplaced, a pixel reads twice its own column (+1 for its center).
    assert!((red(64) - 129.0).abs() <= 3.0, "middle: red {}", red(64));
    // Near the right rim the content comes from further in: 118.5 - 17.6 = column 100.9.
    assert!((red(118) - 202.0).abs() <= 8.0, "rim: red {}", red(118));
    assert!(red(118) < 2.0 * 118.5 - 20.0, "pulled inward, not out: red {}", red(118));
    // Mirror image on the left: 9.5 + 17.6 = column 27.1 (red 54), pulled inward too.
    assert!((red(9) - 54.0).abs() <= 8.0, "left rim: red {}", red(9));
    // The pull is monotonic: more displacement closer to the rim.
    let shift = |x: u32| (2.0 * (x as f32 + 0.5) - red(x)).abs();
    assert!(shift(100) < shift(110) && shift(110) < shift(116), "{} {} {}", shift(100), shift(110), shift(116));
}

/// Timing probe: `cargo nextest run -p aimer_cupid --run-ignored only --no-capture`.
///
/// What the raster thread pays, in the frame a decoded image is first drawn, to
/// create its texture and stage its pixels, before the blit the GPU runs itself.
#[test]
#[ignore = "timing probe, prints results"]
fn profile_first_upload_of_a_decoded_photo() {
    let Some(backend) = MetalBackend::new() else {
        return;
    };
    let mut pipeline = crate::pipeline::image_pipeline::ImagePipeline::new(
        &backend,
        FORMAT,
        crate::AntiAlias::Analytic,
    );
    for (index, (width, height)) in [(1024_u32, 768_u32), (2048, 1536), (2556, 1917), (4032, 3024)]
        .into_iter()
        .enumerate()
    {
        let pixels = (0..width as usize * height as usize * 4)
            .map(|byte| (byte % 251) as u8)
            .collect::<Vec<u8>>();
        let megabytes = pixels.len() as f64 / (1024.0 * 1024.0);
        let start = std::time::Instant::now();
        pipeline.upload_retained_image_with_id(
            &backend,
            index as u32 + 1,
            index as u64 + 1,
            width,
            height,
            &pixels,
        );
        println!(
            "{width}x{height} ({megabytes:5.1} MiB): create texture + stage pixels {:8.2} ms",
            start.elapsed().as_secs_f64() * 1e3
        );
    }
}
