//! Typed script component bridge.
//!
//! The gameplay script API exposes a curated, game-agnostic set of built-in
//! components beyond Transform. A component type becomes script-accessible
//! only when **both** of the following hold:
//!
//! 1. Its stable type key appears in [`script_component_types`], the explicit
//!    curated allow-list. Feature-gated component families (audio, physics)
//!    are listed only when their feature is compiled in.
//! 2. The active world's [`engine_scene::registry::ComponentRegistry`]
//!    carries the type with serialize/deserialize hooks — the same hooks the
//!    scene loader uses for `.scene.ron` files, so scripts and scene files
//!    share one field layout per component.
//!
//! Reads snapshot a component through its serialize hook and convert the
//! scene field map into wire values. Writes merge script-provided fields over
//! the current snapshot (or over the authored defaults when the entity does
//! not carry the component yet) and re-validate the result through a
//! deserialize → serialize round-trip before committing, so unknown field
//! names, mismatched value types, and unsupported enum cases are rejected
//! without partially applying the command.

use std::collections::BTreeMap;
use std::sync::Arc;

use engine_scene::World;
use engine_script::GameplayComponentValue;

/// The curated, game-agnostic component type keys exposed to gameplay
/// scripts.
///
/// Transform is intentionally absent (it has a dedicated, higher-fidelity
/// script path) and retained UI canvases are absent (they are driven through
/// the managed `UICanvas` handles). New components opt in by registering
/// scene serde hooks and adding their type key here.
pub(crate) fn script_component_types() -> &'static [&'static str] {
    &[
        "engine.camera",
        "engine.light",
        #[cfg(feature = "runtime-subsystems")]
        "engine.audio_source",
        #[cfg(feature = "gameplay")]
        "engine.physics.rigid_body",
        #[cfg(feature = "gameplay")]
        "engine.physics.collider",
        #[cfg(feature = "gameplay")]
        "engine.gravity_source",
    ]
}

/// Whether `type_id` is eligible for script component access in this build.
pub(crate) fn is_script_component_type(type_id: &str) -> bool {
    script_component_types().contains(&type_id)
}

/// Outcome of reading one component for a script query.
#[derive(Debug)]
pub(crate) enum ScriptComponentRead {
    /// Field snapshot keyed by field name, converted to wire values.
    Snapshot(BTreeMap<String, GameplayComponentValue>),
    /// The entity does not exist or does not carry the component.
    Missing,
    /// The type key is not script-accessible in this build.
    Unsupported,
}

/// Snapshot one script-accessible component through its registered scene
/// serde hook.
pub(crate) fn read_script_component(
    world: &World,
    entity_id: &str,
    component_type: &str,
) -> ScriptComponentRead {
    if !is_script_component_type(component_type) {
        return ScriptComponentRead::Unsupported;
    }
    let Some(extension) = world
        .component_registry()
        .and_then(|registry| registry.get(component_type))
    else {
        return ScriptComponentRead::Unsupported;
    };
    let Some(serialize) = extension.serialize else {
        return ScriptComponentRead::Unsupported;
    };
    let Some(entity) = world.entity_by_persistent_id(entity_id) else {
        return ScriptComponentRead::Missing;
    };
    let Some(component) = world.get_any(entity, component_type) else {
        return ScriptComponentRead::Missing;
    };
    let fields = serialize(component)
        .into_iter()
        .filter_map(|(name, value)| {
            GameplayComponentValue::from_scene_value(&value).map(|value| (name, value))
        })
        .collect();
    ScriptComponentRead::Snapshot(fields)
}

/// Why a script component write was rejected.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScriptComponentWriteError {
    /// The type key is not script-accessible in this build.
    Unsupported,
    /// The target persistent entity does not exist.
    UnknownEntity,
    /// One or more provided fields were not accepted by the component's serde
    /// round-trip (unknown field name, mismatched value type, or unsupported
    /// enum case). `known` lists the fields the component currently exposes.
    PayloadRejected {
        rejected: Vec<String>,
        known: Vec<String>,
    },
    /// The validated component could not be committed to storage.
    ApplyFailed,
}

