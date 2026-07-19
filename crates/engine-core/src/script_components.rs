//! Typed script component bridge.
//!
//! The gameplay script API exposes built-in components beyond Transform
//! through the generic `Components` bridge. Access is **registry-driven**:
//! there is no hardcoded allow-list. A component type becomes accessible when
//! **all** of the following hold for its entry in the active world's
//! [`engine_scene::registry::ComponentRegistry`]:
//!
//! 1. `ComponentMeta::script_access` is [`engine_scene::registry::ScriptAccess::ReadOnly`] or
//!    [`engine_scene::registry::ScriptAccess::ReadWrite`]. `ScriptAccess::None` opts out entirely and
//!    [`engine_scene::registry::ScriptAccess::DedicatedApi`] routes the component through its own
//!    higher-fidelity script path (Transform commands, retained UI canvas
//!    handles) instead of this bridge.
//! 2. The entry carries both scene serde hooks — the same hooks the scene
//!    loader uses for `.scene.ron` files, so scripts and scene files share
//!    one field layout per component.
//!
//! `ReadOnly` components answer queries but reject writes with a distinct
//! `ReadOnly` outcome (surfaced as `SCRIPT_COMPONENT_READ_ONLY`); anything
//! else that is unreachable is reported as unsupported (surfaced as
//! `SCRIPT_COMPONENT_UNKNOWN`), keeping unknown-type and known-but-forbidden
//! diagnostics distinct.
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

/// How the generic `Components` bridge may treat one component type key,
/// resolved from the active world's component registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptComponentResolution {
    /// Queries and writes are both allowed.
    ReadWrite,
    /// Queries are allowed; writes are rejected as read-only.
    ReadOnly,
    /// The type key is not script-accessible through this bridge: it is
    /// unregistered, opted out (`ScriptAccess::None`), routed through a
    /// dedicated API (`ScriptAccess::DedicatedApi`), or missing one of the
    /// required scene serde hooks.
    Unsupported,
}

/// Resolve the bridge access for `component_type` from `world`'s component
/// registry. This is the single source of truth for script component access;
/// adding a component to the bridge is a registry change, never an
/// engine-core code change.
pub(crate) fn resolve_script_component(
    world: &World,
    component_type: &str,
) -> ScriptComponentResolution {
    let Some(extension) = world
        .component_registry()
        .and_then(|registry| registry.get(component_type))
    else {
        return ScriptComponentResolution::Unsupported;
    };
    let access = extension.meta.script_access;
    if !access.is_queryable() {
        return ScriptComponentResolution::Unsupported;
    }
    if extension.serialize.is_none() || extension.deserialize.is_none() {
        return ScriptComponentResolution::Unsupported;
    }
    if access.is_writable() {
        ScriptComponentResolution::ReadWrite
    } else {
        ScriptComponentResolution::ReadOnly
    }
}

