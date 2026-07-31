use engine_nav::{SphericalNavError, SphericalNavGraph, SphericalNavigationRuntime};
use engine_terrain::{PlanetSurfaceOccupancy, PlanetSurfaceVolumeKey};
use glam::DVec3;

use super::GameLoop;

const SURFACE_ANCHOR_LAYER: &str = "engine-surface-anchor";

impl GameLoop {
    /// Atomically publishes all navigation-blocking construction footprints
    /// to a spherical graph. Other dynamic-obstacle producers are preserved.
    ///
    /// A changed footprint set advances the graph revision, causing
    /// `SphericalNavigationRuntime` agents to replan through their configured
    /// per-frame budget.
    pub fn sync_planet_construction_navigation(
        &self,
        graph: &mut SphericalNavGraph,
    ) -> Result<usize, SphericalNavError> {
        let volume = self
            .nearest_terrain_volume_scope_for_graph(graph)?
            .unwrap_or(PlanetSurfaceVolumeKey::Legacy);
        sync_surface_occupancy(self.terrain_surface_occupancy(), &volume, graph)
    }

    /// Publish only construction footprints belonging to one authored planet.
    ///
    /// This explicit form is recommended when multiple planets intentionally
    /// share a center or when graph ownership is managed outside the scene.
    pub fn sync_planet_construction_navigation_for_volume(
        &self,
        terrain_volume_id: &str,
        graph: &mut SphericalNavGraph,
    ) -> Result<usize, SphericalNavError> {
        sync_surface_occupancy(
            self.terrain_surface_occupancy(),
            &PlanetSurfaceVolumeKey::from_persistent_id(terrain_volume_id),
            graph,
        )
    }

    pub fn sync_planet_construction_navigation_runtime(
        &self,
        runtime: &mut SphericalNavigationRuntime,
    ) -> Result<usize, SphericalNavError> {
        self.sync_planet_construction_navigation(runtime.graph_mut())
    }

    pub fn sync_planet_construction_navigation_runtime_for_volume(
        &self,
        terrain_volume_id: &str,
        runtime: &mut SphericalNavigationRuntime,
    ) -> Result<usize, SphericalNavError> {
        self.sync_planet_construction_navigation_for_volume(terrain_volume_id, runtime.graph_mut())
    }

    fn nearest_terrain_volume_scope_for_graph(
        &self,
        graph: &SphericalNavGraph,
    ) -> Result<Option<PlanetSurfaceVolumeKey>, SphericalNavError> {
        self.runtime
            .with_world(|world| {
                let mut candidates = world
                    .query::<engine_terrain::TerrainVolume>()
                    .filter(|(_, volume)| {
                        volume.enabled
                            && volume.topology == engine_terrain::TerrainTopology::CubeSphere
                            && volume.validate().is_ok()
                    })
                    .map(|(entity, volume)| {
                        let scope = world.persistent_id(entity).map_or_else(
                            || {
                                PlanetSurfaceVolumeKey::from_runtime_entity(
                                    entity.index(),
                                    entity.generation(),
                                )
                            },
                            |id| PlanetSurfaceVolumeKey::Persistent(id.to_owned()),
                        );
                        let distance_squared = DVec3::from_array(volume.planet_center)
                            .distance_squared(graph.center());
                        (scope, distance_squared)
                    })
                    .collect::<Vec<_>>();
                candidates.sort_by(|left, right| {
                    left.1
                        .total_cmp(&right.1)
                        .then_with(|| left.0.cmp(&right.0))
                });
                let Some((nearest, nearest_distance)) = candidates.first() else {
                    return Ok(None);
                };
                if candidates.get(1).is_some_and(|(_, distance)| {
                    distances_are_ambiguous(*nearest_distance, *distance)
                }) {
                    return Err(SphericalNavError::AmbiguousSurfaceBinding);
                }
                Ok(Some(nearest.clone()))
            })
            .unwrap_or(Ok(None))
    }
}

fn distances_are_ambiguous(left: f64, right: f64) -> bool {
    (left - right).abs() <= f64::EPSILON * 8.0 * left.abs().max(right.abs()).max(1.0)
}

