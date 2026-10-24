// Hand-authored HLSL for the native Direct3D 12 backend.
Texture2D<float4> source_texture : register(t0, space0);

struct CompositeVaryings {
    float4 position : SV_Position;
};

CompositeVaryings vs_main(uint vertex_id : SV_VertexID) {
    static const float2 corners[6] = {
        float2(-1.0, 1.0), float2(1.0, 1.0), float2(-1.0, -1.0),
        float2(1.0, 1.0), float2(1.0, -1.0), float2(-1.0, -1.0)
    };
    CompositeVaryings output;
    output.position = float4(corners[vertex_id], 0.0, 1.0);
    return output;
}

float4 fs_main(CompositeVaryings input) : SV_Target0 {
    return source_texture.Load(int3(int2(input.position.xy), 0));
}
