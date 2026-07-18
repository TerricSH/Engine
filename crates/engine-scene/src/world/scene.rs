use std::collections::BTreeMap;
use std::sync::Arc;

use engine_serialize::{AssetId, ComponentTypeId, PersistentId, SchemaVersion, Value};

use crate::registry::ComponentRegistry;
use crate::scene::{
    ComponentRecord, EntityRecord, Scene, SceneLoadDiagnostic, SceneLoadError, SceneLoadReport,
};

use crate::components::{Bounds, Camera, Light, Name, Renderable, Transform};
use crate::prefab_instance::PrefabInstanceRef;
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

            // World metadata is indexed by entity slot. Recover the current
            // live generation instead of guessing generation zero, which
            // would drop recycled entities from scene serialization.
            let Some(entity) = self.entities.live_entity_at(entity_index) else {
                continue;
            };

            let mut components: BTreeMap<ComponentTypeId, ComponentRecord> = BTreeMap::new();
            let schema_version_for = |type_id: &str| {
                self.component_schema_versions
                    .get(idx)
                    .and_then(|versions| versions.get(type_id))
                    .copied()
                    .unwrap_or_else(|| SchemaVersion::new(0, 1, 0))
            };

            // Name
            if let Some(name) = self.get::<Name>(entity) {
                let mut fields = BTreeMap::new();
                fields.insert("name".to_string(), Value::Str(name.0.clone()));
                components.insert(
                    Name::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: schema_version_for(Name::TYPE_ID),
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
                        schema_version: schema_version_for(Transform::TYPE_ID),
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
                        schema_version: schema_version_for(Renderable::TYPE_ID),
                        enabled: true,
                        fields,
                    },
                );
            }

            // Camera
            if let Some(camera) = self.get::<Camera>(entity) {
                components.insert(
                    Camera::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: schema_version_for(Camera::TYPE_ID),
                        enabled: true,
                        fields: crate::components::serialize_camera_fields(camera),
                    },
                );
            }

            // Light
            if let Some(light) = self.get::<Light>(entity) {
                components.insert(
                    Light::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: schema_version_for(Light::TYPE_ID),
                        enabled: true,
                        fields: crate::components::serialize_light_fields(light),
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
                        schema_version: schema_version_for(Bounds::TYPE_ID),
                        enabled: true,
                        fields,
                    },
                );
            }

            // Prefab instance linkage. This is canonical authoring data, not
            // an editor-only side table, so it must survive Scene -> World ->
            // Scene roundtrips.
            if let Some(prefab_ref) = self.get::<PrefabInstanceRef>(entity) {
                let fields = BTreeMap::from([
                    (
                        "source_asset".to_string(),
                        Value::Asset(AssetId::new(&prefab_ref.source_asset)),
                    ),
                    (
                        "instance_id".to_string(),
                        Value::Str(prefab_ref.instance_id.clone()),
                    ),
                    (
                        "entity_persistent_id".to_string(),
                        Value::Str(prefab_ref.entity_persistent_id.clone()),
                    ),
                    (
                        "schema_major".to_string(),
                        Value::UInt(u64::from(prefab_ref.schema_version.major)),
                    ),
                    (
                        "schema_minor".to_string(),
                        Value::UInt(u64::from(prefab_ref.schema_version.minor)),
                    ),
                    (
                        "schema_patch".to_string(),
                        Value::UInt(u64::from(prefab_ref.schema_version.patch)),
                    ),
                ]);
                components.insert(
                    PrefabInstanceRef::TYPE_ID.to_string(),
                    ComponentRecord {
                        schema_version: schema_version_for(PrefabInstanceRef::TYPE_ID),
                        enabled: true,
                        fields,
                    },
                );
            }

            // ── Registered external components (e.g. physics) ──────────
            if let Some(ref registry) = self.component_registry {
                for (&type_id, storage) in &self.storages {
                    if matches!(
                        type_id,
                        Name::TYPE_ID
                            | Transform::TYPE_ID
                            | Renderable::TYPE_ID
                            | Camera::TYPE_ID
                            | Light::TYPE_ID
                            | Bounds::TYPE_ID
                            | PrefabInstanceRef::TYPE_ID
                    ) {
                        continue;
                    }
                    let Some(ext) = registry.get(type_id) else {
                        continue;
                    };
                    let Some(ser_fn) = ext.serialize else {
                        continue;
                    };
                    if let Some(any_ref) = storage.get_any(entity) {
                        let fields = ser_fn(any_ref);
                        components.insert(
                            type_id.to_string(),
                            ComponentRecord {
                                schema_version: schema_version_for(type_id),
                                enabled: true,
                                fields,
                            },
                        );
                    }
                }
            }

            // Disabled records are kept outside component storages so systems
            // cannot accidentally process them. Overlay them last so their
            // exact schema version and field payload survive serialization.
            if let Some(disabled) = self.disabled_components.get(idx) {
                components.extend(disabled.clone());
            }

            let parent = if self.has::<Transform>(entity) {
                self.resolve_parent_to_persistent(entity)
            } else {
                self.entity_parents.get(idx).cloned().flatten()
            };

            scene_entities.push(EntityRecord {
                persistent_id: persistent_id.clone(),
                parent,
                name: self.get::<Name>(entity).map(|n| n.0.clone()),
                enabled: self.is_enabled(entity),
                components,
            });
        }

        Scene {
            schema_version: self.scene_schema_version,
            engine_version: self.scene_engine_version.clone(),
            scene_id: self.scene_id.clone(),
            name: self.scene_name.clone(),
            entities: scene_entities,
            scene_settings: self.scene_settings.clone(),
            dependencies: self.scene_dependencies.clone(),
            diagnostics_policy: self.diagnostics_policy,
        }
    }

    /// Build a [`World`] from an existing [`Scene`].
    ///
    /// All entities in the scene get an [`Entity`] handle and their typed
    /// components are populated from the scene's component records.
    pub fn from_scene(scene: &Scene) -> Self {
        Self::build_from_scene(scene, None).world
    }

    /// Restore a scene with a shared extension registry installed before any
    /// component record is visited. All external-component failures are
    /// returned as structured diagnostics alongside the partially loaded
    /// world.
    pub fn from_scene_with_registry(
        scene: &Scene,
        registry: Arc<ComponentRegistry>,
    ) -> SceneLoadReport {
        Self::build_from_scene(scene, Some(registry))
    }

    /// Strict registry-aware scene loading. Unknown component types, missing
    /// deserialize hooks, invalid storage factories, and type-erased insert
    /// mismatches all fail the load with structured diagnostics.
    pub fn try_from_scene_with_registry(
        scene: &Scene,
        registry: Arc<ComponentRegistry>,
    ) -> Result<Self, SceneLoadError> {
        Self::from_scene_with_registry(scene, registry).into_result()
    }

    fn build_from_scene(
        scene: &Scene,
        registry: Option<Arc<ComponentRegistry>>,
    ) -> SceneLoadReport {
        let mut world = Self::new();
        let mut diagnostics = Vec::new();
        if let Some(registry) = registry {
            let storages = registry.create_storages();
            for (registered_type_id, storage) in &storages {
                if storage.type_id() != *registered_type_id {
                    diagnostics.push(SceneLoadDiagnostic::StorageFactoryTypeMismatch {
                        entity_id: "<registry>".to_string(),
                        component_type_id: (*registered_type_id).to_string(),
                        storage_type_id: storage.type_id().to_string(),
                    });
                }
            }
            // Install the registry itself before any component traversal. Only
            // install its storages after all factories have passed the type-ID
            // check, preventing a bad core-ID factory from making typed core
            // insertion panic.
            world.component_registry = Some(registry);
            if !diagnostics.is_empty() {
                return SceneLoadReport { world, diagnostics };
            }
            world.storages = storages;
        }
        let strict_components = world.component_registry.is_some();

        // Preserve scene-level metadata.
        world.scene_settings = scene.scene_settings.clone();
        world.scene_schema_version = scene.schema_version;
        world.scene_engine_version = scene.engine_version.clone();
        world.scene_id = scene.scene_id.clone();
        world.scene_name = scene.name.clone();
        world.scene_dependencies = scene.dependencies.clone();
        world.diagnostics_policy = scene.diagnostics_policy;

        // First pass: allocate entities and record persistent_id mappings.
        for entity_record in &scene.entities {
            let entity = world.create_entity();
            world.set_enabled(entity, entity_record.enabled);
            let idx = entity.index() as usize;
            // Record persistent_id mapping.
            world
                .persistent_to_entity
                .insert(entity_record.persistent_id.clone(), entity);
            if world.entity_to_persistent.len() <= idx {
                world.entity_to_persistent.resize(idx + 1, None);
            }
            world.entity_to_persistent[idx] = Some(entity_record.persistent_id.clone());
            world.entity_parents[idx] = entity_record.parent.clone();

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
                world.component_schema_versions[entity.index() as usize]
                    .insert(comp_type_id.clone(), comp_record.schema_version);
                if !comp_record.enabled {
                    if strict_components {
                        if let Err(diagnostic) = world.validate_disabled_component(
                            entity,
                            comp_type_id,
                            &comp_record.fields,
                        ) {
                            diagnostics.push(diagnostic);
                        }
                    }
                    world.disabled_components[entity.index() as usize]
                        .insert(comp_type_id.clone(), comp_record.clone());
                    continue;
                }
                if strict_components {
                    if let Err(diagnostic) =
                        world.populate_component_checked(entity, comp_type_id, &comp_record.fields)
                    {
                        diagnostics.push(diagnostic);
                    }
                } else {
                    world.populate_component(entity, comp_type_id, &comp_record.fields);
                }
            }

            // EntityRecord.parent is the canonical hierarchy field. Apply it
            // to an enabled Transform when present, while retaining support
            // for older scenes that encoded the parent only in Transform.
            if let Some(parent_id) = entity_record.parent.as_ref() {
                let parent = world.persistent_to_entity.get(parent_id).copied();
                if let Some(transform) = world.get_mut::<Transform>(entity) {
                    transform.parent = parent;
                }
            }
        }

        SceneLoadReport { world, diagnostics }
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
    pub(crate) fn populate_component(
        &mut self,
        entity: Entity,
        comp_type_id: &str,
        fields: &BTreeMap<String, Value>,
    ) {
        let _ = self.populate_component_checked(entity, comp_type_id, fields);
    }

    fn validate_disabled_component(
        &self,
        entity: Entity,
        comp_type_id: &str,
        fields: &BTreeMap<String, Value>,
    ) -> Result<(), SceneLoadDiagnostic> {
        if matches!(
            comp_type_id,
            Name::TYPE_ID
                | Transform::TYPE_ID
                | Renderable::TYPE_ID
                | Camera::TYPE_ID
                | Light::TYPE_ID
                | Bounds::TYPE_ID
                | PrefabInstanceRef::TYPE_ID
        ) {
            return Ok(());
        }

        let entity_id = self
            .persistent_id(entity)
            .unwrap_or("<runtime-entity>")
            .to_string();
        let Some(registry) = self.component_registry.as_ref() else {
            return Err(SceneLoadDiagnostic::UnknownComponent {
                entity_id,
                component_type_id: comp_type_id.to_string(),
            });
        };
        let Some(extension) = registry.get(comp_type_id) else {
            return Err(SceneLoadDiagnostic::UnknownComponent {
                entity_id,
                component_type_id: comp_type_id.to_string(),
            });
        };
        let Some(deserialize) = extension.deserialize else {
            return Err(SceneLoadDiagnostic::MissingDeserializeHook {
                entity_id,
                component_type_id: comp_type_id.to_string(),
            });
        };

        let mut storage = (extension.storage_factory)();
        if storage.type_id() != extension.meta.type_id {
            return Err(SceneLoadDiagnostic::StorageFactoryTypeMismatch {
                entity_id,
                component_type_id: comp_type_id.to_string(),
                storage_type_id: storage.type_id().to_string(),
            });
        }
        if storage.insert_any(entity, deserialize(fields)).is_err() {
            return Err(SceneLoadDiagnostic::StorageInsertTypeMismatch {
                entity_id,
                component_type_id: comp_type_id.to_string(),
            });
        }
        Ok(())
    }

    fn populate_component_checked(
        &mut self,
        entity: Entity,
        comp_type_id: &str,
        fields: &BTreeMap<String, Value>,
    ) -> Result<(), SceneLoadDiagnostic> {
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
                    _ => return Ok(()), // mesh is required
                };
                let material_asset = match fields.get("material") {
                    Some(Value::Asset(a)) => a.id.clone(),
                    _ => return Ok(()), // material is required
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
                self.add_component(entity, crate::components::deserialize_camera_fields(fields));
            }
            Light::TYPE_ID => {
                self.add_component(entity, crate::components::deserialize_light_fields(fields));
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
            PrefabInstanceRef::TYPE_ID => {
                let source_asset = match fields.get("source_asset") {
                    Some(Value::Asset(asset)) => asset.id.clone(),
                    _ => return Ok(()),
                };
                let instance_id = match fields.get("instance_id") {
                    Some(Value::Str(value)) => value.clone(),
                    _ => return Ok(()),
                };
                let entity_persistent_id = match fields.get("entity_persistent_id") {
                    Some(Value::Str(value)) => value.clone(),
                    _ => return Ok(()),
                };
                let schema_part = |name: &str| match fields.get(name) {
                    Some(Value::UInt(value)) => u16::try_from(*value).ok(),
                    _ => None,
                };
                let Some(schema_major) = schema_part("schema_major") else {
                    return Ok(());
                };
                let Some(schema_minor) = schema_part("schema_minor") else {
                    return Ok(());
                };
                let Some(schema_patch) = schema_part("schema_patch") else {
                    return Ok(());
                };
                self.add_component(
                    entity,
                    PrefabInstanceRef {
                        source_asset,
                        instance_id,
                        entity_persistent_id,
                        schema_version: SchemaVersion::new(
                            schema_major,
                            schema_minor,
                            schema_patch,
                        ),
                    },
                );
            }
            _ => {
                return self.populate_external_component(entity, comp_type_id, fields);
            }
        }
        Ok(())
    }

    fn populate_external_component(
        &mut self,
        entity: Entity,
        comp_type_id: &str,
        fields: &BTreeMap<String, Value>,
    ) -> Result<(), SceneLoadDiagnostic> {
        let entity_id = self
            .persistent_id(entity)
            .unwrap_or("<runtime-entity>")
            .to_string();
        let (registered_type_id, storage_factory, deserialize) = {
            let Some(registry) = self.component_registry.as_ref() else {
                return Err(SceneLoadDiagnostic::UnknownComponent {
                    entity_id,
                    component_type_id: comp_type_id.to_string(),
                });
            };
            let Some(extension) = registry.get(comp_type_id) else {
                return Err(SceneLoadDiagnostic::UnknownComponent {
                    entity_id,
                    component_type_id: comp_type_id.to_string(),
                });
            };
            let Some(deserialize) = extension.deserialize else {
                return Err(SceneLoadDiagnostic::MissingDeserializeHook {
                    entity_id,
                    component_type_id: comp_type_id.to_string(),
                });
            };
            (
                extension.meta.type_id,
                extension.storage_factory,
                deserialize,
            )
        };

        let component = deserialize(fields);
        let storage = self
            .storages
            .entry(registered_type_id)
            .or_insert_with(storage_factory);
        let storage_type_id = storage.type_id();
        if storage_type_id != registered_type_id {
            return Err(SceneLoadDiagnostic::StorageFactoryTypeMismatch {
                entity_id,
                component_type_id: registered_type_id.to_string(),
                storage_type_id: storage_type_id.to_string(),
            });
        }
        storage.insert_any(entity, component).map_err(|_| {
            SceneLoadDiagnostic::StorageInsertTypeMismatch {
                entity_id,
                component_type_id: registered_type_id.to_string(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::components::{Camera, Name, Renderable, Transform};
    use crate::registry::{ComponentExtension, ComponentMeta, ComponentRegistry};
    use crate::scene::{sample_scene, ComponentRecord, SceneLoadDiagnostic};
    use crate::{Component, ComponentStorageDyn, PrefabInstanceRef, SparseSet, World};
    use engine_serialize::{AssetId, SchemaVersion, Value};

    #[derive(Debug, PartialEq)]
    struct ExternalComponent {
        value: u64,
    }

    impl Component for ExternalComponent {
        const TYPE_ID: &'static str = "test.external";
    }

    struct WrongComponent;

    impl Component for WrongComponent {
        const TYPE_ID: &'static str = "test.wrong";
    }

    fn external_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<ExternalComponent>::new())
    }

    fn wrong_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<WrongComponent>::new())
    }

    fn serialize_external(component: &dyn Any) -> BTreeMap<String, Value> {
        let component = component
            .downcast_ref::<ExternalComponent>()
            .expect("external component type");
        BTreeMap::from([("value".to_string(), Value::UInt(component.value))])
    }

    fn deserialize_external(fields: &BTreeMap<String, Value>) -> Box<dyn Any> {
        let value = match fields.get("value") {
            Some(Value::UInt(value)) => *value,
            _ => 0,
        };
        Box::new(ExternalComponent { value })
    }

    fn deserialize_wrong_type(_: &BTreeMap<String, Value>) -> Box<dyn Any> {
        Box::new(WrongComponent)
    }

    fn external_extension(
        deserialize: Option<crate::registry::DeserializeFn>,
    ) -> ComponentExtension {
        ComponentExtension {
            meta: ComponentMeta {
                type_id: ExternalComponent::TYPE_ID,
                display_name: "External",
                schema_version: (0, 1, 0),
                has_editor: false,
                has_script_binding: false,
            },
            storage_factory: external_storage,
            serialize: Some(serialize_external),
            deserialize,
        }
    }

    fn scene_with_component(type_id: &str) -> crate::Scene {
        let mut scene = sample_scene();
        scene.entities[0].components.insert(
            type_id.to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([("value".to_string(), Value::UInt(42))]),
            },
        );
        scene
    }

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
    fn world_scene_roundtrip_preserves_entity_enabled_state() {
        let mut scene = sample_scene();
        scene.entities[0].enabled = false;

        let world = World::from_scene(&scene);
        let entity = world
            .persistent_to_entity
            .get(&scene.entities[0].persistent_id)
            .copied()
            .expect("scene entity should be mapped");
        assert!(!world.is_enabled(entity));

        let roundtripped = world.to_scene();
        let record = roundtripped
            .entities
            .iter()
            .find(|record| record.persistent_id == scene.entities[0].persistent_id)
            .expect("disabled entity should remain serialized");
        assert!(!record.enabled);
    }

    #[test]
    fn world_to_scene_uses_recycled_entity_generation() {
        let mut world = World::from_scene(&sample_scene());
        let first_id = world.entity_to_persistent[0]
            .clone()
            .expect("first scene entity should have an id");
        let first = world.persistent_to_entity[&first_id];
        assert!(world.destroy_entity(first));

        let recycled = world.create_entity();
        assert_eq!(recycled.index(), first.index());
        assert_ne!(recycled.generation(), first.generation());
        let recycled_id = "recycled-entity".to_string();
        world.entity_to_persistent[recycled.index() as usize] = Some(recycled_id.clone());
        world
            .persistent_to_entity
            .insert(recycled_id.clone(), recycled);
        world.add_component(recycled, Name("Recycled".to_string()));

        let roundtripped = world.to_scene();
        assert!(roundtripped
            .entities
            .iter()
            .any(|record| record.persistent_id == recycled_id));
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

        let result = crate::extraction::extract_renderer_input_from_world(&world, 42);
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

    #[test]
    fn registry_aware_load_restores_and_roundtrips_external_component() {
        let scene = scene_with_component(ExternalComponent::TYPE_ID);
        let mut registry = ComponentRegistry::new();
        registry
            .register(external_extension(Some(deserialize_external)))
            .expect("register external component");
        let registry = Arc::new(registry);

        let report = World::from_scene_with_registry(&scene, Arc::clone(&registry));
        assert!(report.is_success(), "{:?}", report.diagnostics);
        let world = report.world;
        assert!(Arc::ptr_eq(
            world.component_registry().expect("registry installed"),
            &registry
        ));
        assert_eq!(
            world
                .query::<ExternalComponent>()
                .next()
                .map(|(_, c)| c.value),
            Some(42)
        );

        let roundtripped_scene = world.to_scene();
        let record = roundtripped_scene
            .entities
            .iter()
            .find_map(|entity| entity.components.get(ExternalComponent::TYPE_ID))
            .expect("external component serialized");
        assert_eq!(record.fields.get("value"), Some(&Value::UInt(42)));

        let restored = World::try_from_scene_with_registry(&roundtripped_scene, registry)
            .expect("roundtripped external component should load");
        assert_eq!(
            restored
                .query::<ExternalComponent>()
                .next()
                .map(|(_, c)| c.value),
            Some(42)
        );
    }

    #[test]
    fn strict_registry_load_rejects_unknown_component() {
        let scene = scene_with_component("test.unknown");
        let result =
            World::try_from_scene_with_registry(&scene, Arc::new(ComponentRegistry::new()));
        let error = match result {
            Ok(_) => panic!("unknown external component must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::UnknownComponent {
                component_type_id,
                ..
            }] if component_type_id == "test.unknown"
        ));
    }

    #[test]
    fn strict_registry_load_rejects_missing_deserialize_hook() {
        let scene = scene_with_component(ExternalComponent::TYPE_ID);
        let mut registry = ComponentRegistry::new();
        registry
            .register(external_extension(None))
            .expect("register external component");
        let result = World::try_from_scene_with_registry(&scene, Arc::new(registry));
        let error = match result {
            Ok(_) => panic!("missing deserialize hook must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::MissingDeserializeHook {
                component_type_id,
                ..
            }] if component_type_id == ExternalComponent::TYPE_ID
        ));
    }

    #[test]
    fn strict_registry_load_rejects_storage_insert_type_mismatch() {
        let scene = scene_with_component(ExternalComponent::TYPE_ID);
        let mut registry = ComponentRegistry::new();
        registry
            .register(external_extension(Some(deserialize_wrong_type)))
            .expect("register external component");
        let result = World::try_from_scene_with_registry(&scene, Arc::new(registry));
        let error = match result {
            Ok(_) => panic!("type-erased storage mismatch must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::StorageInsertTypeMismatch {
                component_type_id,
                ..
            }] if component_type_id == ExternalComponent::TYPE_ID
        ));
    }

    #[test]
    fn strict_registry_load_rejects_storage_factory_type_mismatch_before_traversal() {
        let scene = scene_with_component(ExternalComponent::TYPE_ID);
        let mut registry = ComponentRegistry::new();
        let mut extension = external_extension(Some(deserialize_external));
        extension.storage_factory = wrong_storage;
        registry
            .register(extension)
            .expect("register external component");
        let result = World::try_from_scene_with_registry(&scene, Arc::new(registry));
        let error = match result {
            Ok(_) => panic!("mismatched storage factory must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::StorageFactoryTypeMismatch {
                component_type_id,
                storage_type_id,
                ..
            }] if component_type_id == ExternalComponent::TYPE_ID
                && storage_type_id == WrongComponent::TYPE_ID
        ));
    }

    #[test]
    fn disabled_external_component_is_validated_but_not_instantiated() {
        let mut scene = scene_with_component(ExternalComponent::TYPE_ID);
        let original = scene.entities[0]
            .components
            .get_mut(ExternalComponent::TYPE_ID)
            .expect("external component");
        original.enabled = false;
        original.schema_version = SchemaVersion::new(2, 3, 4);
        let original = original.clone();

        let mut registry = ComponentRegistry::new();
        registry
            .register(external_extension(Some(deserialize_external)))
            .expect("register external component");
        let world = World::try_from_scene_with_registry(&scene, Arc::new(registry))
            .expect("known disabled component should validate");

        assert!(world.query::<ExternalComponent>().next().is_none());
        let roundtripped = world.to_scene();
        assert_eq!(
            roundtripped.entities[0]
                .components
                .get(ExternalComponent::TYPE_ID),
            Some(&original)
        );
    }

    #[test]
    fn strict_load_rejects_unknown_disabled_component() {
        let mut scene = scene_with_component("test.disabled_unknown");
        scene.entities[0]
            .components
            .get_mut("test.disabled_unknown")
            .expect("unknown component")
            .enabled = false;

        let result =
            World::try_from_scene_with_registry(&scene, Arc::new(ComponentRegistry::new()));
        let error = match result {
            Ok(_) => panic!("unknown disabled component must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::UnknownComponent {
                component_type_id,
                ..
            }] if component_type_id == "test.disabled_unknown"
        ));
    }

    #[test]
    fn non_strict_roundtrip_preserves_unknown_disabled_component() {
        let mut scene = scene_with_component("test.disabled_unknown");
        let original = scene.entities[0]
            .components
            .get_mut("test.disabled_unknown")
            .expect("unknown component");
        original.enabled = false;
        original.schema_version = SchemaVersion::new(7, 8, 9);
        let original = original.clone();

        let roundtripped = World::from_scene(&scene).to_scene();
        assert_eq!(
            roundtripped.entities[0]
                .components
                .get("test.disabled_unknown"),
            Some(&original)
        );
    }

    #[test]
    fn roundtrip_preserves_scene_metadata_and_enabled_component_schema() {
        let mut scene = sample_scene();
        scene.schema_version = SchemaVersion::new(0, 9, 7);
        scene.engine_version = "9.8.7-test".to_string();
        scene.diagnostics_policy = crate::scene::DiagnosticsPolicy::EditorRepair;
        scene.dependencies = vec![AssetId::new("kept-explicit-dependency")];
        let renderable = scene.entities[1]
            .components
            .get_mut(Renderable::TYPE_ID)
            .expect("renderable component");
        renderable.schema_version = SchemaVersion::new(3, 2, 1);

        let roundtripped = World::from_scene(&scene).to_scene();
        assert_eq!(roundtripped.schema_version, scene.schema_version);
        assert_eq!(roundtripped.engine_version, scene.engine_version);
        assert_eq!(roundtripped.dependencies, scene.dependencies);
        assert_eq!(roundtripped.diagnostics_policy, scene.diagnostics_policy);
        assert_eq!(
            roundtripped.entities[1].components[Renderable::TYPE_ID].schema_version,
            SchemaVersion::new(3, 2, 1)
        );
    }

    #[test]
    fn entity_record_parent_is_applied_and_preserved() {
        let mut scene = sample_scene();
        let parent_id = scene.entities[0].persistent_id.clone();
        scene.entities[1].parent = Some(parent_id.clone());
        scene.entities[1].components.insert(
            Transform::TYPE_ID.to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );

        let world = World::from_scene(&scene);
        let child = world
            .persistent_to_entity
            .get(&scene.entities[1].persistent_id)
            .copied()
            .expect("child entity");
        let parent = world
            .persistent_to_entity
            .get(&parent_id)
            .copied()
            .expect("parent entity");
        assert_eq!(
            world.get::<Transform>(child).and_then(|t| t.parent),
            Some(parent)
        );

        let roundtripped = world.to_scene();
        assert_eq!(roundtripped.entities[1].parent, Some(parent_id));
    }

    #[test]
    fn prefab_instance_linkage_survives_strict_scene_world_roundtrip() {
        let mut scene = sample_scene();
        scene.entities[1].components.insert(
            PrefabInstanceRef::TYPE_ID.to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    (
                        "source_asset".into(),
                        Value::Asset(AssetId::new("prefab-crate")),
                    ),
                    (
                        "instance_id".into(),
                        Value::Str("prefab-instance-crate".into()),
                    ),
                    (
                        "entity_persistent_id".into(),
                        Value::Str("crate-root".into()),
                    ),
                    ("schema_major".into(), Value::UInt(0)),
                    ("schema_minor".into(), Value::UInt(1)),
                    ("schema_patch".into(), Value::UInt(0)),
                ]),
            },
        );
        let mut registry = ComponentRegistry::new();
        registry.register_core();
        let world = World::try_from_scene_with_registry(&scene, Arc::new(registry)).unwrap();
        let linkage = world
            .query::<PrefabInstanceRef>()
            .next()
            .map(|(_, linkage)| linkage)
            .expect("prefab linkage materialized");
        assert_eq!(linkage.source_asset, "prefab-crate");
        assert_eq!(linkage.entity_persistent_id, "crate-root");

        let roundtripped = world.to_scene();
        assert_eq!(
            roundtripped.entities[1]
                .components
                .get(PrefabInstanceRef::TYPE_ID),
            scene.entities[1].components.get(PrefabInstanceRef::TYPE_ID)
        );
    }
}
