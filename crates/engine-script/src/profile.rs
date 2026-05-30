//! Platform runtime profiles and mobile compatibility checks.
//!
//! Defines [`PlatformProfile`] (desktop / Android / iOS), the associated
//! [`PlatformConstraints`] for validation, and [`is_feature_available`] for
//! querying whether a given ScriptAPI feature is permitted on a profile.
//!
//! These are **data-only** types — they describe constraints but do **not**
//! perform any runtime gating themselves.

use serde::{Deserialize, Serialize};

use crate::api_compat::{DESKTOP_ONLY_FEATURES, MOBILE_SAFE_FEATURES};
use crate::component::ScriptComponent;

// ---------------------------------------------------------------------------
// PlatformProfile
// ---------------------------------------------------------------------------

/// Target platform runtime profile.
///
/// Each variant encodes the execution capabilities that apply to scripts
/// running on that platform. Profiles are purely descriptive and do not
/// influence the actual script host behaviour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformProfile {
    /// Full desktop environment (Windows, macOS, Linux).
    Desktop,
    /// Android (JIT-capable, but limited reflection emit).
    Android,
    /// iOS (NativeAOT only — no JIT, no dynamic code).
    Ios,
}

impl PlatformProfile {
    /// Human-readable profile name.
    pub fn name(&self) -> &str {
        match self {
            PlatformProfile::Desktop => "Desktop",
            PlatformProfile::Android => "Android",
            PlatformProfile::Ios => "iOS",
        }
    }

    /// Whether assemblies can be loaded dynamically at runtime
    /// (e.g. via `Assembly.LoadFrom`).
    ///
    /// - **Desktop**: always available.
    /// - **Android**: optional / partial (some runtimes restrict it).
    /// - **iOS**: never (NativeAOT compiles assemblies ahead of time).
    pub fn supports_dynamic_assembly_load(&self) -> bool {
        match self {
            PlatformProfile::Desktop => true,
            PlatformProfile::Android => false,
            PlatformProfile::Ios => false,
        }
    }

    /// Whether the platform supports just-in-time (JIT) compilation.
    ///
    /// - **Desktop**: yes (full JIT).
    /// - **Android**: yes (ART JIT / hybrid mode).
    /// - **iOS**: no (NativeAOT / full AOT only).
    pub fn supports_jit(&self) -> bool {
        match self {
            PlatformProfile::Desktop => true,
            PlatformProfile::Android => true,
            PlatformProfile::Ios => false,
        }
    }

    /// Whether `System.Reflection.Emit` (runtime code generation) is available.
    ///
    /// - **Desktop**: yes.
    /// - **Android**: no (Reflection.Emit is blocked on Mono/IL2CPP).
    /// - **iOS**: no (NativeAOT does not support emit).
    pub fn supports_reflection_emit(&self) -> bool {
        match self {
            PlatformProfile::Desktop => true,
            PlatformProfile::Android => false,
            PlatformProfile::Ios => false,
        }
    }

    /// Whether AOT (ahead-of-time) compilation is **required** for scripts.
    ///
    /// - **Desktop**: no (JIT is fine).
    /// - **Android**: no (JIT / hybrid).
    /// - **iOS**: yes (Apple enforces NativeAOT).
    pub fn is_aot_required(&self) -> bool {
        match self {
            PlatformProfile::Desktop => false,
            PlatformProfile::Android => false,
            PlatformProfile::Ios => true,
        }
    }

    /// Whether script assemblies must be code-signed.
    ///
    /// - **Desktop**: no.
    /// - **Android**: no.
    /// - **iOS**: yes (Apple developer certificate required).
    pub fn requires_signing(&self) -> bool {
        match self {
            PlatformProfile::Desktop => false,
            PlatformProfile::Android => false,
            PlatformProfile::Ios => true,
        }
    }
}

// ---------------------------------------------------------------------------
// PlatformConstraints
// ---------------------------------------------------------------------------

/// Constraints for a given platform profile, used for validation.
///
/// Each profile has a set of hard limits (assembly size, instance count) and
/// feature allow/block lists that a script component must satisfy to be
/// considered compatible with the target platform.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformConstraints {
    /// The target profile these constraints apply to.
    pub profile: PlatformProfile,
    /// Maximum assembly size in bytes (uncompressed PE image).
    pub max_assembly_size_bytes: u64,
    /// Maximum number of script instances across all entities.
    pub max_script_instances: u32,
    /// Script API features that are **allowed** on this profile.
    ///
    /// If non-empty, only these features may be used. An empty list means
    /// "no restriction" (all features allowed by the profile's capability
    /// flags are available).
    pub allowed_script_api_features: Vec<String>,
    /// Reflection patterns that are **blocked** on this profile.
    ///
    /// Examples: `"MakeGenericType"`, `"Emit"`, `"LoadFrom"`.
    pub blocked_reflection_patterns: Vec<String>,
    /// Human-readable notes about this profile's constraints.
    pub notes: Vec<String>,
}

