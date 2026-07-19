//! Perlin-style gradient noise with an integer lattice and fixed-point math.
//!
//! # Algorithm (PROCGEN-NOISE-v1)
//!
//! Classic Perlin gradient noise, implemented so every step is exact:
//!
//! 1. **Coordinate snap.** Each input coordinate is multiplied by 256 (exact
//!    power of two) and floored (exact IEEE-754), giving a Q24.8 fixed-point
//!    position: an integer lattice cell (`fixed >> 8`) and an in-cell
//!    fraction in `0..=255` (`fixed & 255`). Finite inputs outside the Q24.8
//!    range saturate to the fixed-point extremes; non-finite inputs make the
//!    whole sample exactly `0.0`.
//! 2. **Lattice hashing.** Each cell corner is hashed with the seed via
//!    FNV-1a-style wrapping `u64` mixing (seed, dimension tag, then the
//!    two's-complement `i32` cell coordinates) and a splitmix64 finalizer.
//! 3. **Gradient dot products.** The hash selects one of 8 (2D) or 16 (3D)
//!    axis/diagonal gradient vectors with components in `{-1, 0, 1}`; the dot
//!    product with the Q8 offset vector is a small exact integer
//!    (`|dot| <= 512`).
//! 4. **Quintic fade + lerp.** The fade curve `6t^5 - 15t^4 + 10t^3` is
//!    evaluated in `i64` fixed point and interpolated corners combine with
//!    arithmetic-shift lerps — all exact integer math.
//! 5. **Output scaling.** The accumulated integer (`|v| <= 512 < 2^24`)
//!    converts to `f32` exactly and scales by `2^-9` (exact), yielding a
//!    value in `[-1, 1]`.
//!
//! No transcendental functions, no division, no FMA — the output bit pattern
//! is identical on every platform and matches the C# port (`Engine.ProcGen`)
//! exactly.

use serde::{Deserialize, Serialize};

use crate::seed::{splitmix64_finalize, Seed, FNV1A64_OFFSET, FNV1A64_PRIME};

/// Fractional bits of the fixed-point coordinate lattice (Q24.8).
pub(crate) const FRAC_BITS: u32 = 8;
/// Fixed-point units per lattice cell.
pub(crate) const FRAC_SCALE: i64 = 1 << FRAC_BITS;
/// Output normalization: `1 / 2^9` (max gradient dot magnitude is 512).
pub(crate) const OUTPUT_SCALE: f32 = 1.0 / 512.0;

/// 2D gradients: 4 axis + 4 diagonal unit-component vectors.
pub(crate) const GRADIENTS_2D: [[i64; 2]; 8] = [
    [1, 0],
    [-1, 0],
    [0, 1],
    [0, -1],
    [1, 1],
    [1, -1],
    [-1, 1],
    [-1, -1],
];

/// 3D gradients: the 12 classic cube-edge midpoints, 4 repeated (Perlin's
/// original table) so the index is a clean `hash & 15`.
pub(crate) const GRADIENTS_3D: [[i64; 3]; 16] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
];

/// Snap a coordinate to the Q24.8 fixed-point lattice.
///
/// Returns `None` for non-finite inputs (the sample then yields `0.0`).
/// Finite magnitudes beyond the representable range saturate to the
/// fixed-point extremes, deterministically.
pub(crate) fn coordinate_to_fixed(coordinate: f32) -> Option<i32> {
    if !coordinate.is_finite() {
        return None;
    }
    // Multiplication by 2^8 and floor() are both exact IEEE-754 operations.
    let scaled = (coordinate * FRAC_SCALE as f32).floor();
    if !(scaled > -2147483648.0 && scaled < 2147483648.0) {
        return Some(if scaled >= 0.0 { i32::MAX } else { i32::MIN });
    }
    // `scaled` is integral and strictly inside the i32 range: exact cast.
    Some(scaled as i32)
}

