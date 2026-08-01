use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{validate_component_fields, validate_component_type_key, GameplayComponentValue};
use crate::{validate_entity_id, validate_prefab_id};

pub const MAX_RUNTIME_MESH_VERTICES: usize = 1_000_000;
pub const MAX_RUNTIME_MESH_INDICES: usize = 3_000_000;
pub const MAX_RUNTIME_PREFAB_ENTITIES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayRuntimeMesh {
    pub positions: Vec<[f32; 3]>,
    #[serde(default)]
    pub normals: Vec<[f32; 3]>,
    #[serde(default)]
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl GameplayRuntimeMesh {
    pub fn validate(&self) -> Result<(), String> {
        if self.positions.is_empty() || self.positions.len() > MAX_RUNTIME_MESH_VERTICES {
            return Err(format!(
                "runtime mesh requires 1..={MAX_RUNTIME_MESH_VERTICES} vertices"
            ));
        }
        if self.indices.is_empty()
            || self.indices.len() > MAX_RUNTIME_MESH_INDICES
            || !self.indices.len().is_multiple_of(3)
        {
            return Err(format!(
                "runtime mesh requires a non-empty triangle index list of at most {MAX_RUNTIME_MESH_INDICES} entries"
            ));
        }
        if !self.normals.is_empty() && self.normals.len() != self.positions.len() {
            return Err("runtime mesh normals must be empty or match positions".into());
        }
        if !self.uvs.is_empty() && self.uvs.len() != self.positions.len() {
            return Err("runtime mesh UVs must be empty or match positions".into());
        }
        if self
            .positions
            .iter()
            .flatten()
            .chain(self.normals.iter().flatten())
            .chain(self.uvs.iter().flatten())
            .any(|value| !value.is_finite())
        {
            return Err("runtime mesh attributes must be finite".into());
        }
        let vertex_count = self.positions.len() as u32;
        if self.indices.iter().any(|index| *index >= vertex_count) {
            return Err("runtime mesh index exceeds the vertex count".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayRuntimeMaterial {
    pub base_color: [f32; 4],
    #[serde(default)]
    pub metallic: f32,
    #[serde(default = "default_roughness")]
    pub roughness: f32,
    #[serde(default = "default_occlusion")]
    pub ambient_occlusion: f32,
    #[serde(default)]
    pub emissive: [f32; 3],
    #[serde(default)]
    pub base_color_texture: Option<String>,
    #[serde(default)]
    pub normal_texture: Option<String>,
    #[serde(default)]
    pub metallic_roughness_texture: Option<String>,
    #[serde(default)]
    pub occlusion_texture: Option<String>,
    #[serde(default)]
    pub emissive_texture: Option<String>,
    #[serde(default)]
    pub double_sided: bool,
    #[serde(default)]
    pub alpha_cutoff: Option<f32>,
    #[serde(default)]
    pub blend: bool,
}

impl GameplayRuntimeMaterial {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .base_color
            .iter()
            .chain(self.emissive.iter())
            .any(|value| !value.is_finite())
            || !self.metallic.is_finite()
            || !self.roughness.is_finite()
            || !self.ambient_occlusion.is_finite()
            || self
                .base_color
                .iter()
                .any(|value| !(0.0..=1.0).contains(value))
            || !(0.0..=1.0).contains(&self.metallic)
            || !(0.0..=1.0).contains(&self.roughness)
            || !(0.0..=1.0).contains(&self.ambient_occlusion)
            || self.emissive.iter().any(|value| *value < 0.0)
            || self
                .alpha_cutoff
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err("runtime material PBR values are outside their finite ranges".into());
        }
        for texture in [
            &self.base_color_texture,
            &self.normal_texture,
            &self.metallic_roughness_texture,
            &self.occlusion_texture,
            &self.emissive_texture,
        ]
        .into_iter()
        .flatten()
        {
            validate_entity_id(texture)?;
        }
        Ok(())
    }
}

fn default_roughness() -> f32 {
    1.0
}

fn default_occlusion() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayRuntimePrefabEntity {
    pub entity_id: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub components: BTreeMap<String, BTreeMap<String, GameplayComponentValue>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayRuntimePrefab {
    pub entities: Vec<GameplayRuntimePrefabEntity>,
}

impl GameplayRuntimePrefab {
    pub fn validate(&self) -> Result<(), String> {
        if self.entities.is_empty() || self.entities.len() > MAX_RUNTIME_PREFAB_ENTITIES {
            return Err(format!(
                "runtime prefab requires 1..={MAX_RUNTIME_PREFAB_ENTITIES} entities"
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for entity in &self.entities {
            validate_entity_id(&entity.entity_id)?;
            if !ids.insert(entity.entity_id.as_str()) {
                return Err(format!(
                    "duplicate runtime prefab entity '{}'",
                    entity.entity_id
                ));
            }
            if entity.name.as_ref().is_some_and(|name| name.len() > 256) {
                return Err("runtime prefab entity name exceeds 256 bytes".into());
            }
            for (component, fields) in &entity.components {
                validate_component_type_key(component)?;
                validate_component_fields(fields)?;
            }
        }
        for entity in &self.entities {
            if let Some(parent) = &entity.parent {
                validate_entity_id(parent)?;
                if !ids.contains(parent.as_str()) || parent == &entity.entity_id {
                    return Err(format!(
                        "runtime prefab entity '{}' has invalid parent '{parent}'",
                        entity.entity_id
                    ));
                }
            }
        }
        let parents = self
            .entities
            .iter()
            .map(|entity| (entity.entity_id.as_str(), entity.parent.as_deref()))
            .collect::<BTreeMap<_, _>>();
        for entity in &self.entities {
            let mut visited = std::collections::BTreeSet::new();
            let mut current = Some(entity.entity_id.as_str());
            while let Some(id) = current {
                if !visited.insert(id) {
                    return Err(format!(
                        "runtime prefab hierarchy contains a cycle at '{id}'"
                    ));
                }
                current = parents.get(id).copied().flatten();
            }
        }
        Ok(())
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameplayTerrainBrushMode {
    Add,
    #[default]
    Subtract,
    Smooth,
    SetDensity,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayTerrainBrush {
    pub center: [f64; 3],
    pub radius: f64,
    pub strength: f32,
    #[serde(default)]
    pub mode: GameplayTerrainBrushMode,
    #[serde(default)]
    pub target_density: f32,
    #[serde(default)]
    pub material: Option<u16>,
}

impl GameplayTerrainBrush {
    pub fn validate(&self) -> Result<(), String> {
        if self.center.iter().any(|value| !value.is_finite())
            || !self.radius.is_finite()
            || self.radius <= 0.0
            || !self.strength.is_finite()
            || self.strength <= 0.0
            || (self.mode == GameplayTerrainBrushMode::Smooth && self.strength > 1.0)
            || !self.target_density.is_finite()
        {
            return Err("terrain brush has invalid centre, radius, strength or density".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayRuntimeAssetResult {
    pub request_id: u32,
    pub asset_id: String,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayTerrainBrushRequest {
    pub owner_entity_id: String,
    pub request_id: u32,
    pub terrain_entity_id: String,
    pub brush: GameplayTerrainBrush,
}

pub(crate) fn validate_runtime_asset_id(value: &str) -> Result<(), String> {
    validate_prefab_id(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameplayCommand;

    fn triangle() -> GameplayRuntimeMesh {
        GameplayRuntimeMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn runtime_mesh_command_has_a_stable_json_contract() {
        let command = GameplayCommand::RegisterRuntimeMesh {
            request_id: 17,
            asset_id: "generated-rock".into(),
            mesh: triangle(),
        };
        command.validate().unwrap();
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(
            json,
            r#"{"type":"register_runtime_mesh","request_id":17,"asset_id":"generated-rock","mesh":{"positions":[[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,1.0,0.0]],"normals":[],"uvs":[],"indices":[0,1,2]}}"#
        );
        assert_eq!(
            serde_json::from_str::<GameplayCommand>(&json).unwrap(),
            command
        );
    }

    #[test]
    fn mesh_and_brush_reject_non_finite_or_invalid_payloads() {
        let mut mesh = triangle();
        mesh.positions[0][0] = f32::NAN;
        assert!(mesh.validate().is_err());
        let brush = GameplayTerrainBrush {
            center: [0.0, f64::INFINITY, 0.0],
            radius: 4.0,
            strength: 1.0,
            mode: GameplayTerrainBrushMode::Subtract,
            target_density: 0.0,
            material: None,
        };
        assert!(brush.validate().is_err());
    }

    #[test]
    fn prefab_hierarchy_rejects_cycles() {
        let entity = |entity_id: &str, parent: &str| GameplayRuntimePrefabEntity {
            entity_id: entity_id.into(),
            parent: Some(parent.into()),
            name: None,
            enabled: true,
            components: BTreeMap::new(),
        };
        let prefab = GameplayRuntimePrefab {
            entities: vec![entity("a", "b"), entity("b", "a")],
        };
        assert!(prefab
            .validate()
            .unwrap_err()
            .contains("hierarchy contains a cycle"));
    }
}
