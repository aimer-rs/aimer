// Metal shading language twin of ./rect.wgsl.
//
// Keep the two in sync: every change to rect.wgsl needs the matching change
// here, because the Metal backend cannot consume WGSL.
//
// Conventions this file must honor (they are shared with backend/metal.rs,
// which is the only code that compiles and binds it):
//
//   * Entry points are `vs_main` / `fs_main`. The rect pipeline passes those
//     exact strings through RenderPipelineDescriptor::vertex.entry_point and
//     ::fragment.entry_point, so renaming them breaks the backend silently at
//     the `newFunctionWithName` lookup.
//   * The viewport uniform lives at buffer(0) in BOTH the vertex and fragment
//     argument tables. The instance stream is fed by an MTLVertexDescriptor
//     whose bufferIndex is the vertex argument-table slot the encoder binds
//     it at; it is deliberately never 0, which would alias the viewport.
//   * The vertex descriptor is mandatory rather than a convenience: a
//     RectInstance mixes f32 with four `Unorm8x4` colors in a 144-byte
//     record, and MSL's own struct alignment (float4 wants 16) does not
//     reproduce that layout. The descriptor lets hardware do the striding and
//     the unorm-to-float conversion, exactly as it does for wgpu.
//   * Blending is configured on the pipeline, not here: premultiplied alpha
//     (sourceFactorOne / destinationFactorOneMinusSrcAlpha on RGB and A) for
//     the draw pipeline, blending disabled for the clear twin.
//
// Floating-point parity note: compile with the same MTLCompileOptions wgpu
// uses (language version matched to the device, preserveInvariance = true,
// fastMathEnabled left at its default). Do not introduce precise_* intrinsics
// or re-tune expressions, and do not add a Y flip or an extra sRGB encode
// anywhere outside this file -- the shader already owns both, and the Metal
// tests assert byte equality against the wgpu reference render.

#include <metal_stdlib>

using namespace metal;

// WGSL: @group(0) @binding(0) var<uniform> viewport: Viewport
//
// The C layout here is byte-identical to the WGSL layout and to the Rust
// write of [width, height, is_srgb_f32, 0.0]: float2 aligns to 8, so size@0,
// surface_is_srgb@8, _pad@12, sizeof 16.
struct Viewport {
    float2 size;
    float surface_is_srgb;
    float _pad;
};

// WGSL: var corners = array<vec2<f32>, 6>(...) indexed by vertex_index.
//
// File-scope `constant` is the right address space: immutable, uniform memory,
// and indexable by a non-constant expression. The six values are the contract;
// they must stay in lockstep with rect.wgsl.
constant float2 kCorners[6] = {
    float2(0.0, 0.0),
    float2(1.0, 0.0),
    float2(0.0, 1.0),
    float2(1.0, 0.0),
    float2(1.0, 1.0),
    float2(0.0, 1.0),
};

// The 13 per-instance RectInstance fields. Attribute indices match the
// @location() annotations on the WGSL struct and the shader_location values in
// RectInstance::ATTRIBS, which backend/metal.rs transcribes into the vertex
// descriptor.
struct RectVertexIn {
    float2 pos [[attribute(0)]];
    float2 size [[attribute(1)]];
    float4 color [[attribute(2)]];
    float4 border_radius [[attribute(3)]];
    float4 border_width [[attribute(4)]];
    float4 border_color [[attribute(5)]];
    float4 outline_width [[attribute(6)]];
    float4 outline_color [[attribute(7)]];
    float4 clip_rect [[attribute(8)]];
    float4 clip_border_radius [[attribute(9)]];
    float4 shadow_params [[attribute(10)]];
    float4 shadow_color [[attribute(11)]];
    float4 shadow_flags [[attribute(12)]];
};

// WGSL struct VertexOutput. The `[[position]]` member only appears on the
// vertex side, so the fragment re-declares the same members without it: Metal
// matches stages by attribute index, not by struct type.
struct RectVaryings {
    float4 position [[position]];
    float4 color [[user(loc0)]];
    float2 local_pos [[user(loc1)]];
    float2 rect_size [[user(loc2)]];
    float4 border_radius [[user(loc3)]];
    float4 border_width [[user(loc4)]];
    float4 border_color [[user(loc5)]];
    float4 outline_width [[user(loc6)]];
    float4 outline_color [[user(loc7)]];
    float2 pixel_pos [[user(loc8)]];
    float4 clip_rect [[user(loc9)]];
    float4 clip_border_radius [[user(loc10)]];
    float4 shadow_params [[user(loc11)]];
    float4 shadow_color [[user(loc12)]];
    float4 shadow_flags [[user(loc13)]];
};

