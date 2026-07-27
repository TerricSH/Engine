struct SkyboxOutput {
    float4 position : SV_POSITION;
    float3 direction : TEXCOORD0;
};

cbuffer SkyboxSettings : register(b0) {
    float4x4 inverse_view_projection;
    float4 camera_position;
    float4 skybox_padding[9];
    float4 environment_params;
};

TextureCube environment_map : register(t6);
SamplerState environment_sampler : register(s6);

float3 rotate_environment_direction(float3 direction, float angle) {
    float sine;
    float cosine;
    sincos(angle, sine, cosine);
    return float3(
        cosine * direction.x - sine * direction.z,
        direction.y,
        sine * direction.x + cosine * direction.z
    );
}

SkyboxOutput SkyboxVSMain(uint vertex_id : SV_VertexID) {
    SkyboxOutput output;
    float2 position = float2(
        vertex_id == 2 ? 3.0 : -1.0,
        vertex_id == 1 ? 3.0 : -1.0
    );
    output.position = float4(position, 1.0, 1.0);
    float4 world = mul(float4(position, 1.0, 1.0), inverse_view_projection);
    world.xyz /= max(abs(world.w), 0.00001);
    output.direction = world.xyz - camera_position.xyz;
    return output;
}

float4 SkyboxPSMain(SkyboxOutput input) : SV_TARGET {
    float3 direction = rotate_environment_direction(
        normalize(input.direction),
        environment_params.y
    );
    return float4(
        environment_map.SampleLevel(environment_sampler, direction, 0.0).rgb
            * environment_params.x,
        1.0
    );
}