fn sync_surface_occupancy(
    occupancy: &PlanetSurfaceOccupancy,
    volume: &PlanetSurfaceVolumeKey,
    graph: &mut SphericalNavGraph,
) -> Result<usize, SphericalNavError> {
    graph.replace_obstacle_layer(
        SURFACE_ANCHOR_LAYER,
        occupancy
            .navigation_blockers_for_scope(volume)
            .map(|(owner, footprint)| {
                (
                    owner.stable_key(),
                    DVec3::from_array(footprint.direction),
                    footprint.angular_radius,
                )
            }),
    )
}

#[cfg(test)]
mod tests {
    use engine_nav::{SphericalNavBuildConfig, SphericalNavGraph, SphericalNavObstacle};
    use engine_scene::components::Transform;
    use engine_terrain::{
        PlanetSurfaceAnchor, PlanetSurfaceOccupancy, PlanetSurfaceOwnerKey, PlanetSurfacePlacement,
        TerrainTopology, TerrainVolume,
    };
    use glam::DVec3;

    use super::*;
    use crate::EngineConfig;

    fn placement(direction: DVec3, angular_radius: f64) -> PlanetSurfacePlacement {
        PlanetSurfacePlacement {
            position: direction.to_array(),
            normal: direction.to_array(),
            right: DVec3::X.to_array(),
            forward: DVec3::Z.to_array(),
            rotation: [0.0, 0.0, 0.0, 1.0],
            radial_direction: direction.to_array(),
            angular_radius,
            maximum_slope_radians: 0.0,
            support_height_span: 0.0,
        }
    }

    #[test]
    fn construction_sync_is_stable_and_preserves_other_dynamic_layers() {
        let mut occupancy = PlanetSurfaceOccupancy::default();
        occupancy
            .reserve("building:42", placement(DVec3::X, 0.05), true, 0.0)
            .unwrap();
        occupancy
            .reserve("decoration", placement(DVec3::Y, 0.01), false, 0.0)
            .unwrap();
        let mut graph = SphericalNavGraph::fibonacci(
            DVec3::ZERO,
            1_000.0,
            SphericalNavBuildConfig {
                node_count: 256,
                neighbors_per_node: 8,
                ..Default::default()
            },
        )
        .unwrap();
        graph
            .upsert_obstacle(SphericalNavObstacle::new("weather", DVec3::Z, 0.02).unwrap())
            .unwrap();

        sync_surface_occupancy(&occupancy, &PlanetSurfaceVolumeKey::Legacy, &mut graph).unwrap();
        assert!(graph.obstacles().any(|obstacle| obstacle.id
            == format!(
                "engine-surface-anchor:{}",
                PlanetSurfaceOwnerKey::Persistent("building:42".into()).stable_key()
            )));
        assert!(!graph
            .obstacles()
            .any(|obstacle| obstacle.id.ends_with("decoration")));
        assert!(graph.obstacles().any(|obstacle| obstacle.id == "weather"));
        let revision = graph.dynamic_revision();
        sync_surface_occupancy(&occupancy, &PlanetSurfaceVolumeKey::Legacy, &mut graph).unwrap();
        assert_eq!(graph.dynamic_revision(), revision);
    }

