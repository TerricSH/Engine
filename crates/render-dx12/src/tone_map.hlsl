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
    if ((effect_flags & 8u) == 0u) {
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

float3 bloom_neighborhood(float2 uv) {
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
        float3 sample_color = resolved_hdr(
            saturate(uv + offsets[index] * radius)
        );
        float contribution = saturate(luminance(sample_color) - bloom.x);
        accumulated += sample_color * contribution;
    }
    return accumulated * (bloom.y / 8.0);
}

float4 ToneMapPSMain(ToneMapOutput input) : SV_TARGET {
    float3 color = resolved_hdr(input.uv) * exposure;
    if ((effect_flags & 1u) != 0u) {
        color += bloom_neighborhood(input.uv) * exposure;
    }
    if ((effect_flags & 2u) != 0u) {
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
    if ((effect_flags & 4u) != 0u) {
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
