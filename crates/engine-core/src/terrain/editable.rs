//! Runtime bridge for persistent density-field edits.

use std::collections::{BTreeMap, BTreeSet};

use engine_scene::components::{Bounds, Renderable, Transform};
use engine_terrain::{
    DensityChunkKey, DensityTerrainConfig, EditableTerrain, EditableTerrainMesh,
    TerrainBaseDensity, TerrainBrush, TerrainBrushFalloff, TerrainBrushMode, TerrainEditDelta,
    TerrainEditStore, TerrainVolume, TerrainVolumeId,
};
use glam::{Vec2, Vec3};

use super::{ActiveTerrainVolume, ChunkKey, ChunkRegion, TerrainSystem};
use crate::{EngineRuntime, RuntimeMeshDescriptor, RuntimeMeshHandle};

const EDITABLE_REBUILDS_PER_FRAME: usize = 4;

pub(super) type EditableChunkKey = (TerrainVolumeId, DensityChunkKey);

pub(super) struct EditableChunkBinding {
    pub(super) mesh: RuntimeMeshHandle,
    pub(super) entity: engine_scene::Entity,
    pub(super) origin: [f64; 3],
}

pub(super) struct EditableVolume {
    pub(super) terrain: EditableTerrain,
    pub(super) base_density: TerrainBaseDensity,
    pub(super) material: String,
    pub(super) store: Option<TerrainEditStore>,
    pub(super) planet_center: Option<[f64; 3]>,
}

impl TerrainSystem {
    /// Configure append-only edited-density persistence. Each terrain volume
    /// receives a stable subdirectory derived from its persistent identity.
    pub fn set_edit_store_root(&mut self, root: impl Into<std::path::PathBuf>) {
        self.edit_store_root = Some(root.into());
    }

    /// Apply a local density brush and publish a bounded set of affected
    /// render/collision/navigation chunks. The edit is persisted before this
    /// returns.
    pub fn apply_brush(
        &mut self,
        engine: &mut EngineRuntime,
        terrain_entity_id: &str,
        brush: TerrainBrush,
    ) -> Result<TerrainEditDelta, String> {
        let Some((volume, world_origin)) = engine
            .with_world(|world| {
                let entity = world.entity_by_persistent_id(terrain_entity_id)?;
                let volume = world.get::<TerrainVolume>(entity)?.clone();
                Some((volume, world.world_origin()))
            })
            .flatten()
        else {
            return Err(format!(
                "terrain entity '{terrain_entity_id}' does not exist or has no TerrainVolume"
            ));
        };
        volume.validate().map_err(|error| error.to_string())?;
        let volume_id = TerrainVolumeId::from_persistent_id(terrain_entity_id);
        self.ensure_editable_volume(volume_id, &volume)?;

        let delta = {
            let editable = self
                .editable_volumes
                .get_mut(&volume_id)
                .ok_or_else(|| "editable terrain state was not created".to_string())?;
            let base = editable.base_density;
            let delta = editable
                .terrain
                .apply_brush(&brush, |point| base.sample(point))
                .map_err(|error| error.to_string())?;
            if let Some(store) = &editable.store {
                store
                    .save_modified(&mut editable.terrain)
                    .map_err(|error| error.to_string())?;
            }
            delta
        };
        self.queue_base_replacement_chunks(Some((
            volume_id,
            ChunkRegion {
                min: delta.bounds_min,
                max: delta.bounds_max,
            },
        )));
        self.drain_editable_rebuilds(engine, world_origin, EDITABLE_REBUILDS_PER_FRAME)?;
        self.reconcile_base_edit_suppression(engine);
        Ok(delta)
    }

