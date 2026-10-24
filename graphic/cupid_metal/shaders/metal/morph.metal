#include <metal_stdlib>

using namespace metal;

// Blends two offscreen pictures with a transition effect. See wgsl/morph.wgsl,
// which this file mirrors line for line.
struct Viewport {
    float2 size;
    float surface_is_srgb;
    float _pad;
};

constant float2 kMorphCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

struct MorphVertexIn {
    float4 rect_from [[attribute(0)]];
    float4 rect_to [[attribute(1)]];
    float4 timing [[attribute(2)]];
    float4 shape [[attribute(3)]];
    float4 noise [[attribute(4)]];
};

struct MorphVaryings {
    float4 position [[position]];
    float2 pixel_pos [[user(loc0)]];
    float4 rect_from [[user(loc1)]];
    float4 rect_to [[user(loc2)]];
    float4 timing [[user(loc3)]];
    float4 shape [[user(loc4)]];
    float4 noise [[user(loc5)]];
};

struct MorphFragmentIn {
    float2 pixel_pos [[user(loc0)]];
    float4 rect_from [[user(loc1)]];
    float4 rect_to [[user(loc2)]];
    float4 timing [[user(loc3)]];
    float4 shape [[user(loc4)]];
    float4 noise [[user(loc5)]];
};

// The alpha, after averaging, at which a goo silhouette begins. Below one half the
// shapes swell a little as they melt and the neck between two of them lasts longer,
// which is how a drop of liquid holds on before it parts.
constant float GOO_THRESHOLD = 0.36;

static bool morph_present(float4 rect) {
    return rect.z > 0.0 && rect.w > 0.0;
}

static float4 morph_union_rect(float4 a, float4 b) {
    if (!morph_present(a)) {
        return b;
    }
    if (!morph_present(b)) {
        return a;
    }
    float2 low = min(a.xy, b.xy);
    float2 high = max(a.xy + a.zw, b.xy + b.zw);
    return float4(low, high - low);
}

vertex MorphVaryings vs_main(uint vertex_id [[vertex_id]],
                             MorphVertexIn inst [[stage_in]],
                             constant Viewport& viewport [[buffer(0)]]) {
    float4 bounds = morph_union_rect(inst.rect_from, inst.rect_to);
    float2 pixel_pos = bounds.xy + kMorphCorners[vertex_id] * bounds.zw;
    float2 ndc = float2(pixel_pos.x / viewport.size.x * 2.0 - 1.0,
                        1.0 - pixel_pos.y / viewport.size.y * 2.0);

    MorphVaryings out;
    out.position = float4(ndc, 0.0, 1.0);
    out.pixel_pos = pixel_pos;
    out.rect_from = inst.rect_from;
    out.rect_to = inst.rect_to;
    out.timing = inst.timing;
    out.shape = inst.shape;
    out.noise = inst.noise;
    return out;
}

