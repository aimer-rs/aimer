#include <metal_stdlib>

using namespace metal;

constant float2 kCompositeCorners[6] = {
    float2(-1.0, 1.0), float2(1.0, 1.0), float2(-1.0, -1.0),
    float2(1.0, 1.0), float2(1.0, -1.0), float2(-1.0, -1.0),
};

struct CompositeVertexOut {
    float4 position [[position]];
};

vertex CompositeVertexOut vs_main(uint vertex_id [[vertex_id]]) {
    CompositeVertexOut out;
    out.position = float4(kCompositeCorners[vertex_id], 0.0, 1.0);
    return out;
}

fragment float4 fs_main(CompositeVertexOut in [[stage_in]],
                        texture2d<float> source_frame [[texture(0)]]) {
    return source_frame.read(uint2(in.position.xy));
}
