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
    vec4 cascade_splits;
    mat4 light_vp[3];
} ubo;

layout(push_constant) uniform DrawPush {
    mat4 model;
    vec4 morph_weights[2];
    uvec4 morph_info; // target count, vertex count
} draw;

// Bone palette — uploaded per skinned drawable (max 64 bones, 64 B each = 4096 B).
layout(set = 2, binding = 2) uniform BoneUBO {
    mat4 bones[64];
} bone_ubo;

struct MorphDelta {
    vec4 position;
    vec4 normal;
};
layout(std430, set = 2, binding = 7) readonly buffer MorphTargetSSBO {
    MorphDelta deltas[];
} morph_targets;

layout(location = 0) out vec3 v_world_pos;
layout(location = 1) out vec3 v_normal;
layout(location = 2) out vec2 v_uv;

void main() {
    vec3 morphed_position = in_position;
    vec3 morphed_normal = in_normal;
    uint target_count = min(draw.morph_info.x, 8u);
    for (uint target = 0u; target < target_count; ++target) {
        float weight = target < 4u
            ? draw.morph_weights[0][target]
            : draw.morph_weights[1][target - 4u];
        uint delta_index = target * draw.morph_info.y + uint(gl_VertexIndex);
        morphed_position += morph_targets.deltas[delta_index].position.xyz * weight;
        morphed_normal += morph_targets.deltas[delta_index].normal.xyz * weight;
    }

    // Compute the skinning matrix as a weighted blend of bone transforms.
    mat4 skin_mat = in_weights.x * bone_ubo.bones[in_joints.x]
                  + in_weights.y * bone_ubo.bones[in_joints.y]
                  + in_weights.z * bone_ubo.bones[in_joints.z]
                  + in_weights.w * bone_ubo.bones[in_joints.w];

    // Transform vertex position into world space via the skinning matrix.
    mat4 model_skin = draw.model * skin_mat;
    vec4 world_pos = model_skin * vec4(morphed_position, 1.0);
    v_world_pos = world_pos.xyz;

    // Transform normal (assumes uniform scale — no inverse-transpose needed).
    v_normal = normalize(mat3(model_skin) * normalize(morphed_normal));

    v_uv = in_uv;

    gl_Position = ubo.view_proj * world_pos;
}
