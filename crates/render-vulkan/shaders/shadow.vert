#version 450
layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_normal;
layout(push_constant) uniform PC {
    mat4 mvp;
    vec4 radial_morph;
    vec4 morph_origin;
} pc;
void main() {
    vec3 local_position = in_position;
    if (pc.radial_morph.z > 0.5 && pc.radial_morph.x > 0.0) {
        vec3 radial = local_position - pc.morph_origin.xyz;
        float radial_length = length(radial);
        float encoded_normal_length = length(in_normal);
        if (radial_length > 1.0e-6 && encoded_normal_length > 1.0e-6) {
            float delta = (encoded_normal_length - 1.0)
                * pc.radial_morph.y
                * clamp(pc.radial_morph.x, 0.0, 1.0);
            local_position += radial * (delta / radial_length);
        }
    }
    gl_Position = pc.mvp * vec4(local_position, 1.0);
}
