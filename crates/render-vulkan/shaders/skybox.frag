#version 450

layout(set = 1, binding = 1) uniform samplerCube u_environment_map;
layout(set = 0, binding = 0) uniform UBO {
    mat4 model;
    mat4 view_proj;
    vec4 light_dir;
    vec4 light_color;
    vec4 camera_pos;
    vec4 cascade_splits;
    mat4 light_vp[3];
    vec4 environment_params;
} ubo;

layout(location = 0) in vec3 v_direction;
layout(location = 0) out vec4 out_color;

void main() {
    vec3 direction = normalize(v_direction);
    float rotation_sin = ubo.environment_params.y;
    float rotation_cos = ubo.environment_params.z;
    direction = vec3(
        rotation_cos * direction.x + rotation_sin * direction.z,
        direction.y,
        -rotation_sin * direction.x + rotation_cos * direction.z
    );
    out_color = vec4(
        texture(u_environment_map, direction).rgb * ubo.environment_params.x,
        1.0
    );
}