struct RectFragmentIn {
    float4 color [[user(loc0)]];
    float2 local_pos [[user(loc1)]];
    float2 rect_size [[user(loc2)]];
    float4 border_radius [[user(loc3)]];
    float4 border_width [[user(loc4)]];
    float4 border_color [[user(loc5)]];
    float4 outline_width [[user(loc6)]];
    float4 outline_color [[user(loc7)]];
    float2 pixel_pos [[user(loc8)]];
    float4 clip_rect [[user(loc9)]];
    float4 clip_border_radius [[user(loc10)]];
    float4 shadow_params [[user(loc11)]];
    float4 shadow_color [[user(loc12)]];
    float4 shadow_flags [[user(loc13)]];
};

vertex RectVaryings vs_main(
    uint vi [[vertex_id]],
    RectVertexIn inst [[stage_in]],
    constant Viewport& viewport [[buffer(0)]]
) {
    // Generate a quad from 6 vertices (two triangles).
    float2 corner = kCorners[vi];
    float2 pixel_pos = inst.pos + corner * inst.size;

    // Convert pixel coordinates to NDC: x: [0, width] -> [-1, 1],
    // y: [0, height] -> [1, -1]. Metal's viewport origin is the top-left with
    // +Y downward, so this flip is already the Metal convention.
    float2 ndc = float2(
        (pixel_pos.x / viewport.size.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / viewport.size.y) * 2.0);

    RectVaryings out;
    // z = 0 with w = 1 sits inside Metal's clip volume (-w <= z <= w) and maps
    // to window depth 0.5 under the default viewport znear 0 / zfar 1. There
    // is no depth attachment for this pipeline, so the value is never read.
    out.position = float4(ndc, 0.0, 1.0);
    out.color = inst.color;
    out.local_pos = corner * inst.size;
    out.rect_size = inst.size;
    out.border_radius = inst.border_radius;
    out.border_width = inst.border_width;
    out.border_color = inst.border_color;
    out.outline_width = inst.outline_width;
    out.outline_color = inst.outline_color;
    out.pixel_pos = pixel_pos;
    out.clip_rect = inst.clip_rect;
    out.clip_border_radius = inst.clip_border_radius;
    out.shadow_params = inst.shadow_params;
    out.shadow_color = inst.shadow_color;
    out.shadow_flags = inst.shadow_flags;
    return out;
}