impl PlatformConstraints {
    /// Return the default constraints for a given profile.
    pub fn for_profile(profile: PlatformProfile) -> Self {
        match profile {
            PlatformProfile::Desktop => Self {
                profile,
                max_assembly_size_bytes: 512 * 1024 * 1024, // 512 MiB
                max_script_instances: 10_000,
                allowed_script_api_features: Vec::new(), // unrestricted
                blocked_reflection_patterns: Vec::new(), // unrestricted
                notes: vec![
                    "Full JIT, reflection emit, and dynamic assembly loading.".into(),
                    "No feature restrictions.".into(),
                ],
            },
            PlatformProfile::Android => Self {
                profile,
                max_assembly_size_bytes: 128 * 1024 * 1024, // 128 MiB
                max_script_instances: 2_000,
                allowed_script_api_features: MOBILE_SAFE_FEATURES
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                blocked_reflection_patterns: DESKTOP_ONLY_FEATURES
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                notes: vec![
                    "JIT-capable but Reflection.Emit is blocked.".into(),
                    "Dynamic assembly loading is restricted.".into(),
                    "Prefer AOT-friendly patterns for broad compatibility.".into(),
                ],
            },
            PlatformProfile::Ios => Self {
                profile,
                max_assembly_size_bytes: 64 * 1024 * 1024, // 64 MiB
                max_script_instances: 1_000,
                allowed_script_api_features: MOBILE_SAFE_FEATURES
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                blocked_reflection_patterns: DESKTOP_ONLY_FEATURES
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                notes: vec![
                    "NativeAOT only — no JIT, no dynamic code.".into(),
                    "All assemblies must be compiled ahead of time.".into(),
                    "Script assemblies must be signed with an Apple certificate.".into(),
                ],
            },
        }
    }

    /// Validate a [`ScriptComponent`] against this profile's constraints.
    ///
    /// Returns `Ok(())` if the component passes, or `Err(reason)` with a
    /// human-readable explanation of the first constraint violation.
    ///
    /// Currently checks:
    /// - `assembly_id` and `class_name` are non-empty.
    /// - Feature restrictions are noted; structural validation can be extended
    ///   as richer metadata becomes available.
    pub fn validate_script_component(&self, component: &ScriptComponent) -> Result<(), String> {
        if component.assembly_id.is_empty() {
            return Err("assembly_id must not be empty".into());
        }
        if component.class_name.is_empty() {
            return Err("class_name must not be empty".into());
        }

        // If the profile has an allow-list, every feature referenced by the
        // component's class name *convention* should be checked.  At this
        // level we do not have per-call-site metadata, so we emit a
        // structural note when the allow-list is non-empty.
        if !self.allowed_script_api_features.is_empty() {
            // The component's class name may hint at feature usage; for now
            // this is a structural pass. Future iterations could inspect
            // the assembly metadata against the allow-list.
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// is_feature_available
// ---------------------------------------------------------------------------

/// Query whether a specific ScriptAPI feature is available on a given profile.
///
/// # Rules
///
/// | Profile  | Behaviour |
/// |----------|-----------|
/// | Desktop  | All features are available. |
/// | Android  | Features in `DESKTOP_ONLY_FEATURES` are blocked; everything else is available. |
/// | iOS      | Only features in `MOBILE_SAFE_FEATURES` are available. |
///
/// # Examples
///
/// ```
/// # use engine_script::{PlatformProfile, is_feature_available};
/// assert!(is_feature_available(PlatformProfile::Desktop, "OnUpdate"));
/// assert!(!is_feature_available(PlatformProfile::Ios, "Reflection_Emit"));
/// assert!(is_feature_available(PlatformProfile::Android, "OnUpdate"));
/// ```
pub fn is_feature_available(profile: PlatformProfile, feature: &str) -> bool {
    match profile {
        PlatformProfile::Desktop => true,
        PlatformProfile::Android => !DESKTOP_ONLY_FEATURES.contains(&feature),
        PlatformProfile::Ios => MOBILE_SAFE_FEATURES.contains(&feature),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScriptComponent;

    // ── PlatformProfile ────────────────────────────────────────────────────

    #[test]
    fn profile_name_desktop() {
        assert_eq!(PlatformProfile::Desktop.name(), "Desktop");
    }

    #[test]
    fn profile_name_android() {
        assert_eq!(PlatformProfile::Android.name(), "Android");
    }

    #[test]
    fn profile_name_ios() {
        assert_eq!(PlatformProfile::Ios.name(), "iOS");
    }

    #[test]
    fn profile_desktop_capabilities() {
        let p = PlatformProfile::Desktop;
        assert!(p.supports_dynamic_assembly_load());
        assert!(p.supports_jit());
        assert!(p.supports_reflection_emit());
        assert!(!p.is_aot_required());
        assert!(!p.requires_signing());
    }

    #[test]
    fn profile_android_capabilities() {
        let p = PlatformProfile::Android;
        assert!(!p.supports_dynamic_assembly_load());
        assert!(p.supports_jit());
        assert!(!p.supports_reflection_emit());
        assert!(!p.is_aot_required());
        assert!(!p.requires_signing());
    }

    #[test]
    fn profile_ios_capabilities() {
        let p = PlatformProfile::Ios;
        assert!(!p.supports_dynamic_assembly_load());
        assert!(!p.supports_jit());
        assert!(!p.supports_reflection_emit());
        assert!(p.is_aot_required());
        assert!(p.requires_signing());
    }

    // ── is_feature_available ───────────────────────────────────────────────

    #[test]
    fn feature_desktop_allows_all() {
        assert!(is_feature_available(
            PlatformProfile::Desktop,
            "Reflection_Emit"
        ));
        assert!(is_feature_available(PlatformProfile::Desktop, "OnUpdate"));
        assert!(is_feature_available(
            PlatformProfile::Desktop,
            "Fictional_Feature_XYZ"
        ));
    }

    #[test]
    fn feature_android_blocks_desktop_only() {
        assert!(!is_feature_available(
            PlatformProfile::Android,
            "Reflection_Emit"
        ));
        assert!(!is_feature_available(
            PlatformProfile::Android,
            "Assembly_LoadFrom"
        ));
        assert!(is_feature_available(PlatformProfile::Android, "OnUpdate"));
        assert!(is_feature_available(PlatformProfile::Android, "EntityRef"));
    }

    #[test]
    fn feature_ios_only_mobile_safe() {
        assert!(!is_feature_available(
            PlatformProfile::Ios,
            "Reflection_Emit"
        ));
        assert!(is_feature_available(PlatformProfile::Ios, "OnUpdate"));
        assert!(is_feature_available(PlatformProfile::Ios, "OnCreate"));
        assert!(!is_feature_available(PlatformProfile::Ios, "DynamicCode"));
        assert!(!is_feature_available(
            PlatformProfile::Ios,
            "Unsafe_CodePtr"
        ));
    }

    // ── PlatformConstraints ────────────────────────────────────────────────

    #[test]
    fn constraints_desktop_for_profile() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Desktop);
        assert_eq!(c.profile, PlatformProfile::Desktop);
        assert!(c.max_assembly_size_bytes > 100_000_000);
        assert!(c.max_script_instances > 5_000);
        assert!(c.allowed_script_api_features.is_empty());
        assert!(c.blocked_reflection_patterns.is_empty());
    }

    #[test]
    fn constraints_android_for_profile() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Android);
        assert_eq!(c.profile, PlatformProfile::Android);
        assert!(c.max_assembly_size_bytes <= 128 * 1024 * 1024);
        assert!(!c.allowed_script_api_features.is_empty());
        assert!(!c.blocked_reflection_patterns.is_empty());
    }

