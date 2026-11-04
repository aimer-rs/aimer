//! OpenGL backend tests that drive the real renderer.
//!
//! They live beside the renderer rather than in `aimer_opengl` because
//! they need draw lists, and the backend crate must not depend on them.

use aimer_opengl::*;
use crate::backend::*;
use crate::render_list::DrawList;
use crate::renderer::RendererImpl;
use crate::utilities::{Color, Rect};

#[test]
fn generic_renderer_renders_and_reads_back_with_headless_opengl() {
    const SIZE: u32 = 32;
    let backend = match OpenGlBackend::new_headless() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("skipping: a headless OpenGL 3.3 context is unavailable: {error}");
            return;
        }
    };
    let mut renderer =
        RendererImpl::<OpenGlBackend>::new(&backend, OpenGlBackend::rgba8_unorm_format());
    let mut draw = DrawList::new();
    draw.fill_rect(
        Rect::new(0.0, 0.0, SIZE as f32, (SIZE / 2) as f32),
        Color::red(),
        [0.0; 4],
        [0.0; 4],
        Color::transparent(),
    );
    draw.fill_rect(
        Rect::new(0.0, (SIZE / 2) as f32, SIZE as f32, (SIZE / 2) as f32),
        Color::blue(),
        [0.0; 4],
        [0.0; 4],
        Color::transparent(),
    );
    let target = backend.create_texture(&TextureDescriptor {
        label: Some("Cupid OpenGL readback test target".to_owned()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: OpenGlBackend::rgba8_unorm_format(),
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "Cupid OpenGL test view");
    renderer.render(&backend, &view, SIZE, SIZE, false, &draw);
    let gl_error = backend
        .pending_gl_error()
        .expect("the backend context should be current");
    assert_eq!(gl_error, 0, "OpenGL reported an error after rendering");

    let row_bytes = SIZE * 4;
    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("Cupid OpenGL readback test buffer".to_owned()),
        size: (row_bytes * SIZE) as u64,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("Cupid OpenGL readback test");
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
    let pixels = backend.read_buffer(&readback, (row_bytes * SIZE) as u64);
    let colored_pixels = pixels
        .chunks_exact(4)
        .filter(|pixel| pixel.iter().any(|channel| *channel != 0))
        .count();
    assert_eq!(
        &pixels[..4],
        &[0, 0, 255, 255],
        "the bottom logical band should stay at the bottom"
    );
    let top = ((SIZE - 1) * row_bytes) as usize;
    assert_eq!(
        &pixels[top..top + 4],
        &[255, 0, 0, 255],
        "the top logical band should stay at the top"
    );
    assert_eq!(colored_pixels, (SIZE * SIZE) as usize);
}

/// Regression: the SVG pipeline is the only indexed draw. The command-encoder
/// path used to record the index buffer without binding it to
/// `GL_ELEMENT_ARRAY_BUFFER`, so `glDrawElements` read from client memory at a
/// null base and crashed inside the driver.
#[test]
fn svg_indexed_draw_binds_its_index_buffer_with_headless_opengl() {
    use std::sync::Arc;

    const SIZE: u32 = 32;
    let backend = match OpenGlBackend::new_headless() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("skipping: a headless OpenGL 3.3 context is unavailable: {error}");
            return;
        }
    };
    let mut renderer =
        RendererImpl::<OpenGlBackend>::new(&backend, OpenGlBackend::rgba8_unorm_format());
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
    let mut draw = DrawList::new();
    draw.draw_svg(
        svg_scene,
        Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
        Arc::new([]),
    );
    let target = backend.create_texture(&TextureDescriptor {
        label: Some("Cupid OpenGL SVG test target".to_owned()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: OpenGlBackend::rgba8_unorm_format(),
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "Cupid OpenGL SVG test view");
    renderer.render(&backend, &view, SIZE, SIZE, false, &draw);
    let gl_error = backend
        .pending_gl_error()
        .expect("the backend context should be current");
    assert_eq!(gl_error, 0, "OpenGL reported an error after the SVG draw");

    let row_bytes = SIZE * 4;
    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("Cupid OpenGL SVG readback buffer".to_owned()),
        size: (row_bytes * SIZE) as u64,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("Cupid OpenGL SVG readback");
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
    let pixels = backend.read_buffer(&readback, (row_bytes * SIZE) as u64);
    let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    assert_eq!(&pixels[center..center + 4], &[255, 0, 0, 255]);
}

fn read_gl_target(backend: &OpenGlBackend, target: &OpenGlTexture, size: u32) -> Vec<u8> {
    let row_bytes = size * 4;
    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("Cupid OpenGL readback buffer".to_owned()),
        size: (row_bytes * size) as u64,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("Cupid OpenGL readback");
    backend.copy_texture_to_buffer(
        &mut encoder,
        &TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(size),
            },
        },
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);
    backend.read_buffer(&readback, (row_bytes * size) as u64)
}

