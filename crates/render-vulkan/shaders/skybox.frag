#version 450

layout(set = 1, binding = 1) uniform samplerCube u_environment_map;

layout(location = 0) in vec3 v_direction;
layout(location = 0) out vec4 out_color;

void main() {
    out_color = vec4(texture(u_environment_map, normalize(v_direction)).rgb, 1.0);
}
