use aimer_material_pipeline::{
    GradientShape, MATERIAL_PIPELINE_NAME, MaterialGradient, MaterialKind, MaterialPipeline,
    MaterialRequest,
};
use crate::backend::{metal::MetalBackend, *};
use crate::render_list::DrawList;
use crate::renderer::RendererImpl;
use objc2_metal::MTLPixelFormat;

fn gradient_request(
    shape: GradientShape,
    geometry: [f32; 4],
    stops: &[(f32, [u8; 4])],
) -> MaterialRequest {
    let mut request = MaterialRequest::new(MaterialKind::Gradient, [16.0, 16.0, 96.0, 96.0]);
    request.opacity = 1.0;
    request.corner_radii = [0.0; 4];
    request.gradient = MaterialGradient {
        shape,
        geometry,
        stop_count: stops.len() as u8,
        ..MaterialGradient::default()
    };
    for (index, (offset, color)) in stops.iter().enumerate() {
        request.gradient.offsets[index] = *offset;
        request.gradient.colors[index] = MaterialGradient::pack_color(*color);
    }
    request
}

const SIZE: u32 = 128;
const ROW: u32 = SIZE * 4;
const BYTES: u64 = (ROW * SIZE) as u64;

fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * SIZE + x) * 4) as usize;
    bytes[offset..offset + 4].try_into().unwrap()
}

