use std::{hint::black_box, time::Instant};

use engine_terrain::{desired_terrain_chunks, PlanetTerrainQuery, TerrainTopology, TerrainVolume};

const QUERY_ITERATIONS: usize = 200_000;
const LOD_ITERATIONS: usize = 2_000;

fn main() -> Result<(), String> {
    let volume = benchmark_volume();
    volume.validate().map_err(|error| error.to_string())?;
    let query = PlanetTerrainQuery::new(&volume)?;
    let directions = fibonacci_directions(4_096);

    report("planet_surface_query", QUERY_ITERATIONS, || {
        let direction = directions[black_box(next_index(directions.len()))];
        black_box(query.surface_point_from_direction(direction));
    });

    let mut focus_angle = 0.0_f64;
    report("cube_sphere_lod_selection", LOD_ITERATIONS, || {
        focus_angle += 0.003;
        let radius = volume.planet_radius + 2_500.0;
        let focus = [
            volume.planet_center[0] + focus_angle.cos() * radius,
            volume.planet_center[1] + focus_angle.sin() * radius * 0.2,
            volume.planet_center[2] + focus_angle.sin() * radius,
        ];
        black_box(desired_terrain_chunks(&volume, focus));
    });

    Ok(())
}

fn benchmark_volume() -> TerrainVolume {
    TerrainVolume {
        topology: TerrainTopology::CubeSphere,
        planet_center: [1.0e9, -2.0e9, 3.0e9],
        planet_radius: 6_000_000.0,
        planet_max_lod: 7,
        chunk_size: 1_000.0,
        base_resolution: 33,
        height_scale: 2_000.0,
        lod_distances: vec![
            2_000.0, 4_000.0, 8_000.0, 16_000.0, 32_000.0, 64_000.0, 128_000.0, 256_000.0,
        ],
        lod_hysteresis: 250.0,
        ..TerrainVolume::default()
    }
}

fn fibonacci_directions(count: usize) -> Vec<[f64; 3]> {
    let golden_angle = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    (0..count)
        .map(|index| {
            let y = 1.0 - 2.0 * (index as f64 + 0.5) / count as f64;
            let radial = (1.0 - y * y).sqrt();
            let angle = golden_angle * index as f64;
            [radial * angle.cos(), y, radial * angle.sin()]
        })
        .collect()
}

fn next_index(modulus: usize) -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static INDEX: AtomicUsize = AtomicUsize::new(0);
    INDEX.fetch_add(1, Ordering::Relaxed) % modulus
}

fn report(name: &str, iterations: usize, mut operation: impl FnMut()) {
    for _ in 0..iterations.min(1_000) {
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
