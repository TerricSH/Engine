// Minimal static PBR32 mesh shader used by Dx12SceneRenderer.
struct VSInput {
    float3 position : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
};

struct SkinnedVSInput {
    float3 position : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
    uint4 joints : JOINTS0;
    float4 weights : WEIGHTS0;
};

struct PSInput {
    float4 position : SV_POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
    float4 shadow_position : TEXCOORD1;
    float3 local_position : TEXCOORD2;
};

cbuffer Scene : register(b0) {
    float4x4 mvp;
    float4 base_color;
    float metallic;
    float roughness;
    float ambient_occlusion;
    float material_flags;
    float4x4 light_mvp;
    float4 shadow_params;
    float4 scene_light_direction;
    float4 emissive;
};

Texture2D base_color_map : register(t0);
Texture2D<float> shadow_map : register(t1);
Texture2D normal_map : register(t2);
Texture2D metallic_roughness_map : register(t3);
Texture2D occlusion_map : register(t4);
Texture2D emissive_map : register(t5);
SamplerState base_color_sampler : register(s0);
SamplerState shadow_sampler : register(s1);
SamplerState normal_sampler : register(s2);
SamplerState metallic_roughness_sampler : register(s3);
SamplerState occlusion_sampler : register(s4);
SamplerState emissive_sampler : register(s5);

cbuffer Bones : register(b1) {
    float4x4 bones[64];
};

PSInput VSMain(VSInput input) {
    PSInput output;
    output.position = mul(float4(input.position, 1.0), mvp);
    output.normal = input.normal;
    output.uv = input.uv;
    output.shadow_position = mul(float4(input.position, 1.0), light_mvp);
    output.local_position = input.position;
    return output;
}

PSInput SkinnedVSMain(SkinnedVSInput input) {
    PSInput output;
    float4x4 skin = input.weights.x * bones[input.joints.x]
                  + input.weights.y * bones[input.joints.y]
                  + input.weights.z * bones[input.joints.z]
                  + input.weights.w * bones[input.joints.w];
    float4 local_position = mul(float4(input.position, 1.0), skin);
    output.position = mul(local_position, mvp);
    output.normal = normalize(mul(float4(input.normal, 0.0), skin).xyz);
    output.uv = input.uv;
    output.shadow_position = mul(local_position, light_mvp);
    output.local_position = local_position.xyz;
    return output;
}

float4 ShadowVSMain(VSInput input) : SV_POSITION {
    return mul(float4(input.position, 1.0), mvp);
}

float4 SkinnedShadowVSMain(SkinnedVSInput input) : SV_POSITION {
    float4x4 skin = input.weights.x * bones[input.joints.x]
                  + input.weights.y * bones[input.joints.y]
                  + input.weights.z * bones[input.joints.z]
                  + input.weights.w * bones[input.joints.w];
    return mul(mul(float4(input.position, 1.0), skin), mvp);
}

float shadow_visibility(PSInput input) {
    if (shadow_params.x < 0.5 || input.shadow_position.w <= 0.0) {
        return 1.0;
    }
    float3 projected = input.shadow_position.xyz / input.shadow_position.w;
    float2 uv = float2(projected.x * 0.5 + 0.5, -projected.y * 0.5 + 0.5);
    if (projected.z <= 0.0 || projected.z >= 1.0 || any(uv < 0.0) || any(uv > 1.0)) {
        return 1.0;
    }
    float receiver_depth = projected.z - shadow_params.w;
    if (shadow_params.y < 0.5) {
        return receiver_depth <= shadow_map.SampleLevel(shadow_sampler, uv, 0.0) ? 1.0 : 0.2;
    }
    float visibility = 0.0;
    [unroll]
    for (int y = -1; y <= 1; ++y) {
        [unroll]
        for (int x = -1; x <= 1; ++x) {
            float stored_depth = shadow_map.SampleLevel(
                shadow_sampler,
                uv + float2(x, y) * shadow_params.z,
                0.0
            );
            visibility += receiver_depth <= stored_depth ? 1.0 : 0.2;
        }
    }
    return visibility / 9.0;
}

