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

// The model matrix is per draw. Keeping it in push constants avoids sharing
// one mutable UBO value across every drawable recorded in the frame.
layout(push_constant) uniform DrawPush {
    mat4 model;
    // x=factor, y=encoded delta scale, z=enabled
    vec4 radial_morph;
    vec4 morph_origin;
    // x=enabled, y=inverse world-space tile size, z=blend sharpness
    vec4 material_mapping;
    vec4 mapping_origin;
} draw;

layout(location = 0) out vec3 v_world_pos;
layout(location = 1) out vec3 v_normal;
layout(location = 2) out vec2 v_uv;
layout(location = 3) out vec3 v_mapping_position;
layout(location = 4) flat out vec3 v_mapping_parameters;

void main() {
    vec3 local_position = in_position;
    if (draw.radial_morph.z > 0.5 && draw.radial_morph.x > 0.0) {
        vec3 radial = local_position - draw.morph_origin.xyz;
        float radial_length = length(radial);
        float encoded_normal_length = length(in_normal);
        if (radial_length > 1.0e-6 && encoded_normal_length > 1.0e-6) {
            float delta = (encoded_normal_length - 1.0)
                * draw.radial_morph.y
                * clamp(draw.radial_morph.x, 0.0, 1.0);
            local_position += radial * (delta / radial_length);
        }
    }
    vec4 world_pos = draw.model * vec4(local_position, 1.0);
    v_world_pos = world_pos.xyz;
    // Normal transform — assumes uniform scale (no inverse-transpose needed for MVP).
    v_normal = normalize(mat3(draw.model) * normalize(in_normal));
    v_uv = in_uv;
    v_mapping_position = mat3(draw.model)
        * (local_position - draw.mapping_origin.xyz);
    v_mapping_parameters = draw.material_mapping.xyz;
    gl_Position = ubo.view_proj * world_pos;
}
