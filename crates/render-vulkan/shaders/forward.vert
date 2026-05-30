#version 450

// Forward-rendering vertex shader.
// Reads from the per-frame UBO (set=0, binding=0) and passes world-space
// position + normal to the fragment shader for per-pixel lighting.

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec2 in_uv;

// Per-frame UBO — written by write_default_ubo() / write_ubo() each frame.
// Layout (std140):
//   offset   0: mat4 model          (64 B)
//   offset  64: mat4 view_proj      (64 B)
//   offset 128: vec4 light_dir      (16 B)
//   offset 144: vec4 light_color    (16 B)
//   offset 160: vec4 camera_pos     (16 B)
//   offset 176: vec4 cascade_splits (16 B)  — x=split0, y=split1, z=split2, w=far
//   offset 192: mat4 light_vp[0]    (64 B)
//   offset 256: mat4 light_vp[1]    (64 B)
//   offset 320: mat4 light_vp[2]    (64 B)
// Total: 384 B  (fits in 512 B UBO)
layout(set = 0, binding = 0) uniform UBO {
    mat4 model;
    mat4 view_proj;
    vec4 light_dir;
    vec4 light_color;
    vec4 camera_pos;
    vec4 cascade_splits;
    mat4 light_vp[3];
} ubo;

layout(location = 0) out vec3 v_world_pos;
layout(location = 1) out vec3 v_normal;
layout(location = 2) out vec2 v_uv;

void main() {
    vec4 world_pos = ubo.model * vec4(in_position, 1.0);
    v_world_pos = world_pos.xyz;
    // Normal transform — assumes uniform scale (no inverse-transpose needed for MVP).
    v_normal = normalize(mat3(ubo.model) * in_normal);
    v_uv = in_uv;
    gl_Position = ubo.view_proj * world_pos;
}
