struct DecorationVertexIn {
    float2 pos [[attribute(0)]];
    float2 size [[attribute(1)]];
    float4 color [[attribute(2)]];
    float4 clip_rect [[attribute(3)]];
    float4 clip_radius [[attribute(4)]];
    float4 params [[attribute(5)]];
};

struct DecorationVaryings {
    float4 position [[position]];
    float2 uv [[user(loc0)]];
    float4 color [[user(loc1)]];
    float2 pixel_pos [[user(loc2)]];
    float4 clip_rect [[user(loc3)]];
    float4 clip_border_radius [[user(loc4)]];
    float4 params [[user(loc5)]];
    float2 size [[user(loc6)]];
};

struct DecorationFragmentIn {
    float2 uv [[user(loc0)]];
    float4 color [[user(loc1)]];
    float2 pixel_pos [[user(loc2)]];
    float4 clip_rect [[user(loc3)]];
    float4 clip_border_radius [[user(loc4)]];
    float4 params [[user(loc5)]];
    float2 size [[user(loc6)]];
};

vertex DecorationVaryings vs_main(uint vertex_id [[vertex_id]],
                                  DecorationVertexIn inst [[stage_in]],
                                  constant TextViewport& viewport [[buffer(0)]]) {
    float2 corner = kTextCorners[vertex_id];
    float2 pixel_pos = inst.pos + corner * inst.size;
    float2 ndc = float2(pixel_pos.x / viewport.resolution.x * 2.0 - 1.0,
                        -(pixel_pos.y / viewport.resolution.y * 2.0 - 1.0));

    DecorationVaryings out;
    out.position = float4(ndc, 0.0, 1.0);
    out.uv = corner;
    out.color = inst.color;
    out.pixel_pos = pixel_pos;
    out.clip_rect = inst.clip_rect;
    out.clip_border_radius = inst.clip_radius;
    out.params = inst.params;
    out.size = inst.size;
    return out;
}

static float stroke_coverage(float distance, float half_thickness) {
    return 1.0 - smoothstep(-0.5, 0.5, distance - half_thickness);
}

fragment float4 fs_main(DecorationFragmentIn varyings [[stage_in]],
                        constant TextViewport& viewport [[buffer(0)]]) {
    float style = varyings.params.x;
    float thickness = max(varyings.params.y, 0.5);
    float period = max(varyings.params.z, 1.0);
    float band = max(varyings.params.w, thickness);
    float px = varyings.uv.x * varyings.size.x;
    float py = varyings.uv.y * varyings.size.y;
    float center = band * 0.5;
    float half_thickness = thickness * 0.5;
    float coverage = 0.0;

    if (style < 0.5) {
        coverage = stroke_coverage(abs(py - center), half_thickness);
    } else if (style < 1.5) {
        float c0 = half_thickness;
        float c1 = band - half_thickness;
        coverage = max(stroke_coverage(abs(py - c0), half_thickness),
                       stroke_coverage(abs(py - c1), half_thickness));
    } else if (style < 2.5) {
        float line = stroke_coverage(abs(py - center), half_thickness);
        float phase = px - period * floor(px / period);
        float dot = stroke_coverage(abs(phase - period * 0.5), period * 0.25);
        coverage = line * dot;
    } else if (style < 3.5) {
        float line = stroke_coverage(abs(py - center), half_thickness);
        float phase = px - period * floor(px / period);
        float dash = step(phase, period * 0.6);
        coverage = line * dash;
    } else {
        float amplitude = max((band - thickness) * 0.5, 0.0);
        float wave = center + amplitude * sin(px * 6.2831853 / period);
        coverage = stroke_coverage(abs(py - wave), half_thickness);
    }

    float clip = text_clip_alpha(varyings.pixel_pos, varyings.clip_rect,
                                 varyings.clip_border_radius);
    float alpha = varyings.color.a * coverage * clip;
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