float3 linear_to_srgb(float3 linear_color) {
    float3 linear_rgb = max(linear_color, 0.0.xxx);
    float3 low = 12.92 * linear_rgb;
    float3 high = 1.055 * pow(linear_rgb, 1.0 / 2.4) - 0.055;
    float3 use_low = 1.0 - step(0.0031308.xxx, linear_rgb);
    return lerp(high, low, use_low);
}

float3x3 cotangent_frame(float3 normal, float3 position, float2 uv) {
    float3 dp1 = ddx(position);
    float3 dp2 = ddy(position);
    float2 duv1 = ddx(uv);
    float2 duv2 = ddy(uv);
    float3 dp2_perp = cross(dp2, normal);
    float3 dp1_perp = cross(normal, dp1);
    float3 tangent = dp2_perp * duv1.x + dp1_perp * duv2.x;
    float3 bitangent = dp2_perp * duv1.y + dp1_perp * duv2.y;
    float scale = rsqrt(max(max(dot(tangent, tangent), dot(bitangent, bitangent)), 1e-8));
    return float3x3(tangent * scale, bitangent * scale, normal);
}

float4 PSMain(PSInput input, bool front_face : SV_IsFrontFace) : SV_TARGET {
    float3 normal = normalize(input.normal);
    if (!front_face) {
        normal = -normal;
    }
    uint texture_flags = (uint)(emissive.w + 0.5);
    if ((texture_flags & 2u) != 0u) {
        float3 tangent_normal =
            normal_map.Sample(normal_sampler, input.uv).xyz * 2.0 - 1.0;
        normal = normalize(mul(tangent_normal, cotangent_frame(
            normal, input.local_position, input.uv
        )));
    }
    float3 light_direction = normalize(scene_light_direction.xyz);
    float n_dot_l = saturate(dot(normal, light_direction));
    float diffuse = 0.15 + 0.85 * n_dot_l * shadow_visibility(input);
    float perceptual_roughness = max(roughness, 0.04);
    float specular = pow(saturate(dot(reflect(-light_direction, normal), float3(0.0, 0.0, 1.0))),
                         lerp(128.0, 4.0, perceptual_roughness));
    float4 sampled_base_color = base_color;
    bool masked = material_flags >= 2.0;
    bool uses_texture = (material_flags >= 1.0 && material_flags < 2.0)
                     || material_flags >= 3.0;
    if (uses_texture) {
        sampled_base_color *= base_color_map.Sample(base_color_sampler, input.uv);
    }
    if (masked) {
        float alpha_cutoff = frac(material_flags) * 2.0;
        clip(sampled_base_color.a - alpha_cutoff);
    }
    float metallic_value = metallic;
    float roughness_value = perceptual_roughness;
    float ao_value = ambient_occlusion;
    float3 emissive_color = emissive.rgb;
    if ((texture_flags & 4u) != 0u) {
        float4 metallic_roughness_sample =
            metallic_roughness_map.Sample(metallic_roughness_sampler, input.uv);
        roughness_value = max(roughness_value * metallic_roughness_sample.g, 0.04);
        metallic_value *= metallic_roughness_sample.b;
    }
    if ((texture_flags & 8u) != 0u) {
        ao_value *= occlusion_map.Sample(occlusion_sampler, input.uv).r;
    }
    if ((texture_flags & 16u) != 0u) {
        emissive_color *= emissive_map.Sample(emissive_sampler, input.uv).rgb;
    }
    specular = pow(saturate(dot(reflect(-light_direction, normal), float3(0.0, 0.0, 1.0))),
                   lerp(128.0, 4.0, roughness_value));
    float3 color = sampled_base_color.rgb * diffuse * ao_value;
    color += lerp(0.04.xxx, sampled_base_color.rgb, metallic_value) * specular;
    color += emissive_color;
    // The DX12 swapchain uses an UNORM RTV, so even ToneMapping::None must
    // encode linear lighting into the display sRGB transfer function.
    return float4(linear_to_srgb(color), sampled_base_color.a);
}
