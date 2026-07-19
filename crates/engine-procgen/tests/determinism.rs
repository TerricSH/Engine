//! Determinism and robustness integration tests (ENG-10).
//!
//! Covers batch-vs-single equivalence, rerun stability, and edge-case input
//! handling beyond the unit tests in the crate modules.

use engine_procgen::{
    derive_seed, Fbm2D, Fbm3D, FbmParams, GradientNoise2D, GradientNoise3D, ProcGenError, Seed,
    WarpParams, WarpedFbm2D, WarpedFbm3D,
};

fn grid_2d() -> Vec<[f32; 2]> {
    let mut coords = Vec::new();
    for y in -32..32 {
        for x in -32..32 {
            coords.push([x as f32 * 0.173, y as f32 * 0.311]);
        }
    }
    coords
}

fn grid_3d() -> Vec<[f32; 3]> {
    let mut coords = Vec::new();
    for z in -8..8 {
        for y in -8..8 {
            for x in -8..8 {
                coords.push([x as f32 * 0.173, y as f32 * 0.311, z as f32 * 0.197]);
            }
        }
    }
    coords
}

#[test]
fn batch_sampling_matches_single_sampling_bit_for_bit() {
    let coords2 = grid_2d();
    let coords3 = grid_3d();

    let noise2 = GradientNoise2D::new(Seed(7));
    let mut batched = vec![0.0; coords2.len()];
    noise2.sample_batch(&coords2, &mut batched);
    for (coord, value) in coords2.iter().zip(&batched) {
        assert_eq!(noise2.sample(coord[0], coord[1]).to_bits(), value.to_bits());
    }

    let noise3 = GradientNoise3D::new(Seed(7));
    let mut batched = vec![0.0; coords3.len()];
    noise3.sample_batch(&coords3, &mut batched);
    for (coord, value) in coords3.iter().zip(&batched) {
        assert_eq!(
            noise3.sample(coord[0], coord[1], coord[2]).to_bits(),
            value.to_bits()
        );
    }

    let params = FbmParams::default();
    let fbm2 = Fbm2D::new(Seed(8), params).unwrap();
    let mut batched = vec![0.0; coords2.len()];
    fbm2.sample_batch(&coords2, &mut batched);
    for (coord, value) in coords2.iter().zip(&batched) {
        assert_eq!(fbm2.sample(coord[0], coord[1]).to_bits(), value.to_bits());
    }

    let fbm3 = Fbm3D::new(Seed(8), params).unwrap();
    let mut batched = vec![0.0; coords3.len()];
    fbm3.sample_batch(&coords3, &mut batched);
    for (coord, value) in coords3.iter().zip(&batched) {
        assert_eq!(
            fbm3.sample(coord[0], coord[1], coord[2]).to_bits(),
            value.to_bits()
        );
    }

    let warp2 = WarpedFbm2D::new(fbm2, WarpParams::default()).unwrap();
    let mut batched = vec![0.0; coords2.len()];
    warp2.sample_batch(&coords2, &mut batched);
    for (coord, value) in coords2.iter().zip(&batched) {
        assert_eq!(warp2.sample(coord[0], coord[1]).to_bits(), value.to_bits());
    }

    let warp3 = WarpedFbm3D::new(fbm3, WarpParams::default()).unwrap();
    let mut batched = vec![0.0; coords3.len()];
    warp3.sample_batch(&coords3, &mut batched);
    for (coord, value) in coords3.iter().zip(&batched) {
        assert_eq!(
            warp3.sample(coord[0], coord[1], coord[2]).to_bits(),
            value.to_bits()
        );
    }
}

