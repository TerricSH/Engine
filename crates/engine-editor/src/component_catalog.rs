use std::collections::BTreeMap;

use engine_scene::{ComponentRecord, EntityRecord};
use engine_serialize::{AssetId, SchemaVersion, Value};

use crate::EditorError;

pub const TRANSFORM_COMPONENT: &str = "engine.transform";
pub const RENDERABLE_COMPONENT: &str = "engine.renderable";
pub const CAMERA_COMPONENT: &str = "engine.camera";
pub const LIGHT_COMPONENT: &str = "engine.light";
pub const BOUNDS_COMPONENT: &str = "engine.bounds";
pub const RIGID_BODY_COMPONENT: &str = "engine.physics.rigid_body";
pub const COLLIDER_COMPONENT: &str = "engine.physics.collider";
pub const PHYSICS_MATERIAL_COMPONENT: &str = "engine.physics.physics_material";
pub const GRAVITY_SOURCE_COMPONENT: &str = "engine.gravity_source";
pub const CHARACTER_CONTROLLER_COMPONENT: &str = "engine.character_controller";
pub const AUDIO_LISTENER_COMPONENT: &str = "engine.audio_listener";
pub const UI_CANVAS_COMPONENT: &str = "engine.canvas";
pub const AUDIO_SOURCE_COMPONENT: &str = "engine.audio_source";
pub const IK_TARGET_COMPONENT: &str = "engine.ik_target";
pub const NAV_AGENT_COMPONENT: &str = "engine.nav_agent";
pub const TERRAIN_VOLUME_COMPONENT: &str = "engine.terrain_volume";

#[derive(Clone, Copy)]
pub struct ComponentDescriptor {
    pub type_id: &'static str,
    pub display_name: &'static str,
    pub category: &'static str,
    pub removable: bool,
    pub required_components: &'static [&'static str],
    factory: fn(&EntityRecord) -> Result<ComponentRecord, EditorError>,
}

impl ComponentDescriptor {
    pub fn create_default(self, entity: &EntityRecord) -> Result<ComponentRecord, EditorError> {
        (self.factory)(entity)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EntityTemplate {
    pub id: &'static str,
    pub display_name: &'static str,
    pub category: &'static str,
    pub component_types: &'static [&'static str],
    configure: fn(&mut EntityRecord),
}

pub struct ComponentCatalog;

impl ComponentCatalog {
    pub fn descriptors() -> &'static [ComponentDescriptor] {
        &COMPONENT_DESCRIPTORS
    }

    pub fn descriptor(type_id: &str) -> Option<ComponentDescriptor> {
        COMPONENT_DESCRIPTORS
            .iter()
            .copied()
            .find(|descriptor| descriptor.type_id == type_id)
    }

    pub fn templates() -> &'static [EntityTemplate] {
        &ENTITY_TEMPLATES
    }

    pub fn template(id: &str) -> Option<EntityTemplate> {
        ENTITY_TEMPLATES
            .iter()
            .copied()
            .find(|template| template.id == id)
    }

    pub fn create_component(
        type_id: &str,
        entity: &EntityRecord,
    ) -> Result<ComponentRecord, EditorError> {
        Self::descriptor(type_id)
            .ok_or_else(|| EditorError::ComponentNotFound(type_id.to_string()))?
            .create_default(entity)
    }

    /// Instantiate the selected template beneath `parent`.
    ///
    /// Passing `None` creates a root object; passing a persistent entity ID
    /// creates a child. The same canonical template is used in both cases so
    /// the catalog does not need misleading "root" and "child" aliases.
    pub fn instantiate_template(
        template_id: &str,
        persistent_id: String,
        parent: Option<String>,
    ) -> Result<EntityRecord, EditorError> {
        let template = Self::template(template_id).ok_or_else(|| {
            EditorError::InitFailed(format!("unknown entity template '{template_id}'"))
        })?;
        let mut entity = EntityRecord {
            persistent_id,
            parent,
            name: Some(template.display_name.to_string()),
            enabled: true,
            components: BTreeMap::new(),
        };
        for type_id in template.component_types {
            let component = Self::create_component(type_id, &entity)?;
            entity.components.insert((*type_id).to_string(), component);
        }
        (template.configure)(&mut entity);
        Ok(entity)
    }
}