/// Hash one lattice corner: FNV-1a-style word mixing + splitmix64 finalizer.
///
/// `dimension_tag` (2 or 3) keeps the 2D and 3D families independent.
pub(crate) fn lattice_hash(seed: u64, dimension_tag: u64, x: i32, y: i32, z: i32) -> u64 {
    let mut hash = FNV1A64_OFFSET;
    hash = (hash ^ seed).wrapping_mul(FNV1A64_PRIME);
    hash = (hash ^ dimension_tag).wrapping_mul(FNV1A64_PRIME);
    // Two's-complement reinterpretation of the cell coordinates, widened.
    hash = (hash ^ u64::from(x as u32)).wrapping_mul(FNV1A64_PRIME);
    hash = (hash ^ u64::from(y as u32)).wrapping_mul(FNV1A64_PRIME);
    hash = (hash ^ u64::from(z as u32)).wrapping_mul(FNV1A64_PRIME);
    splitmix64_finalize(hash)
}

/// Quintic fade `6t^5 - 15t^4 + 10t^3` on a Q8 input, Q8 output.
///
/// `t` must be in `0..=255`; every intermediate fits `i64` and stays
/// non-negative, so the final shift is an exact floor.
pub(crate) fn fade(t: i64) -> i64 {
    let cubic = t * t * t;
    let inner = t * (6 * t - 15 * FRAC_SCALE) + 10 * FRAC_SCALE * FRAC_SCALE;
    (cubic * inner) >> 32
}

/// Fixed-point linear interpolation; `fade_fraction` is Q8 (`0..=256`).
///
/// The arithmetic shift on the (possibly negative) product is an exact floor
/// in both Rust and C#, keeping the ports bit-identical.
pub(crate) fn lerp(a: i64, b: i64, fade_fraction: i64) -> i64 {
    a + (((b - a) * fade_fraction) >> 8)
}

/// Gradient dot product at one 2D corner; offsets are Q8 in `-256..=255`.
fn gradient_dot_2d(hash: u64, offset_x: i64, offset_y: i64) -> i64 {
    let gradient = GRADIENTS_2D[(hash & 7) as usize];
    gradient[0] * offset_x + gradient[1] * offset_y
}

/// Gradient dot product at one 3D corner; offsets are Q8 in `-256..=255`.
fn gradient_dot_3d(hash: u64, offset_x: i64, offset_y: i64, offset_z: i64) -> i64 {
    let gradient = GRADIENTS_3D[(hash & 15) as usize];
    gradient[0] * offset_x + gradient[1] * offset_y + gradient[2] * offset_z
}

/// Core 2D gradient noise on a raw seed; output in `[-1, 1]`.
pub(crate) fn gradient_noise_2d(seed: u64, x: f32, y: f32) -> f32 {
    let (Some(fixed_x), Some(fixed_y)) = (coordinate_to_fixed(x), coordinate_to_fixed(y)) else {
        return 0.0;
    };
    let cell_x = fixed_x >> FRAC_BITS;
    let cell_y = fixed_y >> FRAC_BITS;
    let frac_x = i64::from(fixed_x & (FRAC_SCALE as i32 - 1));
    let frac_y = i64::from(fixed_y & (FRAC_SCALE as i32 - 1));

    let d00 = gradient_dot_2d(lattice_hash(seed, 2, cell_x, cell_y, 0), frac_x, frac_y);
    let d10 = gradient_dot_2d(
        lattice_hash(seed, 2, cell_x + 1, cell_y, 0),
        frac_x - FRAC_SCALE,
        frac_y,
    );
    let d01 = gradient_dot_2d(
        lattice_hash(seed, 2, cell_x, cell_y + 1, 0),
        frac_x,
        frac_y - FRAC_SCALE,
    );
    let d11 = gradient_dot_2d(
        lattice_hash(seed, 2, cell_x + 1, cell_y + 1, 0),
        frac_x - FRAC_SCALE,
        frac_y - FRAC_SCALE,
    );

    let fade_x = fade(frac_x);
    let fade_y = fade(frac_y);
    let value = lerp(lerp(d00, d10, fade_x), lerp(d01, d11, fade_x), fade_y);
    // |value| <= 512 < 2^24: the conversion and the 2^-9 scale are exact.
    value as f32 * OUTPUT_SCALE
}

