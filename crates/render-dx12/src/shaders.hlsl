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

struct ParticleVSInput {
    float3 position : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
    float4 instance_position_size : INSTANCE_POSITION_SIZE;
    float2 instance_rotation_age : INSTANCE_ROTATION_AGE;
    uint instance_color : INSTANCE_COLOR;
};

struct PSInput {
    float4 position : SV_POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
    float4 shadow_position : TEXCOORD1;
    float3 local_position : TEXCOORD2;
    float3 mapping_position : TEXCOORD3;
    float3 mapping_parameters : TEXCOORD4;
    float4 particle_color : COLOR0;
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
    uint4 advanced_packed;
    float4 environment_params;
};

Texture2D base_color_map : register(t0);
Texture2D<float> shadow_map : register(t1);
Texture2D normal_map : register(t2);
Texture2D metallic_roughness_map : register(t3);
Texture2D occlusion_map : register(t4);
Texture2D emissive_map : register(t5);
TextureCube environment_map : register(t6);
ByteAddressBuffer clustered_lights : register(t7);
ByteAddressBuffer clustered_grid : register(t8);
ByteAddressBuffer clustered_indices : register(t9);
ByteAddressBuffer gpu_particle_parameters : register(t10);
SamplerState base_color_sampler : register(s0);
SamplerState shadow_sampler : register(s1);
SamplerState normal_sampler : register(s2);
SamplerState metallic_roughness_sampler : register(s3);
SamplerState occlusion_sampler : register(s4);
SamplerState emissive_sampler : register(s5);
SamplerState environment_sampler : register(s6);

cbuffer VertexDraw : register(b1) {
    // Same layout and semantics as Vulkan DrawPush/PC.
    // x=factor, y=encoded normal-length delta scale, z=enabled.
    float4 radial_morph;
    float4 morph_origin;
    // x=enabled, y=inverse world-space tile size, z=blend sharpness.
    float4 material_mapping;
    float4 mapping_origin;
    float4x4 bones[64];
};

float4 unpack_unorm_rgba8(uint packed_value);

float3 radial_geomorph_position(float3 position, float3 encoded_normal) {
    if (radial_morph.z <= 0.5 || radial_morph.x <= 0.0) {
        return position;
    }
    float3 radial = position - morph_origin.xyz;
    float radial_length = length(radial);
    float encoded_normal_length = length(encoded_normal);
    if (radial_length <= 1.0e-6 || encoded_normal_length <= 1.0e-6) {
        return position;
    }
    float delta = (encoded_normal_length - 1.0)
                * radial_morph.y
                * saturate(radial_morph.x);
    return position + radial * (delta / radial_length);
}

PSInput VSMain(VSInput input) {
    PSInput output;
    float3 local_position = radial_geomorph_position(input.position, input.normal);
    output.position = mul(float4(local_position, 1.0), mvp);
    output.normal = input.normal;
    output.uv = input.uv;
    output.shadow_position = mul(float4(local_position, 1.0), light_mvp);
    output.local_position = local_position;
    output.mapping_position = local_position - mapping_origin.xyz;
    output.mapping_parameters = material_mapping.xyz;
    output.particle_color = 1.0.xxxx;
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
    output.mapping_position = 0.0.xxx;
    output.mapping_parameters = 0.0.xxx;
    output.particle_color = 1.0.xxxx;
    return output;
}

PSInput ParticleVSMain(ParticleVSInput input) {
    PSInput output;
    float sine;
    float cosine;
    sincos(input.instance_rotation_age.x, sine, cosine);
    float2 rotated = float2(
        input.position.x * cosine - input.position.y * sine,
        input.position.x * sine + input.position.y * cosine
    ) * input.instance_position_size.w;
    float3 camera_right = float3(light_mvp._m00, light_mvp._m10, light_mvp._m20);
    float3 camera_up = float3(light_mvp._m01, light_mvp._m11, light_mvp._m21);
    float3 world_position = input.instance_position_size.xyz
                          + camera_right * rotated.x
                          + camera_up * rotated.y;
    output.position = mul(float4(world_position, 1.0), mvp);
    output.normal = normalize(cross(camera_right, camera_up));
    output.uv = input.uv;
    output.shadow_position = 0.0.xxxx;
    output.local_position = world_position;
    output.mapping_position = 0.0.xxx;
    output.mapping_parameters = 0.0.xxx;
    output.particle_color = unpack_unorm_rgba8(input.instance_color);
    return output;
}