fn record(fields: BTreeMap<String, Value>) -> ComponentRecord {
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

fn transform(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    Ok(record(BTreeMap::from([
        ("translation".into(), Value::Vec3([0.0, 0.0, 0.0])),
        ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
        ("scale".into(), Value::Vec3([1.0, 1.0, 1.0])),
    ])))
}

fn renderable(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    // `mesh-cube` and `mat-default` are the only renderer-owned built-ins.
    // Other primitive names would create dangling asset references.
    Ok(record(BTreeMap::from([
        ("mesh".into(), Value::Asset(AssetId::new("mesh-cube"))),
        ("material".into(), Value::Asset(AssetId::new("mat-default"))),
        ("visible".into(), Value::Bool(true)),
        ("cast_shadows".into(), Value::Bool(true)),
        ("render_layer".into(), Value::Str("Default".into())),
    ])))
}

fn camera(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    Ok(record(BTreeMap::from([
        ("projection".into(), Value::Enum("Perspective".into())),
        ("near".into(), Value::Float32(0.1)),
        ("far".into(), Value::Float32(1000.0)),
        ("fov_y".into(), Value::Float32(std::f32::consts::FRAC_PI_4)),
        ("ortho_half_height".into(), Value::Float32(5.0)),
        ("render_layer_mask".into(), Value::UInt(u32::MAX as u64)),
        ("clear_flags".into(), Value::UInt(3)),
        ("clear_color".into(), Value::Color([0.02, 0.02, 0.06, 1.0])),
        ("priority".into(), Value::Int(0)),
        ("msaa_samples".into(), Value::UInt(1)),
        ("hdr_output".into(), Value::Bool(false)),
        ("aperture".into(), Value::Float32(16.0)),
        ("shutter_speed".into(), Value::Float32(1.0 / 60.0)),
        ("iso".into(), Value::Float32(100.0)),
        ("ev_compensation".into(), Value::Float32(0.0)),
    ])))
}

fn light(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    Ok(record(BTreeMap::from([
        ("kind".into(), Value::Enum("Directional".into())),
        ("color".into(), Value::Vec3([1.0, 1.0, 1.0])),
        ("intensity".into(), Value::Float32(1.0)),
        ("range".into(), Value::Float32(10.0)),
        ("shadow_mode".into(), Value::UInt(0)),
        ("direction".into(), Value::Vec3([0.0, -1.0, 0.0])),
    ])))
}

fn bounds(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    Ok(record(BTreeMap::from([
        ("center".into(), Value::Vec3([0.0, 0.0, 0.0])),
        ("half_extents".into(), Value::Vec3([0.5, 0.5, 0.5])),
    ])))
}

fn rigid_body(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    Ok(record(BTreeMap::from([
        ("body_type".into(), Value::Enum("Dynamic".into())),
        ("mass".into(), Value::Float32(1.0)),
        ("linear_damping".into(), Value::Float32(0.0)),
        ("angular_damping".into(), Value::Float32(0.0)),
        ("enabled".into(), Value::Bool(true)),
        ("gravity_scale".into(), Value::Float32(1.0)),
        ("can_sleep".into(), Value::Bool(true)),
        ("ccd_enabled".into(), Value::Bool(false)),
    ])))
}

fn collider(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    let shape = Value::Map(BTreeMap::from([
        ("kind".into(), Value::Enum("Cuboid".into())),
        (
            "params".into(),
            Value::Map(BTreeMap::from([
                ("hx".into(), Value::Float32(0.5)),
                ("hy".into(), Value::Float32(0.5)),
                ("hz".into(), Value::Float32(0.5)),
            ])),
        ),
    ]));
    Ok(record(BTreeMap::from([
        ("shape".into(), shape),
        ("density".into(), Value::Float32(1.0)),
        ("friction".into(), Value::Float32(0.5)),
        ("restitution".into(), Value::Float32(0.0)),
        ("is_trigger".into(), Value::Bool(false)),
        ("collision_group".into(), Value::UInt(u32::MAX as u64)),
        ("collision_mask".into(), Value::UInt(u32::MAX as u64)),
    ])))
}

fn physics_material(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    Ok(record(BTreeMap::from([
        ("friction".into(), Value::Float32(0.5)),
        ("restitution".into(), Value::Float32(0.0)),
        ("density".into(), Value::Float32(1.0)),
    ])))
}

fn gravity_source(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    Ok(record(BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("enabled".into(), Value::Bool(true)),
        ("strength".into(), Value::Float32(9.81)),
        ("direction".into(), Value::Vec3([0.0, -1.0, 0.0])),
        ("center".into(), Value::Vec3([0.0, 0.0, 0.0])),
        ("falloff".into(), Value::Enum("None".into())),
    ])))
}

