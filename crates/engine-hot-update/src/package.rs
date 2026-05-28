use std::path::{Path, PathBuf};

use engine_serialize::HashDigest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// PackageState
// ---------------------------------------------------------------------------

/// Lifecycle state of a hot-update package.
///
/// State machine:
/// ```text
/// Discovered → Downloading → Downloaded → Verified → Staged → Active
///                                                              ↓
///                                                         RolledBack
/// ```
/// At any point a fatal error can transition to `Rejected(reason)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageState {
    /// Package manifest has been discovered but not processed.
    Discovered,
    /// Payloads are being downloaded.
    Downloading,
    /// Payloads have been downloaded but not yet verified.
    Downloaded,
    /// All payloads have passed verification.
    Verified,
    /// Package has been staged and is ready for activation.
    Staged,
    /// Package is currently the active (running) update.
    Active,
    /// Package was rejected during any phase.
    Rejected(String),
    /// Package was rolled back to the previous version.
    RolledBack,
}

// ---------------------------------------------------------------------------
// Package
// ---------------------------------------------------------------------------

/// A hot-update package tracked by the system.
///
/// Each package has a unique identity derived from its manifest, a current
/// lifecycle state, and well-known directories under the cache hierarchy.
#[derive(Debug)]
pub struct Package {
    /// The parsed manifest that defines this package.
    pub manifest: engine_serialize::HotUpdateManifest,
    /// Path where payloads are temporarily staged before activation.
    pub staged_path: PathBuf,
    /// Path where the active (live) payloads reside.
    pub active_path: PathBuf,
    /// Current lifecycle state.
    pub state: PackageState,
    /// Unique identifier for this package (hex-encoded SHA-256 of the
    /// serialised manifest).
    id: String,
    /// Base directory for all cache/managed paths.
    base_dir: PathBuf,
}

impl Package {
    /// Create a new package from a manifest.
    ///
    /// The `base_dir` is the root of the hot-update cache hierarchy; derived
    /// directories (`staging_dir`, `active_dir`, `previous_dir`) are computed
    /// relative to it.
    pub fn new(manifest: engine_serialize::HotUpdateManifest, base_dir: &Path) -> Self {
        let id = compute_package_id(&manifest);
        Self {
            id,
            staged_path: base_dir.join("staged"),
            active_path: base_dir.join("active"),
            state: PackageState::Discovered,
            manifest,
            base_dir: base_dir.to_path_buf(),
        }
    }

    /// Unique identifier for this package.
    pub fn package_id(&self) -> &str {
        &self.id
    }

    /// Directory where the package is or will be staged before activation.
    ///
    /// Path: `<base_dir>/staged/<package_id>/`
    pub fn staging_dir(&self) -> PathBuf {
        self.base_dir.join("staged").join(&self.id)
    }

    /// Directory where the package is or will be active.
    ///
    /// Path: `<base_dir>/active/<package_id>/`
    pub fn active_dir(&self) -> PathBuf {
        self.base_dir.join("active").join(&self.id)
    }

    /// Directory where the previous (now-rolled-back) package is kept.
    ///
    /// Path: `<base_dir>/previous/<package_id>/`
    pub fn previous_dir(&self) -> PathBuf {
        self.base_dir.join("previous").join(&self.id)
    }

    /// Compute the staged payload path for a given manifest payload entry.
    pub fn payload_path(&self, relative: &str) -> PathBuf {
        self.staging_dir().join(relative)
    }
}

/// Compute a unique, content-addressed identifier for a manifest.
pub fn compute_package_id(manifest: &engine_serialize::HotUpdateManifest) -> String {
    let json = serde_json::to_vec(manifest).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&json);
    hex_encode(&hasher.finalize())
}

