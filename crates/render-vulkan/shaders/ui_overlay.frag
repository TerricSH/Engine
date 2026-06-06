#version 450

// UI overlay fragment shader.
// Outputs vertex color (texture-less fallback for first-pass UI).

layout(location = 1) in vec4 out_color;

layout(location = 0) out vec4 frag_color;

void main() {
    frag_color = out_color;
}