/// Reads a pixel by top-left logical coordinates. OpenGL readback is bottom-up.
fn gl_pixel(data: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = (((size - 1 - y) * size + x) * 4) as usize;
    data[offset..offset + 4].try_into().expect("RGBA pixel")
}

#[test]
fn opacity_groups_with_partial_damage_update_on_headless_opengl() {
    use crate::damage_region::DamageSet;
    use crate::draw_cmd::{DrawCommand as V2DrawCommand, RenderFrame, RenderTree};
    use crate::frame::{Frame, FramePacket, FrameRenderMetadata};

    const SIZE: u32 = 80;
    let backend = match OpenGlBackend::new_headless() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("skipping: a headless OpenGL 3.3 context is unavailable: {error}");
            return;
        }
    };
    let mut renderer =
        RendererImpl::<OpenGlBackend>::new(&backend, OpenGlBackend::rgba8_unorm_format());
    let target = backend.create_texture(&TextureDescriptor {
        label: Some("Cupid OpenGL opacity target".to_owned()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: OpenGlBackend::rgba8_unorm_format(),
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "Cupid OpenGL opacity view");
    let tree = RenderTree::new();
    let root = tree
        .add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32))
        .unwrap();
    tree.set_opacity(root, 0.5).unwrap();
    let group = tree
        .add_child(root, Rect::new(48.0, 48.0, 32.0, 32.0))
        .unwrap();
    tree.set_opacity(group, 0.5).unwrap();
    let yellow = tree
        .add_child(group, Rect::new(0.0, 0.0, 32.0, 32.0))
        .unwrap();
    tree.context(yellow)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![V2DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 32.0, 32.0),
            color: Color::rgba8(255, 255, 0, 255),
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        }])
        .unwrap();

    let metadata = || FrameRenderMetadata::new(1.0, 401, 1, 1, 1, DamageSet::new(SIZE, SIZE));
    let first = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)],
            operations: tree.render_all(),
        },
        metadata(),
    )
    .expect("lower OpenGL opacity groups");
    renderer.render_packet(&backend, &view, &first, false);
    let first_pixels = read_gl_target(&backend, &target, SIZE);
    let _ = tree.take_damage();
    assert_eq!(
        gl_pixel(&first_pixels, SIZE, 2, 2),
        [0, 0, 0, 0],
        "untouched area must stay transparent, not white"
    );
    let first_yellow = gl_pixel(&first_pixels, SIZE, 70, 70);
    assert!(
        first_yellow[2] <= 2 && first_yellow[0].abs_diff(63) <= 2,
        "group pixel should be yellow-ish, got {first_yellow:?}"
    );

    tree.set_opacity(group, 0.25).unwrap();
    let damage = tree.take_damage();
    let update = FramePacket::from_v2_direct_with_frame(
        RenderFrame {
            damage,
            operations: tree.render_all(),
        },
        metadata(),
        Frame::new(DrawList::new(), SIZE, SIZE),
    )
    .expect("lower OpenGL opacity update");
    renderer.render_packet(&backend, &view, &update, false);
    let updated = read_gl_target(&backend, &target, SIZE);
    assert_eq!(
        gl_pixel(&updated, SIZE, 2, 2),
        [0, 0, 0, 0],
        "untouched area must stay transparent after a partial update"
    );
    let updated_yellow = gl_pixel(&updated, SIZE, 70, 70);
    assert!(
        updated_yellow[2] <= 2 && updated_yellow[0].abs_diff(32) <= 2,
        "partial update should replace the group at 12.5% opacity, got {updated_yellow:?} (was {first_yellow:?})"
    );
}

