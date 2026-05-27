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
} ubo;

layout(location = 0) out vec4 out_color;

// Hardcoded albedo (will be replaced by material sampling later).
const vec3 ALBEDO = vec3(0.8, 0.6, 0.4);

void main() {
    vec3 N = normalize(in_normal);
    vec3 L = normalize(ubo.light_dir.xyz);

    // Diffuse lighting (NdotL clamped to [0, 1]).
    float ndotl = max(dot(N, L), 0.0);

    // Simple ambient term so back-faces are not completely black.
    const float AMBIENT = 0.08;

    vec3 lit = ALBEDO * (AMBIENT + ndotl * ubo.light_color.a);

    // Output straight RGBA (swapchain format is BGRA8, but the shader
    // writes RGBA and the Vulkan swizzle handles the channel mapping).
    out_color = vec4(lit, 1.0);
}
