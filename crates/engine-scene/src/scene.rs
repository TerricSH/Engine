use engine_renderer::ToneMapping;
use engine_serialize::{
    AssetId, ComponentTypeId, EngineVersion, PersistentId, SchemaVersion, Value,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ECS_SCENE_CONTRACT: &str = "ECSScene-v0.1.0";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub schema_version: SchemaVersion,
    pub engine_version: EngineVersion,
    pub scene_id: PersistentId,
    pub name: String,
    pub entities: Vec<EntityRecord>,
    pub scene_settings: SceneSettings,
    pub dependencies: Vec<AssetId>,
    pub diagnostics_policy: DiagnosticsPolicy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityRecord {
    pub persistent_id: PersistentId,
    pub parent: Option<PersistentId>,
    pub name: Option<String>,
    pub enabled: bool,
    pub components: BTreeMap<ComponentTypeId, ComponentRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentRecord {
    pub schema_version: SchemaVersion,
    pub enabled: bool,
    pub fields: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneSettings {
    pub active_camera: Option<PersistentId>,
    pub default_render_layer: String,
    pub fixed_timestep_seconds: f32,
    pub gravity: Option<[f32; 3]>,
    pub ambient: [f32; 4],
    pub environment_map: Option<AssetId>,
    pub tone_mapping: ToneMapping,
}

impl Default for SceneSettings {
    fn default() -> Self {
        Self {
            active_camera: None,
            default_render_layer: "Default".to_string(),
            fixed_timestep_seconds: 1.0 / 60.0,
            gravity: Some([0.0, -9.81, 0.0]),
            ambient: [0.03, 0.03, 0.03, 1.0],
            environment_map: None,
            tone_mapping: ToneMapping::Aces,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticsPolicy {
    Strict,
    EditorRepair,
}

pub fn sample_scene() -> Scene {
    let mut camera_components = BTreeMap::new();
    camera_components.insert("engine.camera".to_string(), component(BTreeMap::new()));

    let mut renderable_fields = BTreeMap::new();
    renderable_fields.insert("mesh".to_string(), Value::Asset(AssetId::new("mesh-cube")));
    renderable_fields.insert(
        "material".to_string(),
        Value::Asset(AssetId::new("mat-default")),
    );
    renderable_fields.insert("visible".to_string(), Value::Bool(true));
    renderable_fields.insert(
        "render_layer".to_string(),
        Value::Str("Default".to_string()),
    );
    renderable_fields.insert("cast_shadows".to_string(), Value::Bool(true));
    let mut renderable_components = BTreeMap::new();
    renderable_components.insert(
        "engine.renderable".to_string(),
        component(renderable_fields),
    );

    Scene {
        schema_version: SchemaVersion::new(0, 1, 0),
        engine_version: "0.1.0".to_string(),
        scene_id: "scene-gate04-valid".to_string(),
        name: "Gate 4 Validation Scene".to_string(),
        entities: vec![
            EntityRecord {
                persistent_id: "camera-main".to_string(),
                parent: None,
                name: Some("Main Camera".to_string()),
                enabled: true,
                components: camera_components,
            },
            EntityRecord {
                persistent_id: "cube-01".to_string(),
                parent: None,
                name: Some("Cube".to_string()),
                enabled: true,
                components: renderable_components,
            },
        ],
        scene_settings: SceneSettings {
            active_camera: Some("camera-main".to_string()),
            ..SceneSettings::default()
        },
        dependencies: vec![AssetId::new("mesh-cube"), AssetId::new("mat-default")],
        diagnostics_policy: DiagnosticsPolicy::Strict,
    }
}

fn component(fields: BTreeMap<String, Value>) -> ComponentRecord {
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}
