#version 450

// Skinned-mesh vertex shader.
// Same per-frame UBO as forward.vert (set=0, binding=0), plus a per-drawable
// bone palette UBO (set=2, binding=2) that stores up to 64 bone matrices.
// The skinning matrix is computed as a weighted sum of bone transforms,
// then applied to the vertex position and normal.

layout(location = 0) in vec3  in_position;
layout(location = 1) in vec3  in_normal;
layout(location = 2) in vec2  in_uv;
layout(location = 3) in uvec4 in_joints;
layout(location = 4) in vec4  in_weights;

// Per-frame UBO — written by write_default_ubo() / write_ubo() each frame.
layout(set = 0, binding = 0) uniform UBO {
    mat4 model;
    mat4 view_proj;
    vec4 light_dir;
    vec4 light_color;
    vec4 camera_pos;
    mat4 light_view_proj;
} ubo;

// Bone palette — uploaded per skinned drawable (max 64 bones, 64 B each = 4096 B).
layout(set = 2, binding = 2) uniform BoneUBO {
    mat4 bones[64];
} bone_ubo;

layout(location = 0) out vec3 v_world_pos;
layout(location = 1) out vec3 v_normal;
layout(location = 2) out vec2 v_uv;

void main() {
    // Compute the skinning matrix as a weighted blend of bone transforms.
    mat4 skin_mat = in_weights.x * bone_ubo.bones[in_joints.x]
                  + in_weights.y * bone_ubo.bones[in_joints.y]
                  + in_weights.z * bone_ubo.bones[in_joints.z]
                  + in_weights.w * bone_ubo.bones[in_joints.w];

    // Transform vertex position into world space via the skinning matrix.
    vec4 world_pos = skin_mat * vec4(in_position, 1.0);
    v_world_pos = world_pos.xyz;

    // Transform normal (assumes uniform scale — no inverse-transpose needed).
    v_normal = normalize(mat3(skin_mat) * in_normal);

    v_uv = in_uv;

    gl_Position = ubo.view_proj * world_pos;
}
