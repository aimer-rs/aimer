#include <metal_stdlib>

using namespace metal;

struct TextViewport {
    float2 resolution;
    float surface_is_srgb;
    float _pad;
};

constant float2 kTextCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(0.0, 1.0), float2(1.0, 0.0), float2(1.0, 1.0),
};

static float text_sdf_rounded_rect(float2 p, float2 half_size, float4 radii) {
    float radius;
    if (p.x < 0.0) {
        radius = p.y < 0.0 ? radii.x : radii.w;
    } else {
        radius = p.y < 0.0 ? radii.y : radii.z;
    }
    radius = min(radius, min(half_size.x, half_size.y));
    float2 q = abs(p) - half_size + float2(radius, radius);
    return length(max(q, float2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

static float text_clip_alpha(float2 pixel_pos, float4 clip_rect, float4 radii) {
    if (clip_rect.z < 0.0) {
        return 1.0;
    }
    if (clip_rect.z <= 0.01 || clip_rect.w <= 0.01) {
        return 0.0;
    }
    if (any(radii > 0.0)) {
        float2 center = clip_rect.xy + clip_rect.zw * 0.5;
        float distance = text_sdf_rounded_rect(pixel_pos - center,
                                               clip_rect.zw * 0.5, radii);
        return 1.0 - smoothstep(-0.5, 0.5, distance);
    }
    float2 clip_min = clip_rect.xy;
    float2 clip_max = clip_rect.xy + clip_rect.zw;
    float left = smoothstep(-0.5, 0.5, pixel_pos.x - clip_min.x);
    float right = smoothstep(-0.5, 0.5, clip_max.x - pixel_pos.x);
    float top = smoothstep(-0.5, 0.5, pixel_pos.y - clip_min.y);
    float bottom = smoothstep(-0.5, 0.5, clip_max.y - pixel_pos.y);
    return left * right * top * bottom;
}

static float text_srgb_to_linear(float value) {
    if (value <= 0.04045) {
        return value / 12.92;
    }
    return pow((value + 0.055) / 1.055, 2.4);
}

static float text_linear_to_srgb(float value) {
    if (value <= 0.0031308) {
        return value * 12.92;
    }
    return 1.055 * pow(value, 1.0 / 2.4) - 0.055;
}
