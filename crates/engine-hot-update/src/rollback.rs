use std::io::Read;

use tracing::{debug, info};

use crate::cache::PackageCache;
use crate::error::UpdateError;
use crate::package::{Package, PackageState};

// ---------------------------------------------------------------------------
// RollbackManager
// ---------------------------------------------------------------------------

/// Manages rollback to a previous known-good package.
pub struct RollbackManager;

impl RollbackManager {
    /// Rollback to the previous known-good package.
    ///
    /// The previous active package (stored in `<cache>/previous/`) is
    /// restored to become the new active package.  The current active
    /// package's directory is removed.
    ///
    /// Returns the newly-activated package on success.
    pub fn rollback(cache: &PackageCache) -> Result<Package, UpdateError> {
        let previous_dir = cache.base_dir.join("previous");

        if !previous_dir.exists() {
            return Err(UpdateError::RollbackFailed(
                "no previous package directory found".into(),
            ));
        }

        // Determine which package ID is in the previous directory by
        // scanning for subdirectories.
        let entries: Vec<_> = std::fs::read_dir(&previous_dir)
            .map_err(|e| UpdateError::RollbackFailed(format!("cannot read previous dir: {e}")))?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();

        if entries.is_empty() {
            // The previous dir might contain files directly (from an older
            // activation).  Try to read the active pointer from before.
            let prev_active = read_previous_pointer(cache);
            match prev_active {
                Some(pkg) => {
                    // Restore from pointer — copy files from previous to active.
                    let pkg_id = pkg.package_id().to_string();
                    let active_dir = cache.base_dir.join("active").join(&pkg_id);
                    if active_dir.exists() {
                        std::fs::remove_dir_all(&active_dir)?;
                    }
                    if let Some(parent) = active_dir.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::rename(&previous_dir, &active_dir)?;

                    cache.set_active_pointer(&pkg_id)?;

                    // Clear the boot marker since we've rolled back.
                    let marker = cache.boot_marker_path();
                    if marker.exists() {
                        std::fs::remove_file(&marker)?;
                    }

                    let mut restored = pkg;
                    restored.state = PackageState::RolledBack;
                    cache.write_state(&restored)?;

                    info!(package_id = %pkg_id, "rollback completed");
                    return Ok(restored);
                }
                None => {
                    return Err(UpdateError::RollbackFailed(
                        "previous directory is empty and no pointer found".into(),
                    ));
                }
            }
        }

        // Use the first subdirectory as the previous package.
        let prev_entry = &entries[0];
        let prev_pkg_id = prev_entry.file_name().to_string_lossy().to_string();

        // Read the previous package state.
        let mut prev_pkg = cache.read_state(&prev_pkg_id).map_err(|_| {
            UpdateError::RollbackFailed(format!(
                "cannot read state for previous package {prev_pkg_id}"
            ))
        })?;

        let active_dir = cache.base_dir.join("active").join(&prev_pkg_id);

        // Remove the current active directory (if different from previous).
        let current_active = cache.active_package();
        if let Some(ref cur) = current_active {
            let cur_active_dir = cur.active_dir();
            if cur_active_dir.exists() && cur_active_dir != active_dir {
                std::fs::remove_dir_all(&cur_active_dir)?;
            }
        }

        // Ensure active parent exists.
        if let Some(parent) = active_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Move previous into active.
        let prev_path = prev_entry.path();
        if active_dir.exists() {
            std::fs::remove_dir_all(&active_dir)?;
        }
        std::fs::rename(&prev_path, &active_dir)?;

        // Update active pointer.
        cache.set_active_pointer(&prev_pkg_id)?;

        // Clear boot marker.
        let marker = cache.boot_marker_path();
        if marker.exists() {
            std::fs::remove_file(&marker)?;
        }

        prev_pkg.state = PackageState::RolledBack;
        cache.write_state(&prev_pkg)?;

        info!(package_id = %prev_pkg_id, "rollback completed");
        Ok(prev_pkg)
    }

    /// Check if a boot marker indicates a boot failure.
    ///
    /// Returns `true` if the boot marker exists (meaning the engine
    /// either hasn't booted since activation or boot failed).
    pub fn needs_rollback(cache: &PackageCache) -> bool {
        let marker = cache.boot_marker_path();
        if marker.exists() {
            debug!("boot marker present — system may need rollback");
            return true;
        }

        // Also check the persistent fail flag.
        let fail_marker = cache.base_dir.join("boot_failed");
        fail_marker.exists()
    }