    /// Restore persistent edit state, finish bounded mesh work and reconcile
    /// density replacements with the currently selected base-terrain LOD.
    /// Returns true when physics bindings or floating-origin transforms changed.
    pub(super) fn tick_editable_terrain(
        &mut self,
        engine: &mut EngineRuntime,
        active_volumes: &BTreeMap<TerrainVolumeId, ActiveTerrainVolume>,
        world_origin: [f64; 3],
    ) -> bool {
        let stale_ids = self
            .editable_volumes
            .keys()
            .copied()
            .filter(|volume_id| !active_volumes.contains_key(volume_id))
            .collect::<Vec<_>>();
        for volume_id in stale_ids {
            let keys = self
                .editable_chunks
                .keys()
                .copied()
                .filter(|key| key.0 == volume_id)
                .collect::<Vec<_>>();
            for key in keys {
                self.remove_editable_binding(engine, key);
            }
            self.editable_coverage.retain(|key, _| key.0 != volume_id);
            self.editable_volumes.remove(&volume_id);
        }

        if self.edit_store_root.is_some() {
            for (volume_id, active) in active_volumes {
                if active.persistent_id.is_none() {
                    continue;
                }
                if let Err(error) = self.ensure_editable_volume(*volume_id, &active.volume) {
                    self.stats.last_error = Some(format!(
                        "cannot restore editable terrain {:016x}: {error}",
                        volume_id.get()
                    ));
                }
            }
        }

        self.queue_base_replacement_chunks(None);
        let mut changed =
            match self.drain_editable_rebuilds(engine, world_origin, EDITABLE_REBUILDS_PER_FRAME) {
                Ok(changed) => changed,
                Err(error) => {
                    self.stats.last_error = Some(error);
                    false
                }
            };
        changed |= self.update_editable_origins(engine, world_origin);
        changed |= self.reconcile_base_edit_suppression(engine);
        changed
    }

    fn ensure_editable_volume(
        &mut self,
        volume_id: TerrainVolumeId,
        volume: &TerrainVolume,
    ) -> Result<(), String> {
        if self.editable_volumes.contains_key(&volume_id) {
            return Ok(());
        }
        let sample_spacing = f64::from(volume.chunk_size)
            / f64::from(volume.base_resolution.saturating_sub(1).max(1));
        let config = DensityTerrainConfig {
            voxel_size: sample_spacing.max(0.01),
            chunk_cells: 16,
            iso_level: 0.0,
            default_density: -1.0,
        };
        let base_density = TerrainBaseDensity::new(volume)?;
        let store = self
            .edit_store_root
            .as_ref()
            .map(|root| TerrainEditStore::new(root.join(format!("{:016x}", volume_id.get()))));
        let loaded = store
            .as_ref()
            .map(TerrainEditStore::load_latest_report)
            .transpose()
            .map_err(|error| error.to_string())?;
        if let Some(issue) = loaded
            .as_ref()
            .and_then(|report| report.skipped_revisions.first())
        {
            tracing::warn!(
                terrain_volume = volume_id.get(),
                skipped_revisions = loaded
                    .as_ref()
                    .map_or(0, |report| report.skipped_revisions.len()),
                first_path = %issue.path.display(),
                first_error = %issue.error,
                "recovered editable terrain by ignoring corrupt revision files"
            );
        }
        let terrain = match loaded.and_then(|report| report.terrain) {
            Some(restored) if restored.config() == &config => restored,
            Some(_) => {
                return Err(
                    "persisted terrain edit voxel configuration no longer matches the volume"
                        .into(),
                );
            }
            None => EditableTerrain::new(config).map_err(|error| error.to_string())?,
        };
        let material = if volume.material_asset.is_empty() {
            "mat-default".to_string()
        } else {
            volume.material_asset.clone()
        };
        self.editable_volumes.insert(
            volume_id,
            EditableVolume {
                terrain,
                base_density,
                material,
                store,
                planet_center: (volume.topology == engine_terrain::TerrainTopology::CubeSphere)
                    .then_some(volume.planet_center),
            },
        );
        Ok(())
    }

