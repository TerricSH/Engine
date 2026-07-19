//! Host bridge from `engine-terrain` CPU output to runtime meshes and ECS.

use std::collections::{BTreeMap, BTreeSet};

use engine_scene::components::{Bounds, Renderable, Transform};
#[cfg(feature = "gameplay")]
use engine_terrain::TerrainCollisionData;
use engine_terrain::{
    chunk_span, desired_chunks_hysteretic, HeightfieldGenerator, TerrainChunkData, TerrainChunkId,
    TerrainDebugSnapshot, TerrainRuntime, TerrainRuntimeConfig, TerrainRuntimeEvent, TerrainVolume,
};
use glam::{Vec2, Vec3};

use crate::{EngineRuntime, RuntimeMeshDescriptor, RuntimeMeshHandle};

struct ChunkBinding {
    mesh: RuntimeMeshHandle,
    entity: engine_scene::Entity,
    region: ChunkRegion,
    #[cfg(feature = "gameplay")]
    collision: Option<TerrainCollisionData>,
    #[cfg(feature = "gameplay")]
    collision_span: f32,
    active: bool,
}

impl ChunkBinding {
    #[cfg(feature = "gameplay")]
    fn release_staged_collision(&mut self) {
        self.collision = None;
    }

    #[cfg(not(feature = "gameplay"))]
    fn release_staged_collision(&mut self) {}
}

type ChunkKey = (TerrainChunkId, u64);

#[derive(Clone, Copy, Debug)]
struct ChunkRegion {
    min_x: f64,
    min_z: f64,
    max_x: f64,
    max_z: f64,
}

impl ChunkRegion {
    fn from_request(id: TerrainChunkId, volume: &TerrainVolume) -> Self {
        let span = chunk_span(volume, id.lod);
        let min_x = id.x as f64 * span;
        let min_z = id.z as f64 * span;
        Self {
            min_x,
            min_z,
            max_x: min_x + span,
            max_z: min_z + span,
        }
    }

    fn overlaps(self, other: Self) -> bool {
        self.min_x < other.max_x
            && other.min_x < self.max_x
            && self.min_z < other.max_z
            && other.min_z < self.max_z
    }
}

