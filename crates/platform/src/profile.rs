//! Runtime platform profiles describing the capabilities of each target.
//!
//! Each [`PlatformProfile`] is a static singleton that encodes what the
//! runtime can do on that platform — JIT vs AOT, dynamic code generation,
//! texture limits, etc.  Consumers (script host, renderer, asset pipeline)
//! check the profile at startup to select the correct code paths.
//!
//! # Mobile vs Desktop
//!
//! | Capability              | Desktop | Android | iOS     |
//! |-------------------------|---------|---------|---------|
//! | C# assembly load        | ✓       | ✓       | ✓       |
//! | JIT                     | ✓       | ✓*      | ✗       |
//! | Dynamic code (emit)     | ✓       | ✗       | ✗       |
//! | Interpreted logic       | ✓       | ✓       | ✓       |
//! | Max texture size        | 16384   | 4096    | 4096    |
//! | Debug markers           | ✓       | ✓       | ✗       |
//!
//! *Android supports JIT via ART but not for all scenarios; we treat it as
//! available for scripting purposes.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Family of platforms sharing the same underlying OS / kernel personality.
///
/// This is used for conditional logic that goes beyond simple capability
/// flags — e.g. dispatch to platform-specific input mappings or file paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformFamily {
    DesktopWindows,
    DesktopMac,
    DesktopLinux,
    Android,
    Ios,
}

/// Runtime profile describing a platform's capabilities.
///
/// Every running instance of the engine has exactly one active profile,
/// provided by the [`PlatformAdapter::profile`][crate::PlatformAdapter::profile]
/// method.  The static constants [`DESKTOP_PROFILE`], [`ANDROID_PROFILE`], and
/// [`IOS_PROFILE`] serve as canonical reference values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformProfile {
    /// Human-readable name (e.g. `"desktop"`, `"android"`, `"ios"`).
    pub name: String,
    /// The OS family this platform belongs to.
    pub family: PlatformFamily,
    /// Whether the runtime can load managed assemblies dynamically.
    ///
    /// Always `true` for CoreCLR and NativeAOT-compiled hosts (the assembly
    /// is baked in or loadable from disk).  Set to `false` only when the
    /// runtime has no managed code support at all.
    pub supports_csharp_assembly_load: bool,
    /// Whether the runtime has a just-in-time compiler available.
    ///
    /// Desktop CoreCLR ✅  —  Android ART ✅  —  iOS NativeAOT ❌
    pub supports_jit: bool,
    /// Whether `Reflection.Emit`, `ILEmit`, or equivalent dynamic code
    /// generation is supported.
    pub supports_dynamic_code: bool,
    /// Whether an interpreter or partial evaluator is available for code
    /// paths that cannot be AOT-compiled ahead of time.
    pub supports_interpreted_logic: bool,
    /// Maximum supported texture dimension in pixels (width / height).
    ///
    /// Desktop GPUs typically allow 16384; mobile GPUs cap at 4096.
    pub max_texture_size: u32,
    /// Whether GPU debug markers (e.g. `VK_EXT_debug_utils`, `GL_KHR_debug`)
    /// are available.  Disabled on iOS where Metal capture is preferred.
    pub supports_debug_markers: bool,
}

// ---------------------------------------------------------------------------
// Static profile singletons
// ---------------------------------------------------------------------------

/// Canonical profile for desktop targets (Windows, Mac, Linux).
///
/// All desktop hosts run under CoreCLR with full JIT and dynamic code
/// generation.
pub static DESKTOP_PROFILE: LazyLock<PlatformProfile> = LazyLock::new(|| PlatformProfile {
    name: "desktop".to_string(),
    family: PlatformFamily::DesktopWindows, // overridden per-platform at init
    supports_csharp_assembly_load: true,
    supports_jit: true,
    supports_dynamic_code: true,
    supports_interpreted_logic: true,
    max_texture_size: 16384,
    supports_debug_markers: true,
});

/// Canonical profile for Android targets (ART / NativeAOT hybrid).
///
/// Android has JIT via ART but lacks `Reflection.Emit` in the AOT path.
/// Texture sizes are limited compared to desktop.
pub static ANDROID_PROFILE: LazyLock<PlatformProfile> = LazyLock::new(|| PlatformProfile {
    name: "android".to_string(),
    family: PlatformFamily::Android,
    supports_csharp_assembly_load: true,
    supports_jit: true,
    supports_dynamic_code: false,
    supports_interpreted_logic: true,
    max_texture_size: 4096,
    supports_debug_markers: true,
});

