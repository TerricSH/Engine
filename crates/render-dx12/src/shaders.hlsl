// SceneRenderer forward vertex shader (position + color → MVP → SV_Position)
struct VSInput {
    float3 position : POSITION;
    float4 color : COLOR;
};
struct PSInput {
    float4 position : SV_POSITION;
    float4 color : COLOR;
};
cbuffer MVP : register(b0) {
    float4x4 mvp;
};
PSInput VSMain(VSInput input) {
    PSInput output;
    output.position = mul(float4(input.position, 1.0), mvp);
    output.color = input.color;
    return output;
}
// SceneRenderer forward pixel shader
float4 PSMain(PSInput input) : SV_TARGET {
    return input.color;
}
