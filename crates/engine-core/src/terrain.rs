//! Host bridge from `engine-terrain` CPU output to runtime meshes and ECS.

mod anchors;
mod material_mapping;

use std::collections::{BTreeMap, BTreeSet};

use engine_scene::components::{Bounds, Renderable, Transform, VertexGeomorph};
use engine_terrain::{
    desired_terrain_chunks_for_volume_hysteretic, terrain_chunk_bounds, terrain_chunk_distance,
    HeightfieldGenerator, PlanetSurfaceAnchor, PlanetSurfaceOccupancy, PlanetSurfaceVolumeKey,
    TerrainChunkData, TerrainChunkId, TerrainDebugSnapshot, TerrainRuntime, TerrainRuntimeConfig,
    TerrainRuntimeEvent, TerrainTopology, TerrainVolume, TerrainVolumeId,
};
#[cfg(feature = "subsystem-physics")]
use engine_terrain::{TerrainCollisionData, TerrainTriangleCollisionData};
use glam::{Vec2, Vec3};

use crate::{EngineRuntime, RuntimeMeshDescriptor, RuntimeMeshHandle};
use material_mapping::projected_material_mapping;

pub(crate) fn runtime_entity_identity_label(entity: engine_scene::Entity) -> String {
    format!("runtime:{}:{}", entity.index(), entity.generation())
}

struct ChunkBinding {
    mesh: RuntimeMeshHandle,
    entity: engine_scene::Entity,
    region: ChunkRegion,
    #[cfg(feature = "subsystem-physics")]
    collision: Option<TerrainCollisionData>,
    #[cfg(feature = "subsystem-physics")]
    collision_span: f32,
    #[cfg(feature = "subsystem-physics")]
    triangle_collision: Option<TerrainTriangleCollisionData>,
    active: bool,
}

impl ChunkBinding {
    #[cfg(feature = "subsystem-physics")]
    fn release_staged_collision(&mut self) {
        self.collision = None;
        self.triangle_collision = None;
    }

    #[cfg(not(feature = "subsystem-physics"))]
    fn release_staged_collision(&mut self) {}
}

type ChunkKey = (TerrainChunkId, u64);

#[derive(Clone, Copy, Debug)]
struct ChunkRegion {
    min: [f64; 3],
    max: [f64; 3],
}

#[derive(Clone, Debug)]
struct TerrainVolumeCandidate {
    volume_id: TerrainVolumeId,
    identity_label: String,
    occupancy_scope: PlanetSurfaceVolumeKey,
    persistent_id: Option<String>,
    volume: TerrainVolume,
}

#[derive(Clone, Debug)]
struct ActiveTerrainVolume {
    occupancy_scope: PlanetSurfaceVolumeKey,
    persistent_id: Option<String>,
    volume: TerrainVolume,
    material: String,
}

