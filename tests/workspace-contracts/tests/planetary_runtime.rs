use engine_nav::{SphericalNavBuildConfig, SphericalNavGraph, SphericalSurfaceSample};
use engine_terrain::{
    desired_terrain_chunks, PlanetTerrainQuery, TerrainFace, TerrainTopology, TerrainVolume,
};
use glam::DVec3;

fn planetary_volume() -> TerrainVolume {
    TerrainVolume {
        topology: TerrainTopology::CubeSphere,
        planet_center: [1.0e12, -2.0e12, 3.0e12],
        planet_radius: 6_000_000.0,
        planet_max_lod: 3,
        chunk_size: 2_000.0,
        base_resolution: 17,
        height_scale: 500.0,
        lod_distances: vec![4_000.0, 16_000.0, 64_000.0, 256_000.0],
        lod_hysteresis: 500.0,
        ..TerrainVolume::default()
    }
}

#[test]
fn terrain_query_lod_and_spherical_navigation_share_one_f64_surface() {
    let volume = planetary_volume();
    volume.validate().expect("planet fixture must be valid");
    let query = PlanetTerrainQuery::new(&volume).expect("planet query must initialize");
    let center = DVec3::from_array(volume.planet_center);

    let focus = center + DVec3::new(volume.planet_radius + 1_000.0, 0.0, 0.0);
    let chunks = desired_terrain_chunks(&volume, focus.to_array());
    assert!(!chunks.is_empty());
    assert!(chunks
        .iter()
        .all(|chunk| chunk.id.face != TerrainFace::Planar));

    let graph = SphericalNavGraph::fibonacci_sampled(
        center,
        volume.planet_radius,
        SphericalNavBuildConfig {
            node_count: 512,
            neighbors_per_node: 8,
            ..SphericalNavBuildConfig::default()
        },
        |direction| {
            Some(SphericalSurfaceSample {
                position: DVec3::from_array(
                    query.surface_point_from_direction(direction.to_array()),
                ),
                traversal_cost: 1.0,
            })
        },
    )
    .expect("surface graph must build");

    let from = DVec3::from_array(query.surface_point_from_direction(DVec3::X.to_array()));
    let to = DVec3::from_array(query.surface_point_from_direction(DVec3::Z.to_array()));
    let path = graph.find_path(from, to).expect("surface path must exist");
    assert!(path.waypoints().len() >= 2);
    assert!(path.length().is_finite() && path.length() > 0.0);
    // At a 1e12 logical origin, one f64 ULP is roughly 0.1 mm. Keep the
    // contract tight enough to detect f32 truncation while allowing the
    // unavoidable subtract/reconstruct round-trip error.
    assert!(path
        .waypoints()
        .iter()
        .all(|point| query.altitude(point.to_array()).abs() < 1.0e-3));
}