/// Hex-encode a byte slice (no external crate dependency).
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Compute SHA-256 hash of data.
pub(crate) fn sha256_hash(data: &[u8]) -> HashDigest {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::{
        AssetId, HotUpdateManifest, PlatformKind, PlatformPayload, RollbackMetadata,
        SchemaVersion,
    };

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
            payload_hashes: vec![],
            signature: None,
            rollback: RollbackMetadata {
                previous_manifest_hash: None,
                fallback_manifest_path: None,
                min_safe_engine_version: "1.4.0".into(),
            },
            created_at: "2026-05-29T12:00:00Z".into(),
        }
    }

    #[test]
    fn package_new_creates_discovered() {
        let dir = std::env::temp_dir().join("pkg_test_new");
        let manifest = sample_manifest();
        let pkg = Package::new(manifest.clone(), &dir);
        assert_eq!(pkg.state, PackageState::Discovered);
        assert_eq!(pkg.manifest.engine_version, "1.5.0");
    }

    #[test]
    fn package_id_is_non_empty() {
        let dir = std::env::temp_dir().join("pkg_test_id");
        let manifest = sample_manifest();
        let pkg = Package::new(manifest, &dir);
        assert!(!pkg.package_id().is_empty());
        assert_eq!(pkg.package_id().len(), 64); // hex-encoded SHA-256
    }

    #[test]
    fn package_id_is_deterministic() {
        let dir = std::env::temp_dir().join("pkg_test_det");
        let manifest = sample_manifest();
        let pkg1 = Package::new(manifest.clone(), &dir);
        let pkg2 = Package::new(manifest, &dir);
        assert_eq!(pkg1.package_id(), pkg2.package_id());
    }

    #[test]
    fn package_staging_dir_contains_id() {
        let dir = std::env::temp_dir().join("pkg_test_stage");
        let manifest = sample_manifest();
        let pkg = Package::new(manifest, &dir);
        let staging = pkg.staging_dir();
        assert!(staging.to_string_lossy().contains("staged"));
        assert!(staging.to_string_lossy().contains(pkg.package_id()));
    }

    #[test]
    fn package_active_dir_contains_id() {
        let dir = std::env::temp_dir().join("pkg_test_active");
        let manifest = sample_manifest();
        let pkg = Package::new(manifest, &dir);
        let active = pkg.active_dir();
        assert!(active.to_string_lossy().contains("active"));
        assert!(active.to_string_lossy().contains(pkg.package_id()));
    }

    #[test]
    fn package_previous_dir_contains_id() {
        let dir = std::env::temp_dir().join("pkg_test_prev");
        let manifest = sample_manifest();
        let pkg = Package::new(manifest, &dir);
        let prev = pkg.previous_dir();
        assert!(prev.to_string_lossy().contains("previous"));
        assert!(prev.to_string_lossy().contains(pkg.package_id()));
    }

    #[test]
    fn package_state_transitions_discovered() {
        assert_eq!(PackageState::Discovered, PackageState::Discovered);
        assert_ne!(PackageState::Discovered, PackageState::Downloading);
    }

    #[test]
    fn package_state_active_not_rejected() {
        assert_ne!(PackageState::Active, PackageState::Rejected("x".into()));
    }

    #[test]
    fn package_state_rolled_back() {
        assert_eq!(PackageState::RolledBack, PackageState::RolledBack);
    }

    #[test]
    fn package_state_rejected_contains_message() {
        let r1 = PackageState::Rejected("hash mismatch".into());
        let r2 = PackageState::Rejected("bad signature".into());
        assert_ne!(r1, r2);
    }

    #[test]
    fn hex_encode_empty() {
        assert_eq!(hex_encode(b""), "");
    }

    #[test]
    fn hex_encode_known() {
        assert_eq!(hex_encode(&[0xAB, 0xCD, 0xEF]), "abcdef");
        assert_eq!(hex_encode(&[0x00, 0xFF]), "00ff");
    }

    #[test]
    fn sha256_hash_consistency() {
        let data = b"some test data";
        let h1 = sha256_hash(data);
        let h2 = sha256_hash(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn sha256_hash_different_data() {
        let h1 = sha256_hash(b"data1");
        let h2 = sha256_hash(b"data2");
        assert_ne!(h1, h2);
    }
}
