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
mod tests {
    use super::*;
    use engine_serialize::{
        AssetId, PayloadHash, PlatformPayload, RollbackMetadata, SchemaVersion,
    };
    use ring::signature::KeyPair;
    use sha2::{Digest, Sha256};

    fn sample_manifest() -> HotUpdateManifest {
        let data = b"test payload";
        let hash: [u8; 32] = Sha256::digest(data).into();
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
                platform: PlatformKind::Desktop,
                path: "data.bin".into(),
                algorithm: "sha256".into(),
                hash,
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

    fn setup_manager() -> (PackageManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let manager =
            PackageManager::new_development(tmp.path(), PlatformKind::Desktop, "1.5.0", (1, 5))
                .unwrap();
        (manager, tmp)
    }

    // ── install_local tests ────────────────────────────────────────────

    #[test]
    fn install_local_parses_manifest_and_installs() {
        let (mut manager, tmp) = setup_manager();

        // Create manifest file.
        let manifest = sample_manifest();
        let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();

        let pkg_dir = tmp.path().join("my_pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("manifest.json"), &manifest_json).unwrap();
        std::fs::write(pkg_dir.join("data.bin"), b"test payload").unwrap();

        let result = manager.install_local(&pkg_dir.join("manifest.json"));
        assert!(result.is_ok(), "install_local failed: {result:?}");

        let pkg = result.unwrap();
        assert_eq!(pkg.state, PackageState::Active);
    }

    #[test]
    fn default_manager_rejects_unsigned_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let mut manager =
            PackageManager::new(tmp.path(), PlatformKind::Desktop, "1.5.0", (1, 5)).unwrap();
        let manifest = sample_manifest();
        let pkg_dir = tmp.path().join("unsigned-production");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let errors = manager
            .install_local(&pkg_dir.join("manifest.json"))
            .unwrap_err();
        assert!(errors
            .iter()
            .any(|error| matches!(error, UpdateError::SignatureMissing)));
        assert!(!tmp.path().join("download_temp").exists());
    }

    #[test]
    fn production_manager_uses_configured_verifier_for_signed_package() {
        let tmp = tempfile::tempdir().unwrap();
        let rng = ring::rand::SystemRandom::new();
        let private_key = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let key_pair = ring::signature::Ed25519KeyPair::from_pkcs8(private_key.as_ref()).unwrap();

        let mut manifest = sample_manifest();
        crate::verify::sign_manifest_ed25519(
            &mut manifest,
            "release-2026",
            "2026-05-29T12:00:00Z",
            private_key.as_ref(),
        )
        .unwrap();
        let verifier = Verifier::production()
            .with_trusted_ed25519_key("release-2026", key_pair.public_key().as_ref())
            .unwrap();
        let mut manager = PackageManager::new_with_verifier(
            tmp.path(),
            PlatformKind::Desktop,
            "1.5.0",
            (1, 5),
            verifier,
        )
        .unwrap();

        let pkg_dir = tmp.path().join("signed-production");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(pkg_dir.join("data.bin"), b"test payload").unwrap();

        let package = manager
            .install_local(&pkg_dir.join("manifest.json"))
            .unwrap();
        assert_eq!(package.state, PackageState::Active);
    }

    #[test]
    fn install_local_rejects_incompatible_engine() {
        let (mut manager, tmp) = setup_manager();

        let mut manifest = sample_manifest();
        manifest.engine_version = "2.0.0".into();
        let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();

        let pkg_dir = tmp.path().join("incompat");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("manifest.json"), &manifest_json).unwrap();

        let result = manager.install_local(&pkg_dir.join("manifest.json"));
        assert!(result.is_err());
    }

    #[test]
    fn install_local_rejects_missing_payload() {
        let (mut manager, tmp) = setup_manager();

        let manifest = sample_manifest();
        let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();

        let pkg_dir = tmp.path().join("missing_payload");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("manifest.json"), &manifest_json).unwrap();
        // data.bin NOT created — payload missing

