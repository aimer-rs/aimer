struct Viewport {
    float2 size;
    float surface_is_srgb;
    float _pad;
};

constant float2 kImageCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

struct ImageVertexIn {
    float2 pos [[attribute(0)]];
    float2 size [[attribute(1)]];
    float2 uv_offset [[attribute(2)]];
    float2 uv_scale [[attribute(3)]];
    float4 clip_rect [[attribute(4)]];
    float4 clip_border_radius [[attribute(5)]];
    float alpha [[attribute(6)]];
    float source_premultiplied [[attribute(7)]];
};

struct ImageVaryings {
    float4 position [[position]];
    float2 uv [[user(loc0)]];
    float2 pixel_pos [[user(loc1)]];
    float4 clip_rect [[user(loc2)]];
    float4 clip_border_radius [[user(loc3)]];
    float alpha [[user(loc4)]];
    float source_premultiplied [[user(loc5)]];
};

struct ImageFragmentIn {
    float2 uv [[user(loc0)]];
    float2 pixel_pos [[user(loc1)]];
    float4 clip_rect [[user(loc2)]];
    float4 clip_border_radius [[user(loc3)]];
    float alpha [[user(loc4)]];
    float source_premultiplied [[user(loc5)]];
};

vertex ImageVaryings vs_main(uint vertex_id [[vertex_id]],
                             ImageVertexIn inst [[stage_in]],
                             constant Viewport& viewport [[buffer(0)]]) {
    float2 corner = kImageCorners[vertex_id];
    float2 pixel_pos = inst.pos + corner * inst.size;
    float2 ndc = float2(pixel_pos.x / viewport.size.x * 2.0 - 1.0,
                        1.0 - pixel_pos.y / viewport.size.y * 2.0);

    ImageVaryings out;
    out.position = float4(ndc, 0.0, 1.0);
    out.uv = inst.uv_offset + corner * inst.uv_scale;
    out.pixel_pos = pixel_pos;
    out.clip_rect = inst.clip_rect;
    out.clip_border_radius = inst.clip_border_radius;
    out.alpha = inst.alpha;
    out.source_premultiplied = inst.source_premultiplied;
    return out;
}

static float sdf_rounded_rect(float2 p, float2 half_size, float4 radii) {
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

static float clip_alpha(float2 pixel_pos, float4 clip_rect, float4 clip_radii) {
    if (clip_rect.z < 0.0) {
        return 1.0;
    }
    if (clip_rect.z <= 0.01 || clip_rect.w <= 0.01) {
        return 0.0;
    }
    if (any(clip_radii > 0.0)) {
        float2 clip_center = clip_rect.xy + clip_rect.zw * 0.5;
        float2 clip_half = clip_rect.zw * 0.5;
        float distance = sdf_rounded_rect(pixel_pos - clip_center, clip_half, clip_radii);
        return 1.0 - smoothstep(-0.5, 0.5, distance);
    }

    float2 clip_min = clip_rect.xy;
    float2 clip_max = clip_rect.xy + clip_rect.zw;
    float a_left = smoothstep(-0.5, 0.5, pixel_pos.x - clip_min.x);
    float a_right = smoothstep(-0.5, 0.5, clip_max.x - pixel_pos.x);
    float a_top = smoothstep(-0.5, 0.5, pixel_pos.y - clip_min.y);
    float a_bottom = smoothstep(-0.5, 0.5, clip_max.y - pixel_pos.y);
    return a_left * a_right * a_top * a_bottom;
}

fragment float4 fs_main(ImageFragmentIn varyings [[stage_in]],
                        constant Viewport& viewport [[buffer(0)]],
                        texture2d<float> t_diffuse [[texture(4)]],
                        sampler s_diffuse [[sampler(5)]]) {
    float4 color = t_diffuse.sample(s_diffuse, varyings.uv);
    float4 result;
    if (varyings.source_premultiplied > 0.5) {
        float alpha = color_offset(color.a);
        if (viewport.surface_is_srgb >= 1.5) {
            result = float4(color.rgb, alpha);
        } else if (viewport.surface_is_srgb < 0.5) {
            if (alpha > 0.00001) {
                float3 straight = color.rgb / alpha;
                result = float4(float3(
                    linear_to_srgb(srgb_to_linear(straight.r)),
                    linear_to_srgb(srgb_to_linear(straight.g)),
                    linear_to_srgb(srgb_to_linear(straight.b))) * alpha, alpha);
            } else {
                result = float4(0.0);
            }
        } else {
            result = float4(color.rgb, alpha);
        }
    } else if (viewport.surface_is_srgb >= 1.5) {
        float alpha = color_offset(color.a);
        result = float4(color.rgb * alpha, alpha);
    } else {
        color = float4(srgb_to_linear(color.r), srgb_to_linear(color.g),
                       srgb_to_linear(color.b), color_offset(color.a));
        result = float4(color.rgb * color.a, color.a);
        if (viewport.surface_is_srgb < 0.5) {
            float alpha = color.a;
            if (alpha > 0.00001) {
                float3 unpremul = result.rgb / alpha;
                float3 srgb_rgb = float3(linear_to_srgb(unpremul.r),
                                         linear_to_srgb(unpremul.g),
                                         linear_to_srgb(unpremul.b));
                result = float4(srgb_rgb * alpha, alpha);
            }
        }
    }

    float clip = clip_alpha(varyings.pixel_pos, varyings.clip_rect,
                            varyings.clip_border_radius);
    return result * (clip * varyings.alpha);
}
