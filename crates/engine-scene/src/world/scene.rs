use std::collections::BTreeMap;

use engine_serialize::{AssetId, ComponentTypeId, PersistentId, SchemaVersion, Value};

use crate::scene::{ComponentRecord, DiagnosticsPolicy, EntityRecord, Scene};

use crate::components::{
    Bounds, Camera, CameraProjection, Light, LightKind, Name, Renderable, Transform,
};
use crate::{Component, Entity};

use super::World;

impl World {
    /// Build a [`Scene`] from the current World state.
    ///
    /// Only entities that have a persistent_id (i.e. were originally created
    /// via [`from_scene`](World::from_scene) or manually assigned) will
    /// appear in the output.
    pub fn to_scene(&self) -> Scene {
        let mut scene_entities: Vec<EntityRecord> = Vec::new();

        // Iterate all entity indices that have persistent IDs.
        for (idx, pid_opt) in self.entity_to_persistent.iter().enumerate() {
            let Some(persistent_id) = pid_opt else {
                continue;
            };
            let entity_index = idx as u32;

            // We need to find a generation for this entity.  Since we
            // don't store generations per-index in the World-level map,
            // we reconstruct from the EntityManager.
            let entity = Entity::new(entity_index, 0);

            // Skip stale / freed entities.
            if !self.entities.is_alive(entity) {
                continue;
            }

            let mut components: BTreeMap<ComponentTypeId, ComponentRecord> = BTreeMap::new();

            // Name
            if let Some(name) = self.get::<Name>(entity) {
                let mut fields = BTreeMap::new();
                fields.insert("name".to_string(), Value::Str(name.0.clone()));
                components.insert(
                    Name::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: SchemaVersion::new(0, 1, 0),
                        enabled: true,
                        fields,
                    },
                );
            }

            // Transform
            if let Some(transform) = self.get::<Transform>(entity) {
                let mut fields = BTreeMap::new();
                fields.insert(
                    "translation".to_string(),
                    Value::Vec3(transform.translation.into()),
                );
                fields.insert(
                    "rotation".to_string(),
                    Value::Quat(transform.rotation.into()),
                );
                fields.insert("scale".to_string(), Value::Vec3(transform.scale.into()));
                if let Some(parent) = &transform.parent {
                    if let Some(pid) = self
                        .entity_to_persistent
                        .get(parent.index() as usize)
                        .and_then(|p| p.as_ref())
                    {
                        fields.insert("parent".to_string(), Value::Entity(pid.clone()));
                    }
                }
                components.insert(
                    Transform::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: SchemaVersion::new(0, 1, 0),
                        enabled: true,
                        fields,
                    },
                );
            }

