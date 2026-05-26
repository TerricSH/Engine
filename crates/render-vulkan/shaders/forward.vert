#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec4 in_color;

layout(push_constant) uniform PushConstants {
    mat4 model;
    vec4 light_dir;
    vec4 light_color;
    vec4 ambient;
} pc;

layout(location = 0) out vec4 out_color;
layout(location = 1) out vec3 out_normal;

void main() {
    gl_Position = pc.model * vec4(in_position, 1.0);
    out_color = in_color;
}
