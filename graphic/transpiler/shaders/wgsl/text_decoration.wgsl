// Text decoration shader — draws styled decoration lines (underline, overline,
// line-through) as solid-color quads with a procedural coverage mask so a single
// pipeline covers Solid / Double / Dotted / Dashed / Wavy without any atlas.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) pixel_pos: vec2<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_border_radius: vec4<f32>,
    // params: [style_id, thickness_px, period_px, band_height_px]
    @location(5) params: vec4<f32>,
    @location(6) size: vec2<f32>,
};

struct FragmentInput {
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) pixel_pos: vec2<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_border_radius: vec4<f32>,
    @location(5) params: vec4<f32>,
    @location(6) size: vec2<f32>,
};

struct Viewport {
    resolution: vec2<f32>,
    surface_is_srgb: f32,
    _pad: f32,
};

// Bindings mirror the text pipeline's layout (viewport + atlas + sampler) so the
// same bind group can be reused; the atlas/sampler are intentionally unused here.
@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_idx: u32,
    @location(0) inst_pos: vec2<f32>,
    @location(1) inst_size: vec2<f32>,
    @location(2) inst_color: vec4<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_radius: vec4<f32>,
    @location(5) inst_params: vec4<f32>,
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

    let pixel_pos = inst_pos + c * inst_size;
    let ndc = vec2<f32>(
        pixel_pos.x / viewport.resolution.x * 2.0 - 1.0,
        -(pixel_pos.y / viewport.resolution.y * 2.0 - 1.0),
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = c;
    out.color = inst_color;
    out.pixel_pos = pixel_pos;
    out.clip_rect = clip_rect;
    out.clip_border_radius = clip_radius;
    out.params = inst_params;
    out.size = inst_size;
    return out;
}

fn sdf_rounded_rect(p: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>) -> f32 {
    var r: f32;
    if p.x < 0.0 {
        if p.y < 0.0 { r = radii.x; } else { r = radii.w; }
    } else {
        if p.y < 0.0 { r = radii.y; } else { r = radii.z; }
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
    // AA centered on the boundary so the clip does not erode ~1px inside each edge.
    let a_left = smoothstep(-0.5, 0.5, pixel_pos.x - clip_min.x);
    let a_right = smoothstep(-0.5, 0.5, clip_max.x - pixel_pos.x);
    let a_top = smoothstep(-0.5, 0.5, pixel_pos.y - clip_min.y);
    let a_bottom = smoothstep(-0.5, 0.5, clip_max.y - pixel_pos.y);
    return a_left * a_right * a_top * a_bottom;
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { return c / 12.92; }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { return c * 12.92; }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

// Anti-aliased "distance to stroke edge" coverage: 1 inside the stroke, 0 outside,
// with a ~1px smooth transition.
fn stroke_cov(dist: f32, half_thickness: f32) -> f32 {
    return smoothstep(0.5, -0.5, dist - half_thickness);
}

@fragment
fn fs_main(in: FragmentInput) -> @location(0) vec4<f32> {
    let style = in.params.x;
    let thickness = max(in.params.y, 0.5);
    let period = max(in.params.z, 1.0);
    let band = max(in.params.w, thickness);

    // Local coordinates inside the band's quad, in pixels.
    let px = in.uv.x * in.size.x;
    let py = in.uv.y * in.size.y;
    let center = band * 0.5;
    let ht = thickness * 0.5;

    var cov = 0.0;
    if style < 0.5 {
        // Solid.
        cov = stroke_cov(abs(py - center), ht);
    } else if style < 1.5 {
        // Double: two strokes at the top and bottom of the band.
        let c0 = ht;
        let c1 = band - ht;
        cov = max(stroke_cov(abs(py - c0), ht), stroke_cov(abs(py - c1), ht));
    } else if style < 2.5 {
        // Dotted: solid vertically, round dots along x (~50% duty at `period`).
        let line_pos = stroke_cov(abs(py - center), ht);
        let phase = px - period * floor(px / period);
        let dot = stroke_cov(abs(phase - period * 0.5), period * 0.25);
        cov = line_pos * dot;
    } else if style < 3.5 {
        // Dashed: solid vertically, longer dashes along x (~60% duty).
        let line_pos = stroke_cov(abs(py - center), ht);
        let phase = px - period * floor(px / period);
        let dash = step(phase, period * 0.6);
        cov = line_pos * dash;
    } else {
        // Wavy: sine curve whose amplitude fills the remaining band height.
        let amp = max((band - thickness) * 0.5, 0.0);
        let wave = center + amp * sin(px * 6.2831853 / period);
        cov = stroke_cov(abs(py - wave), ht);
    }

    let ca = clip_alpha(in.pixel_pos, in.clip_rect, in.clip_border_radius);
    let a = in.color.a * cov * ca;

    var result: vec4<f32>;
    if viewport.surface_is_srgb >= 1.5 {
        result = vec4<f32>(in.color.rgb * a, a);
    } else {
        let r_lin = srgb_to_linear(in.color.r);
        let g_lin = srgb_to_linear(in.color.g);
        let b_lin = srgb_to_linear(in.color.b);
        result = vec4<f32>(r_lin * a, g_lin * a, b_lin * a, a);
        if viewport.surface_is_srgb < 0.5 {
            if a > 0.00001 {
                result = vec4<f32>(linear_to_srgb(r_lin) * a, linear_to_srgb(g_lin) * a, linear_to_srgb(b_lin) * a, a);
            }
        }
    }
    return result;
}