    fn queue_base_replacement_chunks(
        &mut self,
        immediate_edit: Option<(TerrainVolumeId, ChunkRegion)>,
    ) {
        const MAX_REPLACEMENT_CHUNKS_PER_BASE_PATCH: usize = 4_096;

        let edited_regions = self
            .editable_volumes
            .iter()
            .map(|(volume_id, editable)| {
                let config = editable.terrain.config().clone();
                let regions = editable
                    .terrain
                    .materialized_chunk_keys()
                    .map(|key| density_chunk_region(key, &config))
                    .collect::<Vec<_>>();
                (*volume_id, (config, regions))
            })
            .collect::<BTreeMap<_, _>>();
        let mut requested = BTreeMap::<TerrainVolumeId, BTreeSet<DensityChunkKey>>::new();
        for ((chunk_id, _), binding) in &self.chunks {
            let volume_id = chunk_id.volume_id;
            let Some((config, persisted_regions)) = edited_regions.get(&volume_id) else {
                continue;
            };
            let overlaps_edit = match immediate_edit {
                Some((edited_volume, region)) => {
                    edited_volume == volume_id && binding.region.overlaps(region)
                }
                None => persisted_regions
                    .iter()
                    .any(|region| binding.region.overlaps(*region)),
            };
            if !overlaps_edit {
                continue;
            }
            let Some(keys) = density_keys_covering_region(
                binding.region,
                config,
                MAX_REPLACEMENT_CHUNKS_PER_BASE_PATCH,
            ) else {
                tracing::debug!(
                    ?chunk_id,
                    "editable replacement exceeds the bounded density coverage budget; retaining the base terrain patch"
                );
                continue;
            };
            requested.entry(volume_id).or_default().extend(keys);
        }
        for (volume_id, keys) in requested {
            let Some(editable) = self.editable_volumes.get_mut(&volume_id) else {
                continue;
            };
            for key in keys {
                if !self.editable_coverage.contains_key(&(volume_id, key)) {
                    editable.terrain.request_mesh_rebuild(key);
                }
            }
        }
    }

    fn drain_editable_rebuilds(
        &mut self,
        engine: &mut EngineRuntime,
        world_origin: [f64; 3],
        budget: usize,
    ) -> Result<bool, String> {
        let volume_ids = self.editable_volumes.keys().copied().collect::<Vec<_>>();
        let mut blocked = BTreeSet::new();
        let mut remaining = budget.max(1);
        let mut changed = false;
        let mut first_error = None;
        while remaining > 0 {
            let mut progressed = false;
            for volume_id in &volume_ids {
                if remaining == 0 || blocked.contains(volume_id) {
                    continue;
                }
                let Some((key, mesh, material)) = self
                    .editable_volumes
                    .get_mut(volume_id)
                    .and_then(|editable| {
                        let key = editable.terrain.take_dirty_mesh_chunks(1).pop()?;
                        let base = editable.base_density;
                        let mesh = editable
                            .terrain
                            .build_chunk_mesh(key, |point| base.sample(point));
                        Some((key, mesh, editable.material.clone()))
                    })
                else {
                    blocked.insert(*volume_id);
                    continue;
                };
                progressed = true;
                remaining -= 1;
                match self.commit_editable_mesh(engine, *volume_id, mesh, &material, world_origin) {
                    Ok(()) => changed = true,
                    Err(error) => {
                        if let Some(editable) = self.editable_volumes.get_mut(volume_id) {
                            editable.terrain.requeue_mesh_chunk(key);
                        }
                        blocked.insert(*volume_id);
                        first_error.get_or_insert(error);
                    }
                }
            }
            if !progressed || blocked.len() == volume_ids.len() {
                break;
            }
        }
        first_error.map_or(Ok(changed), Err)
    }

