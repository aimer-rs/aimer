 struct Viewport {
    size: vec2<f32>,
    surface_is_srgb: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local_pos: vec2<f32>,
    @location(2) rect_size: vec2<f32>,
    @location(3) border_radius: vec4<f32>,
    @location(4) border_width: vec4<f32>,
    @location(5) border_color: vec4<f32>,
    @location(6) outline_width: vec4<f32>,
    @location(7) outline_color: vec4<f32>,
    @location(8) pixel_pos: vec2<f32>,
    @location(9) clip_rect: vec4<f32>,
    @location(10) clip_border_radius: vec4<f32>,
    @location(11) shadow_params: vec4<f32>,
    @location(12) shadow_color: vec4<f32>,
    @location(13) shadow_flags: vec4<f32>,
};

struct RectInstance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_radius: vec4<f32>,
    @location(4) border_width: vec4<f32>,
    @location(5) border_color: vec4<f32>,
    @location(6) outline_width: vec4<f32>,
    @location(7) outline_color: vec4<f32>,
    @location(8) clip_rect: vec4<f32>,
    @location(9) clip_border_radius: vec4<f32>,
    @location(10) shadow_params: vec4<f32>,
    @location(11) shadow_color: vec4<f32>,
    @location(12) shadow_flags: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: RectInstance) -> VertexOutput {
    // Generate a quad from 6 vertices (two triangles)
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );

    let corner = corners[vi];
    let pixel_pos = inst.pos + corner * inst.size;

    // Convert pixel coordinates to NDC: x: [0, width] -> [-1, 1], y: [0, height] -> [1, -1]
    let ndc = vec2<f32>(
        (pixel_pos.x / viewport.size.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / viewport.size.y) * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
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
fn sdf_rounded_rect(p: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>) -> f32 {
    // Select the radius for the quadrant the point is in
    var r: f32;
    if p.x < 0.0 {
        if p.y < 0.0 {
            r = radii.x; // top-left
        } else {
            r = radii.w; // bottom-left
        }
    } else {
        if p.y < 0.0 {
            r = radii.y; // top-right
        } else {
            r = radii.z; // bottom-right
        }
    }
    r = min(r, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

/// Gaussian integral (erf-based) for smooth shadow falloff.
fn gaussian_integral(x: f32, sigma: f32) -> f32 {
    let normalized = x / (sigma * 1.4142135); // sqrt(2)
    let t = 1.0 / (1.0 + 0.3275911 * abs(normalized));
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741
               + t * (-1.453152027 + t * 1.061405429))));
    let erf_val = 1.0 - poly * exp(-normalized * normalized);
    return 0.5 + 0.5 * sign(normalized) * erf_val;
}

/// Compute shadow alpha from SDF distance and blur radius.
/// Per CSS spec, the blur radius equals ~2σ of the Gaussian, so σ = blur / 2.
fn shadow_alpha(sdf_dist: f32, blur: f32, is_inset: bool) -> f32 {
    let sigma = blur * 0.5;
    if sigma < 0.001 {
        // Hard edge
        if is_inset {
            return select(1.0, 0.0, sdf_dist < 0.0);
        } else {
            return select(0.0, 1.0, sdf_dist < 0.0);
        }
    }
    let a = gaussian_integral(-sdf_dist, sigma);
    if is_inset {
        return 1.0 - a;
    }
    return a;
}