/// The sorted type keys scripts may currently query through the bridge, used
/// by `SCRIPT_COMPONENT_UNKNOWN` diagnostics.
pub(crate) fn supported_script_component_types(world: &World) -> Vec<&'static str> {
    let mut types = world
        .component_registry()
        .map(|registry| {
            registry
                .iter()
                .filter(|extension| {
                    extension.meta.script_access.is_queryable()
                        && extension.serialize.is_some()
                        && extension.deserialize.is_some()
                })
                .map(|extension| extension.meta.type_id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    types.sort_unstable();
    types
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
    if resolve_script_component(world, component_type) == ScriptComponentResolution::Unsupported {
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
    /// The component is script-queryable but not script-writable
    /// ([`engine_scene::registry::ScriptAccess::ReadOnly`]).
    ReadOnly,
    /// The target persistent entity does not exist.
    UnknownEntity,
    /// One or more provided fields were not accepted by the component's serde
    /// round-trip (unknown field name, mismatched value type, or unsupported
    /// enum case). `known` lists the fields the component currently exposes.
    PayloadRejected {
        rejected: Vec<String>,
        known: Vec<String>,
    },
    /// The field types were accepted, but the component's semantic validator
    /// rejected the resulting parameter set.
    ValidationFailed { message: String },
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

    match resolve_script_component(world, component_type) {
        ScriptComponentResolution::ReadWrite => {}
        ScriptComponentResolution::ReadOnly => return Err(Error::ReadOnly),
        ScriptComponentResolution::Unsupported => return Err(Error::Unsupported),
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

    registry
        .validate_fields(component_type, &merged)
        .map_err(|message| Error::ValidationFailed { message })?;

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
    use engine_scene::registry::ScriptAccess;
    use engine_scene::{ComponentRegistry, ComponentStorageDyn, World};

    fn world_with_core_registry() -> World {
        let mut registry = ComponentRegistry::new();
        registry.register_core();
        let mut world = World::new();
        world.set_component_registry(registry);
        world
    }

    struct QueryOnlyComponent;

    impl engine_scene::Component for QueryOnlyComponent {
        const TYPE_ID: &'static str = "test.query_only";
    }

    struct HooklessComponent;

    impl engine_scene::Component for HooklessComponent {
        const TYPE_ID: &'static str = "test.hookless";
    }

    struct DedicatedComponent;

    impl engine_scene::Component for DedicatedComponent {
        const TYPE_ID: &'static str = "test.dedicated";
    }

    fn serialize_empty(
        _component: &dyn std::any::Any,
    ) -> BTreeMap<String, engine_serialize::Value> {
        BTreeMap::new()
    }

    fn deserialize_query_only(
        _fields: &BTreeMap<String, engine_serialize::Value>,
    ) -> Box<dyn std::any::Any> {
        Box::new(QueryOnlyComponent)
    }

    fn deserialize_dedicated(
        _fields: &BTreeMap<String, engine_serialize::Value>,
    ) -> Box<dyn std::any::Any> {
        Box::new(DedicatedComponent)
    }

    fn test_extension<T: engine_scene::Component + 'static>(
        display_name: &'static str,
        script_access: ScriptAccess,
        serialize: Option<engine_scene::registry::SerializeFn>,
        deserialize: Option<engine_scene::registry::DeserializeFn>,
    ) -> engine_scene::ComponentExtension {
        engine_scene::ComponentExtension {
            meta: engine_scene::ComponentMeta {
                type_id: T::TYPE_ID,
                display_name,
                schema_version: (0, 1, 0),
                has_editor: false,
                script_access,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(engine_scene::SparseSet::<T>::new())
            },
            serialize,
            deserialize,
        }
    }

    fn world_with_access_levels() -> World {
        let mut registry = ComponentRegistry::new();
        registry.register_core();
        registry
            .register(test_extension::<QueryOnlyComponent>(
                "Query Only",
                ScriptAccess::ReadOnly,
                Some(serialize_empty),
                Some(deserialize_query_only),
            ))
            .expect("register read-only component");
        registry
            .register(test_extension::<HooklessComponent>(
                "Hookless",
                ScriptAccess::ReadWrite,
                None,
                None,
            ))
            .expect("register hookless component");
        registry
            .register(test_extension::<DedicatedComponent>(
                "Dedicated",
                ScriptAccess::DedicatedApi,
                Some(serialize_empty),
                Some(deserialize_dedicated),
            ))
            .expect("register dedicated component");
        let mut world = World::new();
        world.set_component_registry(registry);
        world
    }

    #[test]
    fn resolution_is_registry_driven_for_every_access_level() {
        let world = world_with_access_levels();
        use ScriptComponentResolution as Resolution;

        // ReadWrite + both hooks: queryable and writable.
        assert_eq!(
            resolve_script_component(&world, "engine.camera"),
            Resolution::ReadWrite
        );
        assert_eq!(
            resolve_script_component(&world, "engine.light"),
            Resolution::ReadWrite
        );
        // ReadOnly + both hooks: queryable, writes rejected.
        assert_eq!(
            resolve_script_component(&world, "test.query_only"),
            Resolution::ReadOnly
        );
        // ReadWrite without serde hooks is not bridge-accessible.
        assert_eq!(
            resolve_script_component(&world, "test.hookless"),
            Resolution::Unsupported
        );
        // Dedicated APIs never enter the generic bridge.
        assert_eq!(
            resolve_script_component(&world, "test.dedicated"),
            Resolution::Unsupported
        );
        assert_eq!(
            resolve_script_component(&world, "engine.transform"),
            Resolution::Unsupported
        );
        // Opted out and unknown keys are unsupported.
        assert_eq!(
            resolve_script_component(&world, "engine.name"),
            Resolution::Unsupported
        );
        assert_eq!(
            resolve_script_component(&world, "game.custom"),
            Resolution::Unsupported
        );
    }

    #[test]
    fn supported_types_track_the_registry() {
        let world = world_with_access_levels();
        let supported = supported_script_component_types(&world);
        assert!(supported.contains(&"engine.camera"));
        assert!(supported.contains(&"engine.light"));
        assert!(supported.contains(&"test.query_only"));
        assert!(!supported.contains(&"engine.transform"));
        assert!(!supported.contains(&"test.dedicated"));
        assert!(!supported.contains(&"test.hookless"));
        // The list is sorted for stable diagnostics.
        let mut sorted = supported.clone();
        sorted.sort_unstable();
        assert_eq!(supported, sorted);
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
    fn read_allows_read_only_components() {
        let mut world = world_with_access_levels();
        let entity = world.create_persistent_entity("probe-01").unwrap();
        world.add_component(entity, QueryOnlyComponent);

        assert!(matches!(
            read_script_component(&world, "probe-01", "test.query_only"),
            ScriptComponentRead::Snapshot(_)
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
    fn write_rejects_read_only_components_without_touching_the_world() {
        let mut world = world_with_access_levels();
        let entity = world.create_persistent_entity("probe-01").unwrap();
        world.add_component(entity, QueryOnlyComponent);

        assert_eq!(
            apply_script_component_write(
                &mut world,
                "probe-01",
                "test.query_only",
                &BTreeMap::from([("anything".to_string(), GameplayComponentValue::Bool(true))]),
            ),
            Err(ScriptComponentWriteError::ReadOnly)
        );
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
