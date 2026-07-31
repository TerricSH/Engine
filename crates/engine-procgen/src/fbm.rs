//! Fractal Brownian motion (fBm) and domain-warp wrappers (PROCGEN-FBM-v1).
//!
//! fBm sums `octaves` of gradient noise with per-octave frequency/amplitude
//! scaling:
//!
//! ```text
//! fx = (x + offset.x) * frequency;  fy = ...
//! sum  = Σ amplitude_i * noise(seed, fx_i, fy_i)
//! fx_{i+1} = fx_i * lacunarity;  amplitude_{i+1} = amplitude_i * gain
//! out  = normalize ? sum / Σ amplitude_i : sum
//! ```
//!
//! Every operation is a single correctly-rounded IEEE-754 binary32 `+`, `*`,
//! or final `/` — no FMA contraction, no transcendental functions — so the
//! accumulation order fixed above reproduces bit-for-bit on every platform
//! and in the C# port. The noise inputs themselves satisfy the crate-level
//! fixed-point guarantee, so non-finite or huge intermediate coordinates
//! degrade to deterministic `0.0` contributions instead of NaN.
//!
//! Domain warp displaces the fBm input by two (or three) independent gradient
//! noise channels derived from the base seed via [`derive_seed`] with
//! versioned keys (`procgen/warp/2d/x`, ...), then evaluates the fBm at the
//! warped position.
//!
//! All knobs live in plain serde data structs ([`FbmParams`], [`WarpParams`])
//! so complete recipes can be authored as data files.

use serde::{Deserialize, Serialize};

use crate::noise::{
    gradient_noise_2d, gradient_noise_2d_wide, gradient_noise_3d, gradient_noise_3d_wide,
};
use crate::seed::{derive_seed, Seed};
use crate::ProcGenError;

/// Warp channel derivation keys (part of the PROCGEN-v1 algorithm contract).
const WARP_2D_X_KEY: &str = "procgen/warp/2d/x";
const WARP_2D_Y_KEY: &str = "procgen/warp/2d/y";
const WARP_3D_X_KEY: &str = "procgen/warp/3d/x";
const WARP_3D_Y_KEY: &str = "procgen/warp/3d/y";
const WARP_3D_Z_KEY: &str = "procgen/warp/3d/z";

/// fBm recipe parameters (PROCGEN-FBM-v1).
///
/// All fields are validated by [`FbmParams::validate`]; the sampler
/// constructors reject invalid recipes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FbmParams {
    /// Number of noise octaves, `1..=32`.
    pub octaves: u32,
    /// Base frequency applied to the (offset) input coordinates, finite and
    /// in `(0, 65536]`.
    pub frequency: f32,
    /// First-octave amplitude, finite and in `[0, 65536]`.
    pub amplitude: f32,
    /// Per-octave frequency multiplier, finite and in `(0, 16]`.
    pub lacunarity: f32,
    /// Per-octave amplitude multiplier (persistence), finite and in `[0, 1]`.
    pub gain: f32,
    /// Coordinate offset added before frequency scaling (x/y for 2D, x/y/z
    /// for 3D). Every component must be finite.
    pub offset: [f32; 3],
    /// Divide the octave sum by the total amplitude, keeping the output
    /// amplitude stable across octave counts.
    pub normalize: bool,
}

impl Default for FbmParams {
    fn default() -> Self {
        FbmParams {
            octaves: 4,
            frequency: 1.0,
            amplitude: 1.0,
            lacunarity: 2.0,
            gain: 0.5,
            offset: [0.0; 3],
            normalize: true,
        }
    }
}

impl FbmParams {
    /// Validate the recipe; sampler constructors call this.
    pub fn validate(&self) -> Result<(), ProcGenError> {
        if !(1..=32).contains(&self.octaves) {
            return Err(ProcGenError::InvalidParams("octaves must be in 1..=32"));
        }
        validate_unit_interval(self.gain, "gain must be finite and in [0, 1]")?;
        if !self.frequency.is_finite() || self.frequency <= 0.0 || self.frequency > 65536.0 {
            return Err(ProcGenError::InvalidParams(
                "frequency must be finite and in (0, 65536]",
            ));
        }
        if !self.amplitude.is_finite() || self.amplitude < 0.0 || self.amplitude > 65536.0 {
            return Err(ProcGenError::InvalidParams(
                "amplitude must be finite and in [0, 65536]",
            ));
        }
        if !self.lacunarity.is_finite() || self.lacunarity <= 0.0 || self.lacunarity > 16.0 {
            return Err(ProcGenError::InvalidParams(
                "lacunarity must be finite and in (0, 16]",
            ));
        }
        if self.offset.iter().any(|value| !value.is_finite()) {
            return Err(ProcGenError::InvalidParams(
                "offset components must be finite",
            ));
        }
        Ok(())
    }
}

