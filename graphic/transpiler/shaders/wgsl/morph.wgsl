// Blends two offscreen pictures with a transition effect.
//
// Both sources are premultiplied renderer-owned group targets, so their
// channels are already in the target's blend space and are mixed as stored.
// The result is premultiplied and composited over the parent with ordinary
// premultiplied-alpha blending.
//
// Styles (timing.x): 0 dissolve, 1 wipe, 2 liquid, 3 bounds, 4 blur, 5 slide,
// 6 goo.
//
// Every style honors a blur (noise.y, target pixels): the outgoing picture is
// blurred by `blur * progress` and the incoming one by `blur * (1 - progress)`,
// so each is sharp while it is the only one showing.

struct Viewport {
    size: vec2<f32>,
    surface_is_srgb: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(1) @binding(0) var t_from: texture_2d<f32>;
@group(1) @binding(1) var t_to: texture_2d<f32>;
@group(1) @binding(2) var s_morph: sampler;

struct MorphInstance {
    // x, y, width, height in target pixels; a width or height of zero means the
    // source is absent and contributes nothing.
    @location(0) rect_from: vec4<f32>,
    @location(1) rect_to: vec4<f32>,
    // style, progress, opacity, seed
    @location(2) timing: vec4<f32>,
    // direction x, direction y, softness, amplitude
    @location(3) shape: vec4<f32>,
    // frequency, blur radius in target pixels, unused, unused
    @location(4) noise: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) pixel_pos: vec2<f32>,
    @location(1) rect_from: vec4<f32>,
    @location(2) rect_to: vec4<f32>,
    @location(3) timing: vec4<f32>,
    @location(4) shape: vec4<f32>,
    @location(5) noise: vec4<f32>,
};

// The alpha, after averaging, at which a goo silhouette begins. Below one half the
// shapes swell a little as they melt and the neck between two of them lasts longer,
// which is how a drop of liquid holds on before it parts.
const GOO_THRESHOLD: f32 = 0.36;

fn present(rect: vec4<f32>) -> bool {
    return rect.z > 0.0 && rect.w > 0.0;
}

// The smallest rectangle holding every present source.
fn union_rect(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    if !present(a) {
        return b;
    }
    if !present(b) {
        return a;
    }
    let low = min(a.xy, b.xy);
    let high = max(a.xy + a.zw, b.xy + b.zw);
    return vec4<f32>(low, high - low);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: MorphInstance) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );

    let bounds = union_rect(inst.rect_from, inst.rect_to);
    let pixel_pos = bounds.xy + corners[vi] * bounds.zw;
    let ndc = vec2<f32>(
        (pixel_pos.x / viewport.size.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / viewport.size.y) * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.pixel_pos = pixel_pos;
    out.rect_from = inst.rect_from;
    out.rect_to = inst.rect_to;
    out.timing = inst.timing;
    out.shape = inst.shape;
    out.noise = inst.noise;
    return out;
}

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}

