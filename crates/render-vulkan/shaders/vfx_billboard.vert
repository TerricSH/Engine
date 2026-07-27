#version 450

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 i_position_size;
layout(location = 4) in vec4 i_rotation_age;

layout(location = 0) out vec3 v_world_pos;
layout(location = 1) out vec3 v_normal;
layout(location = 2) out vec2 v_uv;
layout(location = 3) out vec4 v_particle_color;

layout(binding = 0) uniform PerFrameUBO {
    mat4 model;
    mat4 view_proj;
    vec4 light_dir;
    vec4 light_color;
    vec4 camera_pos;
    vec4 cascade_splits;
    mat4 light_vp[3];
    vec4 environment_params;
} ubo;

layout(push_constant) uniform BillboardPush {
    vec4 camera_right;
    vec4 camera_up;
} billboard;

void main() {
    float rotation = i_rotation_age.x;
    float sine = sin(rotation);
    float cosine = cos(rotation);
    vec2 rotated = vec2(
        a_position.x * cosine - a_position.y * sine,
        a_position.x * sine + a_position.y * cosine
    ) * i_position_size.w;
    vec3 right = normalize(billboard.camera_right.xyz);
    vec3 up = normalize(billboard.camera_up.xyz);
    v_world_pos = i_position_size.xyz + right * rotated.x + up * rotated.y;
    v_normal = normalize(cross(right, up));
    v_uv = a_uv;
    v_particle_color = unpackUnorm4x8(floatBitsToUint(i_rotation_age.z));
    gl_Position = ubo.view_proj * vec4(v_world_pos, 1.0);
}
