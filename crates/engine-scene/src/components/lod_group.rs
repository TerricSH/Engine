use std::collections::BTreeMap;

use engine_serialize::{AssetId, Value};
use serde::{Deserialize, Serialize};

use crate::Component;

use super::field_as_f32;

/// One distance threshold in a [`LodGroup`]. Once the base-camera distance is
/// at least `distance`, extraction substitutes this mesh and optional
/// material for the entity's [`super::Renderable`] assets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LodLevel {
    pub distance: f32,
    pub mesh_asset: String,
    pub material_asset: Option<String>,
}

/// Scene-side LOD/HLOD selection policy.
///
/// `minimum_distance` lets an authored proxy stay hidden while its source
/// objects are near. Pairing that proxy with source objects whose
/// `cull_distance` matches the proxy minimum produces a hand-authored HLOD
/// group without a backend-specific representation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LodGroup {
    pub minimum_distance: f32,
    /// Zero disables far-distance culling.
    pub cull_distance: f32,
    pub levels: Vec<LodLevel>,
}

impl Default for LodGroup {
    fn default() -> Self {
        Self {
            minimum_distance: 0.0,
            cull_distance: 0.0,
            levels: Vec::new(),
        }
    }
}

impl Component for LodGroup {
    const TYPE_ID: &'static str = "engine.lod_group";
}

impl LodGroup {
    /// Resolve the assets visible at `distance`. `None` means the entity is
    /// outside this group's authored distance range.
    pub fn select_assets<'a>(
        &'a self,
        distance: f32,
        base_mesh: &'a str,
        base_material: &'a str,
    ) -> Option<(&'a str, &'a str)> {
        if distance < self.minimum_distance
            || (self.cull_distance > 0.0 && distance >= self.cull_distance)
        {
            return None;
        }
        let mut mesh = base_mesh;
        let mut material = base_material;
        for level in &self.levels {
            if distance < level.distance {
                break;
            }
            mesh = &level.mesh_asset;
            if let Some(level_material) = &level.material_asset {
                material = level_material;
            }
        }
        Some((mesh, material))
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.minimum_distance.is_finite()
            || self.minimum_distance < 0.0
            || !self.cull_distance.is_finite()
            || self.cull_distance < 0.0
            || (self.cull_distance > 0.0 && self.cull_distance <= self.minimum_distance)
        {
            return Err(
                "LOD minimum/cull distances must be finite, non-negative, and ordered".into(),
            );
        }
        let mut previous = 0.0_f32;
        for level in &self.levels {
            if !level.distance.is_finite()
                || level.distance <= previous
                || level.distance < self.minimum_distance
                || (self.cull_distance > 0.0 && level.distance >= self.cull_distance)
                || level.mesh_asset.trim().is_empty()
                || level
                    .material_asset
                    .as_ref()
                    .is_some_and(|asset| asset.trim().is_empty())
            {
                return Err(
                    "LOD levels require strictly increasing finite distances inside the group range and non-empty assets"
                        .into(),
                );
            }
            previous = level.distance;
        }
        Ok(())
    }
}

pub fn serialize_lod_group_fields(group: &LodGroup) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            "minimum_distance".into(),
            Value::Float32(group.minimum_distance),
        ),
        ("cull_distance".into(), Value::Float32(group.cull_distance)),
        (
            "levels".into(),
            Value::List(
                group
                    .levels
                    .iter()
                    .map(|level| {
                        let mut fields = BTreeMap::from([
                            ("distance".into(), Value::Float32(level.distance)),
                            ("mesh".into(), Value::Asset(AssetId::new(&level.mesh_asset))),
                        ]);
                        if let Some(material) = &level.material_asset {
                            fields.insert("material".into(), Value::Asset(AssetId::new(material)));
                        }
                        Value::Map(fields)
                    })
                    .collect(),
            ),
        ),
    ])
}

pub fn deserialize_lod_group_fields(fields: &BTreeMap<String, Value>) -> LodGroup {
    let minimum_distance = fields
        .get("minimum_distance")
        .map(field_as_f32)
        .unwrap_or(0.0);
    let cull_distance = fields.get("cull_distance").map(field_as_f32).unwrap_or(0.0);
    let levels = match fields.get("levels") {
        Some(Value::List(levels)) => levels
            .iter()
            .filter_map(|value| {
                let Value::Map(fields) = value else {
                    return None;
                };
                let distance = fields.get("distance").map(field_as_f32)?;
                let mesh_asset = match fields.get("mesh") {
                    Some(Value::Asset(asset)) => asset.id.clone(),
                    Some(Value::Str(asset)) => asset.clone(),
                    _ => return None,
                };
                let material_asset = match fields.get("material") {
                    Some(Value::Asset(asset)) => Some(asset.id.clone()),
                    Some(Value::Str(asset)) => Some(asset.clone()),
                    _ => None,
                };
                Some(LodLevel {
                    distance,
                    mesh_asset,
                    material_asset,
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    LodGroup {
        minimum_distance,
        cull_distance,
        levels,
    }
}

pub fn serialize_lod_group(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    serialize_lod_group_fields(
        component
            .downcast_ref::<LodGroup>()
            .expect("LodGroup expected"),
    )
}

pub fn deserialize_lod_group(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    Box::new(deserialize_lod_group_fields(fields))
}

pub fn validate_lod_group_fields(fields: &BTreeMap<String, Value>) -> Result<(), String> {
    deserialize_lod_group_fields(fields).validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> LodGroup {
        LodGroup {
            minimum_distance: 0.0,
            cull_distance: 100.0,
            levels: vec![
                LodLevel {
                    distance: 10.0,
                    mesh_asset: "mesh.lod1".into(),
                    material_asset: None,
                },
                LodLevel {
                    distance: 40.0,
                    mesh_asset: "mesh.lod2".into(),
                    material_asset: Some("material.proxy".into()),
                },
            ],
        }
    }

    #[test]
    fn distance_selection_preserves_base_and_applies_ordered_levels() {
        let group = group();
        assert_eq!(
            group.select_assets(5.0, "mesh.base", "material.base"),
            Some(("mesh.base", "material.base"))
        );
        assert_eq!(
            group.select_assets(20.0, "mesh.base", "material.base"),
            Some(("mesh.lod1", "material.base"))
        );
        assert_eq!(
            group.select_assets(50.0, "mesh.base", "material.base"),
            Some(("mesh.lod2", "material.proxy"))
        );
        assert_eq!(
            group.select_assets(100.0, "mesh.base", "material.base"),
            None
        );
    }

    #[test]
    fn field_map_roundtrip_preserves_valid_group() {
        let group = group();
        let fields = serialize_lod_group_fields(&group);
        assert!(validate_lod_group_fields(&fields).is_ok());
        assert_eq!(deserialize_lod_group_fields(&fields), group);
    }
}
