#version 450

// Forward-rendering fragment shader with Cook-Torrance PBR BRDF and CSM.
// Metallic-roughness workflow with GGX normal distribution, Smith
// geometry function, and Fresnel-Schlick approximation.
// Per-frame UBO at set=0, binding=0 (matches descriptor.rs layout).
// Shadow map array at set=1, binding=0 (3-cascade CSM).
// Light SSBO at set=1, binding=2 (clustered additional lights).

layout(location = 0) in vec3 v_world_pos;
layout(location = 1) in vec3 v_normal;
layout(location = 2) in vec2 v_uv;
#ifdef VFX_PARTICLE
layout(location = 3) in vec4 v_particle_color;
#endif

layout(location = 0) out vec4 out_color;
layout(location = 1) out vec4 out_oit_accumulation;
layout(location = 2) out vec4 out_oit_optical_depth;

layout(binding = 0) uniform PerFrameUBO {
    mat4 model;
    mat4 view_proj;
    vec4 light_dir;         // w=0 (directional)
    vec4 light_color;       // rgb = color, a = intensity
    vec4 camera_pos;
    vec4 cascade_splits;    // x=split0, y=split1, z=split2, w=far
    mat4 light_vp[3];       // 3 cascade light VP matrices
    vec4 environment_params; // x=intensity, y=sin(rotation), z=cos(rotation)
} ubo;

// Shadow map array (set=1, binding=0) — 2D array depth texture with PCF.
layout(set = 1, binding = 0) uniform sampler2DArrayShadow u_shadow_map;

// Environment cubemap (set=1, binding=1) — IBL irradiance / prefiltered env
layout(set = 1, binding = 1) uniform samplerCube u_irradiance_map;

// Additional lights SSBO (set=1, binding=2) — clustered shading
struct Light {
    vec4 position;    // xyz = position, w = 0 (directional) or 1 (point) or 2 (spot)
    vec4 direction;   // spot cone direction
    vec4 color;       // rgb = color, a = intensity
    vec4 attenuation; // x = range, y = linear, z = quadratic, w = spot_cutoff_cos
};
layout(std430, set = 1, binding = 2) readonly buffer LightSSBO {
    uvec4 cluster_dimensions; // light count, tile columns, tile rows, depth slices
    uvec4 cluster_stats;      // cluster count, index count, per-cluster cap, overflow
    vec4 cluster_viewport;    // framebuffer x, y, width, height
    vec4 cluster_depth;       // near, far, logarithmic scale, logarithmic bias
    mat4 cluster_inverse_view_projection;
    vec4 cluster_camera_position;
    Light lights[];
} u_light_ssbo;
layout(std430, set = 1, binding = 3) readonly buffer ClusterGridSSBO {
    uvec2 clusters[]; // index-list offset and count
} u_cluster_grid;
layout(std430, set = 1, binding = 4) readonly buffer ClusterIndexSSBO {
    uint light_indices[];
} u_cluster_indices;

// Material parameters (per-drawable, set=2 binding=0).
layout(set = 2, binding = 0) uniform MaterialUBO {
    vec4 base_color;
    float metallic;
    float roughness;
    float ao;
    float alpha_cutoff;
    vec4 emissive;
    vec4 advanced0;          // clearcoat, clearcoat roughness, subsurface, anisotropy
    vec4 subsurface_color;
    vec4 sheen_color;
    vec4 rim_color_power;    // rgb = rim color, a = rim power
} material;

// Base color texture (set=2, binding=1) — optional.
layout(set = 2, binding = 1) uniform sampler2D u_base_color_texture;
layout(set = 2, binding = 3) uniform sampler2D u_normal_texture;
layout(set = 2, binding = 4) uniform sampler2D u_metallic_roughness_texture;
layout(set = 2, binding = 5) uniform sampler2D u_occlusion_texture;
layout(set = 2, binding = 6) uniform sampler2D u_emissive_texture;

const float PI = 3.14159265359;

