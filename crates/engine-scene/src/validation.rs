use engine_serialize::{Diagnostic, DiagnosticSeverity, Value};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use crate::components::{Bounds, Camera, Light, Name, Renderable, Transform};
use crate::scene::{Scene, SceneLoadDiagnostic, ECS_SCENE_CONTRACT};
use crate::{Component, ComponentRegistry, World};

/// Scene-only component metadata is retained in serialized authoring data but
/// is deliberately not materialized into the ECS [`World`].
///
/// The runtime follows the same rule before strict scene loading. Keeping the
/// identifier here prevents editor validation from treating script metadata as
/// an unknown ECS extension while still validating every actual component.
pub const SCENE_ONLY_COMPONENT_TYPES: &[&str] = &["engine.script"];

/// One concrete reason an authoring scene could not be committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneAuthoringFailure {
    SceneContract {
        code: String,
        message: String,
        path: Option<String>,
    },
    InvalidComponentFields {
        entity_id: String,
        component_type_id: String,
        reason: String,
    },
    SceneLoad(SceneLoadDiagnostic),
}

impl fmt::Display for SceneAuthoringFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SceneContract {
                code,
                message,
                path,
            } => {
                write!(formatter, "{code}: {message}")?;
                if let Some(path) = path {
                    write!(formatter, " at {path}")?;
                }
                Ok(())
            }
            Self::InvalidComponentFields {
                entity_id,
                component_type_id,
                reason,
            } => write!(
                formatter,
                "component `{component_type_id}` on entity `{entity_id}` is invalid: {reason}"
            ),
            Self::SceneLoad(diagnostic) => diagnostic.fmt(formatter),
        }
    }
}

/// Aggregated failure from the canonical authoring preflight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneAuthoringValidationError {
    pub failures: Vec<SceneAuthoringFailure>,
}

impl fmt::Display for SceneAuthoringValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, failure) in self.failures.iter().enumerate() {
            if index != 0 {
                formatter.write_str("; ")?;
            }
            failure.fmt(formatter)?;
        }
        Ok(())
    }
}

impl std::error::Error for SceneAuthoringValidationError {}

/// Validate an authoring snapshot through the same Scene -> World boundary
/// used by the runtime before the editor is allowed to commit a command.
///
/// With a component registry, every ECS extension is strictly deserialized.
/// Without one, core components are still strictly validated and materialized,
/// while extension records remain opaque so custom/plugin data is not rejected
/// merely because a model-only editor test did not install the runtime registry.
/// Scene-only script metadata is always retained in the source [`Scene`] and
/// excluded only from the temporary World preflight.
pub fn validate_scene_for_authoring(
    scene: &Scene,
    component_registry: Option<&ComponentRegistry>,
) -> Result<(), SceneAuthoringValidationError> {
    let mut failures = validate_scene(scene)
        .into_iter()
        .chain(scene.check_version())
        .filter(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        })
        .map(|diagnostic| SceneAuthoringFailure::SceneContract {
            code: diagnostic.code,
            message: diagnostic.message,
            path: diagnostic.path,
        })
        .collect::<Vec<_>>();

    for entity in &scene.entities {
        for (component_type_id, component) in &entity.components {
            failures.extend(validate_core_component_fields(
                &entity.persistent_id,
                component_type_id,
                &component.fields,
            ));
        }
    }

    if failures.is_empty() {
        let mut world_scene = scene.clone();
        for entity in &mut world_scene.entities {
            entity.components.retain(|component_type_id, _| {
                !SCENE_ONLY_COMPONENT_TYPES.contains(&component_type_id.as_str())
                    && (component_registry.is_some() || is_core_component(component_type_id))
            });
        }

        let registry = match component_registry {
            Some(registry) => Arc::new(registry.clone()),
            None => {
                let mut registry = ComponentRegistry::new();
                registry.register_core();
                Arc::new(registry)
            }
        };
        if let Err(error) = World::try_from_scene_with_registry(&world_scene, registry) {
            failures.extend(
                error
                    .diagnostics
                    .into_iter()
                    .map(SceneAuthoringFailure::SceneLoad),
            );
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(SceneAuthoringValidationError { failures })
    }
}

fn is_core_component(component_type_id: &str) -> bool {
    matches!(
        component_type_id,
        Name::TYPE_ID
            | Transform::TYPE_ID
            | Renderable::TYPE_ID
            | Camera::TYPE_ID
            | Light::TYPE_ID
            | Bounds::TYPE_ID
    )
}

