#version 450
layout(location = 0) in vec2 in_uv;
layout(location = 0) out vec4 out_color;
layout(binding = 0) uniform sampler2D hdr_input;

layout(push_constant) uniform ToneMapPushConstants {
    uint mode;
    float exposure;
    uint output_is_srgb;
    uint _padding;
} tone_map;

const uint TONE_MAP_ACES = 0u;
const uint TONE_MAP_REINHARD = 1u;
const uint TONE_MAP_NONE = 2u;

vec3 aces_narkowicz(vec3 color) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp(
        (color * (a * color + b)) / (color * (c * color + d) + e),
        vec3(0.0),
        vec3(1.0)
    );
}

vec3 linear_to_srgb(vec3 color) {
    bvec3 use_linear_segment = lessThanEqual(color, vec3(0.0031308));
    vec3 linear_segment = color * 12.92;
    vec3 power_segment = 1.055 * pow(color, vec3(1.0 / 2.4)) - 0.055;
    return mix(power_segment, linear_segment, use_linear_segment);
}

void main() {
    vec3 color = max(texture(hdr_input, in_uv).rgb * tone_map.exposure, vec3(0.0));

    if (tone_map.mode == TONE_MAP_ACES) {
        color = aces_narkowicz(color);
    } else if (tone_map.mode == TONE_MAP_REINHARD) {
        color = color / (color + vec3(1.0));
    } else if (tone_map.mode == TONE_MAP_NONE) {
        color = color;
    }

    // An sRGB attachment performs the transfer function after fragment output.
    // UNORM fallback targets need an explicit linear-to-sRGB conversion.
    if (tone_map.output_is_srgb == 0u) {
        color = linear_to_srgb(color);
    }

    out_color = vec4(color, 1.0);
}
