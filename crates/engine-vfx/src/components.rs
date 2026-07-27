use engine_scene::Component;
use glam::Vec3;
use serde::{Deserialize, Serialize};

pub const BUILTIN_VFX_QUAD_MESH_ID: &str = "mesh-vfx-quad";
pub const BUILTIN_VFX_MATERIAL_ID: &str = "mat-vfx-default";

/// One live CPU particle. Runtime particles are intentionally not serialized:
/// reloading a scene or save restarts the authored emitter deterministically.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    pub age: f32,
    pub lifetime: f32,
    pub rotation: f32,
    pub angular_velocity: f32,
}

/// Selects where particle positions are evaluated. GPU mode is analytic and
/// automatically expands through the same deterministic CPU model on a
/// backend that does not support GPU particle simulation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticleSimulationMode {
    #[default]
    Cpu,
    Gpu,
}

/// Scene-authored particle emitter rendered as camera-facing quads.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleEmitter {
    pub enabled: bool,
    pub simulation_mode: ParticleSimulationMode,
    pub looping: bool,
    pub duration: f32,
    pub emission_rate: f32,
    pub burst_count: u32,
    pub max_particles: u32,
    pub lifetime_min: f32,
    pub lifetime_max: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    pub start_size: f32,
    pub end_size: f32,
    pub start_color: [u8; 4],
    pub end_color: [u8; 4],
    pub direction: Vec3,
    pub spread_angle_radians: f32,
    pub acceleration: Vec3,
    /// Exponential velocity damping per second.
    pub drag: f32,
    /// Deterministic world-space turbulence acceleration.
    pub turbulence_strength: f32,
    pub turbulence_frequency: f32,
    pub angular_velocity_min: f32,
    pub angular_velocity_max: f32,
    pub mesh_asset: String,
    pub material_asset: String,
    pub render_layer: String,
    pub(crate) particles: Vec<Particle>,
    pub(crate) spawn_accumulator: f32,
    pub(crate) elapsed: f32,
    pub(crate) burst_emitted: bool,
    pub(crate) random_state: u64,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            enabled: true,
            simulation_mode: ParticleSimulationMode::Cpu,
            looping: true,
            duration: 5.0,
            emission_rate: 12.0,
            burst_count: 0,
            max_particles: 256,
            lifetime_min: 0.75,
            lifetime_max: 1.5,
            speed_min: 0.5,
            speed_max: 2.0,
            start_size: 0.25,
            end_size: 0.0,
            start_color: [255; 4],
            end_color: [255, 255, 255, 0],
            direction: Vec3::Y,
            spread_angle_radians: 0.35,
            acceleration: Vec3::new(0.0, -1.5, 0.0),
            drag: 0.0,
            turbulence_strength: 0.0,
            turbulence_frequency: 1.0,
            angular_velocity_min: -1.0,
            angular_velocity_max: 1.0,
            mesh_asset: BUILTIN_VFX_QUAD_MESH_ID.to_string(),
            material_asset: BUILTIN_VFX_MATERIAL_ID.to_string(),
            render_layer: "Transparent".to_string(),
            particles: Vec::new(),
            spawn_accumulator: 0.0,
            elapsed: 0.0,
            burst_emitted: false,
            random_state: 0x4d59_5df4_d0f3_3173,
        }
    }
}

impl Component for ParticleEmitter {
    const TYPE_ID: &'static str = "engine.vfx.particle_emitter";
}

impl ParticleEmitter {
    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    pub fn restart(&mut self) {
        self.particles.clear();
        self.spawn_accumulator = 0.0;
        self.elapsed = 0.0;
        self.burst_emitted = false;
        self.random_state = 0x4d59_5df4_d0f3_3173;
    }

    pub(crate) fn advance_gpu(&mut self, dt: f32) {
        if self.enabled {
            self.elapsed += dt;
        }
    }

    pub(crate) fn elapsed(&self) -> f32 {
        self.elapsed
    }

    pub(crate) fn simulation_seed(&self) -> u64 {
        self.random_state
    }