fn character_controller(entity: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    let position = entity
        .components
        .get(TRANSFORM_COMPONENT)
        .and_then(|transform| transform.fields.get("translation"))
        .and_then(|value| match value {
            Value::Vec3(position) => Some(*position),
            _ => None,
        })
        .unwrap_or([0.0; 3]);
    Ok(record(BTreeMap::from([
        ("height".into(), Value::Float32(1.8)),
        ("radius".into(), Value::Float32(0.3)),
        ("move_speed".into(), Value::Float32(5.0)),
        ("acceleration".into(), Value::Float32(20.0)),
        ("deceleration".into(), Value::Float32(15.0)),
        ("air_acceleration".into(), Value::Float32(5.0)),
        ("air_deceleration".into(), Value::Float32(2.0)),
        ("jump_velocity".into(), Value::Float32(5.0)),
        ("gravity_scale".into(), Value::Float32(1.0)),
        ("max_fall_speed".into(), Value::Float32(20.0)),
        ("step_height".into(), Value::Float32(0.3)),
        ("slope_limit".into(), Value::Float32(45.0)),
        ("skin_offset".into(), Value::Float32(0.01)),
        ("state".into(), Value::Enum("Falling".into())),
        ("position".into(), Value::Vec3(position)),
        ("velocity".into(), Value::Vec3([0.0; 3])),
        ("foot_ik_enabled".into(), Value::Bool(true)),
    ])))
}

fn audio_listener(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    Ok(record(BTreeMap::from([(
        "enabled".into(),
        Value::Bool(true),
    )])))
}

fn ui_canvas(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    let mut canvas = engine_ui::Canvas::new(1920.0, 1080.0);
    canvas.scale_mode = engine_ui::ScaleMode::FitWidth;
    Ok(record(engine_ui::serialize_canvas_fields(&canvas)))
}

fn audio_source(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    let source = engine_audio::AudioSourceComponent::default();
    let mut fields = engine_audio::components::serialize_audio_source(&source);
    // Keep the optional slot visible in the generic Inspector. An empty ID is
    // deserialized as an unassigned source until the user selects an asset.
    fields.insert("clip_asset".into(), Value::Asset(AssetId::new("")));
    Ok(record(fields))
}

fn ik_target(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    let target = engine_animation::IkTargetComponent::default();
    Ok(record(engine_animation::serialize_ik_target_fields(
        &target,
    )))
}

fn nav_agent(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    let agent = engine_nav::AiAgent::default();
    let mut fields = engine_nav::components::serialize_ai_agent(&agent);
    fields.insert("navmesh_ref".into(), Value::Asset(AssetId::new("")));
    Ok(record(fields))
}

fn terrain_volume(_: &EntityRecord) -> Result<ComponentRecord, EditorError> {
    let terrain = engine_terrain::TerrainVolume::default();
    Ok(record(BTreeMap::from([
        ("enabled".into(), Value::Bool(terrain.enabled)),
        ("seed".into(), Value::UInt(terrain.seed)),
        ("chunk_size".into(), Value::Float32(terrain.chunk_size)),
        (
            "base_resolution".into(),
            Value::UInt(u64::from(terrain.base_resolution)),
        ),
        ("height_scale".into(), Value::Float32(terrain.height_scale)),
        ("frequency".into(), Value::Float32(terrain.frequency)),
        ("octaves".into(), Value::UInt(u64::from(terrain.octaves))),
        ("lacunarity".into(), Value::Float32(terrain.lacunarity)),
        ("gain".into(), Value::Float32(terrain.gain)),
        (
            "domain_warp_amplitude".into(),
            Value::Float32(terrain.domain_warp_amplitude),
        ),
        (
            "domain_warp_frequency".into(),
            Value::Float32(terrain.domain_warp_frequency),
        ),
        ("skirt_depth".into(), Value::Float32(terrain.skirt_depth)),
        (
            "collision_enabled".into(),
            Value::Bool(terrain.collision_enabled),
        ),
        (
            "material_asset".into(),
            Value::Asset(AssetId::new("mat-default")),
        ),
        (
            "lod_distances".into(),
            Value::List(
                terrain
                    .lod_distances
                    .into_iter()
                    .map(Value::Float32)
                    .collect(),
            ),
        ),
        (
            "lod_hysteresis".into(),
            Value::Float32(terrain.lod_hysteresis),
        ),
    ])))
}

