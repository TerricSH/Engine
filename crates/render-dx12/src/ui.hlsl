struct UiVSInput {
    float2 position : POSITION;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
};

struct UiPSInput {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
};

cbuffer UiSettings : register(b0) {
    float2 screen_size;
};

Texture2D ui_texture : register(t0);
SamplerState ui_sampler : register(s0);

UiPSInput UiVSMain(UiVSInput input) {
    UiPSInput output;
    output.position = float4(
        input.position.x / screen_size.x * 2.0 - 1.0,
        1.0 - input.position.y / screen_size.y * 2.0,
        0.0,
        1.0
    );
    output.uv = input.uv;
    output.color = input.color;
    return output;
}

float3 ui_linear_to_srgb(float3 color) {
    color = max(color, 0.0.xxx);
    float3 low = 12.92 * color;
    float3 high = 1.055 * pow(color, 1.0 / 2.4) - 0.055;
    return lerp(high, low, 1.0 - step(0.0031308.xxx, color));
}

float4 UiPSMain(UiPSInput input) : SV_TARGET {
    float4 color = ui_texture.Sample(ui_sampler, input.uv) * input.color;
    return float4(ui_linear_to_srgb(color.rgb), color.a);
}