/// root (red top, blue bottom) -> a 96 x 96 node at `opacity` that desaturates
/// whatever is behind it, rendered on headless OpenGL.
fn render_faded_filter(opacity: f32) -> Option<Vec<u8>> {
    render_material_node(opacity, |_| {})
}

/// Like [`render_faded_filter`], letting the caller reshape the material.
fn render_material_node(
    opacity: f32,
    configure: impl FnOnce(&mut aimer_material_pipeline::MaterialRequest),
) -> Option<Vec<u8>> {
    // red on the top half, blue on the bottom half: a vertical flip must show.
    use aimer_material_pipeline::{MATERIAL_PIPELINE_NAME, MaterialKind, MaterialRequest};
    use crate::damage_region::DamageSet;
    use crate::draw_cmd::{DrawCommand as V2DrawCommand, RenderFrame, RenderTree};
    use crate::frame::{FramePacket, FrameRenderMetadata};
    use std::sync::Arc;

    const SIZE: u32 = 128;
    let backend = match OpenGlBackend::new_headless() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("skipping: a headless OpenGL 3.3 context is unavailable: {error}");
            return None;
        }
    };
    let mut renderer =
        RendererImpl::<OpenGlBackend>::new(&backend, OpenGlBackend::rgba8_unorm_format());
    let fill = |rect: Rect, color: Color| V2DrawCommand::FillRect {
        rect,
        color,
        border_radius: [0.0; 4],
        border_width: [0.0; 4],
        border_color: Color::transparent(),
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    };
    let tree = RenderTree::new();
    let root = tree
        .add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32))
        .unwrap();
    let node = tree
        .add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0))
        .unwrap();
    tree.context(root)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![
            fill(Rect::new(0.0, 0.0, SIZE as f32, 64.0), Color::rgba8(255, 0, 0, 255)),
            fill(Rect::new(0.0, 64.0, SIZE as f32, 64.0), Color::rgba8(0, 0, 255, 255)),
        ])
        .unwrap();
    let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 96.0, 96.0]);
    filter.blur_radius = 0.0;
    // Identity filter: the node shows the backdrop it was given, unchanged.
    filter.saturation = 1.0;
    filter.opacity = 1.0;
    filter.corner_radii = [0.0; 4];
    configure(&mut filter);
    tree.context(node)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![V2DrawCommand::DrawCustom {
            pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
            data: Arc::from(filter.encode_for_test()),
        }])
        .unwrap();
    tree.set_opacity(node, opacity).unwrap();

    let target = backend.create_texture(&TextureDescriptor {
        label: Some("Cupid OpenGL faded filter target".to_owned()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: OpenGlBackend::rgba8_unorm_format(),
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "Cupid OpenGL faded filter view");
    let packet = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)],
            operations: tree.render_all(),
        },
        FrameRenderMetadata::new(1.0, 501, 1, 1, 1, DamageSet::new(SIZE, SIZE)),
    )
    .expect("lower the faded filter scene");
    renderer.render_packet(&backend, &view, &packet, false);
    assert_eq!(
        backend.pending_gl_error().expect("context is current"),
        0,
        "OpenGL reported an error while drawing the filter"
    );
    Some(read_gl_target(&backend, &target, SIZE))
}

/// The identity filter must show the red above and the blue below, whether or
/// not the node needs a group of its own.
fn assert_backdrop_orientation(opacity: f32) {
    let Some(pixels) = render_faded_filter(opacity) else { return };
    assert_red_over_blue(&pixels, &format!("opacity {opacity}"));
}