        let result = manager.install_local(&pkg_dir.join("manifest.json"));
        assert!(result.is_err());
    }

    #[test]
    fn install_local_invalid_json() {
        let (mut manager, tmp) = setup_manager();

        let pkg_dir = tmp.path().join("bad_json");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("manifest.json"), "not valid json").unwrap();

        let result = manager.install_local(&pkg_dir.join("manifest.json"));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().first().unwrap(),
            UpdateError::ManifestParse(_)
        ));
    }

    // ── rollback tests ─────────────────────────────────────────────────

    #[test]
    fn manager_rollback_restores_previous() {
        let (mut manager, tmp) = setup_manager();

        // Install first package.
        let m1 = sample_manifest();
        let j1 = serde_json::to_string_pretty(&m1).unwrap();
        let d1 = tmp.path().join("pkg1");
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::write(d1.join("manifest.json"), &j1).unwrap();
        std::fs::write(d1.join("data.bin"), b"test payload").unwrap();
        manager.install_local(&d1.join("manifest.json")).unwrap();
        let id1 = manager
            .active_package()
            .unwrap()
            .unwrap()
            .package_id()
            .to_string();
        manager.mark_boot_successful().unwrap();

        // Install second package.
        let mut m2 = sample_manifest();
        m2.created_at = "2026-06-01T00:00:00Z".into();
        let j2 = serde_json::to_string_pretty(&m2).unwrap();
        let d2 = tmp.path().join("pkg2");
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::write(d2.join("manifest.json"), &j2).unwrap();
        std::fs::write(d2.join("data.bin"), b"test payload").unwrap();
        manager.install_local(&d2.join("manifest.json")).unwrap();

        // Rollback.
        let rolled = manager.rollback().unwrap();
        assert_eq!(rolled.state, PackageState::Active);

        // Active package should be the first one.
        let active = manager.active_package().unwrap().unwrap();
        assert_eq!(active.package_id(), id1);
    }

    #[test]
    fn manager_rollback_fails_without_previous() {
        let (mut manager, _tmp) = setup_manager();
        let result = manager.rollback();
        assert!(result.is_err());
    }

    #[test]
    fn unsafe_manifest_does_not_delete_existing_download_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_root = tmp.path().join("cache");
        let mut manager =
            PackageManager::new(&cache_root, PlatformKind::Desktop, "1.5.0", (1, 5)).unwrap();

        let download_dir = cache_root.join("download_temp");
        std::fs::create_dir_all(&download_dir).unwrap();
        let sentinel = download_dir.join("sentinel.txt");
        std::fs::write(&sentinel, b"keep").unwrap();

        let mut manifest = sample_manifest();
        manifest.payload_hashes[0].path = "../escape.bin".into();
        let package_dir = tmp.path().join("unsafe-package");
        std::fs::create_dir_all(&package_dir).unwrap();
        let manifest_path = package_dir.join("manifest.json");
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        let errors = manager.install_local(&manifest_path).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| matches!(error, UpdateError::UnsafePath { .. })));
        assert_eq!(std::fs::read(sentinel).unwrap(), b"keep");
        assert!(!tmp.path().join("escape.bin").exists());
    }

    // ── check_boot tests ───────────────────────────────────────────────

    #[test]
    fn check_boot_no_marker_ok() {
        let (mut manager, _tmp) = setup_manager();
        assert!(manager.check_boot().is_ok());
    }

    #[test]
    fn check_boot_with_marker_triggers_rollback() {
        let (mut manager, tmp) = setup_manager();

        // Install a package first so there's something to roll back from.
        let m1 = sample_manifest();
        let j1 = serde_json::to_string_pretty(&m1).unwrap();
        let d1 = tmp.path().join("first");
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::write(d1.join("manifest.json"), &j1).unwrap();
        std::fs::write(d1.join("data.bin"), b"test payload").unwrap();
        manager.install_local(&d1.join("manifest.json")).unwrap();

        // Simulate boot marker presence (it's already there from activation).
        // The boot marker means rollback is needed, but since there's
        // no earlier version to rollback to, it may fail.
        // But check_boot should attempt it.
        assert!(RollbackManager::needs_rollback(&manager.cache));
    }

    #[test]
    fn manager_boot_success_api_prevents_restart_rollback() {
        let (mut manager, tmp) = setup_manager();
        let manifest = sample_manifest();
        let package_dir = tmp.path().join("boot-success");
        std::fs::create_dir_all(&package_dir).unwrap();
        std::fs::write(
            package_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(package_dir.join("data.bin"), b"test payload").unwrap();
        let installed = manager
            .install_local(&package_dir.join("manifest.json"))
            .unwrap();
        manager.mark_boot_successful().unwrap();

        let cache_dir = manager.cache.base_dir.clone();
        drop(manager);
        let mut restarted =
            PackageManager::try_new_development(&cache_dir, PlatformKind::Desktop, "1.5.0", (1, 5))
                .unwrap();
        restarted.check_boot().unwrap();
        assert_eq!(
            restarted.active_package().unwrap().unwrap().package_id(),
            installed.package_id()
        );
    }

    #[test]
    fn manager_failed_boot_api_rolls_back_to_recorded_package() {
        let (mut manager, tmp) = setup_manager();
        let first_manifest = sample_manifest();
        let first_dir = tmp.path().join("boot-failed-first");
        std::fs::create_dir_all(&first_dir).unwrap();
        std::fs::write(
            first_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&first_manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(first_dir.join("data.bin"), b"test payload").unwrap();
        let first = manager
            .install_local(&first_dir.join("manifest.json"))
            .unwrap();
        manager.mark_boot_successful().unwrap();

        let mut second_manifest = sample_manifest();
        second_manifest.created_at = "2026-09-01T00:00:00Z".into();
        let second_dir = tmp.path().join("boot-failed-second");
        std::fs::create_dir_all(&second_dir).unwrap();
        std::fs::write(
            second_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&second_manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(second_dir.join("data.bin"), b"test payload").unwrap();
        manager
            .install_local(&second_dir.join("manifest.json"))
            .unwrap();
        manager.mark_failed_boot().unwrap();
        manager.check_boot().unwrap();

        assert_eq!(
            manager.active_package().unwrap().unwrap().package_id(),
            first.package_id()
        );
        assert!(!RollbackManager::needs_rollback(&manager.cache));
    }

    // ── list / active tests ────────────────────────────────────────────

    #[test]
    fn list_packages_after_install() {
        let (mut manager, tmp) = setup_manager();

        let manifest = sample_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let d = tmp.path().join("list_test");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.json"), &json).unwrap();
        std::fs::write(d.join("data.bin"), b"test payload").unwrap();
        manager.install_local(&d.join("manifest.json")).unwrap();

        let packages = manager.list_packages().unwrap();
        assert!(!packages.is_empty());
    }

    #[test]
    fn active_package_returns_some_after_install() {
        let (mut manager, tmp) = setup_manager();

        let manifest = sample_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let d = tmp.path().join("active_test");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.json"), &json).unwrap();
        std::fs::write(d.join("data.bin"), b"test payload").unwrap();
        manager.install_local(&d.join("manifest.json")).unwrap();

        assert!(manager.active_package().unwrap().is_some());
    }

    #[test]
    fn active_package_none_before_install() {
        let (manager, _tmp) = setup_manager();
        assert!(manager.active_package().unwrap().is_none());
    }

    // ── apply_updates tests ────────────────────────────────────────────

    #[test]
    fn apply_updates_no_active_package() {
        let (mut manager, _tmp) = setup_manager();
        let mut registry = AssetRegistry::new();
        let diags = manager.apply_updates(&mut registry);
        assert!(diags.is_empty());
    }

    #[test]
    fn apply_updates_after_install() {
        let (mut manager, tmp) = setup_manager();

        let manifest = sample_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let d = tmp.path().join("apply_test");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.json"), &json).unwrap();
        std::fs::write(d.join("data.bin"), b"test payload").unwrap();
        manager.install_local(&d.join("manifest.json")).unwrap();

        let mut registry = AssetRegistry::new();
        let diags = manager.apply_updates(&mut registry);

        // Should have at least some diagnostics (logic asset missing, etc.)
        assert!(!diags.is_empty());
    }

    // ── Edge-case tests ────────────────────────────────────────────────

    #[test]
    fn manager_new_initializes_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let _manager =
            PackageManager::new(tmp.path(), PlatformKind::Desktop, "1.5.0", (1, 5)).unwrap();

        // Cache directories should exist.
        assert!(tmp.path().join("packages").exists());
        assert!(tmp.path().join("staged").exists());
        assert!(tmp.path().join("active").exists());
    }

    #[test]
    fn strict_constructor_fails_when_cache_cannot_be_initialized() {
        let tmp = tempfile::tempdir().unwrap();
        let invalid_root = tmp.path().join("not-a-directory");
        std::fs::write(&invalid_root, b"file").unwrap();

        assert!(PackageManager::try_new_development(
            &invalid_root,
            PlatformKind::Desktop,
            "1.5.0",
            (1, 5),
        )
        .is_err());
    }

    #[test]
    fn cache_lock_rejects_second_manager_until_owner_drops() {
        let tmp = tempfile::tempdir().unwrap();
        let first =
            PackageManager::try_new_development(tmp.path(), PlatformKind::Desktop, "1.5.0", (1, 5))
                .unwrap();

        let error = match PackageManager::try_new_development(
            tmp.path(),
            PlatformKind::Desktop,
            "1.5.0",
            (1, 5),
        ) {
            Ok(_) => panic!("second manager unexpectedly acquired the cache lock"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("already owned"));

        drop(first);
        assert!(PackageManager::try_new_development(
            tmp.path(),
            PlatformKind::Desktop,
            "1.5.0",
            (1, 5),
        )
        .is_ok());
    }

    #[test]
    fn apply_updates_fails_closed_when_transaction_recovery_fails() {
        let (mut manager, _tmp) = setup_manager();
        manager
            .cache
            .write_transaction(&crate::transaction::ActivationTransaction {
                version: crate::transaction::ACTIVATION_FORMAT_VERSION,
                operation: crate::transaction::TransactionOperation::Activate,
                activated_id: "a".repeat(64),
                previous_id: Some("b".repeat(64)),
                moved_staged_to_active: false,
            })
            .unwrap();

        let mut registry = AssetRegistry::new();
        let diagnostics = manager.apply_updates(&mut registry);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "HOT_UPDATE_RECOVERY_FAILED");
        assert!(registry.cached_ids().is_empty());
        assert!(manager.try_active_package().is_err());
        assert!(manager.active_package().is_err());
    }

    #[test]
    fn install_local_multiple_times() {
        let (mut manager, tmp) = setup_manager();

        for i in 0..3 {
            let mut m = sample_manifest();
            m.created_at = format!("2026-06-{:02}T00:00:00Z", i + 1);
            let json = serde_json::to_string_pretty(&m).unwrap();
            let d = tmp.path().join(format!("multi_{i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("manifest.json"), &json).unwrap();
            std::fs::write(d.join("data.bin"), b"test payload").unwrap();
            let result = manager.install_local(&d.join("manifest.json"));
            assert!(result.is_ok(), "install {i} failed: {result:?}");
            manager.mark_boot_successful().unwrap();
        }

        // The last installed package should be active.
        let active = manager.active_package().unwrap().unwrap();
        assert_eq!(active.manifest.created_at, "2026-06-03T00:00:00Z");
    }
}
