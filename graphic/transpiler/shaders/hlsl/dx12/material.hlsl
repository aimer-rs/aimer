// Hand-authored HLSL for Cupid's native backdrop material pipeline.
struct MaterialUniform {
    float4 bounds;
    float4 tint;
    float4 border_color;
    float4 shadow;
    float4 effect;
    float4 light;
    float4 detail;
    float4 radii;
    float4 clip_rect;
    float4 clip_radii;
    float4 viewport;
    float4 backdrop_rect;
    float4 liquid;
    float4 liquid2;
};

ConstantBuffer<MaterialUniform> material : register(b0, space0);
Texture2D<float4> backdrop : register(t1, space0);
Texture2D<float4> horizontal_blur : register(t3, space0);
SamplerState backdrop_sampler : register(s2, space0);

struct MaterialVaryings {
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
    float2 pixel_pos : TEXCOORD1;
};

MaterialVaryings vs_main(uint vertex_id : SV_VertexID) {
    static const float2 corners[6] = {
        float2(0, 0), float2(1, 0), float2(0, 1),
        float2(1, 0), float2(1, 1), float2(0, 1)
    };
    float2 uv = corners[vertex_id];
    float2 pixel = material.bounds.xy + uv * material.bounds.zw;
    MaterialVaryings output;
    output.position = float4(pixel.x / max(material.viewport.x, 1.0) * 2.0 - 1.0,
                             1.0 - pixel.y / max(material.viewport.y, 1.0) * 2.0,
                             0.0, 1.0);
    output.uv = uv;
    output.pixel_pos = pixel;
    return output;
}

float selected_radius(float2 position, float4 radii) {
    float radius = radii.z;
    if (position.x < 0.0) {
        radius = position.y < 0.0 ? radii.x : radii.w;
    } else if (position.y < 0.0) {
        radius = radii.y;
    }
    return radius;
}

