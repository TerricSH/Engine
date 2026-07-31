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
            for component_type_id in registry.singleton_types() {
                let mut owners = scene
                    .entities
                    .iter()
                    .filter(|entity| entity.components.contains_key(component_type_id));
                if let Some(first) = owners.next() {
                    diagnostics.extend(owners.map(|duplicate| {
                        SceneLoadDiagnostic::DuplicateSingletonComponent {
                            entity_id: duplicate.persistent_id.clone(),
                            first_entity_id: first.persistent_id.clone(),
                            component_type_id: component_type_id.to_string(),
                        }
                    }));
                }
            }
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

        // Preserve scene-level metadata.
        world.scene_settings = scene.scene_settings.clone();
        world.scene_schema_version = scene.schema_version;
        world.scene_engine_version = scene.engine_version.clone();
        world.scene_id = scene.scene_id.clone();
        world.scene_name = scene.name.clone();
        world.scene_dependencies = scene.dependencies.clone();
        world.diagnostics_policy = scene.diagnostics_policy;

        let (_created, mut insert_diagnostics) = world.insert_scene_entities(scene);
        diagnostics.append(&mut insert_diagnostics);
        SceneLoadReport { world, diagnostics }
    }

    /// Insert every entity of `scene` into this world using the two-pass
    /// allocate-then-populate flow shared by scene loading and live merging.
    ///
    /// The caller must guarantee that the scene's persistent IDs neither
    /// collide with existing world mappings nor repeat inside the scene
    /// itself: [`from_scene`](World::from_scene) loads into a fresh world,
    /// while [`World::merge_scene`] pre-validates conflicts before calling
    /// this. Returns the created entities in scene order together with any
    /// component-population diagnostics (only produced when a component
    /// registry is installed, mirroring [`build_from_scene`](Self::build_from_scene)).
    pub(crate) fn insert_scene_entities(
        &mut self,
        scene: &Scene,
    ) -> (Vec<Entity>, Vec<SceneLoadDiagnostic>) {
        let mut diagnostics = Vec::new();
        let strict_components = self.component_registry.is_some();
        let mut created = Vec::with_capacity(scene.entities.len());

        // First pass: allocate entities and record persistent_id mappings.
        for entity_record in &scene.entities {
            let entity = self.create_entity();
            created.push(entity);
            self.set_enabled(entity, entity_record.enabled);
            let idx = entity.index() as usize;
            // Record persistent_id mapping.
            self.persistent_to_entity
                .insert(entity_record.persistent_id.clone(), entity);
            if self.entity_to_persistent.len() <= idx {
                self.entity_to_persistent.resize(idx + 1, None);
            }
            self.entity_to_persistent[idx] = Some(entity_record.persistent_id.clone());
            self.entity_parents[idx] = entity_record.parent.clone();

            // Copy EntityRecord.name to a Name component.
            if let Some(ref name) = entity_record.name {
                self.add_component(entity, Name(name.clone()));
            }
        }

        // Second pass: populate components with resolved references.
        for entity_record in &scene.entities {
            let Some(&entity) = self.persistent_to_entity.get(&entity_record.persistent_id) else {
                continue;
            };

            for (comp_type_id, comp_record) in &entity_record.components {
                self.component_schema_versions[entity.index() as usize]
                    .insert(comp_type_id.clone(), comp_record.schema_version);
                if !comp_record.enabled {
                    if strict_components {
                        if let Err(diagnostic) = self.validate_disabled_component(
                            entity,
                            comp_type_id,
                            &comp_record.fields,
                        ) {
                            diagnostics.push(diagnostic);
                        }
                    }
                    self.disabled_components[entity.index() as usize]
                        .insert(comp_type_id.clone(), comp_record.clone());
                    continue;
                }
                if strict_components {
                    if let Err(diagnostic) =
                        self.populate_component_checked(entity, comp_type_id, &comp_record.fields)
                    {
                        diagnostics.push(diagnostic);
                    }
                } else {
                    self.populate_component(entity, comp_type_id, &comp_record.fields);
                }
            }

            // EntityRecord.parent is the canonical hierarchy field. Apply it
            // to an enabled Transform when present, while retaining support
            // for older scenes that encoded the parent only in Transform.
            if let Some(parent_id) = entity_record.parent.as_ref() {
                let parent = self.persistent_to_entity.get(parent_id).copied();
                if let Some(transform) = self.get_mut::<Transform>(entity) {
                    transform.parent = parent;
                }
            }
        }

        (created, diagnostics)
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
        registry
            .validate_fields(comp_type_id, fields)
            .map_err(|message| SceneLoadDiagnostic::InvalidComponentFields {
                entity_id: entity_id.clone(),
                component_type_id: comp_type_id.to_string(),
                message,
            })?;

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
            registry
                .validate_fields(comp_type_id, fields)
                .map_err(|message| SceneLoadDiagnostic::InvalidComponentFields {
                    entity_id: entity_id.clone(),
                    component_type_id: comp_type_id.to_string(),
                    message,
                })?;
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
    include!("scene/tests/common.rs");
    include!("scene/tests/roundtrip.rs");
    include!("scene/tests/registry.rs");
    include!("scene/tests/metadata.rs");
}
