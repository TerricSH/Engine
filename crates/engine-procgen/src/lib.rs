//! Deterministic, versioned procedural-generation primitives (ENG-10).
//!
//! This crate is deliberately game-agnostic and dependency-light (serde
//! only). It provides the reusable building blocks every procedural system
//! (terrain, placement, variation) shares:
//!
//! - [`Seed`] and [`derive_seed`] — stable, versioned seed derivation from
//!   parent seeds and string keys;
//! - [`GradientNoise2D`] / [`GradientNoise3D`] — Perlin-style gradient noise;
//! - [`Fbm2D`] / [`Fbm3D`] — fractal Brownian motion wrappers with plain-data,
//!   serde-friendly parameter structs so "recipes" can live in data files;
//! - [`WarpedFbm2D`] / [`WarpedFbm3D`] — domain-warped variants.
//!
//! # Determinism guarantee
//!
//! Every sampler in this crate is **bit-for-bit deterministic across
//! platforms, endianness, compilers, and processes**: for the same schema
//! version, seed, parameters, and input coordinates, the returned `f32`
//! values have identical bit patterns everywhere. This holds because the
//! implementation uses only:
//!
//! - wrapping integer arithmetic on `u64`/`i64` (two's-complement semantics,
//!   identical in Rust and in the C# port under `unchecked`);
//! - exact IEEE-754 binary32 operations (`+`, `-`, `*` by values that never
//!   overflow the exact range, and correctly-rounded `*`/`+`/`/` in the fBm
//!   accumulation) with **no FMA contraction** and **no transcendental
//!   functions** (no `sin`/`cos`/`pow`/`exp`/`sqrt`);
//! - exact integer→`f32` conversions (all converted magnitudes are < 2^24)
//!   and exact power-of-two scaling.
//!
//! Coordinates are snapped to a Q24.8 fixed-point lattice (1/256 unit
//! resolution). The supported coordinate domain is |c| < 2^23 (after recipe
//! offsets/frequency scaling); larger finite magnitudes saturate
//! deterministically, and non-finite inputs yield exactly `0.0` — samplers
//! never return NaN.
//!
//! Any future change to an algorithm must bump [`PROCGEN_SCHEMA`] and
//! regenerate the golden vectors; the checked-in vectors in
//! `tests/golden_vectors.json` are the CI determinism gate. See
//! `docs/PROCGEN.md` for the full specification and versioning policy.

#![forbid(unsafe_code)]

mod fbm;
mod noise;
mod seed;

#[doc(hidden)]
pub mod golden;

pub use fbm::{Fbm2D, Fbm3D, FbmParams, WarpParams, WarpedFbm2D, WarpedFbm3D};
pub use noise::{GradientNoise2D, GradientNoise3D};
pub use seed::{derive_seed, Seed};

/// Version tag covering every algorithm in this crate.
///
/// The schema string is mixed into nothing implicitly; it exists so recipes,
/// golden vectors, and save files can record which algorithm family produced
/// them. Bump the version (e.g. `PROCGEN-v2`) whenever any hashing, noise,
/// fBm, or warp algorithm changes, and regenerate the golden vectors in the
/// same commit.
pub const PROCGEN_SCHEMA: &str = "PROCGEN-v1";

/// Errors returned when constructing parameterized samplers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcGenError {
    /// A parameter struct failed validation; the payload describes the rule.
    InvalidParams(&'static str),
}

impl std::fmt::Display for ProcGenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcGenError::InvalidParams(reason) => {
                write!(formatter, "invalid procgen parameters: {reason}")
            }
        }
    }
}

impl std::error::Error for ProcGenError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_versioned_and_stable() {
        assert_eq!(PROCGEN_SCHEMA, "PROCGEN-v1");
    }

    #[test]
    fn error_display_is_actionable() {
        let error = ProcGenError::InvalidParams("octaves must be in 1..=32");
        assert!(error.to_string().contains("octaves"));
    }
}
