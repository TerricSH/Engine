#version 450

// Forward-rendering fragment shader with Cook-Torrance PBR BRDF.
// Metallic-roughness workflow with GGX normal distribution, Smith
// geometry function, and Fresnel-Schlick approximation.
// Per-frame UBO at set=0, binding=0 (matches descriptor.rs layout).

layout(location = 0) in vec3 v_world_pos;
layout(location = 1) in vec3 v_normal;
layout(location = 2) in vec2 v_uv;

layout(location = 0) out vec4 out_color;

layout(binding = 0) uniform PerFrameUBO {
    mat4 model;
    mat4 view_proj;
    vec4 light_dir;       // w=0 (directional)
    vec4 light_color;     // rgb = color, a = intensity
    vec4 camera_pos;
} ubo;

// Environment cubemap (set=1, binding=1) — IBL irradiance / prefiltered env
layout(binding = 1) uniform samplerCube u_irradiance_map;

// Material parameters (per-drawable, set=2 binding=0).
layout(set = 2, binding = 0) uniform MaterialUBO {
    vec4 base_color;
    float metallic;
    float roughness;
    float ao;
} material;

// Base color texture (set=2, binding=1) — optional. When bound, the sampled
// texel is multiplied with material.base_color to allow tinting. When no
// texture is bound, material.base_color is used directly.
layout(set = 2, binding = 1) uniform sampler2D u_base_color_texture;

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

void main() {
    vec3 N = normalize(v_normal);
    vec3 V = normalize(ubo.camera_pos.xyz - v_world_pos);

    // Compute base color from texture (if bound) or uniform fallback.
    // When a texture is present, sample it and tint with the uniform.
    vec3 base_color = material.base_color.rgb;
    // We cannot detect "is texture bound" in GLSL, but the driver returns
    // (0,0,0,1) for an unbound descriptor — we use uniform as fallback.
    // A more robust solution would use a push-constant or UBO flag.
    vec3 tex_color = texture(u_base_color_texture, v_uv).rgb;
    if (tex_color != vec3(0.0)) {
        base_color = tex_color * material.base_color.rgb;
    }

    vec3 F0 = mix(vec3(0.04), base_color, material.metallic);

    // Directional light
    vec3 L = normalize(-ubo.light_dir.xyz);
    vec3 H = normalize(V + L);

    float NDF = distribution_ggx(N, H, material.roughness);
    float G = geometry_smith(N, V, L, material.roughness);
    vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);

    vec3 kS = F;
    vec3 kD = (1.0 - kS) * (1.0 - material.metallic);

    vec3 numerator = NDF * G * F;
    float denominator = 4.0 * max(dot(N, V), 0.0) * max(dot(N, L), 0.0) + 0.0001;
    vec3 specular = numerator / denominator;

    float NdotL = max(dot(N, L), 0.0);
    vec3 diffuse = kD * base_color / PI;

    vec3 Lo = (diffuse + specular) * ubo.light_color.rgb * ubo.light_color.a * NdotL;

    // IBL ambient: diffuse (irradiance) + specular (prefiltered env map)
    vec3 irradiance = texture(u_irradiance_map, N).rgb;
    vec3 diffuse_ibl = kD * irradiance * base_color;

    vec3 R = reflect(-V, N);
    vec3 specular_ibl = textureLod(u_irradiance_map, R, material.roughness * MAX_REFLECTION_LOD).rgb * F;

    vec3 ambient = (diffuse_ibl + specular_ibl) * material.ao;

    out_color = vec4(Lo + ambient, 1.0);
}
