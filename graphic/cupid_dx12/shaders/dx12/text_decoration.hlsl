// Hand-authored HLSL for underline, strike-through, dotted, dashed and wavy runs.
struct TextViewport { float2 resolution; float surface_is_srgb; float _pad; };
ConstantBuffer<TextViewport> viewport : register(b0, space0);
struct DecorationVertexIn {
    float2 pos : TEXCOORD0; float2 size : TEXCOORD1; float4 color : TEXCOORD2;
    float4 clip_rect : TEXCOORD3; float4 clip_radius : TEXCOORD4; float4 params : TEXCOORD5;
};
struct DecorationVaryings {
    float4 position : SV_Position; float2 uv : TEXCOORD0; float4 color : TEXCOORD1;
    float2 pixel_pos : TEXCOORD2; float4 clip_rect : TEXCOORD3;
    float4 clip_radius : TEXCOORD4; float4 params : TEXCOORD5; float2 size : TEXCOORD6;
};
DecorationVaryings vs_main(uint vertex_id : SV_VertexID, DecorationVertexIn inst) {
    static const float2 corners[6] = {float2(0,0),float2(1,0),float2(0,1),
                                      float2(1,0),float2(1,1),float2(0,1)};
    float2 corner=corners[vertex_id]; float2 pixel=inst.pos+corner*inst.size;
    DecorationVaryings output;
    output.position=float4(pixel.x/viewport.resolution.x*2.0-1.0,
                           1.0-pixel.y/viewport.resolution.y*2.0,0.0,1.0);
    output.uv=corner; output.color=inst.color; output.pixel_pos=pixel;
    output.clip_rect=inst.clip_rect; output.clip_radius=inst.clip_radius;
    output.params=inst.params; output.size=inst.size; return output;
}
float sdf_rounded_rect(float2 p,float2 h,float4 r){
    float qrad=p.x<0.0?(p.y<0.0?r.x:r.w):(p.y<0.0?r.y:r.z);
    qrad=min(qrad,min(h.x,h.y));float2 q=abs(p)-h+qrad;
    return length(max(q,0.0))+min(max(q.x,q.y),0.0)-qrad;
}
float clip_alpha(float2 p,float4 rect,float4 radii){
    float coverage=1.0;
    if(rect.z>=0.0){
        if(rect.z<=0.01||rect.w<=0.01)coverage=0.0;
        else if(any(radii>0.0)){float2 h=rect.zw*0.5;coverage=1.0-smoothstep(-0.5,0.5,
            sdf_rounded_rect(p-rect.xy-h,h,radii));}
        else {float2 hi=rect.xy+rect.zw;
            coverage=smoothstep(-0.5,0.5,p.x-rect.x)*smoothstep(-0.5,0.5,hi.x-p.x)
                *smoothstep(-0.5,0.5,p.y-rect.y)*smoothstep(-0.5,0.5,hi.y-p.y);}
    }
    return coverage;
}
float srgb_to_linear(float c){return c<=0.04045?c/12.92:pow(max((c+0.055)/1.055,0.0),2.4);}
float linear_to_srgb(float c){return c<=0.0031308?c*12.92:1.055*pow(max(c,0.0),1.0/2.4)-0.055;}
float stroke(float distance,float half_width){return 1.0-smoothstep(-0.5,0.5,distance-half_width);}
float4 fs_main(DecorationVaryings input) : SV_Target0 {
    float style=input.params.x;float thickness=max(input.params.y,0.5);
    float period=max(input.params.z,1.0);float band=max(input.params.w,thickness);
    float px=input.uv.x*input.size.x,py=input.uv.y*input.size.y;
    float center=band*0.5,half_width=thickness*0.5,coverage=0.0;
    if(style<0.5) coverage=stroke(abs(py-center),half_width);
    else if(style<1.5) coverage=max(stroke(abs(py-half_width),half_width),
                                    stroke(abs(py-(band-half_width)),half_width));
    else if(style<2.5){float line_coverage=stroke(abs(py-center),half_width);
        float phase=px-period*floor(px/period);
        coverage=line_coverage*stroke(abs(phase-period*0.5),period*0.25);}
    else if(style<3.5){float line_coverage=stroke(abs(py-center),half_width);
        float phase=px-period*floor(px/period);coverage=line_coverage*step(phase,period*0.6);}
    else {float amplitude=max((band-thickness)*0.5,0.0);
        float wave=center+amplitude*sin(px*6.2831853/period);
        coverage=stroke(abs(py-wave),half_width);}
    coverage*=clip_alpha(input.pixel_pos,input.clip_rect,input.clip_radius);
    float alpha=input.color.a*coverage;float3 rgb=input.color.rgb;
    if(viewport.surface_is_srgb<1.5)rgb=float3(srgb_to_linear(rgb.r),srgb_to_linear(rgb.g),srgb_to_linear(rgb.b));
    float4 result=float4(rgb*alpha,alpha);
    if(viewport.surface_is_srgb<0.5&&alpha>0.00001)result.rgb=float3(linear_to_srgb(rgb.r),linear_to_srgb(rgb.g),linear_to_srgb(rgb.b))*alpha;
    return result;
}