    fn commit_editable_mesh(
        &mut self,
        engine: &mut EngineRuntime,
        volume_id: TerrainVolumeId,
        data: EditableTerrainMesh,
        material: &str,
        world_origin: [f64; 3],
    ) -> Result<(), String> {
        let key = (volume_id, data.key);
        let region = self
            .editable_volumes
            .get(&volume_id)
            .map(|volume| density_chunk_region(data.key, volume.terrain.config()))
            .ok_or_else(|| "editable terrain state disappeared during mesh commit".to_string())?;
        if data.mesh.indices.is_empty() {
            self.remove_editable_binding(engine, key);
            self.editable_coverage.insert(key, region);
            return Ok(());
        }

        let descriptor = RuntimeMeshDescriptor {
            positions: data
                .mesh
                .positions
                .iter()
                .copied()
                .map(Vec3::from_array)
                .collect(),
            normals: data
                .mesh
                .normals
                .iter()
                .copied()
                .map(Vec3::from_array)
                .collect(),
            uvs: data
                .mesh
                .uvs
                .iter()
                .copied()
                .map(Vec2::from_array)
                .collect(),
            indices: data.mesh.indices.clone(),
            bounds: Some((
                Vec3::from_array(data.mesh.bounds_min),
                Vec3::from_array(data.mesh.bounds_max),
            )),
        };
        if let Some(binding) = self.editable_chunks.get(&key) {
            engine
                .update_runtime_mesh(binding.mesh, descriptor)
                .map_err(|error| error.to_string())?;
            let updated = engine.with_world_mut(|world| {
                let min = Vec3::from_array(data.mesh.bounds_min);
                let max = Vec3::from_array(data.mesh.bounds_max);
                if let Some(transform) = world.get_mut::<Transform>(binding.entity) {
                    transform.translation = Vec3::new(
                        (data.origin[0] - world_origin[0]) as f32,
                        (data.origin[1] - world_origin[1]) as f32,
                        (data.origin[2] - world_origin[2]) as f32,
                    );
                }
                if let Some(bounds) = world.get_mut::<Bounds>(binding.entity) {
                    bounds.center = ((min + max) * 0.5).to_array();
                    bounds.half_extents = ((max - min) * 0.5).to_array();
                }
                #[cfg(feature = "subsystem-physics")]
                if let Some(collider) = world.get_mut::<engine_physics::Collider>(binding.entity) {
                    collider.shape = engine_physics::ColliderShape::TriMesh {
                        vertices: data.collision.positions.clone(),
                        indices: data.collision.triangles.clone(),
                    };
                }
            });
            if updated.is_none() {
                return Err("cannot update editable terrain without an active World".into());
            }
            if let Some(binding) = self.editable_chunks.get_mut(&key) {
                binding.origin = data.origin;
            }
            self.editable_coverage.insert(key, region);
            #[cfg(feature = "subsystem-navigation")]
            self.rebuild_editable_navigation(volume_id, &data);
            return Ok(());
        }

        self.create_editable_binding(engine, volume_id, data, material, world_origin)?;
        self.editable_coverage.insert(key, region);
        Ok(())
    }

    fn create_editable_binding(
        &mut self,
        engine: &mut EngineRuntime,
        volume_id: TerrainVolumeId,
        data: EditableTerrainMesh,
        material: &str,
        world_origin: [f64; 3],
    ) -> Result<(), String> {
        let key = (volume_id, data.key);
        let logical_origin = data.origin;
        let descriptor = RuntimeMeshDescriptor {
            positions: data
                .mesh
                .positions
                .iter()
                .copied()
                .map(Vec3::from_array)
                .collect(),
            normals: data
                .mesh
                .normals
                .iter()
                .copied()
                .map(Vec3::from_array)
                .collect(),
            uvs: data
                .mesh
                .uvs
                .iter()
                .copied()
                .map(Vec2::from_array)
                .collect(),
            indices: data.mesh.indices.clone(),
            bounds: Some((
                Vec3::from_array(data.mesh.bounds_min),
                Vec3::from_array(data.mesh.bounds_max),
            )),
        };
        let name = format!(
            "terrain-edit-{:016x}-{}-{}-{}",
            volume_id.get(),
            data.key.x,
            data.key.y,
            data.key.z
        );
        let mesh = engine
            .create_runtime_mesh(&name, descriptor)
            .map_err(|error| error.to_string())?;
        let Some(mesh_asset) = engine.runtime_mesh_asset_id(mesh).map(|id| id.id) else {
            let _ = engine.destroy_runtime_mesh(mesh);
            return Err("editable terrain mesh has no registered asset ID".into());
        };
        let relative_origin = Vec3::new(
            (data.origin[0] - world_origin[0]) as f32,
            (data.origin[1] - world_origin[1]) as f32,
            (data.origin[2] - world_origin[2]) as f32,
        );
        let Some(entity) = engine.with_world_mut(|world| {
            let entity = world.create_entity();
            world.add_component(
                entity,
                Transform {
                    translation: relative_origin,
                    ..Transform::default()
                },
            );
            world.add_component(
                entity,
                Renderable {
                    mesh_asset,
                    material_asset: material.to_string(),
                    visible: true,
                    cast_shadows: true,
                    render_layer: "default".into(),
                },
            );
            let min = Vec3::from_array(data.mesh.bounds_min);
            let max = Vec3::from_array(data.mesh.bounds_max);
            world.add_component(
                entity,
                Bounds {
                    center: ((min + max) * 0.5).to_array(),
                    half_extents: ((max - min) * 0.5).to_array(),
                },
            );
            #[cfg(feature = "subsystem-physics")]
            {
                world.add_component(
                    entity,
                    engine_physics::RigidBody {
                        body_type: engine_physics::BodyType::Static,
                        gravity_scale: 0.0,
                        ..engine_physics::RigidBody::default()
                    },
                );
                world.add_component(
                    entity,
                    engine_physics::Collider {
                        shape: engine_physics::ColliderShape::TriMesh {
                            vertices: data.collision.positions.clone(),
                            indices: data.collision.triangles.clone(),
                        },
                        ..engine_physics::Collider::default()
                    },
                );
            }
            entity
        }) else {
            let _ = engine.destroy_runtime_mesh(mesh);
            return Err("cannot create editable terrain without an active World".into());
        };
        self.editable_chunks.insert(
            key,
            EditableChunkBinding {
                mesh,
                entity,
                origin: logical_origin,
            },
        );
        #[cfg(feature = "subsystem-navigation")]
        self.rebuild_editable_navigation(volume_id, &data);
        Ok(())
    }

