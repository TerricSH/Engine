use std::path::{Path, PathBuf};

use engine_serialize::HotUpdateManifest;
use tracing::{debug, info, warn};

use crate::atomic_fs::atomic_write;
use crate::error::UpdateError;
use crate::package::{Package, PackageState};
use crate::path_safety::{
    is_link_or_reparse, remove_dir_all_safe, safe_join, safe_package_path,
    validate_manifest_paths_once, validate_package_id,
};

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
/// active/<id>/                  — immutable activated payloads (old versions retained)
/// previous/<id>/                — legacy rollback payloads (migration only)
/// active_pointer.txt            — atomically replaced package_id commit point
/// activation_record.json        — deterministic rollback metadata
/// activation_transaction.json   — interrupted-operation recovery journal
/// boot_marker                   — pending-boot activation record
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
            let path = safe_join(&self.base_dir, subdir, "cache directory")?;
            if !path.exists() {
                info!(dir = %path.display(), "creating cache directory");
                std::fs::create_dir_all(&path)?;
            }
        }
        self.recover_interrupted_transaction()?;
        debug!("cache initialised at {:?}", self.base_dir);
        Ok(())
    }

    /// Return the `packages/` metadata directory for a given package ID.
    pub(crate) fn meta_dir(&self, pkg_id: &str) -> Result<PathBuf, UpdateError> {
        safe_package_path(&self.base_dir, "packages", pkg_id)
    }

    /// Return the staged directory for a given package ID.
    pub(crate) fn staged_dir(&self, pkg_id: &str) -> Result<PathBuf, UpdateError> {
        safe_package_path(&self.base_dir, "staged", pkg_id)
    }

    /// Return the active directory for a given package ID.
    pub(crate) fn active_dir(&self, pkg_id: &str) -> Result<PathBuf, UpdateError> {
        safe_package_path(&self.base_dir, "active", pkg_id)
    }

    /// Return the previous directory for a given package ID.
    pub(crate) fn previous_dir(&self, pkg_id: &str) -> Result<PathBuf, UpdateError> {
        safe_package_path(&self.base_dir, "previous", pkg_id)
    }

    /// Path to the active pointer file.
    pub(crate) fn active_pointer_path(&self) -> Result<PathBuf, UpdateError> {
        safe_join(&self.base_dir, "active_pointer.txt", "active pointer")
    }

    /// Path to the boot marker.
    pub fn boot_marker_path(&self) -> PathBuf {
        self.base_dir.join("boot_marker")
    }

    pub(crate) fn checked_boot_marker_path(&self) -> Result<PathBuf, UpdateError> {
        safe_join(&self.base_dir, "boot_marker", "boot marker")
    }

    /// Persist the package's manifest and state to disk.
    pub fn write_state(&self, package: &Package) -> Result<(), UpdateError> {
        validate_package_id(package.package_id())?;
        validate_manifest_paths_once(&package.manifest)?;
        let meta_dir = self.meta_dir(package.package_id())?;
        std::fs::create_dir_all(&meta_dir)?;

        // Write manifest.
        let manifest_path = safe_join(&meta_dir, "manifest.json", "cached manifest")?;
        let manifest_json = serde_json::to_string_pretty(&package.manifest)?;
        atomic_write(&manifest_path, manifest_json.as_bytes())?;

        // Write state.
        let state_path = safe_join(&meta_dir, "state.json", "cached package state")?;
        let state_json = serde_json::to_string_pretty(&package.state)?;
        atomic_write(&state_path, state_json.as_bytes())?;

        Ok(())
    }

    /// Read a persisted package from disk.
    ///
    /// Returns `Err(UpdateError::CacheCorrupt(...))` if the metadata is
    /// missing or unparseable.
    pub fn read_state(&self, package_id: &str) -> Result<Package, UpdateError> {
        validate_package_id(package_id)?;
        let meta_dir = self.meta_dir(package_id)?;

        let manifest_path = safe_join(&meta_dir, "manifest.json", "cached manifest")?;
        if !manifest_path.exists() {
            return Err(UpdateError::CacheCorrupt(format!(
                "manifest not found for package {package_id}"
            )));
        }
        let manifest_json = std::fs::read_to_string(&manifest_path)?;
        let manifest: HotUpdateManifest =
            serde_json::from_str(&manifest_json).map_err(|error| {
                UpdateError::CacheCorrupt(format!("invalid manifest for {package_id}: {error}"))
            })?;
        validate_manifest_paths_once(&manifest)?;

        let state_path = safe_join(&meta_dir, "state.json", "cached package state")?;
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
        self.try_list_packages().unwrap_or_else(|error| {
            warn!("cannot list package cache: {error}");
            Vec::new()
        })
    }

    /// Strict package listing that does not hide cache or I/O errors.
    pub fn try_list_packages(&self) -> Result<Vec<Package>, UpdateError> {
        let packages_dir = safe_join(&self.base_dir, "packages", "package metadata root")?;
        let mut packages = Vec::new();

        let entries = std::fs::read_dir(&packages_dir)?;

        for entry in entries {
            let entry = entry?;
            let dir_name = entry.file_name();
            let pkg_id = dir_name.into_string().map_err(|_| {
                UpdateError::CacheCorrupt("non-UTF-8 package cache directory".into())
            })?;
            validate_package_id(&pkg_id).map_err(|error| {
                UpdateError::CacheCorrupt(format!(
                    "invalid package cache directory {pkg_id:?}: {error}"
                ))
            })?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if !metadata.is_dir() || is_link_or_reparse(&metadata) {
                return Err(UpdateError::CacheCorrupt(format!(
                    "package cache entry is not a regular directory: {}",
                    entry.path().display()
                )));
            }
            packages.push(self.read_state(&pkg_id)?);
        }

        Ok(packages)
    }

    /// Get the currently active package.
    ///
    /// Reads the `active_pointer.txt` file to determine which package is
    /// active, then loads its state.
    pub fn active_package(&self) -> Option<Package> {
        self.try_active_package().ok().flatten()
    }

    /// Strict active-package lookup that preserves recovery and cache errors.
    pub fn try_active_package(&self) -> Result<Option<Package>, UpdateError> {
        let Some(pkg_id) = self.active_package_id()? else {
            return Ok(None);
        };

        let mut pkg = self.read_state(&pkg_id)?;
        pkg.state = PackageState::Active;
        pkg.active_path = self.active_dir(&pkg_id)?;
        pkg.staged_path = self.staged_dir(&pkg_id)?;
        if !pkg.active_path.is_dir() {
            return Err(UpdateError::CacheCorrupt(format!(
                "active pointer {pkg_id} has no immutable payload directory"
            )));
        }
        Ok(Some(pkg))
    }

    /// Read the durable record that determines the exact rollback target.
    pub fn activation_record(&self) -> Result<Option<crate::ActivationRecord>, UpdateError> {
        self.read_activation_record()
    }

    /// Strictly read and validate the active pointer.
    pub(crate) fn active_package_id(&self) -> Result<Option<String>, UpdateError> {
        let pointer_path = self.active_pointer_path()?;
        if !pointer_path.exists() {
            return Ok(None);
        }
        let package_id = std::fs::read_to_string(&pointer_path)?;
        let package_id = package_id.trim();
        if package_id.is_empty() {
            return Err(UpdateError::CacheCorrupt("active pointer is empty".into()));
        }
        validate_package_id(package_id).map_err(|error| {
            UpdateError::CacheCorrupt(format!("active pointer contains an invalid id: {error}"))
        })?;
        Ok(Some(package_id.to_string()))
    }

    /// Get a specific package by ID.
    pub fn get_package(&self, package_id: &str) -> Option<Package> {
        self.read_state(package_id).ok()
    }

    /// Set the active pointer to a given package ID.
    pub(crate) fn set_active_pointer(&self, package_id: &str) -> Result<(), UpdateError> {
        validate_package_id(package_id)?;
        if !self.active_dir(package_id)?.is_dir() {
            return Err(UpdateError::CacheCorrupt(format!(
                "cannot point at package without an active payload directory: {package_id}"
            )));
        }
        self.read_state(package_id)?;
        atomic_write(&self.active_pointer_path()?, package_id.as_bytes())
    }

    /// Clean up old packages beyond the retention limit.
    ///
    /// Keeps the `keep_count` most-recently-written packages (by manifest
    /// creation date).  Also removes associated staged and active
    /// directories.
    pub fn gc(&self, keep_count: usize) -> Result<(), UpdateError> {
        self.recover_interrupted_transaction()?;
        let mut packages = self.try_list_packages()?;
        let active_id = self.active_package_id()?;
        let activation_record = self.read_activation_record()?;

        // Sort by creation date (newest first).
        packages.sort_by(|a, b| b.manifest.created_at.cmp(&a.manifest.created_at));

        if packages.len() <= keep_count {
            return Ok(());
        }

        for pkg in &packages[keep_count..] {
            let id = pkg.package_id().to_string();
            let is_protected = active_id.as_deref() == Some(id.as_str())
                || activation_record.as_ref().is_some_and(|record| {
                    record.activated_id == id || record.previous_id.as_deref() == Some(id.as_str())
                });
            if is_protected {
                debug!("GC: retaining active or rollback package {id}");
                continue;
            }
            info!("GC: removing package {id}");

            // Remove metadata.
            let meta_dir = self.meta_dir(&id)?;
            if meta_dir.exists() {
                remove_dir_all_safe(&meta_dir, "package metadata GC")?;
            }

            // Remove staged.
            let staged = self.staged_dir(&id)?;
            if staged.exists() {
                remove_dir_all_safe(&staged, "staged package GC")?;
            }

            let active_dir = self.active_dir(&id)?;
            if active_dir.exists() {
                remove_dir_all_safe(&active_dir, "inactive immutable package GC")?;
            }

            // Remove previous.
            let previous = self.previous_dir(&id)?;
            if previous.exists() {
                remove_dir_all_safe(&previous, "previous package GC")?;
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
        std::fs::create_dir_all(cache.active_dir(pkg.package_id()).unwrap()).unwrap();
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

    #[test]
    fn cache_public_package_id_apis_reject_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();
        let victim = tmp.path().join("victim.txt");
        std::fs::write(&victim, b"keep").unwrap();

        assert!(matches!(
            cache.read_state("../victim.txt"),
            Err(UpdateError::UnsafePath { .. })
        ));
        assert!(cache.get_package("..\\victim.txt").is_none());
        assert!(matches!(
            cache.set_active_pointer("../victim.txt"),
            Err(UpdateError::UnsafePath { .. })
        ));
        assert_eq!(std::fs::read(victim).unwrap(), b"keep");
        assert!(!tmp.path().join("active_pointer.txt").exists());
    }

    #[test]
    fn cache_and_apply_runtime_have_no_unwrap_or_expect_calls() {
        for (name, source) in [
            ("cache", include_str!("cache.rs")),
            ("apply", include_str!("apply.rs")),
        ] {
            let runtime = source
                .split_once("\n#[cfg(test)]")
                .map_or(source, |(runtime, _)| runtime);
            assert!(
                !runtime.contains(".unwrap()"),
                "runtime unwrap found in {name}.rs"
            );
            assert!(
                !runtime.contains(".expect("),
                "runtime expect found in {name}.rs"
            );
        }
    }
}
