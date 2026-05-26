#version 450

layout(location = 0) in vec4 in_color;
layout(location = 1) in vec3 in_normal;

layout(push_constant) uniform PushConstants {
    mat4 model;
    vec4 light_dir;
    vec4 light_color;
    vec4 ambient;
} pc;

layout(location = 0) out vec4 out_color;

void main() {
    vec3 n = normalize(in_normal);
    vec3 l = normalize(pc.light_dir.xyz);
    float ndotl = max(dot(n, l), 0.0);
    vec3 amb = pc.ambient.rgb * pc.ambient.a;
    vec3 lit = pc.light_color.rgb * pc.light_color.a;
    vec3 color = in_color.rgb * (amb + lit * ndotl);
    out_color = vec4(color, in_color.a);
}