fn validate_unit_interval(value: f32, reason: &'static str) -> Result<(), ProcGenError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ProcGenError::InvalidParams(reason));
    }
    Ok(())
}

/// Domain-warp recipe parameters (PROCGEN-WARP-v1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpParams {
    /// Displacement magnitude in input-coordinate units, finite and in
    /// `[0, 65536]`.
    pub amplitude: f32,
    /// Frequency of the warp field, finite and in `(0, 65536]`.
    pub frequency: f32,
}

impl Default for WarpParams {
    fn default() -> Self {
        WarpParams {
            amplitude: 1.0,
            frequency: 1.0,
        }
    }
}

impl WarpParams {
    /// Validate the warp recipe; sampler constructors call this.
    pub fn validate(&self) -> Result<(), ProcGenError> {
        if !self.amplitude.is_finite() || self.amplitude < 0.0 || self.amplitude > 65536.0 {
            return Err(ProcGenError::InvalidParams(
                "warp amplitude must be finite and in [0, 65536]",
            ));
        }
        if !self.frequency.is_finite() || self.frequency <= 0.0 || self.frequency > 65536.0 {
            return Err(ProcGenError::InvalidParams(
                "warp frequency must be finite and in (0, 65536]",
            ));
        }
        Ok(())
    }
}

/// Seeded 2D fBm sampler over [`crate::GradientNoise2D`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fbm2D {
    /// Base seed; every octave samples the same field at scaled coordinates.
    pub seed: Seed,
    /// Recipe parameters (validated at construction).
    pub params: FbmParams,
}

impl Fbm2D {
    /// Create a sampler, validating the recipe.
    pub fn new(seed: Seed, params: FbmParams) -> Result<Self, ProcGenError> {
        params.validate()?;
        Ok(Fbm2D { seed, params })
    }

    /// Sample the fBm at `(x, y)`. Never returns NaN.
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let params = &self.params;
        let mut sum = 0.0f32;
        let mut amplitude = params.amplitude;
        let mut total = 0.0f32;
        let mut fx = (x + params.offset[0]) * params.frequency;
        let mut fy = (y + params.offset[1]) * params.frequency;
        for _ in 0..params.octaves {
            sum += amplitude * gradient_noise_2d(self.seed.0, fx, fy);
            total += amplitude;
            fx *= params.lacunarity;
            fy *= params.lacunarity;
            amplitude *= params.gain;
        }
        if params.normalize && total > 0.0 {
            sum / total
        } else {
            sum
        }
    }

    /// Sample using f64 logical coordinates and an extended i64 lattice.
    ///
    /// Intended for native floating-origin systems. This does not alter the
    /// versioned f32/script sampling contract or its golden vectors.
    pub fn sample_wide(&self, x: f64, y: f64) -> f32 {
        let params = &self.params;
        let mut sum = 0.0f32;
        let mut amplitude = params.amplitude;
        let mut total = 0.0f32;
        let mut fx = (x + f64::from(params.offset[0])) * f64::from(params.frequency);
        let mut fy = (y + f64::from(params.offset[1])) * f64::from(params.frequency);
        for _ in 0..params.octaves {
            sum += amplitude * gradient_noise_2d_wide(self.seed.0, fx, fy);
            total += amplitude;
            fx *= f64::from(params.lacunarity);
            fy *= f64::from(params.lacunarity);
            amplitude *= params.gain;
        }
        if params.normalize && total > 0.0 {
            sum / total
        } else {
            sum
        }
    }

    /// Batch sampling; bit-identical to per-coordinate [`Self::sample`].
    /// Panics when the slices differ in length.
    pub fn sample_batch(&self, coords: &[[f32; 2]], out: &mut [f32]) {
        assert_eq!(
            coords.len(),
            out.len(),
            "procgen batch sampling requires coords.len() == out.len()"
        );
        for (coord, value) in coords.iter().zip(out.iter_mut()) {
            *value = self.sample(coord[0], coord[1]);
        }
    }
}

