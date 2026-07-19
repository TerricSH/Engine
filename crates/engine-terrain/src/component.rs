use std::collections::BTreeMap;

use engine_scene::{
    Component, ComponentExtension, ComponentMeta, ComponentRegistry, ComponentStorageDyn,
    ScriptAccess, SparseSet,
};
use engine_serialize::Value;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Serializable, game-authored heightfield parameter block.
///
/// The engine does not prescribe values or derive a planet/biome recipe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainVolume {
    pub enabled: bool,
    pub seed: u64,
    pub chunk_size: f32,
    /// Finest grid dimension. Must be `2^n + 1` in `3..=513`.
    pub base_resolution: u32,
    pub height_scale: f32,
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub domain_warp_amplitude: f32,
    pub domain_warp_frequency: f32,
    pub skirt_depth: f32,
    pub collision_enabled: bool,
    pub material_asset: String,
    /// Increasing world-space distance cutoffs, one per LOD.
    pub lod_distances: Vec<f32>,
    /// World-space dead band applied when splitting/merging CDLOD nodes.
    pub lod_hysteresis: f32,
}

impl Default for TerrainVolume {
    fn default() -> Self {
        Self {
            enabled: true,
            seed: 0,
            chunk_size: 64.0,
            base_resolution: 65,
            height_scale: 24.0,
            frequency: 0.008,
            octaves: 5,
            lacunarity: 2.0,
            gain: 0.5,
            domain_warp_amplitude: 0.0,
            domain_warp_frequency: 0.01,
            skirt_depth: 4.0,
            collision_enabled: true,
            material_asset: String::new(),
            lod_distances: vec![160.0, 320.0, 640.0],
            lod_hysteresis: 16.0,
        }
    }
}

impl Component for TerrainVolume {
    const TYPE_ID: &'static str = "engine.terrain_volume";
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TerrainConfigError {
    #[error("{0}")]
    Invalid(&'static str),
}

impl TerrainVolume {
    pub fn validate(&self) -> Result<(), TerrainConfigError> {
        if !self.chunk_size.is_finite() || self.chunk_size <= 0.0 {
            return Err(TerrainConfigError::Invalid(
                "chunk_size must be finite and positive",
            ));
        }
        let cells = self.base_resolution.saturating_sub(1);
        if !(3..=513).contains(&self.base_resolution) || !cells.is_power_of_two() {
            return Err(TerrainConfigError::Invalid(
                "base_resolution must be 2^n + 1 in 3..=513",
            ));
        }
        if !self.height_scale.is_finite() || self.height_scale < 0.0 {
            return Err(TerrainConfigError::Invalid(
                "height_scale must be finite and non-negative",
            ));
        }
        if !self.frequency.is_finite() || self.frequency <= 0.0 || self.frequency > 65536.0 {
            return Err(TerrainConfigError::Invalid(
                "frequency must be finite and in (0, 65536]",
            ));
        }
        if !(1..=32).contains(&self.octaves) {
            return Err(TerrainConfigError::Invalid("octaves must be in 1..=32"));
        }
        if !self.lacunarity.is_finite() || self.lacunarity <= 0.0 || self.lacunarity > 16.0 {
            return Err(TerrainConfigError::Invalid("lacunarity must be in (0, 16]"));
        }
        if !self.gain.is_finite() || !(0.0..=1.0).contains(&self.gain) {
            return Err(TerrainConfigError::Invalid("gain must be in [0, 1]"));
        }
        if !self.domain_warp_amplitude.is_finite()
            || self.domain_warp_amplitude < 0.0
            || self.domain_warp_amplitude > 65536.0
        {
            return Err(TerrainConfigError::Invalid(
                "domain_warp_amplitude must be finite and in [0, 65536]",
            ));
        }
        if !self.domain_warp_frequency.is_finite()
            || self.domain_warp_frequency <= 0.0
            || self.domain_warp_frequency > 65536.0
        {
            return Err(TerrainConfigError::Invalid(
                "domain_warp_frequency must be finite and in (0, 65536]",
            ));
        }
        if !self.skirt_depth.is_finite() || self.skirt_depth < 0.0 {
            return Err(TerrainConfigError::Invalid(
                "skirt_depth must be finite and non-negative",
            ));
        }
        if !(self.height_scale + self.skirt_depth).is_finite() {
            return Err(TerrainConfigError::Invalid(
                "height_scale plus skirt_depth must remain finite",
            ));
        }
        if self.lod_distances.is_empty()
            || self.lod_distances.len() > 16
            || self
                .lod_distances
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
            || self.lod_distances.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(TerrainConfigError::Invalid(
                "lod_distances must contain 1..=16 finite, positive, increasing values",
            ));
        }
        if !self.lod_hysteresis.is_finite()
            || self.lod_hysteresis < 0.0
            || self.lod_hysteresis >= self.lod_distances[0]
        {
            return Err(TerrainConfigError::Invalid(
                "lod_hysteresis must be finite, non-negative, and smaller than the first LOD distance",
            ));
        }
        let max_span = f64::from(self.chunk_size)
            * 2.0f64.powi(self.lod_distances.len().saturating_sub(1) as i32);
        if !max_span.is_finite() || max_span > f64::from(f32::MAX / 2.0) {
            return Err(TerrainConfigError::Invalid(
                "chunk_size and LOD count produce an unrepresentable chunk span",
            ));
        }
        Ok(())
    }

    pub fn revision(&self) -> u64 {
        // Stable FNV-1a over a deliberately fixed field order. This is used
        // only for stale-work rejection, not as the procedural seed.
        let mut hash = 0xcbf29ce484222325u64;
        let mut mix = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        };
        mix(&self.seed.to_le_bytes());
        for value in [
            self.chunk_size,
            self.height_scale,
            self.frequency,
            self.lacunarity,
            self.gain,
            self.domain_warp_amplitude,
            self.domain_warp_frequency,
            self.skirt_depth,
            self.lod_hysteresis,
        ] {
            mix(&value.to_bits().to_le_bytes());
        }
        mix(&self.base_resolution.to_le_bytes());
        mix(&self.octaves.to_le_bytes());
        mix(&[self.collision_enabled as u8]);
        for distance in &self.lod_distances {
            mix(&distance.to_bits().to_le_bytes());
        }
        mix(self.material_asset.as_bytes());
        hash
    }
}