/// Compute anti-aliased clip alpha from a clip rect in pixel coordinates.
/// clip_rect = (x, y, width, height). If width <= 0, clipping is disabled (returns 1.0).
fn clip_alpha(pixel_pos: vec2<f32>, clip_rect: vec4<f32>, clip_radii: vec4<f32>) -> f32 {
    if clip_rect.z < 0.0 {
        return 1.0;
    }

    if clip_rect.z <= 0.01 || clip_rect.w <= 0.01 {
        return 0.0;
    }

    if clip_radii.x > 0.0 || clip_radii.y > 0.0 || clip_radii.z > 0.0 || clip_radii.w > 0.0 {
        // Rounded clip: use SDF
        let clip_center = clip_rect.xy + clip_rect.zw * 0.5;
        let clip_half = clip_rect.zw * 0.5;
        let p = pixel_pos - clip_center;
        let d = sdf_rounded_rect(p, clip_half, clip_radii);
        return 1.0 - smoothstep(-0.5, 0.5, d);
    }

    let clip_min = clip_rect.xy;
    let clip_max = clip_rect.xy + clip_rect.zw;

    // Signed distance from each edge (positive = inside)
    let d_left   = pixel_pos.x - clip_min.x;
    let d_right  = clip_max.x - pixel_pos.x;
    let d_top    = pixel_pos.y - clip_min.y;
    let d_bottom = clip_max.y - pixel_pos.y;

    // Smoothstep each edge for anti-aliasing (1px transition centered on the
    // boundary). Centering on the edge (`-0.5..0.5`) keeps the clipped region
    // fully opaque right up to the boundary; ramping from `0..1` instead would
    // erode ~1px of content just inside every edge (see the rounded-clip branch
    // above, which already centers with `smoothstep(-0.5, 0.5, d)`).
    let a_left   = smoothstep(-0.5, 0.5, d_left);
    let a_right  = smoothstep(-0.5, 0.5, d_right);
    let a_top    = smoothstep(-0.5, 0.5, d_top);
    let a_bottom = smoothstep(-0.5, 0.5, d_bottom);

    return a_left * a_right * a_top * a_bottom;
}

// Convert a single sRGB channel to linear space.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

// Convert a single linear channel back to sRGB.
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}


// Convert an sRGB color (with alpha) to linear space. Alpha is kept as-is.
fn srgb_color_to_linear(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(
        srgb_to_linear(c.r),
        srgb_to_linear(c.g),
        srgb_to_linear(c.b),
        c.a,
    );
}

// Convert a linear color (with alpha) back to sRGB space.
fn linear_color_to_srgb(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(
        linear_to_srgb(c.r),
        linear_to_srgb(c.g),
        linear_to_srgb(c.b),
        c.a,
    );
}

