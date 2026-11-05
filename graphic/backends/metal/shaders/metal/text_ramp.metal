// Text shader that also scales each glyph's alpha by a linear ramp: a fade over
// text without an offscreen pass. Generated from the same logic as `text.metal`;
// keep the two in step.
struct TextVertexIn {
    float2 pos [[attribute(0)]];
    float2 size [[attribute(1)]];
    float4 uv_rect [[attribute(2)]];
    float4 color [[attribute(3)]];
    float4 clip_rect [[attribute(4)]];
    float4 clip_radius [[attribute(5)]];
    float skew [[attribute(6)]];
    float coverage_exponent [[attribute(7)]];
    // The ramp this glyph's alpha is scaled by; see `AlphaRamp` in
    // `aimer_text_pipeline`.
    float4 ramp_line [[attribute(8)]];
    float4 ramp_rect [[attribute(9)]];
    float4 ramp_offsets [[attribute(10)]];
    float4 ramp_alphas [[attribute(11)]];
};

struct TextVaryings {
    float4 position [[position]];
    float2 uv [[user(loc0)]];
    float4 color [[user(loc1)]];
    float2 pixel_pos [[user(loc2)]];
    float4 clip_rect [[user(loc3)]];
    float4 clip_border_radius [[user(loc4)]];
    float coverage_exponent [[user(loc5)]];
    float4 ramp_line [[user(loc6)]];
    float4 ramp_rect [[user(loc7)]];
    float4 ramp_offsets [[user(loc8)]];
    float4 ramp_alphas [[user(loc9)]];
};

struct TextFragmentIn {
    float2 uv [[user(loc0)]];
    float4 color [[user(loc1)]];
    float2 pixel_pos [[user(loc2)]];
    float4 clip_rect [[user(loc3)]];
    float4 clip_border_radius [[user(loc4)]];
    float coverage_exponent [[user(loc5)]];
    float4 ramp_line [[user(loc6)]];
    float4 ramp_rect [[user(loc7)]];
    float4 ramp_offsets [[user(loc8)]];
    float4 ramp_alphas [[user(loc9)]];
};

// The alpha of a linear ramp at `p`, in target pixels: the stops' alpha held
// before the first and after the last, and nothing outside the rectangle it is
// defined on. `AlphaRamp::alpha_at` is this, in Rust.
static float text_ramp_alpha(float2 p, float4 line, float4 rect, float4 offsets,
                             float4 alphas) {
    if (p.x < rect.x || p.y < rect.y || p.x >= rect.x + rect.z ||
        p.y >= rect.y + rect.w) {
        return 0.0;
    }
    float2 axis = line.zw - line.xy;
    float t = clamp(dot(p - line.xy, axis) / max(dot(axis, axis), 0.000001), 0.0, 1.0);
    float along = alphas.w;
    if (t <= offsets.x) {
        along = alphas.x;
    } else if (t <= offsets.y) {
        along = mix(alphas.x, alphas.y,
                    clamp((t - offsets.x) / max(offsets.y - offsets.x, 0.000001), 0.0, 1.0));
    } else if (t <= offsets.z) {
        along = mix(alphas.y, alphas.z,
                    clamp((t - offsets.y) / max(offsets.z - offsets.y, 0.000001), 0.0, 1.0));
    } else if (t <= offsets.w) {
        along = mix(alphas.z, alphas.w,
                    clamp((t - offsets.z) / max(offsets.w - offsets.z, 0.000001), 0.0, 1.0));
    }
    // The pixels at the rectangle's edge are softened as the edge of the quad a
    // ramp is drawn on is, so a fade looks the same drawn either way.
    float2 half_size = rect.zw * 0.5;
    float2 q = abs(p - (rect.xy + half_size)) - half_size;
    float distance = length(max(q, float2(0.0))) + min(max(q.x, q.y), 0.0);
    float edge = 1.0 - smoothstep(-1.0, 1.0, distance);
    return along * edge;
}

vertex TextVaryings vs_main(uint vertex_id [[vertex_id]],
                            TextVertexIn inst [[stage_in]],
                            constant TextViewport& viewport [[buffer(0)]]) {
    float2 corner = kTextCorners[vertex_id];
    float2 pixel_pos = inst.pos + corner * inst.size;
    pixel_pos.x += inst.skew * inst.size.y * (1.0 - corner.y);
    float2 ndc = float2(pixel_pos.x / viewport.resolution.x * 2.0 - 1.0,
                        -(pixel_pos.y / viewport.resolution.y * 2.0 - 1.0));

    TextVaryings out;
    out.position = float4(ndc, 0.0, 1.0);
    out.uv = mix(inst.uv_rect.xy, inst.uv_rect.zw, corner);
    out.color = inst.color;
    out.pixel_pos = pixel_pos;
    out.clip_rect = inst.clip_rect;
    out.clip_border_radius = inst.clip_radius;
    out.coverage_exponent = inst.coverage_exponent;
    out.ramp_line = inst.ramp_line;
    out.ramp_rect = inst.ramp_rect;
    out.ramp_offsets = inst.ramp_offsets;
    out.ramp_alphas = inst.ramp_alphas;
    return out;
}

fragment float4 fs_main(TextFragmentIn varyings [[stage_in]],
                        constant TextViewport& viewport [[buffer(0)]],
                        texture2d<float> atlas_texture [[texture(1)]],
                        sampler atlas_sampler [[sampler(2)]]) {
    float coverage = atlas_texture.sample(atlas_sampler, varyings.uv,
                                          level(0.0)).r;
    float clip = text_clip_alpha(varyings.pixel_pos, varyings.clip_rect,
                                 varyings.clip_border_radius);
    float corrected = viewport.surface_is_srgb < 0.5
        ? coverage
        : pow(coverage, varyings.coverage_exponent);
    float ramp = text_ramp_alpha(varyings.pixel_pos, varyings.ramp_line,
                                 varyings.ramp_rect, varyings.ramp_offsets,
                                 varyings.ramp_alphas);
    float alpha = varyings.color.a * corrected * clip * ramp;

    float4 result;
    if (viewport.surface_is_srgb >= 1.5) {
        result = float4(varyings.color.rgb * alpha, alpha);
    } else {
        float3 linear = float3(text_srgb_to_linear(varyings.color.r),
                               text_srgb_to_linear(varyings.color.g),
                               text_srgb_to_linear(varyings.color.b));
        result = float4(linear * alpha, alpha);
        if (viewport.surface_is_srgb < 0.5 && alpha > 0.00001) {
            result = float4(float3(text_linear_to_srgb(linear.r),
                                   text_linear_to_srgb(linear.g),
                                   text_linear_to_srgb(linear.b)) * alpha, alpha);
        }
    }
    return result;
}