static float morph_hash21(float2 p) {
    float2 q = fract(p * float2(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}

static float morph_value_noise(float2 p) {
    float2 i = floor(p);
    float2 f = fract(p);
    float2 u = f * f * (3.0 - 2.0 * f);
    float a = morph_hash21(i);
    float b = morph_hash21(i + float2(1.0, 0.0));
    float c = morph_hash21(i + float2(0.0, 1.0));
    float d = morph_hash21(i + float2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

static float4 morph_sample_over(texture2d<float> tex, sampler smp, float4 rect,
                                float4 source, float2 p, float radius) {
    if (!morph_present(source)) {
        return float4(0.0);
    }
    float2 edge = min(p - rect.xy, rect.xy + rect.zw - p);
    float coverage = clamp(edge.x + 0.5, 0.0, 1.0) * clamp(edge.y + 0.5, 0.0, 1.0);
    if (radius < 0.5) {
        float2 uv = (p - rect.xy) / rect.zw;
        return tex.sample(smp, clamp(uv, float2(0.0), float2(1.0)), level(0.0)) * coverage;
    }
    float turn = morph_hash21(p) * 6.2831853;
    float4 sum = float4(0.0);
    for (int k = 0; k < 16; k = k + 1) {
        float fk = float(k);
        float angle = fk * 2.3999632 + turn;
        float reach = radius * sqrt((fk + 0.5) / 16.0);
        float2 tap = p + float2(cos(angle), sin(angle)) * reach;
        float2 uv = (tap - rect.xy) / rect.zw;
        sum = sum + tex.sample(smp, clamp(uv, float2(0.0), float2(1.0)), level(0.0));
    }
    return sum / 16.0 * coverage;
}

static float4 morph_tap_hard(texture2d<float> tex, sampler smp, float4 rect, float2 p) {
    if (!morph_present(rect)) {
        return float4(0.0);
    }
    float2 uv = (p - rect.xy) / rect.zw;
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) {
        return float4(0.0);
    }
    return tex.sample(smp, uv, level(0.0));
}

fragment float4 fs_main(MorphFragmentIn in [[stage_in]],
                        texture2d<float> t_from [[texture(4)]],
                        texture2d<float> t_to [[texture(5)]],
                        sampler s_morph [[sampler(6)]]) {
    uint style = uint(in.timing.x + 0.5);
    float t = clamp(in.timing.y, 0.0, 1.0);
    float opacity = in.timing.z;
    float seed = in.timing.w;
    float softness = max(in.shape.z, 0.001);
    float amplitude = in.shape.w;
    float frequency = max(in.noise.x, 0.001);
    float radius_from = in.noise.y * t;
    float radius_to = in.noise.y * (1.0 - t);

    float4 bounds = morph_union_rect(in.rect_from, in.rect_to);
    float unit = max(min(bounds.z, bounds.w), 1.0);
    float2 q = (in.pixel_pos - bounds.xy) / bounds.zw;
    float2 np = (in.pixel_pos - bounds.xy) / unit * frequency + float2(seed, seed * 1.7);

    float4 from_color;
    float4 to_color;
    float mix_amount;

    if (style == 0u) {
        from_color = morph_sample_over(t_from, s_morph, in.rect_from, in.rect_from, in.pixel_pos, radius_from);
        to_color = morph_sample_over(t_to, s_morph, in.rect_to, in.rect_to, in.pixel_pos, radius_to);
        float n = morph_value_noise(np) * 0.75 + morph_value_noise(np * 2.3 + 11.0) * 0.25;
        mix_amount = clamp((t * (1.0 + softness) - n) / softness, 0.0, 1.0);
    } else if (style == 1u) {
        from_color = morph_sample_over(t_from, s_morph, in.rect_from, in.rect_from, in.pixel_pos, radius_from);
        to_color = morph_sample_over(t_to, s_morph, in.rect_to, in.rect_to, in.pixel_pos, radius_to);
        float2 dir = in.shape.xy;
        if (dot(dir, dir) < 0.000001) {
            dir = float2(1.0, 0.0);
        }
        dir = normalize(dir);
        float extent = max(abs(dir.x) + abs(dir.y), 0.0001);
        float s = dot(q - 0.5, dir) / extent + 0.5;
        mix_amount = clamp((t * (1.0 + softness) - s) / softness, 0.0, 1.0);
    } else if (style == 2u) {
        float2 flow = float2(morph_value_noise(np) - 0.5,
                             morph_value_noise(np + float2(37.0, 91.0)) - 0.5)
                      * 2.0 * amplitude * unit;
        float2 p_from = in.pixel_pos + flow * t;
        float2 p_to = in.pixel_pos - flow * (1.0 - t);
        from_color = morph_sample_over(t_from, s_morph, in.rect_from, in.rect_from, p_from, radius_from);
        to_color = morph_sample_over(t_to, s_morph, in.rect_to, in.rect_to, p_to, radius_to);
        mix_amount = t * t * (3.0 - 2.0 * t);
    } else if (style == 4u) {
        from_color = morph_sample_over(t_from, s_morph, in.rect_from, in.rect_from, in.pixel_pos, radius_from);
        to_color = morph_sample_over(t_to, s_morph, in.rect_to, in.rect_to, in.pixel_pos, radius_to);
        mix_amount = t * t * (3.0 - 2.0 * t);
    } else if (style == 5u) {
        float2 dir = in.shape.xy;
        if (dot(dir, dir) < 0.000001) {
            dir = float2(1.0, 0.0);
        }
        dir = normalize(dir);
        float travel = abs(dir.x) * bounds.z + abs(dir.y) * bounds.w;
        float4 slid_from = morph_sample_over(t_from, s_morph, in.rect_from, in.rect_from, in.pixel_pos - dir * travel * t, radius_from);
        float4 slid_to = morph_sample_over(t_to, s_morph, in.rect_to, in.rect_to, in.pixel_pos + dir * travel * (1.0 - t), radius_to);
        return (slid_to + slid_from * (1.0 - slid_to.a)) * opacity;
    } else if (style == 6u) {
        float reach = in.noise.y * sin(3.14159265 * t);
        float amount = t * t * (3.0 - 2.0 * t);
        if (reach < 0.5) {
            from_color = morph_sample_over(t_from, s_morph, in.rect_from, in.rect_from, in.pixel_pos, 0.0);
            to_color = morph_sample_over(t_to, s_morph, in.rect_to, in.rect_to, in.pixel_pos, 0.0);
            return mix(from_color, to_color, amount) * opacity;
        }
        float turn = 0.0;
        float4 sum = float4(0.0);
        for (int k = 0; k < 64; k = k + 1) {
            float fk = float(k);
            float angle = fk * 2.3999632 + turn;
            float ring = reach * sqrt((fk + 0.5) / 64.0);
            float2 tap = in.pixel_pos + float2(cos(angle), sin(angle)) * ring;
            sum = sum + mix(morph_tap_hard(t_from, s_morph, in.rect_from, tap),
                            morph_tap_hard(t_to, s_morph, in.rect_to, tap), amount);
        }
        float4 average = sum / 64.0;
        float cover = smoothstep(GOO_THRESHOLD - softness * 0.5, GOO_THRESHOLD + softness * 0.5, average.a);
        float3 straight = average.rgb / max(average.a, 0.0001);
        return float4(straight * cover, cover) * opacity;
    } else {
        float4 moving = mix(in.rect_from, in.rect_to, t);
        if (!morph_present(in.rect_from)) {
            moving = in.rect_to;
        } else if (!morph_present(in.rect_to)) {
            moving = in.rect_from;
        }
        from_color = morph_sample_over(t_from, s_morph, moving, in.rect_from, in.pixel_pos, radius_from);
        to_color = morph_sample_over(t_to, s_morph, moving, in.rect_to, in.pixel_pos, radius_to);
        mix_amount = t * t * (3.0 - 2.0 * t);
    }

    return mix(from_color, to_color, mix_amount) * opacity;
}
