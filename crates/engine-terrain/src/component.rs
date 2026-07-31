use std::collections::BTreeMap;

use engine_scene::{
    Component, ComponentExtension, ComponentMeta, ComponentRegistry, ComponentStorageDyn,
    ScriptAccess, SparseSet,
};
use engine_serialize::Value;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Geometric domain used by a terrain volume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerrainTopology {
    /// Infinite X/Z heightfield centered around the streaming focus.
    #[default]
    Planar,
    /// Six independently streamed quadtree faces projected onto a sphere.
    CubeSphere,
}

/// Coordinate system used to project authored material textures onto terrain.
///
/// This policy belongs to the terrain/rendering contract rather than gameplay:
/// it does not choose textures or biome rules, it only defines how every PBR
/// texture slot is sampled continuously across streamed chunk boundaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerrainMaterialProjection {
    /// Select world-relative triplanar for planar terrain and planet-relative
    /// triplanar for cube-sphere terrain.
    #[default]
    Automatic,
    /// Preserve the generated mesh UVs. Cube-sphere patches use independent
    /// UV islands, so this mode is intended for explicitly patch-aware assets.
    MeshUv,
    /// Project from logical world coordinates with a stable repeating phase.
    WorldTriplanar,
    /// Project from coordinates relative to `planet_center`.
    PlanetTriplanar,
}

/// Serializable, game-authored terrain parameter block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainVolume {
    pub enabled: bool,
    #[serde(default)]
    pub topology: TerrainTopology,
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
    /// Coordinate policy shared by base-color, normal and all PBR texture
    /// slots. Ordinary non-terrain renderables continue to use mesh UVs.
    #[serde(default)]
    pub material_projection: TerrainMaterialProjection,
    /// World-space length covered by one repeat of a projected texture.
    #[serde(default = "default_material_tile_size")]
    pub material_tile_size: f32,
    /// Exponent used to sharpen triplanar axis weights.
    #[serde(default = "default_triplanar_blend_sharpness")]
    pub triplanar_blend_sharpness: f32,
    /// Increasing world-space distance cutoffs, one per LOD.
    pub lod_distances: Vec<f32>,
    /// World-space dead band applied when splitting/merging CDLOD nodes.
    pub lod_hysteresis: f32,
    /// Logical-space center used by [`TerrainTopology::CubeSphere`].
    #[serde(default)]
    pub planet_center: [f64; 3],
    /// Base sea-level radius used by [`TerrainTopology::CubeSphere`].
    #[serde(default = "default_planet_radius")]
    pub planet_radius: f64,
    /// Cube-face quadtree depth. LOD 0 is the finest level and this value is
    /// the one-patch-per-face root level.
    #[serde(default = "default_planet_max_lod")]
    pub planet_max_lod: u8,
    /// Reject cube-sphere nodes that are conservatively hidden beyond the
    /// geometric horizon before they enter generation or render extraction.
    #[serde(default = "default_true")]
    pub horizon_culling: bool,
    /// Encode a parent-compatible radial target in generated vertex normals
    /// so the renderer can continuously morph between adjacent quadtree LODs.
    #[serde(default = "default_true")]
    pub geomorph_enabled: bool,
    /// Fraction of an LOD cutoff at which morphing begins. `1.0` disables the
    /// transition interval; lower values produce a wider continuous blend.
    #[serde(default = "default_geomorph_start_ratio")]
    pub geomorph_start_ratio: f32,
}

const fn default_planet_radius() -> f64 {
    1_000.0
}

const fn default_planet_max_lod() -> u8 {
    2
}

const fn default_true() -> bool {
    true
}

const fn default_geomorph_start_ratio() -> f32 {
    0.7
}

const fn default_material_tile_size() -> f32 {
    16.0
}

const fn default_triplanar_blend_sharpness() -> f32 {
    4.0
}