pub fn register_terrain_extensions(registry: &mut ComponentRegistry) {
    let registered = registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: TerrainVolume::TYPE_ID,
                display_name: "Terrain Volume",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: ScriptAccess::ReadWrite,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<TerrainVolume>::new())
            },
            serialize: Some(serialize_terrain),
            deserialize: Some(deserialize_terrain),
        })
        .is_ok();
    if registered {
        let _ = registry.register_fields_validator(TerrainVolume::TYPE_ID, validate_terrain_fields);
        let _ = registry.register_singleton(TerrainVolume::TYPE_ID);
    }
}

fn validate_terrain_fields(fields: &BTreeMap<String, Value>) -> Result<(), String> {
    let terrain = deserialize_terrain(fields)
        .downcast::<TerrainVolume>()
        .map_err(|_| "terrain deserializer returned an incompatible value".to_string())?;
    let normalized = serialize_terrain(terrain.as_ref());
    let rejected = fields
        .iter()
        .filter_map(|(name, value)| {
            let accepted = match (name.as_str(), value, normalized.get(name)) {
                ("material_asset", Value::Str(value), Some(Value::Asset(normalized))) => {
                    value == &normalized.id
                }
                (_, value, Some(normalized)) => value == normalized,
                _ => false,
            };
            (!accepted).then_some(name.clone())
        })
        .collect::<Vec<_>>();
    if !rejected.is_empty() {
        return Err(format!(
            "unknown or incorrectly typed fields: {}",
            rejected.join(", ")
        ));
    }
    terrain.validate().map_err(|error| error.to_string())
}

fn serialize_terrain(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let terrain = component
        .downcast_ref::<TerrainVolume>()
        .expect("TerrainVolume expected");
    let mut fields = BTreeMap::new();
    fields.insert("enabled".into(), Value::Bool(terrain.enabled));
    fields.insert("seed".into(), Value::UInt(terrain.seed));
    fields.insert("chunk_size".into(), Value::Float32(terrain.chunk_size));
    fields.insert(
        "base_resolution".into(),
        Value::UInt(u64::from(terrain.base_resolution)),
    );
    fields.insert("height_scale".into(), Value::Float32(terrain.height_scale));
    fields.insert("frequency".into(), Value::Float32(terrain.frequency));
    fields.insert("octaves".into(), Value::UInt(u64::from(terrain.octaves)));
    fields.insert("lacunarity".into(), Value::Float32(terrain.lacunarity));
    fields.insert("gain".into(), Value::Float32(terrain.gain));
    fields.insert(
        "domain_warp_amplitude".into(),
        Value::Float32(terrain.domain_warp_amplitude),
    );
    fields.insert(
        "domain_warp_frequency".into(),
        Value::Float32(terrain.domain_warp_frequency),
    );
    fields.insert("skirt_depth".into(), Value::Float32(terrain.skirt_depth));
    fields.insert(
        "collision_enabled".into(),
        Value::Bool(terrain.collision_enabled),
    );
    fields.insert(
        "material_asset".into(),
        Value::Asset(engine_serialize::AssetId::new(&terrain.material_asset)),
    );
    fields.insert(
        "lod_distances".into(),
        Value::List(
            terrain
                .lod_distances
                .iter()
                .copied()
                .map(Value::Float32)
                .collect(),
        ),
    );
    fields.insert(
        "lod_hysteresis".into(),
        Value::Float32(terrain.lod_hysteresis),
    );
    fields
}

