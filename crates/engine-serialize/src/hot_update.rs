use crate::{AssetId, HashDigest, SchemaVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Version helpers ───────────────────────────────────────────────────────

/// Extract the major version component from a semver-like string.
fn parse_major(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

// ── Core types ────────────────────────────────────────────────────────────

/// MobileHotUpdate-v0 manifest.
///
/// Defines everything needed to validate and apply a hot update package.
/// This is a pure schema/contract type — no download, install, or rollback
/// logic is included.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HotUpdateManifest {
    /// Manifest schema version (this is v0.1.0).
    pub manifest_version: SchemaVersion,
    /// The engine version this package targets (exact or semver-compatible).
    pub engine_version: String,
    /// Script API version used by this manifest (e.g. (1, 2)).
    pub script_api_version: (u16, u16),
    /// Content schema version.
    pub content_schema_version: SchemaVersion,
    /// Logic asset schema version.
    pub logic_asset_schema_version: SchemaVersion,
    /// Per-platform payload entries.
    pub platform_payloads: Vec<PlatformPayload>,
    /// Hashes for each payload file.
    pub payload_hashes: Vec<PayloadHash>,
    /// Optional cryptographic signature (dev mode may omit).
    pub signature: Option<ManifestSignature>,
    /// Rollback metadata.
    pub rollback: RollbackMetadata,
    /// Timestamp of manifest creation (ISO 8601).
    pub created_at: String,
}

/// A platform-specific payload entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformPayload {
    /// Target platform.
    pub platform: PlatformKind,
    /// Asset IDs included in this platform payload.
    pub asset_ids: Vec<AssetId>,
    /// Logic asset IDs (by string identifier) included in this platform payload.
    pub logic_asset_ids: Vec<String>,
    /// Optional assembly payload (C# assemblies). iOS rejects this.
    pub optional_assembly: Option<AssemblyPayload>,
}

/// Target platform discriminator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformKind {
    /// Cross-platform (applies to all targets).
    All,
    /// Desktop (Windows, macOS, Linux).
    Desktop,
    /// Android.
    Android,
    /// iOS.
    Ios,
}

/// A C# assembly payload included in a hot update.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssemblyPayload {
    /// Path to the assembly file within the package.
    pub path: String,
    /// Size of the assembly in bytes.
    pub size_bytes: u64,
    /// Hash digest of the assembly content.
    pub hash: HashDigest,
    /// Minimum engine version required to load this assembly.
    pub min_engine_version: String,
}

/// A payload hash entry for integrity verification.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PayloadHash {
    /// File path relative to the package root.
    pub path: String,
    /// Hash algorithm name (e.g. "sha256").
    pub algorithm: String,
    /// Hash digest value.
    pub hash: HashDigest,
}

/// Cryptographic signature for the manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestSignature {
    /// Signature algorithm ("ed25519" or "rsa-sha256").
    pub algorithm: String,
    /// Raw signature bytes.
    pub value: Vec<u8>,
    /// Key identifier.
    pub key_id: String,
    /// Signature timestamp (ISO 8601).
    pub signed_at: String,
}

/// Metadata for rolling back a hot update.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RollbackMetadata {
    /// Hash of the previous manifest (if any).
    pub previous_manifest_hash: Option<HashDigest>,
    /// Path to a fallback manifest file (if any).
    pub fallback_manifest_path: Option<String>,
    /// Minimum safe engine version for rollback.
    pub min_safe_engine_version: String,
}

/// Compatibility verification result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompatibilityResult {
    /// The manifest is compatible with the current runtime.
    Compatible,
    /// The manifest is incompatible for the given reasons.
    Incompatible { reasons: Vec<String> },
}

// ── Implementation ────────────────────────────────────────────────────────

impl HotUpdateManifest {
    /// Validate structural integrity of the manifest.
    ///
    /// Returns a list of error messages (empty = valid).
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Manifest version must be non-zero.
        if self.manifest_version == SchemaVersion::new(0, 0, 0) {
            errors.push("manifest_version must be non-zero".into());
        }

        // Engine version must not be empty.
        if self.engine_version.is_empty() {
            errors.push("engine_version must not be empty".into());
        }

        // Content schema version must be non-zero.
        if self.content_schema_version == SchemaVersion::new(0, 0, 0) {
            errors.push("content_schema_version must be non-zero".into());
        }

        // Logic asset schema version must be non-zero.
        if self.logic_asset_schema_version == SchemaVersion::new(0, 0, 0) {
            errors.push("logic_asset_schema_version must be non-zero".into());
        }

        // Platform payloads must not be empty.
        if self.platform_payloads.is_empty() {
            errors.push("platform_payloads must not be empty".into());
        }