const COMPONENT_DESCRIPTORS: [ComponentDescriptor; 16] = [
    ComponentDescriptor {
        type_id: TRANSFORM_COMPONENT,
        display_name: "Transform",
        category: "Core",
        removable: false,
        required_components: &[],
        factory: transform,
    },
    ComponentDescriptor {
        type_id: RENDERABLE_COMPONENT,
        display_name: "Mesh Renderer",
        category: "Rendering",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT],
        factory: renderable,
    },
    ComponentDescriptor {
        type_id: CAMERA_COMPONENT,
        display_name: "Camera",
        category: "Rendering",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT],
        factory: camera,
    },
    ComponentDescriptor {
        type_id: LIGHT_COMPONENT,
        display_name: "Light",
        category: "Rendering",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT],
        factory: light,
    },
    ComponentDescriptor {
        type_id: BOUNDS_COMPONENT,
        display_name: "Bounds",
        category: "Rendering",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT],
        factory: bounds,
    },
    ComponentDescriptor {
        type_id: RIGID_BODY_COMPONENT,
        display_name: "Rigidbody",
        category: "Physics",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT],
        factory: rigid_body,
    },
    ComponentDescriptor {
        type_id: COLLIDER_COMPONENT,
        display_name: "Box Collider",
        category: "Physics",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT, RIGID_BODY_COMPONENT],
        factory: collider,
    },
    ComponentDescriptor {
        type_id: PHYSICS_MATERIAL_COMPONENT,
        display_name: "Physics Material",
        category: "Physics",
        removable: true,
        required_components: &[
            TRANSFORM_COMPONENT,
            RIGID_BODY_COMPONENT,
            COLLIDER_COMPONENT,
        ],
        factory: physics_material,
    },
    ComponentDescriptor {
        type_id: GRAVITY_SOURCE_COMPONENT,
        display_name: "Gravity Source",
        category: "Physics",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT],
        factory: gravity_source,
    },
    ComponentDescriptor {
        type_id: CHARACTER_CONTROLLER_COMPONENT,
        display_name: "Character Controller",
        category: "Gameplay",
        removable: true,
        // The engine character controller is kinematic and explicitly does
        // not own a physics rigid body or collider.
        required_components: &[TRANSFORM_COMPONENT],
        factory: character_controller,
    },
    ComponentDescriptor {
        type_id: AUDIO_LISTENER_COMPONENT,
        display_name: "Audio Listener",
        category: "Audio",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT],
        factory: audio_listener,
    },
    ComponentDescriptor {
        type_id: UI_CANVAS_COMPONENT,
        display_name: "Canvas",
        category: "UI",
        removable: true,
        required_components: &[],
        factory: ui_canvas,
    },
    ComponentDescriptor {
        type_id: AUDIO_SOURCE_COMPONENT,
        display_name: "Audio Source",
        category: "Audio",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT],
        factory: audio_source,
    },
    ComponentDescriptor {
        type_id: IK_TARGET_COMPONENT,
        display_name: "IK Target",
        category: "Animation",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT],
        factory: ik_target,
    },
    ComponentDescriptor {
        type_id: NAV_AGENT_COMPONENT,
        display_name: "AI Navigation Agent",
        category: "Navigation",
        removable: true,
        required_components: &[TRANSFORM_COMPONENT, CHARACTER_CONTROLLER_COMPONENT],
        factory: nav_agent,
    },
    ComponentDescriptor {
        type_id: TERRAIN_VOLUME_COMPONENT,
        display_name: "Terrain Volume",
        category: "Terrain",
        removable: true,
        required_components: &[],
        factory: terrain_volume,
    },
];

fn configure_unchanged(_: &mut EntityRecord) {}

