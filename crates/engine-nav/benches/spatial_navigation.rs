use std::{hint::black_box, time::Instant};

use engine_nav::{SpaceCell, SpaceNavGrid, SphericalNavBuildConfig, SphericalNavGraph};
use glam::{DVec3, Vec3};

const PATH_ITERATIONS: usize = 100;

fn main() -> Result<(), String> {
    let space_grid = space_grid()?;
    let space_from = Vec3::new(0.5, 0.5, 0.5);
    let space_to = Vec3::new(63.5, 15.5, 63.5);
    space_grid
        .find_path(space_from, space_to)
        .map_err(|error| format!("space benchmark fixture has no route: {error}"))?;
    report("space_astar_64x16x64", PATH_ITERATIONS, || {
        let path = match space_grid.find_path(space_from, space_to) {
            Ok(path) => path,
            Err(_) => std::process::abort(),
        };
        black_box(path);
    });

    let spherical = SphericalNavGraph::fibonacci(
        DVec3::new(1.0e9, -2.0e9, 3.0e9),
        6_000_000.0,
        SphericalNavBuildConfig {
            node_count: 2_048,
            neighbors_per_node: 8,
            ..SphericalNavBuildConfig::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let center = spherical.center();
    let spherical_from = center + DVec3::X * 6_000_000.0;
    let spherical_to = center - DVec3::X * 6_000_000.0;
    spherical
        .find_path(spherical_from, spherical_to)
        .map_err(|error| format!("spherical benchmark fixture has no route: {error}"))?;
    report("spherical_astar_2048", PATH_ITERATIONS, || {
        let path = match spherical.find_path(spherical_from, spherical_to) {
            Ok(path) => path,
            Err(_) => std::process::abort(),
        };
        black_box(path);
    });

    Ok(())
}

fn space_grid() -> Result<SpaceNavGrid, String> {
    let mut grid =
        SpaceNavGrid::new(Vec3::ZERO, [64, 16, 64], 1.0).map_err(|error| error.to_string())?;
    for y in 0..16 {
        for z in 0..64 {
            if z != 31 {
                grid.set_blocked(SpaceCell { x: 32, y, z }, true);
            }
        }
    }
    Ok(grid)
}

fn report(name: &str, iterations: usize, mut operation: impl FnMut()) {
    for _ in 0..iterations.min(10) {
        operation();
    }
    let started = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    let elapsed = started.elapsed();
    let nanos_per_iteration = elapsed.as_nanos() as f64 / iterations as f64;
    println!(
        "benchmark={name} iterations={iterations} elapsed_ms={:.3} ns_per_iteration={nanos_per_iteration:.1}",
        elapsed.as_secs_f64() * 1_000.0
    );
}
