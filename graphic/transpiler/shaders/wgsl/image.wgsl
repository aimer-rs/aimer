struct Viewport {
    size: vec2<f32>,
    surface_is_srgb: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) pixel_pos: vec2<f32>,
    @location(2) clip_rect: vec4<f32>,
    @location(3) clip_border_radius: vec4<f32>,
    @location(4) alpha: f32,
    @location(5) source_premultiplied: f32,
};

struct ImageInstance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_offset: vec2<f32>,
    @location(3) uv_scale: vec2<f32>,
    @location(4) clip_rect: vec4<f32>,
    @location(5) clip_border_radius: vec4<f32>,
    @location(6) alpha: f32,
    @location(7) source_premultiplied: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: ImageInstance) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );

    let corner = corners[vi];
    let pixel_pos = inst.pos + corner * inst.size;

    let ndc = vec2<f32>(
        (pixel_pos.x / viewport.size.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / viewport.size.y) * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = inst.uv_offset + corner * inst.uv_scale;
    out.pixel_pos = pixel_pos;
    out.clip_rect = inst.clip_rect;
    out.clip_border_radius = inst.clip_border_radius;
    out.alpha = inst.alpha;
    out.source_premultiplied = inst.source_premultiplied;
    return out;
}

/// Compute anti-aliased clip alpha from a clip rect in pixel coordinates.
/// clip_rect = (x, y, width, height). If width <= 0, clipping is disabled (returns 1.0).
/// SDF for a rounded rectangle with per-corner radii.
fn sdf_rounded_rect(p: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>) -> f32 {
    var r: f32;
    if p.x < 0.0 {
        if p.y < 0.0 {
            r = radii.x;
        } else {
            r = radii.w;
        }
    } else {
        if p.y < 0.0 {
            r = radii.y;
        } else {
            r = radii.z;
        }
    }
    r = min(r, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn clip_alpha(pixel_pos: vec2<f32>, clip_rect: vec4<f32>, clip_radii: vec4<f32>) -> f32 {
    if clip_rect.z < 0.0 {
        return 1.0;
    }

    if clip_rect.z <= 0.01 || clip_rect.w <= 0.01 {
        return 0.0;
    }

    if clip_radii.x > 0.0 || clip_radii.y > 0.0 || clip_radii.z > 0.0 || clip_radii.w > 0.0 {
        let clip_center = clip_rect.xy + clip_rect.zw * 0.5;
        let clip_half = clip_rect.zw * 0.5;
        let p = pixel_pos - clip_center;
        let d = sdf_rounded_rect(p, clip_half, clip_radii);
        return 1.0 - smoothstep(-0.5, 0.5, d);
    }

    let clip_min = clip_rect.xy;
    let clip_max = clip_rect.xy + clip_rect.zw;

    let d_left   = pixel_pos.x - clip_min.x;
    let d_right  = clip_max.x - pixel_pos.x;
    let d_top    = pixel_pos.y - clip_min.y;
    let d_bottom = clip_max.y - pixel_pos.y;

    // AA centered on the boundary so the clip does not erode ~1px inside each edge.
    let a_left   = smoothstep(-0.5, 0.5, d_left);
    let a_right  = smoothstep(-0.5, 0.5, d_right);
    let a_top    = smoothstep(-0.5, 0.5, d_top);
    let a_bottom = smoothstep(-0.5, 0.5, d_bottom);

    return a_left * a_right * a_top * a_bottom;
}




@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(t_diffuse, s_diffuse, in.uv);
    
    var result: vec4<f32>;
    if in.source_premultiplied > 0.5 {
        // Retained layers are render targets. Their RGB channels already
        // carry alpha, unlike decoded image uploads. On an unorm target the
        // stored channels are sRGB-encoded premultiplied values, so recover
        // straight sRGB before applying the ordinary image conversion. An
        // sRGB view decodes the layer to linear premultiplied values while it
        // is sampled; those values are already in the target blend space.
        let a = color_offset(color.a);
        if viewport.surface_is_srgb >= 1.5 {
            result = vec4<f32>(color.rgb, a);
        } else if viewport.surface_is_srgb < 0.5 {
            if a > 0.00001 {
                let straight = color.rgb / a;
                result = vec4<f32>(
                    linear_to_srgb(srgb_to_linear(straight.r)) * a,
                    linear_to_srgb(srgb_to_linear(straight.g)) * a,
                    linear_to_srgb(srgb_to_linear(straight.b)) * a,
                    a,
                );
            } else {
                result = vec4<f32>(0.0);
            }
        } else {
            result = vec4<f32>(color.rgb, a);
        }
    } else if viewport.surface_is_srgb >= 1.5 {
        // Ordinary decoded images are straight-alpha and already use the
        // platform's target encoding on Android.
        let a = color_offset(color.a);
        result = vec4<f32>(color.rgb * a, a);
    } else {
        color = vec4<f32>(
            srgb_to_linear(color.r),
            srgb_to_linear(color.g),
            srgb_to_linear(color.b),
            color_offset(color.a)
        );

        result = vec4<f32>(color.rgb * color.a, color.a);

        if viewport.surface_is_srgb < 0.5 {
            let a = color.a;
            if a > 0.00001 {
                let unpremul = result.rgb / a;
                let srgb_rgb = vec3<f32>(linear_to_srgb(unpremul.r), linear_to_srgb(unpremul.g), linear_to_srgb(unpremul.b));
                result = vec4<f32>(srgb_rgb * a, a);
            }
        }
    }
    
    let ca = clip_alpha(in.pixel_pos, in.clip_rect, in.clip_border_radius);
    return result * (ca * in.alpha);
}