/// Computes a side mask based on the fragment's position relative to the rect center.
/// side_type: 0=All, 1=Top, 2=Right, 3=Bottom, 4=Left, 5=Vertical, 6=Horizontal, 7=Range,
///            8=TopLeft, 9=TopRight, 10=BottomRight, 11=BottomLeft
/// centered: fragment position relative to rect center
/// half_size: half-size of the original (unexpanded) rect
/// border_radius: corner radii (top-left, top-right, bottom-right, bottom-left)
/// For Range, angle_start and angle_end are in radians.
fn shadow_side_mask(centered: vec2<f32>, half_size: vec2<f32>, border_radius: vec4<f32>, side_type: f32, angle_start: f32, angle_end: f32) -> f32 {
    if side_type < 0.5 {
        // All
        return 1.0;
    }

    // For cardinal sides, use edge-distance masking with smoothstep transition.
    // When multiple sides are used (e.g., Top + Left), their shadows naturally
    // overlap at shared corners, which is the correct behavior.
    let edge = 2.0;

    if side_type < 1.5 {
        // Top: visible above the top edge
        return 1.0 - smoothstep(-half_size.y - edge, -half_size.y + edge, centered.y);
    }
    if side_type < 2.5 {
        // Right: visible beyond the right edge
        return smoothstep(half_size.x - edge, half_size.x + edge, centered.x);
    }
    if side_type < 3.5 {
        // Bottom: visible below the bottom edge
        return smoothstep(half_size.y - edge, half_size.y + edge, centered.y);
    }
    if side_type < 4.5 {
        // Left: visible beyond the left edge
        return 1.0 - smoothstep(-half_size.x - edge, -half_size.x + edge, centered.x);
    }
    if side_type < 5.5 {
        // Vertical: top + bottom
        let dist_y = abs(centered.y) - half_size.y;
        let dist_x = abs(centered.x) - half_size.x;
        return smoothstep(-edge, edge, dist_y - dist_x);
    }
    if side_type < 6.5 {
        // Horizontal: left + right
        let dist_x = abs(centered.x) - half_size.x;
        let dist_y = abs(centered.y) - half_size.y;
        return smoothstep(-edge, edge, dist_x - dist_y);
    }
    if side_type < 7.5 {
        // Range: angular mask (handled below)
    } else if side_type < 8.5 {
        // TopLeft: visible where fragment is closer to top or left edge than to bottom or right
        let dist_top = -(centered.y + half_size.y);
        let dist_left = -(centered.x + half_size.x);
        let dist_bottom = centered.y - half_size.y;
        let dist_right = centered.x - half_size.x;
        let near = max(dist_top, dist_left);
        let far = max(dist_bottom, dist_right);
        return smoothstep(-edge, edge, near - far);
    } else if side_type < 9.5 {
        // TopRight: visible where fragment is closer to top or right edge than to bottom or left
        let dist_top = -(centered.y + half_size.y);
        let dist_right = centered.x - half_size.x;
        let dist_bottom = centered.y - half_size.y;
        let dist_left = -(centered.x + half_size.x);
        let near = max(dist_top, dist_right);
        let far = max(dist_bottom, dist_left);
        return smoothstep(-edge, edge, near - far);
    } else if side_type < 10.5 {
        // BottomRight: visible where fragment is closer to bottom or right edge than to top or left
        let dist_bottom = centered.y - half_size.y;
        let dist_right = centered.x - half_size.x;
        let dist_top = -(centered.y + half_size.y);
        let dist_left = -(centered.x + half_size.x);
        let near = max(dist_bottom, dist_right);
        let far = max(dist_top, dist_left);
        return smoothstep(-edge, edge, near - far);
    } else if side_type < 11.5 {
        // BottomLeft: visible where fragment is closer to bottom or left edge than to top or right
        let dist_bottom = centered.y - half_size.y;
        let dist_left = -(centered.x + half_size.x);
        let dist_top = -(centered.y + half_size.y);
        let dist_right = centered.x - half_size.x;
        let near = max(dist_bottom, dist_left);
        let far = max(dist_top, dist_right);
        return smoothstep(-edge, edge, near - far);
    }
    // Range: angular mask
    let angle = atan2(centered.y, centered.x);
    var a = angle - angle_start;
    let range = angle_end - angle_start;
    let two_pi = 6.2831853;
    a = a - floor(a / two_pi) * two_pi;
    let r = range - floor(range / two_pi) * two_pi;
    if a <= r {
        return 1.0;
    }
    return 0.0;
}