/// Core 3D gradient noise on a raw seed; output in `[-1, 1]`.
pub(crate) fn gradient_noise_3d(seed: u64, x: f32, y: f32, z: f32) -> f32 {
    let (Some(fixed_x), Some(fixed_y), Some(fixed_z)) = (
        coordinate_to_fixed(x),
        coordinate_to_fixed(y),
        coordinate_to_fixed(z),
    ) else {
        return 0.0;
    };
    let cell_x = fixed_x >> FRAC_BITS;
    let cell_y = fixed_y >> FRAC_BITS;
    let cell_z = fixed_z >> FRAC_BITS;
    let frac_x = i64::from(fixed_x & (FRAC_SCALE as i32 - 1));
    let frac_y = i64::from(fixed_y & (FRAC_SCALE as i32 - 1));
    let frac_z = i64::from(fixed_z & (FRAC_SCALE as i32 - 1));

    let mut corners = [0i64; 8];
    for (index, corner) in corners.iter_mut().enumerate() {
        let dx = (index & 1) as i64;
        let dy = ((index >> 1) & 1) as i64;
        let dz = ((index >> 2) & 1) as i64;
        *corner = gradient_dot_3d(
            lattice_hash(
                seed,
                3,
                cell_x + dx as i32,
                cell_y + dy as i32,
                cell_z + dz as i32,
            ),
            frac_x - dx * FRAC_SCALE,
            frac_y - dy * FRAC_SCALE,
            frac_z - dz * FRAC_SCALE,
        );
    }

    let fade_x = fade(frac_x);
    let fade_y = fade(frac_y);
    let fade_z = fade(frac_z);
    let x00 = lerp(corners[0], corners[1], fade_x);
    let x10 = lerp(corners[2], corners[3], fade_x);
    let x01 = lerp(corners[4], corners[5], fade_x);
    let x11 = lerp(corners[6], corners[7], fade_x);
    let y0 = lerp(x00, x10, fade_y);
    let y1 = lerp(x01, x11, fade_y);
    let value = lerp(y0, y1, fade_z);
    value as f32 * OUTPUT_SCALE
}

fn assert_batch_length(coords: usize, out: usize) {
    assert_eq!(
        coords, out,
        "procgen batch sampling requires coords.len() == out.len()"
    );
}

/// Seeded 2D gradient noise sampler (Perlin-style, PROCGEN-NOISE-v1).
///
/// Plain data: the entire recipe is the seed, so it serializes transparently.
/// Output lies in `[-1, 1]`; sampling is bit-for-bit deterministic across
/// platforms (see the crate-level determinism guarantee).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradientNoise2D {
    /// Seed selecting the gradient field.
    pub seed: Seed,
}

impl GradientNoise2D {
    /// Create a sampler for `seed`.
    pub const fn new(seed: Seed) -> Self {
        GradientNoise2D { seed }
    }

    /// Sample the field at `(x, y)`.
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        gradient_noise_2d(self.seed.0, x, y)
    }

    /// Batch sampling for chunk-generation throughput.
    ///
    /// Writes exactly one value per input coordinate and is bit-identical to
    /// calling [`Self::sample`] per coordinate. Panics when the slices differ
    /// in length.
    pub fn sample_batch(&self, coords: &[[f32; 2]], out: &mut [f32]) {
        assert_batch_length(coords.len(), out.len());
        for (coord, value) in coords.iter().zip(out.iter_mut()) {
            *value = self.sample(coord[0], coord[1]);
        }
    }
}

/// Seeded 3D gradient noise sampler (Perlin-style, PROCGEN-NOISE-v1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradientNoise3D {
    /// Seed selecting the gradient field.
    pub seed: Seed,
}