fn configure_static_body(entity: &mut EntityRecord) {
    let Some(body) = entity.components.get_mut(RIGID_BODY_COMPONENT) else {
        return;
    };
    body.fields
        .insert("body_type".into(), Value::Enum("Static".into()));
    body.fields.insert("mass".into(), Value::Float32(0.0));
    body.fields
        .insert("gravity_scale".into(), Value::Float32(0.0));
}

fn configure_point_light(entity: &mut EntityRecord) {
    let Some(light) = entity.components.get_mut(LIGHT_COMPONENT) else {
        return;
    };
    light
        .fields
        .insert("kind".into(), Value::Enum("Point".into()));
    light
        .fields
        .insert("intensity".into(), Value::Float32(800.0));
    light.fields.insert("range".into(), Value::Float32(10.0));
}

fn configure_spot_light(entity: &mut EntityRecord) {
    let Some(light) = entity.components.get_mut(LIGHT_COMPONENT) else {
        return;
    };
    light
        .fields
        .insert("kind".into(), Value::Enum("Spot".into()));
    light
        .fields
        .insert("intensity".into(), Value::Float32(800.0));
    light.fields.insert("range".into(), Value::Float32(10.0));
    light.fields.insert(
        "spot_angles".into(),
        Value::List(vec![
            Value::Float32(25.0_f32.to_radians()),
            Value::Float32(35.0_f32.to_radians()),
        ]),
    );
}

