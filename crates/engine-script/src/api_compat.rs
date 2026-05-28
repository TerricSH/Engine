//! Script API compatibility versioning and mobile-safe feature subsets.
//!
//! Provides [`ApiCompatRange`] for expressing supported API version ranges,
//! and the [`MOBILE_SAFE_FEATURES`] / [`DESKTOP_ONLY_FEATURES`] constants
//! that define which ScriptAPI features are available on mobile (AOT) vs.
//! desktop platforms.
//!
//! The mobile-safe subset corresponds to the NativeAOT-compilable portion of
//! **ScriptAPI-v0** as described by the Gate 7 design.  It is expressed as a
//! `script_api_version_range` constraint on the `MobileHotUpdate-v0` manifest.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ApiCompatRange
// ---------------------------------------------------------------------------

/// A range of supported script API versions `[min_version, max_version]`.
///
/// Versions are expressed as `(major, minor)` tuples.  The range is
/// **inclusive** on both ends.
///
/// # Examples
///
/// ```
/// use engine_script::ApiCompatRange;
///
/// let range = ApiCompatRange::new((0, 1), (0, 5));
/// assert!(range.contains((0, 3)));
/// assert!(!range.contains((1, 0)));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiCompatRange {
    /// Minimum inclusive version `(major, minor)`.
    pub min_version: (u16, u16),
    /// Maximum inclusive version `(major, minor)`.
    pub max_version: (u16, u16),
}

impl ApiCompatRange {
    /// Create a new compatibility range.
    ///
    /// # Panics
    ///
    /// Panics if `min > max` (lexicographically).
    pub fn new(min: (u16, u16), max: (u16, u16)) -> Self {
        assert!(
            min <= max,
            "ApiCompatRange: min_version ({min:?}) > max_version ({max:?})",
        );
        Self {
            min_version: min,
            max_version: max,
        }
    }

    /// Check whether a given version falls within this range (inclusive).
    pub fn contains(&self, version: (u16, u16)) -> bool {
        version >= self.min_version && version <= self.max_version
    }
}

// ---------------------------------------------------------------------------
// Feature constants
// ---------------------------------------------------------------------------

/// Mobile-safe subset of ScriptAPI features.
///
/// These features are available on **all** profiles, including AOT-only
/// platforms (iOS).  Everything not in this list requires desktop / JIT
/// capabilities.
///
/// Based on the NativeAOT-compilable portion of ScriptAPI-v0.
pub const MOBILE_SAFE_FEATURES: &[&str] = &[
    "OnCreate",
    "OnStart",
    "OnUpdate",
    "OnDestroy",
    "GetField",
    "SetField",
    "EntityRef",
    "AssetRef",
    "Vec3",
    "Quat",
    "Transform",
    "Time_deltaTime",
];

/// Features that are **desktop-only** (blocked on mobile platforms).
///
/// These features rely on JIT compilation, `System.Reflection.Emit`, or
/// dynamic assembly loading, none of which are available under NativeAOT
/// on iOS or under restricted runtimes on Android.
pub const DESKTOP_ONLY_FEATURES: &[&str] = &[
    "Reflection_Emit",
    "Assembly_LoadFrom",
    "Type_MakeGenericType",
    "DynamicCode",
    "Unsafe_CodePtr",
];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── ApiCompatRange ────────────────────────────────────────────────────

    #[test]
    fn range_new_valid() {
        let r = ApiCompatRange::new((0, 1), (0, 5));
        assert_eq!(r.min_version, (0, 1));
        assert_eq!(r.max_version, (0, 5));
    }

    #[test]
    fn range_contains_within() {
        let r = ApiCompatRange::new((0, 1), (0, 5));
        assert!(r.contains((0, 1)));
        assert!(r.contains((0, 3)));
        assert!(r.contains((0, 5)));
    }

    #[test]
    fn range_contains_below_min() {
        let r = ApiCompatRange::new((0, 2), (0, 5));
        assert!(!r.contains((0, 1)));
    }

    #[test]
    fn range_contains_above_max() {
        let r = ApiCompatRange::new((0, 1), (0, 5));
        assert!(!r.contains((0, 6)));
    }

    #[test]
    fn range_contains_major_version_gap() {
        let r = ApiCompatRange::new((0, 1), (1, 0));
        assert!(r.contains((0, 9)));
        assert!(r.contains((1, 0)));
        assert!(!r.contains((1, 1)));
    }

    #[test]
    fn range_single_version() {
        let r = ApiCompatRange::new((2, 0), (2, 0));
        assert!(r.contains((2, 0)));
        assert!(!r.contains((2, 1)));
        assert!(!r.contains((1, 9)));
    }

    #[test]
    #[should_panic(expected = "min_version")]
    fn range_new_panics_on_invalid_order() {
        let _ = ApiCompatRange::new((1, 0), (0, 5));
    }

    #[test]
    fn range_serde_roundtrip() {
        let r = ApiCompatRange::new((0, 1), (0, 10));
        let json = serde_json::to_string(&r).unwrap();
        let back: ApiCompatRange = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn range_debug_output() {
        let r = ApiCompatRange::new((0, 1), (0, 5));
        let debug = format!("{:?}", r);
        assert!(debug.contains("ApiCompatRange"));
    }

    // ── Feature constants ─────────────────────────────────────────────────

    #[test]
    fn mobile_safe_features_contains_lifecycle() {
        assert!(MOBILE_SAFE_FEATURES.contains(&"OnCreate"));
        assert!(MOBILE_SAFE_FEATURES.contains(&"OnStart"));
        assert!(MOBILE_SAFE_FEATURES.contains(&"OnUpdate"));
        assert!(MOBILE_SAFE_FEATURES.contains(&"OnDestroy"));
    }

    #[test]
    fn mobile_safe_features_contains_data_types() {
        assert!(MOBILE_SAFE_FEATURES.contains(&"Vec3"));
        assert!(MOBILE_SAFE_FEATURES.contains(&"Quat"));
        assert!(MOBILE_SAFE_FEATURES.contains(&"Transform"));
    }

    #[test]
    fn desktop_only_features_contains_reflection_emit() {
        assert!(DESKTOP_ONLY_FEATURES.contains(&"Reflection_Emit"));
        assert!(DESKTOP_ONLY_FEATURES.contains(&"Assembly_LoadFrom"));
        assert!(DESKTOP_ONLY_FEATURES.contains(&"Type_MakeGenericType"));
    }

    #[test]
    fn no_overlap_between_mobile_safe_and_desktop_only() {
        for feature in MOBILE_SAFE_FEATURES {
            assert!(
                !DESKTOP_ONLY_FEATURES.contains(feature),
                "Feature '{feature}' appears in both MOBILE_SAFE_FEATURES and DESKTOP_ONLY_FEATURES",
            );
        }
    }

    #[test]
    fn mobile_safe_count() {
        // Sanity check: the list should have a reasonable number of entries
        // for the initial ScriptAPI-v0 subset.
        assert!(MOBILE_SAFE_FEATURES.len() >= 10);
        assert!(DESKTOP_ONLY_FEATURES.len() >= 4);
    }
}
