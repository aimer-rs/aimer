// Hand-authored HLSL for the native Direct3D 12 rectangle pipeline.
struct Viewport {
    float2 size;
    float surface_is_srgb;
    float _pad;
};
ConstantBuffer<Viewport> viewport : register(b0, space0);

struct RectVertexIn {
    float2 pos : TEXCOORD0;
    float2 size : TEXCOORD1;
    float4 color : TEXCOORD2;
    float4 border_radius : TEXCOORD3;
    float4 border_width : TEXCOORD4;
    float4 border_color : TEXCOORD5;
    float4 outline_width : TEXCOORD6;
    float4 outline_color : TEXCOORD7;
    float4 clip_rect : TEXCOORD8;
    float4 clip_border_radius : TEXCOORD9;
    float4 shadow_params : TEXCOORD10;
    float4 shadow_color : TEXCOORD11;
    float4 shadow_flags : TEXCOORD12;
};

struct RectVaryings {
    float4 position : SV_Position;
    float4 color : TEXCOORD0;
    float2 local_pos : TEXCOORD1;
    float2 rect_size : TEXCOORD2;
    float4 border_radius : TEXCOORD3;
    float4 border_width : TEXCOORD4;
    float4 border_color : TEXCOORD5;
    float4 outline_width : TEXCOORD6;
    float4 outline_color : TEXCOORD7;
    float2 pixel_pos : TEXCOORD8;
    float4 clip_rect : TEXCOORD9;
    float4 clip_border_radius : TEXCOORD10;
    float4 shadow_params : TEXCOORD11;
    float4 shadow_color : TEXCOORD12;
    float4 shadow_flags : TEXCOORD13;
};

RectVaryings vs_main(uint vertex_id : SV_VertexID, RectVertexIn inst) {
    static const float2 corners[6] = {
        float2(0, 0), float2(1, 0), float2(0, 1),
        float2(1, 0), float2(1, 1), float2(0, 1)
    };
    float2 corner = corners[vertex_id];
    float2 pixel = inst.pos + corner * inst.size;
    RectVaryings output;
    output.position = float4(pixel.x / viewport.size.x * 2.0 - 1.0,
                             1.0 - pixel.y / viewport.size.y * 2.0, 0.0, 1.0);
    output.color = inst.color;
    output.local_pos = corner * inst.size;
    output.rect_size = inst.size;
    output.border_radius = inst.border_radius;
    output.border_width = inst.border_width;
    output.border_color = inst.border_color;
    output.outline_width = inst.outline_width;
    output.outline_color = inst.outline_color;
    output.pixel_pos = pixel;
    output.clip_rect = inst.clip_rect;
    output.clip_border_radius = inst.clip_border_radius;
    output.shadow_params = inst.shadow_params;
    output.shadow_color = inst.shadow_color;
    output.shadow_flags = inst.shadow_flags;
    return output;
}

float rounded_rect_sdf(float2 p, float2 half_size, float4 radii) {
    float radius = p.x < 0.0 ? (p.y < 0.0 ? radii.x : radii.w)
                             : (p.y < 0.0 ? radii.y : radii.z);
    radius = min(radius, min(half_size.x, half_size.y));
    float2 q = abs(p) - half_size + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

float gaussian_integral(float x, float sigma) {
    float normalized = x / (sigma * 1.4142135);
    float t = 1.0 / (1.0 + 0.3275911 * abs(normalized));
    float polynomial = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741
        + t * (-1.453152027 + t * 1.061405429))));
    float erf_value = 1.0 - polynomial * exp(-normalized * normalized);
    return 0.5 + 0.5 * sign(normalized) * erf_value;
}

float shadow_alpha(float distance, float blur, bool is_inset) {
    float sigma = blur * 0.5;
    float alpha = 0.0;
    if (sigma < 0.001) {
        if (is_inset) alpha = distance < 0.0 ? 0.0 : 1.0;
        else alpha = distance < 0.0 ? 1.0 : 0.0;
    } else {
        alpha = gaussian_integral(-distance, sigma);
        if (is_inset) alpha = 1.0 - alpha;
    }
    return alpha;
}

float clip_coverage(float2 pixel, float4 rect, float4 radii) {
    float coverage = 1.0;
    if (rect.z >= 0.0) {
        if (rect.z <= 0.01 || rect.w <= 0.01) {
            coverage = 0.0;
        } else if (any(radii > 0.0)) {
            float2 half_size = rect.zw * 0.5;
            coverage = 1.0 - smoothstep(-0.5, 0.5,
                rounded_rect_sdf(pixel - rect.xy - half_size, half_size, radii));
        } else {
            float2 high = rect.xy + rect.zw;
            coverage = smoothstep(-0.5, 0.5, pixel.x - rect.x)
                     * smoothstep(-0.5, 0.5, high.x - pixel.x)
                     * smoothstep(-0.5, 0.5, pixel.y - rect.y)
                     * smoothstep(-0.5, 0.5, high.y - pixel.y);
        }
    }
    return coverage;
}

