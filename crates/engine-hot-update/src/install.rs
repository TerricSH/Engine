use std::path::Path;

use engine_serialize::HotUpdateManifest;
use tracing::{debug, info, warn};

use crate::cache::PackageCache;
use crate::error::UpdateError;
use crate::package::{Package, PackageState};

// ---------------------------------------------------------------------------
// Installer
// ---------------------------------------------------------------------------

/// Handles staging and atomic activation of verified packages.
///
/// Activation uses rename-then-remove semantics:
/// 1. The current active directory (if any) is renamed to `previous/`.
/// 2. The staged directory is renamed to `active/`.
/// 3. The active pointer file is updated.
/// 4. A boot marker is created so the engine can detect failed boots.
pub struct Installer;

impl Installer {
    /// Stage a verified package by moving its payloads into the cache's
    /// staged area.
    ///
    /// `staging_dir` is the temporary download directory containing
    /// verified payloads.  The files are moved (not copied) into
    /// `<cache>/staged/<package_id>/` for efficiency.
    ///
    /// Returns the [`Package`] in [`PackageState::Staged`].
    pub fn stage(
        manifest: &HotUpdateManifest,
        staging_dir: &Path,
        cache: &PackageCache,
    ) -> Result<Package, UpdateError> {
        let mut pkg = Package::new(manifest.clone(), &cache.base_dir);
        let staged_dest = pkg.staging_dir();

        // Remove any existing staged directory.
        if staged_dest.exists() {
            debug!("removing existing staged directory: {:?}", staged_dest);
            std::fs::remove_dir_all(&staged_dest)?;
        }

        // Ensure parent exists.
        if let Some(parent) = staged_dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Rename (move) the download staging dir to the cache's staged dir.
        // If rename fails across filesystems, fall back to copy + remove.
        if let Err(e) = std::fs::rename(staging_dir, &staged_dest) {
            warn!(
                "rename from {:?} to {:?} failed ({e}), falling back to copy",
                staging_dir, staged_dest
            );
            copy_dir_all(staging_dir, &staged_dest)?;
            let _ = std::fs::remove_dir_all(staging_dir);
        }

        pkg.state = PackageState::Staged;
        pkg.staged_path = staged_dest;
        pkg.active_path = pkg.active_dir();

        // Persist state.
        cache.write_state(&pkg)?;

        info!(
            package_id = %pkg.package_id(),
            "package staged"
        );

        Ok(pkg)
    }

    /// Atomically activate a staged package.
    ///
    /// 1. If a previous active directory exists, it is removed.
    /// 2. The current active directory is moved to `previous/`.
    /// 3. The staged directory is moved to `active/`.
    /// 4. The active pointer is updated.
    /// 5. A boot marker is created.
    ///
    /// On failure the system is left in a safe state (previous active
    /// is still in `previous/`).
    pub fn activate(package: &mut Package, cache: &PackageCache) -> Result<(), UpdateError> {
        let pkg_id = package.package_id().to_string();
        let staged_dir = package.staging_dir();
        let active_dir = package.active_dir();
        let previous_dir = package.previous_dir();

        // Verify the staged directory exists.
        if !staged_dir.exists() {
            return Err(UpdateError::ActivationFailed(format!(
                "staged directory not found: {:?}",
                staged_dir
            )));
        }

        // 1. Remove the previous directory if it exists (we only keep one
        //    level of rollback).
        if previous_dir.exists() {
            std::fs::remove_dir_all(&previous_dir)?;
        }

        // 2. Move current active to previous (keyed by previous package's ID).
        let current_active = cache.active_package();
        if let Some(ref prev) = current_active {
            let prev_id = prev.package_id().to_string();
            let prev_active_dir = prev.active_dir();
            let rollback_dir = cache.base_dir.join("previous").join(&prev_id);
            if prev_active_dir.exists() {
                if rollback_dir.exists() {
                    std::fs::remove_dir_all(&rollback_dir)?;
                }
                std::fs::rename(&prev_active_dir, &rollback_dir)?;
                debug!(
                    "moved previous active {:?} -> {:?}",
                    prev_active_dir, rollback_dir
                );
            }
        }

        // 3. Rename staged -> active.
        if let Err(e) = std::fs::rename(&staged_dir, &active_dir) {
            // Restore previous if rename fails.
            warn!("activate rename failed ({e}), attempting restore");
            if previous_dir.exists() {
                let restore_dir = if let Some(ref prev) = current_active {
                    prev.active_dir()
                } else {
                    active_dir.clone()
                };
                let _ = std::fs::rename(&previous_dir, &restore_dir);
            }
            return Err(UpdateError::ActivationFailed(format!(
                "failed to rename staged to active: {e}"
            )));
        }

        // 4. Update the active pointer.
        cache.set_active_pointer(&pkg_id)?;

        // 5. Create boot marker.
        std::fs::write(cache.boot_marker_path(), pkg_id.as_bytes())?;

        package.state = PackageState::Active;
        package.active_path = active_dir;
        package.staged_path = staged_dir; // no longer exists, but keep for ref

        // Persist state.
        cache.write_state(package)?;

        info!(package_id = %pkg_id, "package activated");
        Ok(())
    }