fn validate_core_component_fields(
    entity_id: &str,
    component_type_id: &str,
    fields: &std::collections::BTreeMap<String, Value>,
) -> Vec<SceneAuthoringFailure> {
    if !is_core_component(component_type_id) {
        return Vec::new();
    }

    let mut failures = Vec::new();
    for (field_name, value) in fields {
        let valid = match component_type_id {
            Name::TYPE_ID => field_name == "name" && matches!(value, Value::Str(_)),
            Transform::TYPE_ID => match field_name.as_str() {
                "translation" | "scale" => matches!(value, Value::Vec3(values) if finite(values)),
                "rotation" => matches!(value, Value::Quat(values) if finite(values)),
                "parent" => matches!(value, Value::Entity(_)),
                _ => false,
            },
            Renderable::TYPE_ID => match field_name.as_str() {
                "mesh" | "material" => matches!(value, Value::Asset(_)),
                "visible" | "cast_shadows" => matches!(value, Value::Bool(_)),
                "render_layer" => matches!(value, Value::Str(_)),
                _ => false,
            },
            Camera::TYPE_ID => match field_name.as_str() {
                "projection" => {
                    matches!(value, Value::Enum(value) if value == "Perspective" || value == "Orthographic")
                }
                "near" | "far" | "fov_y" | "ortho_half_height" | "aperture" | "shutter_speed"
                | "iso" | "ev_compensation" => finite_number(value),
                "viewport_rect" => {
                    matches!(value, Value::List(values) if values.len() == 4 && values.iter().all(finite_number))
                }
                "render_layer_mask" | "clear_flags" | "msaa_samples" => {
                    matches!(value, Value::UInt(_))
                }
                "clear_color" => matches!(value, Value::Color(values) if finite(values)),
                "priority" => matches!(value, Value::Int(_)),
                "hdr_output" => matches!(value, Value::Bool(_)),
                _ => false,
            },
            Light::TYPE_ID => match field_name.as_str() {
                "kind" => {
                    matches!(value, Value::Enum(value) if matches!(value.as_str(), "Directional" | "Point" | "Spot"))
                }
                "color" | "direction" => matches!(value, Value::Vec3(values) if finite(values)),
                "intensity" | "range" => finite_number(value),
                "spot_angles" => {
                    matches!(value, Value::List(values) if values.len() == 2 && values.iter().all(finite_number))
                }
                "shadow_mode" => matches!(value, Value::UInt(_)),
                _ => false,
            },
            Bounds::TYPE_ID => match field_name.as_str() {
                "center" | "half_extents" => {
                    matches!(value, Value::Vec3(values) if finite(values))
                }
                _ => false,
            },
            _ => true,
        };
        if !valid {
            failures.push(SceneAuthoringFailure::InvalidComponentFields {
                entity_id: entity_id.to_string(),
                component_type_id: component_type_id.to_string(),
                reason: format!(
                    "field `{field_name}` has an unsupported name, type, or non-finite value"
                ),
            });
        }
    }

    if component_type_id == Renderable::TYPE_ID {
        for required in ["mesh", "material"] {
            if !fields.contains_key(required) {
                failures.push(SceneAuthoringFailure::InvalidComponentFields {
                    entity_id: entity_id.to_string(),
                    component_type_id: component_type_id.to_string(),
                    reason: format!("required field `{required}` is missing"),
                });
            }
        }
    }
    failures
}

fn finite<const N: usize>(values: &[f32; N]) -> bool {
    values.iter().all(|value| value.is_finite())
}

fn finite_number(value: &Value) -> bool {
    match value {
        Value::Float32(value) => value.is_finite(),
        Value::Float64(value) => value.is_finite(),
        _ => false,
    }
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

    let settings = &scene.scene_settings;
    if settings.default_render_layer.trim().is_empty() {
        diagnostics.push(
            Diagnostic::new(
                "SC0018",
                DiagnosticSeverity::Error,
                "engine-scene",
                "default_render_layer must not be empty",
            )
            .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
            .path("scene_settings.default_render_layer"),
        );
    }
    if !settings.fixed_timestep_seconds.is_finite()
        || settings.fixed_timestep_seconds <= 0.0
        || settings.fixed_timestep_seconds > 1.0
    {
        diagnostics.push(
            Diagnostic::new(
                "SC0019",
                DiagnosticSeverity::Error,
                "engine-scene",
                "fixed_timestep_seconds must be finite and in (0, 1]",
            )
            .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
            .path("scene_settings.fixed_timestep_seconds"),
        );
    }
    if settings.gravity.is_some_and(|gravity| !finite(&gravity)) {
        diagnostics.push(
            Diagnostic::new(
                "SC0020",
                DiagnosticSeverity::Error,
                "engine-scene",
                "gravity components must be finite",
            )
            .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
            .path("scene_settings.gravity"),
        );
    }
    if !finite(&settings.ambient) || settings.ambient.iter().any(|value| *value < 0.0) {
        diagnostics.push(
            Diagnostic::new(
                "SC0021",
                DiagnosticSeverity::Error,
                "engine-scene",
                "ambient values must be finite and non-negative",
            )
            .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
            .path("scene_settings.ambient"),
        );
    }
    if settings
        .environment_map
        .as_ref()
        .is_some_and(|asset| asset.id.trim().is_empty())
    {
        diagnostics.push(
            Diagnostic::new(
                "SC0022",
                DiagnosticSeverity::Error,
                "engine-scene",
                "environment_map asset ID must not be empty",
            )
            .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
            .path("scene_settings.environment_map"),
        );
    }
    let environment = engine_renderer::EnvironmentSettings {
        environment_map: settings.environment_map.clone(),
        intensity: settings.environment_intensity,
        rotation_radians: settings.environment_rotation_radians,
        reflection_probes: settings.reflection_probes.clone(),
    };
    if let Some(message) = engine_renderer::validate_environment_settings(&environment) {
        diagnostics.push(
            Diagnostic::new("SC0023", DiagnosticSeverity::Error, "engine-scene", message)
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .path("scene_settings.environment"),
        );
    }
    if let Some(message) = engine_renderer::validate_post_process_settings(&settings.post_process) {
        diagnostics.push(
            Diagnostic::new("SC0024", DiagnosticSeverity::Error, "engine-scene", message)
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .path("scene_settings.post_process"),
        );
    }
    diagnostics.extend(engine_renderer::validate_pass_graph_settings(
        &settings.pass_graph_config,
        settings.tone_mapping,
    ));

    diagnostics
}