    pub fn validate(&self) -> Result<(), String> {
        let finite = [
            self.duration,
            self.emission_rate,
            self.lifetime_min,
            self.lifetime_max,
            self.speed_min,
            self.speed_max,
            self.start_size,
            self.end_size,
            self.spread_angle_radians,
            self.drag,
            self.turbulence_strength,
            self.turbulence_frequency,
            self.angular_velocity_min,
            self.angular_velocity_max,
        ]
        .into_iter()
        .all(f32::is_finite)
            && self.direction.is_finite()
            && self.acceleration.is_finite();
        if !finite {
            return Err("particle parameters must be finite".to_string());
        }
        if self.duration < 0.0
            || self.emission_rate < 0.0
            || self.max_particles == 0
            || self.lifetime_min <= 0.0
            || self.lifetime_max < self.lifetime_min
            || self.speed_max < self.speed_min
            || self.start_size < 0.0
            || self.end_size < 0.0
            || self.drag < 0.0
            || self.turbulence_strength < 0.0
            || self.turbulence_frequency <= 0.0
            || !(0.0..=std::f32::consts::PI).contains(&self.spread_angle_radians)
            || self.angular_velocity_max < self.angular_velocity_min
        {
            return Err("particle ranges are invalid".to_string());
        }
        if self.mesh_asset.trim().is_empty()
            || self.material_asset.trim().is_empty()
            || engine_scene::render_layer_bit(&self.render_layer).is_none()
        {
            return Err("particle mesh, material, and render layer must be valid".to_string());
        }
        Ok(())
    }

    pub(crate) fn particles_mut(&mut self) -> &mut Vec<Particle> {
        &mut self.particles
    }

    pub(crate) fn take_spawn_budget(&mut self, dt: f32) -> usize {
        if !self.enabled {
            return 0;
        }
        let was_in_duration = self.looping || self.duration == 0.0 || self.elapsed < self.duration;
        self.elapsed += dt;
        let mut count = 0usize;
        if !self.burst_emitted && was_in_duration {
            count = self.burst_count as usize;
            self.burst_emitted = true;
        }
        if was_in_duration && self.emission_rate > 0.0 {
            self.spawn_accumulator += self.emission_rate * dt;
            let continuous = self.spawn_accumulator.floor() as usize;
            self.spawn_accumulator -= continuous as f32;
            count = count.saturating_add(continuous);
        }
        let available = (self.max_particles as usize).saturating_sub(self.particles.len());
        count.min(available)
    }

    pub(crate) fn random_unit(&mut self) -> f32 {
        let mut x = self.random_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.random_state = x;
        ((x >> 40) as u32) as f32 / ((1_u32 << 24) - 1) as f32
    }

    pub(crate) fn random_range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.random_unit()
    }
}

/// Mesh-based surface decal. The local XY plane is placed on the surface and
/// local +Z is the projection normal. A finite lifetime of zero means
/// permanent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decal {
    pub enabled: bool,
    pub size: [f32; 2],
    pub normal_bias: f32,
    pub lifetime: f32,
    pub mesh_asset: String,
    pub material_asset: String,
    pub render_layer: String,
    #[serde(skip)]
    pub(crate) elapsed: f32,
}

impl Default for Decal {
    fn default() -> Self {
        Self {
            enabled: true,
            size: [1.0, 1.0],
            normal_bias: 0.002,
            lifetime: 0.0,
            mesh_asset: BUILTIN_VFX_QUAD_MESH_ID.to_string(),
            material_asset: BUILTIN_VFX_MATERIAL_ID.to_string(),
            render_layer: "Transparent".to_string(),
            elapsed: 0.0,
        }
    }
}

impl Component for Decal {
    const TYPE_ID: &'static str = "engine.vfx.decal";
}

impl Decal {
    pub fn expired(&self) -> bool {
        self.lifetime > 0.0 && self.elapsed >= self.lifetime
    }

    pub fn restart(&mut self) {
        self.elapsed = 0.0;
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.size.into_iter().all(f32::is_finite)
            || !self.normal_bias.is_finite()
            || !self.lifetime.is_finite()
            || self.size[0] <= 0.0
            || self.size[1] <= 0.0
            || self.lifetime < 0.0
        {
            return Err("decal size, bias, and lifetime are invalid".to_string());
        }
        if self.mesh_asset.trim().is_empty()
            || self.material_asset.trim().is_empty()
            || engine_scene::render_layer_bit(&self.render_layer).is_none()
        {
            return Err("decal mesh, material, and render layer must be valid".to_string());
        }
        Ok(())
    }

    pub(crate) fn tick(&mut self, dt: f32) {
        if self.enabled && !self.expired() {
            self.elapsed += dt;
        }
    }
}
