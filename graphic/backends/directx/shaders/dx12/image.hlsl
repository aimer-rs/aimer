// Hand-authored HLSL for Cupid's instanced image pipeline.
struct Viewport { float2 size; float surface_is_srgb; float _pad; };
ConstantBuffer<Viewport> viewport : register(b0, space0);
Texture2D<float4> t_diffuse : register(t0, space1);
SamplerState s_diffuse : register(s1, space1);

struct ImageVertexIn {
    float2 pos : TEXCOORD0;
    float2 size : TEXCOORD1;
    float2 uv_offset : TEXCOORD2;
    float2 uv_scale : TEXCOORD3;
    float4 clip_rect : TEXCOORD4;
    float4 clip_border_radius : TEXCOORD5;
    float alpha : TEXCOORD6;
    float source_premultiplied : TEXCOORD7;
};
struct ImageVaryings {
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
    float2 pixel_pos : TEXCOORD1;
    float4 clip_rect : TEXCOORD2;
    float4 clip_border_radius : TEXCOORD3;
    float alpha : TEXCOORD4;
    float source_premultiplied : TEXCOORD5;
};

ImageVaryings vs_main(uint vertex_id : SV_VertexID, ImageVertexIn inst) {
    static const float2 corners[6] = {float2(0,0),float2(1,0),float2(0,1),
                                      float2(1,0),float2(1,1),float2(0,1)};
    float2 corner = corners[vertex_id];
    float2 pixel = inst.pos + corner * inst.size;
    ImageVaryings output;
    output.position = float4(pixel.x / viewport.size.x * 2.0 - 1.0,
                             1.0 - pixel.y / viewport.size.y * 2.0, 0.0, 1.0);
    output.uv = inst.uv_offset + corner * inst.uv_scale;
    output.pixel_pos = pixel;
    output.clip_rect = inst.clip_rect;
    output.clip_border_radius = inst.clip_border_radius;
    output.alpha = inst.alpha;
    output.source_premultiplied = inst.source_premultiplied;
    return output;
}

float rounded_rect_sdf(float2 p, float2 half_size, float4 radii) {
    float r = p.x < 0.0 ? (p.y < 0.0 ? radii.x : radii.w)
                        : (p.y < 0.0 ? radii.y : radii.z);
    r = min(r, min(half_size.x, half_size.y));
    float2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
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
            coverage = smoothstep(-0.5,0.5,pixel.x-rect.x)
                     * smoothstep(-0.5,0.5,high.x-pixel.x)
                     * smoothstep(-0.5,0.5,pixel.y-rect.y)
                     * smoothstep(-0.5,0.5,high.y-pixel.y);
        }
    }
    return coverage;
}
float srgb_to_linear(float c) {
    return c <= 0.04045 ? c / 12.92 : pow(max((c + 0.055) / 1.055, 0.0), 2.4);
}
float linear_to_srgb(float c) {
    return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(max(c, 0.0), 1.0 / 2.4) - 0.055;
}
float4 fs_main(ImageVaryings input) : SV_Target0 {
    float4 sample = t_diffuse.Sample(s_diffuse, input.uv);
    float4 result;
    if (input.source_premultiplied > 0.5) {
        float alpha = sample.a;
        if (viewport.surface_is_srgb >= 1.5 || viewport.surface_is_srgb >= 0.5) {
            result = float4(sample.rgb, alpha);
        } else if (alpha > 0.00001) {
            float3 straight = sample.rgb / alpha;
            result = float4(float3(linear_to_srgb(srgb_to_linear(straight.r)),
                                   linear_to_srgb(srgb_to_linear(straight.g)),
                                   linear_to_srgb(srgb_to_linear(straight.b))) * alpha, alpha);
        } else result = 0.0;
    } else if (viewport.surface_is_srgb >= 1.5) {
        result = float4(sample.rgb * sample.a, sample.a);
    } else {
        float3 linear_rgb = float3(srgb_to_linear(sample.r), srgb_to_linear(sample.g),
                                   srgb_to_linear(sample.b));
        result = float4(linear_rgb * sample.a, sample.a);
        if (viewport.surface_is_srgb < 0.5 && sample.a > 0.00001) {
            float3 encoded = float3(linear_to_srgb(linear_rgb.r), linear_to_srgb(linear_rgb.g),
                                    linear_to_srgb(linear_rgb.b));
            result.rgb = encoded * sample.a;
        }
    }
    return result * (clip_coverage(input.pixel_pos, input.clip_rect,
                                   input.clip_border_radius) * input.alpha);
}