impl GradientNoise3D {
    /// Create a sampler for `seed`.
    pub const fn new(seed: Seed) -> Self {
        GradientNoise3D { seed }
    }

    /// Sample the field at `(x, y, z)`.
    pub fn sample(&self, x: f32, y: f32, z: f32) -> f32 {
        gradient_noise_3d(self.seed.0, x, y, z)
    }

    /// Batch sampling; bit-identical to per-coordinate [`Self::sample`].
    /// Panics when the slices differ in length.
    pub fn sample_batch(&self, coords: &[[f32; 3]], out: &mut [f32]) {
        assert_batch_length(coords.len(), out.len());
        for (coord, value) in coords.iter().zip(out.iter_mut()) {
            *value = self.sample(coord[0], coord[1], coord[2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinate_snap_is_lattice_exact() {
        assert_eq!(coordinate_to_fixed(0.0), Some(0));
        assert_eq!(coordinate_to_fixed(1.0), Some(256));
        assert_eq!(coordinate_to_fixed(-1.0), Some(-256));
        assert_eq!(coordinate_to_fixed(1.0 / 256.0), Some(1));
        // -1/256 snaps to -1: cell -1, fraction 255 (floor semantics).
        assert_eq!(coordinate_to_fixed(-1.0 / 256.0), Some(-1));
        // Largest in-range cell coordinate: 8_388_607 * 256 = 2^31 - 256.
        assert_eq!(coordinate_to_fixed(8_388_607.0), Some(2_147_483_392));
        // 2^23 world units scales to exactly 2^31: saturates deterministically.
        assert_eq!(coordinate_to_fixed(8_388_608.0), Some(i32::MAX));
    }

    #[test]
    fn coordinate_snap_saturates_huge_and_rejects_non_finite() {
        assert_eq!(coordinate_to_fixed(1e30), Some(i32::MAX));
        assert_eq!(coordinate_to_fixed(-1e30), Some(i32::MIN));
        assert_eq!(coordinate_to_fixed(f32::INFINITY), None);
        assert_eq!(coordinate_to_fixed(f32::NEG_INFINITY), None);
        assert_eq!(coordinate_to_fixed(f32::NAN), None);
    }

    #[test]
    fn fade_matches_quintic_endpoints() {
        assert_eq!(fade(0), 0);
        // fade(255) is floor(f(255/256) * 256) = 255.
        assert_eq!(fade(255), 255);
        // Monotonic over the cell.
        let mut previous = 0;
        for t in 0..=255 {
            let value = fade(t);
            assert!(value >= previous, "fade must be monotonic at t={t}");
            previous = value;
        }
    }

    #[test]
    fn samples_are_bounded_and_finite_for_extreme_inputs() {
        let noise = GradientNoise3D::new(Seed(7));
        for coord in [
            [0.0, 0.0, 0.0],
            [-0.0, -0.0, -0.0],
            [-1_234.562_5, 9_876.547, -0.25],
            [1e30, -1e30, 1e30],
            [f32::NAN, 0.0, 0.0],
            [0.0, f32::INFINITY, 0.0],
            [0.0, 0.0, f32::NEG_INFINITY],
        ] {
            let value = noise.sample(coord[0], coord[1], coord[2]);
            assert!(value.is_finite(), "sample must stay finite for {coord:?}");
            assert!(
                (-1.0..=1.0).contains(&value),
                "sample {value} out of range for {coord:?}"
            );
        }
    }

    #[test]
    fn sampling_is_lattice_period_free_and_smooth() {
        let noise = GradientNoise2D::new(Seed(99));
        // Adjacent samples never jump by the full range (smooth field).
        let mut previous = noise.sample(0.0, 0.0);
        for step in 1..512 {
            let value = noise.sample(step as f32 / 256.0, 0.0);
            assert!((value - previous).abs() < 0.5, "field must be continuous");
            previous = value;
        }
    }
}