fn assert_red_over_blue(pixels: &[u8], what: &str) {
    let top = gl_pixel_sized(pixels, 128, 64, 40);
    let bottom = gl_pixel_sized(pixels, 128, 64, 100);
    assert!(
        top[0] > 200 && top[2] < 60,
        "{what}: the node's top shows the red backdrop, got {top:?}"
    );
    assert!(
        bottom[2] > 200 && bottom[0] < 60,
        "{what}: the node's bottom shows the blue backdrop, got {bottom:?}"
    );
}

#[test]
fn backdrop_filter_in_an_opaque_node_keeps_the_backdrop_upright_on_headless_opengl() {
    assert_backdrop_orientation(1.0);
}

/// Regression: a glass surface fades in as an opacity group seeded with the
/// frame behind it. The seed copy used top-left rows on a bottom-up target, so
/// the group sampled the wrong backdrop and the glass came out black.
#[test]
fn backdrop_filter_in_a_faded_node_keeps_the_backdrop_upright_on_headless_opengl() {
    assert_backdrop_orientation(0.5);
}

fn gl_pixel_sized(data: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
    gl_pixel(data, size, x, y)
}

/// The liquid tab bar's lens: a magnifier with refraction over a backdrop. It
/// came out as a black area instead of the backdrop it enlarges.
#[test]
fn magnification_lens_shows_the_upright_backdrop_on_headless_opengl() {
    use aimer_material_pipeline::{MaterialKind, MagnificationStyle};
    for opacity in [1.0, 0.5] {
        let Some(pixels) = render_material_node(opacity, |request| {
            request.kind = MaterialKind::Magnification;
            request.magnification = 1.25;
            request.bevel_radius = 10.0;
            request.bulge_strength = 0.15;
            request.magnification_style = MagnificationStyle::DEFAULT;
            request.tint = [0.0; 4];
            request.border_width = 0.0;
            request.shadow_color = [0.0; 4];
            request.shadow_blur = 0.0;
            request.elevation = 0.0;
            request.edge_lighting = 0.0;
            request.specular_highlight = 0.0;
            request.distortion_strength = 0.0;
            request.chromatic_aberration = 0.0;
        }) else {
            return;
        };
        assert_red_over_blue(&pixels, &format!("lens at opacity {opacity}"));
    }
}

/// A blurred backdrop filter reads the capture through the blur passes.
#[test]
fn blurred_backdrop_filter_keeps_the_backdrop_upright_on_headless_opengl() {
    for blur in [4.0, 24.0] {
        let Some(pixels) = render_material_node(1.0, |request| request.blur_radius = blur) else {
            return;
        };
        assert_red_over_blue(&pixels, &format!("blur {blur}"));
    }
}

/// Regression: every window owns an OpenGL context, but they share one EGL
/// display. Closing a window terminated that display and left the other
/// windows' contexts unusable, so the parent could no longer render.
#[test]
fn dropping_one_opengl_context_leaves_the_others_usable() {
    const SIZE: u32 = 16;
    let (Ok(parent), Ok(child)) = (OpenGlBackend::new_headless(), OpenGlBackend::new_headless())
    else {
        eprintln!("skipping: a headless OpenGL 3.3 context is unavailable");
        return;
    };
    drop(child);

    let mut renderer =
        RendererImpl::<OpenGlBackend>::new(&parent, OpenGlBackend::rgba8_unorm_format());
    let mut draw = DrawList::new();
    draw.fill_rect(
        Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
        Color::red(),
        [0.0; 4],
        [0.0; 4],
        Color::transparent(),
    );
    let target = parent.create_texture(&TextureDescriptor {
        label: Some("Cupid OpenGL surviving context target".to_owned()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: OpenGlBackend::rgba8_unorm_format(),
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = parent.create_texture_view(&target, "Cupid OpenGL surviving context view");
    renderer.render(&parent, &view, SIZE, SIZE, false, &draw);
    let pixels = read_gl_target(&parent, &target, SIZE);
    let center = gl_pixel(&pixels, SIZE, 8, 8);
    assert_eq!(center, [255, 0, 0, 255], "the remaining context still renders");
}
