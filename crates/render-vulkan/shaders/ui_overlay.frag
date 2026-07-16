#version 450

// UI overlay fragment shader. Texture-less UI batches bind the renderer's
// opaque white fallback texture, keeping one deterministic shader path.

layout(set = 0, binding = 0) uniform sampler2D ui_texture;

layout(location = 0) in vec2 out_uv;
layout(location = 1) in vec4 out_color;

layout(location = 0) out vec4 frag_color;

void main() {
    frag_color = texture(ui_texture, out_uv) * out_color;
}