/// SDF for a rounded rectangle with per-corner radii.
/// radii = (top-left, top-right, bottom-right, bottom-left)
static float sdf_rounded_rect(float2 p, float2 half_size, float4 radii) {
    // Select the radius for the quadrant the point is in.
    float r;
    if (p.x < 0.0) {
        if (p.y < 0.0) {
            r = radii.x; // top-left
        } else {
            r = radii.w; // bottom-left
        }
    } else {
        if (p.y < 0.0) {
            r = radii.y; // top-right
        } else {
            r = radii.z; // bottom-right
        }
    }
    r = min(r, min(half_size.x, half_size.y));
    float2 q = abs(p) - half_size + float2(r, r);
    return length(max(q, float2(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

/// Gaussian integral (erf-based) for smooth shadow falloff.
static float gaussian_integral(float x, float sigma) {
    float normalized = x / (sigma * 1.4142135); // sqrt(2)
    float t = 1.0 / (1.0 + 0.3275911 * abs(normalized));
    float poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741
               + t * (-1.453152027 + t * 1.061405429))));
    float erf_val = 1.0 - poly * exp(-normalized * normalized);
    return 0.5 + 0.5 * sign(normalized) * erf_val;
}

/// Compute shadow alpha from SDF distance and blur radius.
/// Per CSS spec, the blur radius equals ~2 sigma of the Gaussian, so
/// sigma = blur / 2.
///
/// WGSL select(e1, e2, cond) returns e2 when cond is true and MSL select(x, y,
/// a) has the same polarity, but every select site in rect.wgsl is
/// scalar-conditioned, so the ternary below is an exact translation and avoids
/// the question of whether a scalar-bool select overload exists at all.
static float shadow_alpha(float sdf_dist, float blur, bool is_inset) {
    float sigma = blur * 0.5;
    if (sigma < 0.001) {
        // Hard edge.
        if (is_inset) {
            // WGSL: select(1.0, 0.0, sdf_dist < 0.0)
            return (sdf_dist < 0.0) ? 0.0 : 1.0;
        } else {
            // WGSL: select(0.0, 1.0, sdf_dist < 0.0)
            return (sdf_dist < 0.0) ? 1.0 : 0.0;
        }
    }
    float a = gaussian_integral(-sdf_dist, sigma);
    if (is_inset) {
        return 1.0 - a;
    }
    return a;
}

/// Compute anti-aliased clip alpha from a clip rect in pixel coordinates.
/// clip_rect = (x, y, width, height). If width < 0, clipping is disabled
/// (returns 1.0).
static float clip_alpha(float2 pixel_pos, float4 clip_rect, float4 clip_radii) {
    if (clip_rect.z < 0.0) {
        return 1.0;
    }

    if (clip_rect.z <= 0.01 || clip_rect.w <= 0.01) {
        return 0.0;
    }

    if (clip_radii.x > 0.0 || clip_radii.y > 0.0 || clip_radii.z > 0.0 || clip_radii.w > 0.0) {
        // Rounded clip: use SDF.
        float2 clip_center = clip_rect.xy + clip_rect.zw * 0.5;
        float2 clip_half = clip_rect.zw * 0.5;
        float2 p = pixel_pos - clip_center;
        float d = sdf_rounded_rect(p, clip_half, clip_radii);
        return 1.0 - smoothstep(-0.5, 0.5, d);
    }

    float2 clip_min = clip_rect.xy;
    float2 clip_max = clip_rect.xy + clip_rect.zw;

    // Signed distance from each edge (positive = inside).
    float d_left = pixel_pos.x - clip_min.x;
    float d_right = clip_max.x - pixel_pos.x;
    float d_top = pixel_pos.y - clip_min.y;
    float d_bottom = clip_max.y - pixel_pos.y;

    // Smoothstep each edge for anti-aliasing (1px transition centered on the
    // boundary). Centering on the edge (-0.5..0.5) keeps the clipped region
    // fully opaque right up to the boundary; ramping from 0..1 instead would
    // erode ~1px of content just inside every edge.
    float a_left = smoothstep(-0.5, 0.5, d_left);
    float a_right = smoothstep(-0.5, 0.5, d_right);
    float a_top = smoothstep(-0.5, 0.5, d_top);
    float a_bottom = smoothstep(-0.5, 0.5, d_bottom);

    return a_left * a_right * a_top * a_bottom;
}

// Convert a single sRGB channel to linear space.
static float srgb_to_linear(float c) {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

// Convert a single linear channel back to sRGB.
static float linear_to_srgb(float c) {
    if (c <= 0.0031308) {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

// Convert an sRGB color (with alpha) to linear space. Alpha is kept as-is.
static float4 srgb_color_to_linear(float4 c) {
    return float4(
        srgb_to_linear(c.r),
        srgb_to_linear(c.g),
        srgb_to_linear(c.b),
        c.a);
}

/// Computes a side mask based on the fragment's position relative to the rect
/// center. side_type: 0=All, 1=Top, 2=Right, 3=Bottom, 4=Left, 5=Vertical,
/// 6=Horizontal, 7=Range, 8=TopLeft, 9=TopRight, 10=BottomRight,
/// 11=BottomLeft. For Range, angle_start and angle_end are in radians.
static float shadow_side_mask(float2 centered, float2 half_size, float4 border_radius,
                              float side_type, float angle_start, float angle_end) {
    if (side_type < 0.5) {
        // All
        return 1.0;
    }

    // For cardinal sides, use edge-distance masking with a smoothstep
    // transition. When multiple sides are used (e.g. Top + Left), their
    // shadows naturally overlap at shared corners, which is correct.
    float edge = 2.0;

    if (side_type < 1.5) {
        // Top: visible above the top edge.
        return 1.0 - smoothstep(-half_size.y - edge, -half_size.y + edge, centered.y);
    }
    if (side_type < 2.5) {
        // Right: visible beyond the right edge.
        return smoothstep(half_size.x - edge, half_size.x + edge, centered.x);
    }
    if (side_type < 3.5) {
        // Bottom: visible below the bottom edge.
        return smoothstep(half_size.y - edge, half_size.y + edge, centered.y);
    }
    if (side_type < 4.5) {
        // Left: visible beyond the left edge.
        return 1.0 - smoothstep(-half_size.x - edge, -half_size.x + edge, centered.x);
    }
    if (side_type < 5.5) {
        // Vertical: top + bottom.
        float dist_y = abs(centered.y) - half_size.y;
        float dist_x = abs(centered.x) - half_size.x;
        return smoothstep(-edge, edge, dist_y - dist_x);
    }
    if (side_type < 6.5) {
        // Horizontal: left + right.
        float dist_x = abs(centered.x) - half_size.x;
        float dist_y = abs(centered.y) - half_size.y;
        return smoothstep(-edge, edge, dist_x - dist_y);
    }
    if (side_type < 7.5) {
        // Range: angular mask (handled below).
    } else if (side_type < 8.5) {
        // TopLeft: closer to the top or left edge than to the bottom or right.
        float dist_top = -(centered.y + half_size.y);
        float dist_left = -(centered.x + half_size.x);
        float dist_bottom = centered.y - half_size.y;
        float dist_right = centered.x - half_size.x;
        float near = max(dist_top, dist_left);
        float far = max(dist_bottom, dist_right);
        return smoothstep(-edge, edge, near - far);
    } else if (side_type < 9.5) {
        // TopRight
        float dist_top = -(centered.y + half_size.y);
        float dist_right = centered.x - half_size.x;
        float dist_bottom = centered.y - half_size.y;
        float dist_left = -(centered.x + half_size.x);
        float near = max(dist_top, dist_right);
        float far = max(dist_bottom, dist_left);
        return smoothstep(-edge, edge, near - far);
    } else if (side_type < 10.5) {
        // BottomRight
        float dist_bottom = centered.y - half_size.y;
        float dist_right = centered.x - half_size.x;
        float dist_top = -(centered.y + half_size.y);
        float dist_left = -(centered.x + half_size.x);
        float near = max(dist_bottom, dist_right);
        float far = max(dist_top, dist_left);
        return smoothstep(-edge, edge, near - far);
    } else if (side_type < 11.5) {
        // BottomLeft
        float dist_bottom = centered.y - half_size.y;
        float dist_left = -(centered.x + half_size.x);
        float dist_top = -(centered.y + half_size.y);
        float dist_right = centered.x - half_size.x;
        float near = max(dist_bottom, dist_left);
        float far = max(dist_top, dist_right);
        return smoothstep(-edge, edge, near - far);
    }
    // Range: angular mask. MSL atan2(y, x) takes the same argument order as
    // WGSL atan2(y, x).
    float angle = atan2(centered.y, centered.x);
    float a = angle - angle_start;
    float range = angle_end - angle_start;
    float two_pi = 6.2831853;
    a = a - floor(a / two_pi) * two_pi;
    float r = range - floor(range / two_pi) * two_pi;
    if (a <= r) {
        return 1.0;
    }
    return 0.0;
}

/// Converts an sRGB input color to the correct color space, applies
/// premultiplied alpha with the given alpha factor, and converts back to sRGB
/// if needed.
///
/// `surface_is_srgb` is threaded in as a parameter because an MSL helper
/// cannot see a buffer-bound argument.
static float4 finalize_color(float4 color, float alpha, float surface_is_srgb) {
    float4 sc;
    if (surface_is_srgb >= 1.5) {
        sc = color;
    } else {
        sc = srgb_color_to_linear(color);
    }
    float4 result = float4(sc.rgb * sc.a, sc.a) * alpha;
    if (surface_is_srgb < 0.5) {
        float a = result.a;
        if (a > 0.00001) {
            float3 unpremul = result.rgb / a;
            float3 srgb_rgb = float3(linear_to_srgb(unpremul.r),
                                     linear_to_srgb(unpremul.g),
                                     linear_to_srgb(unpremul.b));
            result = float4(srgb_rgb * a, a);
        }
    }
    return result;
}

// NOTE: the WGSL parameter is named `in`, which is a reserved storage
// qualifier in MSL function parameter position, so the varying is named
// `varyings` here.
fragment float4 fs_main(RectFragmentIn varyings [[stage_in]],
                        constant Viewport& viewport [[buffer(0)]]) {
    // --- Shadow early path ---
    if (varyings.shadow_color.a > 0.0) {
        float2 shadow_offset = varyings.shadow_params.xy;
        float shadow_blur = varyings.shadow_params.z;
        float shadow_spread = varyings.shadow_params.w;
        bool is_inset = varyings.shadow_flags.x > 0.5;

        // The rect_size IS the expanded quad size; the original rect size is
        // stored implicitly. The quad was expanded per-axis by
        // (blur + |spread| + |offset|) on each side.
        float expand_x = shadow_blur + abs(shadow_spread) + abs(shadow_offset.x);
        float expand_y = shadow_blur + abs(shadow_spread) + abs(shadow_offset.y);
        float2 orig_size = varyings.rect_size - float2(expand_x * 2.0, expand_y * 2.0);
        float2 orig_half = orig_size * 0.5;

        // local_pos is relative to the expanded quad; center it on the
        // original rect.
        float2 centered = varyings.local_pos - varyings.rect_size * 0.5;

        // Compute SDF at the shadow-offset position with spread-adjusted
        // half-size and radii.
        float2 shadow_half = orig_half + float2(shadow_spread);
        float2 shadow_p = centered - shadow_offset;
        float4 spread_radii = float4(
            max(varyings.border_radius.x + shadow_spread, 0.0),
            max(varyings.border_radius.y + shadow_spread, 0.0),
            max(varyings.border_radius.z + shadow_spread, 0.0),
            max(varyings.border_radius.w + shadow_spread, 0.0));
        float shadow_d = sdf_rounded_rect(shadow_p, shadow_half, spread_radii);

        float sa = shadow_alpha(shadow_d, shadow_blur, is_inset);

        // Apply side mask.
        float side_type = varyings.shadow_flags.y;
        float angle_start = varyings.shadow_flags.z;
        float angle_end = varyings.shadow_flags.w;
        float side_mask = shadow_side_mask(centered, orig_half, varyings.border_radius,
                                           side_type, angle_start, angle_end);

        // For inset shadows, clip to the original rect bounds.
        float ca = clip_alpha(varyings.pixel_pos, varyings.clip_rect, varyings.clip_border_radius);
        if (is_inset) {
            float orig_d = sdf_rounded_rect(centered, orig_half, varyings.border_radius);
            float inside = 1.0 - smoothstep(-0.5, 0.5, orig_d);
            return finalize_color(varyings.shadow_color, sa * inside * ca * side_mask,
                                  viewport.surface_is_srgb);
        } else {
            return finalize_color(varyings.shadow_color, sa * ca * side_mask,
                                  viewport.surface_is_srgb);
        }
    }

    // The quad may have been expanded by outline_width on each side.
    float ow_top = varyings.outline_width.x;
    float ow_right = varyings.outline_width.y;
    float ow_bottom = varyings.outline_width.z;
    float ow_left = varyings.outline_width.w;
    bool has_outline = (ow_top + ow_right + ow_bottom + ow_left) > 0.0;

    // Original rect size (before outline expansion).
    float2 orig_size = float2(
        varyings.rect_size.x - ow_left - ow_right,
        varyings.rect_size.y - ow_top - ow_bottom);
    float2 orig_half = orig_size * 0.5;

    // local_pos relative to the original rect.
    float2 orig_local = varyings.local_pos - float2(ow_left, ow_top);
    float2 orig_centered = orig_local - orig_half;

    // SDF for the original rect outer edge.
    float d = sdf_rounded_rect(orig_centered, orig_half, varyings.border_radius);
    // Anti-aliasing with a small margin to ensure no gap.
    float outer_alpha = 1.0 - smoothstep(-0.5, 0.5, d);

    // Anti-aliased clip.
    float ca = clip_alpha(varyings.pixel_pos, varyings.clip_rect, varyings.clip_border_radius);

    // On Android (surface_is_srgb >= 1.5), skip sRGB conversion entirely.
    float4 fill_color;
    float4 stroke_color;
    float4 ol_color;
    if (viewport.surface_is_srgb >= 1.5) {
        fill_color = varyings.color;
        stroke_color = varyings.border_color;
        ol_color = varyings.outline_color;
    } else {
        fill_color = srgb_color_to_linear(varyings.color);
        stroke_color = srgb_color_to_linear(varyings.border_color);
        ol_color = srgb_color_to_linear(varyings.outline_color);
    }

    // Compute outline ring if needed. Outline outer edge: expanded rect with
    // expanded radii. A corner only grows rounder here when the box itself
    // already has a radius there -- offsetting a sharp corner outward by a
    // uniform thickness is still sharp. The zero branch of each select below
    // keeps square corners square no matter how thick the outline is.
    float4 outline_premul = float4(0.0, 0.0, 0.0, 0.0);
    if (has_outline) {
        float2 expanded_half = varyings.rect_size * 0.5;
        float2 expanded_centered = varyings.local_pos - expanded_half;
        float4 outline_radii = float4(
            (varyings.border_radius.x > 0.0) ? varyings.border_radius.x + max(ow_top, ow_left) : 0.0,
            (varyings.border_radius.y > 0.0) ? varyings.border_radius.y + max(ow_top, ow_right) : 0.0,
            (varyings.border_radius.z > 0.0) ? varyings.border_radius.z + max(ow_bottom, ow_right) : 0.0,
            (varyings.border_radius.w > 0.0) ? varyings.border_radius.w + max(ow_bottom, ow_left) : 0.0);
        float outline_d = sdf_rounded_rect(expanded_centered, expanded_half, outline_radii);
        float outline_outer_alpha = 1.0 - smoothstep(-0.5, 0.5, outline_d);
        float outline_ring_alpha = clamp(outline_outer_alpha - outer_alpha, 0.0, 1.0);
        outline_premul = float4(ol_color.rgb * ol_color.a, ol_color.a) * outline_ring_alpha;
    }

    // border_width: (top, right, bottom, left)
    bool has_border = (varyings.border_width.x + varyings.border_width.y +
                       varyings.border_width.z + varyings.border_width.w) > 0.0;

    if (has_border) {
        float bw_top = varyings.border_width.x;
        float bw_right = varyings.border_width.y;
        float bw_bottom = varyings.border_width.z;
        float bw_left = varyings.border_width.w;

        // Inner rect center offset and half-size.
        float2 inner_offset = float2((bw_left - bw_right) * 0.5, (bw_top - bw_bottom) * 0.5);
        float2 inner_half = float2(
            orig_half.x - (bw_left + bw_right) * 0.5,
            orig_half.y - (bw_top + bw_bottom) * 0.5);

        // Inner radii: shrink each corner radius by the max of its two
        // adjacent border widths.
        float4 inner_radii = float4(
            max(varyings.border_radius.x - max(bw_top, bw_left), 0.0),     // top-left
            max(varyings.border_radius.y - max(bw_top, bw_right), 0.0),    // top-right
            max(varyings.border_radius.z - max(bw_bottom, bw_right), 0.0), // bottom-right
            max(varyings.border_radius.w - max(bw_bottom, bw_left), 0.0)); // bottom-left

        float2 inner_p = orig_centered - inner_offset;
        float inner_d = sdf_rounded_rect(inner_p, max(inner_half, float2(0.0, 0.0)), inner_radii);
        float inner_alpha = 1.0 - smoothstep(-0.5, 0.5, inner_d);

        // Border ring: outer minus inner.
        float border_alpha = clamp(outer_alpha - inner_alpha, 0.0, 1.0);
        float4 bc = stroke_color;
        float4 fc = fill_color;

        // Ensure fill and border combined do not exceed outer_alpha.
        float final_inner_alpha = min(inner_alpha, outer_alpha);

        float4 fill_premul = float4(fc.rgb * fc.a, fc.a) * final_inner_alpha;
        float4 border_premul = float4(bc.rgb * bc.a, bc.a) * border_alpha;

        // Combine layers. Since they are disjoint, addition is correct.
        float4 combined = (outline_premul + border_premul + fill_premul) * ca;

        if (viewport.surface_is_srgb < 0.5) {
            float a = combined.a;
            if (a > 0.00001) {
                float3 unpremul = combined.rgb / a;
                float3 srgb_rgb = float3(linear_to_srgb(unpremul.r),
                                         linear_to_srgb(unpremul.g),
                                         linear_to_srgb(unpremul.b));
                return float4(srgb_rgb * a, a);
            }
        }
        return combined;
    } else {
        float4 fill_premul = float4(fill_color.rgb * fill_color.a, fill_color.a) * outer_alpha;
        float4 combined = (outline_premul + fill_premul) * ca;

        if (viewport.surface_is_srgb < 0.5) {
            float a = combined.a;
            if (a > 0.000001) {
                float3 unpremul = combined.rgb / a;
                float3 srgb_rgb = float3(linear_to_srgb(unpremul.r),
                                         linear_to_srgb(unpremul.g),
                                         linear_to_srgb(unpremul.b));
                return float4(srgb_rgb * a, a);
            }
        }
        return combined;
    }
}