const ENTITY_TEMPLATES: [EntityTemplate; 14] = [
    EntityTemplate {
        id: "empty",
        display_name: "GameObject",
        category: "Core",
        component_types: &[TRANSFORM_COMPONENT],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "terrain",
        display_name: "Terrain",
        category: "3D Object",
        component_types: &[TERRAIN_VOLUME_COMPONENT],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "cube",
        display_name: "Cube",
        category: "3D Object",
        component_types: &[TRANSFORM_COMPONENT, RENDERABLE_COMPONENT],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "dynamic-cube",
        display_name: "Dynamic Cube",
        category: "3D Object",
        component_types: &[
            TRANSFORM_COMPONENT,
            RENDERABLE_COMPONENT,
            RIGID_BODY_COMPONENT,
            COLLIDER_COMPONENT,
        ],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "static-cube",
        display_name: "Static Cube",
        category: "3D Object",
        component_types: &[
            TRANSFORM_COMPONENT,
            RENDERABLE_COMPONENT,
            RIGID_BODY_COMPONENT,
            COLLIDER_COMPONENT,
        ],
        configure: configure_static_body,
    },
    EntityTemplate {
        id: "camera",
        display_name: "Camera",
        category: "Camera",
        component_types: &[
            TRANSFORM_COMPONENT,
            CAMERA_COMPONENT,
            AUDIO_LISTENER_COMPONENT,
        ],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "directional-light",
        display_name: "Directional Light",
        category: "Light",
        component_types: &[TRANSFORM_COMPONENT, LIGHT_COMPONENT],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "point-light",
        display_name: "Point Light",
        category: "Light",
        component_types: &[TRANSFORM_COMPONENT, LIGHT_COMPONENT],
        configure: configure_point_light,
    },
    EntityTemplate {
        id: "spot-light",
        display_name: "Spot Light",
        category: "Light",
        component_types: &[TRANSFORM_COMPONENT, LIGHT_COMPONENT],
        configure: configure_spot_light,
    },
    EntityTemplate {
        id: "audio-listener",
        display_name: "Audio Listener",
        category: "Audio",
        component_types: &[TRANSFORM_COMPONENT, AUDIO_LISTENER_COMPONENT],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "character",
        display_name: "Character",
        category: "Gameplay",
        component_types: &[
            TRANSFORM_COMPONENT,
            RENDERABLE_COMPONENT,
            CHARACTER_CONTROLLER_COMPONENT,
        ],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "canvas",
        display_name: "Canvas",
        category: "UI",
        component_types: &[UI_CANVAS_COMPONENT],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "audio-source",
        display_name: "Audio Source",
        category: "Audio",
        component_types: &[TRANSFORM_COMPONENT, AUDIO_SOURCE_COMPONENT],
        configure: configure_unchanged,
    },
    EntityTemplate {
        id: "nav-agent",
        display_name: "Navigation Agent",
        category: "Navigation",
        component_types: &[
            TRANSFORM_COMPONENT,
            RENDERABLE_COMPONENT,
            CHARACTER_CONTROLLER_COMPONENT,
            NAV_AGENT_COMPONENT,
        ],
        configure: configure_unchanged,
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use engine_scene::{sample_scene, validate_scene, World};

    use super::*;

    fn empty_entity() -> EntityRecord {
        EntityRecord {
            persistent_id: "entity".into(),
            parent: None,
            name: None,
            enabled: true,
            components: BTreeMap::new(),
        }
    }

    #[test]
    fn descriptor_ids_and_template_ids_are_unique() {
        let mut component_ids = BTreeSet::new();
        for descriptor in ComponentCatalog::descriptors() {
            assert!(
                component_ids.insert(descriptor.type_id),
                "{}",
                descriptor.type_id
            );
        }

        let mut template_ids = BTreeSet::new();
        for template in ComponentCatalog::templates() {
            assert!(template_ids.insert(template.id), "{}", template.id);
        }
    }

    #[test]
    fn every_template_uses_catalog_components_and_validates_as_a_scene() {
        for template in ComponentCatalog::templates() {
            for type_id in template.component_types {
                assert!(ComponentCatalog::descriptor(type_id).is_some(), "{type_id}");
            }
            let entity = ComponentCatalog::instantiate_template(
                template.id,
                format!("{}-1", template.id),
                None,
            )
            .unwrap();
            let mut scene = sample_scene();
            scene.entities = vec![entity];
            scene.scene_settings.active_camera = None;
            scene.dependencies = scene.collect_asset_dependencies();
            assert!(validate_scene(&scene).is_empty(), "{}", template.id);
        }
    }

    #[test]
    fn one_empty_template_supports_root_and_child_creation_without_aliases() {
        let root = ComponentCatalog::instantiate_template("empty", "parent".into(), None).unwrap();
        let child = ComponentCatalog::instantiate_template(
            "empty",
            "child".into(),
            Some(root.persistent_id.clone()),
        )
        .unwrap();
        assert_eq!(child.parent.as_deref(), Some("parent"));

        let mut scene = sample_scene();
        scene.entities = vec![root, child];
        scene.scene_settings.active_camera = None;
        assert!(validate_scene(&scene).is_empty());
    }

    #[test]
    fn templates_apply_real_component_variants() {
        let static_cube =
            ComponentCatalog::instantiate_template("static-cube", "static".into(), None).unwrap();
        assert_eq!(
            static_cube.components[RIGID_BODY_COMPONENT]
                .fields
                .get("body_type"),
            Some(&Value::Enum("Static".into()))
        );

        let point =
            ComponentCatalog::instantiate_template("point-light", "point".into(), None).unwrap();
        assert_eq!(
            point.components[LIGHT_COMPONENT].fields.get("kind"),
            Some(&Value::Enum("Point".into()))
        );

        let spot =
            ComponentCatalog::instantiate_template("spot-light", "spot".into(), None).unwrap();
        assert_eq!(
            spot.components[LIGHT_COMPONENT].fields.get("kind"),
            Some(&Value::Enum("Spot".into()))
        );
        assert!(spot.components[LIGHT_COMPONENT]
            .fields
            .contains_key("spot_angles"));
    }

    #[test]
    fn character_template_uses_the_kinematic_dependency_chain() {
        let character =
            ComponentCatalog::instantiate_template("character", "character-1".into(), None)
                .unwrap();
        assert!(character
            .components
            .contains_key(CHARACTER_CONTROLLER_COMPONENT));
        assert!(!character.components.contains_key(RIGID_BODY_COMPONENT));
        assert!(!character.components.contains_key(COLLIDER_COMPONENT));
    }

    #[test]
    fn runtime_authorable_components_and_templates_are_public() {
        for type_id in [
            UI_CANVAS_COMPONENT,
            AUDIO_SOURCE_COMPONENT,
            IK_TARGET_COMPONENT,
            NAV_AGENT_COMPONENT,
            TERRAIN_VOLUME_COMPONENT,
        ] {
            assert!(ComponentCatalog::descriptor(type_id).is_some(), "{type_id}");
        }
        for template_id in ["canvas", "audio-source", "nav-agent"] {
            assert!(
                ComponentCatalog::template(template_id).is_some(),
                "{template_id}"
            );
        }
        for type_id in [
            "engine.script",
            "engine.animation_player",
            "engine.skeleton",
        ] {
            assert!(ComponentCatalog::descriptor(type_id).is_none(), "{type_id}");
        }
    }

    #[cfg(feature = "tooling-editor")]
    fn editable_registry() -> engine_scene::registry::ComponentRegistry {
        let mut components = engine_scene::registry::ComponentRegistry::new();
        components.register_core();
        engine_character::register_character_extensions(&mut components, None);
        engine_physics::register_physics_extensions(&mut components, None);
        engine_ui::register_ui_extensions(&mut components);
        engine_terrain::register_terrain_extensions(&mut components);

        let mut assets = engine_scene::registry::AssetTypeRegistry::new();
        engine_audio::register_audio_extensions(&mut components, &mut assets);
        engine_nav::register_nav_extensions(&mut components, None, &mut assets);

        let mut render_extensions = engine_renderer::RenderExtensionRegistry::new();
        let mut debug_draw = engine_renderer::DebugDrawRegistry::new();
        engine_animation::register_animation_extensions(
            &mut components,
            &mut assets,
            &mut render_extensions,
            &mut debug_draw,
        );
        components
    }

    fn add_component_with_dependencies(entity: &mut EntityRecord, type_id: &str) {
        if entity.components.contains_key(type_id) {
            return;
        }
        let descriptor = ComponentCatalog::descriptor(type_id)
            .unwrap_or_else(|| panic!("{type_id} is not authorable"));
        for dependency in descriptor.required_components {
            add_component_with_dependencies(entity, dependency);
        }
        let component = descriptor.create_default(entity).unwrap();
        entity.components.insert(type_id.into(), component);
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn every_registered_authorable_default_roundtrips_through_scene_world() {
        let registry = Arc::new(editable_registry());

        for descriptor in ComponentCatalog::descriptors() {
            let extension = registry
                .get(descriptor.type_id)
                .unwrap_or_else(|| panic!("{} is not registered", descriptor.type_id));
            assert!(
                extension.meta.has_editor,
                "{} is not editor-enabled",
                descriptor.type_id
            );

            let mut entity = empty_entity();
            add_component_with_dependencies(&mut entity, descriptor.type_id);
            let expected_components = entity.components.clone();
            let mut scene = sample_scene();
            scene.entities = vec![entity];
            scene.scene_settings.active_camera = None;
            assert!(validate_scene(&scene).is_empty(), "{}", descriptor.type_id);
            let world = World::try_from_scene_with_registry(&scene, Arc::clone(&registry))
                .unwrap_or_else(|error| {
                    panic!(
                        "{} default record failed registry load: {:?}",
                        descriptor.type_id, error.diagnostics
                    )
                });
            let roundtripped = world.to_scene();
            assert_eq!(roundtripped.entities.len(), 1);
            for (type_id, expected) in expected_components {
                assert_eq!(
                    roundtripped.entities[0].components.get(&type_id),
                    Some(&expected),
                    "{type_id} did not preserve its canonical default record"
                );
            }
        }
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn registered_editor_metadata_cannot_reintroduce_unauthorable_entries() {
        let registry = editable_registry();
        let catalog_ids = ComponentCatalog::descriptors()
            .iter()
            .map(|descriptor| descriptor.type_id)
            .collect::<BTreeSet<_>>();
        let intentionally_hidden = BTreeSet::from([
            // EntityRecord.name is the canonical editor name field.
            "engine.name",
            // These are not currently editor-enabled, but keeping them in the
            // deny set prevents a metadata-only change from exposing them.
            "engine.animation_player",
            "engine.skeleton",
        ]);

        for extension in registry
            .iter()
            .filter(|extension| extension.meta.has_editor)
        {
            assert!(
                catalog_ids.contains(extension.meta.type_id)
                    || intentionally_hidden.contains(extension.meta.type_id),
                "registered editor component {} needs an explicit authoring decision",
                extension.meta.type_id
            );
        }
        for type_id in intentionally_hidden {
            assert!(!catalog_ids.contains(type_id), "{type_id}");
        }
        assert!(!catalog_ids.contains("engine.script"));
    }
}