/// Seeded 3D fBm sampler over [`crate::GradientNoise3D`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fbm3D {
    /// Base seed; every octave samples the same field at scaled coordinates.
    pub seed: Seed,
    /// Recipe parameters (validated at construction).
    pub params: FbmParams,
}

impl Fbm3D {
    /// Create a sampler, validating the recipe.
    pub fn new(seed: Seed, params: FbmParams) -> Result<Self, ProcGenError> {
        params.validate()?;
        Ok(Fbm3D { seed, params })
    }

    /// Sample the fBm at `(x, y, z)`. Never returns NaN.
    pub fn sample(&self, x: f32, y: f32, z: f32) -> f32 {
        let params = &self.params;
        let mut sum = 0.0f32;
        let mut amplitude = params.amplitude;
        let mut total = 0.0f32;
        let mut fx = (x + params.offset[0]) * params.frequency;
        let mut fy = (y + params.offset[1]) * params.frequency;
        let mut fz = (z + params.offset[2]) * params.frequency;
        for _ in 0..params.octaves {
            sum += amplitude * gradient_noise_3d(self.seed.0, fx, fy, fz);
            total += amplitude;
            fx *= params.lacunarity;
            fy *= params.lacunarity;
            fz *= params.lacunarity;
            amplitude *= params.gain;
        }
        if params.normalize && total > 0.0 {
            sum / total
        } else {
            sum
        }
    }

    /// Sample with f64 logical coordinates and the extended i64 lattice.
    ///
    /// Planetary terrain uses this path so a large radius or floating-world
    /// coordinate does not collapse nearby samples in an f32 mantissa.
    pub fn sample_wide(&self, x: f64, y: f64, z: f64) -> f32 {
        let params = &self.params;
        let mut sum = 0.0f32;
        let mut amplitude = params.amplitude;
        let mut total = 0.0f32;
        let mut fx = (x + f64::from(params.offset[0])) * f64::from(params.frequency);
        let mut fy = (y + f64::from(params.offset[1])) * f64::from(params.frequency);
        let mut fz = (z + f64::from(params.offset[2])) * f64::from(params.frequency);
        for _ in 0..params.octaves {
            sum += amplitude * gradient_noise_3d_wide(self.seed.0, fx, fy, fz);
            total += amplitude;
            fx *= f64::from(params.lacunarity);
            fy *= f64::from(params.lacunarity);
            fz *= f64::from(params.lacunarity);
            amplitude *= params.gain;
        }
        if params.normalize && total > 0.0 {
            sum / total
        } else {
            sum
        }
    }

    /// Batch sampling; bit-identical to per-coordinate [`Self::sample`].
    /// Panics when the slices differ in length.
    pub fn sample_batch(&self, coords: &[[f32; 3]], out: &mut [f32]) {
        assert_eq!(
            coords.len(),
            out.len(),
            "procgen batch sampling requires coords.len() == out.len()"
        );
        for (coord, value) in coords.iter().zip(out.iter_mut()) {
            *value = self.sample(coord[0], coord[1], coord[2]);
        }
    }
}

/// Domain-warped 2D fBm (PROCGEN-WARP-v1).
///
/// Samples two independent gradient-noise channels (seeds derived from the
/// fBm seed) and evaluates the fBm at the displaced position.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpedFbm2D {
    /// Base fBm recipe.
    pub fbm: Fbm2D,
    /// Warp field parameters.
    pub warp: WarpParams,
}

impl WarpedFbm2D {
    /// Create a sampler, validating both recipes.
    pub fn new(fbm: Fbm2D, warp: WarpParams) -> Result<Self, ProcGenError> {
        warp.validate()?;
        Ok(WarpedFbm2D { fbm, warp })
    }

