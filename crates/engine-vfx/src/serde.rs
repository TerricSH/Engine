use std::collections::BTreeMap;

use engine_serialize::{AssetId, Value};

use crate::{Decal, ParticleEmitter, ParticleSimulationMode};

fn bool_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<bool> {
    match fields.get(key) {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn f32_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<f32> {
    match fields.get(key) {
        Some(Value::Float32(value)) => Some(*value),
        Some(Value::Float64(value)) => Some(*value as f32),
        Some(Value::Int(value)) => Some(*value as f32),
        Some(Value::UInt(value)) => Some(*value as f32),
        _ => None,
    }
}

fn u32_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<u32> {
    match fields.get(key) {
        Some(Value::UInt(value)) => u32::try_from(*value).ok(),
        Some(Value::Int(value)) => u32::try_from(*value).ok(),
        _ => None,
    }
}

fn string_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    match fields.get(key) {
        Some(Value::Str(value)) | Some(Value::Enum(value)) => Some(value.clone()),
        Some(Value::Asset(value)) => Some(value.id.clone()),
        _ => None,
    }
}

fn vec3_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<glam::Vec3> {
    match fields.get(key) {
        Some(Value::Vec3(value)) => Some(glam::Vec3::from_array(*value)),
        _ => None,
    }
}

fn color_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<[u8; 4]> {
    match fields.get(key) {
        Some(Value::Color(value)) if value.iter().all(|channel| channel.is_finite()) => {
            Some(value.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8))
        }
        _ => None,
    }
}

pub fn serialize_particle_emitter(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let emitter = component
        .downcast_ref::<ParticleEmitter>()
        .expect("ParticleEmitter expected");
    BTreeMap::from([
        ("enabled".into(), Value::Bool(emitter.enabled)),
        (
            "simulation_mode".into(),
            Value::Enum(
                match emitter.simulation_mode {
                    ParticleSimulationMode::Cpu => "cpu",
                    ParticleSimulationMode::Gpu => "gpu",
                }
                .to_string(),
            ),
        ),
        ("looping".into(), Value::Bool(emitter.looping)),
        ("duration".into(), Value::Float32(emitter.duration)),
        (
            "emission_rate".into(),
            Value::Float32(emitter.emission_rate),
        ),
        (
            "burst_count".into(),
            Value::UInt(u64::from(emitter.burst_count)),
        ),
        (
            "max_particles".into(),
            Value::UInt(u64::from(emitter.max_particles)),
        ),
        ("lifetime_min".into(), Value::Float32(emitter.lifetime_min)),
        ("lifetime_max".into(), Value::Float32(emitter.lifetime_max)),
        ("speed_min".into(), Value::Float32(emitter.speed_min)),
        ("speed_max".into(), Value::Float32(emitter.speed_max)),
        ("start_size".into(), Value::Float32(emitter.start_size)),
        ("end_size".into(), Value::Float32(emitter.end_size)),
        (
            "start_color".into(),
            Value::Color(
                emitter
                    .start_color
                    .map(|channel| f32::from(channel) / 255.0),
            ),
        ),
        (
            "end_color".into(),
            Value::Color(emitter.end_color.map(|channel| f32::from(channel) / 255.0)),
        ),
        (
            "direction".into(),
            Value::Vec3(emitter.direction.to_array()),
        ),
        (
            "spread_angle_radians".into(),
            Value::Float32(emitter.spread_angle_radians),
        ),
        (
            "acceleration".into(),
            Value::Vec3(emitter.acceleration.to_array()),
        ),
        ("drag".into(), Value::Float32(emitter.drag)),
        (
            "turbulence_strength".into(),
            Value::Float32(emitter.turbulence_strength),
        ),
        (
            "turbulence_frequency".into(),
            Value::Float32(emitter.turbulence_frequency),
        ),
        (
            "angular_velocity_min".into(),
            Value::Float32(emitter.angular_velocity_min),
        ),
        (
            "angular_velocity_max".into(),
            Value::Float32(emitter.angular_velocity_max),
        ),
        (
            "mesh_asset".into(),
            Value::Asset(AssetId::new(&emitter.mesh_asset)),
        ),
        (
            "material_asset".into(),
            Value::Asset(AssetId::new(&emitter.material_asset)),
        ),
        (
            "render_layer".into(),
            Value::Enum(emitter.render_layer.clone()),
        ),
    ])
}

