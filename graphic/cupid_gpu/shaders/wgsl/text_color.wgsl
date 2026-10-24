// Color text shader — for COLR layers and embedded color-strike glyphs.
// Mirrors text.wgsl but samples the full RGBA color from the atlas and
// modulates only by the per-instance alpha + clip alpha. The atlas stores
// non-premultiplied RGBA8; output is premultiplied so the same blend state
// (PREMULTIPLIED_ALPHA_BLENDING) applies as for monochrome glyphs.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) pixel_pos: vec2<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_border_radius: vec4<f32>,
};

struct FragmentInput {
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) pixel_pos: vec2<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_border_radius: vec4<f32>,
};

struct Viewport {
    resolution: vec2<f32>,
    surface_is_srgb: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_idx: u32,
    @location(0) inst_pos: vec2<f32>,
    @location(1) inst_size: vec2<f32>,
    @location(2) inst_uv: vec4<f32>,
    @location(3) inst_color: vec4<f32>,
    @location(4) clip_rect: vec4<f32>,
    @location(5) clip_radius: vec4<f32>,
    @location(6) inst_skew: f32,
) -> VertexOutput {
    let tri_idx = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let c = tri_idx[vertex_idx];

    // Synthetic italic shear, pinned at the quad's bottom edge (see text.wgsl).
    var pixel_pos = inst_pos + c * inst_size;
    pixel_pos.x = pixel_pos.x + inst_skew * inst_size.y * (1.0 - c.y);
    let ndc = vec2<f32>(
        pixel_pos.x / viewport.resolution.x * 2.0 - 1.0,
        -(pixel_pos.y / viewport.resolution.y * 2.0 - 1.0),
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = mix(inst_uv.xy, inst_uv.zw, c);
    out.color = inst_color;
    out.pixel_pos = pixel_pos;
    out.clip_rect = clip_rect;
    out.clip_border_radius = clip_radius;
    return out;
}

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

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

@fragment
fn fs_main(in: FragmentInput) -> @location(0) vec4<f32> {
    // Atlas stores non-premultiplied RGBA. We don't tint with `in.color.rgb` —
    // emoji are full-color art; we only honour the layout's alpha and clip.
    let sample = textureSampleLevel(atlas_texture, atlas_sampler, in.uv, 0.0);
    let ca = clip_alpha(in.pixel_pos, in.clip_rect, in.clip_border_radius);
    let glyph_alpha = sample.a;
    let final_alpha = glyph_alpha * in.color.a * ca;

    var rgb = sample.rgb;

    // Match the gamma/space behaviour of text.wgsl. Color emoji bitmaps are
    // sRGB encoded; on Android (surface_is_srgb >= 1.5), the surface skips
    // sRGB conversion entirely so we feed sRGB straight through. Otherwise we
    // convert sRGB → linear, optionally back to sRGB if the surface format is
    // already linear.
    if viewport.surface_is_srgb < 1.5 {
        let r_lin = srgb_to_linear(rgb.r);
        let g_lin = srgb_to_linear(rgb.g);
        let b_lin = srgb_to_linear(rgb.b);

        if viewport.surface_is_srgb < 0.5 {
            rgb = vec3<f32>(linear_to_srgb(r_lin), linear_to_srgb(g_lin), linear_to_srgb(b_lin));
        } else {
            rgb = vec3<f32>(r_lin, g_lin, b_lin);
        }
    }

    return vec4<f32>(rgb * final_alpha, final_alpha);
}
