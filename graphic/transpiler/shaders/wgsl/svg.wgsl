struct SvgVertex {
    @location(0) position: vec2<f32>,
    @location(7) coverage: f32,
};

struct SvgInstance {
    @location(1) transform_x: vec4<f32>,
    @location(2) transform_y: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) clip_rect: vec4<f32>,
    @location(5) clip_border_radius: vec4<f32>,
    @location(6) viewport: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) pixel_pos: vec2<f32>,
    @location(2) clip_rect: vec4<f32>,
    @location(3) clip_border_radius: vec4<f32>,
    @location(4) surface_is_srgb: f32,
    @location(5) coverage: f32,
};

@vertex
fn vs_main(vertex: SvgVertex, instance: SvgInstance) -> VertexOutput {
    let local = vec3<f32>(vertex.position, 1.0);
    let pixel_pos = vec2<f32>(
        dot(local, instance.transform_x.xyz),
        dot(local, instance.transform_y.xyz),
    );
    let ndc = vec2<f32>(
        pixel_pos.x / instance.viewport.x * 2.0 - 1.0,
        1.0 - pixel_pos.y / instance.viewport.y * 2.0,
    );
    var output: VertexOutput;
    output.position = vec4<f32>(ndc, 0.0, 1.0);
    output.color = instance.color;
    output.pixel_pos = pixel_pos;
    output.clip_rect = instance.clip_rect;
    output.clip_border_radius = instance.clip_border_radius;
    output.surface_is_srgb = instance.viewport.z;
    output.coverage = vertex.coverage;
    return output;
}

fn rounded_rect_distance(input_point: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>) -> f32 {
    var radius: f32;
    if input_point.x < 0.0 {
        radius = select(radii.w, radii.x, input_point.y < 0.0);
    } else {
        radius = select(radii.z, radii.y, input_point.y < 0.0);
    }
    radius = min(radius, min(half_size.x, half_size.y));
    let q = abs(input_point) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn clip_alpha(pixel_pos: vec2<f32>, clip_rect: vec4<f32>, radii: vec4<f32>) -> f32 {
    if clip_rect.z <= 0.0 {
        return 1.0;
    }
    let half_size = clip_rect.zw * 0.5;
    let center = clip_rect.xy + half_size;
    let distance = rounded_rect_distance(pixel_pos - center, half_size, radii);
    return 1.0 - smoothstep(-0.5, 0.5, distance);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let clip = clip_alpha(input.pixel_pos, input.clip_rect, input.clip_border_radius);
    var color = input.color;
    if input.surface_is_srgb < 1.5 {
        color = vec4<f32>(
            srgb_to_linear(color.r),
            srgb_to_linear(color.g),
            srgb_to_linear(color.b),
            color.a,
        );
    }
    var result = vec4<f32>(color.rgb * color.a, color.a) * clip * input.coverage;
    if input.surface_is_srgb < 0.5 && result.a > 0.000001 {
        let unpremultiplied = result.rgb / result.a;
        result = vec4<f32>(
            vec3<f32>(linear_to_srgb(unpremultiplied.r), linear_to_srgb(unpremultiplied.g), linear_to_srgb(unpremultiplied.b)) * result.a,
            result.a,
        );
    }
    return result;
}