pub fn deserialize_particle_emitter(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let mut emitter = ParticleEmitter::default();
    if let Some(value) = bool_field(fields, "enabled") {
        emitter.enabled = value;
    }
    if let Some(value) = string_field(fields, "simulation_mode") {
        emitter.simulation_mode = match value.as_str() {
            "gpu" => ParticleSimulationMode::Gpu,
            _ => ParticleSimulationMode::Cpu,
        };
    }
    if let Some(value) = bool_field(fields, "looping") {
        emitter.looping = value;
    }
    for (key, target) in [
        ("duration", &mut emitter.duration),
        ("emission_rate", &mut emitter.emission_rate),
        ("lifetime_min", &mut emitter.lifetime_min),
        ("lifetime_max", &mut emitter.lifetime_max),
        ("speed_min", &mut emitter.speed_min),
        ("speed_max", &mut emitter.speed_max),
        ("start_size", &mut emitter.start_size),
        ("end_size", &mut emitter.end_size),
        ("drag", &mut emitter.drag),
        ("turbulence_strength", &mut emitter.turbulence_strength),
        ("turbulence_frequency", &mut emitter.turbulence_frequency),
        ("spread_angle_radians", &mut emitter.spread_angle_radians),
        ("angular_velocity_min", &mut emitter.angular_velocity_min),
        ("angular_velocity_max", &mut emitter.angular_velocity_max),
    ] {
        if let Some(value) = f32_field(fields, key) {
            *target = value;
        }
    }
    if let Some(value) = u32_field(fields, "burst_count") {
        emitter.burst_count = value;
    }
    if let Some(value) = u32_field(fields, "max_particles") {
        emitter.max_particles = value;
    }
    if let Some(value) = vec3_field(fields, "direction") {
        emitter.direction = value;
    }
    if let Some(value) = vec3_field(fields, "acceleration") {
        emitter.acceleration = value;
    }
    if let Some(value) = color_field(fields, "start_color") {
        emitter.start_color = value;
    }
    if let Some(value) = color_field(fields, "end_color") {
        emitter.end_color = value;
    }
    if let Some(value) = string_field(fields, "mesh_asset") {
        emitter.mesh_asset = value;
    }
    if let Some(value) = string_field(fields, "material_asset") {
        emitter.material_asset = value;
    }
    if let Some(value) = string_field(fields, "render_layer") {
        emitter.render_layer = value;
    }
    Box::new(emitter)
}

pub fn validate_particle_emitter_fields(fields: &BTreeMap<String, Value>) -> Result<(), String> {
    deserialize_particle_emitter(fields)
        .downcast::<ParticleEmitter>()
        .expect("ParticleEmitter decoder type")
        .validate()
}

pub fn serialize_decal(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let decal = component.downcast_ref::<Decal>().expect("Decal expected");
    BTreeMap::from([
        ("enabled".into(), Value::Bool(decal.enabled)),
        (
            "size".into(),
            Value::List(
                decal
                    .size
                    .into_iter()
                    .map(Value::Float32)
                    .collect::<Vec<_>>(),
            ),
        ),
        ("normal_bias".into(), Value::Float32(decal.normal_bias)),
        ("lifetime".into(), Value::Float32(decal.lifetime)),
        (
            "mesh_asset".into(),
            Value::Asset(AssetId::new(&decal.mesh_asset)),
        ),
        (
            "material_asset".into(),
            Value::Asset(AssetId::new(&decal.material_asset)),
        ),
        (
            "render_layer".into(),
            Value::Enum(decal.render_layer.clone()),
        ),
    ])
}

pub fn deserialize_decal(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let mut decal = Decal::default();
    if let Some(value) = bool_field(fields, "enabled") {
        decal.enabled = value;
    }
    if let Some(Value::List(values)) = fields.get("size") {
        if values.len() == 2 {
            decal.size = [
                match &values[0] {
                    Value::Float32(value) => *value,
                    Value::Float64(value) => *value as f32,
                    _ => decal.size[0],
                },
                match &values[1] {
                    Value::Float32(value) => *value,
                    Value::Float64(value) => *value as f32,
                    _ => decal.size[1],
                },
            ];
        }
    }
    if let Some(value) = f32_field(fields, "normal_bias") {
        decal.normal_bias = value;
    }
    if let Some(value) = f32_field(fields, "lifetime") {
        decal.lifetime = value;
    }
    if let Some(value) = string_field(fields, "mesh_asset") {
        decal.mesh_asset = value;
    }
    if let Some(value) = string_field(fields, "material_asset") {
        decal.material_asset = value;
    }
    if let Some(value) = string_field(fields, "render_layer") {
        decal.render_layer = value;
    }
    Box::new(decal)
}

pub fn validate_decal_fields(fields: &BTreeMap<String, Value>) -> Result<(), String> {
    deserialize_decal(fields)
        .downcast::<Decal>()
        .expect("Decal decoder type")
        .validate()
}