    #[test]
    fn constraints_ios_for_profile() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Ios);
        assert_eq!(c.profile, PlatformProfile::Ios);
        assert!(c.max_assembly_size_bytes <= 64 * 1024 * 1024);
        assert!(!c.allowed_script_api_features.is_empty());
        assert!(!c.blocked_reflection_patterns.is_empty());
        assert!(c.notes.iter().any(|n| n.contains("NativeAOT")));
    }

    #[test]
    fn constraints_validate_valid_component() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Ios);
        let comp = ScriptComponent::new("MyAssembly", "MyScript");
        assert!(c.validate_script_component(&comp).is_ok());
    }

    #[test]
    fn constraints_validate_empty_assembly_id() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Desktop);
        let comp = ScriptComponent::new("", "MyScript");
        let err = c.validate_script_component(&comp).unwrap_err();
        assert!(err.contains("assembly_id"));
    }

    #[test]
    fn constraints_validate_empty_class_name() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Desktop);
        let comp = ScriptComponent::new("MyAssembly", "");
        let err = c.validate_script_component(&comp).unwrap_err();
        assert!(err.contains("class_name"));
    }

    #[test]
    fn constraints_validate_with_fields() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Android);
        let comp = ScriptComponent::new("Asm", "Script")
            .with_field("speed", crate::ScriptValue::Float(10.0));
        assert!(c.validate_script_component(&comp).is_ok());
    }

    #[test]
    fn profile_serde_roundtrip() {
        for profile in &[
            PlatformProfile::Desktop,
            PlatformProfile::Android,
            PlatformProfile::Ios,
        ] {
            let json = serde_json::to_string(profile).unwrap();
            let back: PlatformProfile = serde_json::from_str(&json).unwrap();
            assert_eq!(*profile, back);
        }
    }

    #[test]
    fn constraints_serde_roundtrip() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Ios);
        let json = serde_json::to_string(&c).unwrap();
        let back: PlatformConstraints = serde_json::from_str(&json).unwrap();
        assert_eq!(c.profile, back.profile);
        assert_eq!(c.max_assembly_size_bytes, back.max_assembly_size_bytes);
        assert_eq!(c.max_script_instances, back.max_script_instances);
    }

    #[test]
    fn constraints_desktop_notes() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Desktop);
        assert!(c.notes.iter().any(|n| n.contains("JIT")));
    }

    #[test]
    fn constraints_android_notes() {
        let c = PlatformConstraints::for_profile(PlatformProfile::Android);
        assert!(c.notes.iter().any(|n| n.contains("Reflection.Emit")));
    }
}