uint particle_hash(uint seed_low, uint seed_high, uint ordinal, uint stream) {
    uint value = ordinal * 0x9e3779b9u + stream * 0x85ebca6bu;
    value ^= seed_low ^ seed_high;
    value ^= value >> 16u;
    value *= 0x7feb352du;
    value ^= value >> 15u;
    value *= 0x846ca68bu;
    value ^= value >> 16u;
    return value;
}

float particle_random(uint2 seed, uint ordinal, uint stream) {
    return (float)particle_hash(seed.x, seed.y, ordinal, stream)
         / 4294967295.0;
}

float3 particle_tangent(float3 axis) {
    return abs(axis.x) > abs(axis.z)
        ? normalize(float3(-axis.y, axis.x, 0.0))
        : normalize(float3(0.0, -axis.z, axis.y));
}

float3 particle_integrated_displacement(
    float3 initial_velocity,
    float3 acceleration,
    float drag,
    float age
) {
    if (drag <= 1e-5) {
        return initial_velocity * age + acceleration * (0.5 * age * age);
    }
    float decay = exp(-drag * age);
    float velocity_integral = (1.0 - decay) / drag;
    return initial_velocity * velocity_integral
         + acceleration * (age / drag - velocity_integral / drag);
}

PSInput GpuParticleVSMain(VSInput input, uint instance_id : SV_InstanceID) {
    PSInput output;
    float4 origin_elapsed = asfloat(gpu_particle_parameters.Load4(0));
    float4 emission_lifetime = asfloat(gpu_particle_parameters.Load4(16));
    float4 speed_size_spread = asfloat(gpu_particle_parameters.Load4(32));
    float4 direction_drag = asfloat(gpu_particle_parameters.Load4(48));
    float4 acceleration_turbulence = asfloat(gpu_particle_parameters.Load4(64));
    float4 frequency_angular_duration = asfloat(gpu_particle_parameters.Load4(80));
    uint4 colors_ordinals = gpu_particle_parameters.Load4(96);
    uint4 seed_burst_max = gpu_particle_parameters.Load4(112);
    uint ordinal = colors_ordinals.z + instance_id;
    float spawn_time = ordinal < seed_burst_max.z
        ? 0.0
        : (emission_lifetime.x > 0.0
            ? (float)(ordinal - seed_burst_max.z + 1u) / emission_lifetime.x
            : 3.402823466e+38);
    float age = origin_elapsed.w - spawn_time;
    float lifetime = lerp(
        emission_lifetime.y,
        emission_lifetime.z,
        particle_random(seed_burst_max.xy, ordinal, 0u)
    );
    bool live = age >= 0.0 && age < lifetime;
    float3 axis = normalize(direction_drag.xyz);
    if (all(abs(axis) < 1e-6.xxx)) {
        axis = float3(0.0, 1.0, 0.0);
    }
    float cos_limit = cos(speed_size_spread.w);
    float cos_theta = 1.0
        - particle_random(seed_burst_max.xy, ordinal, 1u) * (1.0 - cos_limit);
    float sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    float phi = particle_random(seed_burst_max.xy, ordinal, 2u) * 6.28318530718;
    float phi_sine;
    float phi_cosine;
    sincos(phi, phi_sine, phi_cosine);
    float3 tangent = particle_tangent(axis);
    float3 bitangent = cross(axis, tangent);
    float3 direction = normalize(
        axis * cos_theta
        + tangent * (sin_theta * phi_cosine)
        + bitangent * (sin_theta * phi_sine)
    );
    float speed = lerp(
        emission_lifetime.w,
        speed_size_spread.x,
        particle_random(seed_burst_max.xy, ordinal, 3u)
    );
    float3 world_position = origin_elapsed.xyz
        + particle_integrated_displacement(
            direction * speed,
            acceleration_turbulence.xyz,
            direction_drag.w,
            max(age, 0.0)
        );
    if (acceleration_turbulence.w > 0.0) {
        float3 phase = float3(
            particle_random(seed_burst_max.xy, ordinal, 6u),
            particle_random(seed_burst_max.xy, ordinal, 7u),
            particle_random(seed_burst_max.xy, ordinal, 8u)
        ) * 6.28318530718
          + max(age, 0.0) * frequency_angular_duration.x;
        float3 turbulence = float3(
            sin(phase.y + phase.z * 1.37),
            sin(phase.z + phase.x * 1.79),
            sin(phase.x + phase.y * 2.11)
        ) * (0.5 * acceleration_turbulence.w * age * age);
        world_position += turbulence;
    }
    float normalized_age = saturate(age / max(lifetime, 1e-6));
    float angular_velocity = lerp(
        frequency_angular_duration.y,
        frequency_angular_duration.z,
        particle_random(seed_burst_max.xy, ordinal, 4u)
    );
    float rotation =
        particle_random(seed_burst_max.xy, ordinal, 5u) * 6.28318530718
        + angular_velocity * age;
    float rotation_sine;
    float rotation_cosine;
    sincos(rotation, rotation_sine, rotation_cosine);
    float size = live
        ? lerp(speed_size_spread.y, speed_size_spread.z, normalized_age)
        : 0.0;
    float2 rotated = float2(
        input.position.x * rotation_cosine - input.position.y * rotation_sine,
        input.position.x * rotation_sine + input.position.y * rotation_cosine
    ) * size;
    float3 camera_right = normalize(
        float3(light_mvp._m00, light_mvp._m10, light_mvp._m20)
    );
    float3 camera_up = normalize(
        float3(light_mvp._m01, light_mvp._m11, light_mvp._m21)
    );
    world_position += camera_right * rotated.x + camera_up * rotated.y;
    output.position = live
        ? mul(float4(world_position, 1.0), mvp)
        : float4(2.0, 2.0, 2.0, 1.0);
    output.normal = normalize(cross(camera_right, camera_up));
    output.uv = input.uv;
    output.shadow_position = 0.0.xxxx;
    output.local_position = world_position;
    output.mapping_position = 0.0.xxx;
    output.mapping_parameters = 0.0.xxx;
    output.particle_color = lerp(
        unpack_unorm_rgba8(colors_ordinals.x),
        unpack_unorm_rgba8(colors_ordinals.y),
        normalized_age
    );
    return output;
}

