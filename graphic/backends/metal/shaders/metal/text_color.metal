struct TextColorVertexIn {
    float2 pos [[attribute(0)]];
    float2 size [[attribute(1)]];
    float4 uv_rect [[attribute(2)]];
    float4 color [[attribute(3)]];
    float4 clip_rect [[attribute(4)]];
    float4 clip_radius [[attribute(5)]];
    float skew [[attribute(6)]];
};

struct TextColorVaryings {
    float4 position [[position]];
    float2 uv [[user(loc0)]];
    float4 color [[user(loc1)]];
    float2 pixel_pos [[user(loc2)]];
    float4 clip_rect [[user(loc3)]];
    float4 clip_border_radius [[user(loc4)]];
};

struct TextColorFragmentIn {
    float2 uv [[user(loc0)]];
    float4 color [[user(loc1)]];
    float2 pixel_pos [[user(loc2)]];
    float4 clip_rect [[user(loc3)]];
    float4 clip_border_radius [[user(loc4)]];
};

vertex TextColorVaryings vs_main(uint vertex_id [[vertex_id]],
                                 TextColorVertexIn inst [[stage_in]],
                                 constant TextViewport& viewport [[buffer(0)]]) {
    float2 corner = kTextCorners[vertex_id];
    float2 pixel_pos = inst.pos + corner * inst.size;
    pixel_pos.x += inst.skew * inst.size.y * (1.0 - corner.y);
    float2 ndc = float2(pixel_pos.x / viewport.resolution.x * 2.0 - 1.0,
                        -(pixel_pos.y / viewport.resolution.y * 2.0 - 1.0));

    TextColorVaryings out;
    out.position = float4(ndc, 0.0, 1.0);
    out.uv = mix(inst.uv_rect.xy, inst.uv_rect.zw, corner);
    out.color = inst.color;
    out.pixel_pos = pixel_pos;
    out.clip_rect = inst.clip_rect;
    out.clip_border_radius = inst.clip_radius;
    return out;
}

fragment float4 fs_main(TextColorFragmentIn varyings [[stage_in]],
                        constant TextViewport& viewport [[buffer(0)]],
                        texture2d<float> atlas_texture [[texture(1)]],
                        sampler atlas_sampler [[sampler(2)]]) {
    float4 sample = atlas_texture.sample(atlas_sampler, varyings.uv,
                                         level(0.0));
    float clip = text_clip_alpha(varyings.pixel_pos, varyings.clip_rect,
                                 varyings.clip_border_radius);
    float alpha = sample.a * varyings.color.a * clip;
    float3 rgb = sample.rgb;
    if (viewport.surface_is_srgb < 1.5) {
        float3 linear = float3(text_srgb_to_linear(rgb.r),
                               text_srgb_to_linear(rgb.g),
                               text_srgb_to_linear(rgb.b));
        rgb = viewport.surface_is_srgb < 0.5
            ? float3(text_linear_to_srgb(linear.r),
                     text_linear_to_srgb(linear.g),
                     text_linear_to_srgb(linear.b))
            : linear;
    }
    return float4(rgb * alpha, alpha);
}