    #[test]
    fn construction_sync_filters_identical_directions_by_planet() {
        let mut occupancy = PlanetSurfaceOccupancy::default();
        occupancy
            .reserve_for_volume("planet-a", "building", placement(DVec3::X, 0.05), true, 0.0)
            .unwrap();
        occupancy
            .reserve_for_volume("planet-b", "building", placement(DVec3::X, 0.08), true, 0.0)
            .unwrap();
        let mut graph = SphericalNavGraph::fibonacci(
            DVec3::ZERO,
            1_000.0,
            SphericalNavBuildConfig {
                node_count: 256,
                neighbors_per_node: 8,
                ..Default::default()
            },
        )
        .unwrap();

        sync_surface_occupancy(
            &occupancy,
            &PlanetSurfaceVolumeKey::Persistent("planet-a".into()),
            &mut graph,
        )
        .unwrap();
        let obstacles = graph.obstacles().collect::<Vec<_>>();
        assert_eq!(obstacles.len(), 1);
        assert_eq!(
            obstacles[0].id,
            format!(
                "engine-surface-anchor:{}",
                PlanetSurfaceOwnerKey::Persistent("building".into()).stable_key()
            )
        );
        assert!((obstacles[0].angular_radius - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn navigation_ids_keep_persistent_and_runtime_owners_distinct() {
        let volume = PlanetSurfaceVolumeKey::Persistent("planet".into());
        let mut occupancy = PlanetSurfaceOccupancy::default();
        occupancy
            .reserve_scoped(
                volume.clone(),
                PlanetSurfaceOwnerKey::Persistent("runtime:7:3".into()),
                placement(DVec3::X, 0.01),
                true,
                0.0,
            )
            .unwrap();
        occupancy
            .reserve_scoped(
                volume.clone(),
                PlanetSurfaceOwnerKey::Runtime {
                    index: 7,
                    generation: 3,
                },
                placement(DVec3::Y, 0.01),
                true,
                0.0,
            )
            .unwrap();
        let mut graph = SphericalNavGraph::fibonacci(
            DVec3::ZERO,
            1_000.0,
            SphericalNavBuildConfig {
                node_count: 64,
                neighbors_per_node: 4,
                ..Default::default()
            },
        )
        .unwrap();

        sync_surface_occupancy(&occupancy, &volume, &mut graph).unwrap();
        let ids = graph
            .obstacles()
            .map(|obstacle| obstacle.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn game_loop_selects_the_graph_planet_and_rejects_center_ties() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        let mut world = engine_scene::World::new();
        for (id, center, radius) in [
            ("planet-a", [0.0, 0.0, 0.0], 100.0),
            ("planet-b", [1_000.0, 0.0, 0.0], 200.0),
        ] {
            let entity = world.create_persistent_entity(id).unwrap();
            world.add_component(
                entity,
                TerrainVolume {
                    topology: TerrainTopology::CubeSphere,
                    planet_center: center,
                    planet_radius: radius,
                    planet_max_lod: 0,
                    base_resolution: 3,
                    lod_distances: vec![500.0],
                    height_scale: 0.0,
                    ..TerrainVolume::default()
                },
            );
            let anchor = world
                .create_persistent_entity(format!("anchor-{id}"))
                .unwrap();
            world.add_component(anchor, Transform::default());
            world.add_component(
                anchor,
                PlanetSurfaceAnchor {
                    terrain_volume_id: id.to_string(),
                    direction: [1.0, 0.0, 0.0],
                    footprint_radius: 1.0,
                    max_slope_radians: 1.0e-4,
                    max_height_delta: 1.0,
                    ..PlanetSurfaceAnchor::default()
                },
            );
        }
        game_loop.runtime.set_world(world);
        game_loop.tick_terrain(Some([0.0, 0.0, 150.0]));

        let config = SphericalNavBuildConfig {
            node_count: 64,
            neighbors_per_node: 4,
            ..Default::default()
        };
        let mut second_graph =
            SphericalNavGraph::fibonacci(DVec3::new(1_000.0, 0.0, 0.0), 200.0, config).unwrap();
        game_loop
            .sync_planet_construction_navigation(&mut second_graph)
            .unwrap();
        let obstacle_ids = second_graph
            .obstacles()
            .map(|obstacle| obstacle.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            obstacle_ids,
            [format!(
                "engine-surface-anchor:{}",
                PlanetSurfaceOwnerKey::Persistent("anchor-planet-b".into()).stable_key()
            )]
        );

        let tie_graph =
            SphericalNavGraph::fibonacci(DVec3::new(500.0, 0.0, 0.0), 100.0, config).unwrap();
        let mut ambiguous_graph = tie_graph.clone();
        assert!(matches!(
            game_loop.sync_planet_construction_navigation(&mut ambiguous_graph),
            Err(SphericalNavError::AmbiguousSurfaceBinding)
        ));

        let mut tie_graph = tie_graph;
        game_loop
            .sync_planet_construction_navigation_for_volume("planet-a", &mut tie_graph)
            .unwrap();
        assert_eq!(tie_graph.obstacles().count(), 1);
    }
}