// Normal Distribution Function — GGX / Trowbridge-Reitz
float distribution_ggx(vec3 N, vec3 H, float roughness) {
    float a = roughness * roughness;
    float a2 = a * a;
    float NdotH = max(dot(N, H), 0.0);
    float NdotH2 = NdotH * NdotH;
    float denom = NdotH2 * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

// Anisotropic GGX distribution in the derivative-generated tangent frame.
float distribution_ggx_anisotropic(
    vec3 N, vec3 H, vec3 T, vec3 B, float roughness, float anisotropy
) {
    float alpha = max(roughness * roughness, 0.0025);
    float stretch = 0.9 * clamp(anisotropy, -1.0, 1.0);
    float alpha_t = max(alpha * (1.0 + stretch), 0.0025);
    float alpha_b = max(alpha * (1.0 - stretch), 0.0025);
    float TdotH = dot(T, H);
    float BdotH = dot(B, H);
    float NdotH = max(dot(N, H), 0.0);
    float denom = TdotH * TdotH / (alpha_t * alpha_t)
                + BdotH * BdotH / (alpha_b * alpha_b)
                + NdotH * NdotH;
    return 1.0 / max(PI * alpha_t * alpha_b * denom * denom, 0.0001);
}

// Geometry function — Smith GGX correlation (IBL-friendly k)
float geometry_smith(vec3 N, vec3 V, vec3 L, float roughness) {
    float r = roughness + 1.0;
    float k = (r * r) / 8.0;

    float NdotV = max(dot(N, V), 0.0);
    float NdotL = max(dot(N, L), 0.0);
    float ggx1 = NdotV / (NdotV * (1.0 - k) + k);
    float ggx2 = NdotL / (NdotL * (1.0 - k) + k);
    return ggx1 * ggx2;
}

// Fresnel-Schlick approximation
vec3 fresnel_schlick(float cos_theta, vec3 F0) {
    return F0 + (1.0 - F0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

vec3 environment_direction(vec3 direction) {
    float rotation_sin = ubo.environment_params.y;
    float rotation_cos = ubo.environment_params.z;
    return vec3(
        rotation_cos * direction.x + rotation_sin * direction.z,
        direction.y,
        -rotation_sin * direction.x + rotation_cos * direction.z
    );
}

mat3 cotangent_frame(vec3 N, vec3 position, vec2 uv) {
    vec3 dp1 = dFdx(position);
    vec3 dp2 = dFdy(position);
    vec2 duv1 = dFdx(uv);
    vec2 duv2 = dFdy(uv);
    vec3 dp2_perp = cross(dp2, N);
    vec3 dp1_perp = cross(N, dp1);
    vec3 T = dp2_perp * duv1.x + dp1_perp * duv2.x;
    vec3 B = dp2_perp * duv1.y + dp1_perp * duv2.y;
    float scale = inversesqrt(max(max(dot(T, T), dot(B, B)), 1e-8));
    return mat3(T * scale, B * scale, N);
}

/// Sample the CSM shadow map at the given light-space position and cascade layer.
float sample_cascade_shadow(vec4 light_pos, int cascade) {
    vec3 proj = light_pos.xyz / light_pos.w;
    proj = proj * 0.5 + 0.5;

    if (proj.x < 0.0 || proj.x > 1.0 || proj.y < 0.0 || proj.y > 1.0 || proj.z < 0.0 || proj.z > 1.0) {
        return 1.0;
    }

    float bias = 0.005;
    float ref_depth = proj.z - bias;
    return texture(u_shadow_map, vec4(proj.xy, float(cascade), ref_depth));
}

/// Compute the shadow factor for the current fragment using 3-cascade CSM.
float compute_csm_shadow() {
    float view_dist = length(v_world_pos - ubo.camera_pos.xyz);
    int cascade = 0;
    if (view_dist >= ubo.cascade_splits.y) {
        cascade = 2;
    } else if (view_dist >= ubo.cascade_splits.x) {
        cascade = 1;
    }

    vec4 light_pos = ubo.light_vp[cascade] * vec4(v_world_pos, 1.0);
    return sample_cascade_shadow(light_pos, cascade);
}

// PBR light contribution for a single light (Cook-Torrance BRDF).
vec3 compute_light_contribution(
    vec3 N, vec3 V, vec3 L, vec3 T, vec3 B,
    vec3 light_color, float intensity,
    vec3 base_color, vec3 F0,
    float roughness, float metallic,
    float atten
) {
    vec3 H = normalize(V + L);
    float NDF = distribution_ggx_anisotropic(
        N, H, T, B, roughness, material.advanced0.w
    );
    float G = geometry_smith(N, V, L, roughness);
    vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);
    vec3 kS = F;
    vec3 kD = (1.0 - kS) * (1.0 - metallic);
    vec3 numerator = NDF * G * F;
    float denominator = 4.0 * max(dot(N, V), 0.0) * max(dot(N, L), 0.0) + 0.0001;
    vec3 specular = numerator / denominator;
    float NdotL = max(dot(N, L), 0.0);
    vec3 diffuse = kD * base_color / PI;

    float coat_roughness = clamp(material.advanced0.y, 0.04, 1.0);
    float coat_ndf = distribution_ggx(N, H, coat_roughness);
    float coat_geometry = geometry_smith(N, V, L, coat_roughness);
    float coat_fresnel = 0.04 + 0.96
        * pow(clamp(1.0 - dot(H, V), 0.0, 1.0), 5.0);
    float clearcoat = material.advanced0.x
        * coat_ndf * coat_geometry * coat_fresnel / denominator;

    float wrapped_diffuse = max((dot(N, L) + 0.5) / 1.5, 0.0);
    vec3 subsurface = material.subsurface_color.rgb
        * base_color
        * material.advanced0.z
        * wrapped_diffuse
        / PI;
    vec3 sheen = material.sheen_color.rgb
        * pow(clamp(1.0 - dot(H, V), 0.0, 1.0), 5.0)
        * NdotL;
    vec3 base_lobes = (diffuse + specular + vec3(clearcoat)) * NdotL;
    return (base_lobes + subsurface + sheen)
        * light_color * intensity * atten;
}

void main() {
    bool weighted_oit = material.alpha_cutoff <= -1.5;
    vec3 N = normalize(v_normal);
    if (!gl_FrontFacing) {
        N = -N;
    }
    mat3 surface_frame = cotangent_frame(N, v_world_pos, v_uv);
    uint texture_flags = uint(material.emissive.a + 0.5);
    if ((texture_flags & 2u) != 0u) {
        vec3 tangent_normal = texture(u_normal_texture, v_uv).xyz * 2.0 - 1.0;
        N = normalize(surface_frame * tangent_normal);
    }
    vec3 T = normalize(surface_frame[0]);
    vec3 B = normalize(cross(N, T));
    T = normalize(cross(B, N));
    vec3 V = normalize(ubo.camera_pos.xyz - v_world_pos);

    // A valid descriptor is always bound. Materials without a texture use the
    // renderer's 1x1 white fallback, so black texture data stays black.
    vec4 sampled_base_color = material.base_color
                            * texture(u_base_color_texture, v_uv);
#ifdef VFX_PARTICLE
    sampled_base_color *= v_particle_color;
#endif
    if (material.alpha_cutoff >= 0.0 && sampled_base_color.a < material.alpha_cutoff) {
        discard;
    }
    vec3 base_color = sampled_base_color.rgb;
    float metallic_value = material.metallic;
    float roughness_value = material.roughness;
    float ao_value = material.ao;
    vec3 emissive_color = material.emissive.rgb;
    if ((texture_flags & 4u) != 0u) {
        vec4 metallic_roughness = texture(u_metallic_roughness_texture, v_uv);
        roughness_value *= metallic_roughness.g;
        metallic_value *= metallic_roughness.b;
    }
    roughness_value = clamp(roughness_value, 0.04, 1.0);
    if ((texture_flags & 8u) != 0u) {
        ao_value *= texture(u_occlusion_texture, v_uv).r;
    }
    if ((texture_flags & 16u) != 0u) {
        emissive_color *= texture(u_emissive_texture, v_uv).rgb;
    }

    vec3 F0 = mix(vec3(0.04), base_color, metallic_value);

    // CSM shadow factor for the UBO directional light
    float shadow = compute_csm_shadow();

    // --- Directional light from UBO ---
    vec3 L_dir = normalize(-ubo.light_dir.xyz);
    vec3 Lo = compute_light_contribution(
        N, V, L_dir, T, B,
        ubo.light_color.rgb, ubo.light_color.a,
        base_color, F0,
        roughness_value, metallic_value,
        1.0
    ) * shadow;

    // --- Additional lights from the fragment's screen/depth cluster ---
    vec2 local_pixel = gl_FragCoord.xy - u_light_ssbo.cluster_viewport.xy;
    vec2 normalized_pixel = clamp(
        local_pixel / max(u_light_ssbo.cluster_viewport.zw, vec2(1.0)),
        vec2(0.0),
        vec2(0.999999)
    );
    uvec2 tile_count = max(u_light_ssbo.cluster_dimensions.yz, uvec2(1u));
    uvec2 tile = min(uvec2(normalized_pixel * vec2(tile_count)), tile_count - 1u);
    float cluster_distance = clamp(
        length(v_world_pos - ubo.camera_pos.xyz),
        u_light_ssbo.cluster_depth.x,
        u_light_ssbo.cluster_depth.y
    );
    uint depth_slice = uint(clamp(
        floor(log(cluster_distance) * u_light_ssbo.cluster_depth.z
            + u_light_ssbo.cluster_depth.w),
        0.0,
        float(max(u_light_ssbo.cluster_dimensions.w, 1u) - 1u)
    ));
    uint cluster_index = (depth_slice * tile_count.y + tile.y) * tile_count.x + tile.x;
    uvec2 cluster = u_cluster_grid.clusters[
        min(cluster_index, max(u_light_ssbo.cluster_stats.x, 1u) - 1u)
    ];
    uint cluster_light_count = min(cluster.y, u_light_ssbo.cluster_stats.z);
    for (uint cluster_light = 0u; cluster_light < cluster_light_count; ++cluster_light) {
        uint list_index = cluster.x + cluster_light;
        if (list_index >= u_light_ssbo.cluster_stats.y) {
            break;
        }
        uint light_index = u_cluster_indices.light_indices[list_index];
        if (light_index >= u_light_ssbo.cluster_dimensions.x) {
            continue;
        }
        Light lt = u_light_ssbo.lights[light_index];
        float light_type = lt.position.w;

        vec3 L;
        float atten = 1.0;

        if (light_type < 0.5) {
            // Directional light (no per-light shadow for SSBO lights)
            L = normalize(-lt.direction.xyz);
        } else {
            // Point or spot light
            vec3 to_light = lt.position.xyz - v_world_pos;
            float distance = length(to_light);
            L = to_light / distance;

            // Range check
            float range = lt.attenuation.x;
            if (range > 0.0 && distance > range) {
                continue;
            }

            // Attenuation: 1 / (1 + linear*d + quadratic*d^2)
            atten = 1.0 / (1.0 + lt.attenuation.y * distance + lt.attenuation.z * distance * distance);

            // Spot cone
            if (light_type > 1.5) {
                float spot_cutoff_cos = lt.attenuation.w;
                float spot_dir = dot(normalize(-lt.direction.xyz), -L);
                if (spot_dir < spot_cutoff_cos) {
                    continue;
                }
                atten *= smoothstep(spot_cutoff_cos, 1.0, spot_dir);
            }
        }

        Lo += compute_light_contribution(
            N, V, L, T, B,
            lt.color.rgb, lt.color.a,
            base_color, F0,
            roughness_value, metallic_value,
            atten
        );
    }

    // IBL ambient: diffuse (irradiance) + specular (prefiltered env map)
    float environment_max_lod = float(max(textureQueryLevels(u_irradiance_map), 1) - 1);
    vec3 irradiance = textureLod(
        u_irradiance_map,
        environment_direction(N),
        environment_max_lod
    ).rgb;
    vec3 diffuse_ibl = (1.0 - F0) * (1.0 - metallic_value) * irradiance * base_color;

    vec3 R = reflect(-V, N);
    vec3 specular_ibl = textureLod(
        u_irradiance_map,
        environment_direction(R),
        roughness_value * environment_max_lod
    ).rgb * F0;

    vec3 ambient = (diffuse_ibl + specular_ibl)
                 * ao_value
                 * ubo.environment_params.x;

    float rim = pow(
        clamp(1.0 - max(dot(N, V), 0.0), 0.0, 1.0),
        material.rim_color_power.a
    );
    vec3 rim_lighting = material.rim_color_power.rgb * rim;

    vec3 lit_color = Lo + ambient + emissive_color + rim_lighting;
    out_color = vec4(lit_color, sampled_base_color.a);
    out_oit_accumulation = vec4(0.0);
    out_oit_optical_depth = vec4(0.0);
    if (weighted_oit) {
        float alpha = clamp(sampled_base_color.a, 0.0, 1.0);
        float depth_weight = pow(clamp(1.0 - gl_FragCoord.z * 0.9, 0.0, 1.0), 3.0);
        float weight = clamp(
            pow(alpha + 0.01, 3.0) * 10000.0 * depth_weight,
            0.01,
            3000.0
        );
        out_color = vec4(0.0);
        out_oit_accumulation = vec4(
            lit_color * alpha * weight,
            alpha * weight
        );
        out_oit_optical_depth = vec4(
            -log(max(1.0 - alpha, 0.0001)),
            0.0,
            0.0,
            0.0
        );
    }
}