fn deserialize_terrain(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let mut terrain = TerrainVolume::default();
    if let Some(Value::Bool(value)) = fields.get("enabled") {
        terrain.enabled = *value;
    }
    if let Some(Value::UInt(value)) = fields.get("seed") {
        terrain.seed = *value;
    }
    macro_rules! float_field {
        ($name:literal, $field:ident) => {
            if let Some(Value::Float32(value)) = fields.get($name) {
                terrain.$field = *value;
            }
        };
    }
    float_field!("chunk_size", chunk_size);
    float_field!("height_scale", height_scale);
    float_field!("frequency", frequency);
    float_field!("lacunarity", lacunarity);
    float_field!("gain", gain);
    float_field!("domain_warp_amplitude", domain_warp_amplitude);
    float_field!("domain_warp_frequency", domain_warp_frequency);
    float_field!("skirt_depth", skirt_depth);
    float_field!("lod_hysteresis", lod_hysteresis);
    if let Some(Value::UInt(value)) = fields.get("base_resolution") {
        terrain.base_resolution = *value as u32;
    }
    if let Some(Value::UInt(value)) = fields.get("octaves") {
        terrain.octaves = *value as u32;
    }
    if let Some(Value::Bool(value)) = fields.get("collision_enabled") {
        terrain.collision_enabled = *value;
    }
    match fields.get("material_asset") {
        Some(Value::Asset(value)) => terrain.material_asset = value.id.clone(),
        Some(Value::Str(value)) => terrain.material_asset = value.clone(),
        _ => {}
    }
    if let Some(Value::List(values)) = fields.get("lod_distances") {
        terrain.lod_distances = values
            .iter()
            .filter_map(|value| match value {
                Value::Float32(value) => Some(*value),
                _ => None,
            })
            .collect();
    }
    Box::new(terrain)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn component_round_trips_through_registry_hooks() {
        let terrain = TerrainVolume {
            seed: 42,
            material_asset: "ground".into(),
            ..Default::default()
        };
        let fields = serialize_terrain(&terrain);
        let restored = deserialize_terrain(&fields)
            .downcast::<TerrainVolume>()
            .expect("terrain type");
        assert_eq!(*restored, terrain);
    }

    #[test]
    fn validation_rejects_non_power_of_two_grid() {
        let terrain = TerrainVolume {
            base_resolution: 64,
            ..Default::default()
        };
        assert!(terrain.validate().is_err());
    }

    #[test]
    fn validation_matches_procgen_limits_and_rejects_height_overflow() {
        assert!(TerrainVolume {
            frequency: 65537.0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(TerrainVolume {
            domain_warp_amplitude: 65537.0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(TerrainVolume {
            height_scale: f32::MAX,
            skirt_depth: f32::MAX,
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn registry_validator_rejects_invalid_or_lossy_fields() {
        let mut registry = ComponentRegistry::new();
        register_terrain_extensions(&mut registry);
        let mut fields = serialize_terrain(&TerrainVolume::default());
        fields.insert("base_resolution".into(), Value::UInt(64));
        assert!(registry
            .validate_fields(TerrainVolume::TYPE_ID, &fields)
            .unwrap_err()
            .contains("base_resolution"));

        fields.insert("base_resolution".into(), Value::UInt(u64::MAX));
        assert!(registry
            .validate_fields(TerrainVolume::TYPE_ID, &fields)
            .unwrap_err()
            .contains("base_resolution"));

        fields.insert("base_resolution".into(), Value::Str("65".into()));
        assert!(registry
            .validate_fields(TerrainVolume::TYPE_ID, &fields)
            .unwrap_err()
            .contains("base_resolution"));
    }

    #[test]
    fn registry_rejects_multiple_terrain_volumes_in_one_scene() {
        let mut registry = ComponentRegistry::new();
        register_terrain_extensions(&mut registry);
        let mut scene = engine_scene::sample_scene();
        let component = engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: serialize_terrain(&TerrainVolume::default()),
        };
        scene.entities[0]
            .components
            .insert(TerrainVolume::TYPE_ID.to_string(), component.clone());
        scene.entities[1]
            .components
            .insert(TerrainVolume::TYPE_ID.to_string(), component);

        let error =
            match engine_scene::World::try_from_scene_with_registry(&scene, Arc::new(registry)) {
                Ok(_) => panic!("duplicate singleton terrain must fail"),
                Err(error) => error,
            };
        assert!(error.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            engine_scene::SceneLoadDiagnostic::DuplicateSingletonComponent { .. }
        )));
    }
}