    /// Sample the warped fBm at `(x, y)`. Never returns NaN.
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let seed = self.fbm.seed;
        let wx = x * self.warp.frequency;
        let wy = y * self.warp.frequency;
        let qx =
            x + self.warp.amplitude * gradient_noise_2d(derive_seed(seed, WARP_2D_X_KEY).0, wx, wy);
        let qy =
            y + self.warp.amplitude * gradient_noise_2d(derive_seed(seed, WARP_2D_Y_KEY).0, wx, wy);
        self.fbm.sample(qx, qy)
    }

    /// Wide-coordinate domain-warped sampling for native floating-origin
    /// systems. The warp displacement remains f32 recipe data, while logical
    /// position and lattice selection retain f64/i64 precision.
    pub fn sample_wide(&self, x: f64, y: f64) -> f32 {
        let seed = self.fbm.seed;
        let wx = x * f64::from(self.warp.frequency);
        let wy = y * f64::from(self.warp.frequency);
        let qx = x + f64::from(self.warp.amplitude)
            * f64::from(gradient_noise_2d_wide(
                derive_seed(seed, WARP_2D_X_KEY).0,
                wx,
                wy,
            ));
        let qy = y + f64::from(self.warp.amplitude)
            * f64::from(gradient_noise_2d_wide(
                derive_seed(seed, WARP_2D_Y_KEY).0,
                wx,
                wy,
            ));
        self.fbm.sample_wide(qx, qy)
    }

    /// Batch sampling; bit-identical to per-coordinate [`Self::sample`].
    /// Panics when the slices differ in length.
    pub fn sample_batch(&self, coords: &[[f32; 2]], out: &mut [f32]) {
        assert_eq!(
            coords.len(),
            out.len(),
            "procgen batch sampling requires coords.len() == out.len()"
        );
        for (coord, value) in coords.iter().zip(out.iter_mut()) {
            *value = self.sample(coord[0], coord[1]);
        }
    }
}

/// Domain-warped 3D fBm (PROCGEN-WARP-v1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpedFbm3D {
    /// Base fBm recipe.
    pub fbm: Fbm3D,
    /// Warp field parameters.
    pub warp: WarpParams,
}

impl WarpedFbm3D {
    /// Create a sampler, validating both recipes.
    pub fn new(fbm: Fbm3D, warp: WarpParams) -> Result<Self, ProcGenError> {
        warp.validate()?;
        Ok(WarpedFbm3D { fbm, warp })
    }

    /// Sample the warped fBm at `(x, y, z)`. Never returns NaN.
    pub fn sample(&self, x: f32, y: f32, z: f32) -> f32 {
        let seed = self.fbm.seed;
        let wx = x * self.warp.frequency;
        let wy = y * self.warp.frequency;
        let wz = z * self.warp.frequency;
        let qx = x + self.warp.amplitude
            * gradient_noise_3d(derive_seed(seed, WARP_3D_X_KEY).0, wx, wy, wz);
        let qy = y + self.warp.amplitude
            * gradient_noise_3d(derive_seed(seed, WARP_3D_Y_KEY).0, wx, wy, wz);
        let qz = z + self.warp.amplitude
            * gradient_noise_3d(derive_seed(seed, WARP_3D_Z_KEY).0, wx, wy, wz);
        self.fbm.sample(qx, qy, qz)
    }

    /// Wide-coordinate domain-warped sampling for native planetary terrain.
    pub fn sample_wide(&self, x: f64, y: f64, z: f64) -> f32 {
        let seed = self.fbm.seed;
        let wx = x * f64::from(self.warp.frequency);
        let wy = y * f64::from(self.warp.frequency);
        let wz = z * f64::from(self.warp.frequency);
        let amplitude = f64::from(self.warp.amplitude);
        let qx = x + amplitude
            * f64::from(gradient_noise_3d_wide(
                derive_seed(seed, WARP_3D_X_KEY).0,
                wx,
                wy,
                wz,
            ));
        let qy = y + amplitude
            * f64::from(gradient_noise_3d_wide(
                derive_seed(seed, WARP_3D_Y_KEY).0,
                wx,
                wy,
                wz,
            ));
        let qz = z + amplitude
            * f64::from(gradient_noise_3d_wide(
                derive_seed(seed, WARP_3D_Z_KEY).0,
                wx,
                wy,
                wz,
            ));
        self.fbm.sample_wide(qx, qy, qz)
    }