float shadow_side_mask(float2 centered, float2 half_size, float side_type,
                       float angle_start, float angle_end) {
    float mask = 0.0;
    const float edge = 2.0;
    if (side_type < 0.5) {
        mask = 1.0;
    } else if (side_type < 1.5) {
        mask = 1.0 - smoothstep(-half_size.y - edge, -half_size.y + edge, centered.y);
    } else if (side_type < 2.5) {
        mask = smoothstep(half_size.x - edge, half_size.x + edge, centered.x);
    } else if (side_type < 3.5) {
        mask = smoothstep(half_size.y - edge, half_size.y + edge, centered.y);
    } else if (side_type < 4.5) {
        mask = 1.0 - smoothstep(-half_size.x - edge, -half_size.x + edge, centered.x);
    } else if (side_type < 5.5) {
        float distance_y = abs(centered.y) - half_size.y;
        float distance_x = abs(centered.x) - half_size.x;
        mask = smoothstep(-edge, edge, distance_y - distance_x);
    } else if (side_type < 6.5) {
        float distance_x = abs(centered.x) - half_size.x;
        float distance_y = abs(centered.y) - half_size.y;
        mask = smoothstep(-edge, edge, distance_x - distance_y);
    } else {
        float distance_top = -(centered.y + half_size.y);
        float distance_right = centered.x - half_size.x;
        float distance_bottom = centered.y - half_size.y;
        float distance_left = -(centered.x + half_size.x);
        if (side_type >= 7.5 && side_type < 8.5) {
            mask = smoothstep(-edge, edge,
                max(distance_top, distance_left) - max(distance_bottom, distance_right));
        } else if (side_type >= 8.5 && side_type < 9.5) {
            mask = smoothstep(-edge, edge,
                max(distance_top, distance_right) - max(distance_bottom, distance_left));
        } else if (side_type >= 9.5 && side_type < 10.5) {
            mask = smoothstep(-edge, edge,
                max(distance_bottom, distance_right) - max(distance_top, distance_left));
        } else if (side_type >= 10.5 && side_type < 11.5) {
            mask = smoothstep(-edge, edge,
                max(distance_bottom, distance_left) - max(distance_top, distance_right));
        } else {
            float angle = atan2(centered.y, centered.x);
            float wrapped_angle = angle - angle_start;
            float range = angle_end - angle_start;
            const float two_pi = 6.2831853;
            wrapped_angle -= floor(wrapped_angle / two_pi) * two_pi;
            range -= floor(range / two_pi) * two_pi;
            mask = wrapped_angle <= range ? 1.0 : 0.0;
        }
    }
    return mask;
}

float srgb_to_linear(float c) {
    return c <= 0.04045 ? c / 12.92 : pow(max((c + 0.055) / 1.055, 0.0), 2.4);
}
float linear_to_srgb(float c) {
    return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(max(c, 0.0), 1.0 / 2.4) - 0.055;
}
float4 srgb_color_to_linear(float4 color) {
    return float4(srgb_to_linear(color.r), srgb_to_linear(color.g),
                  srgb_to_linear(color.b), color.a);
}
float4 finalize_color(float4 color, float alpha) {
    float4 converted = viewport.surface_is_srgb >= 1.5
        ? color : srgb_color_to_linear(color);
    float4 result = float4(converted.rgb * converted.a, converted.a) * alpha;
    if (viewport.surface_is_srgb < 0.5 && result.a > 0.00001) {
        float3 straight = result.rgb / result.a;
        result.rgb = float3(linear_to_srgb(straight.r), linear_to_srgb(straight.g),
                            linear_to_srgb(straight.b)) * result.a;
    }
    return result;
}