fn reject_colliding_volume_ids(
    candidates: Vec<TerrainVolumeCandidate>,
) -> (Vec<TerrainVolumeCandidate>, Vec<String>) {
    let mut groups = BTreeMap::<TerrainVolumeId, Vec<TerrainVolumeCandidate>>::new();
    for candidate in candidates {
        groups
            .entry(candidate.volume_id)
            .or_default()
            .push(candidate);
    }
    let mut unique = Vec::new();
    let mut errors = Vec::new();
    for (volume_id, mut group) in groups {
        if group.len() == 1 {
            unique.push(group.pop().expect("one candidate"));
            continue;
        }
        group.sort_by(|left, right| left.identity_label.cmp(&right.identity_label));
        errors.push(format!(
            "terrain volume identity collision for {:016x}: {}",
            volume_id.get(),
            group
                .iter()
                .map(|candidate| candidate.identity_label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut occupancy_groups =
        BTreeMap::<PlanetSurfaceVolumeKey, Vec<TerrainVolumeCandidate>>::new();
    for candidate in unique {
        occupancy_groups
            .entry(candidate.occupancy_scope.clone())
            .or_default()
            .push(candidate);
    }
    let mut fully_unique = Vec::new();
    for (_, mut group) in occupancy_groups {
        if group.len() == 1 {
            fully_unique.push(group.pop().expect("one candidate"));
            continue;
        }
        group.sort_by(|left, right| left.identity_label.cmp(&right.identity_label));
        errors.push(format!(
            "terrain volume occupancy identity collision: {}",
            group
                .iter()
                .map(|candidate| candidate.identity_label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    fully_unique.sort_by(|left, right| left.identity_label.cmp(&right.identity_label));
    (fully_unique, errors)
}

impl ChunkRegion {
    fn from_request(id: TerrainChunkId, volume: &TerrainVolume) -> Self {
        let (min, max) = terrain_chunk_bounds(volume, id);
        Self { min, max }
    }

    fn overlaps(self, other: Self) -> bool {
        (0..3).all(|axis| self.min[axis] < other.max[axis] && other.min[axis] < self.max[axis])
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
            (desired_key.0.volume_id == key.0.volume_id && region.overlaps(*desired_region))
                .then_some(*desired_key)
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
    surface_occupancy: PlanetSurfaceOccupancy,
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
            surface_occupancy: PlanetSurfaceOccupancy::default(),
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
                    if let Some(persistent_id) = world.persistent_id(entity) {
                        TerrainVolumeCandidate {
                            volume_id: TerrainVolumeId::from_persistent_id(persistent_id),
                            identity_label: persistent_id.to_string(),
                            occupancy_scope: PlanetSurfaceVolumeKey::Persistent(
                                persistent_id.to_string(),
                            ),
                            persistent_id: Some(persistent_id.to_string()),
                            volume: volume.clone(),
                        }
                    } else {
                        let identity_label = runtime_entity_identity_label(entity);
                        TerrainVolumeCandidate {
                            volume_id: TerrainVolumeId::from_runtime_entity(
                                entity.index(),
                                entity.generation(),
                            ),
                            occupancy_scope: PlanetSurfaceVolumeKey::from_runtime_entity(
                                entity.index(),
                                entity.generation(),
                            ),
                            identity_label,
                            persistent_id: None,
                            volume: volume.clone(),
                        }
                    }
                })
                .collect::<Vec<_>>();
            volumes.sort_by(|left, right| left.identity_label.cmp(&right.identity_label));
            let origin = world.world_origin();
            let camera = engine_scene::active_camera_world_position(world).unwrap_or(Vec3::ZERO);
            (volumes, origin, camera)
        });
        let Some((volumes, world_origin, camera_relative)) = selected else {
            return false;
        };
        let focus = focus_logical.unwrap_or([
            world_origin[0] + f64::from(camera_relative.x),
            world_origin[1] + f64::from(camera_relative.y),
            world_origin[2] + f64::from(camera_relative.z),
        ]);
        let mut active_volumes = BTreeMap::<TerrainVolumeId, ActiveTerrainVolume>::new();
        let mut desired = Vec::new();
        let (volumes, mut selection_errors) = reject_colliding_volume_ids(volumes);
        for candidate in volumes {
            let TerrainVolumeCandidate {
                volume_id,
                identity_label,
                occupancy_scope,
                persistent_id,
                volume,
            } = candidate;
            if let Err(error) = volume.validate() {
                selection_errors.push(format!(
                    "terrain volume '{identity_label}' has invalid configuration: {error}"
                ));
                continue;
            }
            let material = if volume.material_asset.is_empty() {
                "mat-default".to_string()
            } else {
                volume.material_asset.clone()
            };
            let mut volume_desired = desired_terrain_chunks_for_volume_hysteretic(
                volume_id,
                &volume,
                focus,
                &self.previous_desired,
            );
            for request in &mut volume_desired {
                request.revision ^= self.regeneration_epoch;
            }
            desired.extend(volume_desired);
            active_volumes.insert(
                volume_id,
                ActiveTerrainVolume {
                    occupancy_scope,
                    persistent_id,
                    volume,
                    material,
                },
            );
        }
        if selection_errors.is_empty() {
            if self.stats.last_error.as_deref().is_some_and(|message| {
                message.starts_with("terrain volume '")
                    || message.starts_with("terrain volume identity collision")
                    || message.starts_with("terrain volume occupancy identity collision")
                    || message.starts_with("invalid terrain configuration:")
                    || message.starts_with("multiple enabled TerrainVolume")
            }) {
                self.stats.last_error = None;
            }
        } else {
            self.stats.last_error = Some(selection_errors.join("; "));
        }
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
                    let Some(volume) = active_volumes.get(&data.id.volume_id) else {
                        let message = format!(
                            "terrain chunk {:?} completed without an active owning volume",
                            data.id
                        );
                        let _ = self
                            .runtime
                            .commit_failed(data.id, data.revision, message.clone());
                        self.stats.mesh_failures += 1;
                        self.stats.last_error = Some(message);
                        continue;
                    };
                    let region = desired_regions.get(&key).copied().unwrap_or(ChunkRegion {
                        min: std::array::from_fn(|axis| {
                            data.origin[axis] + f64::from(data.mesh.bounds_min[axis])
                        }),
                        max: std::array::from_fn(|axis| {
                            data.origin[axis] + f64::from(data.mesh.bounds_max[axis])
                        }),
                    });
                    let stage = self.chunks.iter().any(|(existing_key, binding)| {
                        *existing_key != key
                            && existing_key.0.volume_id == key.0.volume_id
                            && binding.active
                            && !desired_regions.contains_key(existing_key)
                            && binding.region.overlaps(region)
                    });
                    match self.create_chunk_binding(
                        engine,
                        &data,
                        &volume.material,
                        Some(&volume.volume),
                        world_origin,
                        !stage,
                    ) {
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
                        || blocker_key.0.volume_id != key.0.volume_id
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
        self.update_chunk_geomorph(engine, &active_volumes, focus);
        physics_changed |= self.update_surface_anchors(engine, &active_volumes, world_origin);
        self.stats.live_chunks = self.chunks.len();
        physics_changed
    }

    pub fn debug_snapshot(&self) -> TerrainDebugSnapshot {
        self.runtime.snapshot()
    }

    pub fn binding_stats(&self) -> &TerrainBindingStats {
        &self.stats
    }

    /// Current deterministic construction reservations rebuilt from authored
    /// [`PlanetSurfaceAnchor`] components on the most recent terrain tick.
    pub fn surface_occupancy(&self) -> &PlanetSurfaceOccupancy {
        &self.surface_occupancy
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
        self.surface_occupancy = PlanetSurfaceOccupancy::default();
        self.stats.live_chunks = 0;
    }

    fn create_chunk_binding(
        &mut self,
        engine: &mut EngineRuntime,
        data: &TerrainChunkData,
        material: &str,
        volume: Option<&TerrainVolume>,
        world_origin: [f64; 3],
        active: bool,
    ) -> Result<ChunkBinding, String> {
        #[cfg(feature = "subsystem-physics")]
        let collision_span = data
            .collision
            .as_ref()
            .map(|collision| collision.sample_spacing * collision.columns.saturating_sub(1) as f32)
            .unwrap_or(data.mesh.bounds_max[0] - data.mesh.bounds_min[0]);
        let local_center = Vec3::from_array(data.local_center);
        let descriptor = RuntimeMeshDescriptor {
            positions: data
                .mesh
                .positions
                .iter()
                .map(|position| Vec3::from_array(*position) - local_center)
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
                Vec3::from_array(data.mesh.bounds_min) - local_center,
                Vec3::from_array(data.mesh.bounds_max) - local_center,
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
            (data.origin[0] + f64::from(local_center.x) - world_origin[0]) as f32,
            (data.origin[1] + f64::from(local_center.y) - world_origin[1]) as f32,
            (data.origin[2] + f64::from(local_center.z) - world_origin[2]) as f32,
        );
        let material_mapping = projected_material_mapping(data, volume);
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
            let min = Vec3::from_array(data.mesh.bounds_min) - local_center;
            let max = Vec3::from_array(data.mesh.bounds_max) - local_center;
            world.add_component(
                entity,
                Bounds {
                    center: ((min + max) * 0.5).to_array(),
                    half_extents: ((max - min) * 0.5).to_array(),
                },
            );
            if let Some(mapping) = material_mapping {
                world.add_component(entity, mapping);
            }
            if let Some(geomorph) = data.geomorph {
                world.add_component(
                    entity,
                    VertexGeomorph {
                        factor: 0.0,
                        delta_scale: geomorph.delta_scale,
                        local_origin: (Vec3::from_array(geomorph.local_origin) - local_center)
                            .to_array(),
                    },
                );
            }
            #[cfg(feature = "subsystem-physics")]
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
                } else if let Some(collision) = &data.triangle_collision {
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
                                vertices: collision
                                    .positions
                                    .iter()
                                    .map(|position| {
                                        (Vec3::from_array(*position) - local_center).to_array()
                                    })
                                    .collect(),
                                indices: collision.triangles.clone(),
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
        Ok(ChunkBinding {
            mesh,
            entity,
            region: ChunkRegion {
                min: std::array::from_fn(|axis| {
                    data.origin[axis] + f64::from(data.mesh.bounds_min[axis])
                }),
                max: std::array::from_fn(|axis| {
                    data.origin[axis] + f64::from(data.mesh.bounds_max[axis])
                }),
            },
            #[cfg(feature = "subsystem-physics")]
            collision: if active { None } else { data.collision.clone() },
            #[cfg(feature = "subsystem-physics")]
            collision_span,
            #[cfg(feature = "subsystem-physics")]
            triangle_collision: if active {
                None
            } else {
                data.triangle_collision
                    .as_ref()
                    .map(|collision| TerrainTriangleCollisionData {
                        positions: collision
                            .positions
                            .iter()
                            .map(|position| (Vec3::from_array(*position) - local_center).to_array())
                            .collect(),
                        triangles: collision.triangles.clone(),
                    })
            },
            active,
        })
    }

    fn update_chunk_geomorph(
        &self,
        engine: &mut EngineRuntime,
        volumes: &BTreeMap<TerrainVolumeId, ActiveTerrainVolume>,
        focus: [f64; 3],
    ) {
        let updates = self
            .chunks
            .iter()
            .map(|((id, _), binding)| {
                let factor = volumes
                    .get(&id.volume_id)
                    .map(|volume| &volume.volume)
                    .filter(|volume| {
                        volume.geomorph_enabled
                            && volume.topology == TerrainTopology::CubeSphere
                            && id.face != engine_terrain::TerrainFace::Planar
                            && id.lod < volume.planet_max_lod
                    })
                    .and_then(|volume| {
                        volume.lod_distances.get(usize::from(id.lod)).map(|cutoff| {
                            let cutoff = f64::from(*cutoff);
                            let start = cutoff * f64::from(volume.geomorph_start_ratio);
                            let width = (cutoff - start).max(f64::EPSILON);
                            let t = ((terrain_chunk_distance(volume, *id, focus) - start) / width)
                                .clamp(0.0, 1.0) as f32;
                            t * t * (3.0 - 2.0 * t)
                        })
                    })
                    .unwrap_or(0.0);
                (binding.entity, factor)
            })
            .collect::<Vec<_>>();
        let _ = engine.with_world_mut(|world| {
            for (entity, factor) in updates {
                if let Some(morph) = world.get_mut::<VertexGeomorph>(entity) {
                    morph.factor = factor;
                }
            }
        });
    }

    fn update_surface_anchors(
        &mut self,
        engine: &mut EngineRuntime,
        volumes: &BTreeMap<TerrainVolumeId, ActiveTerrainVolume>,
        world_origin: [f64; 3],
    ) -> bool {
        anchors::update_surface_anchors(self, engine, volumes, world_origin)
    }

    fn activate_chunk(&mut self, engine: &mut EngineRuntime, key: ChunkKey) -> bool {
        let Some(binding) = self.chunks.get_mut(&key) else {
            return false;
        };
        if binding.active {
            return false;
        }
        let entity = binding.entity;
        #[cfg(feature = "subsystem-physics")]
        let collision = binding.collision.clone();
        #[cfg(feature = "subsystem-physics")]
        let collision_span = binding.collision_span;
        #[cfg(feature = "subsystem-physics")]
        let triangle_collision = binding.triangle_collision.clone();
        let activated = engine
            .with_world_mut(|world| {
                let Some(renderable) = world.get_mut::<Renderable>(entity) else {
                    return false;
                };
                renderable.visible = true;
                #[cfg(feature = "subsystem-physics")]
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
#[path = "terrain/tests.rs"]
mod tests;