/// Converts an sRGB input color to the correct color space, applies premultiplied alpha
/// with the given alpha factor, and converts back to sRGB if needed.
fn finalize_color(color: vec4<f32>, alpha: f32) -> vec4<f32> {
    var sc: vec4<f32>;
    if viewport.surface_is_srgb >= 1.5 {
        sc = color;
    } else {
        sc = srgb_color_to_linear(color);
    }
    var result = vec4<f32>(sc.rgb * sc.a, sc.a) * alpha;
    if viewport.surface_is_srgb < 0.5 {
        let a = result.a;
        if a > 0.00001 {
            let unpremul = result.rgb / a;
            let srgb_rgb = vec3<f32>(linear_to_srgb(unpremul.r), linear_to_srgb(unpremul.g), linear_to_srgb(unpremul.b));
            result = vec4<f32>(srgb_rgb * a, a);
        }
    }
    return result;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // --- Shadow early path ---
    if in.shadow_color.a > 0.0 {
        let shadow_offset = in.shadow_params.xy;
        let shadow_blur = in.shadow_params.z;
        let shadow_spread = in.shadow_params.w;
        let is_inset = in.shadow_flags.x > 0.5;

        // The rect_size IS the expanded quad size; the original rect size is stored
        // implicitly. We need to recover the original rect half-size.
        // The quad was expanded per-axis by (blur + |spread| + |offset|) on each side.
        let expand_x = shadow_blur + abs(shadow_spread) + abs(shadow_offset.x);
        let expand_y = shadow_blur + abs(shadow_spread) + abs(shadow_offset.y);
        let orig_size = in.rect_size - vec2<f32>(expand_x * 2.0, expand_y * 2.0);
        let orig_half = orig_size * 0.5;

        // local_pos is relative to the expanded quad; center it on the original rect
        let centered = in.local_pos - in.rect_size * 0.5;

        // Compute SDF at the shadow-offset position with spread-adjusted half-size and radii
        let shadow_half = orig_half + vec2<f32>(shadow_spread);
        let shadow_p = centered - shadow_offset;
        let spread_radii = vec4<f32>(
            max(in.border_radius.x + shadow_spread, 0.0),
            max(in.border_radius.y + shadow_spread, 0.0),
            max(in.border_radius.z + shadow_spread, 0.0),
            max(in.border_radius.w + shadow_spread, 0.0),
        );
        let shadow_d = sdf_rounded_rect(shadow_p, shadow_half, spread_radii);

        let sa = shadow_alpha(shadow_d, shadow_blur, is_inset);

        // Apply side mask
        let side_type = in.shadow_flags.y;
        let angle_start = in.shadow_flags.z;
        let angle_end = in.shadow_flags.w;
        let side_mask = shadow_side_mask(centered, orig_half, in.border_radius, side_type, angle_start, angle_end);

        // For inset shadows, clip to the original rect bounds
        let ca = clip_alpha(in.pixel_pos, in.clip_rect, in.clip_border_radius);
        if is_inset {
            let orig_d = sdf_rounded_rect(centered, orig_half, in.border_radius);
            let inside = 1.0 - smoothstep(-0.5, 0.5, orig_d);
            return finalize_color(in.shadow_color, sa * inside * ca * side_mask);
        } else {
            return finalize_color(in.shadow_color, sa * ca * side_mask);
        }
    }

    // The quad may have been expanded by outline_width on each side.
    let ow_top = in.outline_width.x;
    let ow_right = in.outline_width.y;
    let ow_bottom = in.outline_width.z;
    let ow_left = in.outline_width.w;
    let has_outline = (ow_top + ow_right + ow_bottom + ow_left) > 0.0;

    // Original rect size (before outline expansion)
    let orig_size = vec2<f32>(
        in.rect_size.x - ow_left - ow_right,
        in.rect_size.y - ow_top - ow_bottom,
    );
    let orig_half = orig_size * 0.5;

    // local_pos relative to the original rect
    let orig_local = in.local_pos - vec2<f32>(ow_left, ow_top);
    let orig_centered = orig_local - orig_half;

    // SDF for the original rect outer edge
    let d = sdf_rounded_rect(orig_centered, orig_half, in.border_radius);
    // Anti-aliasing with a small margin to ensure no gap
    let outer_alpha = 1.0 - smoothstep(-0.5, 0.5, d);

    // Anti-aliased clip
    let ca = clip_alpha(in.pixel_pos, in.clip_rect, in.clip_border_radius);

    // On Android (surface_is_srgb >= 1.5), skip sRGB conversion entirely.
    var fill_color: vec4<f32>;
    var stroke_color: vec4<f32>;
    var ol_color: vec4<f32>;
    if viewport.surface_is_srgb >= 1.5 {
        fill_color = in.color;
        stroke_color = in.border_color;
        ol_color = in.outline_color;
    } else {
        fill_color = srgb_color_to_linear(in.color);
        stroke_color = srgb_color_to_linear(in.border_color);
        ol_color = srgb_color_to_linear(in.outline_color);
    }


    // Compute outline ring if needed
    var outline_premul = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    if has_outline {
        // Outline outer edge: expanded rect with expanded radii.
        //
        // A corner only grows rounder here when the box itself already has a
        // radius there — offsetting a *sharp* corner outward by a uniform
        // thickness is still sharp, it doesn't gain a `max(ow_top, ow_left)`
        // radius of its own. `select`'s zero branch keeps square corners
        // square no matter how thick the outline is.
        let expanded_half = in.rect_size * 0.5;
        let expanded_centered = in.local_pos - expanded_half;
        let outline_radii = vec4<f32>(
            select(0.0, in.border_radius.x + max(ow_top, ow_left), in.border_radius.x > 0.0),
            select(0.0, in.border_radius.y + max(ow_top, ow_right), in.border_radius.y > 0.0),
            select(0.0, in.border_radius.z + max(ow_bottom, ow_right), in.border_radius.z > 0.0),
            select(0.0, in.border_radius.w + max(ow_bottom, ow_left), in.border_radius.w > 0.0),
        );
        let outline_d = sdf_rounded_rect(expanded_centered, expanded_half, outline_radii);
        let outline_outer_alpha = 1.0 - smoothstep(-0.5, 0.5, outline_d);
        let outline_ring_alpha = clamp(outline_outer_alpha - outer_alpha, 0.0, 1.0);
        outline_premul = vec4<f32>(ol_color.rgb * ol_color.a, ol_color.a) * outline_ring_alpha;
    }

    // border_width: (top, right, bottom, left)
    let has_border = (in.border_width.x + in.border_width.y + in.border_width.z + in.border_width.w) > 0.0;

    if has_border {
        let bw_top = in.border_width.x;
        let bw_right = in.border_width.y;
        let bw_bottom = in.border_width.z;
        let bw_left = in.border_width.w;

        // Inner rect center offset and half-size
        let inner_offset = vec2<f32>((bw_left - bw_right) * 0.5, (bw_top - bw_bottom) * 0.5);
        let inner_half = vec2<f32>(
            orig_half.x - (bw_left + bw_right) * 0.5,
            orig_half.y - (bw_top + bw_bottom) * 0.5,
        );

        // Inner radii: shrink each corner radius by the max of its two adjacent border widths
        let inner_radii = vec4<f32>(
            max(in.border_radius.x - max(bw_top, bw_left), 0.0),     // top-left
            max(in.border_radius.y - max(bw_top, bw_right), 0.0),    // top-right
            max(in.border_radius.z - max(bw_bottom, bw_right), 0.0), // bottom-right
            max(in.border_radius.w - max(bw_bottom, bw_left), 0.0),  // bottom-left
        );

        let inner_p = orig_centered - inner_offset;
        let inner_d = sdf_rounded_rect(inner_p, max(inner_half, vec2<f32>(0.0, 0.0)), inner_radii);
        let inner_alpha = 1.0 - smoothstep(-0.5, 0.5, inner_d);

        // Border ring: outer minus inner
        let border_alpha = clamp(outer_alpha - inner_alpha, 0.0, 1.0);
        let bc = stroke_color;
        let fc = fill_color;
        
        // Ensure fill and border combined do not exceed outer_alpha
        let final_inner_alpha = min(inner_alpha, outer_alpha);
        
        let fill_premul = vec4<f32>(fc.rgb * fc.a, fc.a) * final_inner_alpha;
        let border_premul = vec4<f32>(bc.rgb * bc.a, bc.a) * border_alpha;
        
        // Combine layers. Since they are disjoint, addition is correct.
        let combined = (outline_premul + border_premul + fill_premul) * ca;
        
        if viewport.surface_is_srgb < 0.5 {
            let a = combined.a;
            if a > 0.00001 {
                let unpremul = combined.rgb / a;
                let srgb_rgb = vec3<f32>(linear_to_srgb(unpremul.r), linear_to_srgb(unpremul.g), linear_to_srgb(unpremul.b));
                return vec4<f32>(srgb_rgb * a, a);
            }
        }
        return combined;
    } else {
        let fill_premul = vec4<f32>(fill_color.rgb * fill_color.a, fill_color.a) * outer_alpha;
        let combined = (outline_premul + fill_premul) * ca;

        if viewport.surface_is_srgb < 0.5 {
            let a = combined.a;
            if a > 0.000001 {
                let unpremul = combined.rgb / a;
                let srgb_rgb = vec3<f32>(linear_to_srgb(unpremul.r), linear_to_srgb(unpremul.g), linear_to_srgb(unpremul.b));
                return vec4<f32>(srgb_rgb * a, a);
            }
        }
        return combined;
    }
}
