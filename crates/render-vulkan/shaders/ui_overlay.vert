#version 450

// UI overlay vertex shader.
// Layout matches the UiVertex format converted to float:
//   position: vec2 (offset 0)
//   uv:       vec2 (offset 8)
//   color:    vec4 (offset 16)
// Total stride: 32 bytes

layout(location = 0) in vec2 in_position;
layout(location = 1) in vec2 in_uv;
layout(location = 2) in vec4 in_color;

layout(location = 0) out vec2 out_uv;
layout(location = 1) out vec4 out_color;

// Push constants: screen dimensions for NDC conversion
layout(push_constant) uniform PushConstants {
    vec2 screen_size;
} pc;

void main() {
    float x = (in_position.x / pc.screen_size.x) * 2.0 - 1.0;
    float y = -(in_position.y / pc.screen_size.y) * 2.0 + 1.0;
    gl_Position = vec4(x, y, 0.0, 1.0);
    out_uv = in_uv;
    out_color = in_color;
}