    /// Remove a specific package version from the cache.
    ///
    /// This removes all traces: metadata, staged, active, and previous
    /// directories.
    pub fn remove_package(cache: &PackageCache, package_id: &str) -> Result<(), UpdateError> {
        info!("removing package: {package_id}");

        // Metadata.
        let meta_dir = cache.base_dir.join("packages").join(package_id);
        if meta_dir.exists() {
            std::fs::remove_dir_all(&meta_dir)?;
        }

        // Staged.
        let staged_dir = cache.base_dir.join("staged").join(package_id);
        if staged_dir.exists() {
            std::fs::remove_dir_all(&staged_dir)?;
        }

        // Active (only if not the currently active package).
        if let Some(active) = cache.active_package() {
            if active.package_id() != package_id {
                let active_dir = cache.base_dir.join("active").join(package_id);
                if active_dir.exists() {
                    std::fs::remove_dir_all(&active_dir)?;
                }
            }
        } else {
            let active_dir = cache.base_dir.join("active").join(package_id);
            if active_dir.exists() {
                std::fs::remove_dir_all(&active_dir)?;
            }
        }

        // Previous.
        let previous_dir = cache.base_dir.join("previous").join(package_id);
        if previous_dir.exists() {
            std::fs::remove_dir_all(&previous_dir)?;
        }

        debug!("package {package_id} removed");
        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Try to read the active pointer file, falling back to the boot marker
/// content.
fn read_previous_pointer(cache: &PackageCache) -> Option<Package> {
    // The boot marker contains the package_id that was being activated.
    let marker = cache.boot_marker_path();
    if marker.exists() {
        let mut content = String::new();
        std::fs::File::open(&marker)
            .ok()?
            .read_to_string(&mut content)
            .ok()?;
        let pkg_id = content.trim();
        if !pkg_id.is_empty() {
            return cache.read_state(pkg_id).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::{
        AssetId, PlatformKind, PlatformPayload, RollbackMetadata, SchemaVersion,
    };

    fn sample_manifest() -> engine_serialize::HotUpdateManifest {
        engine_serialize::HotUpdateManifest {
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

    fn setup_with_active_package() -> (PackageCache, Package, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let staging_dir = tmp.path().join("download");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("data.bin"), b"payload").unwrap();

        let mut pkg = crate::install::Installer::stage(&manifest, &staging_dir, &cache).unwrap();
        crate::install::Installer::activate(&mut pkg, &cache).unwrap();

        (cache, pkg, tmp)
    }

    #[test]
    fn needs_rollback_no_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        assert!(!RollbackManager::needs_rollback(&cache));
    }

    #[test]
    fn needs_rollback_with_boot_marker() {
        let (cache, _pkg, _tmp) = setup_with_active_package();
        assert!(RollbackManager::needs_rollback(&cache));
    }

    #[test]
    fn needs_rollback_with_fail_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        std::fs::write(tmp.path().join("boot_failed"), "failed").unwrap();
        assert!(RollbackManager::needs_rollback(&cache));
    }

    #[test]
    fn needs_rollback_cleared_after_boot() {
        let (cache, _pkg, _tmp) = setup_with_active_package();

        // Simulate successful boot by removing the marker.
        let marker = cache.boot_marker_path();
        if marker.exists() {
            std::fs::remove_file(&marker).unwrap();
        }

        assert!(!RollbackManager::needs_rollback(&cache));
    }

    #[test]
    fn rollback_restores_previous_package() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        // First package.
        let m1 = sample_manifest();
        let s1 = tmp.path().join("dl1");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(s1.join("data.bin"), b"version1").unwrap();
        let mut p1 = crate::install::Installer::stage(&m1, &s1, &cache).unwrap();
        crate::install::Installer::activate(&mut p1, &cache).unwrap();

        // Second package.
        let mut m2 = sample_manifest();
        m2.created_at = "2026-06-01T00:00:00Z".into();
        let s2 = tmp.path().join("dl2");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s2.join("data.bin"), b"version2").unwrap();
        let mut p2 = crate::install::Installer::stage(&m2, &s2, &cache).unwrap();
        crate::install::Installer::activate(&mut p2, &cache).unwrap();

        // Rollback.
        let rolled_back = RollbackManager::rollback(&cache).unwrap();
        assert_eq!(rolled_back.state, PackageState::RolledBack);

        // The active package should now be the first one.
        let active = cache.active_package().unwrap();
        assert_eq!(active.package_id(), p1.package_id());
    }

    #[test]
    fn rollback_fails_without_previous() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let result = RollbackManager::rollback(&cache);
        assert!(result.is_err());
        assert!(matches!(result, Err(UpdateError::RollbackFailed(_))));
    }

    #[test]
    fn remove_package_cleans_directories() {
        let (cache, pkg, _tmp) = setup_with_active_package();
        let pkg_id = pkg.package_id().to_string();

        RollbackManager::remove_package(&cache, &pkg_id).unwrap();

        assert!(cache.get_package(&pkg_id).is_none());
    }

    #[test]
    fn remove_package_unknown_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        // Should not error for unknown package.
        assert!(RollbackManager::remove_package(&cache, "nonexistent").is_ok());
    }

    #[test]
    fn rollback_creates_valid_active_pointer() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let m1 = sample_manifest();
        let s1 = tmp.path().join("dl_r1");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(s1.join("data.bin"), b"v1").unwrap();
        let mut p1 = crate::install::Installer::stage(&m1, &s1, &cache).unwrap();
        crate::install::Installer::activate(&mut p1, &cache).unwrap();
        let id1 = p1.package_id().to_string();

        let mut m2 = sample_manifest();
        m2.created_at = "2026-07-01T00:00:00Z".into();
        let s2 = tmp.path().join("dl_r2");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s2.join("data.bin"), b"v2").unwrap();
        let mut p2 = crate::install::Installer::stage(&m2, &s2, &cache).unwrap();
        crate::install::Installer::activate(&mut p2, &cache).unwrap();

        RollbackManager::rollback(&cache).unwrap();

        let active = cache.active_package().unwrap();
        assert_eq!(active.package_id(), id1);
    }
}