            // Renderable
            if let Some(renderable) = self.get::<Renderable>(entity) {
                let mut fields = BTreeMap::new();
                fields.insert(
                    "mesh".to_string(),
                    Value::Asset(AssetId::new(&renderable.mesh_asset)),
                );
                fields.insert(
                    "material".to_string(),
                    Value::Asset(AssetId::new(&renderable.material_asset)),
                );
                fields.insert("visible".to_string(), Value::Bool(renderable.visible));
                fields.insert(
                    "cast_shadows".to_string(),
                    Value::Bool(renderable.cast_shadows),
                );
                fields.insert(
                    "render_layer".to_string(),
                    Value::Str(renderable.render_layer.clone()),
                );
                components.insert(
                    Renderable::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: SchemaVersion::new(0, 1, 0),
                        enabled: true,
                        fields,
                    },
                );
            }

            // Camera
            if let Some(camera) = self.get::<Camera>(entity) {
                let mut fields = BTreeMap::new();
                fields.insert(
                    "projection".to_string(),
                    Value::Enum(match camera.projection {
                        CameraProjection::Perspective => "Perspective".to_string(),
                        CameraProjection::Orthographic => "Orthographic".to_string(),
                    }),
                );
                fields.insert("near".to_string(), Value::Float32(camera.near));
                fields.insert("far".to_string(), Value::Float32(camera.far));
                fields.insert("fov_y".to_string(), Value::Float32(camera.fov_y));
                fields.insert(
                    "ortho_half_height".to_string(),
                    Value::Float32(camera.ortho_half_height),
                );
                if let Some(vp) = camera.viewport_rect {
                    fields.insert(
                        "viewport_rect".to_string(),
                        Value::List(vp.iter().map(|v| Value::Float32(*v)).collect()),
                    );
                }
                fields.insert(
                    "render_layer_mask".to_string(),
                    Value::UInt(camera.render_layer_mask as u64),
                );
                fields.insert(
                    "clear_flags".to_string(),
                    Value::UInt(camera.clear_flags as u64),
                );
                fields.insert("clear_color".to_string(), Value::Color(camera.clear_color));
                fields.insert("priority".to_string(), Value::Int(camera.priority as i64));
                fields.insert(
                    "msaa_samples".to_string(),
                    Value::UInt(camera.msaa_samples as u64),
                );
                fields.insert("hdr_output".to_string(), Value::Bool(camera.hdr_output));
                fields.insert("aperture".to_string(), Value::Float32(camera.aperture));
                fields.insert(
                    "shutter_speed".to_string(),
                    Value::Float32(camera.shutter_speed),
                );
                fields.insert("iso".to_string(), Value::Float32(camera.iso));
                fields.insert(
                    "ev_compensation".to_string(),
                    Value::Float32(camera.ev_compensation),
                );
                components.insert(
                    Camera::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: SchemaVersion::new(0, 1, 0),
                        enabled: true,
                        fields,
                    },
                );
            }

            // Light
            if let Some(light) = self.get::<Light>(entity) {
                let mut fields = BTreeMap::new();
                fields.insert(
                    "kind".to_string(),
                    Value::Enum(match light.kind {
                        LightKind::Directional => "Directional".to_string(),
                        LightKind::Point => "Point".to_string(),
                        LightKind::Spot => "Spot".to_string(),
                    }),
                );
                fields.insert("color".to_string(), Value::Vec3(light.color));
                fields.insert("intensity".to_string(), Value::Float32(light.intensity));
                fields.insert("range".to_string(), Value::Float32(light.range));
                if let Some(angles) = light.spot_angles {
                    fields.insert(
                        "spot_angles".to_string(),
                        Value::List(vec![Value::Float32(angles[0]), Value::Float32(angles[1])]),
                    );
                }
                fields.insert(
                    "shadow_mode".to_string(),
                    Value::UInt(light.shadow_mode as u64),
                );
                fields.insert("direction".to_string(), Value::Vec3(light.direction));
                components.insert(
                    Light::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: SchemaVersion::new(0, 1, 0),
                        enabled: true,
                        fields,
                    },
                );
            }

            // Bounds
            if let Some(bounds) = self.get::<Bounds>(entity) {
                let mut fields = BTreeMap::new();
                fields.insert("center".to_string(), Value::Vec3(bounds.center));
                fields.insert("half_extents".to_string(), Value::Vec3(bounds.half_extents));
                components.insert(
                    Bounds::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: SchemaVersion::new(0, 1, 0),
                        enabled: true,
                        fields,
                    },
                );
            }

            scene_entities.push(EntityRecord {
                persistent_id: persistent_id.clone(),
                parent: self.resolve_parent_to_persistent(entity),
                name: self.get::<Name>(entity).map(|n| n.0.clone()),
                enabled: true,
                components,
            });
        }

        Scene {
            schema_version: SchemaVersion::new(0, 1, 0),
            engine_version: "0.1.0".to_string(),
            scene_id: self.scene_id.clone(),
            name: self.scene_name.clone(),
            entities: scene_entities,
            scene_settings: self.scene_settings.clone(),
            dependencies: Vec::new(),
            diagnostics_policy: DiagnosticsPolicy::Strict,
        }
    }

    /// Build a [`World`] from an existing [`Scene`].
    ///
    /// All entities in the scene get an [`Entity`] handle and their typed
    /// components are populated from the scene's component records.
    pub fn from_scene(scene: &Scene) -> Self {
        let mut world = Self::new();

        // Preserve scene-level metadata.
        world.scene_settings = scene.scene_settings.clone();
        world.scene_id = scene.scene_id.clone();
        world.scene_name = scene.name.clone();

        // First pass: allocate entities and record persistent_id mappings.
        for entity_record in &scene.entities {
            let entity = world.create_entity();
            let idx = entity.index() as usize;
            // Record persistent_id mapping.
            world
                .persistent_to_entity
                .insert(entity_record.persistent_id.clone(), entity);
            if world.entity_to_persistent.len() <= idx {
                world.entity_to_persistent.resize(idx + 1, None);
            }
            world.entity_to_persistent[idx] = Some(entity_record.persistent_id.clone());

            // Copy EntityRecord.name to a Name component.
            if let Some(ref name) = entity_record.name {
                world.add_component(entity, Name(name.clone()));
            }
        }

        // Second pass: populate components with resolved references.
        for entity_record in &scene.entities {
            let Some(&entity) = world.persistent_to_entity.get(&entity_record.persistent_id) else {
                continue;
            };

            for (comp_type_id, comp_record) in &entity_record.components {
                if !comp_record.enabled {
                    continue;
                }
                world.populate_component(entity, comp_type_id, &comp_record.fields);
            }
        }

        world
    }

    // ── Internal helpers ──────────────────────────────────────────────

    /// Resolve the parent entity to a persistent_id string for serialization.
    fn resolve_parent_to_persistent(&self, entity: Entity) -> Option<PersistentId> {
        if let Some(transform) = self.get::<Transform>(entity) {
            if let Some(parent) = &transform.parent {
                let idx = parent.index() as usize;
                if idx < self.entity_to_persistent.len() {
                    return self.entity_to_persistent[idx].clone();
                }
            }
        }
        None
    }

    /// Populate a typed component from scene field data.
    fn populate_component(
        &mut self,
        entity: Entity,
        comp_type_id: &str,
        fields: &BTreeMap<String, Value>,
    ) {
        match comp_type_id {
            Name::TYPE_ID => {
                if let Some(Value::Str(name)) = fields.get("name") {
                    self.add_component(entity, Name(name.clone()));
                }
            }
            Transform::TYPE_ID => {
                let translation = match fields.get("translation") {
                    Some(Value::Vec3(v)) => glam::Vec3::from(*v),
                    _ => glam::Vec3::ZERO,
                };
                let rotation = match fields.get("rotation") {
                    Some(Value::Quat(q)) => glam::Quat::from_array(*q),
                    _ => glam::Quat::IDENTITY,
                };
                let scale = match fields.get("scale") {
                    Some(Value::Vec3(v)) => glam::Vec3::from(*v),
                    _ => glam::Vec3::ONE,
                };
                let parent = match fields.get("parent") {
                    Some(Value::Entity(pid)) => self.persistent_to_entity.get(pid).copied(),
                    _ => None,
                };
                self.add_component(
                    entity,
                    Transform {
                        translation,
                        rotation,
                        scale,
                        parent,
                    },
                );
            }
            Renderable::TYPE_ID => {
                let mesh_asset = match fields.get("mesh") {
                    Some(Value::Asset(a)) => a.id.clone(),
                    _ => return, // mesh is required
                };
                let material_asset = match fields.get("material") {
                    Some(Value::Asset(a)) => a.id.clone(),
                    _ => return, // material is required
                };
                let visible = match fields.get("visible") {
                    Some(Value::Bool(v)) => *v,
                    _ => true,
                };
                let cast_shadows = match fields.get("cast_shadows") {
                    Some(Value::Bool(v)) => *v,
                    _ => true,
                };
                let render_layer = match fields.get("render_layer") {
                    Some(Value::Str(s)) => s.clone(),
                    _ => "Default".to_string(),
                };
                self.add_component(
                    entity,
                    Renderable {
                        mesh_asset,
                        material_asset,
                        visible,
                        cast_shadows,
                        render_layer,
                    },
                );
            }
            Camera::TYPE_ID => {
                let projection = match fields.get("projection") {
                    Some(Value::Enum(s)) if s == "Orthographic" => CameraProjection::Orthographic,
                    _ => CameraProjection::Perspective,
                };
                let near = match fields.get("near") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => Camera::default().near,
                };
                let far = match fields.get("far") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => Camera::default().far,
                };
                let fov_y = match fields.get("fov_y") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => Camera::default().fov_y,
                };
                let ortho_half_height = match fields.get("ortho_half_height") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => Camera::default().ortho_half_height,
                };
                let viewport_rect = match fields.get("viewport_rect") {
                    Some(Value::List(items)) if items.len() == 4 => Some([
                        Self::value_as_f32(&items[0]),
                        Self::value_as_f32(&items[1]),
                        Self::value_as_f32(&items[2]),
                        Self::value_as_f32(&items[3]),
                    ]),
                    _ => None,
                };
                let render_layer_mask = match fields.get("render_layer_mask") {
                    Some(Value::UInt(v)) => *v as u32,
                    _ => Camera::default().render_layer_mask,
                };
                let clear_flags = match fields.get("clear_flags") {
                    Some(Value::UInt(v)) => *v as u8,
                    _ => Camera::default().clear_flags,
                };
                let clear_color = match fields.get("clear_color") {
                    Some(Value::Color(c)) => *c,
                    _ => Camera::default().clear_color,
                };
                let priority = match fields.get("priority") {
                    Some(Value::Int(v)) => *v as i32,
                    _ => Camera::default().priority,
                };
                let msaa_samples = match fields.get("msaa_samples") {
                    Some(Value::UInt(v)) => *v as u8,
                    _ => Camera::default().msaa_samples,
                };
                let hdr_output = match fields.get("hdr_output") {
                    Some(Value::Bool(v)) => *v,
                    _ => Camera::default().hdr_output,
                };
                let aperture = match fields.get("aperture") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => Camera::default().aperture,
                };
                let shutter_speed = match fields.get("shutter_speed") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => Camera::default().shutter_speed,
                };
                let iso = match fields.get("iso") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => Camera::default().iso,
                };
                let ev_compensation = match fields.get("ev_compensation") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => Camera::default().ev_compensation,
                };
                self.add_component(
                    entity,
                    Camera {
                        projection,
                        near,
                        far,
                        fov_y,
                        ortho_half_height,
                        viewport_rect,
                        render_layer_mask,
                        clear_flags,
                        clear_color,
                        priority,
                        msaa_samples,
                        hdr_output,
                        aperture,
                        shutter_speed,
                        iso,
                        ev_compensation,
                    },
                );
            }
            Light::TYPE_ID => {
                let kind = match fields.get("kind") {
                    Some(Value::Enum(s)) if s == "Point" => LightKind::Point,
                    Some(Value::Enum(s)) if s == "Spot" => LightKind::Spot,
                    _ => LightKind::Directional,
                };
                let color = match fields.get("color") {
                    Some(Value::Vec3(c)) => *c,
                    _ => [1.0, 1.0, 1.0],
                };
                let intensity = match fields.get("intensity") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => 1.0,
                };
                let range = match fields.get("range") {
                    Some(Value::Float32(v)) => *v,
                    Some(Value::Float64(v)) => *v as f32,
                    _ => 10.0,
                };
                let spot_angles = match fields.get("spot_angles") {
                    Some(Value::List(items)) if items.len() == 2 => {
                        Some([Self::value_as_f32(&items[0]), Self::value_as_f32(&items[1])])
                    }
                    _ => None,
                };
                let shadow_mode = match fields.get("shadow_mode") {
                    Some(Value::UInt(v)) => *v as u8,
                    _ => 0,
                };
                let direction = match fields.get("direction") {
                    Some(Value::Vec3(d)) => *d,
                    _ => [0.0, -1.0, 0.0],
                };
                self.add_component(
                    entity,
                    Light {
                        kind,
                        color,
                        intensity,
                        range,
                        spot_angles,
                        shadow_mode,
                        direction,
                    },
                );
            }
            Bounds::TYPE_ID => {
                let center = match fields.get("center") {
                    Some(Value::Vec3(c)) => *c,
                    _ => [0.0, 0.0, 0.0],
                };
                let half_extents = match fields.get("half_extents") {
                    Some(Value::Vec3(h)) => *h,
                    _ => [0.5, 0.5, 0.5],
                };
                self.add_component(
                    entity,
                    Bounds {
                        center,
                        half_extents,
                    },
                );
            }
            _ => {
                // Unknown component type — skip (future extensibility).
            }
        }
    }

    /// Helper to extract an f32 from a Value, defaulting to 0.0.
    fn value_as_f32(value: &Value) -> f32 {
        match value {
            Value::Float32(v) => *v,
            Value::Float64(v) => *v as f32,
            Value::Int(v) => *v as f32,
            Value::UInt(v) => *v as f32,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::World;
    use crate::components::{Camera, Name, Renderable};
    use crate::scene::sample_scene;
    use engine_serialize::{AssetId, Value};

    #[test]
    fn world_from_scene_roundtrip() {
        let scene = sample_scene();
        let world = World::from_scene(&scene);
        assert_eq!(world.alive_count(), 2);

        // Verify Name components
        let names: Vec<_> = world.query::<Name>().map(|(_, n)| n.0.clone()).collect();
        assert!(names.contains(&"Main Camera".to_string()));
        assert!(names.contains(&"Cube".to_string()));

        // Verify Camera component
        let cameras: Vec<_> = world.query::<Camera>().collect();
        assert_eq!(cameras.len(), 1);

        // Verify Renderable component
        let renderables: Vec<_> = world.query::<Renderable>().collect();
        assert_eq!(renderables.len(), 1);
        assert_eq!(renderables[0].1.mesh_asset, "mesh-cube");
        assert_eq!(renderables[0].1.material_asset, "mat-default");
    }

    #[test]
    fn world_to_scene_roundtrip() {
        let scene = sample_scene();
        let world = World::from_scene(&scene);
        let scene_back = world.to_scene();

        // The round-tripped scene should have the same number of entities.
        assert_eq!(scene_back.entities.len(), scene.entities.len());

        // Check entity persistent_ids are preserved.
        for orig_entity in &scene.entities {
            let found = scene_back
                .entities
                .iter()
                .any(|e| e.persistent_id == orig_entity.persistent_id);
            assert!(found, "missing entity {}", orig_entity.persistent_id);
        }

        // Check that typed components round-trip.
        for entity in &scene_back.entities {
            if entity.persistent_id == "camera-main" {
                assert!(entity.components.contains_key("engine.camera"));
            }
            if entity.persistent_id == "cube-01" {
                assert!(entity.components.contains_key("engine.renderable"));
                let renderable = &entity.components["engine.renderable"];
                let mesh = renderable.fields.get("mesh");
                assert!(matches!(mesh, Some(Value::Asset(a)) if a.id == "mesh-cube"));
            }
        }
    }

    #[test]
    fn world_from_scene_to_scene_preserves_renderable_fields() {
        let scene = sample_scene();
        let world = World::from_scene(&scene);
        let scene_back = world.to_scene();

        let cube = scene_back
            .entities
            .iter()
            .find(|e| e.persistent_id == "cube-01")
            .expect("cube-01 should exist");

        let r = &cube.components["engine.renderable"];
        assert_eq!(
            r.fields.get("mesh"),
            Some(&Value::Asset(AssetId::new("mesh-cube")))
        );
        assert_eq!(
            r.fields.get("material"),
            Some(&Value::Asset(AssetId::new("mat-default")))
        );
        assert_eq!(r.fields.get("visible"), Some(&Value::Bool(true)));
        assert_eq!(
            r.fields.get("render_layer"),
            Some(&Value::Str("Default".to_string()))
        );
        assert_eq!(r.fields.get("cast_shadows"), Some(&Value::Bool(true)));
    }

    #[test]
    fn world_scene_roundtrip_with_extraction() {
        // Verify that a scene converted to world and back still produces
        // valid extraction output (the existing extraction path still works).
        let scene = sample_scene();
        let world = World::from_scene(&scene);
        let scene_back = world.to_scene();

        // The round-tripped scene should be structurally valid for validation
        // and extraction (no duplicate IDs, valid camera, etc.)
        let diagnostics = crate::validation::validate_scene(&scene_back);
        assert!(
            diagnostics.is_empty(),
            "round-tripped scene has validation errors: {:?}",
            diagnostics
        );

        let result = crate::extraction::extract_renderer_input(&scene_back, 42);
        assert!(
            result.is_ok(),
            "round-tripped scene extraction failed: {:?}",
            result
        );
        let input = result.unwrap();
        assert_eq!(input.frame_index, 42);
        assert_eq!(input.drawables.len(), 1);
        assert_eq!(input.views.len(), 1);
    }
}
