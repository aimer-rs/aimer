// Hand-authored HLSL for the native SVG path renderer.
struct SvgVertexIn {
    float2 position : TEXCOORD0;
    float4 transform_x : TEXCOORD1;
    float4 transform_y : TEXCOORD2;
    float4 color : TEXCOORD3;
    float4 clip_rect : TEXCOORD4;
    float4 clip_border_radius : TEXCOORD5;
    float4 viewport : TEXCOORD6;
    float coverage : TEXCOORD7;
};
struct SvgVaryings {
    float4 position : SV_Position;
    float4 color : TEXCOORD0;
    float2 pixel_pos : TEXCOORD1;
    float4 clip_rect : TEXCOORD2;
    float4 clip_border_radius : TEXCOORD3;
    float surface_is_srgb : TEXCOORD4;
    float coverage : TEXCOORD5;
};
SvgVaryings vs_main(SvgVertexIn input) {
    float3 local = float3(input.position, 1.0);
    float2 pixel = float2(dot(local, input.transform_x.xyz),
                         dot(local, input.transform_y.xyz));
    SvgVaryings output;
    output.position = float4(pixel.x / input.viewport.x * 2.0 - 1.0,
                             1.0 - pixel.y / input.viewport.y * 2.0, 0.0, 1.0);
    output.color = input.color;
    output.pixel_pos = pixel;
    output.clip_rect = input.clip_rect;
    output.clip_border_radius = input.clip_border_radius;
    output.surface_is_srgb = input.viewport.z;
    output.coverage = input.coverage;
    return output;
}
float sdf_rounded_rect(float2 p, float2 half_size, float4 radii) {
    float r = p.x < 0.0 ? (p.y < 0.0 ? radii.x : radii.w)
                        : (p.y < 0.0 ? radii.y : radii.z);
    r = min(r, min(half_size.x, half_size.y));
    float2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
float srgb_to_linear(float c) {
    return c <= 0.04045 ? c / 12.92 : pow(max((c + 0.055) / 1.055, 0.0), 2.4);
}
float linear_to_srgb(float c) {
    return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(max(c, 0.0), 1.0 / 2.4) - 0.055;
}
float4 fs_main(SvgVaryings input) : SV_Target0 {
    float clip = 1.0;
    if (input.clip_rect.z > 0.0) {
        float2 half_size = input.clip_rect.zw * 0.5;
        float2 center = input.clip_rect.xy + half_size;
        float distance = sdf_rounded_rect(input.pixel_pos - center, half_size,
                                           input.clip_border_radius);
        clip = 1.0 - smoothstep(-0.5, 0.5, distance);
    }
    float4 color = input.color;
    if (input.surface_is_srgb < 1.5) {
        color.rgb = float3(srgb_to_linear(color.r), srgb_to_linear(color.g),
                           srgb_to_linear(color.b));
    }
    float4 result = float4(color.rgb * color.a, color.a) * (clip * input.coverage);
    if (input.surface_is_srgb < 0.5 && result.a > 0.000001) {
        float3 straight = result.rgb / result.a;
        result.rgb = float3(linear_to_srgb(straight.r), linear_to_srgb(straight.g),
                            linear_to_srgb(straight.b)) * result.a;
    }
    return result;
}
