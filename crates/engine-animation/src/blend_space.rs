use serde::{Deserialize, Serialize};

/// A single sample point in a 1D blend space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendSpaceSample {
    /// Parameter threshold (e.g. speed value) at this sample point.
    pub threshold: f32,
    /// Clip to blend at this point.
    pub clip_asset: String,
}

/// A 1D blend space that interpolates between multiple animation clips based on a single parameter.
///
/// Each clip has a threshold value. The blend space finds the two clips whose thresholds bracket
/// the current parameter value and blends between them.
///
/// Example: walk clip at threshold 1.5, run clip at threshold 5.0.
/// At speed=3.0, the result is ~57% walk + ~43% run (linear blend).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendSpace1D {
    /// Name of the parameter driving this blend (e.g. "speed").
    pub parameter_name: String,
    /// Sorted list of sample points. Blend is performed between the two
    /// surrounding samples.
    pub clips: Vec<BlendSpaceSample>,
}

impl BlendSpace1D {
    /// Create a new 1D blend space.
    /// `clips` will be sorted by threshold value.
    pub fn new(parameter_name: impl Into<String>, mut clips: Vec<BlendSpaceSample>) -> Self {
        clips.sort_by(|a, b| {
            a.threshold
                .partial_cmp(&b.threshold)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Self {
            parameter_name: parameter_name.into(),
            clips,
        }
    }

    /// Get the blend weight between the two nearest clips for a given parameter value.
    /// Returns (index_of_lower_clip, blend_weight_to_upper) where blend_weight is 0..1.
    /// If param <= first threshold: returns (0, 0.0) — first clip only.
    /// If param >= last threshold: returns (last-1, 1.0) — last clip only.
    pub fn sample_weight(&self, param: f32) -> (usize, f32) {
        let count = self.clips.len();
        if count < 2 || param <= self.clips[0].threshold {
            return (0, 0.0);
        }
        if param >= self.clips[count - 1].threshold {
            return (count - 2, 1.0);
        }
        // Binary search for the bracketing interval
        let mut lo = 0usize;
        let mut hi = count - 1;
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if param < self.clips[mid].threshold {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let range = self.clips[hi].threshold - self.clips[lo].threshold;
        let t = if range > 0.0 {
            ((param - self.clips[lo].threshold) / range).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (lo, t)
    }
}
