#version 450

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;

layout(location = 0) out vec3 v_world_pos;
layout(location = 1) out vec3 v_normal;
layout(location = 2) out vec2 v_uv;
layout(location = 3) out vec4 v_particle_color;

layout(binding = 0) uniform PerFrameUBO {
    mat4 model;
    mat4 view_proj;
    vec4 light_dir;
    vec4 light_color;
    vec4 camera_pos;
    vec4 cascade_splits;
    mat4 light_vp[3];
    vec4 environment_params;
} ubo;

layout(push_constant) uniform GpuParticlePush {
    vec4 origin_elapsed;
    vec4 emission_lifetime;
    vec4 speed_size_spread;
    vec4 direction_drag;
    vec4 acceleration_turbulence;
    vec4 frequency_angular_duration;
    uvec4 colors_ordinals;
    uvec4 seed_burst_max;
} particle;

uint particle_hash(uint ordinal, uint stream) {
    uint value = ordinal * 0x9e3779b9u + stream * 0x85ebca6bu;
    value ^= particle.seed_burst_max.x ^ particle.seed_burst_max.y;
    value ^= value >> 16u;
    value *= 0x7feb352du;
    value ^= value >> 15u;
    value *= 0x846ca68bu;
    value ^= value >> 16u;
    return value;
}

float particle_random(uint ordinal, uint stream) {
    return float(particle_hash(ordinal, stream)) / 4294967295.0;
}

vec3 particle_tangent(vec3 axis) {
    return abs(axis.x) > abs(axis.z)
        ? normalize(vec3(-axis.y, axis.x, 0.0))
        : normalize(vec3(0.0, -axis.z, axis.y));
}

vec3 integrated_displacement(
    vec3 initial_velocity,
    vec3 acceleration,
    float drag,
    float age
) {
    if (drag <= 1e-5) {
        return initial_velocity * age + acceleration * (0.5 * age * age);
    }
    float decay = exp(-drag * age);
    float velocity_integral = (1.0 - decay) / drag;
    return initial_velocity * velocity_integral
        + acceleration * (age / drag - velocity_integral / drag);
}

void main() {
    uint ordinal = particle.colors_ordinals.z + uint(gl_InstanceIndex);
    float spawn_time = ordinal < particle.seed_burst_max.z
        ? 0.0
        : (particle.emission_lifetime.x > 0.0
            ? float(ordinal - particle.seed_burst_max.z + 1u)
                / particle.emission_lifetime.x
            : 3.402823466e+38);
    float age = particle.origin_elapsed.w - spawn_time;
    float lifetime = mix(
        particle.emission_lifetime.y,
        particle.emission_lifetime.z,
        particle_random(ordinal, 0u)
    );
    bool live = age >= 0.0 && age < lifetime;

    vec3 axis = normalize(particle.direction_drag.xyz);
    if (all(lessThan(abs(axis), vec3(1e-6)))) {
        axis = vec3(0.0, 1.0, 0.0);
    }
    float cos_limit = cos(particle.speed_size_spread.w);
    float cos_theta =
        1.0 - particle_random(ordinal, 1u) * (1.0 - cos_limit);
    float sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    float phi = particle_random(ordinal, 2u) * 6.28318530718;
    vec3 tangent = particle_tangent(axis);
    vec3 bitangent = cross(axis, tangent);
    vec3 direction = normalize(
        axis * cos_theta
        + tangent * (sin_theta * cos(phi))
        + bitangent * (sin_theta * sin(phi))
    );
    float speed = mix(
        particle.emission_lifetime.w,
        particle.speed_size_spread.x,
        particle_random(ordinal, 3u)
    );
    float safe_age = max(age, 0.0);
    vec3 world_position = particle.origin_elapsed.xyz
        + integrated_displacement(
            direction * speed,
            particle.acceleration_turbulence.xyz,
            particle.direction_drag.w,
            safe_age
        );
    if (particle.acceleration_turbulence.w > 0.0) {
        vec3 phase = vec3(
            particle_random(ordinal, 6u),
            particle_random(ordinal, 7u),
            particle_random(ordinal, 8u)
        ) * 6.28318530718
            + vec3(safe_age * particle.frequency_angular_duration.x);
        world_position += vec3(
            sin(phase.y + phase.z * 1.37),
            sin(phase.z + phase.x * 1.79),
            sin(phase.x + phase.y * 2.11)
        ) * (0.5 * particle.acceleration_turbulence.w * age * age);
    }

    float normalized_age = clamp(age / max(lifetime, 1e-6), 0.0, 1.0);
    float angular_velocity = mix(
        particle.frequency_angular_duration.y,
        particle.frequency_angular_duration.z,
        particle_random(ordinal, 4u)
    );
    float rotation =
        particle_random(ordinal, 5u) * 6.28318530718
        + angular_velocity * age;
    float particle_size = live
        ? mix(
            particle.speed_size_spread.y,
            particle.speed_size_spread.z,
            normalized_age
        )
        : 0.0;
    vec2 rotated = vec2(
        a_position.x * cos(rotation) - a_position.y * sin(rotation),
        a_position.x * sin(rotation) + a_position.y * cos(rotation)
    ) * particle_size;
    mat4 inverse_view_projection = inverse(ubo.view_proj);
    vec3 camera_right = normalize(inverse_view_projection[0].xyz);
    vec3 camera_up = normalize(inverse_view_projection[1].xyz);
    world_position += camera_right * rotated.x + camera_up * rotated.y;

    v_world_pos = world_position;
    v_normal = normalize(cross(camera_right, camera_up));
    v_uv = a_uv;
    v_particle_color = mix(
        unpackUnorm4x8(particle.colors_ordinals.x),
        unpackUnorm4x8(particle.colors_ordinals.y),
        normalized_age
    );
    gl_Position = live
        ? ubo.view_proj * vec4(world_position, 1.0)
        : vec4(2.0, 2.0, 2.0, 1.0);
}
