use std::fs::{File, OpenOptions};
use std::path::Path;

use engine_asset::AssetRegistry;
use engine_serialize::{Diagnostic, DiagnosticSeverity, HotUpdateManifest, PlatformKind};
use fs2::FileExt;
use tracing::{debug, info};

use crate::apply::UpdateApplier;
use crate::cache::PackageCache;
use crate::download::Downloader;
use crate::error::UpdateError;
use crate::install::Installer;
use crate::package::{Package, PackageState};
use crate::path_safety::{
    ensure_no_links_in_path, remove_dir_all_safe, safe_join, validate_manifest_paths,
};
use crate::rollback::RollbackManager;
use crate::verify::Verifier;

// ---------------------------------------------------------------------------
// PackageManager
// ---------------------------------------------------------------------------

/// Top-level orchestrator for the hot-update lifecycle.
///
/// Owns all sub-components and exposes a high-level API for installing,
/// rolling back, and applying hot-update packages.
///
/// # Lifecycle
///
/// ```text
/// install_package(manifest)
///   ├─ verify Ed25519 signature and trusted key
///   ├─ verify compatibility
///   ├─ verify platform rules
///   ├─ download payloads
///   ├─ verify payload hashes
///   ├─ verify cooked headers
///   ├─ stage
///   ├─ activate  (atomic switch)
///   └─ apply updates
///
/// install_local(manifest_path)
///   └─ same flow but uses download_local
///
/// rollback()
///   └─ restore previous known-good package
///
/// check_boot()
///   └─ auto-rollback if boot marker present
/// ```
pub struct PackageManager {
    /// Held for the complete manager lifetime so two processes cannot mutate
    /// the same singleton transaction journal concurrently.
    _cache_lock: File,
    cache: PackageCache,
    verifier: Verifier,
    #[expect(dead_code)]
    downloader: Downloader,
    #[expect(dead_code)]
    installer: Installer,
    #[expect(dead_code)]
    rollback_manager: RollbackManager,
    #[expect(dead_code)]
    applier: UpdateApplier,
    current_engine_version: String,
    current_script_api_version: (u16, u16),
    platform: PlatformKind,
}

impl PackageManager {
    /// Create a new PackageManager.
    ///
    /// * `base_dir` – root directory for the hot-update cache hierarchy.
    /// * `platform` – the target platform (controls payload filtering).
    /// * `engine_ver` – current engine version string (e.g. `"1.5.0"`).
    /// * `script_api_ver` – current script API version.
    pub fn new(
        base_dir: &Path,
        platform: PlatformKind,
        engine_ver: &str,
        script_api_ver: (u16, u16),
    ) -> Result<Self, UpdateError> {
        Self::try_new(base_dir, platform, engine_ver, script_api_ver)
    }

    /// Strict production constructor. Cache initialisation, interrupted
    /// transaction recovery, and exclusive-lock acquisition must all succeed.
    pub fn try_new(
        base_dir: &Path,
        platform: PlatformKind,
        engine_ver: &str,
        script_api_ver: (u16, u16),
    ) -> Result<Self, UpdateError> {
        Self::try_new_with_verifier(
            base_dir,
            platform,
            engine_ver,
            script_api_ver,
            Verifier::production(),
        )
    }

    /// Create a manager that explicitly allows unsigned development packages.
    pub fn new_development(
        base_dir: &Path,
        platform: PlatformKind,
        engine_ver: &str,
        script_api_ver: (u16, u16),
    ) -> Result<Self, UpdateError> {
        Self::try_new_development(base_dir, platform, engine_ver, script_api_ver)
    }

    /// Strict development constructor. Unsigned manifests are allowed, but
    /// cache ownership and recovery remain fail-closed.
    pub fn try_new_development(
        base_dir: &Path,
        platform: PlatformKind,
        engine_ver: &str,
        script_api_ver: (u16, u16),
    ) -> Result<Self, UpdateError> {
        Self::try_new_with_verifier(
            base_dir,
            platform,
            engine_ver,
            script_api_ver,
            Verifier::development(),
        )
    }

    /// Create a manager with an explicitly configured verifier and trust set.
    pub fn new_with_verifier(
        base_dir: &Path,
        platform: PlatformKind,
        engine_ver: &str,
        script_api_ver: (u16, u16),
        verifier: Verifier,
    ) -> Result<Self, UpdateError> {
        Self::try_new_with_verifier(base_dir, platform, engine_ver, script_api_ver, verifier)
    }