        // Payload hash paths must be unique.
        let mut seen_paths = std::collections::HashSet::new();
        for hash_entry in &self.payload_hashes {
            if hash_entry.path.is_empty() {
                errors.push("payload_hash path must not be empty".into());
            }
            if hash_entry.algorithm.is_empty() {
                errors.push("payload_hash algorithm must not be empty".into());
            }
            if !seen_paths.insert(&hash_entry.path) {
                errors.push(format!("duplicate payload hash path: {}", hash_entry.path));
            }
        }

        // Created_at must not be empty.
        if self.created_at.is_empty() {
            errors.push("created_at must not be empty".into());
        }

        errors
    }

    /// Check compatibility against current engine/script/api versions.
    ///
    /// Compatibility rules:
    /// - Engine version must match current (same major, semver-compatible)
    /// - Script API version must be within [(0, 1), (current_major+1, 0)]
    /// - iOS platform payload must not contain `optional_assembly`
    /// - Android `optional_assembly` is allowed
    /// - Signature is optional in dev mode (not checked here)
    pub fn check_compatibility(
        &self,
        current_engine: &str,
        current_script_api: (u16, u16),
        platform: PlatformKind,
    ) -> CompatibilityResult {
        let mut reasons = Vec::new();

        // Engine version: must be semver-compatible (same major).
        let manifest_major = parse_major(&self.engine_version);
        let current_major = parse_major(current_engine);
        match (manifest_major, current_major) {
            (Some(m), Some(c)) if m == c => { /* compatible */ }
            (Some(m), Some(c)) => {
                reasons.push(format!(
                    "engine version major mismatch: manifest requires major {}, current engine is major {}",
                    m, c
                ));
            }
            _ => {
                reasons.push(format!(
                    "could not parse engine versions: manifest='{}', current='{}'",
                    self.engine_version, current_engine
                ));
            }
        }

        // Script API version: must be >= (0, 1) and <= (current_script_api.0 + 1, 0).
        let min_api: (u16, u16) = (0, 1);
        let max_api: (u16, u16) = (current_script_api.0 + 1, 0);
        if self.script_api_version < min_api || self.script_api_version > max_api {
            reasons.push(format!(
                "script API version ({:?}) is out of range [{:?}, {:?}]",
                self.script_api_version, min_api, max_api
            ));
        }

        // iOS: must NOT have optional_assembly in any iOS payload.
        if platform == PlatformKind::Ios {
            for payload in &self.platform_payloads {
                if payload.platform == PlatformKind::Ios && payload.optional_assembly.is_some() {
                    reasons.push("iOS platform payload must not contain optional_assembly".into());
                }
            }
        }

        // Android: optional_assembly is allowed (no rejection here).

        if reasons.is_empty() {
            CompatibilityResult::Compatible
        } else {
            CompatibilityResult::Incompatible { reasons }
        }
    }

    /// Get payloads for a specific platform.
    ///
    /// Returns all payloads whose platform matches the given `platform` or
    /// [`PlatformKind::All`].
    pub fn payloads_for_platform(&self, platform: PlatformKind) -> Vec<&PlatformPayload> {
        self.platform_payloads
            .iter()
            .filter(|p| p.platform == platform || p.platform == PlatformKind::All)
            .collect()
    }

    /// Verify a payload hash against data.
    ///
    /// Computes SHA-256 of `data` and compares it to the hash stored for `path`.
    /// Returns `false` if no hash entry exists for the given path.
    pub fn verify_payload_hash(&self, path: &str, data: &[u8]) -> bool {
        self.payload_hashes
            .iter()
            .find(|h| h.path == path)
            .map(|h| {
                let computed = Sha256::digest(data);
                computed[..] == h.hash[..]
            })
            .unwrap_or(false)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetId, SchemaVersion};

    // ── Helpers ────────────────────────────────────────────────────────

    fn sample_manifest() -> HotUpdateManifest {
        HotUpdateManifest {
            manifest_version: SchemaVersion::new(0, 1, 0),
            engine_version: "1.5.0".into(),
            script_api_version: (1, 2),
            content_schema_version: SchemaVersion::new(1, 0, 0),
            logic_asset_schema_version: SchemaVersion::new(1, 0, 0),
            platform_payloads: vec![PlatformPayload {
                platform: PlatformKind::Desktop,
                asset_ids: vec![AssetId::new("mesh-cube")],
                logic_asset_ids: vec!["logic-player".into()],
                optional_assembly: None,
            }],
            payload_hashes: vec![PayloadHash {
                path: "data/desktop/patch.bundle".into(),
                algorithm: "sha256".into(),
                hash: [0xAA; 32],
            }],
            signature: None,
            rollback: RollbackMetadata {
                previous_manifest_hash: None,
                fallback_manifest_path: None,
                min_safe_engine_version: "1.4.0".into(),
            },
            created_at: "2026-05-29T12:00:00Z".into(),
        }
    }

    // ── Round-trip tests ───────────────────────────────────────────────

    #[test]
    fn manifest_minimal_roundtrip() {
        let manifest = sample_manifest();
        let json = serde_json::to_string_pretty(&manifest).expect("serialize to JSON");
        let deserialized: HotUpdateManifest =
            serde_json::from_str(&json).expect("deserialize from JSON");

        assert_eq!(deserialized.manifest_version, manifest.manifest_version);
        assert_eq!(deserialized.engine_version, manifest.engine_version);
        assert_eq!(deserialized.script_api_version, manifest.script_api_version);
        assert_eq!(
            deserialized.content_schema_version,
            manifest.content_schema_version
        );
        assert_eq!(
            deserialized.logic_asset_schema_version,
            manifest.logic_asset_schema_version
        );
        assert_eq!(
            deserialized.platform_payloads.len(),
            manifest.platform_payloads.len()
        );
        assert_eq!(
            deserialized.payload_hashes.len(),
            manifest.payload_hashes.len()
        );
        assert_eq!(deserialized.created_at, manifest.created_at);
    }

    // ── Compatibility tests ────────────────────────────────────────────

    #[test]
    fn manifest_compatible_accepts() {
        let manifest = sample_manifest();
        let result = manifest.check_compatibility("1.5.0", (1, 5), PlatformKind::Desktop);
        assert_eq!(
            result,
            CompatibilityResult::Compatible,
            "expected compatible, got: {result:?}"
        );
    }

    #[test]
    fn manifest_engine_mismatch_rejects() {
        let manifest = sample_manifest();
        let result = manifest.check_compatibility("2.0.0", (1, 5), PlatformKind::Desktop);
        assert!(
            matches!(result, CompatibilityResult::Incompatible { .. }),
            "expected Incompatible, got: {result:?}"
        );
        if let CompatibilityResult::Incompatible { ref reasons } = result {
            assert!(
                reasons.iter().any(|r| r.contains("major mismatch")),
                "expected reason about major mismatch, got: {reasons:?}"
            );
        }
    }

    #[test]
    fn manifest_script_api_out_of_range_rejects() {
        let manifest = sample_manifest();
        // manifest script_api_version is (1, 2), current is (0, 5),
        // so max = (0 + 1, 0) = (1, 0) and (1, 2) > (1, 0) → out of range
        let result = manifest.check_compatibility("1.5.0", (0, 5), PlatformKind::Desktop);
        assert!(
            matches!(result, CompatibilityResult::Incompatible { .. }),
            "expected Incompatible, got: {result:?}"
        );
        if let CompatibilityResult::Incompatible { ref reasons } = result {
            assert!(
                reasons.iter().any(|r| r.contains("out of range")),
                "expected reason about API version range, got: {reasons:?}"
            );
        }
    }

    #[test]
    fn manifest_ios_rejects_assembly_payload() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads.push(PlatformPayload {
            platform: PlatformKind::Ios,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(AssemblyPayload {
                path: "ios/assembly.dll".into(),
                size_bytes: 1024,
                hash: [0xBB; 32],
                min_engine_version: "1.5.0".into(),
            }),
        });
        let result = manifest.check_compatibility("1.5.0", (1, 5), PlatformKind::Ios);
        assert!(
            matches!(result, CompatibilityResult::Incompatible { .. }),
            "expected Incompatible for iOS with assembly, got: {result:?}"
        );
        if let CompatibilityResult::Incompatible { ref reasons } = result {
            assert!(
                reasons
                    .iter()
                    .any(|r| r.contains("iOS") && r.contains("assembly")),
                "expected reason about iOS assembly rejection, got: {reasons:?}"
            );
        }
    }

    #[test]
    fn manifest_android_allows_assembly_payload() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads.push(PlatformPayload {
            platform: PlatformKind::Android,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(AssemblyPayload {
                path: "android/assembly.dll".into(),
                size_bytes: 2048,
                hash: [0xCC; 32],
                min_engine_version: "1.5.0".into(),
            }),
        });
        let result = manifest.check_compatibility("1.5.0", (1, 5), PlatformKind::Android);
        assert_eq!(
            result,
            CompatibilityResult::Compatible,
            "expected Compatible for Android with assembly, got: {result:?}"
        );
    }

    // ── Validation tests ───────────────────────────────────────────────

    #[test]
    fn manifest_validate_missing_fields() {
        let manifest = HotUpdateManifest {
            manifest_version: SchemaVersion::new(0, 0, 0),
            engine_version: "".into(),
            script_api_version: (0, 1),
            content_schema_version: SchemaVersion::new(0, 0, 0),
            logic_asset_schema_version: SchemaVersion::new(0, 0, 0),
            platform_payloads: vec![],
            payload_hashes: vec![],
            signature: None,
            rollback: RollbackMetadata {
                previous_manifest_hash: None,
                fallback_manifest_path: None,
                min_safe_engine_version: "".into(),
            },
            created_at: "".into(),
        };
        let errors = manifest.validate();
        assert!(!errors.is_empty(), "expected errors, got empty");
        assert!(
            errors.iter().any(|e| e.contains("manifest_version")),
            "expected manifest_version error, got: {errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.contains("engine_version")),
            "expected engine_version error, got: {errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.contains("content_schema_version")),
            "expected content_schema_version error, got: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("logic_asset_schema_version")),
            "expected logic_asset_schema_version error, got: {errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.contains("platform_payloads")),
            "expected platform_payloads error, got: {errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.contains("created_at")),
            "expected created_at error, got: {errors:?}"
        );
    }

    #[test]
    fn manifest_validate_duplicate_payload_hash() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes.push(PayloadHash {
            path: "data/desktop/patch.bundle".into(),
            algorithm: "sha256".into(),
            hash: [0xBB; 32],
        });
        let errors = manifest.validate();
        assert!(
            errors.iter().any(|e| e.contains("duplicate")),
            "expected duplicate hash error, got: {errors:?}"
        );
    }

    #[test]
    fn manifest_validate_empty_payload_hash_path() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes.push(PayloadHash {
            path: "".into(),
            algorithm: "sha256".into(),
            hash: [0xBB; 32],
        });
        let errors = manifest.validate();
        assert!(
            errors.iter().any(|e| e.contains("payload_hash path")),
            "expected empty path error, got: {errors:?}"
        );
    }

    // ── Hash verification tests ────────────────────────────────────────

    #[test]
    fn manifest_payload_hash_verification() {
        let data = b"test payload data for hash verification";
        let computed_hash: [u8; 32] = Sha256::digest(data).into();

        let mut manifest = sample_manifest();
        manifest.payload_hashes[0].hash = computed_hash;

        // Correct data verifies.
        assert!(manifest.verify_payload_hash("data/desktop/patch.bundle", data));

        // Wrong data fails.
        assert!(!manifest.verify_payload_hash("data/desktop/patch.bundle", b"tampered data"));

        // Non-existent path fails.
        assert!(!manifest.verify_payload_hash("nonexistent/path", data));
    }

    // ── payloads_for_platform tests ────────────────────────────────────

    #[test]
    fn manifest_payloads_for_platform_filters_by_kind() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads.push(PlatformPayload {
            platform: PlatformKind::All,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: None,
        });
        manifest.platform_payloads.push(PlatformPayload {
            platform: PlatformKind::Android,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: None,
        });

        // Desktop should get [Desktop, All].
        let desktop = manifest.payloads_for_platform(PlatformKind::Desktop);
        assert_eq!(desktop.len(), 2);
        assert!(desktop.iter().any(|p| p.platform == PlatformKind::Desktop));
        assert!(desktop.iter().any(|p| p.platform == PlatformKind::All));

        // Android should get [All, Android].
        let android = manifest.payloads_for_platform(PlatformKind::Android);
        assert_eq!(android.len(), 2);
        assert!(android.iter().any(|p| p.platform == PlatformKind::Android));
        assert!(android.iter().any(|p| p.platform == PlatformKind::All));
    }

    // ── Edge-case tests ────────────────────────────────────────────────

    #[test]
    fn manifest_engine_above_min_still_compatible() {
        let manifest = sample_manifest();
        // engine is "1.5.0", current "1.99.0" — same major (1).
        let result = manifest.check_compatibility("1.99.0", (1, 5), PlatformKind::Desktop);
        assert_eq!(result, CompatibilityResult::Compatible);
    }

    #[test]
    fn manifest_script_api_at_lower_bound() {
        let mut manifest = sample_manifest();
        manifest.script_api_version = (0, 1);
        let result = manifest.check_compatibility("1.5.0", (1, 5), PlatformKind::Desktop);
        assert_eq!(result, CompatibilityResult::Compatible);
    }

    #[test]
    fn manifest_script_api_at_upper_bound() {
        let mut manifest = sample_manifest();
        // current is (1, 5), so max = (2, 0). Set script_api to (1, 5) — within range.
        manifest.script_api_version = (2, 0);
        let result = manifest.check_compatibility("1.5.0", (1, 5), PlatformKind::Desktop);
        assert_eq!(result, CompatibilityResult::Compatible);
    }
}