    fn update_editable_origins(
        &mut self,
        engine: &mut EngineRuntime,
        world_origin: [f64; 3],
    ) -> bool {
        let bindings = self
            .editable_chunks
            .values()
            .map(|binding| (binding.entity, binding.origin))
            .collect::<Vec<_>>();
        engine
            .with_world_mut(|world| {
                let mut changed = false;
                for (entity, origin) in bindings {
                    let Some(transform) = world.get_mut::<Transform>(entity) else {
                        continue;
                    };
                    let relative = Vec3::new(
                        (origin[0] - world_origin[0]) as f32,
                        (origin[1] - world_origin[1]) as f32,
                        (origin[2] - world_origin[2]) as f32,
                    );
                    if transform.translation != relative {
                        transform.translation = relative;
                        changed = true;
                    }
                }
                changed
            })
            .unwrap_or(false)
    }

    fn reconcile_base_edit_suppression(&mut self, engine: &mut EngineRuntime) -> bool {
        const MAX_REPLACEMENT_CHUNKS_PER_BASE_PATCH: usize = 4_096;
        let desired = self
            .chunks
            .iter()
            .map(|(key, binding)| {
                let suppressed =
                    self.editable_volumes
                        .get(&key.0.volume_id)
                        .is_some_and(|editable| {
                            density_region_is_covered(
                                &self.editable_coverage,
                                key.0.volume_id,
                                binding.region,
                                editable.terrain.config(),
                                MAX_REPLACEMENT_CHUNKS_PER_BASE_PATCH,
                            )
                        });
                (*key, suppressed)
            })
            .collect::<Vec<_>>();
        desired
            .into_iter()
            .fold(false, |changed, (key, suppressed)| {
                self.set_base_edit_suppressed(engine, key, suppressed) || changed
            })
    }

    fn set_base_edit_suppressed(
        &mut self,
        engine: &mut EngineRuntime,
        key: ChunkKey,
        suppressed: bool,
    ) -> bool {
        let Some(binding) = self.chunks.get(&key) else {
            return false;
        };
        if binding.edit_suppressed == suppressed {
            return false;
        }
        let entity = binding.entity;
        let active = binding.active;
        #[cfg(feature = "subsystem-physics")]
        let collision = binding.collision.clone();
        #[cfg(feature = "subsystem-physics")]
        let collision_span = binding.collision_span;
        #[cfg(feature = "subsystem-physics")]
        let triangle_collision = binding.triangle_collision.clone();
        let updated = engine
            .with_world_mut(|world| {
                let Some(renderable) = world.get_mut::<Renderable>(entity) else {
                    return false;
                };
                renderable.visible = active && !suppressed;
                #[cfg(feature = "subsystem-physics")]
                if suppressed || !active {
                    world.remove_component::<engine_physics::Collider>(entity);
                    world.remove_component::<engine_physics::RigidBody>(entity);
                } else if let Some(collision) = collision {
                    world.add_component(
                        entity,
                        engine_physics::RigidBody {
                            body_type: engine_physics::BodyType::Static,
                            gravity_scale: 0.0,
                            ..engine_physics::RigidBody::default()
                        },
                    );
                    world.add_component(
                        entity,
                        engine_physics::Collider {
                            shape: engine_physics::ColliderShape::HeightField {
                                rows: collision.rows,
                                columns: collision.columns,
                                heights: collision.heights,
                                scale: [collision_span, 1.0, collision_span],
                            },
                            ..engine_physics::Collider::default()
                        },
                    );
                } else if let Some(collision) = triangle_collision {
                    world.add_component(
                        entity,
                        engine_physics::RigidBody {
                            body_type: engine_physics::BodyType::Static,
                            gravity_scale: 0.0,
                            ..engine_physics::RigidBody::default()
                        },
                    );
                    world.add_component(
                        entity,
                        engine_physics::Collider {
                            shape: engine_physics::ColliderShape::TriMesh {
                                vertices: collision.positions,
                                indices: collision.triangles,
                            },
                            ..engine_physics::Collider::default()
                        },
                    );
                }
                true
            })
            .unwrap_or(false);
        if updated {
            if let Some(binding) = self.chunks.get_mut(&key) {
                binding.edit_suppressed = suppressed;
            }
        }
        updated
    }

