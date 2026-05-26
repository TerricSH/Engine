#![forbid(unsafe_code)]

use engine_renderer::{
    AxisAlignedBox, ClearFlags, LightItem, LightKind, Rect, RenderFrameInput, RenderView,
    RenderableItem, ShadowMode, ToneMapping, ViewCompose, IDENTITY_MAT4,
};
use engine_serialize::{
    AssetId, ComponentTypeId, Diagnostic, DiagnosticSeverity, EngineVersion, PersistentId,
    SchemaVersion, Value,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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

pub fn validate_scene(scene: &Scene) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut ids = BTreeSet::new();

    for entity in &scene.entities {
        if !ids.insert(entity.persistent_id.clone()) {
            diagnostics.push(
                Diagnostic::new(
                    "SC0015",
                    DiagnosticSeverity::Error,
                    "engine-scene",
                    "duplicate entity persistent_id",
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .path(format!("entities[{}].persistent_id", entity.persistent_id)),
            );
        }
    }

    for entity in &scene.entities {
        if let Some(parent) = &entity.parent {
            if !ids.contains(parent) {
                diagnostics.push(
                    Diagnostic::new(
                        "SC0016",
                        DiagnosticSeverity::Error,
                        "engine-scene",
                        "entity parent does not exist in this scene",
                    )
                    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                    .path(format!("entities[{}].parent", entity.persistent_id)),
                );
            }
        }
    }

    if let Some(active_camera) = &scene.scene_settings.active_camera {
        let camera_ok = scene.entities.iter().any(|entity| {
            entity.enabled
                && &entity.persistent_id == active_camera
                && entity
                    .components
                    .get("engine.camera")
                    .map(|component| component.enabled)
                    .unwrap_or(false)
        });
        if !camera_ok {
            diagnostics.push(Diagnostic::new(
                "SC0017",
                DiagnosticSeverity::Error,
                "engine-scene",
                "active_camera must reference an enabled entity with an enabled engine.camera component",
            ).contract("ECSScene-v0", ECS_SCENE_CONTRACT).path("scene_settings.active_camera"));
        }
    }

    diagnostics
}

pub fn extract_renderer_input(
    scene: &Scene,
    frame_index: u64,
) -> Result<RenderFrameInput, Vec<Diagnostic>> {
    let diagnostics = validate_scene(scene);
    if diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.severity,
            DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
        )
    }) {
        return Err(diagnostics);
    }

    let mut input = RenderFrameInput::empty(frame_index);
    input.render_options.tone_mapping = scene.scene_settings.tone_mapping;
    input.stats_scope = Some(scene.name.clone());

    let Some(camera_entity) = active_camera_entity(scene) else {
        return Err(vec![Diagnostic::new(
            "SC0018",
            DiagnosticSeverity::Error,
            "engine-scene",
            "scene extraction requires at least one enabled active camera",
        )
        .contract("ECSScene-v0", ECS_SCENE_CONTRACT)]);
    };

    input.views.push(RenderView {
        view_id: 0,
        camera_entity: Some(camera_entity.persistent_id.clone()),
        viewport: Rect::FULL,
        viewport_rect_normalized: Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: scene.scene_settings.ambient,
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: scene.scene_settings.ambient,
        },
        stack_order: 0,
        frustum: None,
    });

    for entity in scene.entities.iter().filter(|entity| entity.enabled) {
        if let Some(renderable) = enabled_component(entity, "engine.renderable") {
            if bool_field(renderable, "visible").unwrap_or(true) {
                if let (Some(mesh), Some(material)) = (
                    asset_field(renderable, "mesh"),
                    asset_field(renderable, "material"),
                ) {
                    input.drawables.push(RenderableItem {
                        entity: Some(entity.persistent_id.clone()),
                        mesh,
                        material,
                        world_transform: IDENTITY_MAT4,
                        bounds: AxisAlignedBox::UNIT,
                        render_layer: string_field(renderable, "render_layer")
                            .unwrap_or_else(|| scene.scene_settings.default_render_layer.clone()),
                        cast_shadows: bool_field(renderable, "cast_shadows").unwrap_or(true),
                        sort_key: input.drawables.len() as u64,
                    });
                }
            }
        }

        if let Some(light) = enabled_component(entity, "engine.light") {
            input.lights.push(LightItem {
                entity: Some(entity.persistent_id.clone()),
                kind: light_kind_field(light).unwrap_or(LightKind::Directional),
                color: vec3_field(light, "color").unwrap_or([1.0, 1.0, 1.0]),
                intensity: f32_field(light, "intensity").unwrap_or(1.0),
                range: f32_field(light, "range").unwrap_or(10.0),
                position: vec3_field(light, "position").unwrap_or([0.0, 0.0, 0.0]),
                direction: vec3_field(light, "direction").unwrap_or([0.0, -1.0, 0.0]),
                spot_angles: None,
                shadow_mode: ShadowMode::Off,
            });
        }
    }

    Ok(input)
}

fn active_camera_entity(scene: &Scene) -> Option<&EntityRecord> {
    let active_camera = scene.scene_settings.active_camera.as_ref()?;
    scene.entities.iter().find(|entity| {
        entity.enabled
            && &entity.persistent_id == active_camera
            && entity
                .components
                .get("engine.camera")
                .map(|component| component.enabled)
                .unwrap_or(false)
    })
}

fn enabled_component<'a>(
    entity: &'a EntityRecord,
    component_type: &str,
) -> Option<&'a ComponentRecord> {
    entity
        .components
        .get(component_type)
        .filter(|component| component.enabled)
}

fn asset_field(component: &ComponentRecord, field: &str) -> Option<AssetId> {
    match component.fields.get(field) {
        Some(Value::Asset(asset)) => Some(asset.clone()),
        _ => None,
    }
}

fn string_field(component: &ComponentRecord, field: &str) -> Option<String> {
    match component.fields.get(field) {
        Some(Value::Str(value)) => Some(value.clone()),
        _ => None,
    }
}

fn bool_field(component: &ComponentRecord, field: &str) -> Option<bool> {
    match component.fields.get(field) {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn f32_field(component: &ComponentRecord, field: &str) -> Option<f32> {
    match component.fields.get(field) {
        Some(Value::Float32(value)) => Some(*value),
        Some(Value::Float64(value)) => Some(*value as f32),
        _ => None,
    }
}

fn vec3_field(component: &ComponentRecord, field: &str) -> Option<[f32; 3]> {
    match component.fields.get(field) {
        Some(Value::Vec3(value)) => Some(*value),
        Some(Value::Color(value)) => Some([value[0], value[1], value[2]]),
        _ => None,
    }
}

fn light_kind_field(component: &ComponentRecord) -> Option<LightKind> {
    match component.fields.get("kind") {
        Some(Value::Enum(kind)) if kind == "Point" => Some(LightKind::Point),
        Some(Value::Enum(kind)) if kind == "Spot" => Some(LightKind::Spot),
        Some(Value::Enum(kind)) if kind == "Directional" => Some(LightKind::Directional),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::{extract_renderer_input, sample_scene, validate_scene};

    #[test]
    fn sample_scene_validates_and_extracts() {
        let scene = sample_scene();
        assert!(validate_scene(&scene).is_empty());
        let input = extract_renderer_input(&scene, 7).expect("sample scene extracts");
        assert_eq!(input.frame_index, 7);
        assert_eq!(input.views.len(), 1);
        assert_eq!(input.drawables.len(), 1);
    }
}