impl Default for TerrainVolume {
    fn default() -> Self {
        Self {
            enabled: true,
            topology: TerrainTopology::Planar,
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
            material_projection: TerrainMaterialProjection::Automatic,
            material_tile_size: default_material_tile_size(),
            triplanar_blend_sharpness: default_triplanar_blend_sharpness(),
            lod_distances: vec![160.0, 320.0, 640.0],
            lod_hysteresis: 16.0,
            planet_center: [0.0; 3],
            planet_radius: default_planet_radius(),
            planet_max_lod: default_planet_max_lod(),
            horizon_culling: default_true(),
            geomorph_enabled: default_true(),
            geomorph_start_ratio: default_geomorph_start_ratio(),
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
        if !self.material_tile_size.is_finite() || self.material_tile_size <= 0.0 {
            return Err(TerrainConfigError::Invalid(
                "material_tile_size must be finite and positive",
            ));
        }
        if !self.triplanar_blend_sharpness.is_finite()
            || !(1.0..=32.0).contains(&self.triplanar_blend_sharpness)
        {
            return Err(TerrainConfigError::Invalid(
                "triplanar_blend_sharpness must be finite and in [1, 32]",
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
        if !self.geomorph_start_ratio.is_finite()
            || !(0.0..1.0).contains(&self.geomorph_start_ratio)
        {
            return Err(TerrainConfigError::Invalid(
                "geomorph_start_ratio must be finite and in [0, 1)",
            ));
        }
        if self.topology == TerrainTopology::CubeSphere {
            if self.planet_center.iter().any(|value| !value.is_finite()) {
                return Err(TerrainConfigError::Invalid(
                    "planet_center must contain only finite values",
                ));
            }
            if !self.planet_radius.is_finite()
                || self.planet_radius <= f64::from(self.height_scale + self.skirt_depth)
                || self.planet_radius > f64::from(f32::MAX / 4.0)
            {
                return Err(TerrainConfigError::Invalid(
                    "planet_radius must be finite, greater than height_scale plus skirt_depth, and representable by local f32 patch geometry",
                ));
            }
            if self.planet_max_lod > 15 {
                return Err(TerrainConfigError::Invalid(
                    "planet_max_lod must be in 0..=15",
                ));
            }
            if self.lod_distances.len() != usize::from(self.planet_max_lod) + 1 {
                return Err(TerrainConfigError::Invalid(
                    "cube-sphere terrain requires one lod_distances entry per level (planet_max_lod + 1)",
                ));
            }
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
        mix(&[match self.topology {
            TerrainTopology::Planar => 0,
            TerrainTopology::CubeSphere => 1,
        }]);
        mix(&[match self.material_projection {
            TerrainMaterialProjection::Automatic => 0,
            TerrainMaterialProjection::MeshUv => 1,
            TerrainMaterialProjection::WorldTriplanar => 2,
            TerrainMaterialProjection::PlanetTriplanar => 3,
        }]);
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
            self.material_tile_size,
            self.triplanar_blend_sharpness,
        ] {
            mix(&value.to_bits().to_le_bytes());
        }
        mix(&self.base_resolution.to_le_bytes());
        mix(&self.octaves.to_le_bytes());
        mix(&[self.collision_enabled as u8]);
        for value in self.planet_center {
            mix(&value.to_bits().to_le_bytes());
        }
        mix(&self.planet_radius.to_bits().to_le_bytes());
        mix(&[
            self.planet_max_lod,
            self.horizon_culling as u8,
            self.geomorph_enabled as u8,
        ]);
        mix(&self.geomorph_start_ratio.to_bits().to_le_bytes());
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
    }
    crate::placement::register_planet_surface_anchor(registry);
    crate::transition_component::register_planet_scene_transition(registry);
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
    serialize_terrain_fields(terrain)
}

/// Serialize the canonical scene fields for an authored terrain volume.
///
/// Editor factories use this entry point so newly added terrain parameters
/// cannot drift from the runtime component registry's defaults.
pub fn serialize_terrain_fields(terrain: &TerrainVolume) -> BTreeMap<String, Value> {
    let mut fields = BTreeMap::new();
    fields.insert("enabled".into(), Value::Bool(terrain.enabled));
    fields.insert(
        "topology".into(),
        Value::Enum(
            match terrain.topology {
                TerrainTopology::Planar => "Planar",
                TerrainTopology::CubeSphere => "CubeSphere",
            }
            .into(),
        ),
    );
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
        "material_projection".into(),
        Value::Enum(
            match terrain.material_projection {
                TerrainMaterialProjection::Automatic => "Automatic",
                TerrainMaterialProjection::MeshUv => "MeshUv",
                TerrainMaterialProjection::WorldTriplanar => "WorldTriplanar",
                TerrainMaterialProjection::PlanetTriplanar => "PlanetTriplanar",
            }
            .into(),
        ),
    );
    fields.insert(
        "material_tile_size".into(),
        Value::Float32(terrain.material_tile_size),
    );
    fields.insert(
        "triplanar_blend_sharpness".into(),
        Value::Float32(terrain.triplanar_blend_sharpness),
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
    fields.insert(
        "planet_center".into(),
        Value::List(
            terrain
                .planet_center
                .into_iter()
                .map(Value::Float64)
                .collect(),
        ),
    );
    fields.insert(
        "planet_radius".into(),
        Value::Float64(terrain.planet_radius),
    );
    fields.insert(
        "planet_max_lod".into(),
        Value::UInt(u64::from(terrain.planet_max_lod)),
    );
    fields.insert(
        "horizon_culling".into(),
        Value::Bool(terrain.horizon_culling),
    );
    fields.insert(
        "geomorph_enabled".into(),
        Value::Bool(terrain.geomorph_enabled),
    );
    fields.insert(
        "geomorph_start_ratio".into(),
        Value::Float32(terrain.geomorph_start_ratio),
    );
    fields
}

fn deserialize_terrain(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let mut terrain = TerrainVolume::default();
    if let Some(Value::Bool(value)) = fields.get("enabled") {
        terrain.enabled = *value;
    }
    match fields.get("topology") {
        Some(Value::Enum(value)) | Some(Value::Str(value)) if value == "CubeSphere" => {
            terrain.topology = TerrainTopology::CubeSphere;
        }
        _ => {}
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
    float_field!("geomorph_start_ratio", geomorph_start_ratio);
    float_field!("material_tile_size", material_tile_size);
    float_field!("triplanar_blend_sharpness", triplanar_blend_sharpness);
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
    terrain.material_projection = match fields.get("material_projection") {
        Some(Value::Enum(value)) | Some(Value::Str(value)) if value == "MeshUv" => {
            TerrainMaterialProjection::MeshUv
        }
        Some(Value::Enum(value)) | Some(Value::Str(value)) if value == "WorldTriplanar" => {
            TerrainMaterialProjection::WorldTriplanar
        }
        Some(Value::Enum(value)) | Some(Value::Str(value)) if value == "PlanetTriplanar" => {
            TerrainMaterialProjection::PlanetTriplanar
        }
        _ => TerrainMaterialProjection::Automatic,
    };
    if let Some(Value::List(values)) = fields.get("lod_distances") {
        terrain.lod_distances = values
            .iter()
            .filter_map(|value| match value {
                Value::Float32(value) => Some(*value),
                _ => None,
            })
            .collect();
    }
    if let Some(Value::List(values)) = fields.get("planet_center") {
        if let [x, y, z] = values.as_slice() {
            let number = |value: &Value| match value {
                Value::Float64(value) => Some(*value),
                Value::Float32(value) => Some(f64::from(*value)),
                _ => None,
            };
            if let (Some(x), Some(y), Some(z)) = (number(x), number(y), number(z)) {
                terrain.planet_center = [x, y, z];
            }
        }
    }
    match fields.get("planet_radius") {
        Some(Value::Float64(value)) => terrain.planet_radius = *value,
        Some(Value::Float32(value)) => terrain.planet_radius = f64::from(*value),
        _ => {}
    }
    if let Some(Value::UInt(value)) = fields.get("planet_max_lod") {
        terrain.planet_max_lod = u8::try_from(*value).unwrap_or(u8::MAX);
    }
    if let Some(Value::Bool(value)) = fields.get("horizon_culling") {
        terrain.horizon_culling = *value;
    }
    if let Some(Value::Bool(value)) = fields.get("geomorph_enabled") {
        terrain.geomorph_enabled = *value;
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
            material_projection: TerrainMaterialProjection::PlanetTriplanar,
            material_tile_size: 128.0,
            triplanar_blend_sharpness: 6.0,
            topology: TerrainTopology::CubeSphere,
            planet_center: [1.0e9, -2.0e9, 3.0e9],
            planet_radius: 6_000_000.0,
            ..Default::default()
        };
        let fields = serialize_terrain(&terrain);
        let restored = deserialize_terrain(&fields)
            .downcast::<TerrainVolume>()
            .expect("terrain type");
        assert_eq!(*restored, terrain);
    }

    #[test]
    fn validation_rejects_invalid_projected_material_parameters() {
        assert!(TerrainVolume {
            material_tile_size: 0.0,
            ..TerrainVolume::default()
        }
        .validate()
        .unwrap_err()
        .to_string()
        .contains("material_tile_size"));
        assert!(TerrainVolume {
            triplanar_blend_sharpness: 33.0,
            ..TerrainVolume::default()
        }
        .validate()
        .unwrap_err()
        .to_string()
        .contains("triplanar_blend_sharpness"));
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
    fn cube_sphere_validation_requires_a_complete_lod_chain() {
        let terrain = TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            planet_max_lod: 3,
            lod_distances: vec![100.0, 200.0, 400.0],
            ..TerrainVolume::default()
        };
        assert!(terrain
            .validate()
            .unwrap_err()
            .to_string()
            .contains("planet_max_lod + 1"));
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
    fn registry_allows_multiple_terrain_volumes_in_one_scene() {
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

        let world = engine_scene::World::try_from_scene_with_registry(&scene, Arc::new(registry))
            .expect("terrain volumes are ordinary per-entity components");
        assert_eq!(world.query::<TerrainVolume>().count(), 2);
    }
}