float4 ShadowVSMain(VSInput input) : SV_POSITION {
    float3 local_position = radial_geomorph_position(input.position, input.normal);
    return mul(float4(local_position, 1.0), mvp);
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

float3 triplanar_weights(float3 normal, float sharpness) {
    float3 weights = pow(
        max(abs(normal), float3(1.0e-4, 1.0e-4, 1.0e-4)),
        float3(
            clamp(sharpness, 1.0, 32.0),
            clamp(sharpness, 1.0, 32.0),
            clamp(sharpness, 1.0, 32.0)
        )
    );
    return weights / max(weights.x + weights.y + weights.z, 1.0e-5);
}

float projection_sign(float value) {
    return value < 0.0 ? -1.0 : 1.0;
}

float2 triplanar_uv_x(float3 position, float3 normal) {
    return float2(-projection_sign(normal.x) * position.z, position.y);
}

float2 triplanar_uv_y(float3 position, float3 normal) {
    return float2(projection_sign(normal.y) * position.x, -position.z);
}

float2 triplanar_uv_z(float3 position, float3 normal) {
    return float2(projection_sign(normal.z) * position.x, position.y);
}

float4 sample_triplanar(
    Texture2D texture_map,
    SamplerState texture_sampler,
    float3 position,
    float3 geometric_normal,
    float3 weights
) {
    return texture_map.Sample(
        texture_sampler,
        triplanar_uv_x(position, geometric_normal)
    ) * weights.x
         + texture_map.Sample(
            texture_sampler,
            triplanar_uv_y(position, geometric_normal)
        ) * weights.y
         + texture_map.Sample(
            texture_sampler,
            triplanar_uv_z(position, geometric_normal)
        ) * weights.z;
}

float3 sample_triplanar_normal(
    Texture2D texture_map,
    SamplerState texture_sampler,
    float3 position,
    float3 geometric_normal,
    float3 weights
) {
    float sign_x = projection_sign(geometric_normal.x);
    float sign_y = projection_sign(geometric_normal.y);
    float sign_z = projection_sign(geometric_normal.z);
    float3 tangent_x = texture_map.Sample(
        texture_sampler,
        triplanar_uv_x(position, geometric_normal)
    ).xyz * 2.0 - 1.0;
    float3 tangent_y = texture_map.Sample(
        texture_sampler,
        triplanar_uv_y(position, geometric_normal)
    ).xyz * 2.0 - 1.0;
    float3 tangent_z = texture_map.Sample(
        texture_sampler,
        triplanar_uv_z(position, geometric_normal)
    ).xyz * 2.0 - 1.0;
    float3 normal_x = float3(
        sign_x * tangent_x.z,
        tangent_x.y,
        -sign_x * tangent_x.x
    );
    float3 normal_y = float3(
        sign_y * tangent_y.x,
        sign_y * tangent_y.z,
        -tangent_y.y
    );
    float3 normal_z = float3(
        sign_z * tangent_z.x,
        tangent_z.y,
        sign_z * tangent_z.z
    );
    float3 perturbation =
        (normal_x - float3(sign_x, 0.0, 0.0)) * weights.x
      + (normal_y - float3(0.0, sign_y, 0.0)) * weights.y
      + (normal_z - float3(0.0, 0.0, sign_z)) * weights.z;
    return normalize(geometric_normal + perturbation);
}

float4 unpack_unorm_rgba8(uint packed_value) {
    return float4(
        packed_value & 255u,
        (packed_value >> 8u) & 255u,
        (packed_value >> 16u) & 255u,
        (packed_value >> 24u) & 255u
    ) / 255.0;
}

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

float distribution_ggx(float3 normal, float3 halfway, float roughness_value) {
    float alpha = roughness_value * roughness_value;
    float alpha_squared = alpha * alpha;
    float n_dot_h = saturate(dot(normal, halfway));
    float denominator = n_dot_h * n_dot_h * (alpha_squared - 1.0) + 1.0;
    return alpha_squared / max(3.14159265 * denominator * denominator, 1e-5);
}

float geometry_schlick_ggx(float n_dot_direction, float roughness_value) {
    float r = roughness_value + 1.0;
    float k = r * r * 0.125;
    return n_dot_direction / max(n_dot_direction * (1.0 - k) + k, 1e-5);
}

float3 fresnel_schlick(float cosine, float3 f0) {
    return f0 + (1.0 - f0) * pow(saturate(1.0 - cosine), 5.0);
}

float3 evaluate_cluster_light(
    float3 normal,
    float3 view_direction,
    float3 light_direction,
    float3 light_color,
    float intensity,
    float attenuation,
    float3 albedo,
    float metallic_value,
    float roughness_value
) {
    float n_dot_l = saturate(dot(normal, light_direction));
    if (n_dot_l <= 0.0 || attenuation <= 0.0 || intensity <= 0.0) {
        return 0.0.xxx;
    }
    float3 halfway = normalize(view_direction + light_direction);
    float ndf = distribution_ggx(normal, halfway, roughness_value);
    float geometry = geometry_schlick_ggx(saturate(dot(normal, view_direction)), roughness_value)
                   * geometry_schlick_ggx(n_dot_l, roughness_value);
    float3 f0 = lerp(0.04.xxx, albedo, metallic_value);
    float3 fresnel = fresnel_schlick(saturate(dot(halfway, view_direction)), f0);
    float denominator = max(
        4.0 * saturate(dot(normal, view_direction)) * n_dot_l,
        1e-4
    );
    float3 specular_lobe = ndf * geometry * fresnel / denominator;
    float3 diffuse_lobe = (1.0 - fresnel) * (1.0 - metallic_value)
                        * albedo / 3.14159265;
    return (diffuse_lobe + specular_lobe) * light_color
         * intensity * attenuation * n_dot_l;
}

float3 reconstruct_cluster_world_position(float4 screen_position) {
    float4 viewport = asfloat(clustered_lights.Load4(32));
    float2 normalized = (screen_position.xy - viewport.xy)
                      / max(viewport.zw, 1.0.xx);
    float4 clip_position = float4(
        normalized.x * 2.0 - 1.0,
        1.0 - normalized.y * 2.0,
        screen_position.z,
        1.0
    );
    float4 inverse_column0 = asfloat(clustered_lights.Load4(64));
    float4 inverse_column1 = asfloat(clustered_lights.Load4(80));
    float4 inverse_column2 = asfloat(clustered_lights.Load4(96));
    float4 inverse_column3 = asfloat(clustered_lights.Load4(112));
    float4 world_position = inverse_column0 * clip_position.x
                          + inverse_column1 * clip_position.y
                          + inverse_column2 * clip_position.z
                          + inverse_column3;
    return world_position.xyz / max(abs(world_position.w), 1e-6);
}

float3 evaluate_clustered_lights(
    PSInput input,
    float3 normal,
    float3 albedo,
    float metallic_value,
    float roughness_value
) {
    uint4 dimensions = clustered_lights.Load4(0);
    uint4 stats = clustered_lights.Load4(16);
    if (dimensions.x == 0u || stats.x == 0u) {
        return 0.0.xxx;
    }
    float4 viewport = asfloat(clustered_lights.Load4(32));
    float4 depth_parameters = asfloat(clustered_lights.Load4(48));
    float3 camera_position = asfloat(clustered_lights.Load4(128)).xyz;
    float3 world_position = reconstruct_cluster_world_position(input.position);
    float3 view_direction = normalize(camera_position - world_position);
    float2 normalized = saturate(
        (input.position.xy - viewport.xy) / max(viewport.zw, 1.0.xx)
    );
    uint2 tile_count = max(dimensions.yz, 1u.xx);
    uint2 tile = min(
        (uint2)(normalized * (float2)tile_count),
        tile_count - 1u
    );
    float distance_to_camera = clamp(
        length(world_position - camera_position),
        depth_parameters.x,
        depth_parameters.y
    );
    uint depth_slice = (uint)clamp(
        floor(log(distance_to_camera) * depth_parameters.z + depth_parameters.w),
        0.0,
        (float)(max(dimensions.w, 1u) - 1u)
    );
    uint cluster_index = (depth_slice * tile_count.y + tile.y) * tile_count.x + tile.x;
    cluster_index = min(cluster_index, stats.x - 1u);
    uint2 cluster = clustered_grid.Load2(cluster_index * 8u);
    uint count = min(cluster.y, stats.z);
    float3 contribution = 0.0.xxx;
    [loop]
    for (uint item = 0u; item < count; ++item) {
        uint list_index = cluster.x + item;
        if (list_index >= stats.y) {
            break;
        }
        uint light_index = clustered_indices.Load(list_index * 4u);
        if (light_index >= dimensions.x) {
            continue;
        }
        uint light_offset = 144u + light_index * 64u;
        float4 light_position = asfloat(clustered_lights.Load4(light_offset));
        float4 light_direction_data = asfloat(clustered_lights.Load4(light_offset + 16u));
        float4 light_color = asfloat(clustered_lights.Load4(light_offset + 32u));
        float4 light_attenuation = asfloat(clustered_lights.Load4(light_offset + 48u));
        float3 light_direction;
        float attenuation = 1.0;
        if (light_position.w < 0.5) {
            light_direction = normalize(-light_direction_data.xyz);
            attenuation *= shadow_visibility(input);
        } else {
            float3 to_light = light_position.xyz - world_position;
            float distance_to_light = length(to_light);
            if (distance_to_light <= 1e-5
                || (light_attenuation.x > 0.0
                    && distance_to_light > light_attenuation.x)) {
                continue;
            }
            light_direction = to_light / distance_to_light;
            attenuation = 1.0 / (
                1.0 + light_attenuation.y * distance_to_light
                + light_attenuation.z * distance_to_light * distance_to_light
            );
            if (light_position.w > 1.5) {
                float cone = dot(normalize(-light_direction_data.xyz), -light_direction);
                if (cone < light_attenuation.w) {
                    continue;
                }
                attenuation *= smoothstep(light_attenuation.w, 1.0, cone);
            }
        }
        contribution += evaluate_cluster_light(
            normal,
            view_direction,
            light_direction,
            light_color.rgb,
            light_color.a,
            attenuation,
            albedo,
            metallic_value,
            roughness_value
        );
    }
    return contribution;
}

struct ForwardOutput {
    float4 hdr : SV_TARGET0;
    float4 oit_accumulation : SV_TARGET1;
    float4 oit_optical_depth : SV_TARGET2;
};

ForwardOutput PSMain(PSInput input, bool front_face : SV_IsFrontFace) {
    float3 normal = normalize(input.normal);
    if (!front_face) {
        normal = -normal;
    }
    bool uses_triplanar = input.mapping_parameters.x > 0.5;
    float3 mapping_position =
        input.mapping_position * input.mapping_parameters.y;
    float3 mapping_normal = normal;
    float3 mapping_weights = triplanar_weights(
        mapping_normal,
        input.mapping_parameters.z
    );
    uint texture_flags = (uint)(emissive.w + 0.5);
    if ((texture_flags & 2u) != 0u) {
        if (uses_triplanar) {
            normal = sample_triplanar_normal(
                normal_map,
                normal_sampler,
                mapping_position,
                mapping_normal,
                mapping_weights
            );
        } else {
            float3 tangent_normal =
                normal_map.Sample(normal_sampler, input.uv).xyz * 2.0 - 1.0;
            normal = normalize(mul(tangent_normal, cotangent_frame(
                normal, input.local_position, input.uv
            )));
        }
    }
    float perceptual_roughness = max(roughness, 0.04);
    float4 sampled_base_color = base_color;
    bool weighted_oit = material_flags >= 8.0;
    float surface_flags = weighted_oit ? material_flags - 8.0 : material_flags;
    bool masked = surface_flags >= 2.0;
    bool uses_texture = (surface_flags >= 1.0 && surface_flags < 2.0)
                     || surface_flags >= 3.0;
    if (uses_texture) {
        sampled_base_color *= uses_triplanar
            ? sample_triplanar(
                base_color_map,
                base_color_sampler,
                mapping_position,
                mapping_normal,
                mapping_weights
            )
            : base_color_map.Sample(base_color_sampler, input.uv);
    }
    sampled_base_color *= input.particle_color;
    if (masked) {
        float alpha_cutoff = frac(surface_flags) * 2.0;
        clip(sampled_base_color.a - alpha_cutoff);
    }
    float metallic_value = metallic;
    float roughness_value = perceptual_roughness;
    float ao_value = ambient_occlusion;
    float3 emissive_color = emissive.rgb;
    if ((texture_flags & 4u) != 0u) {
        float4 metallic_roughness_sample = uses_triplanar
            ? sample_triplanar(
                metallic_roughness_map,
                metallic_roughness_sampler,
                mapping_position,
                mapping_normal,
                mapping_weights
            )
            : metallic_roughness_map.Sample(
                metallic_roughness_sampler,
                input.uv
            );
        roughness_value = max(roughness_value * metallic_roughness_sample.g, 0.04);
        metallic_value *= metallic_roughness_sample.b;
    }
    if ((texture_flags & 8u) != 0u) {
        ao_value *= uses_triplanar
            ? sample_triplanar(
                occlusion_map,
                occlusion_sampler,
                mapping_position,
                mapping_normal,
                mapping_weights
            ).r
            : occlusion_map.Sample(occlusion_sampler, input.uv).r;
    }
    if ((texture_flags & 16u) != 0u) {
        emissive_color *= uses_triplanar
            ? sample_triplanar(
                emissive_map,
                emissive_sampler,
                mapping_position,
                mapping_normal,
                mapping_weights
            ).rgb
            : emissive_map.Sample(emissive_sampler, input.uv).rgb;
    }
    float3 light_direction = normalize(scene_light_direction.xyz);
    float n_dot_l = saturate(dot(normal, light_direction));
    float3 color = sampled_base_color.rgb * 0.03 * ao_value;
    color += evaluate_clustered_lights(
        input,
        normal,
        sampled_base_color.rgb,
        metallic_value,
        roughness_value
    );
    float3 cluster_world_position = reconstruct_cluster_world_position(input.position);
    float3 cluster_camera_position = asfloat(clustered_lights.Load4(128)).xyz;
    float3 view_direction = normalize(cluster_camera_position - cluster_world_position);
    float n_dot_v = saturate(dot(normal, view_direction));
    float4 advanced_weights = unpack_unorm_rgba8(advanced_packed.x);
    advanced_weights.w = advanced_weights.w * 2.0 - 1.0;
    float clearcoat_specular = pow(
        saturate(dot(reflect(-light_direction, normal), view_direction)),
        lerp(192.0, 8.0, advanced_weights.y)
    );
    color += advanced_weights.x * 0.25 * clearcoat_specular.xxx * n_dot_l;
    float3 subsurface_color = unpack_unorm_rgba8(advanced_packed.y).rgb;
    float wrapped_light = saturate((dot(normal, light_direction) + 0.5) / 1.5);
    color += sampled_base_color.rgb * subsurface_color
           * advanced_weights.z * wrapped_light * 0.35;
    float3 sheen_color = unpack_unorm_rgba8(advanced_packed.z).rgb;
    float grazing = pow(1.0 - n_dot_v, 5.0);
    color += sheen_color * grazing * (0.5 + 0.5 * abs(advanced_weights.w));
    float4 rim = unpack_unorm_rgba8(advanced_packed.w);
    float rim_power = lerp(0.01, 32.0, rim.a);
    color += rim.rgb * pow(1.0 - n_dot_v, rim_power);
    if (environment_params.w > 0.5 && environment_params.x > 0.0) {
        float3 environment_normal =
            rotate_environment_direction(normal, environment_params.y);
        float3 reflection_direction = rotate_environment_direction(
            reflect(-view_direction, normal),
            environment_params.y
        );
        float3 irradiance = environment_map.SampleLevel(
            environment_sampler,
            environment_normal,
            environment_params.z
        ).rgb;
        float3 radiance = environment_map.SampleLevel(
            environment_sampler,
            reflection_direction,
            roughness_value * environment_params.z
        ).rgb;
        float3 dielectric_f0 = 0.04.xxx;
        float3 f0 = lerp(dielectric_f0, sampled_base_color.rgb, metallic_value);
        color += environment_params.x * (
            irradiance * sampled_base_color.rgb * (1.0 - metallic_value) * ao_value
            + radiance * f0
        );
    }
    color += emissive_color;
    // Preserve linear HDR values; the terminal tone-map pass performs display
    // transfer encoding for the UNORM swapchain.
    ForwardOutput output;
    output.hdr = float4(color, sampled_base_color.a);
    output.oit_accumulation = 0.0.xxxx;
    output.oit_optical_depth = 0.0.xxxx;
    if (weighted_oit) {
        float alpha = saturate(sampled_base_color.a);
        // McGuire/Bavoil weighted blended OIT: emphasize opaque, nearby
        // fragments while keeping the accumulation bounded in half precision.
        float depth_weight = pow(saturate(1.0 - input.position.z * 0.9), 3.0);
        float weight = clamp(
            pow(alpha + 0.01, 3.0) * 10000.0 * depth_weight,
            0.01,
            3000.0
        );
        output.hdr = 0.0.xxxx;
        output.oit_accumulation = float4(color * alpha * weight, alpha * weight);
        output.oit_optical_depth = float4(
            -log(max(1.0 - alpha, 0.0001)),
            0.0,
            0.0,
            0.0
        );
    }
    return output;
}