/// Canonical profile for iOS targets (NativeAOT only, no JIT).
///
/// Apple's policy prohibits JIT compilation on iOS, so all managed code must
/// be pre-compiled via NativeAOT.  Debug markers are unavailable because
/// Metal is the only graphics API and Xcode's GPU debugger uses a different
/// mechanism.
pub static IOS_PROFILE: LazyLock<PlatformProfile> = LazyLock::new(|| PlatformProfile {
    name: "ios".to_string(),
    family: PlatformFamily::Ios,
    supports_csharp_assembly_load: true,
    supports_jit: false,
    supports_dynamic_code: false,
    supports_interpreted_logic: true,
    max_texture_size: 4096,
    supports_debug_markers: false,
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Desktop ───────────────────────────────────────────────────────────

    #[test]
    fn desktop_profile_name() {
        assert_eq!(DESKTOP_PROFILE.name, "desktop");
    }

    #[test]
    fn desktop_profile_supports_jit() {
        assert!(DESKTOP_PROFILE.supports_jit);
    }

    #[test]
    fn desktop_profile_supports_dynamic_code() {
        assert!(DESKTOP_PROFILE.supports_dynamic_code);
    }

    #[test]
    fn desktop_profile_max_texture_size() {
        assert_eq!(DESKTOP_PROFILE.max_texture_size, 16384);
    }

    #[test]
    fn desktop_profile_supports_debug_markers() {
        assert!(DESKTOP_PROFILE.supports_debug_markers);
    }

    // ── Android ───────────────────────────────────────────────────────────

    #[test]
    fn android_profile_name() {
        assert_eq!(ANDROID_PROFILE.name, "android");
    }

    #[test]
    fn android_profile_family() {
        assert_eq!(ANDROID_PROFILE.family, PlatformFamily::Android);
    }

    #[test]
    fn android_profile_supports_jit() {
        assert!(ANDROID_PROFILE.supports_jit);
    }

    #[test]
    fn android_profile_no_dynamic_code() {
        assert!(!ANDROID_PROFILE.supports_dynamic_code);
    }

    #[test]
    fn android_profile_max_texture_size() {
        assert_eq!(ANDROID_PROFILE.max_texture_size, 4096);
    }

    #[test]
    fn android_profile_supports_debug_markers() {
        assert!(ANDROID_PROFILE.supports_debug_markers);
    }

    // ── iOS ───────────────────────────────────────────────────────────────

    #[test]
    fn ios_profile_name() {
        assert_eq!(IOS_PROFILE.name, "ios");
    }

    #[test]
    fn ios_profile_family() {
        assert_eq!(IOS_PROFILE.family, PlatformFamily::Ios);
    }

    #[test]
    fn ios_profile_no_jit() {
        assert!(!IOS_PROFILE.supports_jit);
    }

    #[test]
    fn ios_profile_no_dynamic_code() {
        assert!(!IOS_PROFILE.supports_dynamic_code);
    }

    #[test]
    fn ios_profile_max_texture_size() {
        assert_eq!(IOS_PROFILE.max_texture_size, 4096);
    }

    #[test]
    fn ios_profile_no_debug_markers() {
        assert!(!IOS_PROFILE.supports_debug_markers);
    }

    #[test]
    fn ios_profile_assembly_load() {
        assert!(IOS_PROFILE.supports_csharp_assembly_load);
    }

    // ── Cross-profile ─────────────────────────────────────────────────────

    #[test]
    fn profiles_are_distinct() {
        assert_ne!(*DESKTOP_PROFILE, *ANDROID_PROFILE);
        assert_ne!(*DESKTOP_PROFILE, *IOS_PROFILE);
        assert_ne!(*ANDROID_PROFILE, *IOS_PROFILE);
    }

    #[test]
    fn platform_family_serde_roundtrip() {
        for family in &[
            PlatformFamily::DesktopWindows,
            PlatformFamily::DesktopMac,
            PlatformFamily::DesktopLinux,
            PlatformFamily::Android,
            PlatformFamily::Ios,
        ] {
            let json = serde_json::to_string(family).unwrap();
            let back: PlatformFamily = serde_json::from_str(&json).unwrap();
            assert_eq!(*family, back);
        }
    }

    #[test]
    fn platform_profile_serde_roundtrip() {
        let json = serde_json::to_string(&*DESKTOP_PROFILE).unwrap();
        let back: PlatformProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(*DESKTOP_PROFILE, back);
    }
}
