use std::collections::BTreeMap;

use engine_scene::{
    Component, ComponentExtension, ComponentMeta, ComponentRegistry, ComponentStorageDyn,
    ScriptAccess, SparseSet,
};
use engine_serialize::Value;

use crate::PlanetSceneTransitionConfig;

impl Component for PlanetSceneTransitionConfig {
    const TYPE_ID: &'static str = "engine.planet_scene_transition";
}

pub(crate) fn register_planet_scene_transition(registry: &mut ComponentRegistry) {
    let registered = registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: PlanetSceneTransitionConfig::TYPE_ID,
                display_name: "Planet Scene Transition",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: ScriptAccess::None,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<PlanetSceneTransitionConfig>::new())
            },
            serialize: Some(serialize_transition),
            deserialize: Some(deserialize_transition),
        })
        .is_ok();
    if registered {
        let _ = registry.register_fields_validator(
            PlanetSceneTransitionConfig::TYPE_ID,
            validate_transition_fields,
        );
    }
}

fn serialize_transition(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let transition = component
        .downcast_ref::<PlanetSceneTransitionConfig>()
        .expect("PlanetSceneTransitionConfig expected");
    serialize_planet_scene_transition_fields(transition)
}

/// Serialize an authored transition policy through the canonical scene schema.
///
/// Editor and host tooling use this function instead of maintaining a second
/// copy of the component registry's field layout.
pub fn serialize_planet_scene_transition_fields(
    transition: &PlanetSceneTransitionConfig,
) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("enabled".into(), Value::Bool(transition.enabled)),
        (
            "terrain_volume_id".into(),
            Value::Str(transition.terrain_volume_id.clone()),
        ),
        (
            "orbit_scene_id".into(),
            Value::Str(transition.orbit_scene_id.clone()),
        ),
        (
            "surface_scene_id".into(),
            Value::Str(transition.surface_scene_id.clone()),
        ),
        (
            "enter_surface_altitude".into(),
            Value::Float64(transition.enter_surface_altitude),
        ),
        (
            "exit_surface_altitude".into(),
            Value::Float64(transition.exit_surface_altitude),
        ),
        (
            "minimum_dwell_seconds".into(),
            Value::Float64(transition.minimum_dwell_seconds),
        ),
    ])
}

fn deserialize_transition(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let mut transition = PlanetSceneTransitionConfig::default();
    if let Some(Value::Bool(value)) = fields.get("enabled") {
        transition.enabled = *value;
    }
    for (name, target) in [
        ("terrain_volume_id", &mut transition.terrain_volume_id),
        ("orbit_scene_id", &mut transition.orbit_scene_id),
        ("surface_scene_id", &mut transition.surface_scene_id),
    ] {
        if let Some(Value::Str(value)) = fields.get(name) {
            *target = value.clone();
        }
    }
    for (name, target) in [
        (
            "enter_surface_altitude",
            &mut transition.enter_surface_altitude,
        ),
        (
            "exit_surface_altitude",
            &mut transition.exit_surface_altitude,
        ),
        (
            "minimum_dwell_seconds",
            &mut transition.minimum_dwell_seconds,
        ),
    ] {
        if let Some(Value::Float64(value)) = fields.get(name) {
            *target = *value;
        }
    }
    Box::new(transition)
}

fn validate_transition_fields(fields: &BTreeMap<String, Value>) -> Result<(), String> {
    let transition = deserialize_transition(fields)
        .downcast::<PlanetSceneTransitionConfig>()
        .map_err(|_| "planet transition deserializer returned an incompatible value".to_string())?;
    let normalized = serialize_transition(transition.as_ref());
    let rejected = fields
        .iter()
        .filter_map(|(name, value)| (normalized.get(name) != Some(value)).then_some(name.clone()))
        .collect::<Vec<_>>();
    if !rejected.is_empty() {
        return Err(format!(
            "unknown or incorrectly typed fields: {}",
            rejected.join(", ")
        ));
    }
    transition.validate().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use engine_scene::{Component, ComponentRegistry};

    use super::*;

    #[test]
    fn registered_transition_round_trips_and_is_engine_owned() {
        let mut registry = ComponentRegistry::new();
        crate::register_terrain_extensions(&mut registry);
        let extension = registry
            .get(PlanetSceneTransitionConfig::TYPE_ID)
            .expect("planet transition component registered");
        assert_eq!(extension.meta.script_access, ScriptAccess::None);

        let transition = PlanetSceneTransitionConfig {
            enabled: true,
            terrain_volume_id: "planet-a".into(),
            orbit_scene_id: "system".into(),
            surface_scene_id: "planet-a-surface".into(),
            enter_surface_altitude: 1_500.0,
            exit_surface_altitude: 2_250.0,
            minimum_dwell_seconds: 0.75,
        };
        let fields = (extension.serialize.expect("serialize hook"))(&transition);
        registry
            .validate_fields(PlanetSceneTransitionConfig::TYPE_ID, &fields)
            .expect("canonical fields validate");
        let restored = (extension.deserialize.expect("deserialize hook"))(&fields)
            .downcast::<PlanetSceneTransitionConfig>()
            .expect("typed component");
        assert_eq!(*restored, transition);
    }

    #[test]
    fn validator_rejects_invalid_hysteresis_and_lossy_fields() {
        let mut registry = ComponentRegistry::new();
        crate::register_terrain_extensions(&mut registry);
        let extension = registry
            .get(PlanetSceneTransitionConfig::TYPE_ID)
            .expect("planet transition component registered");
        let mut fields =
            (extension.serialize.expect("serialize hook"))(&PlanetSceneTransitionConfig {
                enabled: true,
                ..PlanetSceneTransitionConfig::default()
            });
        fields.insert("exit_surface_altitude".into(), Value::Float64(100.0));
        assert!(registry
            .validate_fields(PlanetSceneTransitionConfig::TYPE_ID, &fields)
            .is_err());

        fields.insert("exit_surface_altitude".into(), Value::Str("3000".into()));
        assert!(registry
            .validate_fields(PlanetSceneTransitionConfig::TYPE_ID, &fields)
            .is_err());
    }
}