float4 fs_main(RectVaryings input) : SV_Target0 {
    if (input.shadow_color.a > 0.0) {
        float2 shadow_offset = input.shadow_params.xy;
        float shadow_blur = input.shadow_params.z;
        float shadow_spread = input.shadow_params.w;
        bool is_inset = input.shadow_flags.x > 0.5;
        float expand_x = shadow_blur + abs(shadow_spread) + abs(shadow_offset.x);
        float expand_y = shadow_blur + abs(shadow_spread) + abs(shadow_offset.y);
        float2 original_size = input.rect_size - float2(expand_x, expand_y) * 2.0;
        float2 original_half = original_size * 0.5;
        float2 centered = input.local_pos - input.rect_size * 0.5;
        float2 shadow_half = original_half + shadow_spread;
        float2 shadow_point = centered - shadow_offset;
        float4 spread_radii = max(input.border_radius + float4(shadow_spread,
            shadow_spread, shadow_spread, shadow_spread), float4(0.0, 0.0, 0.0, 0.0));
        float distance = rounded_rect_sdf(shadow_point, shadow_half, spread_radii);
        float alpha = shadow_alpha(distance, shadow_blur, is_inset);
        alpha *= shadow_side_mask(centered, original_half, input.shadow_flags.y,
                                  input.shadow_flags.z, input.shadow_flags.w);
        float clip = clip_coverage(input.pixel_pos, input.clip_rect,
                                   input.clip_border_radius);
        if (is_inset) {
            float original_distance = rounded_rect_sdf(centered, original_half,
                                                       input.border_radius);
            float inside = 1.0 - smoothstep(-0.5, 0.5, original_distance);
            return finalize_color(input.shadow_color, alpha * inside * clip);
        }
        return finalize_color(input.shadow_color, alpha * clip);
    }

    float outline_top = input.outline_width.x;
    float outline_right = input.outline_width.y;
    float outline_bottom = input.outline_width.z;
    float outline_left = input.outline_width.w;
    bool has_outline = outline_top + outline_right + outline_bottom + outline_left > 0.0;
    float2 original_size = float2(input.rect_size.x - outline_left - outline_right,
                                 input.rect_size.y - outline_top - outline_bottom);
    float2 original_half = original_size * 0.5;
    float2 original_local = input.local_pos - float2(outline_left, outline_top);
    float2 original_centered = original_local - original_half;
    float distance = rounded_rect_sdf(original_centered, original_half,
                                      input.border_radius);
    float outer_alpha = 1.0 - smoothstep(-0.5, 0.5, distance);
    float clip = clip_coverage(input.pixel_pos, input.clip_rect,
                               input.clip_border_radius);

    float4 fill_color = input.color;
    float4 stroke_color = input.border_color;
    float4 outline_color = input.outline_color;
    if (viewport.surface_is_srgb < 1.5) {
        fill_color = srgb_color_to_linear(fill_color);
        stroke_color = srgb_color_to_linear(stroke_color);
        outline_color = srgb_color_to_linear(outline_color);
    }

    float4 outline_premultiplied = 0.0;
    if (has_outline) {
        float2 expanded_half = input.rect_size * 0.5;
        float2 expanded_centered = input.local_pos - expanded_half;
        float4 outline_radii = float4(
            input.border_radius.x > 0.0 ? input.border_radius.x + max(outline_top, outline_left) : 0.0,
            input.border_radius.y > 0.0 ? input.border_radius.y + max(outline_top, outline_right) : 0.0,
            input.border_radius.z > 0.0 ? input.border_radius.z + max(outline_bottom, outline_right) : 0.0,
            input.border_radius.w > 0.0 ? input.border_radius.w + max(outline_bottom, outline_left) : 0.0);
        float outline_distance = rounded_rect_sdf(expanded_centered, expanded_half,
                                                   outline_radii);
        float outline_outer_alpha = 1.0 - smoothstep(-0.5, 0.5, outline_distance);
        float outline_ring_alpha = saturate(outline_outer_alpha - outer_alpha);
        outline_premultiplied = float4(outline_color.rgb * outline_color.a,
                                       outline_color.a) * outline_ring_alpha;
    }

    bool has_border = input.border_width.x + input.border_width.y
        + input.border_width.z + input.border_width.w > 0.0;
    if (has_border) {
        float border_top = input.border_width.x;
        float border_right = input.border_width.y;
        float border_bottom = input.border_width.z;
        float border_left = input.border_width.w;
        float2 inner_offset = float2((border_left - border_right) * 0.5,
                                     (border_top - border_bottom) * 0.5);
        float2 inner_half = max(float2(original_half.x - (border_left + border_right) * 0.5,
                                       original_half.y - (border_top + border_bottom) * 0.5),
                                float2(0.0, 0.0));
        float4 inner_radii = max(float4(
            input.border_radius.x - max(border_top, border_left),
            input.border_radius.y - max(border_top, border_right),
            input.border_radius.z - max(border_bottom, border_right),
            input.border_radius.w - max(border_bottom, border_left)),
            float4(0.0, 0.0, 0.0, 0.0));
        float2 inner_point = original_centered - inner_offset;
        float inner_distance = rounded_rect_sdf(inner_point, inner_half, inner_radii);
        float inner_alpha = 1.0 - smoothstep(-0.5, 0.5, inner_distance);
        float border_alpha = saturate(outer_alpha - inner_alpha);
        float final_inner_alpha = min(inner_alpha, outer_alpha);
        float4 fill_premultiplied = float4(fill_color.rgb * fill_color.a,
                                          fill_color.a) * final_inner_alpha;
        float4 border_premultiplied = float4(stroke_color.rgb * stroke_color.a,
                                            stroke_color.a) * border_alpha;
        float4 combined = (outline_premultiplied + border_premultiplied
                           + fill_premultiplied) * clip;
        if (viewport.surface_is_srgb < 0.5 && combined.a > 0.00001) {
            float3 straight = combined.rgb / combined.a;
            combined.rgb = float3(linear_to_srgb(straight.r), linear_to_srgb(straight.g),
                                  linear_to_srgb(straight.b)) * combined.a;
        }
        return combined;
    }

    float4 fill_premultiplied = float4(fill_color.rgb * fill_color.a,
                                       fill_color.a) * outer_alpha;
    float4 combined = (outline_premultiplied + fill_premultiplied) * clip;
    if (viewport.surface_is_srgb < 0.5 && combined.a > 0.000001) {
        float3 straight = combined.rgb / combined.a;
        combined.rgb = float3(linear_to_srgb(straight.r), linear_to_srgb(straight.g),
                              linear_to_srgb(straight.b)) * combined.a;
    }
    return combined;
}