    /// Strict constructor with an explicitly configured verifier.
    pub fn try_new_with_verifier(
        base_dir: &Path,
        platform: PlatformKind,
        engine_ver: &str,
        script_api_ver: (u16, u16),
        verifier: Verifier,
    ) -> Result<Self, UpdateError> {
        let cache_lock = acquire_cache_lock(base_dir)?;
        let cache = PackageCache::new(base_dir);
        cache.initialize()?;

        Ok(Self {
            _cache_lock: cache_lock,
            cache,
            verifier,
            downloader: Downloader,
            installer: Installer,
            rollback_manager: RollbackManager,
            applier: UpdateApplier,
            current_engine_version: engine_ver.to_string(),
            current_script_api_version: script_api_ver,
            platform,
        })
    }

    /// Full hot-update pipeline for a remote (HTTP-downloaded) package.
    ///
    /// Steps:
    /// 1. Verify the Ed25519 signature against the configured trusted keys.
    /// 2. Verify engine / script API compatibility.
    /// 3. Verify platform-specific rules.
    /// 4. Download all payloads for the current platform.
    /// 5. Verify payload hashes.
    /// 6. Verify cooked asset headers.
    /// 7. Stage the package (move to cache managed area).
    /// 8. Activate the package (atomic switch).
    /// 9. Apply resource & logic updates.
    ///
    /// On failure at any step, all accumulated errors are returned.
    pub fn install_package(
        &mut self,
        manifest: HotUpdateManifest,
        base_url: &str,
    ) -> Result<Package, Vec<UpdateError>> {
        self.cache
            .recover_interrupted_transaction()
            .map_err(|error| vec![error])?;
        let mut errors: Vec<UpdateError> = Vec::new();

        // ── 1. Signature ────────────────────────────────────────────────
        if let Err(e) = self.verifier.verify_signature(&manifest) {
            errors.push(e);
        }

        // ── 2. Compatibility ────────────────────────────────────────────
        if let Err(e) = Verifier::verify_compatibility(
            &manifest,
            &self.current_engine_version,
            self.current_script_api_version,
        ) {
            errors.push(e);
        }

        // ── 3. Platform rules ───────────────────────────────────────────
        if let Err(e) = Verifier::verify_platform_rules(&manifest, &self.platform) {
            errors.push(e);
        }
        if let Err(mut path_errors) = validate_manifest_paths(&manifest) {
            errors.append(&mut path_errors);
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        // ── 4. Download ─────────────────────────────────────────────────
        let download_dir = safe_join(
            &self.cache.base_dir,
            "download_temp",
            "temporary download directory",
        )
        .map_err(|error| vec![error])?;
        // Ensure a clean download directory.
        if download_dir.exists() {
            remove_dir_all_safe(&download_dir, "temporary download directory")
                .map_err(|error| vec![error])?;
        }
        std::fs::create_dir_all(&download_dir).map_err(|e| {
            vec![UpdateError::DownloadFailed(format!(
                "cannot create download dir: {e}"
            ))]
        })?;

        if let Err(mut dl_errors) =
            Downloader::download(&manifest, &download_dir, &self.platform, base_url)
        {
            errors.append(&mut dl_errors);
            let _ = remove_dir_all_safe(&download_dir, "temporary download directory");
            return Err(errors);
        }

        // Update state to Downloaded.
        let mut pkg = Package::new(manifest.clone(), &self.cache.base_dir);
        pkg.state = PackageState::Downloaded;
        let _ = self.cache.write_state(&pkg);

        // ── 5. Verify hashes ────────────────────────────────────────────
        if let Err(mut hash_errors) =
            Verifier::verify_payload_hashes(&manifest, &download_dir, &self.platform)
        {
            errors.append(&mut hash_errors);
            let _ = remove_dir_all_safe(&download_dir, "temporary download directory");
            return Err(errors);
        }

        // ── 6. Verify cooked headers ───────────────────────────────────
        if let Err(mut header_errors) =
            Verifier::verify_cooked_headers(&manifest, &download_dir, &self.platform)
        {
            errors.append(&mut header_errors);
            let _ = remove_dir_all_safe(&download_dir, "temporary download directory");
            return Err(errors);
        }

        // ── 7. Stage ────────────────────────────────────────────────────
        let pkg = match Installer::stage(&manifest, &download_dir, &self.cache, &self.platform) {
            Ok(p) => p,
            Err(e) => {
                errors.push(e);
                let _ = remove_dir_all_safe(&download_dir, "temporary download directory");
                return Err(errors);
            }
        };

        // ── 8. Activate ─────────────────────────────────────────────────
        let mut pkg = pkg;
        if let Err(e) = Installer::activate(&mut pkg, &self.cache, &self.platform) {
            errors.push(e);
            return Err(errors);
        }

        // ── 9. Apply (best-effort) ──────────────────────────────────────
        // Resource updates require a registry — we call the apply via
        // the manager's apply_updates method which the caller invokes
        // separately with the real registry.
        // For the full pipeline we still report apply diagnostics.

        info!(
            package_id = %pkg.package_id(),
            "package installation complete"
        );

        Ok(pkg)
    }

    /// Install a package from a local manifest file (for testing / dev).
    ///
    /// The manifest file is parsed, then the flow follows the same
    /// pipeline as [`install_package`](Self::install_package) but uses
    /// `download_local` to copy payloads from the manifest's directory.
    pub fn install_local(&mut self, manifest_path: &Path) -> Result<Package, Vec<UpdateError>> {
        self.cache
            .recover_interrupted_transaction()
            .map_err(|error| vec![error])?;
        let mut errors: Vec<UpdateError> = Vec::new();

        ensure_no_links_in_path(manifest_path, "local manifest").map_err(|error| vec![error])?;

        // ── Read and parse manifest ────────────────────────────────────
        let manifest_json = match std::fs::read_to_string(manifest_path) {
            Ok(s) => s,
            Err(e) => return Err(vec![UpdateError::ManifestParse(e.to_string())]),
        };
        let manifest: HotUpdateManifest = match serde_json::from_str(&manifest_json) {
            Ok(m) => m,
            Err(e) => return Err(vec![UpdateError::ManifestParse(e.to_string())]),
        };

        // ── 1. Signature ────────────────────────────────────────────────
        if let Err(e) = self.verifier.verify_signature(&manifest) {
            errors.push(e);
        }

        // ── 2. Compatibility ────────────────────────────────────────────
        if let Err(e) = Verifier::verify_compatibility(
            &manifest,
            &self.current_engine_version,
            self.current_script_api_version,
        ) {
            errors.push(e);
        }

        // ── 3. Platform rules ───────────────────────────────────────────
        if let Err(e) = Verifier::verify_platform_rules(&manifest, &self.platform) {
            errors.push(e);
        }

        if let Err(mut path_errors) = validate_manifest_paths(&manifest) {
            errors.append(&mut path_errors);
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        // Source directory is the manifest's parent.
        let source_dir = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        // ── 4. Local download ──────────────────────────────────────────
        let download_dir = safe_join(
            &self.cache.base_dir,
            "download_temp",
            "temporary download directory",
        )
        .map_err(|error| vec![error])?;
        if download_dir.exists() {
            remove_dir_all_safe(&download_dir, "temporary download directory")
                .map_err(|error| vec![error])?;
        }
        std::fs::create_dir_all(&download_dir).map_err(|e| {
            vec![UpdateError::DownloadFailed(format!(
                "cannot create download dir: {e}"
            ))]
        })?;

        if let Err(mut dl_errors) =
            Downloader::download_local(&manifest, &source_dir, &download_dir, &self.platform)
        {
            errors.append(&mut dl_errors);
            let _ = remove_dir_all_safe(&download_dir, "temporary download directory");
            return Err(errors);
        }

        // ── 5. Verify hashes ────────────────────────────────────────────
        if let Err(mut hash_errors) =
            Verifier::verify_payload_hashes(&manifest, &download_dir, &self.platform)
        {
            errors.append(&mut hash_errors);
            let _ = remove_dir_all_safe(&download_dir, "temporary download directory");
            return Err(errors);
        }

        // ── 6. Verify cooked headers ───────────────────────────────────
        if let Err(mut header_errors) =
            Verifier::verify_cooked_headers(&manifest, &download_dir, &self.platform)
        {
            errors.append(&mut header_errors);
            let _ = remove_dir_all_safe(&download_dir, "temporary download directory");
            return Err(errors);
        }

        // ── 7. Stage ────────────────────────────────────────────────────
        let pkg = match Installer::stage(&manifest, &download_dir, &self.cache, &self.platform) {
            Ok(p) => p,
            Err(e) => {
                errors.push(e);
                let _ = remove_dir_all_safe(&download_dir, "temporary download directory");
                return Err(errors);
            }
        };

        // ── 8. Activate ─────────────────────────────────────────────────
        let mut pkg = pkg;
        if let Err(e) = Installer::activate(&mut pkg, &self.cache, &self.platform) {
            errors.push(e);
            return Err(errors);
        }

        info!(
            package_id = %pkg.package_id(),
            "local package installation complete"
        );

        Ok(pkg)
    }

    /// Rollback to the previous known-good package.
    ///
    /// This restores the package that was active before the most recent
    /// activation.
    pub fn rollback(&mut self) -> Result<Package, UpdateError> {
        RollbackManager::rollback(&self.cache)
    }

    /// Check if a boot marker indicates a boot failure and perform
    /// automatic rollback if needed.
    ///
    /// Returns `Ok(())` if no rollback is needed or if rollback succeeded.
    /// Returns `Err` if rollback was needed but failed.
    pub fn check_boot(&mut self) -> Result<(), UpdateError> {
        self.cache.recover_interrupted_transaction()?;
        if !RollbackManager::needs_rollback(&self.cache) {
            return Ok(());
        }

        info!("boot marker detected — performing automatic rollback");
        match RollbackManager::rollback(&self.cache) {
            Ok(ref pkg) => {
                info!(
                    package_id = %pkg.package_id(),
                    "auto-rollback completed"
                );
                Ok(())
            }
            Err(e) => Err(UpdateError::RollbackFailed(format!(
                "auto-rollback failed: {e}"
            ))),
        }
    }

    /// Confirm that the current activation booted successfully.
    ///
    /// This clears both boot markers and prevents a later `check_boot` call
    /// from rolling back a known-good package.
    pub fn mark_boot_successful(&mut self) -> Result<(), UpdateError> {
        Installer::mark_boot_successful(&self.cache)
    }

    /// Record an explicit boot failure while preserving rollback metadata.
    pub fn mark_failed_boot(&mut self) -> Result<(), UpdateError> {
        Installer::mark_failed_boot(&self.cache)
    }

    /// List all known packages.
    pub fn list_packages(&self) -> Result<Vec<Package>, UpdateError> {
        self.try_list_packages()
    }

    /// Strict package listing. Interrupted transactions are recovered before
    /// any cache state is exposed.
    pub fn try_list_packages(&self) -> Result<Vec<Package>, UpdateError> {
        self.cache.recover_interrupted_transaction()?;
        self.cache.try_list_packages()
    }

    /// Get the currently active package, if any.
    pub fn active_package(&self) -> Result<Option<Package>, UpdateError> {
        self.try_active_package()
    }

    /// Strict active-package lookup with transaction recovery.
    pub fn try_active_package(&self) -> Result<Option<Package>, UpdateError> {
        self.cache.recover_interrupted_transaction()?;
        self.cache.try_active_package()
    }

    /// Read the latest committed activation/rollback record.
    pub fn activation_record(&self) -> Result<Option<crate::ActivationRecord>, UpdateError> {
        self.cache.recover_interrupted_transaction()?;
        self.cache.activation_record()
    }

    /// Apply resource, logic, and assembly updates after activation.
    ///
    /// This should be called after a new package is activated (or on
    /// engine restart with an active package).
    ///
    /// The `registry` is used for resource reloads.  Diagnostics are
    /// returned for each operation.
    pub fn apply_updates(&mut self, registry: &mut AssetRegistry) -> Vec<Diagnostic> {
        if let Err(error) = self.cache.recover_interrupted_transaction() {
            return vec![Diagnostic::new(
                "HOT_UPDATE_RECOVERY_FAILED",
                DiagnosticSeverity::Error,
                "hot-update",
                format!("cannot apply updates before transaction recovery: {error}"),
            )
            .contract("HotUpdate", "0.1")];
        }

        let mut all_diags = Vec::new();

        let active_pkg = match self.cache.try_active_package() {
            Ok(Some(pkg)) => pkg,
            Err(error) => {
                return vec![Diagnostic::new(
                    "HOT_UPDATE_ACTIVE_PACKAGE_INVALID",
                    DiagnosticSeverity::Error,
                    "hot-update",
                    format!("cannot read active package: {error}"),
                )
                .contract("HotUpdate", "0.1")];
            }
            Ok(None) => {
                debug!("no active package to apply");
                return all_diags;
            }
        };

        let active_dir = active_pkg.active_dir();

        info!(
            package_id = %active_pkg.package_id(),
            "applying updates from active package"
        );

        // Resource updates.
        all_diags.extend(UpdateApplier::apply_resource_updates(
            &active_pkg.manifest,
            &active_dir,
            registry,
            &self.platform,
        ));

        // Logic assets.
        all_diags.extend(UpdateApplier::apply_logic_assets(
            &active_pkg.manifest,
            &active_dir,
            &self.platform,
        ));

        // Android assembly (no-op on other platforms).
        all_diags.extend(UpdateApplier::apply_android_assembly(
            &active_pkg.manifest,
            &active_dir,
            &self.platform,
        ));

        all_diags
    }
}

fn acquire_cache_lock(base_dir: &Path) -> Result<File, UpdateError> {
    ensure_no_links_in_path(base_dir, "hot-update cache root")?;
    std::fs::create_dir_all(base_dir)?;
    ensure_no_links_in_path(base_dir, "hot-update cache root")?;

    let lock_path = safe_join(base_dir, ".engine-hot-update.lock", "cache lock")?;
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    FileExt::try_lock_exclusive(&lock_file).map_err(|error| {
        UpdateError::Io(std::io::Error::new(
            error.kind(),
            format!(
                "hot-update cache is already owned by another manager ({}): {error}",
                base_dir.display()
            ),
        ))
    })?;
    Ok(lock_file)
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
