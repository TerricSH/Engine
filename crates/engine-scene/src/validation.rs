use engine_renderer::LightKind;
use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity, Value};
use std::collections::BTreeSet;

use crate::scene::{ECS_SCENE_CONTRACT, EntityRecord, ComponentRecord, Scene};

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

pub(crate) fn active_camera_entity(scene: &Scene) -> Option<&EntityRecord> {
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

pub(crate) fn enabled_component<'a>(
    entity: &'a EntityRecord,
    component_type: &str,
) -> Option<&'a ComponentRecord> {
    entity
        .components
        .get(component_type)
        .filter(|component| component.enabled)
}

pub(crate) fn asset_field(component: &ComponentRecord, field: &str) -> Option<AssetId> {
    match component.fields.get(field) {
        Some(Value::Asset(asset)) => Some(asset.clone()),
        _ => None,
    }
}

pub(crate) fn string_field(component: &ComponentRecord, field: &str) -> Option<String> {
    match component.fields.get(field) {
        Some(Value::Str(value)) => Some(value.clone()),
        _ => None,
    }
}

pub(crate) fn bool_field(component: &ComponentRecord, field: &str) -> Option<bool> {
    match component.fields.get(field) {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

pub(crate) fn f32_field(component: &ComponentRecord, field: &str) -> Option<f32> {
    match component.fields.get(field) {
        Some(Value::Float32(value)) => Some(*value),
        Some(Value::Float64(value)) => Some(*value as f32),
        _ => None,
    }
}

pub(crate) fn vec3_field(component: &ComponentRecord, field: &str) -> Option<[f32; 3]> {
    match component.fields.get(field) {
        Some(Value::Vec3(value)) => Some(*value),
        Some(Value::Color(value)) => Some([value[0], value[1], value[2]]),
        _ => None,
    }
}

pub(crate) fn light_kind_field(component: &ComponentRecord) -> Option<LightKind> {
    match component.fields.get("kind") {
        Some(Value::Enum(kind)) if kind == "Point" => Some(LightKind::Point),
        Some(Value::Enum(kind)) if kind == "Spot" => Some(LightKind::Spot),
        Some(Value::Enum(kind)) if kind == "Directional" => Some(LightKind::Directional),
        _ => None,
    }
}