fn render(
    backend: &MetalBackend,
    renderer: &mut RendererImpl<MetalBackend>,
    draw: &DrawList,
) -> Vec<u8> {
    let target = backend.create_texture(&TextureDescriptor {
        label: None,
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: MTLPixelFormat::RGBA8Unorm,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "gradient pixels");
    renderer.render(backend, &view, SIZE, SIZE, false, draw);
    let readback = backend.create_buffer(&BufferDescriptor {
        label: None,
        size: BYTES,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("gradient readback");
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
                bytes_per_row: Some(ROW),
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
    backend.read_buffer(&readback, BYTES)
}

fn draw(request: MaterialRequest, scale: f32) -> DrawList {
    let mut draw = DrawList::new();
    draw.scale(scale, scale);
    draw.draw_custom(MATERIAL_PIPELINE_NAME, request.encode_for_test());
    draw
}

fn near(actual: [u8; 4], expected: [u8; 4], tolerance: u8, what: &str) {
    for (actual_channel, expected_channel) in actual.into_iter().zip(expected) {
        assert!(
            actual_channel.abs_diff(expected_channel) <= tolerance,
            "{what}: {actual:?} vs {expected:?}"
        );
    }
}

#[test]
fn a_linear_ramp_blends_between_stops_and_holds_past_the_ends() {
    let backend = MetalBackend::new().expect("gradient pixel regression requires Metal");
    let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
    // Black at a quarter of the width, white at three quarters.
    let request = gradient_request(
        GradientShape::Linear,
        [24.0, 0.0, 72.0, 0.0],
        &[(0.0, [0, 0, 0, 255]), (1.0, [255, 255, 255, 255])],
    );
    let pixels = render(&backend, &mut renderer, &draw(request, 1.0));
    near(pixel(&pixels, 18, 60), [0, 0, 0, 255], 2, "held before the first stop");
    near(pixel(&pixels, 64, 60), [128, 128, 128, 255], 4, "midpoint");
    near(pixel(&pixels, 110, 60), [255, 255, 255, 255], 2, "held after the last stop");
    near(pixel(&pixels, 48, 30), pixel(&pixels, 48, 90), 1, "constant across the ramp");
    assert!(pixel(&pixels, 40, 60)[0] < pixel(&pixels, 90, 60)[0], "monotonic");
    assert_eq!(pixel(&pixels, 8, 60), [0; 4], "nothing outside the bounds");
    assert_eq!(pixel(&pixels, 120, 60), [0; 4], "nothing outside the bounds");
}

#[test]
fn a_ramp_follows_device_scale_and_rounded_corners() {
    let backend = MetalBackend::new().expect("gradient pixel regression requires Metal");
    let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
    let mut request = gradient_request(
        GradientShape::Linear,
        [0.0, 0.0, 48.0, 0.0],
        &[(0.0, [255, 0, 0, 255]), (1.0, [0, 0, 255, 255])],
    );
    request.bounds = [8.0, 8.0, 48.0, 48.0];
    request.corner_radii = [20.0; 4];
    let pixels = render(&backend, &mut renderer, &draw(request, 2.0));
    near(pixel(&pixels, 64, 64), [128, 0, 128, 255], 6, "logical geometry reaches the ramp");
    assert!(pixel(&pixels, 16, 16)[3] < 40, "the corner is rounded at device scale");
    near(pixel(&pixels, 60, 20), [0, 0, 130, 255], 160, "inside the straight edge");
    assert!(pixel(&pixels, 60, 20)[3] > 200);
}

#[test]
fn stop_alpha_is_blended_premultiplied() {
    let backend = MetalBackend::new().expect("gradient pixel regression requires Metal");
    let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
    // Opaque red to fully transparent blue: the transparent end must not
    // tint the middle toward blue.
    let request = gradient_request(
        GradientShape::Linear,
        [0.0, 0.0, 96.0, 0.0],
        &[(0.0, [255, 0, 0, 255]), (1.0, [0, 0, 255, 0])],
    );
    let pixels = render(&backend, &mut renderer, &draw(request, 1.0));
    let middle = pixel(&pixels, 64, 60);
    assert!(middle[0].abs_diff(128) <= 6, "premultiplied red: {middle:?}");
    assert!(middle[2] <= 2, "transparent stop adds no blue: {middle:?}");
    assert!(middle[3].abs_diff(128) <= 6, "alpha interpolates: {middle:?}");
}

#[test]
fn radial_and_conic_ramps_vary_with_distance_and_angle() {
    let backend = MetalBackend::new().expect("gradient pixel regression requires Metal");
    let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
    let radial = gradient_request(
        GradientShape::Radial,
        [48.0, 48.0, 24.0, 24.0],
        &[(0.0, [255, 255, 255, 255]), (1.0, [0, 0, 0, 255])],
    );
    let pixels = render(&backend, &mut renderer, &draw(radial, 1.0));
    // The pixel center sits 0.7 px from the ramp center: t = 0.7 / 24.
    near(pixel(&pixels, 64, 64), [255, 255, 255, 255], 10, "center is the first stop");
    near(pixel(&pixels, 64 + 12, 64), [128, 128, 128, 255], 8, "half radius");
    near(pixel(&pixels, 64 + 40, 64), [0, 0, 0, 255], 4, "past the radius holds the last stop");

    let conic = gradient_request(
        GradientShape::Conic,
        [48.0, 48.0, 0.0, 0.0],
        &[(0.0, [255, 0, 0, 255]), (0.5, [0, 255, 0, 255]), (1.0, [255, 0, 0, 255])],
    );
    let pixels = render(&backend, &mut renderer, &draw(conic, 1.0));
    near(pixel(&pixels, 64 + 40, 64 + 1), [255, 0, 0, 255], 24, "just past the start angle");
    near(pixel(&pixels, 64 - 40, 64), [0, 255, 0, 255], 24, "half a turn around");
}

#[test]
fn the_coverage_mask_marker_is_skipped_until_the_renderer_composites_masks() {
    let backend = MetalBackend::new().expect("gradient pixel regression requires Metal");
    let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
    let request = gradient_request(
        GradientShape::Linear,
        [0.0, 0.0, 96.0, 0.0],
        &[(0.0, [255, 0, 0, 255]), (1.0, [0, 0, 255, 255])],
    );
    let plain = render(&backend, &mut renderer, &draw(request, 1.0));

    let mut marked = DrawList::new();
    marked.draw_custom(
        crate::draw_cmd::COVERAGE_MASK_PIPELINE_NAME,
        Vec::<u8>::new(),
    );
    marked.draw_custom(MATERIAL_PIPELINE_NAME, request.encode_for_test());
    let with_marker = render(&backend, &mut renderer, &marked);
    assert_eq!(
        plain, with_marker,
        "an unregistered pipeline name must not disturb the commands around it"
    );
    assert_ne!(pixel(&with_marker, 64, 60), [0; 4], "the gradient still draws");
}