// Smooth value noise in 0..1.
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// A source sampled over `rect`, which maps the whole picture onto the
// rectangle. Outside the rectangle it is transparent, with a one pixel
// antialiased edge. A `radius` of half a pixel or more blurs it: sixteen taps
// on a spiral, turned a different amount at every pixel so the pattern shows as
// fine grain instead of rings. Taps past the picture's edge repeat the edge, so
// a solid picture stays solid to its border while it blurs.
fn sample_over(
    tex: texture_2d<f32>,
    rect: vec4<f32>,
    source: vec4<f32>,
    p: vec2<f32>,
    radius: f32,
) -> vec4<f32> {
    if !present(source) {
        return vec4<f32>(0.0);
    }
    let edge = min(p - rect.xy, rect.xy + rect.zw - p);
    let coverage = clamp(edge.x + 0.5, 0.0, 1.0) * clamp(edge.y + 0.5, 0.0, 1.0);
    if radius < 0.5 {
        let uv = (p - rect.xy) / rect.zw;
        return textureSampleLevel(tex, s_morph, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0)
            * coverage;
    }
    let turn = hash21(p) * 6.2831853;
    var sum = vec4<f32>(0.0);
    for (var k = 0; k < 16; k = k + 1) {
        let fk = f32(k);
        let angle = fk * 2.3999632 + turn;
        let reach = radius * sqrt((fk + 0.5) / 16.0);
        let tap = p + vec2<f32>(cos(angle), sin(angle)) * reach;
        let uv = (tap - rect.xy) / rect.zw;
        sum = sum + textureSampleLevel(tex, s_morph, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    }
    return sum / 16.0 * coverage;
}

// A source's pixel at `p` for a silhouette average: transparent outside the
// rectangle, with no antialiasing, because the average does the softening.
fn tap_hard(tex: texture_2d<f32>, rect: vec4<f32>, p: vec2<f32>) -> vec4<f32> {
    if !present(rect) {
        return vec4<f32>(0.0);
    }
    let uv = (p - rect.xy) / rect.zw;
    if uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0 {
        return vec4<f32>(0.0);
    }
    return textureSampleLevel(tex, s_morph, uv, 0.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let style = u32(in.timing.x + 0.5);
    let t = clamp(in.timing.y, 0.0, 1.0);
    let opacity = in.timing.z;
    let seed = in.timing.w;
    let softness = max(in.shape.z, 0.001);
    let amplitude = in.shape.w;
    let frequency = max(in.noise.x, 0.001);
    let radius_from = in.noise.y * t;
    let radius_to = in.noise.y * (1.0 - t);

    let bounds = union_rect(in.rect_from, in.rect_to);
    let unit = max(min(bounds.z, bounds.w), 1.0);
    // Position inside the morph, 0..1 across each axis.
    let q = (in.pixel_pos - bounds.xy) / bounds.zw;
    // Noise coordinates: square cells, `frequency` of them across the shorter side.
    let np = (in.pixel_pos - bounds.xy) / unit * frequency + vec2<f32>(seed, seed * 1.7);

    var from_color: vec4<f32>;
    var to_color: vec4<f32>;
    var mix_amount: f32;

    if style == 0u {
        // Dissolve: each pixel switches when the sweep passes its noise value.
        from_color = sample_over(t_from, in.rect_from, in.rect_from, in.pixel_pos, radius_from);
        to_color = sample_over(t_to, in.rect_to, in.rect_to, in.pixel_pos, radius_to);
        let n = value_noise(np) * 0.75 + value_noise(np * 2.3 + 11.0) * 0.25;
        mix_amount = clamp((t * (1.0 + softness) - n) / softness, 0.0, 1.0);
    } else if style == 1u {
        // Wipe: the edge travels along the direction across the morph.
        from_color = sample_over(t_from, in.rect_from, in.rect_from, in.pixel_pos, radius_from);
        to_color = sample_over(t_to, in.rect_to, in.rect_to, in.pixel_pos, radius_to);
        var dir = in.shape.xy;
        if dot(dir, dir) < 0.000001 {
            dir = vec2<f32>(1.0, 0.0);
        }
        dir = normalize(dir);
        let extent = max(abs(dir.x) + abs(dir.y), 0.0001);
        let s = dot(q - 0.5, dir) / extent + 0.5;
        mix_amount = clamp((t * (1.0 + softness) - s) / softness, 0.0, 1.0);
    } else if style == 2u {
        // Liquid: both pictures are pushed along a flow field that is strongest
        // mid-way, while they cross-fade.
        let flow = vec2<f32>(
            value_noise(np) - 0.5,
            value_noise(np + vec2<f32>(37.0, 91.0)) - 0.5,
        ) * 2.0 * amplitude * unit;
        let p_from = in.pixel_pos + flow * t;
        let p_to = in.pixel_pos - flow * (1.0 - t);
        from_color = sample_over(t_from, in.rect_from, in.rect_from, p_from, radius_from);
        to_color = sample_over(t_to, in.rect_to, in.rect_to, p_to, radius_to);
        mix_amount = t * t * (3.0 - 2.0 * t);
    } else if style == 4u {
        // Blur: each picture stays in place; they only trade focus.
        from_color = sample_over(t_from, in.rect_from, in.rect_from, in.pixel_pos, radius_from);
        to_color = sample_over(t_to, in.rect_to, in.rect_to, in.pixel_pos, radius_to);
        mix_amount = t * t * (3.0 - 2.0 * t);
    } else if style == 5u {
        // Slide: both pictures travel along the direction by the morph's extent
        // in that direction, the outgoing one away, the incoming one in. The
        // incoming one is painted over the outgoing one; neither fades.
        var dir = in.shape.xy;
        if dot(dir, dir) < 0.000001 {
            dir = vec2<f32>(1.0, 0.0);
        }
        dir = normalize(dir);
        let travel = abs(dir.x) * bounds.z + abs(dir.y) * bounds.w;
        let slid_from = sample_over(t_from, in.rect_from, in.rect_from, in.pixel_pos - dir * travel * t, radius_from);
        let slid_to = sample_over(t_to, in.rect_to, in.rect_to, in.pixel_pos + dir * travel * (1.0 - t), radius_to);
        return (slid_to + slid_from * (1.0 - slid_to.a)) * opacity;
    } else if style == 6u {
        // Goo: blend the two pictures, average that over a disc as wide as
        // `blur * sin(pi * t)` (nothing at either end), and cut the silhouette
        // out of the average's alpha with a soft threshold. Shapes that were
        // apart bridge when the average between them rises over the threshold
        // and part again when it falls; shapes that appear bud out of nothing.
        // The color comes from the average too, so it blurs through the neck.
        let reach = in.noise.y * sin(3.14159265 * t);
        let amount = t * t * (3.0 - 2.0 * t);
        if reach < 0.5 {
            from_color = sample_over(t_from, in.rect_from, in.rect_from, in.pixel_pos, 0.0);
            to_color = sample_over(t_to, in.rect_to, in.rect_to, in.pixel_pos, 0.0);
            return mix(from_color, to_color, amount) * opacity;
        }
        // A fixed pattern, not the turned one a plain blur uses: a threshold turns
        // per-pixel noise into a ragged edge, while a fixed pattern leaves only a
        // smooth ripple that the soft threshold hides.
        let turn = 0.0;
        var sum = vec4<f32>(0.0);
        for (var k = 0; k < 64; k = k + 1) {
            let fk = f32(k);
            let angle = fk * 2.3999632 + turn;
            let ring = reach * sqrt((fk + 0.5) / 64.0);
            let tap = in.pixel_pos + vec2<f32>(cos(angle), sin(angle)) * ring;
            sum = sum + mix(tap_hard(t_from, in.rect_from, tap), tap_hard(t_to, in.rect_to, tap), amount);
        }
        let average = sum / 64.0;
        let cover = smoothstep(GOO_THRESHOLD - softness * 0.5, GOO_THRESHOLD + softness * 0.5, average.a);
        let straight = average.rgb / max(average.a, 0.0001);
        return vec4<f32>(straight * cover, cover) * opacity;
    } else {
        // Bounds: both pictures are stretched into the box that travels from
        // the outgoing rectangle to the incoming one.
        var moving = mix(in.rect_from, in.rect_to, t);
        if !present(in.rect_from) {
            moving = in.rect_to;
        } else if !present(in.rect_to) {
            moving = in.rect_from;
        }
        from_color = sample_over(t_from, moving, in.rect_from, in.pixel_pos, radius_from);
        to_color = sample_over(t_to, moving, in.rect_to, in.pixel_pos, radius_to);
        mix_amount = t * t * (3.0 - 2.0 * t);
    }

    return mix(from_color, to_color, mix_amount) * opacity;
}
