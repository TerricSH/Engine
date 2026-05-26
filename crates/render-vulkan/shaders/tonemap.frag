#version 450
layout(location = 0) in vec2 in_uv;
layout(location = 0) out vec4 out_color;
layout(binding = 0) uniform sampler2D hdr_input;
void main() {
    vec3 c = texture(hdr_input, in_uv).rgb;
    c = c / (c + vec3(1.0));
    out_color = vec4(pow(c, vec3(1.0/2.2)), 1.0);
}
