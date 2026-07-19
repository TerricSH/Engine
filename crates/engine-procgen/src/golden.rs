//! Golden determinism vectors (CI gate).
//!
//! This module defines the fixed input sets (seeds, recipes, coordinates) and
//! the JSON wire format shared by:
//!
//! - `examples/generate_golden_vectors.rs`, which regenerates
//!   `tests/golden_vectors.json` (run it and redirect stdout after any
//!   intentional, schema-bumping algorithm change);
//! - `tests/golden_vectors.rs`, which recomputes every vector and compares
//!   exact `f32::to_bits` patterns against the checked-in file;
//! - the C# parity harness (`scripts/csharp/ProcGenParity`), which checks the
//!   managed port against the same file.
//!
//! It is `#[doc(hidden)]`: part of the engineering determinism gate, not the
//! game-facing API.

use serde::{Deserialize, Serialize};

use crate::{
    Fbm2D, Fbm3D, FbmParams, GradientNoise2D, GradientNoise3D, Seed, WarpParams, WarpedFbm2D,
    WarpedFbm3D,
};

/// Version of the golden vector file format itself.
pub const GOLDEN_SCHEMA: &str = "PROCGEN-GOLDEN-v1";

/// Root of the checked-in golden vector file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoldenVectors {
    pub schema: String,
    pub procgen_schema: String,
    pub derive_seed: Vec<DeriveSeedVector>,
    pub noise2d: Vec<Noise2DVector>,
    pub noise3d: Vec<Noise3DVector>,
    pub fbm2d: Vec<Fbm2DVector>,
    pub fbm3d: Vec<Fbm3DVector>,
    pub warp2d: Vec<Warp2DVector>,
    pub warp3d: Vec<Warp3DVector>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeriveSeedVector {
    pub parent: u64,
    pub key: String,
    pub expected: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Noise2DVector {
    pub seed: u64,
    pub coords: Vec<[f32; 2]>,
    /// Expected outputs as exact IEEE-754 bit patterns (`f32::to_bits`).
    pub expected_bits: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Noise3DVector {
    pub seed: u64,
    pub coords: Vec<[f32; 3]>,
    pub expected_bits: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fbm2DVector {
    pub seed: u64,
    pub params: FbmParams,
    pub coords: Vec<[f32; 2]>,
    pub expected_bits: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fbm3DVector {
    pub seed: u64,
    pub params: FbmParams,
    pub coords: Vec<[f32; 3]>,
    pub expected_bits: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warp2DVector {
    pub seed: u64,
    pub fbm: FbmParams,
    pub warp: WarpParams,
    pub coords: Vec<[f32; 2]>,
    pub expected_bits: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warp3DVector {
    pub seed: u64,
    pub fbm: FbmParams,
    pub warp: WarpParams,
    pub coords: Vec<[f32; 3]>,
    pub expected_bits: Vec<u32>,
}

/// Fixed `(parent, key)` cases for seed derivation.
pub fn derive_seed_cases() -> Vec<(u64, String)> {
    vec![
        (0, String::new()),
        (0, "terrain".to_string()),
        (0, "procgen/warp/2d/x".to_string()),
        (1, "terrain".to_string()),
        (u64::MAX, "terrain".to_string()),
        (0x0071_5EED_5EA0_C1D2, "soak".to_string()),
        (0xDEAD_BEEF_CAFE_F00D, "地形/🌲".to_string()),
        (
            42,
            "a very long namespaced key/with/slashes/and.dots/0123456789".to_string(),
        ),
    ]
}

/// Fixed 2D coordinate set reused by the 2D noise/fBm/warp vectors.
pub fn coords_2d() -> Vec<[f32; 2]> {
    vec![
        [0.0, 0.0],
        [-0.0, -0.0],
        [1.0, 1.0],
        [-1.0, -1.0],
        [0.5, -0.25],
        [1.0 / 256.0, -1.0 / 256.0],
        [255.0 / 256.0, 1.0 + 255.0 / 256.0],
        [-1024.5, 2048.25],
        [1_234.562_5, -9_876.547],
        [65_535.996, -65_535.996],
        [8_388_000.0, -8_388_000.0],
        [3.140625, 2.71875],
        [-0.1, 0.1],
        [1e30, -1e30],
        [123_456.79, 987_654.3],
        [-7.75, 8_191.996],
    ]
}

/// Fixed 3D coordinate set reused by the 3D noise/fBm/warp vectors.
pub fn coords_3d() -> Vec<[f32; 3]> {
    vec![
        [0.0, 0.0, 0.0],
        [1.0, -1.0, 0.5],
        [-0.5, 0.25, -0.125],
        [1.0 / 256.0, 2.0 / 256.0, -3.0 / 256.0],
        [100.25, -200.5, 300.75],
        [-4096.0, 4096.0, -4096.0],
        [1_234.562_5, 0.0, -9_876.547],
        [8_388_000.0, 8_388_000.0, -8_388_000.0],
        [1e30, -1e30, 1e30],
        [-3.140625, 2.71875, 1.414_062_5],
        [0.1, -0.1, 0.2],
        [65_535.996, 0.5, -0.5],
    ]
}

/// Fixed fBm recipes: the default recipe, an unnormalized high-octave
/// recipe, and an offset low-frequency recipe.
pub fn fbm_recipes() -> Vec<FbmParams> {
    vec![
        FbmParams::default(),
        FbmParams {
            octaves: 8,
            frequency: 0.03125,
            amplitude: 2.0,
            lacunarity: 2.5,
            gain: 0.45,
            offset: [0.0; 3],
            normalize: false,
        },
        FbmParams {
            octaves: 1,
            frequency: 0.5,
            amplitude: 0.75,
            lacunarity: 3.0,
            gain: 1.0,
            offset: [13.5, -7.25, 101.0],
            normalize: true,
        },
    ]
}

/// Fixed warp recipes.
pub fn warp_recipes() -> Vec<WarpParams> {
    vec![
        WarpParams::default(),
        WarpParams {
            amplitude: 4.0,
            frequency: 0.25,
        },
    ]
}

/// Recompute every golden vector from the current implementation.
///
/// The example prints this as JSON; the determinism test compares the
/// checked-in file against it field by field.
pub fn generate() -> GoldenVectors {
    let derive_seed = derive_seed_cases()
        .into_iter()
        .map(|(parent, key)| DeriveSeedVector {
            parent,
            expected: crate::derive_seed(Seed(parent), &key).0,
            key,
        })
        .collect();

    let noise2d = [0u64, 1, 0x5EED_5EED_5EED_5EED]
        .into_iter()
        .map(|seed| {
            let sampler = GradientNoise2D::new(Seed(seed));
            let coords = coords_2d();
            let mut expected = vec![0.0; coords.len()];
            sampler.sample_batch(&coords, &mut expected);
            Noise2DVector {
                seed,
                coords,
                expected_bits: expected.iter().map(|value| value.to_bits()).collect(),
            }
        })
        .collect();

    let noise3d = [0u64, 1, 0x5EED_5EED_5EED_5EED]
        .into_iter()
        .map(|seed| {
            let sampler = GradientNoise3D::new(Seed(seed));
            let coords = coords_3d();
            let mut expected = vec![0.0; coords.len()];
            sampler.sample_batch(&coords, &mut expected);
            Noise3DVector {
                seed,
                coords,
                expected_bits: expected.iter().map(|value| value.to_bits()).collect(),
            }
        })
        .collect();

    let fbm2d = fbm_recipes()
        .into_iter()
        .enumerate()
        .map(|(index, params)| {
            let seed = 0xF00D_0000 + index as u64;
            let sampler = Fbm2D::new(Seed(seed), params).expect("golden fbm recipe is valid");
            let coords = coords_2d();
            let mut expected = vec![0.0; coords.len()];
            sampler.sample_batch(&coords, &mut expected);
            Fbm2DVector {
                seed,
                params,
                coords,
                expected_bits: expected.iter().map(|value| value.to_bits()).collect(),
            }
        })
        .collect();

    let fbm3d = fbm_recipes()
        .into_iter()
        .enumerate()
        .map(|(index, params)| {
            let seed = 0xF00D_1000 + index as u64;
            let sampler = Fbm3D::new(Seed(seed), params).expect("golden fbm recipe is valid");
            let coords = coords_3d();
            let mut expected = vec![0.0; coords.len()];
            sampler.sample_batch(&coords, &mut expected);
            Fbm3DVector {
                seed,
                params,
                coords,
                expected_bits: expected.iter().map(|value| value.to_bits()).collect(),
            }
        })
        .collect();

    let warp2d = warp_recipes()
        .into_iter()
        .enumerate()
        .map(|(index, warp)| {
            let seed = 0xF00D_2000 + index as u64;
            let sampler = WarpedFbm2D::new(
                Fbm2D::new(Seed(seed), FbmParams::default()).expect("golden fbm recipe is valid"),
                warp,
            )
            .expect("golden warp recipe is valid");
            let coords = coords_2d();
            let mut expected = vec![0.0; coords.len()];
            sampler.sample_batch(&coords, &mut expected);
            Warp2DVector {
                seed,
                fbm: FbmParams::default(),
                warp,
                coords,
                expected_bits: expected.iter().map(|value| value.to_bits()).collect(),
            }
        })
        .collect();

    let warp3d = warp_recipes()
        .into_iter()
        .enumerate()
        .map(|(index, warp)| {
            let seed = 0xF00D_3000 + index as u64;
            let sampler = WarpedFbm3D::new(
                Fbm3D::new(Seed(seed), FbmParams::default()).expect("golden fbm recipe is valid"),
                warp,
            )
            .expect("golden warp recipe is valid");
            let coords = coords_3d();
            let mut expected = vec![0.0; coords.len()];
            sampler.sample_batch(&coords, &mut expected);
            Warp3DVector {
                seed,
                fbm: FbmParams::default(),
                warp,
                coords,
                expected_bits: expected.iter().map(|value| value.to_bits()).collect(),
            }
        })
        .collect();

    GoldenVectors {
        schema: GOLDEN_SCHEMA.to_string(),
        procgen_schema: crate::PROCGEN_SCHEMA.to_string(),
        derive_seed,
        noise2d,
        noise3d,
        fbm2d,
        fbm3d,
        warp2d,
        warp3d,
    }
}
