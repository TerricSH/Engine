// Minimal static PBR32 mesh shader used by Dx12SceneRenderer.
struct VSInput {
    float3 position : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
};

struct PSInput {
    float4 position : SV_POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
};

cbuffer MVP : register(b0) {
    float4x4 mvp;
};

PSInput VSMain(VSInput input) {
    PSInput output;
    output.position = mul(float4(input.position, 1.0), mvp);
    output.normal = input.normal;
    output.uv = input.uv;
    return output;
}

float4 PSMain(PSInput input) : SV_TARGET {
    // Until material descriptors are implemented, show a stable normal-based
    // fallback instead of reading bytes from an incompatible vertex layout.
    return float4(abs(normalize(input.normal)), 1.0);
}