    #[cfg(feature = "subsystem-navigation")]
    fn rebuild_editable_navigation(
        &mut self,
        volume_id: TerrainVolumeId,
        data: &EditableTerrainMesh,
    ) {
        let key = engine_nav::DynamicNavTileKey::new(
            volume_id.get(),
            [data.key.x, data.key.y, data.key.z],
        );
        let up = self
            .editable_volumes
            .get(&volume_id)
            .and_then(|volume| volume.planet_center)
            .map(|center| {
                let mesh_center = std::array::from_fn(|axis| {
                    data.origin[axis]
                        + f64::from((data.mesh.bounds_min[axis] + data.mesh.bounds_max[axis]) * 0.5)
                });
                (glam::DVec3::from_array(mesh_center) - glam::DVec3::from_array(center))
                    .normalize_or_zero()
                    .as_vec3()
            })
            .filter(|up| up.length_squared() > f32::EPSILON)
            .unwrap_or(Vec3::Y);
        let config = engine_nav::DynamicNavBuildConfig {
            up,
            ..engine_nav::DynamicNavBuildConfig::default()
        };
        if let Err(error) = self.editable_navigation.rebuild(
            key,
            data.origin,
            data.revision,
            &data.collision.positions,
            &data.collision.triangles,
            &config,
        ) {
            tracing::warn!(?key, %error, "editable terrain navigation rebuild failed; retaining previous tile");
        }
    }

    pub(super) fn remove_editable_binding(
        &mut self,
        engine: &mut EngineRuntime,
        key: EditableChunkKey,
    ) {
        #[cfg(feature = "subsystem-navigation")]
        self.editable_navigation
            .remove(engine_nav::DynamicNavTileKey::new(
                key.0.get(),
                [key.1.x, key.1.y, key.1.z],
            ));
        let Some(binding) = self.editable_chunks.remove(&key) else {
            return;
        };
        let _ = engine.with_world_mut(|world| world.destroy_entity(binding.entity));
        let _ = engine.destroy_runtime_mesh(binding.mesh);
    }

    /// Navigation tiles generated from the latest successfully rebuilt
    /// editable terrain chunks.
    #[cfg(feature = "subsystem-navigation")]
    pub fn editable_navigation(&self) -> &engine_nav::DynamicNavTileSet {
        &self.editable_navigation
    }
}

fn density_chunk_region(key: DensityChunkKey, config: &DensityTerrainConfig) -> ChunkRegion {
    let min = key.origin(config);
    let span = config.voxel_size * f64::from(config.chunk_cells);
    ChunkRegion {
        min,
        max: std::array::from_fn(|axis| min[axis] + span),
    }
}