float rounded_rect_sdf(float2 position, float2 half_size, float4 radii) {
    float radius = min(selected_radius(position, radii), min(half_size.x, half_size.y));
    float2 q = abs(position) - half_size + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

float sdf_alpha(float distance) {
    float antialias = max(fwidth(distance), 0.65);
    return 1.0 - smoothstep(-antialias, antialias, distance);
}

float liquid_shape_distance(float2 uv) {
    float2 size = max(material.bounds.zw, float2(1.0, 1.0));
    float2 half_size = size * 0.5;
    float2 local_position = (uv - 0.5) * size;
    float tip_pull = material.liquid.w;
    if (tip_pull > 0.0) {
        float vertical_t = saturate(local_position.y / max(half_size.y, 1.0) * 0.5 + 0.5);
        float taper = lerp(1.0 - tip_pull * 0.82, 1.0, vertical_t);
        local_position.x /= max(taper, 0.12);
    }
    float distance = rounded_rect_sdf(local_position, half_size, material.radii);
    float blob_amount = material.liquid.x;
    if (blob_amount > 0.0) {
        float seed = material.liquid.y * 6.28318;
        float angle = atan2(local_position.y, local_position.x);
        float wobble = sin(angle * 3.0 + seed) * 0.5
            + sin(angle * 5.0 - seed * 1.7) * 0.3
            + sin(angle * 7.0 + seed * 2.3) * 0.2;
        float amplitude = blob_amount * min(half_size.x, half_size.y) * 0.20;
        distance -= wobble * amplitude;
    }
    return distance;
}

float surface_alpha(float2 uv) {
    float distance;
    if (material.effect.x > 0.5 && material.effect.x < 1.5) {
        distance = liquid_shape_distance(uv);
    } else {
        float2 size = max(material.bounds.zw, float2(1.0, 1.0));
        float2 local_position = (uv - 0.5) * size;
        distance = rounded_rect_sdf(local_position, size * 0.5, material.radii);
    }
    return sdf_alpha(distance);
}

float clip_alpha(float2 pixel) {
    float coverage = 1.0;
    if (material.clip_rect.z >= 0.0) {
        if (material.clip_rect.z <= 0.0 || material.clip_rect.w <= 0.0) {
            coverage = 0.0;
        } else {
            float2 half_size = material.clip_rect.zw * 0.5;
            float2 center = material.clip_rect.xy + half_size;
            coverage = sdf_alpha(rounded_rect_sdf(pixel - center, half_size,
                                                   material.clip_radii));
        }
    }
    return coverage;
}

float srgb_to_linear(float value) {
    return value <= 0.04045 ? value / 12.92
        : pow(max((value + 0.055) / 1.055, 0.0), 2.4);
}

float linear_to_srgb(float value) {
    return value <= 0.0031308 ? value * 12.92
        : 1.055 * pow(max(value, 0.0), 1.0 / 2.4) - 0.055;
}

float3 srgb_rgb_to_linear(float3 color) {
    return float3(srgb_to_linear(color.r), srgb_to_linear(color.g),
                  srgb_to_linear(color.b));
}

float3 linear_rgb_to_srgb(float3 color) {
    return float3(linear_to_srgb(color.r), linear_to_srgb(color.g),
                  linear_to_srgb(color.b));
}

float4 sample_backdrop(float2 pixel) {
    float2 valid_size = max(material.backdrop_rect.zw, float2(1.0, 1.0));
    float center_offset = (material.effect.x > 1.5 || material.effect.x < -0.5) ? 0.5 : 0.0;
    float2 local_pixel = clamp(pixel - material.backdrop_rect.xy - float2(center_offset, center_offset),
                               float2(0.0, 0.0), valid_size - 1.0);
    uint width;
    uint height;
    backdrop.GetDimensions(width, height);
    float2 dimensions = float2(width, height);
    return backdrop.Sample(backdrop_sampler,
                          (local_pixel + 0.5) / max(dimensions, float2(1.0, 1.0)));
}

float4 sample_gaussian_source(float2 pixel) {
    float2 size = max(material.backdrop_rect.zw, float2(1.0, 1.0));
    float2 local = clamp(pixel - material.backdrop_rect.xy - 0.5, 0.0, size - 1.0);
    uint width, height;
    backdrop.GetDimensions(width, height);
    float2 uv = (local + 0.5) / float2(width, height);
    if (material.effect.x < -0.5) return backdrop.SampleLevel(backdrop_sampler, uv, 0.0);
    return horizontal_blur.SampleLevel(backdrop_sampler, uv, 0.0);
}

float4 gaussian_backdrop(float2 pixel) {
    float2 axis = material.effect.x < -0.5 ? float2(1.0, 0.0) : float2(0.0, 1.0);
    int support = int(ceil(material.detail.z));
    float4 total = sample_gaussian_source(pixel);
    float normalization = 1.0;
    float previous = 1.0;
    float ratio = material.liquid2.z;
    for (int tap = 1; tap <= support; tap += 2) {
        float first = previous * ratio;
        ratio *= material.liquid2.w;
        float second = tap + 1 <= support ? first * ratio : 0.0;
        float weight = first + second;
        float2 offset = axis * (float(tap) + second / max(weight, 0.00000001));
        total += (sample_gaussian_source(pixel + offset) + sample_gaussian_source(pixel - offset)) * weight;
        normalization += 2.0 * weight;
        previous = second;
        ratio *= material.liquid2.w;
    }
    return total / normalization;
}

// A large blur is made on a texture `material.liquid.x` times smaller in each
// direction (a power of two up to 8; below 2 the blur is made at full size). One
// pass averages each block of the capture into a texel, a horizontal and a
// vertical Gaussian then run over the small texture, where every pass center is
// a texel center and the two-taps-in-one bilinear read of `gaussian_backdrop` is
// exact, and the material's pass reads the result back with bilinear filtering.
// Sigma is at least four texels there, so the filtering costs under a level.
float4 reduced_fetch(float2 position, float2 valid, bool from_capture) {
    float2 clamped = clamp(position, float2(0.5, 0.5), valid - float2(0.5, 0.5));
    uint width, height;
    if (from_capture) {
        backdrop.GetDimensions(width, height);
        return backdrop.SampleLevel(backdrop_sampler, clamped / float2(width, height), 0.0);
    }
    horizontal_blur.GetDimensions(width, height);
    return horizontal_blur.SampleLevel(backdrop_sampler, clamped / float2(width, height), 0.0);
}

// The average of the `factor` x `factor` block of capture texels under a texel
// of the small texture, as `factor / 2` x `factor / 2` bilinear reads of four
// texels each.
float4 decimate_backdrop(float2 pixel) {
    float factor = material.liquid.x;
    int count = int(factor) / 2;
    float2 center = pixel * factor;
    float2 valid = max(material.backdrop_rect.zw, float2(1.0, 1.0));
    float4 total = float4(0.0, 0.0, 0.0, 0.0);
    for (int row = 0; row < count; row++) {
        for (int column = 0; column < count; column++) {
            float2 offset = float2(float(2 * column - (count - 1)), float(2 * row - (count - 1)));
            total += reduced_fetch(center + offset, valid, true);
        }
    }
    return total / float(count * count);
}

// `gaussian_backdrop` over the small texture, along `axis`.
float4 gaussian_reduced(float2 pixel, float2 axis) {
    float2 valid = max(material.backdrop_rect.zw, float2(1.0, 1.0));
    int support = int(ceil(material.detail.z));
    float4 total = reduced_fetch(pixel, valid, false);
    float normalization = 1.0;
    float previous = 1.0;
    float ratio = material.liquid2.z;
    for (int tap = 1; tap <= support; tap += 2) {
        float first = previous * ratio;
        ratio *= material.liquid2.w;
        float second = tap + 1 <= support ? first * ratio : 0.0;
        float weight = first + second;
        float2 offset = axis * (float(tap) + second / max(weight, 0.00000001));
        total += (reduced_fetch(pixel + offset, valid, false) + reduced_fetch(pixel - offset, valid, false)) * weight;
        normalization += 2.0 * weight;
        previous = second;
        ratio *= material.liquid2.w;
    }
    return total / normalization;
}

// The blurred small texture under a fragment of the material's own pass.
float4 reduced_result(float2 pixel) {
    float factor = material.liquid.x;
    float2 valid = max(ceil(material.backdrop_rect.zw / factor), float2(1.0, 1.0));
    return reduced_fetch((pixel - material.backdrop_rect.xy) / factor, valid, false);
}

float4 frosted_backdrop(float2 pixel) {
    float radius = material.detail.z;
    float4 total = sample_backdrop(pixel);
    if (radius > 0.0) {
        float2 center = pixel;
        if (material.effect.x < 1.5) {
            float reed = sin(pixel.x * 0.115) + sin(pixel.x * 0.037 + pixel.y * 0.021);
            float ripple = cos(pixel.y * 0.083 + reed * 0.7);
            center = pixel + float2(reed * radius * 0.075, ripple * radius * 0.035);
        }
        float axis = radius * 0.42;
        float diagonal = radius * 0.24;
        total = sample_backdrop(center) * 0.20;
        total += sample_backdrop(center + float2(axis, 0.0)) * 0.12;
        total += sample_backdrop(center - float2(axis, 0.0)) * 0.12;
        total += sample_backdrop(center + float2(0.0, axis)) * 0.12;
        total += sample_backdrop(center - float2(0.0, axis)) * 0.12;
        total += sample_backdrop(center + float2(diagonal, diagonal)) * 0.08;
        total += sample_backdrop(center + float2(diagonal, -diagonal)) * 0.08;
        total += sample_backdrop(center + float2(-diagonal, diagonal)) * 0.08;
        total += sample_backdrop(center - float2(diagonal, diagonal)) * 0.08;
    }
    return total;
}

float3 backdrop_to_material_rgb(float3 color) {
    float3 result = color;
    if (material.viewport.w > 0.5 && material.viewport.z > 0.5)
        result = linear_rgb_to_srgb(color);
    return result;
}

float3 adjust_backdrop(float3 color) {
    float luminance = dot(color, float3(0.2126, 0.7152, 0.0722));
    float3 saturated = lerp(luminance.xxx, color, material.light.x);
    float3 contrasted = (saturated - 0.5) * material.light.z + 0.5;
    return saturate(contrasted * material.light.y);
}

float3 glass_frosted_base(float3 sampled_backdrop) {
    float3 milky_tint = lerp(float3(0.86, 0.89, 0.92), material.tint.rgb, 0.72);
    float tint_strength = clamp(material.effect.y * material.tint.a * 0.76, 0.0, 0.82);
    return lerp(sampled_backdrop, milky_tint, tint_strength);
}

float3 glass_glow(float2 uv, float3 sampled_backdrop) {
    float2 blue_point = (uv - float2(0.60, 0.34)) * float2(1.35, 1.05);
    float blue = exp(-dot(blue_point, blue_point) * 3.2);
    float2 cyan_point = (uv - float2(0.18, 0.84)) * float2(1.5, 1.1);
    float cyan = exp(-dot(cyan_point, cyan_point) * 4.4);
    float diagonal = smoothstep(0.18, 0.92, uv.x + (1.0 - uv.y) * 0.32);
    float3 base = glass_frosted_base(sampled_backdrop);
    float3 highlight_color = lerp(float3(0.82, 0.91, 1.0), material.tint.rgb, 0.38);
    float highlight = (blue * 0.22 + cyan * 0.28 + diagonal * 0.08) * material.detail.x;
    return base + highlight_color * highlight;
}

float glass_rim(float2 uv) {
    float edge = min(min(uv.x, uv.y), min(1.0 - uv.x, 1.0 - uv.y));
    float rim = 1.0 - smoothstep(0.0, 0.075, edge);
    float top_sheen = 1.0 - smoothstep(0.0, 0.18, uv.y + uv.x * 0.18);
    return clamp(rim * 0.82 + top_sheen * 0.14, 0.0, 1.0);
}

float2 liquid_warp(float2 uv) {
    float phase = material.effect.z;
    float strength = material.effect.w;
    float interaction = material.detail.y;
    return float2(
        sin(uv.y * 25.0 + phase * 1.2 + interaction * 4.0) * 0.006
            + sin(uv.y * 8.0 - phase * 0.55) * 0.004,
        cos(uv.x * 22.0 - phase * 0.9 + interaction * 3.0) * 0.0055
            + cos(uv.x * 7.0 + phase * 0.45) * 0.0035) * strength;
}

float2 liquid_shape_normal(float2 uv) {
    const float epsilon = 0.0015;
    float dx = liquid_shape_distance(uv + float2(epsilon, 0.0))
        - liquid_shape_distance(uv - float2(epsilon, 0.0));
    float dy = liquid_shape_distance(uv + float2(0.0, epsilon))
        - liquid_shape_distance(uv - float2(0.0, epsilon));
    float2 gradient = float2(dx, dy);
    float gradient_length = length(gradient);
    float2 normal = float2(0.0, 0.0);
    if (gradient_length >= 1e-5) normal = gradient / gradient_length;
    return normal;
}

float liquid_bevel_slope(float inset, float bevel_radius) {
    float t = clamp(inset / bevel_radius, 0.0, 1.0);
    float u = 1.0 - t;
    return u / sqrt(max(1.0 - u * u, 1e-4));
}

float3 liquid_bevel_normal(float2 uv, float distance, float bevel_radius) {
    float inset = max(-distance, 0.0);
    float slope = liquid_bevel_slope(inset, bevel_radius);
    float2 outward = liquid_shape_normal(uv);
    return normalize(float3(outward * slope, 1.0));
}

float2 liquid_bevel_refract_offset(float3 normal, float eta, float depth) {
    float3 incident = float3(0.0, 0.0, -1.0);
    float cos_i = -dot(normal, incident);
    float sin2_t = eta * eta * max(1.0 - cos_i * cos_i, 0.0);
    float2 offset = -normal.xy * depth;
    if (sin2_t < 1.0) {
        float cos_t = sqrt(1.0 - sin2_t);
        float3 refracted = eta * incident + (eta * cos_i - cos_t) * normal;
        offset = refracted.xy / max(abs(refracted.z), 0.05) * depth;
    }
    return offset;
}

float3 liquid_chromatic_sample(float2 pixel, float3 normal, float eta,
                               float depth, float aberration) {
    float3 sample = float3(0.0, 0.0, 0.0);
    if (aberration <= 0.0) {
        float2 offset = liquid_bevel_refract_offset(normal, eta, depth);
        sample = backdrop_to_material_rgb(sample_backdrop(pixel + offset).rgb);
    } else {
        float spread = aberration * 0.05;
        float2 red_offset = liquid_bevel_refract_offset(normal, eta * (1.0 - spread), depth);
        float2 green_offset = liquid_bevel_refract_offset(normal, eta, depth);
        float2 blue_offset = liquid_bevel_refract_offset(normal, eta * (1.0 + spread), depth);
        float red = backdrop_to_material_rgb(sample_backdrop(pixel + red_offset).rgb).r;
        float green = backdrop_to_material_rgb(sample_backdrop(pixel + green_offset).rgb).g;
        float blue = backdrop_to_material_rgb(sample_backdrop(pixel + blue_offset).rgb).b;
        sample = float3(red, green, blue);
    }
    return sample;
}

float liquid_rim(float distance) {
    float2 size = max(material.bounds.zw, float2(1.0, 1.0));
    float width = max(min(size.x, size.y) * 0.10, 3.0);
    float inner = saturate(-distance / width);
    return pow(1.0 - inner, 2.2);
}

float liquid_sheen(float2 uv, float rim) {
    float phase = material.effect.z;
    float interaction = material.detail.y;
    float seed = material.liquid.y;
    float light_angle = 3.9 + sin(seed * 6.28318) * 0.25
        + sin(phase * 0.2) * 0.10 + interaction * 0.15;
    float2 light_dir = float2(cos(light_angle), sin(light_angle));
    float2 outward = normalize(uv - float2(0.5, 0.5) + float2(0.0001, -0.0001));
    float facing = clamp(dot(outward, light_dir), 0.0, 1.0);
    return rim * pow(facing, 2.4);
}

float3 adaptive_tint(float3 sampled_backdrop) {
    float luminance = dot(sampled_backdrop, float3(0.2126, 0.7152, 0.0722));
    float3 lightened = lerp(material.tint.rgb, float3(1.0, 1.0, 1.0), 0.55);
    float3 darkened = lerp(material.tint.rgb, float3(0.0, 0.0, 0.0), 0.45);
    return lerp(lightened, darkened, luminance);
}

float3 material_rgb(float2 uv, float3 sampled_backdrop) {
    float3 color = float3(0.0, 0.0, 0.0);
    if (material.effect.x < 0.5) {
        float3 glow = glass_glow(uv, sampled_backdrop);
        float rim = glass_rim(uv);
        float border_mix = material.border_color.a * clamp(material.detail.w, 0.0, 1.0);
        float3 edge_color = lerp(material.tint.rgb, material.border_color.rgb, border_mix);
        float edge_strength = rim * material.light.w * 0.35;
        color = lerp(glow, edge_color, edge_strength);
    } else {
        float distance = liquid_shape_distance(uv);
        float rim = liquid_rim(distance);
        float sheen = liquid_sheen(uv, rim);
        float luminance = dot(sampled_backdrop, float3(0.2126, 0.7152, 0.0722));
        float tint_strength = clamp(material.tint.a * material.effect.y * 0.35, 0.0, 0.5);
        float3 tinted = lerp(sampled_backdrop, adaptive_tint(sampled_backdrop), tint_strength);
        float3 rim_tone = lerp(float3(1.0, 1.0, 1.0), float3(0.0, 0.0, 0.0), luminance);
        float3 rim_shaded = lerp(tinted, rim_tone, rim * material.light.w * 0.55);
        color = lerp(rim_shaded, float3(1.0, 1.0, 1.0), sheen * material.detail.x);
    }
    return color;
}

float material_alpha(float2 uv) {
    float opacity = material.effect.y * material.tint.a;
    float alpha = 0.0;
    if (material.effect.x < 0.5) {
        float rim = glass_rim(uv);
        if (material.viewport.w > 0.5 && material.detail.z > 0.0) alpha = 1.0;
        else alpha = opacity * (0.62 + rim * material.light.w * 0.30);
    } else {
        float distance = liquid_shape_distance(uv);
        float rim = liquid_rim(distance);
        float sheen = liquid_sheen(uv, rim);
        float base_alpha = lerp(0.8, 1.0, clamp(opacity, 0.0, 1.0));
        alpha = clamp(base_alpha + rim * material.light.w * 0.15
            + sheen * material.detail.x * 0.15, 0.0, 1.0);
    }
    return alpha;
}

// Rounded-rectangle normal in device pixels, independent of aspect ratio.
float3 lens_bevel_normal(float2 position, float2 half_size, float4 radii, float distance, float bevel_width) {
    float radius = min(selected_radius(position, radii), min(half_size.x, half_size.y));
    float2 q = abs(position) - half_size + radius;
    float2 outer = max(q, float2(0.0, 0.0));
    float2 outward = q.x > q.y ? float2(1.0, 0.0) : float2(0.0, 1.0);
    if (dot(outer, outer) > 0.0) outward = normalize(outer);
    outward *= sign(position);
    float slope = liquid_bevel_slope(max(-distance, 0.0), bevel_width);
    return normalize(float3(outward * slope, 1.0));
}

float4 light_edge(float2 pixel) {
    float2 half_size = material.bounds.zw * 0.5;
    float2 position = pixel - material.bounds.xy - half_size;
    float radius = min(selected_radius(position, material.radii), min(half_size.x, half_size.y));
    float2 q = abs(position) - half_size + float2(radius, radius);
    float2 outer = max(q, float2(0.0, 0.0));
    // Smooth only the lighting normal across straight/curved joins. Keep
    // the exact SDF for coverage; limiting the blend to flat spans preserves circles.
    float2 blend = min(float2(material.detail.w, material.detail.w), max(half_size - float2(radius, radius), float2(0.0, 0.0)));
    float2 join = max(blend - abs(q), float2(0.0, 0.0));
    float2 lighting_outer = outer + join * join / max(4.0 * blend, float2(0.0001, 0.0001));
    float2 outward = q.x > q.y ? float2(1.0, 0.0) : float2(0.0, 1.0);
    if (dot(lighting_outer, lighting_outer) > 0.0) outward = normalize(lighting_outer);
    outward *= sign(position);
    float distance = length(outer) + min(max(q.x, q.y), 0.0) - radius;
    // A cubic inward falloff has no solid band or abrupt inner boundary.
    // Grazing light narrows the bevel; the opposite side catches a weaker reflection.
    float alignment = dot(outward, material.effect.zw);
    float width = max(material.detail.w * (0.35 + 0.65 * abs(alignment)), 0.0001);
    float remaining = 1.0 - clamp(max(-distance, 0.0) / width, 0.0, 1.0);
    float coverage = sdf_alpha(distance) * remaining * remaining * remaining;
    float facing = pow(abs(alignment), 2.4) * (alignment >= 0.0 ? 1.0 : material.light.w);
    float alpha = clamp(coverage * facing * material.detail.x * material.border_color.a
        * material.effect.y * clip_alpha(pixel), 0.0, 1.0);
    float3 rgb = material.border_color.rgb;
    float3 converted = material.viewport.z > 0.5 ? srgb_rgb_to_linear(rgb) : rgb;
    return float4(converted * alpha, alpha);
}

// Analytic color ramp. The ramp lives in slots the other kinds use for other
// things (see `MaterialUniform::apply_gradient`): `tint` is the geometry,
// `light`/`detail` the stop offsets, `border_color`/`shadow` the packed RGBA8
// stop colors stored as raw float bits, and `liquid.xy` the shape and count.
float gradient_stop_offset(uint index) {
    return index < 4u ? material.light[index] : material.detail[index - 4u];
}

float4 gradient_stop_color(uint index) {
    float bits = index < 4u ? material.border_color[index] : material.shadow[index - 4u];
    uint packed = asuint(bits);
    float4 straight = float4(float(packed & 255u), float((packed >> 8u) & 255u),
                             float((packed >> 16u) & 255u), float(packed >> 24u)) / 255.0;
    // Blend premultiplied so a transparent stop does not tint its neighbors.
    return float4(straight.rgb * straight.a, straight.a);
}

float4 gradient_ramp(float t) {
    uint count = uint(material.liquid.y);
    float4 previous = gradient_stop_color(0u);
    float previous_offset = gradient_stop_offset(0u);
    if (t <= previous_offset) return previous;
    [loop]
    for (uint index = 1u; index < count; index++) {
        float offset = gradient_stop_offset(index);
        float4 current = gradient_stop_color(index);
        if (t <= offset) {
            float f = clamp((t - previous_offset) / max(offset - previous_offset, 0.000001), 0.0, 1.0);
            return lerp(previous, current, f);
        }
        previous = current;
        previous_offset = offset;
    }
    return previous;
}

float4 gradient_fill(float2 pixel) {
    float2 half_size = material.bounds.zw * 0.5;
    float2 position = pixel - material.bounds.xy - half_size;
    float distance = rounded_rect_sdf(position, half_size, material.radii);
    float coverage = sdf_alpha(distance) * clip_alpha(pixel);
    float2 local = pixel - material.bounds.xy;
    float4 geometry = material.tint;
    float shape = material.liquid.x;
    float t;
    if (shape < 0.5) {
        float2 axis = geometry.zw - geometry.xy;
        t = dot(local - geometry.xy, axis) / max(dot(axis, axis), 0.000001);
    } else if (shape < 1.5) {
        t = length((local - geometry.xy) / max(geometry.zw, float2(0.0001, 0.0001)));
    } else {
        float2 offset = local - geometry.xy;
        t = frac((atan2(offset.y, offset.x) - geometry.z) / 6.28318530718);
    }
    float4 ramp = gradient_ramp(clamp(t, 0.0, 1.0));
    // Interleaved gradient noise, one 8-bit step wide, hides banding in slow ramps.
    float noise = frac(52.9829189 * frac(dot(pixel, float2(0.06711056, 0.00583715))));
    float3 rgb = clamp(ramp.rgb / max(ramp.a, 0.00001) + (noise - 0.5) / 255.0, 0.0, 1.0);
    float alpha = clamp(ramp.a * material.effect.y * coverage, 0.0, 1.0);
    float3 converted = material.viewport.z > 0.5 ? srgb_rgb_to_linear(rgb) : rgb;
    return float4(converted * alpha, alpha);
}

float4 fs_main(MaterialVaryings input) : SV_Target0 {
    if (material.effect.x > 4.5) return gradient_fill(input.pixel_pos);
    if (material.effect.x > 3.5) return light_edge(input.pixel_pos);
    if (material.effect.x > 1.5 && material.viewport.w < 0.5) {
        discard;
    }
    if (material.effect.x < -0.5) {
        if (material.effect.x < -3.5) return gaussian_reduced(input.pixel_pos, float2(0.0, 1.0));
        if (material.effect.x < -2.5) return gaussian_reduced(input.pixel_pos, float2(1.0, 0.0));
        if (material.effect.x < -1.5) return decimate_backdrop(input.pixel_pos);
        return gaussian_backdrop(input.pixel_pos);
    }
    if (material.effect.x > 2.5) {
        float2 center = material.bounds.xy + material.bounds.zw * 0.5;
        float distance = rounded_rect_sdf(input.pixel_pos - center, material.bounds.zw * 0.5, material.radii);
        float depth = material.liquid2.y;
        float bevel_width = material.liquid2.z;
        float2 offset = float2(0.0, 0.0);
        if (bevel_width > 0.0 && -distance < bevel_width) {
            float3 normal = lens_bevel_normal(input.pixel_pos - center, material.bounds.zw * 0.5, material.radii, distance, bevel_width);
            offset = liquid_bevel_refract_offset(normal, 2.0 / 3.0, depth);
        }
        float2 origin = material.liquid.xy;
        float2 axes = material.effect.zw;
        float bulge = 1.0;
        if (material.liquid.w > 0.0) {
            float2 extent = max(max(origin - material.bounds.xy, material.bounds.xy + material.bounds.zw - origin), float2(0.001, 0.001));
            float2 normalized = (input.pixel_pos - origin) / extent * axes;
            float falloff = max(0.0, 1.0 - dot(normalized, normalized));
            bulge += material.liquid.w * falloff * falloff;
        }
        float2 warped = origin + (input.pixel_pos - origin + offset) / (material.liquid.z * bulge);
        float2 sample_pixel = lerp(input.pixel_pos, warped, axes);
        float mask = clamp(material.effect.y * sdf_alpha(distance) * clip_alpha(input.pixel_pos), 0.0, 1.0);
        if (mask <= 0.0) discard;
        float4 enlarged = sample_backdrop(sample_pixel);
        if (mask >= 1.0) { return enlarged; }
        return lerp(sample_backdrop(input.pixel_pos), enlarged, mask);
    }
    float4 filtered;
    if (material.effect.x > 1.5) {
        if (material.detail.z <= 0.0) {
            filtered = sample_backdrop(input.pixel_pos);
        } else if (material.liquid.x > 1.5) {
            filtered = reduced_result(input.pixel_pos);
        } else {
            filtered = gaussian_backdrop(input.pixel_pos);
        }
    } else {
        filtered = frosted_backdrop(input.pixel_pos);
    }
    if (material.effect.x > 1.5) {
        float3 color = backdrop_to_material_rgb(filtered.rgb / max(filtered.a, 0.00001));
        float3 rgb = adjust_backdrop(color);
        float3 converted = material.viewport.z > 0.5 ? srgb_rgb_to_linear(rgb) : rgb;
        float mask = surface_alpha(input.uv) * clip_alpha(input.pixel_pos);
        float weight = clamp(material.effect.y * mask, 0.0, 1.0);
        if (weight <= 0.0) {
            discard;
        }
        float4 result = float4(converted * filtered.a, filtered.a);
        if (weight < 1.0) {
            result = lerp(sample_backdrop(input.pixel_pos), result, weight);
        }
        return result;
    }
    float3 softened = backdrop_to_material_rgb(filtered.rgb);
    float3 material_backdrop = softened;
    if (material.effect.x > 0.5 && material.effect.x < 1.5) {
        float2 size = max(material.bounds.zw, float2(1.0, 1.0));
        float edge_distance = liquid_shape_distance(input.uv);
        float bevel_radius = max(material.liquid2.y, 1.0);
        float3 normal = liquid_bevel_normal(input.uv, edge_distance, bevel_radius);
        float magnification = material.liquid.z;
        float eta = 1.0 / (1.0 + magnification * 0.6);
        float depth = bevel_radius * (1.0 + magnification);
        float2 ripple_pixel = input.pixel_pos + liquid_warp(input.uv) * size;
        float3 refracted = liquid_chromatic_sample(ripple_pixel, normal, eta, depth,
                                                    material.liquid2.x);
        material_backdrop = lerp(softened, refracted, 0.78);
    }
    float3 sampled_backdrop = adjust_backdrop(material_backdrop);
    float3 rgb = saturate(material_rgb(input.uv, sampled_backdrop));
    float mask = surface_alpha(input.uv) * clip_alpha(input.pixel_pos);
    float alpha = saturate(material_alpha(input.uv) * mask);
    float3 converted = material.viewport.z > 0.5 ? srgb_rgb_to_linear(rgb) : rgb;
    return float4(converted * alpha, alpha);
}
