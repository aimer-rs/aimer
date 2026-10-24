struct SvgVertexIn {
    float2 position [[attribute(0)]];
    float4 transform_x [[attribute(1)]];
    float4 transform_y [[attribute(2)]];
    float4 color [[attribute(3)]];
    float4 clip_rect [[attribute(4)]];
    float4 clip_border_radius [[attribute(5)]];
    float4 viewport [[attribute(6)]];
    float coverage [[attribute(7)]];
};

struct SvgVaryings {
    float4 position [[position]];
    float4 color [[user(loc0)]];
    float2 pixel_pos [[user(loc1)]];
    float4 clip_rect [[user(loc2)]];
    float4 clip_border_radius [[user(loc3)]];
    float surface_is_srgb [[user(loc4)]];
    float coverage [[user(loc5)]];
};

struct SvgFragmentIn {
    float4 color [[user(loc0)]];
    float2 pixel_pos [[user(loc1)]];
    float4 clip_rect [[user(loc2)]];
    float4 clip_border_radius [[user(loc3)]];
    float surface_is_srgb [[user(loc4)]];
    float coverage [[user(loc5)]];
};

vertex SvgVaryings vs_main(SvgVertexIn attributes [[stage_in]]) {
    float3 local = float3(attributes.position, 1.0);
    float2 pixel_pos = float2(dot(local, attributes.transform_x.xyz),
                              dot(local, attributes.transform_y.xyz));
    float2 ndc = float2(pixel_pos.x / attributes.viewport.x * 2.0 - 1.0,
                        1.0 - pixel_pos.y / attributes.viewport.y * 2.0);

    SvgVaryings out;
    out.position = float4(ndc, 0.0, 1.0);
    out.color = attributes.color;
    out.pixel_pos = pixel_pos;
    out.clip_rect = attributes.clip_rect;
    out.clip_border_radius = attributes.clip_border_radius;
    out.surface_is_srgb = attributes.viewport.z;
    out.coverage = attributes.coverage;
    return out;
}

static float selected_radius(float2 point, float4 radii) {
    if (point.x < 0.0) {
        return point.y < 0.0 ? radii.x : radii.w;
    }
    return point.y < 0.0 ? radii.y : radii.z;
}

static float sdf_rounded_rect(float2 point, float2 half_size, float4 radii) {
    float radius = min(selected_radius(point, radii),
                       min(half_size.x, half_size.y));
    float2 q = abs(point) - half_size + float2(radius);
    return length(max(q, float2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

static float clip_alpha(float2 pixel_pos, float4 clip_rect, float4 radii) {
    if (clip_rect.z <= 0.0) {
        return 1.0;
    }
    float2 half_size = clip_rect.zw * 0.5;
    float2 center = clip_rect.xy + half_size;
    float distance = sdf_rounded_rect(pixel_pos - center, half_size, radii);
    return 1.0 - smoothstep(-0.5, 0.5, distance);
}

fragment float4 fs_main(SvgFragmentIn varyings [[stage_in]]) {
    float clip = clip_alpha(varyings.pixel_pos, varyings.clip_rect,
                            varyings.clip_border_radius);
    float4 color = varyings.color;
    if (varyings.surface_is_srgb < 1.5) {
        color = float4(srgb_to_linear(color.r), srgb_to_linear(color.g),
                       srgb_to_linear(color.b), color.a);
    }
    float4 result = float4(color.rgb * color.a, color.a) * clip * varyings.coverage;
    if (varyings.surface_is_srgb < 0.5 && result.a > 0.000001) {
        float3 unpremultiplied = result.rgb / result.a;
        result = float4(float3(linear_to_srgb(unpremultiplied.r),
                               linear_to_srgb(unpremultiplied.g),
                               linear_to_srgb(unpremultiplied.b)) * result.a,
                        result.a);
    }
    return result;
}
