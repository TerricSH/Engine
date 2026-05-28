#version 450

// Forward-rendering fragment shader.
// Uses the same per-frame UBO (set=0, binding=0) as the vertex shader.
// Computes a simple NdotL diffuse term with a hardcoded albedo.
// The albedo will be driven by a material texture in a later step.

layout(location = 0) in vec3 in_world_pos;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec2 in_uv;

layout(set = 0, binding = 0) uniform UBO {
    mat4 model;
    mat4 view_proj;
    vec4 light_dir;
    vec4 light_color;
    vec4 camera_pos;
    // Light view-projection for shadow mapping (written by render_model_frame).
    mat4 light_view_proj;
} ubo;

layout(set = 1, binding = 0) uniform sampler2DShadow u_shadow_map;

layout(location = 0) out vec4 out_color;

// Hardcoded albedo (will be replaced by material sampling later).
const vec3 ALBEDO = vec3(0.8, 0.6, 0.4);

// Bias to reduce shadow acne.
const float SHADOW_BIAS = 0.005;

/// Compute PCF shadow factor in [0, 1] (1 = fully lit, 0 = fully shadowed).
float shadow_factor(vec3 world_pos) {
    vec4 light_space = ubo.light_view_proj * vec4(world_pos, 1.0);
    vec3 proj_coords = light_space.xyz / light_space.w;
    // Map from [-1, 1] to [0, 1] for texture sampling.
    proj_coords = proj_coords * 0.5 + 0.5;
    proj_coords.z -= SHADOW_BIAS;
    // sampler2DShadow automatically compares proj_coords.z against the stored
    // depth value using the sampler's COMPARE_OP; returns PCF-weighted average.
    return texture(u_shadow_map, proj_coords);
}

void main() {
    vec3 N = normalize(in_normal);
    vec3 L = normalize(ubo.light_dir.xyz);

    // Diffuse lighting (NdotL clamped to [0, 1]).
    float ndotl = max(dot(N, L), 0.0);

    // Simple ambient term so back-faces are not completely black.
    const float AMBIENT = 0.08;

    float shadow = shadow_factor(in_world_pos);
    vec3 lit = ALBEDO * (AMBIENT + ndotl * ubo.light_color.a * shadow);

    // Output straight RGBA (swapchain format is BGRA8, but the shader
    // writes RGBA and the Vulkan swizzle handles the channel mapping).
    out_color = vec4(lit, 1.0);
}
