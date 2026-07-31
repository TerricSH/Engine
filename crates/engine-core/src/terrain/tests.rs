use std::time::{Duration, Instant};

use engine_scene::{
    components::{Camera, CameraProjection},
    extract_renderer_input_from_world, World,
};

use super::*;
use crate::EngineConfig;

#[path = "tests/cube_sphere_rendering.rs"]
mod cube_sphere_rendering;

struct TerrainAssetUploadBackend {
    uploads: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl TerrainAssetUploadBackend {
    fn record(&self, value: String) {
        self.uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(value);
    }
}

impl engine_renderer::BackendRenderer for TerrainAssetUploadBackend {
    fn begin_frame(
        &mut self,
        _input: &engine_renderer::RenderFrameInput,
    ) -> Result<(), Vec<engine_renderer::Diagnostic>> {
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &engine_renderer::RenderFrameInput,
        _pass: &engine_renderer::render_graph2::PassNode,
        _barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<engine_renderer::Diagnostic>> {
        Ok(())
    }

    fn execute_pass(
        &mut self,
        _input: &engine_renderer::RenderFrameInput,
        _pass: &engine_renderer::render_graph2::PassNode,
        _frame_stats: &mut engine_renderer::FrameStats,
    ) -> Result<(), Vec<engine_renderer::Diagnostic>> {
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<engine_renderer::Diagnostic>> {
        Ok(())
    }

    fn end_frame(
        &mut self,
        _stats: &mut engine_renderer::FrameStats,
    ) -> Result<(), Vec<engine_renderer::Diagnostic>> {
        Ok(())
    }

    fn upload_mesh(
        &mut self,
        upload: engine_renderer::MeshUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<engine_renderer::Diagnostic>> {
        self.record(format!("mesh:{}", upload.mesh_id.id));
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_texture(
        &mut self,
        upload: engine_renderer::TextureUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<engine_renderer::Diagnostic>> {
        self.record(format!("texture:{}", upload.texture_id.id));
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        upload: engine_renderer::MaterialUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<engine_renderer::Diagnostic>> {
        self.record(format!("material:{}", upload.material_id.id));
        Ok(engine_renderer::UploadReceipt::new(1))
    }
}

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

#[test]
fn colliding_volume_identity_group_is_entirely_rejected() {
    let collision_id = TerrainVolumeId::new(0x42);
    let candidate = |identity_label: &str| TerrainVolumeCandidate {
        volume_id: collision_id,
        identity_label: identity_label.to_string(),
        occupancy_scope: PlanetSurfaceVolumeKey::Persistent(identity_label.to_string()),
        persistent_id: Some(identity_label.to_string()),
        volume: TerrainVolume::default(),
    };

    let (single, errors) = reject_colliding_volume_ids(vec![candidate("planet-b")]);
    assert_eq!(single.len(), 1);
    assert!(errors.is_empty());

    let (next, errors) =
        reject_colliding_volume_ids(vec![candidate("planet-a"), candidate("planet-b")]);
    assert!(
        next.is_empty(),
        "neither member may inherit a prior resident namespace"
    );
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("planet-a, planet-b"));
}

#[test]
fn colliding_occupancy_scope_group_is_entirely_rejected() {
    let candidate = |volume_id, identity_label: &str| TerrainVolumeCandidate {
        volume_id: TerrainVolumeId::new(volume_id),
        identity_label: identity_label.to_string(),
        occupancy_scope: PlanetSurfaceVolumeKey::Persistent("shared".into()),
        persistent_id: Some(identity_label.to_string()),
        volume: TerrainVolume::default(),
    };

    let (next, errors) =
        reject_colliding_volume_ids(vec![candidate(1, "planet-a"), candidate(2, "planet-b")]);
    assert!(next.is_empty());
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("planet-a, planet-b"));
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
    tick_until_settled_at(terrain, runtime, [0.0; 3]);
}

fn tick_until_settled_at(
    terrain: &mut TerrainSystem,
    runtime: &mut EngineRuntime,
    focus: [f64; 3],
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        terrain.tick(runtime, Some(focus));
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
    #[cfg(feature = "subsystem-physics")]
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
fn runtime_only_volumes_receive_distinct_stable_chunk_namespaces() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut world = World::new();
    let volume = TerrainVolume {
        topology: TerrainTopology::CubeSphere,
        planet_radius: 100.0,
        planet_max_lod: 0,
        base_resolution: 3,
        height_scale: 0.0,
        lod_distances: vec![500.0],
        ..TerrainVolume::default()
    };
    let first = world.create_entity();
    world.add_component(first, volume.clone());
    let second = world.create_entity();
    world.add_component(second, volume);
    runtime.set_world(world);

    let mut terrain = TerrainSystem::default();
    terrain.tick(&mut runtime, Some([0.0, 0.0, 110.0]));
    let identities = terrain
        .debug_snapshot()
        .chunks
        .into_iter()
        .map(|chunk| chunk.id.volume_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        identities,
        BTreeSet::from([
            TerrainVolumeId::from_runtime_entity(first.index(), first.generation()),
            TerrainVolumeId::from_runtime_entity(second.index(), second.generation()),
        ])
    );
    assert!(terrain.binding_stats().last_error.is_none());
}

#[test]
fn authored_runtime_shaped_id_cannot_alias_an_anonymous_volume() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut world = World::new();
    let volume = TerrainVolume {
        topology: TerrainTopology::CubeSphere,
        planet_radius: 100.0,
        planet_max_lod: 0,
        base_resolution: 3,
        height_scale: 0.0,
        lod_distances: vec![500.0],
        ..TerrainVolume::default()
    };
    let anonymous = world.create_entity();
    world.add_component(anonymous, volume.clone());
    let authored_id = runtime_entity_identity_label(anonymous);
    let authored = world
        .create_persistent_entity(&authored_id)
        .expect("runtime-shaped persistent ID is still an authored domain");
    world.add_component(authored, volume);
    runtime.set_world(world);

    let mut terrain = TerrainSystem::default();
    terrain.tick(&mut runtime, Some([0.0, 0.0, 110.0]));
    let anonymous_id =
        TerrainVolumeId::from_runtime_entity(anonymous.index(), anonymous.generation());
    let authored_id = TerrainVolumeId::from_persistent_id(&authored_id);
    assert_ne!(anonymous_id, authored_id);
    let identities = terrain
        .debug_snapshot()
        .chunks
        .into_iter()
        .map(|chunk| chunk.id.volume_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(identities, BTreeSet::from([anonymous_id, authored_id]));
    assert!(terrain.binding_stats().last_error.is_none());
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

#[test]
fn two_planets_stream_identical_quadtrees_and_disable_or_destroy_in_isolation() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut world = World::new();
    let planet = |material_asset: &str| TerrainVolume {
        topology: TerrainTopology::CubeSphere,
        planet_center: [0.0; 3],
        planet_radius: 100.0,
        planet_max_lod: 1,
        base_resolution: 3,
        height_scale: 0.0,
        lod_distances: vec![30.0, 500.0],
        lod_hysteresis: 2.0,
        material_asset: material_asset.to_string(),
        ..TerrainVolume::default()
    };
    let first = world
        .create_persistent_entity("planet-a")
        .expect("first planet id");
    world.add_component(first, planet("mat-planet-a"));
    let second = world
        .create_persistent_entity("planet-b")
        .expect("second planet id");
    world.add_component(second, planet("mat-planet-b"));
    runtime.set_world(world);

    let mut terrain = TerrainSystem::new(TerrainRuntimeConfig {
        worker_count: 2,
        max_in_flight: 4,
        ..TerrainRuntimeConfig::default()
    });
    let focus = [0.0, 0.0, 110.0];
    tick_until_settled_at(&mut terrain, &mut runtime, focus);

    let first_id = TerrainVolumeId::from_persistent_id("planet-a");
    let second_id = TerrainVolumeId::from_persistent_id("planet-b");
    let snapshot = terrain.debug_snapshot();
    let coordinates_for = |volume_id| {
        snapshot
            .chunks
            .iter()
            .filter(|chunk| chunk.id.volume_id == volume_id)
            .map(|chunk| (chunk.id.face, chunk.id.x, chunk.id.z, chunk.id.lod))
            .collect::<BTreeSet<_>>()
    };
    let first_coordinates = coordinates_for(first_id);
    let second_coordinates = coordinates_for(second_id);
    assert!(!first_coordinates.is_empty());
    assert_eq!(first_coordinates, second_coordinates);
    for volume_id in [first_id, second_id] {
        assert!(snapshot.chunks.iter().any(|chunk| {
            chunk.id.volume_id == volume_id
                && chunk.id.face == engine_terrain::TerrainFace::PositiveZ
                && chunk.id.lod == 0
        }));
        assert!(!snapshot.chunks.iter().any(|chunk| {
            chunk.id.volume_id == volume_id
                && chunk.id.face == engine_terrain::TerrainFace::NegativeZ
        }));
    }
    assert!(terrain.chunks.values().all(|binding| binding.active));
    let material_counts = runtime
        .with_world(|world| {
            [
                world
                    .query::<Renderable>()
                    .filter(|(_, renderable)| renderable.material_asset == "mat-planet-a")
                    .count(),
                world
                    .query::<Renderable>()
                    .filter(|(_, renderable)| renderable.material_asset == "mat-planet-b")
                    .count(),
            ]
        })
        .expect("world");
    assert!(material_counts.into_iter().all(|count| count > 0));

    runtime
        .with_world_mut(|world| {
            world.get_mut::<TerrainVolume>(first).unwrap().enabled = false;
        })
        .expect("world");
    terrain.tick(&mut runtime, Some(focus));
    assert!(terrain
        .chunks
        .keys()
        .all(|(id, _)| id.volume_id == second_id));

    runtime
        .with_world_mut(|world| {
            world.get_mut::<TerrainVolume>(first).unwrap().enabled = true;
        })
        .expect("world");
    tick_until_settled_at(&mut terrain, &mut runtime, focus);
    assert!(terrain
        .chunks
        .keys()
        .any(|(id, _)| id.volume_id == first_id));
    assert!(terrain
        .chunks
        .keys()
        .any(|(id, _)| id.volume_id == second_id));

    runtime
        .with_world_mut(|world| {
            assert!(world.destroy_entity(second));
        })
        .expect("world");
    terrain.tick(&mut runtime, Some(focus));
    assert!(terrain
        .chunks
        .keys()
        .all(|(id, _)| id.volume_id == first_id));
    assert!(terrain.binding_stats().last_error.is_none());
}