#[test]
fn sampling_is_repeatable_within_and_across_sampler_instances() {
    // Two independently constructed samplers with identical recipes must
    // produce identical bit patterns (rerun/process determinism).
    let params = FbmParams {
        octaves: 6,
        frequency: 0.125,
        amplitude: 1.5,
        lacunarity: 2.0,
        gain: 0.5,
        offset: [3.25, -17.5, 0.0],
        normalize: true,
    };
    let first = WarpedFbm2D::new(
        Fbm2D::new(Seed(0xC0FFEE), params).unwrap(),
        WarpParams::default(),
    )
    .unwrap();
    let second = WarpedFbm2D::new(
        Fbm2D::new(Seed(0xC0FFEE), params).unwrap(),
        WarpParams::default(),
    )
    .unwrap();
    for coord in grid_2d() {
        assert_eq!(
            first.sample(coord[0], coord[1]).to_bits(),
            second.sample(coord[0], coord[1]).to_bits()
        );
    }
}

#[test]
fn edge_case_coordinates_never_produce_nan() {
    let params = FbmParams::default();
    let fbm2 = Fbm2D::new(Seed(1), params).unwrap();
    let fbm3 = Fbm3D::new(Seed(1), params).unwrap();
    let warp2 = WarpedFbm2D::new(fbm2, WarpParams::default()).unwrap();
    let warp3 = WarpedFbm3D::new(fbm3, WarpParams::default()).unwrap();
    let noise2 = GradientNoise2D::new(Seed(1));
    let noise3 = GradientNoise3D::new(Seed(1));

    let extremes = [
        0.0,
        -0.0,
        1e30,
        -1e30,
        8_388_607.0,
        -8_388_607.0,
        8_388_608.0,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ];
    for &x in &extremes {
        for &y in &extremes {
            assert!(noise2.sample(x, y).is_finite(), "noise2({x}, {y})");
            assert!(fbm2.sample(x, y).is_finite(), "fbm2({x}, {y})");
            assert!(warp2.sample(x, y).is_finite(), "warp2({x}, {y})");
            for &z in &extremes[..4] {
                assert!(noise3.sample(x, y, z).is_finite(), "noise3({x}, {y}, {z})");
                assert!(fbm3.sample(x, y, z).is_finite(), "fbm3({x}, {y}, {z})");
                assert!(warp3.sample(x, y, z).is_finite(), "warp3({x}, {y}, {z})");
            }
        }
    }
}

#[test]
fn non_finite_coordinates_yield_exact_zero_from_base_noise() {
    let noise2 = GradientNoise2D::new(Seed(9));
    assert_eq!(noise2.sample(f32::NAN, 0.0).to_bits(), 0.0f32.to_bits());
    assert_eq!(
        noise2.sample(0.0, f32::INFINITY).to_bits(),
        0.0f32.to_bits()
    );
    let noise3 = GradientNoise3D::new(Seed(9));
    assert_eq!(
        noise3.sample(0.0, 0.0, f32::NEG_INFINITY).to_bits(),
        0.0f32.to_bits()
    );
}

#[test]
fn zero_octaves_is_a_validation_error_not_a_nan() {
    let params = FbmParams {
        octaves: 0,
        ..FbmParams::default()
    };
    assert_eq!(
        Fbm2D::new(Seed(1), params).unwrap_err(),
        ProcGenError::InvalidParams("octaves must be in 1..=32")
    );
}

#[test]
fn derive_seed_lineage_is_stable() {
    // Namespace-style derivation chains are deterministic and order-sensitive.
    let world = Seed::ROOT.derive("world");
    let terrain_a = world.derive("terrain");
    let terrain_b = derive_seed(derive_seed(Seed::ROOT, "world"), "terrain");
    assert_eq!(terrain_a, terrain_b);
    assert_ne!(world.derive("terrain"), Seed::ROOT.derive("terrain"));
}

#[test]
#[should_panic(expected = "coords.len() == out.len()")]
fn batch_length_mismatch_panics() {
    let noise = GradientNoise2D::new(Seed(1));
    let coords = [[0.0, 0.0]; 4];
    let mut out = [0.0; 3];
    noise.sample_batch(&coords, &mut out);
}
