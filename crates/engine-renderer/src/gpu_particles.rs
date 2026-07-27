//! Deterministic analytic GPU-particle model and CPU fallback.

use glam::Vec3;

use crate::{GpuParticleSimulation, ParticleInstance};

pub const GPU_PARTICLE_PARAMETER_SIZE: usize = 128;

impl GpuParticleSimulation {
    fn emission_elapsed(self) -> f32 {
        if self.emission_duration > 0.0 {
            self.elapsed.min(self.emission_duration)
        } else {
            self.elapsed
        }
    }

    pub fn spawned_count(self) -> u64 {
        let emission_elapsed = self.emission_elapsed();
        let continuous = if self.emission_rate > 0.0 && emission_elapsed > 0.0 {
            (emission_elapsed * self.emission_rate).floor() as u64
        } else {
            0
        };
        u64::from(self.burst_count).saturating_add(continuous)
    }

    pub fn draw_range(self) -> (u32, u32) {
        let total = self.spawned_count();
        let capacity_first = total.saturating_sub(u64::from(self.max_particles));
        let lifetime_cutoff = self.elapsed - self.lifetime_max;
        let expired_first = if lifetime_cutoff >= 0.0 {
            let expired_continuous = if self.emission_rate > 0.0 {
                (lifetime_cutoff * self.emission_rate).floor() as u64
            } else {
                0
            };
            u64::from(self.burst_count)
                .saturating_add(expired_continuous)
                .min(total)
        } else {
            0
        };
        let first = capacity_first.max(expired_first).min(total);
        let count = total
            .saturating_sub(first)
            .min(u64::from(self.max_particles)) as u32;
        (first as u32, count)
    }

    pub fn parameter_bytes(self) -> [u8; GPU_PARTICLE_PARAMETER_SIZE] {
        let (first_ordinal, draw_count) = self.draw_range();
        let mut bytes = [0_u8; GPU_PARTICLE_PARAMETER_SIZE];
        write_f32x4(
            &mut bytes,
            0,
            [self.origin[0], self.origin[1], self.origin[2], self.elapsed],
        );
        write_f32x4(
            &mut bytes,
            16,
            [
                self.emission_rate,
                self.lifetime_min,
                self.lifetime_max,
                self.speed_min,
            ],
        );
        write_f32x4(
            &mut bytes,
            32,
            [
                self.speed_max,
                self.start_size,
                self.end_size,
                self.spread_angle_radians,
            ],
        );
        write_f32x4(
            &mut bytes,
            48,
            [
                self.direction[0],
                self.direction[1],
                self.direction[2],
                self.drag,
            ],
        );
        write_f32x4(
            &mut bytes,
            64,
            [
                self.acceleration[0],
                self.acceleration[1],
                self.acceleration[2],
                self.turbulence_strength,
            ],
        );
        write_f32x4(
            &mut bytes,
            80,
            [
                self.turbulence_frequency,
                self.angular_velocity_min,
                self.angular_velocity_max,
                self.emission_duration,
            ],
        );
        write_u32x4(
            &mut bytes,
            96,
            [
                u32::from_le_bytes(self.start_color),
                u32::from_le_bytes(self.end_color),
                first_ordinal,
                draw_count,
            ],
        );
        write_u32x4(
            &mut bytes,
            112,
            [
                self.seed as u32,
                (self.seed >> 32) as u32,
                self.burst_count,
                self.max_particles,
            ],
        );
        bytes
    }
}

pub fn expand_gpu_particle_simulation(simulation: GpuParticleSimulation) -> Vec<ParticleInstance> {
    let (first, count) = simulation.draw_range();
    (0..count)
        .filter_map(|index| particle_instance(simulation, first.wrapping_add(index)))
        .collect()
}

fn particle_instance(simulation: GpuParticleSimulation, ordinal: u32) -> Option<ParticleInstance> {
    let age = simulation.elapsed - spawn_time(simulation, ordinal);
    if age < 0.0 {
        return None;
    }
    let lifetime = mix(
        simulation.lifetime_min,
        simulation.lifetime_max,
        random_unit(simulation.seed, ordinal, 0),
    );
    if age >= lifetime {
        return None;
    }
    let axis = Vec3::from_array(simulation.direction).normalize_or_zero();
    let axis = if axis == Vec3::ZERO { Vec3::Y } else { axis };
    let cos_limit = simulation.spread_angle_radians.cos();
    let cos_theta = 1.0 - random_unit(simulation.seed, ordinal, 1) * (1.0 - cos_limit);
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = random_unit(simulation.seed, ordinal, 2) * std::f32::consts::TAU;
    let tangent = stable_tangent(axis);
    let bitangent = axis.cross(tangent);
    let direction = (axis * cos_theta
        + tangent * (sin_theta * phi.cos())
        + bitangent * (sin_theta * phi.sin()))
    .normalize_or_zero();
    let speed = mix(
        simulation.speed_min,
        simulation.speed_max,
        random_unit(simulation.seed, ordinal, 3),
    );
    let position = Vec3::from_array(simulation.origin)
        + integrated_displacement(
            direction * speed,
            Vec3::from_array(simulation.acceleration),
            simulation.drag,
            age,
        )
        + analytic_turbulence(simulation, ordinal, age);
    let normalized_age = (age / lifetime).clamp(0.0, 1.0);
    let angular_velocity = mix(
        simulation.angular_velocity_min,
        simulation.angular_velocity_max,
        random_unit(simulation.seed, ordinal, 4),
    );
    Some(ParticleInstance {
        position: position.to_array(),
        size: mix(simulation.start_size, simulation.end_size, normalized_age),
        rotation_radians: random_unit(simulation.seed, ordinal, 5) * std::f32::consts::TAU
            + angular_velocity * age,
        normalized_age,
        color: std::array::from_fn(|channel| {
            mix(
                f32::from(simulation.start_color[channel]),
                f32::from(simulation.end_color[channel]),
                normalized_age,
            )
            .round()
            .clamp(0.0, 255.0) as u8
        }),
    })
}

