// Hand-authored HLSL for RGBA color glyphs.
struct TextViewport { float2 resolution; float surface_is_srgb; float _pad; };
ConstantBuffer<TextViewport> viewport : register(b0, space0);
Texture2D<float4> t_atlas : register(t1, space0);
SamplerState s_atlas : register(s2, space0);
struct ColorTextVertexIn {
    float2 pos : TEXCOORD0; float2 size : TEXCOORD1; float4 uv_rect : TEXCOORD2;
    float4 color : TEXCOORD3; float4 clip_rect : TEXCOORD4;
    float4 clip_radius : TEXCOORD5; float skew : TEXCOORD6;
};
struct ColorTextVaryings {
    float4 position : SV_Position; float2 uv : TEXCOORD0; float4 color : TEXCOORD1;
    float2 pixel_pos : TEXCOORD2; float4 clip_rect : TEXCOORD3;
    float4 clip_radius : TEXCOORD4;
};
ColorTextVaryings vs_main(uint vertex_id : SV_VertexID, ColorTextVertexIn inst) {
    static const float2 corners[6] = {float2(0,0),float2(1,0),float2(0,1),
                                      float2(1,0),float2(1,1),float2(0,1)};
    float2 corner = corners[vertex_id];
    float2 pixel = inst.pos + corner * inst.size;
    pixel.x += inst.skew * inst.size.y * (1.0 - corner.y);
    ColorTextVaryings output;
    output.position = float4(pixel.x / viewport.resolution.x * 2.0 - 1.0,
                             1.0 - pixel.y / viewport.resolution.y * 2.0, 0.0, 1.0);
    // uv_rect stores (u_min, v_min, u_max, v_max), not offset and extent.
    output.uv = lerp(inst.uv_rect.xy, inst.uv_rect.zw, corner);
    output.color = inst.color; output.pixel_pos = pixel;
    output.clip_rect = inst.clip_rect; output.clip_radius = inst.clip_radius;
    return output;
}
float sdf_rounded_rect(float2 p, float2 half_size, float4 radii) {
    float r = p.x < 0.0 ? (p.y < 0.0 ? radii.x : radii.w)
                        : (p.y < 0.0 ? radii.y : radii.z);
    r = min(r, min(half_size.x, half_size.y));
    float2 q = abs(p) - half_size + r;
    return length(max(q,0.0)) + min(max(q.x,q.y),0.0) - r;
}
float clip_alpha(float2 pixel, float4 rect, float4 radii) {
    float coverage=1.0;
    if(rect.z>=0.0){
        if(rect.z<=0.01||rect.w<=0.01) coverage=0.0;
        else if(any(radii>0.0)){float2 h=rect.zw*0.5;coverage=1.0-smoothstep(-0.5,0.5,
            sdf_rounded_rect(pixel-rect.xy-h,h,radii));}
        else {float2 hi=rect.xy+rect.zw;
            coverage=smoothstep(-0.5,0.5,pixel.x-rect.x)*smoothstep(-0.5,0.5,hi.x-pixel.x)
                *smoothstep(-0.5,0.5,pixel.y-rect.y)*smoothstep(-0.5,0.5,hi.y-pixel.y);}
    }
    return coverage;
}
float srgb_to_linear(float c){return c<=0.04045?c/12.92:pow(max((c+0.055)/1.055,0.0),2.4);}
float linear_to_srgb(float c){return c<=0.0031308?c*12.92:1.055*pow(max(c,0.0),1.0/2.4)-0.055;}
float4 fs_main(ColorTextVaryings input) : SV_Target0 {
    float4 texel=t_atlas.SampleLevel(s_atlas,input.uv,0.0);
    float alpha=texel.a*input.color.a*clip_alpha(input.pixel_pos,input.clip_rect,input.clip_radius);
    // Color glyph atlases contain finished artwork; layout color controls opacity only.
    float3 rgb=texel.rgb;
    if(viewport.surface_is_srgb<1.5){rgb=float3(srgb_to_linear(rgb.r),srgb_to_linear(rgb.g),srgb_to_linear(rgb.b));}
    float4 result=float4(rgb*alpha,alpha);
    if(viewport.surface_is_srgb<0.5&&alpha>0.00001){result.rgb=float3(linear_to_srgb(rgb.r),linear_to_srgb(rgb.g),linear_to_srgb(rgb.b))*alpha;}
    return result;
}
