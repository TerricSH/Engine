//! Golden-vector determinism gate (ENG-10).
//!
//! The checked-in `golden_vectors.json` records exact IEEE-754 bit patterns
//! (`f32::to_bits`) for fixed seed/recipe/coordinate sets. This test
//! regenerates every vector from the current implementation and compares the
//! full document — a bit-level proof that results are identical across
//! platforms, processes, and reruns. The same file drives the C# parity
//! harness (`scripts/csharp/ProcGenParity`), so the managed port is held to
//! the same bits.

use engine_procgen::golden::{self, GoldenVectors, GOLDEN_SCHEMA};
use engine_procgen::{
    derive_seed, Fbm2D, Fbm3D, GradientNoise2D, GradientNoise3D, Seed, WarpedFbm2D, WarpedFbm3D,
    PROCGEN_SCHEMA,
};

const GOLDEN_JSON: &str = include_str!("golden_vectors.json");

fn checked_in() -> GoldenVectors {
    serde_json::from_str(GOLDEN_JSON).expect("golden_vectors.json must parse")
}

#[test]
fn golden_file_matches_current_schema_versions() {
    let vectors = checked_in();
    assert_eq!(vectors.schema, GOLDEN_SCHEMA);
    assert_eq!(
        vectors.procgen_schema, PROCGEN_SCHEMA,
        "golden vectors were generated for a different PROCGEN_SCHEMA; \
         regenerate with `cargo run -p engine-procgen --example generate_golden_vectors`"
    );
}

#[test]
fn regenerate_matches_checked_in_file_exactly() {
    let regenerated = golden::generate();
    let checked = checked_in();
    assert_eq!(
        regenerated, checked,
        "determinism gate failed: recomputed vectors differ from \
         tests/golden_vectors.json (either the algorithm changed without a \
         schema bump/regeneration, or this platform is not bit-stable)"
    );
}

#[test]
fn derive_seed_vectors_match_bit_exactly() {
    for vector in &checked_in().derive_seed {
        assert_eq!(
            derive_seed(Seed(vector.parent), &vector.key),
            Seed(vector.expected),
            "derive_seed({}, {:?})",
            vector.parent,
            vector.key
        );
    }
}

#[test]
fn noise2d_vectors_match_bit_exactly() {
    for vector in &checked_in().noise2d {
        let sampler = GradientNoise2D::new(Seed(vector.seed));
        for (coord, expected_bits) in vector.coords.iter().zip(&vector.expected_bits) {
            assert_eq!(
                sampler.sample(coord[0], coord[1]).to_bits(),
                *expected_bits,
                "noise2d seed={} coord={coord:?}",
                vector.seed
            );
        }
    }
}

#[test]
fn noise3d_vectors_match_bit_exactly() {
    for vector in &checked_in().noise3d {
        let sampler = GradientNoise3D::new(Seed(vector.seed));
        for (coord, expected_bits) in vector.coords.iter().zip(&vector.expected_bits) {
            assert_eq!(
                sampler.sample(coord[0], coord[1], coord[2]).to_bits(),
                *expected_bits,
                "noise3d seed={} coord={coord:?}",
                vector.seed
            );
        }
    }
}

#[test]
fn fbm_vectors_match_bit_exactly() {
    for vector in &checked_in().fbm2d {
        let sampler = Fbm2D::new(Seed(vector.seed), vector.params).unwrap();
        for (coord, expected_bits) in vector.coords.iter().zip(&vector.expected_bits) {
            assert_eq!(
                sampler.sample(coord[0], coord[1]).to_bits(),
                *expected_bits,
                "fbm2d seed={} coord={coord:?}",
                vector.seed
            );
        }
    }
    for vector in &checked_in().fbm3d {
        let sampler = Fbm3D::new(Seed(vector.seed), vector.params).unwrap();
        for (coord, expected_bits) in vector.coords.iter().zip(&vector.expected_bits) {
            assert_eq!(
                sampler.sample(coord[0], coord[1], coord[2]).to_bits(),
                *expected_bits,
                "fbm3d seed={} coord={coord:?}",
                vector.seed
            );
        }
    }
}

#[test]
fn domain_warp_vectors_match_bit_exactly() {
    for vector in &checked_in().warp2d {
        let fbm = Fbm2D::new(Seed(vector.seed), vector.fbm).unwrap();
        let sampler = WarpedFbm2D::new(fbm, vector.warp).unwrap();
        for (coord, expected_bits) in vector.coords.iter().zip(&vector.expected_bits) {
            assert_eq!(
                sampler.sample(coord[0], coord[1]).to_bits(),
                *expected_bits,
                "warp2d seed={} coord={coord:?}",
                vector.seed
            );
        }
    }
    for vector in &checked_in().warp3d {
        let fbm = Fbm3D::new(Seed(vector.seed), vector.fbm).unwrap();
        let sampler = WarpedFbm3D::new(fbm, vector.warp).unwrap();
        for (coord, expected_bits) in vector.coords.iter().zip(&vector.expected_bits) {
            assert_eq!(
                sampler.sample(coord[0], coord[1], coord[2]).to_bits(),
                *expected_bits,
                "warp3d seed={} coord={coord:?}",
                vector.seed
            );
        }
    }
}