    /// Mark the current active package as having failed boot.
    ///
    /// This removes the boot marker, which signals the
    /// [`RollbackManager`](crate::rollback::RollbackManager) that a
    /// rollback is needed.
    pub fn mark_failed_boot(cache: &PackageCache) -> Result<(), UpdateError> {
        let marker = cache.boot_marker_path();
        if marker.exists() {
            std::fs::remove_file(&marker)?;
            info!("boot marker removed — system will detect failed boot");
        }
        // Also create a persistent failure flag.
        let fail_marker = cache.base_dir.join("boot_failed");
        std::fs::write(&fail_marker, "failed")?;
        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Recursively copy a directory.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let src_path = entry.path();
        let dst_path = dst.join(&name);

        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::{
        AssetId, PayloadHash, PlatformKind, PlatformPayload, RollbackMetadata, SchemaVersion,
    };
    use sha2::{Digest, Sha256};

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
                path: "data.bin".into(),
                algorithm: "sha256".into(),
                hash: {
                    let h = Sha256::digest(b"test payload");
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&h);
                    arr
                },
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

    #[test]
    fn installer_stage_moves_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let staging_dir = tmp.path().join("download_temp");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("data.bin"), b"test payload").unwrap();

        let pkg = Installer::stage(&manifest, &staging_dir, &cache).unwrap();
        assert_eq!(pkg.state, PackageState::Staged);
        assert!(pkg.staging_dir().join("data.bin").exists());
    }

    #[test]
    fn installer_stage_persists_state() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let staging_dir = tmp.path().join("download_temp2");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("data.bin"), b"test").unwrap();

        let pkg = Installer::stage(&manifest, &staging_dir, &cache).unwrap();

        // Verify state was persisted.
        let loaded = cache.get_package(pkg.package_id()).unwrap();
        assert_eq!(loaded.state, PackageState::Staged);
    }

    #[test]
    fn installer_activate_switches_active() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let staging_dir = tmp.path().join("download_act");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("data.bin"), b"activate me").unwrap();

        let mut pkg = Installer::stage(&manifest, &staging_dir, &cache).unwrap();
        Installer::activate(&mut pkg, &cache).unwrap();

        assert_eq!(pkg.state, PackageState::Active);
        assert!(pkg.active_dir().join("data.bin").exists());

        // Active pointer should point to this package.
        let active = cache.active_package().unwrap();
        assert_eq!(active.package_id(), pkg.package_id());
    }

    #[test]
    fn installer_activate_creates_boot_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let staging_dir = tmp.path().join("download_boot");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("data.bin"), b"boot").unwrap();

        let mut pkg = Installer::stage(&manifest, &staging_dir, &cache).unwrap();
        Installer::activate(&mut pkg, &cache).unwrap();

        assert!(cache.boot_marker_path().exists());
    }

    #[test]
    fn installer_activate_fails_without_staged_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let mut pkg = Package::new(manifest, tmp.path());

        let result = Installer::activate(&mut pkg, &cache);
        assert!(result.is_err());
        assert!(matches!(result, Err(UpdateError::ActivationFailed(_))));
    }

    #[test]
    fn installer_activate_preserves_previous() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        // Create and activate first package.
        let m1 = sample_manifest();
        let s1 = tmp.path().join("dl_first");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(s1.join("data.bin"), b"first").unwrap();
        let mut pkg1 = Installer::stage(&m1, &s1, &cache).unwrap();
        Installer::activate(&mut pkg1, &cache).unwrap();

        // Create and activate second package.
        let mut m2 = sample_manifest();
        m2.created_at = "2026-06-01T00:00:00Z".into();
        let s2 = tmp.path().join("dl_second");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s2.join("data.bin"), b"second").unwrap();
        let mut pkg2 = Installer::stage(&m2, &s2, &cache).unwrap();
        Installer::activate(&mut pkg2, &cache).unwrap();

        // Previous should still contain the first package's data,
        // now stored under the first package's ID.
        let prev_path = cache.base_dir.join("previous").join(pkg1.package_id());
        let prev_content = std::fs::read(prev_path.join("data.bin")).unwrap();
        assert_eq!(prev_content, b"first");
    }

    #[test]
    fn installer_mark_failed_boot_creates_fail_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        // Create boot marker.
        std::fs::write(cache.boot_marker_path(), "test-pkg").unwrap();

        Installer::mark_failed_boot(&cache).unwrap();

        // Boot marker should be removed.
        assert!(!cache.boot_marker_path().exists());
        // Fail flag should exist.
        assert!(tmp.path().join("boot_failed").exists());
    }

    #[test]
    fn installer_activate_replaces_old_active() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        // First package.
        let m1 = sample_manifest();
        let s1 = tmp.path().join("dl_rep1");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(s1.join("data.bin"), b"v1").unwrap();
        let mut p1 = Installer::stage(&m1, &s1, &cache).unwrap();
        Installer::activate(&mut p1, &cache).unwrap();
        let id1 = p1.package_id().to_string();

        // Second package.
        let mut m2 = sample_manifest();
        m2.created_at = "2026-07-01T00:00:00Z".into();
        let s2 = tmp.path().join("dl_rep2");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s2.join("data.bin"), b"v2").unwrap();
        let mut p2 = Installer::stage(&m2, &s2, &cache).unwrap();
        Installer::activate(&mut p2, &cache).unwrap();

        // Active pointer should now point to p2.
        let active = cache.active_package().unwrap();
        assert_eq!(active.package_id(), p2.package_id());

        // p1 should NOT be in active directory (moved to previous).
        let p1_active = cache.base_dir.join("active").join(&id1);
        assert!(!p1_active.exists());
    }
}