fn spawn_time(simulation: GpuParticleSimulation, ordinal: u32) -> f32 {
    if ordinal < simulation.burst_count {
        0.0
    } else if simulation.emission_rate > 0.0 {
        (ordinal - simulation.burst_count + 1) as f32 / simulation.emission_rate
    } else {
        f32::INFINITY
    }
}

fn stable_tangent(axis: Vec3) -> Vec3 {
    if axis.x.abs() > axis.z.abs() {
        Vec3::new(-axis.y, axis.x, 0.0).normalize()
    } else {
        Vec3::new(0.0, -axis.z, axis.y).normalize()
    }
}

fn integrated_displacement(initial: Vec3, acceleration: Vec3, drag: f32, age: f32) -> Vec3 {
    if drag <= 1.0e-5 {
        initial * age + acceleration * (0.5 * age * age)
    } else {
        let decay = (-drag * age).exp();
        let velocity_integral = (1.0 - decay) / drag;
        initial * velocity_integral + acceleration * (age / drag - velocity_integral / drag)
    }
}

fn analytic_turbulence(simulation: GpuParticleSimulation, ordinal: u32, age: f32) -> Vec3 {
    if simulation.turbulence_strength <= 0.0 {
        return Vec3::ZERO;
    }
    let phase = Vec3::new(
        random_unit(simulation.seed, ordinal, 6),
        random_unit(simulation.seed, ordinal, 7),
        random_unit(simulation.seed, ordinal, 8),
    ) * std::f32::consts::TAU
        + Vec3::splat(age * simulation.turbulence_frequency);
    Vec3::new(
        (phase.y + phase.z * 1.37).sin(),
        (phase.z + phase.x * 1.79).sin(),
        (phase.x + phase.y * 2.11).sin(),
    ) * (0.5 * simulation.turbulence_strength * age * age)
}

fn random_unit(seed: u64, ordinal: u32, stream: u32) -> f32 {
    let mut value = ordinal
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(stream.wrapping_mul(0x85eb_ca6b))
        ^ seed as u32
        ^ (seed >> 32) as u32;
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value as f32 / u32::MAX as f32
}

fn mix(left: f32, right: f32, amount: f32) -> f32 {
    left + (right - left) * amount
}

fn write_f32x4(bytes: &mut [u8], offset: usize, values: [f32; 4]) {
    write_u32x4(bytes, offset, values.map(f32::to_bits));
}

fn write_u32x4(bytes: &mut [u8], offset: usize, values: [u32; 4]) {
    for (index, value) in values.into_iter().enumerate() {
        bytes[offset + index * 4..offset + index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simulation() -> GpuParticleSimulation {
        GpuParticleSimulation {
            origin: [1.0, 2.0, 3.0],
            elapsed: 1.0,
            emission_duration: 0.0,
            emission_rate: 4.0,
            burst_count: 2,
            max_particles: 32,
            lifetime_min: 2.0,
            lifetime_max: 2.0,
            speed_min: 1.0,
            speed_max: 1.0,
            start_size: 1.0,
            end_size: 0.0,
            start_color: [255, 0, 0, 255],
            end_color: [0, 0, 255, 0],
            direction: [0.0, 1.0, 0.0],
            spread_angle_radians: 0.2,
            acceleration: [0.0, -1.0, 0.0],
            drag: 0.1,
            turbulence_strength: 0.0,
            turbulence_frequency: 1.0,
            angular_velocity_min: 0.0,
            angular_velocity_max: 0.0,
            seed: 7,
        }
    }

    #[test]
    fn analytic_range_is_bounded_and_parameter_abi_is_complete() {
        let simulation = simulation();
        assert_eq!(simulation.draw_range(), (0, 6));
        let bytes = simulation.parameter_bytes();
        assert_eq!(bytes.len(), GPU_PARTICLE_PARAMETER_SIZE);
        assert_eq!(u32::from_ne_bytes(bytes[104..108].try_into().unwrap()), 0);
        assert_eq!(u32::from_ne_bytes(bytes[108..112].try_into().unwrap()), 6);
    }

    #[test]
    fn cpu_fallback_is_deterministic_and_removes_expired_slots() {
        let first = expand_gpu_particle_simulation(simulation());
        let second = expand_gpu_particle_simulation(simulation());
        assert_eq!(first, second);
        assert_eq!(first.len(), 6);
        assert!(first.iter().all(|particle| particle.size > 0.0));
    }
}
