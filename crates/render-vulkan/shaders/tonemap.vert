#version 450
layout(location = 0) out vec2 out_uv;
void main() {
    vec2 p = vec2(gl_VertexIndex & 1, (gl_VertexIndex >> 1) & 1) * 4.0 - 1.0;
    gl_Position = vec4(p.x, -p.y, 0.0, 1.0);
    out_uv = p * 0.5 + 0.5;
}