/// Merge script-provided fields into one script-accessible component.
///
/// The write is a validated read-modify-write through the component's
/// registered scene serde hooks: the merged field map is deserialized and
/// re-serialized, and every provided field must survive that round-trip
/// unchanged before anything is committed. On any rejection the world is left
/// untouched.
pub(crate) fn apply_script_component_write(
    world: &mut World,
    entity_id: &str,
    component_type: &str,
    fields: &BTreeMap<String, GameplayComponentValue>,
) -> Result<(), ScriptComponentWriteError> {
    use ScriptComponentWriteError as Error;

    if !is_script_component_type(component_type) {
        return Err(Error::Unsupported);
    }
    let Some(registry) = world.component_registry().map(Arc::clone) else {
        return Err(Error::Unsupported);
    };
    let Some(extension) = registry.get(component_type) else {
        return Err(Error::Unsupported);
    };
    let (Some(serialize), Some(deserialize)) = (extension.serialize, extension.deserialize) else {
        return Err(Error::Unsupported);
    };
    let Some(entity) = world.entity_by_persistent_id(entity_id) else {
        return Err(Error::UnknownEntity);
    };

    // Fields not provided by the script keep their current values, or the
    // component's authored defaults when the entity does not carry the
    // component yet (the deserialize hooks tolerate missing fields the same
    // way the scene loader does).
    let base = world
        .get_any(entity, component_type)
        .map(serialize)
        .unwrap_or_else(|| serialize(deserialize(&BTreeMap::new()).as_ref()));
    let mut merged = base.clone();
    for (name, value) in fields {
        merged.insert(name.clone(), value.to_scene_value());
    }

    let candidate = deserialize(&merged);
    let reserialized = serialize(candidate.as_ref());
    let rejected = fields
        .keys()
        .filter(|name| reserialized.get(*name) != merged.get(*name))
        .cloned()
        .collect::<Vec<_>>();
    if !rejected.is_empty() {
        return Err(Error::PayloadRejected {
            rejected,
            known: base.keys().cloned().collect(),
        });
    }

    if world.set_any(entity, component_type, candidate) {
        Ok(())
    } else {
        Err(Error::ApplyFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::components::{Camera, Light};
    use engine_scene::{ComponentRegistry, World};

    fn world_with_core_registry() -> World {
        let mut registry = ComponentRegistry::new();
        registry.register_core();
        let mut world = World::new();
        world.set_component_registry(registry);
        world
    }

    #[test]
    fn curated_component_set_is_explicit_and_feature_gated() {
        let types = script_component_types();
        assert!(types.contains(&"engine.camera"));
        assert!(types.contains(&"engine.light"));
        #[cfg(feature = "runtime-subsystems")]
        assert!(types.contains(&"engine.audio_source"));
        #[cfg(feature = "gameplay")]
        {
            assert!(types.contains(&"engine.physics.rigid_body"));
            assert!(types.contains(&"engine.physics.collider"));
            assert!(types.contains(&"engine.gravity_source"));
        }
        // Transform and retained UI stay on their dedicated paths.
        assert!(!types.contains(&"engine.transform"));
        assert!(!types.contains(&"engine.canvas"));
        assert!(!types.contains(&"engine.renderable"));
    }

    #[test]
    fn read_snapshots_registered_component_fields() {
        let mut world = world_with_core_registry();
        let entity = world.create_persistent_entity("camera-main").unwrap();
        world.add_component(entity, Camera::default());

        match read_script_component(&world, "camera-main", "engine.camera") {
            ScriptComponentRead::Snapshot(fields) => {
                assert_eq!(
                    fields.get("projection"),
                    Some(&GameplayComponentValue::Enum("Perspective".into()))
                );
                assert_eq!(
                    fields.get("near"),
                    Some(&GameplayComponentValue::Float(0.1))
                );
                // Optional fields that are absent stay absent.
                assert!(!fields.contains_key("viewport_rect"));
            }
            other => panic!("expected snapshot, got {other:?}"),
        }
    }

    #[test]
    fn read_reports_missing_entities_and_components() {
        let mut world = world_with_core_registry();
        let entity = world.create_persistent_entity("cube-01").unwrap();
        world.add_component(
            entity,
            Light {
                kind: engine_scene::components::LightKind::Point,
                color: [1.0, 1.0, 1.0],
                intensity: 2.0,
                range: 12.0,
                spot_angles: None,
                shadow_mode: 0,
                direction: [0.0, -1.0, 0.0],
            },
        );

        assert!(matches!(
            read_script_component(&world, "missing", "engine.camera"),
            ScriptComponentRead::Missing
        ));
        assert!(matches!(
            read_script_component(&world, "cube-01", "engine.camera"),
            ScriptComponentRead::Missing
        ));
        assert!(matches!(
            read_script_component(&world, "cube-01", "engine.canvas"),
            ScriptComponentRead::Unsupported
        ));
        assert!(matches!(
            read_script_component(&world, "cube-01", "game.custom"),
            ScriptComponentRead::Unsupported
        ));
    }

    #[test]
    fn write_merges_fields_over_the_current_snapshot() {
        let mut world = world_with_core_registry();
        let entity = world.create_persistent_entity("light-01").unwrap();
        world.add_component(
            entity,
            Light {
                kind: engine_scene::components::LightKind::Point,
                color: [1.0, 1.0, 1.0],
                intensity: 2.0,
                range: 12.0,
                spot_angles: None,
                shadow_mode: 0,
                direction: [0.0, -1.0, 0.0],
            },
        );

        apply_script_component_write(
            &mut world,
            "light-01",
            "engine.light",
            &BTreeMap::from([("intensity".to_string(), GameplayComponentValue::Float(7.5))]),
        )
        .expect("single-field write applies");

        let light = world.get::<Light>(entity).expect("light still present");
        assert_eq!(light.intensity, 7.5);
        // Untouched fields keep their previous values.
        assert_eq!(light.range, 12.0);
        assert_eq!(light.kind, engine_scene::components::LightKind::Point);
    }

    #[test]
    fn write_inserts_defaults_for_entities_without_the_component() {
        let mut world = world_with_core_registry();
        let entity = world.create_persistent_entity("camera-main").unwrap();

        apply_script_component_write(
            &mut world,
            "camera-main",
            "engine.camera",
            &BTreeMap::from([("fov_y".to_string(), GameplayComponentValue::Float(1.2))]),
        )
        .expect("write installs the component with authored defaults");

        let camera = world.get::<Camera>(entity).expect("camera installed");
        assert_eq!(camera.fov_y, 1.2);
        assert_eq!(camera.near, Camera::default().near);
    }

    #[test]
    fn write_rejects_unknown_fields_type_mismatches_and_enum_cases() {
        let mut world = world_with_core_registry();
        let entity = world.create_persistent_entity("light-01").unwrap();
        world.add_component(
            entity,
            Light {
                kind: engine_scene::components::LightKind::Directional,
                color: [1.0, 1.0, 1.0],
                intensity: 1.0,
                range: 10.0,
                spot_angles: None,
                shadow_mode: 0,
                direction: [0.0, -1.0, 0.0],
            },
        );

        let unknown_field = apply_script_component_write(
            &mut world,
            "light-01",
            "engine.light",
            &BTreeMap::from([("intensty".to_string(), GameplayComponentValue::Float(9.0))]),
        )
        .expect_err("unknown field names are rejected");
        match unknown_field {
            ScriptComponentWriteError::PayloadRejected { rejected, known } => {
                assert_eq!(rejected, vec!["intensty".to_string()]);
                assert!(known.contains(&"intensity".to_string()));
            }
            other => panic!("expected payload rejection, got {other:?}"),
        }

        let type_mismatch = apply_script_component_write(
            &mut world,
            "light-01",
            "engine.light",
            &BTreeMap::from([(
                "intensity".to_string(),
                GameplayComponentValue::Str("bright".to_string()),
            )]),
        )
        .expect_err("mismatched value types are rejected");
        assert!(matches!(
            type_mismatch,
            ScriptComponentWriteError::PayloadRejected { .. }
        ));

        let enum_case = apply_script_component_write(
            &mut world,
            "light-01",
            "engine.light",
            &BTreeMap::from([(
                "kind".to_string(),
                GameplayComponentValue::Enum("Neon".to_string()),
            )]),
        )
        .expect_err("unsupported enum cases are rejected");
        assert!(matches!(
            enum_case,
            ScriptComponentWriteError::PayloadRejected { .. }
        ));

        // Rejected writes leave the component untouched.
        let light = world.get::<Light>(entity).expect("light still present");
        assert_eq!(light.intensity, 1.0);
        assert_eq!(light.kind, engine_scene::components::LightKind::Directional);
    }

    #[test]
    fn write_rejects_unknown_entities_and_unsupported_types() {
        let mut world = world_with_core_registry();
        world.create_persistent_entity("cube-01").unwrap();

        assert_eq!(
            apply_script_component_write(&mut world, "missing", "engine.camera", &BTreeMap::new(),),
            Err(ScriptComponentWriteError::UnknownEntity)
        );
        assert_eq!(
            apply_script_component_write(
                &mut world,
                "cube-01",
                "engine.transform",
                &BTreeMap::new(),
            ),
            Err(ScriptComponentWriteError::Unsupported)
        );
    }
}