    /// Batch sampling; bit-identical to per-coordinate [`Self::sample`].
    /// Panics when the slices differ in length.
    pub fn sample_batch(&self, coords: &[[f32; 3]], out: &mut [f32]) {
        assert_eq!(
            coords.len(),
            out.len(),
            "procgen batch sampling requires coords.len() == out.len()"
        );
        for (coord, value) in coords.iter().zip(out.iter_mut()) {
            *value = self.sample(coord[0], coord[1], coord[2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> FbmParams {
        FbmParams::default()
    }

    #[test]
    fn default_recipe_is_valid() {
        params().validate().unwrap();
        WarpParams::default().validate().unwrap();
    }

    #[test]
    fn invalid_recipes_are_rejected() {
        let cases = [
            (
                FbmParams {
                    octaves: 0,
                    ..params()
                },
                "octaves",
            ),
            (
                FbmParams {
                    octaves: 33,
                    ..params()
                },
                "octaves",
            ),
            (
                FbmParams {
                    frequency: 0.0,
                    ..params()
                },
                "frequency",
            ),
            (
                FbmParams {
                    frequency: f32::NAN,
                    ..params()
                },
                "frequency",
            ),
            (
                FbmParams {
                    amplitude: -1.0,
                    ..params()
                },
                "amplitude",
            ),
            (
                FbmParams {
                    lacunarity: f32::INFINITY,
                    ..params()
                },
                "lacunarity",
            ),
            (
                FbmParams {
                    gain: 1.5,
                    ..params()
                },
                "gain",
            ),
            (
                FbmParams {
                    gain: f32::NAN,
                    ..params()
                },
                "gain",
            ),
            (
                FbmParams {
                    offset: [0.0, f32::NAN, 0.0],
                    ..params()
                },
                "offset",
            ),
        ];
        for (recipe, field) in cases {
            let error = recipe.validate().unwrap_err();
            assert!(
                error.to_string().contains(field),
                "expected a '{field}' error, got: {error}"
            );
            assert!(Fbm2D::new(Seed(1), recipe).is_err());
        }
        assert!(WarpParams {
            amplitude: -0.5,
            frequency: 1.0
        }
        .validate()
        .is_err());
        assert!(WarpParams {
            amplitude: 1.0,
            frequency: 0.0
        }
        .validate()
        .is_err());
    }

    #[test]
    fn zero_amplitude_and_zero_gain_do_not_nan() {
        let fbm = Fbm2D::new(
            Seed(3),
            FbmParams {
                amplitude: 0.0,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(fbm.sample(1.25, -4.5), 0.0);

        let fbm = Fbm2D::new(
            Seed(3),
            FbmParams {
                gain: 0.0,
                ..params()
            },
        )
        .unwrap();
        assert!(fbm.sample(1.25, -4.5).is_finite());
    }

    #[test]
    fn fbm_output_range_is_sane() {
        let fbm = Fbm3D::new(Seed(11), params()).unwrap();
        for index in 0..256 {
            let t = index as f32 * 0.37 - 47.0;
            let value = fbm.sample(t, t * 0.5, -t);
            assert!(value.is_finite());
            assert!(
                (-1.0..=1.0).contains(&value),
                "normalized fBm should stay in [-1, 1], got {value}"
            );
        }
    }

    #[test]
    fn warped_sampler_is_finite_for_extreme_inputs() {
        let warped = WarpedFbm2D::new(
            Fbm2D::new(Seed(5), params()).unwrap(),
            WarpParams {
                amplitude: 4.0,
                frequency: 0.5,
            },
        )
        .unwrap();
        for coord in [
            [0.0, 0.0],
            [-1000.0, 1000.0],
            [1e30, -1e30],
            [f32::NAN, 1.0],
            [f32::INFINITY, 1.0],
        ] {
            assert!(warped.sample(coord[0], coord[1]).is_finite());
        }
    }

    #[test]
    fn recipes_roundtrip_through_serde() {
        let warped = WarpedFbm3D::new(
            Fbm3D::new(Seed(42), params()).unwrap(),
            WarpParams::default(),
        )
        .unwrap();
        let json = serde_json::to_string(&warped).unwrap();
        let parsed: WarpedFbm3D = serde_json::from_str(&json).unwrap();
        assert_eq!(warped, parsed);
    }
}
