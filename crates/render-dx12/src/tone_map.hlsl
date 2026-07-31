struct ToneMapOutput {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

cbuffer ToneMapSettings : register(b0) {
    uint tone_map_mode;
    float exposure;
    uint output_is_srgb;
    uint effect_flags;
    float4 bloom;
    float4 color_filter_saturation;
    float4 contrast;
    float4 lift;
    float4 gamma;
    float4 gain;
    float4 vignette;
};

Texture2D hdr_color : register(t0);
Texture2D oit_accumulation : register(t1);
Texture2D oit_optical_depth : register(t2);
SamplerState hdr_sampler : register(s0);
SamplerState oit_accumulation_sampler : register(s1);
SamplerState oit_optical_depth_sampler : register(s2);

static const uint EFFECT_BLOOM = 1u;
static const uint EFFECT_COLOR_GRADING = 2u;
static const uint EFFECT_VIGNETTE = 4u;
static const uint EFFECT_WEIGHTED_OIT = 8u;
static const uint EFFECT_PLANETARY_LENS = 16u;

ToneMapOutput ToneMapVSMain(uint vertex_id : SV_VertexID) {
    ToneMapOutput output;
    float2 position = float2(
        vertex_id == 2 ? 3.0 : -1.0,
        vertex_id == 1 ? 3.0 : -1.0
    );
    output.position = float4(position, 0.0, 1.0);
    output.uv = float2(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return output;
}

float luminance(float3 color) {
    return dot(color, float3(0.2126, 0.7152, 0.0722));
}

float3 aces_fitted(float3 color) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return saturate((color * (a * color + b)) / (color * (c * color + d) + e));
}

float3 linear_to_srgb(float3 color) {
    color = max(color, 0.0.xxx);
    float3 low = 12.92 * color;
    float3 high = 1.055 * pow(color, 1.0 / 2.4) - 0.055;
    return lerp(high, low, 1.0 - step(0.0031308.xxx, color));
}

float3 resolved_hdr(float2 uv) {
    float3 opaque = hdr_color.SampleLevel(hdr_sampler, uv, 0.0).rgb;
    if ((effect_flags & EFFECT_WEIGHTED_OIT) == 0u) {
        return opaque;
    }
    float4 accumulation = oit_accumulation.SampleLevel(
        oit_accumulation_sampler,
        uv,
        0.0
    );
    float optical_depth = oit_optical_depth.SampleLevel(
        oit_optical_depth_sampler,
        uv,
        0.0
    ).r;
    float3 transparent_color =
        accumulation.rgb / max(accumulation.a, 0.00001);
    float coverage = 1.0 - exp(-max(optical_depth, 0.0));
    return lerp(opaque, transparent_color, saturate(coverage));
}

float2 planetary_lens_uv(float2 uv) {
    float2 centered = uv * 2.0 - 1.0;
    float radius_squared = dot(centered, centered);
    centered *= 1.0 + contrast.y * radius_squared;
    centered.y += contrast.z
        * centered.x * centered.x
        * (1.0 - 0.25 * abs(centered.y));
    return centered * 0.5 + 0.5;
}

float3 resolved_planetary_lens(float2 uv) {
    float2 centered = uv - 0.5;
    float2 chromatic_offset = centered * vignette.w;
    float3 center_color = resolved_hdr(saturate(uv));
    return float3(
        resolved_hdr(saturate(uv + chromatic_offset)).r,
        center_color.g,
        resolved_hdr(saturate(uv - chromatic_offset)).b
    );
}

float3 resolved_source(float2 uv, bool planetary_lens) {
    return planetary_lens
        ? resolved_planetary_lens(uv)
        : resolved_hdr(uv);
}

float3 bloom_neighborhood(float2 uv, bool planetary_lens) {
    uint width;
    uint height;
    hdr_color.GetDimensions(width, height);
    float2 texel = 1.0 / float2(max(width, 1u), max(height, 1u));
    float2 radius = texel * max(bloom.z, 0.5);
    const float2 offsets[8] = {
        float2(-1.0, 0.0), float2(1.0, 0.0),
        float2(0.0, -1.0), float2(0.0, 1.0),
        float2(-0.707, -0.707), float2(0.707, -0.707),
        float2(-0.707, 0.707), float2(0.707, 0.707)
    };
    float3 accumulated = 0.0.xxx;
    [unroll]
    for (uint index = 0; index < 8; ++index) {
        float3 sample_color = resolved_source(
            saturate(uv + offsets[index] * radius),
            planetary_lens
        );
        float contribution = saturate(luminance(sample_color) - bloom.x);
        accumulated += sample_color * contribution;
    }
    return accumulated * (bloom.y / 8.0);
}

float4 ToneMapPSMain(ToneMapOutput input) : SV_TARGET {
    bool planetary_lens = (effect_flags & EFFECT_PLANETARY_LENS) != 0u;
    float2 sample_uv = planetary_lens
        ? planetary_lens_uv(input.uv)
        : input.uv;
    float3 hdr = resolved_source(sample_uv, planetary_lens);
    if ((effect_flags & EFFECT_BLOOM) != 0u) {
        hdr += bloom_neighborhood(sample_uv, planetary_lens);
    }
    if (planetary_lens) {
        float2 centered = input.uv * 2.0 - 1.0;
        float limb = smoothstep(0.35, 1.15, length(centered));
        hdr += float3(0.08, 0.24, 0.55) * contrast.w * limb;
    }
    float3 color = hdr * exposure;
    if ((effect_flags & EFFECT_COLOR_GRADING) != 0u) {
        color = max(color + lift.rgb, 0.0.xxx);
        color = pow(color, 1.0 / max(gamma.rgb, 0.0001.xxx)) * gain.rgb;
        float gray = luminance(color);
        color = lerp(gray.xxx, color, color_filter_saturation.w);
        color = (color - 0.5) * contrast.x + 0.5;
        color *= color_filter_saturation.rgb;
    }
    if (tone_map_mode == 0u) {
        color = aces_fitted(color);
    } else if (tone_map_mode == 1u) {
        color = color / (1.0 + color);
    }
    if ((effect_flags & EFFECT_VIGNETTE) != 0u) {
        float2 centered = abs(input.uv * 2.0 - 1.0);
        float radial = lerp(max(centered.x, centered.y), length(centered), vignette.z);
        float edge = smoothstep(
            max(0.0, 1.0 - vignette.y),
            1.0,
            radial
        );
        color *= 1.0 - saturate(vignette.x) * edge;
    }
    if (output_is_srgb == 0u) {
        color = linear_to_srgb(color);
    }
    return float4(color, 1.0);
}
