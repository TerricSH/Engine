use thiserror::Error;

use crate::{PostProcessSettings, ToneMapping};

pub const TONE_MAP_MODE_ACES: u32 = 0;
pub const TONE_MAP_MODE_REINHARD: u32 = 1;
pub const TONE_MAP_MODE_NONE: u32 = 2;

/// Artist-scale lighting is neutral at the engine's default physical camera
/// (f/16, 1/60 s, ISO 100) while still responding in photographic stops.
pub const ARTISTIC_LIGHTING_REFERENCE_EV100: f32 = 13.906_891;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToneMapPlanOptions {
    pub output_is_srgb: bool,
    pub weighted_oit_resolve: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneMapPlan {
    pub mode: u32,
    pub exposure: f32,
    pub output_is_srgb: u32,
    pub effect_flags: u32,
    pub bloom: [f32; 4],
    pub color_filter_saturation: [f32; 4],
    pub contrast: [f32; 4],
    pub lift: [f32; 4],
    pub gamma: [f32; 4],
    pub gain: [f32; 4],
    pub vignette: [f32; 4],
}

impl ToneMapPlan {
    pub const SIZE: usize = 128;

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut bytes = [0; Self::SIZE];
        bytes[0..4].copy_from_slice(&self.mode.to_ne_bytes());
        bytes[4..8].copy_from_slice(&self.exposure.to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.output_is_srgb.to_ne_bytes());
        bytes[12..16].copy_from_slice(&self.effect_flags.to_ne_bytes());
        for (vector_index, vector) in [
            self.bloom,
            self.color_filter_saturation,
            self.contrast,
            self.lift,
            self.gamma,
            self.gain,
            self.vignette,
        ]
        .into_iter()
        .enumerate()
        {
            for (component_index, component) in vector.into_iter().enumerate() {
                let start = 16 + vector_index * 16 + component_index * 4;
                bytes[start..start + 4].copy_from_slice(&component.to_ne_bytes());
            }
        }
        bytes
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq)]
pub enum ToneMapPlanError {
    #[error("exposure_ev100 must be finite, received {0}")]
    NonFiniteEv100(f32),
    #[error("exposure_ev100 {0:?} produces a non-finite exposure multiplier")]
    NonFiniteExposureMultiplier(Option<f32>),
}

pub fn prepare_tone_map_plan(
    tone_mapping: ToneMapping,
    exposure_ev100: Option<f32>,
    post_process: PostProcessSettings,
    options: ToneMapPlanOptions,
) -> Result<ToneMapPlan, ToneMapPlanError> {
    let mode = match tone_mapping {
        ToneMapping::Aces => TONE_MAP_MODE_ACES,
        ToneMapping::Reinhard => TONE_MAP_MODE_REINHARD,
        ToneMapping::None => TONE_MAP_MODE_NONE,
    };
    let exposure = match exposure_ev100 {
        None => 1.0,
        Some(ev100) if ev100.is_finite() => (ARTISTIC_LIGHTING_REFERENCE_EV100 - ev100).exp2(),
        Some(ev100) => return Err(ToneMapPlanError::NonFiniteEv100(ev100)),
    };
    if !exposure.is_finite() {
        return Err(ToneMapPlanError::NonFiniteExposureMultiplier(
            exposure_ev100,
        ));
    }

    let master_enabled = post_process.enabled;
    let effect_flags = u32::from(master_enabled && post_process.bloom.enabled)
        | (u32::from(master_enabled && post_process.color_grading.enabled) << 1)
        | (u32::from(master_enabled && post_process.vignette.enabled) << 2)
        | (u32::from(options.weighted_oit_resolve) << 3)
        | (u32::from(master_enabled && post_process.planetary_lens.enabled) << 4);
    let grading = post_process.color_grading;
    let vignette = post_process.vignette;
    let lens = post_process.planetary_lens;
    Ok(ToneMapPlan {
        mode,
        exposure,
        output_is_srgb: u32::from(options.output_is_srgb),
        effect_flags,
        bloom: [
            post_process.bloom.threshold,
            post_process.bloom.intensity,
            post_process.bloom.radius,
            0.0,
        ],
        color_filter_saturation: [
            grading.color_filter[0],
            grading.color_filter[1],
            grading.color_filter[2],
            grading.saturation,
        ],
        contrast: [
            grading.contrast,
            lens.barrel_distortion,
            lens.horizon_curvature,
            lens.atmosphere_intensity,
        ],
        lift: [grading.lift[0], grading.lift[1], grading.lift[2], 0.0],
        gamma: [grading.gamma[0], grading.gamma[1], grading.gamma[2], 0.0],
        gain: [grading.gain[0], grading.gain[1], grading.gain[2], 0.0],
        vignette: [
            vignette.intensity,
            vignette.smoothness,
            vignette.roundness,
            lens.chromatic_aberration,
        ],
    })
}
