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

layout(location = 0) out vec4 out_color;

layout(binding = 0) uniform PerFrameUBO {
    mat4 model;
    mat4 view_proj;
    vec4 light_dir;         // w=0 (directional)
    vec4 light_color;       // rgb = color, a = intensity
    vec4 camera_pos;
    vec4 cascade_splits;    // x=split0, y=split1, z=split2, w=far
    mat4 light_vp[3];       // 3 cascade light VP matrices
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
    Light lights[];
} u_light_ssbo;

// Material parameters (per-drawable, set=2 binding=0).
layout(set = 2, binding = 0) uniform MaterialUBO {
    vec4 base_color;
    float metallic;
    float roughness;
    float ao;
    float alpha_cutoff;
    vec4 emissive;
} material;

// Base color texture (set=2, binding=1) — optional.
layout(set = 2, binding = 1) uniform sampler2D u_base_color_texture;
layout(set = 2, binding = 3) uniform sampler2D u_normal_texture;
layout(set = 2, binding = 4) uniform sampler2D u_metallic_roughness_texture;
layout(set = 2, binding = 5) uniform sampler2D u_occlusion_texture;
layout(set = 2, binding = 6) uniform sampler2D u_emissive_texture;

const float PI = 3.14159265359;
const float MAX_REFLECTION_LOD = 4.0;

// Normal Distribution Function — GGX / Trowbridge-Reitz
float distribution_ggx(vec3 N, vec3 H, float roughness) {
    float a = roughness * roughness;
    float a2 = a * a;
    float NdotH = max(dot(N, H), 0.0);
    float NdotH2 = NdotH * NdotH;
    float denom = NdotH2 * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
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
    vec3 N, vec3 V, vec3 L,
    vec3 light_color, float intensity,
    vec3 base_color, vec3 F0,
    float roughness, float metallic,
    float atten
) {
    vec3 H = normalize(V + L);
    float NDF = distribution_ggx(N, H, roughness);
    float G = geometry_smith(N, V, L, roughness);
    vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);
    vec3 kS = F;
    vec3 kD = (1.0 - kS) * (1.0 - metallic);
    vec3 numerator = NDF * G * F;
    float denominator = 4.0 * max(dot(N, V), 0.0) * max(dot(N, L), 0.0) + 0.0001;
    vec3 specular = numerator / denominator;
    float NdotL = max(dot(N, L), 0.0);
    vec3 diffuse = kD * base_color / PI;
    return (diffuse + specular) * light_color * intensity * NdotL * atten;
}

void main() {
    vec3 N = normalize(v_normal);
    if (!gl_FrontFacing) {
        N = -N;
    }
    uint texture_flags = uint(material.emissive.a + 0.5);
    if ((texture_flags & 2u) != 0u) {
        vec3 tangent_normal = texture(u_normal_texture, v_uv).xyz * 2.0 - 1.0;
        N = normalize(cotangent_frame(N, v_world_pos, v_uv) * tangent_normal);
    }
    vec3 V = normalize(ubo.camera_pos.xyz - v_world_pos);

    // A valid descriptor is always bound. Materials without a texture use the
    // renderer's 1x1 white fallback, so black texture data stays black.
    vec4 sampled_base_color = material.base_color
                            * texture(u_base_color_texture, v_uv);
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
        N, V, L_dir,
        ubo.light_color.rgb, ubo.light_color.a,
        base_color, F0,
        roughness_value, metallic_value,
        1.0
    ) * shadow;

    // --- Additional lights from SSBO (point, spot, extra directional) ---
    int num_lights = u_light_ssbo.lights.length();
    for (int i = 0; i < num_lights; i++) {
        Light lt = u_light_ssbo.lights[i];
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
            N, V, L,
            lt.color.rgb, lt.color.a,
            base_color, F0,
            roughness_value, metallic_value,
            atten
        );
    }

    // IBL ambient: diffuse (irradiance) + specular (prefiltered env map)
    vec3 irradiance = texture(u_irradiance_map, N).rgb;
    vec3 diffuse_ibl = (1.0 - F0) * (1.0 - metallic_value) * irradiance * base_color;

    vec3 R = reflect(-V, N);
    vec3 specular_ibl = textureLod(u_irradiance_map, R, roughness_value * MAX_REFLECTION_LOD).rgb * F0;

    vec3 ambient = (diffuse_ibl + specular_ibl) * ao_value;

    out_color = vec4(Lo + ambient + emissive_color, sampled_base_color.a);
}