fn density_keys_covering_region(
    region: ChunkRegion,
    config: &DensityTerrainConfig,
    limit: usize,
) -> Option<Vec<DensityChunkKey>> {
    let span = config.voxel_size * f64::from(config.chunk_cells);
    if !span.is_finite()
        || span <= 0.0
        || region.min.iter().any(|value| !value.is_finite())
        || region.max.iter().any(|value| !value.is_finite())
        || (0..3).any(|axis| region.min[axis] >= region.max[axis])
    {
        return None;
    }
    let mut min_key = [0_i64; 3];
    let mut max_key = [0_i64; 3];
    for axis in 0..3 {
        let min = (region.min[axis] / span).floor();
        let max = (region.max[axis] / span).ceil() - 1.0;
        if min < i64::MIN as f64
            || min > i64::MAX as f64
            || max < i64::MIN as f64
            || max > i64::MAX as f64
        {
            return None;
        }
        min_key[axis] = min as i64;
        max_key[axis] = max as i64;
    }
    let count = (0..3).try_fold(1_u64, |count, axis| {
        count.checked_mul(max_key[axis].abs_diff(min_key[axis]).saturating_add(1))
    })?;
    if count > limit as u64 {
        return None;
    }
    let mut keys = Vec::with_capacity(count as usize);
    for z in min_key[2]..=max_key[2] {
        for y in min_key[1]..=max_key[1] {
            for x in min_key[0]..=max_key[0] {
                keys.push(DensityChunkKey::new(x, y, z));
            }
        }
    }
    Some(keys)
}

fn density_region_is_covered(
    coverage: &BTreeMap<EditableChunkKey, ChunkRegion>,
    volume_id: TerrainVolumeId,
    region: ChunkRegion,
    config: &DensityTerrainConfig,
    limit: usize,
) -> bool {
    density_keys_covering_region(region, config, limit).is_some_and(|keys| {
        !keys.is_empty()
            && keys
                .iter()
                .all(|key| coverage.contains_key(&(volume_id, *key)))
    })
}

#[cfg(feature = "subsystem-scripting-csharp")]
impl crate::game_loop::GameLoop {
    pub(crate) fn process_script_terrain_brushes(&mut self) {
        let requests = self.runtime.take_pending_terrain_brushes();
        if requests.is_empty() {
            return;
        }
        let mut physics_changed = false;
        for request in requests {
            let mode = match request.brush.mode {
                engine_script::GameplayTerrainBrushMode::Add => TerrainBrushMode::Add,
                engine_script::GameplayTerrainBrushMode::Subtract => TerrainBrushMode::Subtract,
                engine_script::GameplayTerrainBrushMode::Smooth => TerrainBrushMode::Smooth,
                engine_script::GameplayTerrainBrushMode::SetDensity => {
                    TerrainBrushMode::SetDensity(request.brush.target_density)
                }
            };
            let brush = TerrainBrush {
                center: request.brush.center,
                radius: request.brush.radius,
                strength: request.brush.strength,
                falloff: TerrainBrushFalloff::Smooth,
                mode,
                material: request.brush.material,
            };
            let result =
                self.terrain
                    .apply_brush(&mut self.runtime, &request.terrain_entity_id, brush);
            physics_changed |= result.is_ok();
            self.runtime.push_runtime_asset_result(
                request.owner_entity_id,
                engine_script::GameplayRuntimeAssetResult {
                    request_id: request.request_id,
                    asset_id: request.terrain_entity_id,
                    success: result.is_ok(),
                    error: result.err(),
                },
            );
        }
        #[cfg(feature = "subsystem-physics")]
        if physics_changed {
            self.resync_physics_from_world();
        }
        #[cfg(not(feature = "subsystem-physics"))]
        let _ = physics_changed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_coverage_uses_exact_exclusive_boundaries_at_negative_coordinates() {
        let config = DensityTerrainConfig::default();
        let region = ChunkRegion {
            min: [-16.0, 0.0, 0.0],
            max: [0.0, 16.0, 16.0],
        };
        assert_eq!(
            density_keys_covering_region(region, &config, 8).unwrap(),
            vec![DensityChunkKey::new(-1, 0, 0)]
        );
    }

    #[test]
    fn base_patch_waits_for_every_replacement_chunk() {
        let config = DensityTerrainConfig::default();
        let volume_id = TerrainVolumeId::from_persistent_id("planet");
        let region = ChunkRegion {
            min: [0.0; 3],
            max: [32.0, 16.0, 16.0],
        };
        let keys = density_keys_covering_region(region, &config, 8).unwrap();
        assert_eq!(keys.len(), 2);
        let mut coverage = BTreeMap::new();
        coverage.insert((volume_id, keys[0]), density_chunk_region(keys[0], &config));
        assert!(!density_region_is_covered(
            &coverage, volume_id, region, &config, 8
        ));
        coverage.insert((volume_id, keys[1]), density_chunk_region(keys[1], &config));
        assert!(density_region_is_covered(
            &coverage, volume_id, region, &config, 8
        ));
    }
}
