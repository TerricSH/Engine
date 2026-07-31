#version 450
layout(location = 0) in vec2 in_uv;
layout(location = 0) out vec4 out_color;
layout(binding = 0) uniform sampler2D hdr_input;
layout(binding = 1) uniform sampler2D oit_accumulation;
layout(binding = 2) uniform sampler2D oit_optical_depth;

layout(push_constant) uniform ToneMapPushConstants {
    uint mode;
    float exposure;
    uint output_is_srgb;
    uint effect_flags;
    vec4 bloom;                    // threshold, intensity, radius in pixels
    vec4 color_filter_saturation;  // rgb filter, saturation
    vec4 contrast;                 // contrast, lens barrel, curvature, atmosphere
    vec4 lift;
    vec4 gamma;
    vec4 gain;
    vec4 vignette;                 // intensity, smoothness, roundness, chromatic aberration
} tone_map;

const uint TONE_MAP_ACES = 0u;
const uint TONE_MAP_REINHARD = 1u;
const uint TONE_MAP_NONE = 2u;
const uint EFFECT_BLOOM = 1u;
const uint EFFECT_COLOR_GRADING = 2u;
const uint EFFECT_VIGNETTE = 4u;
const uint EFFECT_WEIGHTED_OIT = 8u;
const uint EFFECT_PLANETARY_LENS = 16u;

vec3 resolved_hdr(vec2 uv) {
    vec3 opaque = texture(hdr_input, uv).rgb;
    if ((tone_map.effect_flags & EFFECT_WEIGHTED_OIT) == 0u) {
        return opaque;
    }
    vec4 accumulation = texture(oit_accumulation, uv);
    float optical_depth = texture(oit_optical_depth, uv).r;
    vec3 transparent_color =
        accumulation.rgb / max(accumulation.a, 0.00001);
    float coverage = 1.0 - exp(-max(optical_depth, 0.0));
    return mix(opaque, transparent_color, clamp(coverage, 0.0, 1.0));
}

vec2 planetary_lens_uv(vec2 uv) {
    vec2 centered = uv * 2.0 - 1.0;
    float radius_squared = dot(centered, centered);
    centered *= 1.0 + tone_map.contrast.y * radius_squared;
    centered.y += tone_map.contrast.z
        * centered.x * centered.x
        * (1.0 - 0.25 * abs(centered.y));
    return centered * 0.5 + 0.5;
}

vec3 resolved_planetary_lens(vec2 uv) {
    vec2 centered = uv - 0.5;
    vec2 chromatic_offset = centered * tone_map.vignette.w;
    vec3 center_color = resolved_hdr(clamp(uv, vec2(0.0), vec2(1.0)));
    return vec3(
        resolved_hdr(clamp(uv + chromatic_offset, vec2(0.0), vec2(1.0))).r,
        center_color.g,
        resolved_hdr(clamp(uv - chromatic_offset, vec2(0.0), vec2(1.0))).b
    );
}

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
    bool planetary_lens = (tone_map.effect_flags & EFFECT_PLANETARY_LENS) != 0u;
    vec2 sample_uv = planetary_lens ? planetary_lens_uv(in_uv) : in_uv;
    vec3 hdr_color = planetary_lens
        ? resolved_planetary_lens(sample_uv)
        : resolved_hdr(sample_uv);
    if ((tone_map.effect_flags & EFFECT_BLOOM) != 0u) {
        vec2 texel = tone_map.bloom.z / vec2(textureSize(hdr_input, 0));
        const vec2 OFFSETS[8] = vec2[](
            vec2(-1.0, -1.0), vec2(0.0, -1.0), vec2(1.0, -1.0),
            vec2(-1.0,  0.0),                    vec2(1.0,  0.0),
            vec2(-1.0,  1.0), vec2(0.0,  1.0), vec2(1.0,  1.0)
        );
        vec3 bloom_color = vec3(0.0);
        for (int sample_index = 0; sample_index < 8; ++sample_index) {
            vec3 sample_color = planetary_lens
                ? resolved_planetary_lens(sample_uv + OFFSETS[sample_index] * texel)
                : resolved_hdr(sample_uv + OFFSETS[sample_index] * texel);
            float brightness = max(max(sample_color.r, sample_color.g), sample_color.b);
            float contribution = max(brightness - tone_map.bloom.x, 0.0)
                               / max(brightness, 0.0001);
            bloom_color += sample_color * contribution;
        }
        hdr_color += bloom_color * (tone_map.bloom.y / 8.0);
    }
    if (planetary_lens) {
        vec2 centered = in_uv * 2.0 - 1.0;
        float limb = smoothstep(0.35, 1.15, length(centered));
        hdr_color += vec3(0.08, 0.24, 0.55)
            * tone_map.contrast.w
            * limb;
    }

    vec3 color = max(hdr_color * tone_map.exposure, vec3(0.0));

    if (tone_map.mode == TONE_MAP_ACES) {
        color = aces_narkowicz(color);
    } else if (tone_map.mode == TONE_MAP_REINHARD) {
        color = color / (color + vec3(1.0));
    } else if (tone_map.mode == TONE_MAP_NONE) {
        color = color;
    }

    if ((tone_map.effect_flags & EFFECT_COLOR_GRADING) != 0u) {
        color *= tone_map.color_filter_saturation.rgb;
        float luminance = dot(color, vec3(0.2126, 0.7152, 0.0722));
        color = mix(vec3(luminance), color, tone_map.color_filter_saturation.a);
        color = (color - 0.5) * tone_map.contrast.x + 0.5;
        color = max(color + tone_map.lift.rgb, vec3(0.0));
        color = pow(color, vec3(1.0) / max(tone_map.gamma.rgb, vec3(0.0001)));
        color *= tone_map.gain.rgb;
    }

    if ((tone_map.effect_flags & EFFECT_VIGNETTE) != 0u) {
        vec2 centered = abs(in_uv * 2.0 - 1.0);
        float box_distance = max(centered.x, centered.y);
        float circle_distance = length(centered) * 0.70710678;
        float distance_to_center = mix(
            box_distance,
            circle_distance,
            tone_map.vignette.z
        );
        float vignette_mask = 1.0 - smoothstep(
            1.0 - tone_map.vignette.y,
            1.0,
            distance_to_center
        );
        color *= mix(1.0, vignette_mask, tone_map.vignette.x);
    }

    // An sRGB attachment performs the transfer function after fragment output.
    // UNORM fallback targets need an explicit linear-to-sRGB conversion.
    if (tone_map.output_is_srgb == 0u) {
        color = linear_to_srgb(color);
    }

    out_color = vec4(color, 1.0);
}
