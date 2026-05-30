use std::path::{Path, PathBuf};

use engine_serialize::HotUpdateManifest;
use tracing::{debug, info, warn};

use crate::error::UpdateError;
use crate::package::{Package, PackageState};

// ---------------------------------------------------------------------------
// PackageCache
// ---------------------------------------------------------------------------

/// Versioned package cache that manages the on-disk directory hierarchy.
///
/// Directory layout under `base_dir`:
/// ```text
/// packages/<id>/manifest.json   — serialised manifest
/// packages/<id>/state.json      — serialised state
/// staged/<id>/                  — verified, ready-to-activate payloads
/// active/<id>/                  — currently active payloads
/// previous/<id>/                — previous active (for rollback)
/// active_pointer.txt            — package_id of the active package
/// boot_marker                   — created on activation, deleted on success
/// ```
pub struct PackageCache {
    /// Root directory for all cache data.
    pub(crate) base_dir: PathBuf,
}

impl PackageCache {
    /// Create a new cache rooted at `base_dir`.
    ///
    /// Does **not** create directories or validate the layout — call
    /// [`initialize`](Self::initialize) for that.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            base_dir: base_dir.to_path_buf(),
        }
    }

    /// Initialize or validate the cache directory structure.
    ///
    /// Creates the `packages/`, `staged/`, `active/`, and `previous/`
    /// subdirectories if they do not exist.
    pub fn initialize(&self) -> Result<(), UpdateError> {
        for subdir in &["packages", "staged", "active", "previous"] {
            let path = self.base_dir.join(subdir);
            if !path.exists() {
                info!(dir = %path.display(), "creating cache directory");
                std::fs::create_dir_all(&path)?;
            }
        }
        debug!("cache initialised at {:?}", self.base_dir);
        Ok(())
    }

    /// Return the `packages/` metadata directory for a given package ID.
    fn meta_dir(&self, pkg_id: &str) -> PathBuf {
        self.base_dir.join("packages").join(pkg_id)
    }

    /// Return the staged directory for a given package ID.
    fn staged_dir(&self, pkg_id: &str) -> PathBuf {
        self.base_dir.join("staged").join(pkg_id)
    }

    /// Return the active directory for a given package ID.
    fn active_dir(&self, pkg_id: &str) -> PathBuf {
        self.base_dir.join("active").join(pkg_id)
    }

    /// Return the previous directory for a given package ID.
    fn previous_dir(&self, pkg_id: &str) -> PathBuf {
        self.base_dir.join("previous").join(pkg_id)
    }

    /// Path to the active pointer file.
    fn active_pointer_path(&self) -> PathBuf {
        self.base_dir.join("active_pointer.txt")
    }

    /// Path to the boot marker.
    pub fn boot_marker_path(&self) -> PathBuf {
        self.base_dir.join("boot_marker")
    }

    /// Persist the package's manifest and state to disk.
    pub fn write_state(&self, package: &Package) -> Result<(), UpdateError> {
        let meta_dir = self.meta_dir(package.package_id());
        std::fs::create_dir_all(&meta_dir)?;

        // Write manifest.
        let manifest_path = meta_dir.join("manifest.json");
        let manifest_json = serde_json::to_string_pretty(&package.manifest)?;
        std::fs::write(&manifest_path, &manifest_json)?;

        // Write state.
        let state_path = meta_dir.join("state.json");
        let state_json = serde_json::to_string_pretty(&package.state)?;
        std::fs::write(&state_path, &state_json)?;

        Ok(())
    }

    /// Read a persisted package from disk.
    ///
    /// Returns `Err(UpdateError::CacheCorrupt(...))` if the metadata is
    /// missing or unparseable.
    pub fn read_state(&self, package_id: &str) -> Result<Package, UpdateError> {
        let meta_dir = self.meta_dir(package_id);

        let manifest_path = meta_dir.join("manifest.json");
        if !manifest_path.exists() {
            return Err(UpdateError::CacheCorrupt(format!(
                "manifest not found for package {package_id}"
            )));
        }
        let manifest_json = std::fs::read_to_string(&manifest_path)?;
        let manifest: HotUpdateManifest = serde_json::from_str(&manifest_json)?;

        let state_path = meta_dir.join("state.json");
        let state = if state_path.exists() {
            let state_json = std::fs::read_to_string(&state_path)?;
            serde_json::from_str(&state_json).map_err(|e| {
                UpdateError::CacheCorrupt(format!("invalid state for {package_id}: {e}"))
            })?
        } else {
            PackageState::Discovered
        };

        let mut pkg = Package::new(manifest, &self.base_dir);
        pkg.state = state;
        Ok(pkg)
    }

    /// List all known packages by scanning the `packages/` directory.
    pub fn list_packages(&self) -> Vec<Package> {
        let packages_dir = self.base_dir.join("packages");
        let mut packages = Vec::new();

        let entries = match std::fs::read_dir(&packages_dir) {
            Ok(e) => e,
            Err(_) => return packages,
        };

        for entry in entries.flatten() {
            let dir_name = entry.file_name();
            let pkg_id = dir_name.to_string_lossy().to_string();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                match self.read_state(&pkg_id) {
                    Ok(pkg) => packages.push(pkg),
                    Err(e) => {
                        warn!("failed to read package {pkg_id}: {e}");
                    }
                }
            }
        }

        packages
    }

    /// Get the currently active package.
    ///
    /// Reads the `active_pointer.txt` file to determine which package is
    /// active, then loads its state.
    pub fn active_package(&self) -> Option<Package> {
        let pointer_path = self.active_pointer_path();
        if !pointer_path.exists() {
            return None;
        }

        let pkg_id = std::fs::read_to_string(&pointer_path).ok()?;
        let pkg_id = pkg_id.trim();
        if pkg_id.is_empty() {
            return None;
        }

        let mut pkg = self.read_state(pkg_id).ok()?;
        pkg.state = PackageState::Active;
        pkg.active_path = self.active_dir(pkg_id);
        pkg.staged_path = self.staged_dir(pkg_id);
        Some(pkg)
    }

    /// Get a specific package by ID.
    pub fn get_package(&self, package_id: &str) -> Option<Package> {
        self.read_state(package_id).ok()
    }

    /// Set the active pointer to a given package ID.
    pub fn set_active_pointer(&self, package_id: &str) -> Result<(), UpdateError> {
        std::fs::write(self.active_pointer_path(), package_id)?;
        Ok(())
    }

    /// Clean up old packages beyond the retention limit.
    ///
    /// Keeps the `keep_count` most-recently-written packages (by manifest
    /// creation date).  Also removes associated staged and active
    /// directories.
    pub fn gc(&self, keep_count: usize) -> Result<(), UpdateError> {
        let mut packages = self.list_packages();

        // Sort by creation date (newest first).
        packages.sort_by(|a, b| b.manifest.created_at.cmp(&a.manifest.created_at));

        if packages.len() <= keep_count {
            return Ok(());
        }

        for pkg in &packages[keep_count..] {
            let id = pkg.package_id().to_string();
            info!("GC: removing package {id}");

            // Remove metadata.
            let meta_dir = self.meta_dir(&id);
            if meta_dir.exists() {
                std::fs::remove_dir_all(&meta_dir)?;
            }

            // Remove staged.
            let staged = self.staged_dir(&id);
            if staged.exists() {
                std::fs::remove_dir_all(&staged)?;
            }

            // Remove active (but not if it's the current active).
            if let Some(active) = self.active_package() {
                if active.package_id() != id {
                    let active_dir = self.active_dir(&id);
                    if active_dir.exists() {
                        std::fs::remove_dir_all(&active_dir)?;
                    }
                }
            } else {
                let active_dir = self.active_dir(&id);
                if active_dir.exists() {
                    std::fs::remove_dir_all(&active_dir)?;
                }
            }

            // Remove previous.
            let previous = self.previous_dir(&id);
            if previous.exists() {
                std::fs::remove_dir_all(&previous)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::{
        AssetId, PlatformKind, PlatformPayload, RollbackMetadata, SchemaVersion,
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

    fn setup_cache() -> (PackageCache, Package, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();
        let manifest = sample_manifest();
        let pkg = Package::new(manifest, tmp.path());
        (cache, pkg, tmp)
    }

    #[test]
    fn cache_initialise_creates_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        assert!(tmp.path().join("packages").exists());
        assert!(tmp.path().join("staged").exists());
        assert!(tmp.path().join("active").exists());
        assert!(tmp.path().join("previous").exists());
    }

    #[test]
    fn cache_write_and_read_state() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let pkg = Package::new(manifest, tmp.path());
        cache.write_state(&pkg).unwrap();

        let loaded = cache.read_state(pkg.package_id()).unwrap();
        assert_eq!(loaded.package_id(), pkg.package_id());
        assert_eq!(loaded.state, PackageState::Discovered);
    }

    #[test]
    fn cache_read_state_missing_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let result = cache.read_state("nonexistent");
        assert!(result.is_err());
        assert!(matches!(result, Err(UpdateError::CacheCorrupt(_))));
    }

    #[test]
    fn cache_list_packages_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        assert!(cache.list_packages().is_empty());
    }

    #[test]
    fn cache_list_packages_after_write() {
        let (cache, pkg, _tmp) = setup_cache();
        cache.write_state(&pkg).unwrap();

        let packages = cache.list_packages();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_id(), pkg.package_id());
    }

    #[test]
    fn cache_active_package_none_when_no_pointer() {
        let (cache, _pkg, _tmp) = setup_cache();
        assert!(cache.active_package().is_none());
    }

    #[test]
    fn cache_active_package_returns_active() {
        let (cache, pkg, _tmp) = setup_cache();
        cache.write_state(&pkg).unwrap();
        cache.set_active_pointer(pkg.package_id()).unwrap();

        let active = cache.active_package().unwrap();
        assert_eq!(active.package_id(), pkg.package_id());
        assert_eq!(active.state, PackageState::Active);
    }

    #[test]
    fn cache_get_package_returns_none_for_unknown() {
        let (cache, _pkg, _tmp) = setup_cache();
        assert!(cache.get_package("unknown").is_none());
    }

    #[test]
    fn cache_get_package_returns_known() {
        let (cache, pkg, _tmp) = setup_cache();
        cache.write_state(&pkg).unwrap();

        let loaded = cache.get_package(pkg.package_id()).unwrap();
        assert_eq!(loaded.package_id(), pkg.package_id());
    }

    #[test]
    fn cache_gc_removes_old_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        // Create two packages with different timestamps.
        let mut m1 = sample_manifest();
        m1.created_at = "2026-01-01T00:00:00Z".into();
        let pkg1 = Package::new(m1, tmp.path());
        cache.write_state(&pkg1).unwrap();

        let mut m2 = sample_manifest();
        m2.created_at = "2026-06-01T00:00:00Z".into();
        let pkg2 = Package::new(m2, tmp.path());
        cache.write_state(&pkg2).unwrap();

        // GC keeping 1 package — should remove pkg1 (older).
        cache.gc(1).unwrap();

        let packages = cache.list_packages();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_id(), pkg2.package_id());
    }

    #[test]
    fn cache_gc_keeps_all_if_under_limit() {
        let (cache, pkg, _tmp) = setup_cache();
        cache.write_state(&pkg).unwrap();
        cache.gc(5).unwrap(); // keep 5, only 1 exists

        let packages = cache.list_packages();
        assert_eq!(packages.len(), 1);
    }

    #[test]
    fn cache_write_state_preserves_state() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let mut pkg = Package::new(manifest.clone(), tmp.path());
        pkg.state = PackageState::Downloaded;
        cache.write_state(&pkg).unwrap();

        let loaded = cache.read_state(pkg.package_id()).unwrap();
        assert_eq!(loaded.state, PackageState::Downloaded);
    }

    #[test]
    fn cache_persists_multiple_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let m1 = sample_manifest();
        let pkg1 = Package::new(m1, tmp.path());
        cache.write_state(&pkg1).unwrap();

        let mut m2 = sample_manifest();
        m2.created_at = "2026-07-01T00:00:00Z".into();
        let pkg2 = Package::new(m2, tmp.path());
        cache.write_state(&pkg2).unwrap();

        assert_eq!(cache.list_packages().len(), 2);
    }

    #[test]
    fn cache_boot_marker_path() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        let marker = cache.boot_marker_path();
        assert!(marker.to_string_lossy().contains("boot_marker"));
    }
}