fn binding_can_retire(
    key: ChunkKey,
    region: ChunkRegion,
    desired: &BTreeMap<ChunkKey, ChunkRegion>,
    committed: &BTreeSet<ChunkKey>,
) -> bool {
    if desired.contains_key(&key) {
        return false;
    }
    let mut replacements = desired
        .iter()
        .filter_map(|(desired_key, desired_region)| {
            region.overlaps(*desired_region).then_some(*desired_key)
        })
        .peekable();
    replacements.peek().is_none()
        || replacements.all(|replacement| committed.contains(&replacement))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerrainBindingStats {
    pub live_chunks: usize,
    pub mesh_failures: u64,
    pub generation_failures: u64,
    pub last_error: Option<String>,
}

/// Optional terrain subsystem owned by [`crate::game_loop::GameLoop`].
///
/// Generation stays in `engine-terrain`; this adapter performs the bounded
/// main-thread commit to the existing ENG-20 mesh path and creates transient
/// ECS entities. It never uses world-partition cell IDs.
pub struct TerrainSystem {
    runtime: TerrainRuntime<HeightfieldGenerator>,
    chunks: BTreeMap<ChunkKey, ChunkBinding>,
    previous_desired: BTreeSet<TerrainChunkId>,
    stats: TerrainBindingStats,
    regeneration_epoch: u64,
}

impl Default for TerrainSystem {
    fn default() -> Self {
        Self::new(TerrainRuntimeConfig::default())
    }
}

impl TerrainSystem {
    pub fn new(config: TerrainRuntimeConfig) -> Self {
        Self {
            runtime: TerrainRuntime::new(HeightfieldGenerator, config),
            chunks: BTreeMap::new(),
            previous_desired: BTreeSet::new(),
            stats: TerrainBindingStats::default(),
            regeneration_epoch: 0,
        }
    }

    /// Drive selection, background completion and main-thread commit once.
    /// `focus_logical` is an optional absolute/logical position; when absent,
    /// the active camera plus the current floating origin is used.
    /// Returns true when ECS physics bindings changed.
    pub fn tick(&mut self, engine: &mut EngineRuntime, focus_logical: Option<[f64; 3]>) -> bool {
        let selected = engine.with_world(|world| {
            let mut volumes = world
                .query::<TerrainVolume>()
                .filter(|(_, volume)| volume.enabled)
                .map(|(entity, volume)| {
                    (
                        world
                            .persistent_id(entity)
                            .unwrap_or("<runtime-entity>")
                            .to_string(),
                        volume.clone(),
                    )
                })
                .collect::<Vec<_>>();
            volumes.sort_by(|left, right| left.0.cmp(&right.0));
            let origin = world.world_origin();
            let camera = engine_scene::active_camera_world_position(world).unwrap_or(Vec3::ZERO);
            (volumes, origin, camera)
        });
        let Some((volumes, world_origin, camera_relative)) = selected else {
            return false;
        };
        let volume = match volumes.as_slice() {
            [] => None,
            [(_, volume)] => Some(volume.clone()),
            _ => {
                self.stats.last_error = Some(format!(
                    "multiple enabled TerrainVolume components are not allowed: {}",
                    volumes
                        .iter()
                        .map(|(entity_id, _)| entity_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                None
            }
        };
        if volumes.len() <= 1
            && self
                .stats
                .last_error
                .as_deref()
                .is_some_and(|message| message.starts_with("multiple enabled TerrainVolume"))
        {
            self.stats.last_error = None;
        }
        let focus = focus_logical.unwrap_or([
            world_origin[0] + f64::from(camera_relative.x),
            world_origin[1] + f64::from(camera_relative.y),
            world_origin[2] + f64::from(camera_relative.z),
        ]);
        let (desired, material) = if let Some(volume) = volume {
            let mut desired = match volume.validate() {
                Ok(()) => {
                    if self.stats.last_error.as_deref().is_some_and(|message| {
                        message.starts_with("invalid terrain configuration:")
                            || message.starts_with("multiple enabled TerrainVolume")
                    }) {
                        self.stats.last_error = None;
                    }
                    desired_chunks_hysteretic(&volume, [focus[0], focus[2]], &self.previous_desired)
                }
                Err(error) => {
                    self.stats.last_error = Some(format!("invalid terrain configuration: {error}"));
                    Vec::new()
                }
            };
            for request in &mut desired {
                request.revision ^= self.regeneration_epoch;
            }
            let material = if volume.material_asset.is_empty() {
                "mat-default".to_string()
            } else {
                volume.material_asset
            };
            (desired, material)
        } else {
            (Vec::new(), "mat-default".to_string())
        };
        let next_desired = desired
            .iter()
            .map(|request| request.id)
            .collect::<BTreeSet<_>>();
        let desired_regions = desired
            .iter()
            .map(|request| {
                (
                    (request.id, request.revision),
                    ChunkRegion::from_request(request.id, &request.volume),
                )
            })
            .collect::<BTreeMap<_, _>>();
        self.previous_desired = next_desired;

        let mut events = self.runtime.set_desired(desired);
        events.extend(self.runtime.tick());
        let mut physics_changed = false;
        for event in events {
            match event {
                TerrainRuntimeEvent::Ready(data) => {
                    let key = (data.id, data.revision);
                    let region = desired_regions.get(&key).copied().unwrap_or(ChunkRegion {
                        min_x: data.origin[0],
                        min_z: data.origin[2],
                        max_x: data.origin[0] + f64::from(data.mesh.bounds_max[0]),
                        max_z: data.origin[2] + f64::from(data.mesh.bounds_max[2]),
                    });
                    let stage = self.chunks.iter().any(|(existing_key, binding)| {
                        *existing_key != key
                            && binding.active
                            && !desired_regions.contains_key(existing_key)
                            && binding.region.overlaps(region)
                    });
                    match self.create_chunk_binding(engine, &data, &material, world_origin, !stage)
                    {
                        Ok(binding) => {
                            if self.runtime.commit_succeeded(data.id, data.revision) {
                                if let Some(replaced) = self.chunks.insert(key, binding) {
                                    self.destroy_chunk_binding(engine, replaced);
                                }
                                physics_changed = true;
                            } else {
                                self.destroy_chunk_binding(engine, binding);
                                let message =
                                    "terrain runtime rejected host commit acknowledgement"
                                        .to_string();
                                self.stats.mesh_failures += 1;
                                self.stats.last_error = Some(message.clone());
                                tracing::warn!(chunk = ?data.id, error = %message, "terrain chunk commit failed");
                            }
                        }
                        Err(message) => {
                            let _ =
                                self.runtime
                                    .commit_failed(data.id, data.revision, message.clone());
                            self.stats.mesh_failures += 1;
                            self.stats.last_error = Some(message.clone());
                            tracing::warn!(chunk = ?data.id, error = %message, "terrain chunk commit failed");
                        }
                    }
                }
                // Host bindings are reconciled by physical coverage below.
                // Runtime unloads cannot identify an older revision and a
                // CDLOD parent has different coordinates from its children.
                TerrainRuntimeEvent::Unload(_) => {}
                TerrainRuntimeEvent::Failed { id, message, .. } => {
                    self.stats.generation_failures += 1;
                    self.stats.last_error = Some(message.clone());
                    tracing::warn!(chunk = ?id, error = %message, "terrain chunk generation failed");
                }
            }
        }
        // Keep an obsolete parent/revision until every desired region that it
        // overlaps has a committed replacement. This makes both split (one
        // parent -> four children) and merge transitions hole-free.
        let committed = self.chunks.keys().copied().collect::<BTreeSet<_>>();
        let stale_bindings = self
            .chunks
            .iter()
            .filter_map(|(key, binding)| {
                binding_can_retire(*key, binding.region, &desired_regions, &committed)
                    .then_some(*key)
            })
            .collect::<Vec<_>>();
        let retiring = stale_bindings.iter().copied().collect::<BTreeSet<_>>();
        let activating = self
            .chunks
            .iter()
            .filter_map(|(key, binding)| {
                if binding.active || !desired_regions.contains_key(key) {
                    return None;
                }
                let blockers_retire = self.chunks.iter().all(|(blocker_key, blocker)| {
                    !blocker.active
                        || desired_regions.contains_key(blocker_key)
                        || !blocker.region.overlaps(binding.region)
                        || retiring.contains(blocker_key)
                });
                blockers_retire.then_some(*key)
            })
            .collect::<Vec<_>>();
        for key in stale_bindings {
            physics_changed |= self.unload_chunk(engine, key);
        }
        for key in activating {
            physics_changed |= self.activate_chunk(engine, key);
        }
        self.stats.live_chunks = self.chunks.len();
        physics_changed
    }

    pub fn debug_snapshot(&self) -> TerrainDebugSnapshot {
        self.runtime.snapshot()
    }

    pub fn binding_stats(&self) -> &TerrainBindingStats {
        &self.stats
    }

    /// Invalidate all current/in-flight chunks without changing authored
    /// parameters. Used by the procgen debug panel's Regenerate action.
    pub fn force_regenerate(&mut self) {
        self.regeneration_epoch = self.regeneration_epoch.wrapping_add(1).max(1);
    }

    pub fn retry_failed(&mut self) {
        self.runtime.retry_failed();
    }

    pub fn reset(&mut self, engine: &mut EngineRuntime) {
        let keys = self.chunks.keys().copied().collect::<Vec<_>>();
        for key in keys {
            self.unload_chunk(engine, key);
        }
        let _ = self.runtime.set_desired(std::iter::empty());
        self.previous_desired.clear();
        self.stats.live_chunks = 0;
    }

    fn create_chunk_binding(
        &mut self,
        engine: &mut EngineRuntime,
        data: &TerrainChunkData,
        material: &str,
        world_origin: [f64; 3],
        active: bool,
    ) -> Result<ChunkBinding, String> {
        let collision_span = data
            .collision
            .as_ref()
            .map(|collision| collision.sample_spacing * collision.columns.saturating_sub(1) as f32)
            .unwrap_or(data.mesh.bounds_max[0] - data.mesh.bounds_min[0]);
        let half_span = collision_span * 0.5;
        let descriptor = RuntimeMeshDescriptor {
            positions: data
                .mesh
                .positions
                .iter()
                .map(|position| {
                    Vec3::new(
                        position[0] - half_span,
                        position[1],
                        position[2] - half_span,
                    )
                })
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
                Vec3::new(
                    data.mesh.bounds_min[0] - half_span,
                    data.mesh.bounds_min[1],
                    data.mesh.bounds_min[2] - half_span,
                ),
                Vec3::new(
                    data.mesh.bounds_max[0] - half_span,
                    data.mesh.bounds_max[1],
                    data.mesh.bounds_max[2] - half_span,
                ),
            )),
        };
        let name = format!("terrain-{}-r{:016x}", data.id.label(), data.revision);
        let mesh = engine
            .create_runtime_mesh(&name, descriptor)
            .map_err(|error| error.to_string())?;
        let mesh_asset = engine
            .runtime_mesh_asset_id(mesh)
            .expect("new terrain runtime mesh has an asset id")
            .id;
        let relative_center = Vec3::new(
            (data.origin[0] + f64::from(half_span) - world_origin[0]) as f32,
            (data.origin[1] - world_origin[1]) as f32,
            (data.origin[2] + f64::from(half_span) - world_origin[2]) as f32,
        );
        let entity = match engine.with_world_mut(|world| {
            let entity = world.create_entity();
            world.add_component(
                entity,
                Transform {
                    translation: relative_center,
                    ..Transform::default()
                },
            );
            world.add_component(
                entity,
                Renderable {
                    mesh_asset,
                    material_asset: material.to_string(),
                    visible: active,
                    cast_shadows: true,
                    render_layer: "default".to_string(),
                },
            );
            let min = Vec3::from_array(data.mesh.bounds_min) - Vec3::new(half_span, 0.0, half_span);
            let max = Vec3::from_array(data.mesh.bounds_max) - Vec3::new(half_span, 0.0, half_span);
            world.add_component(
                entity,
                Bounds {
                    center: ((min + max) * 0.5).to_array(),
                    half_extents: ((max - min) * 0.5).to_array(),
                },
            );
            #[cfg(feature = "gameplay")]
            if active {
                if let Some(collision) = &data.collision {
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
                                heights: collision.heights.clone(),
                                scale: [collision_span, 1.0, collision_span],
                            },
                            ..engine_physics::Collider::default()
                        },
                    );
                }
            }
            entity
        }) {
            Some(entity) => entity,
            None => {
                let _ = engine.destroy_runtime_mesh(mesh);
                return Err("cannot commit terrain without an active World".to_string());
            }
        };
        let span = f64::from(collision_span);
        Ok(ChunkBinding {
            mesh,
            entity,
            region: ChunkRegion {
                min_x: data.origin[0],
                min_z: data.origin[2],
                max_x: data.origin[0] + span,
                max_z: data.origin[2] + span,
            },
            #[cfg(feature = "gameplay")]
            collision: if active { None } else { data.collision.clone() },
            #[cfg(feature = "gameplay")]
            collision_span,
            active,
        })
    }

    fn activate_chunk(&mut self, engine: &mut EngineRuntime, key: ChunkKey) -> bool {
        let Some(binding) = self.chunks.get_mut(&key) else {
            return false;
        };
        if binding.active {
            return false;
        }
        let entity = binding.entity;
        #[cfg(feature = "gameplay")]
        let collision = binding.collision.clone();
        #[cfg(feature = "gameplay")]
        let collision_span = binding.collision_span;
        let activated = engine
            .with_world_mut(|world| {
                let Some(renderable) = world.get_mut::<Renderable>(entity) else {
                    return false;
                };
                renderable.visible = true;
                #[cfg(feature = "gameplay")]
                if let Some(collision) = collision {
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
                }
                true
            })
            .unwrap_or(false);
        if activated {
            binding.active = true;
            binding.release_staged_collision();
        } else {
            self.stats.last_error = Some(format!(
                "cannot activate staged terrain binding for {:?}",
                key.0
            ));
        }
        activated
    }

    fn unload_chunk(&mut self, engine: &mut EngineRuntime, key: ChunkKey) -> bool {
        let Some(binding) = self.chunks.remove(&key) else {
            return false;
        };
        let _ = engine.with_world_mut(|world| world.destroy_entity(binding.entity));
        if let Err(error) = engine.destroy_runtime_mesh(binding.mesh) {
            self.stats.last_error = Some(error.to_string());
        }
        true
    }

    fn destroy_chunk_binding(&mut self, engine: &mut EngineRuntime, binding: ChunkBinding) {
        let _ = engine.with_world_mut(|world| world.destroy_entity(binding.entity));
        if let Err(error) = engine.destroy_runtime_mesh(binding.mesh) {
            self.stats.last_error = Some(error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use engine_scene::World;

    use super::*;
    use crate::EngineConfig;

    #[test]
    fn parent_retires_only_after_all_overlapping_children_commit() {
        let volume = TerrainVolume {
            chunk_size: 8.0,
            lod_distances: vec![6.0, 16.0],
            lod_hysteresis: 1.0,
            ..TerrainVolume::default()
        };
        let parent_id = TerrainChunkId::new(0, 0, 1);
        let parent_key = (parent_id, 1);
        let parent_region = ChunkRegion::from_request(parent_id, &volume);
        let desired = [(0, 0), (1, 0), (0, 1), (1, 1)]
            .into_iter()
            .map(|(x, z)| {
                let id = TerrainChunkId::new(x, z, 0);
                ((id, 2), ChunkRegion::from_request(id, &volume))
            })
            .collect::<BTreeMap<_, _>>();
        let mut committed = desired.keys().copied().collect::<BTreeSet<_>>();
        let missing = *committed.first().expect("four children");
        committed.remove(&missing);
        assert!(!binding_can_retire(
            parent_key,
            parent_region,
            &desired,
            &committed
        ));
        committed.insert(missing);
        assert!(binding_can_retire(
            parent_key,
            parent_region,
            &desired,
            &committed
        ));
    }

    fn runtime_with_small_terrain() -> EngineRuntime {
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let mut world = World::new();
        let entity = world.create_entity();
        world.add_component(
            entity,
            TerrainVolume {
                chunk_size: 8.0,
                base_resolution: 9,
                lod_distances: vec![6.0],
                lod_hysteresis: 1.0,
                ..TerrainVolume::default()
            },
        );
        runtime.set_world(world);
        runtime
    }

    fn tick_until_settled(terrain: &mut TerrainSystem, runtime: &mut EngineRuntime) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            terrain.tick(runtime, Some([0.0; 3]));
            let stats = terrain.debug_snapshot().stats;
            if stats.queued + stats.generating + stats.ready_to_commit == 0 && stats.resident > 0 {
                break;
            }
            assert!(Instant::now() < deadline, "terrain integration timed out");
            std::thread::yield_now();
        }
    }

    #[test]
    fn generated_chunk_uses_runtime_mesh_and_transient_ecs_entity() {
        let mut runtime = runtime_with_small_terrain();
        let mut terrain = TerrainSystem::new(TerrainRuntimeConfig {
            worker_count: 1,
            max_in_flight: 1,
            ..TerrainRuntimeConfig::default()
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while terrain.binding_stats().live_chunks == 0 {
            terrain.tick(&mut runtime, Some([0.0; 3]));
            assert!(Instant::now() < deadline, "terrain integration timed out");
            std::thread::yield_now();
        }
        assert!(runtime.runtime_mesh_memory().mesh_count > 0);
        assert!(
            runtime
                .with_world(|world| world.query::<Renderable>().count())
                .unwrap()
                > 0
        );
        #[cfg(feature = "gameplay")]
        assert!(runtime
            .with_world(|world| world
                .query::<engine_physics::Collider>()
                .any(|(_, collider)| {
                    matches!(
                        collider.shape,
                        engine_physics::ColliderShape::HeightField { .. }
                    )
                }))
            .unwrap());

        runtime.with_world_mut(|world| {
            for (_, volume) in world.query_mut::<TerrainVolume>() {
                volume.enabled = false;
            }
        });
        terrain.tick(&mut runtime, Some([0.0; 3]));
        assert_eq!(terrain.binding_stats().live_chunks, 0);
        assert_eq!(runtime.runtime_mesh_memory().mesh_count, 0);
    }

    #[test]
    fn failed_replacement_keeps_the_previous_binding_active() {
        let mut runtime = runtime_with_small_terrain();
        let mut terrain = TerrainSystem::new(TerrainRuntimeConfig {
            worker_count: 1,
            max_in_flight: 1,
            ..TerrainRuntimeConfig::default()
        });
        tick_until_settled(&mut terrain, &mut runtime);
        let old_key = *terrain
            .chunks
            .keys()
            .next()
            .expect("resident terrain chunk");
        assert!(terrain.chunks[&old_key].active);

        let revised = runtime
            .with_world_mut(|world| {
                let (_, volume) = world
                    .query_mut::<TerrainVolume>()
                    .next()
                    .expect("terrain volume");
                volume.height_scale += 1.0;
                volume.revision()
            })
            .expect("world");
        let conflicting_name = format!("terrain-{}-r{revised:016x}", old_key.0.label());
        let conflict = runtime
            .create_runtime_mesh(
                &conflicting_name,
                RuntimeMeshDescriptor::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y], vec![0, 1, 2]),
            )
            .expect("reserve replacement mesh name");

        let deadline = Instant::now() + Duration::from_secs(3);
        while terrain.binding_stats().mesh_failures == 0 {
            terrain.tick(&mut runtime, Some([0.0; 3]));
            assert!(Instant::now() < deadline, "host failure was not observed");
            std::thread::yield_now();
        }
        assert!(terrain.chunks.contains_key(&old_key));
        assert!(terrain.chunks[&old_key].active);
        assert!(!terrain.chunks.contains_key(&(old_key.0, revised)));
        runtime
            .destroy_runtime_mesh(conflict)
            .expect("release reserved mesh name");
    }
}
