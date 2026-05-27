#![forbid(unsafe_code)]

mod manager;
mod manifest;
mod verify;

pub use manager::PackageManager;
pub use manifest::{HotUpdateError, InstallationSnapshot, PackageFileEntry, PackageManifest, PackageState};
pub use verify::compute_hash;

#[cfg(test)]
mod tests {
    use super::*;

    // ── HotUpdateError display tests ─────────────────────────────────────

    #[test]
    fn hot_update_error_package_not_found_display() {
        let err = HotUpdateError::PackageNotFound("my-pkg".to_string());
        assert_eq!(err.to_string(), "package not found: my-pkg");
    }

    #[test]
    fn hot_update_error_verification_failed_display() {
        let err = HotUpdateError::VerificationFailed {
            expected: [1u8; 32],
            actual: [2u8; 32],
        };
        let msg = err.to_string();
        assert!(msg.contains("verification failed"));
    }

    #[test]
    fn hot_update_error_install_failed_display() {
        let err = HotUpdateError::InstallFailed("disk full".to_string());
        assert_eq!(err.to_string(), "install failed: disk full");
    }

    #[test]
    fn hot_update_error_rollback_failed_display() {
        let err = HotUpdateError::RollbackFailed("no backup".to_string());
        assert_eq!(err.to_string(), "rollback failed: no backup");
    }

    #[test]
    fn hot_update_error_io_display() {
        let err = HotUpdateError::Io("permission denied".to_string());
        assert_eq!(err.to_string(), "io error: permission denied");
    }

    #[test]
    fn hot_update_error_invalid_package_display() {
        let err = HotUpdateError::InvalidPackage("corrupted header".to_string());
        assert_eq!(err.to_string(), "invalid package: corrupted header");
    }

    // ── PackageState tests ───────────────────────────────────────────────

    #[test]
    fn package_state_not_installed() {
        assert_eq!(PackageState::NotInstalled, PackageState::NotInstalled);
    }

    #[test]
    fn package_state_downloading() {
        let state = PackageState::Downloading { progress: 0.5 };
        assert_eq!(state, PackageState::Downloading { progress: 0.5 });
        assert_ne!(state, PackageState::Downloading { progress: 0.0 });
    }

    #[test]
    fn package_state_ready_to_install() {
        assert_eq!(PackageState::ReadyToInstall, PackageState::ReadyToInstall);
    }

    #[test]
    fn package_state_installed() {
        let state = PackageState::Installed {
            version: "1.0.0".to_string(),
            installed_at: "12345".to_string(),
        };
        assert_eq!(
            state,
            PackageState::Installed {
                version: "1.0.0".to_string(),
                installed_at: "12345".to_string(),
            }
        );
    }

    #[test]
    fn package_state_install_failed() {
        let state = PackageState::InstallFailed {
            error: "out of memory".to_string(),
        };
        assert_eq!(
            state,
            PackageState::InstallFailed {
                error: "out of memory".to_string(),
            }
        );
    }

    #[test]
    fn package_state_rolled_back() {
        let state = PackageState::RolledBack {
            previous_version: "0.9.0".to_string(),
        };
        assert_eq!(
            state,
            PackageState::RolledBack {
                previous_version: "0.9.0".to_string(),
            }
        );
    }

    // ── compute_hash tests ───────────────────────────────────────────────

    #[test]
    fn compute_hash_empty_data() {
        let hash = compute_hash(b"");
        assert_eq!(hash.len(), 32);
        // SHA-256 of empty string is known
        let expected: [u8; 32] = [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14,
            0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24,
            0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c,
            0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
        ];
        assert_eq!(hash, expected);
    }

    #[test]
    fn compute_hash_known_input() {
        let hash = compute_hash(b"hello");
        let expected: [u8; 32] = [
            0x2c, 0xf2, 0x4d, 0xba, 0x5f, 0xb0, 0xa3, 0x0e,
            0x26, 0xe8, 0x3b, 0x2a, 0xc5, 0xb9, 0xe2, 0x9e,
            0x1b, 0x16, 0x1e, 0x5c, 0x1f, 0xa7, 0x42, 0x5e,
            0x73, 0x04, 0x33, 0x62, 0x93, 0x8b, 0x98, 0x24,
        ];
        assert_eq!(hash, expected);
    }

    #[test]
    fn compute_hash_consistency() {
        let data = b"some package data";
        let h1 = compute_hash(data);
        let h2 = compute_hash(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_hash_different_inputs_different() {
        let h1 = compute_hash(b"data1");
        let h2 = compute_hash(b"data2");
        assert_ne!(h1, h2);
    }

    // ── PackageManifest tests ────────────────────────────────────────────

    #[test]
    fn package_manifest_construction() {
        let manifest = PackageManifest {
            package_id: "test-pkg".to_string(),
            version: "1.0.0".to_string(),
            required_engine_version: "0.1.0".to_string(),
            files: vec![],
            content_hash: [0u8; 32],
            dependencies: vec![],
            description: "A test package".to_string(),
            release_notes: "Initial release".to_string(),
        };
        assert_eq!(manifest.package_id, "test-pkg");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.description, "A test package");
    }

    #[test]
    fn package_manifest_debug() {
        let manifest = PackageManifest {
            package_id: "pkg".to_string(),
            version: "1.0.0".to_string(),
            required_engine_version: "0.1.0".to_string(),
            files: vec![],
            content_hash: [0u8; 32],
            dependencies: vec![],
            description: String::new(),
            release_notes: String::new(),
        };
        let debug = format!("{:?}", manifest);
        assert!(debug.contains("PackageManifest"));
    }
}
