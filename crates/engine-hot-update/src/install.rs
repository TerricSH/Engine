use std::path::Path;

use engine_serialize::{HotUpdateManifest, PlatformKind};
use tracing::{debug, info, warn};

use crate::atomic_fs::atomic_write;
use crate::atomic_fs::durable_rename_directory;
use crate::cache::PackageCache;
use crate::error::UpdateError;
use crate::package::{Package, PackageState};
use crate::path_safety::{
    ensure_tree_has_no_links, remove_dir_all_safe, safe_join, safe_package_path,
    validate_manifest_paths_once, validate_package_id,
};
use crate::transaction::{
    ActivationPhase, ActivationRecord, ActivationTransaction, TransactionOperation,
    ACTIVATION_FORMAT_VERSION,
};
use crate::verify::Verifier;

// ---------------------------------------------------------------------------
// Installer
// ---------------------------------------------------------------------------

/// Handles staging and atomic activation of verified packages.
///
/// Activated payload directories are immutable and retained. The only commit
/// point is an atomic replacement of `active_pointer.txt`; a durable journal
/// makes crashes immediately before and after that replacement recoverable.
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
        platform: &PlatformKind,
    ) -> Result<Package, UpdateError> {
        // Complete validation happens before an existing staged directory is
        // removed. A malicious manifest therefore cannot mutate the cache.
        validate_manifest_paths_once(manifest)?;
        ensure_tree_has_no_links(staging_dir, "download staging tree")?;

        let mut pkg = Package::new(manifest.clone(), &cache.base_dir);
        validate_package_id(pkg.package_id())?;
        let staged_dest = safe_package_path(&cache.base_dir, "staged", pkg.package_id())?;

        // Preflight every selected source and destination before replacing an
        // existing stage. Files for other platforms may be present in the
        // input tree, but are deliberately never copied into the package.
        let selected = manifest.payload_hashes_for_platform(*platform);
        let mut prepared = Vec::with_capacity(selected.len());
        for payload in selected {
            let source = safe_join(staging_dir, &payload.path, "selected staged payload")?;
            let destination = safe_join(&staged_dest, &payload.path, "staged package payload")?;
            let metadata = std::fs::symlink_metadata(&source)?;
            if crate::path_safety::is_link_or_reparse(&metadata) || !metadata.is_file() {
                return Err(UpdateError::UnsafePath {
                    field: "selected staged payload".into(),
                    path: source.display().to_string(),
                    reason: "payload must be a regular file and not a link or reparse point".into(),
                });
            }
            prepared.push((source, destination));
        }

        // Remove any existing staged directory.
        if staged_dest.exists() {
            debug!("removing existing staged directory: {:?}", staged_dest);
            remove_dir_all_safe(&staged_dest, "existing staged package")?;
        }

        std::fs::create_dir_all(&staged_dest)?;
        for (source, destination) in prepared {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if let Err(error) = std::fs::copy(&source, &destination) {
                let _ = remove_dir_all_safe(&staged_dest, "partial staged package");
                return Err(UpdateError::Io(error));
            }
        }
        let _ = remove_dir_all_safe(staging_dir, "download staging tree");

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
    pub fn activate(
        package: &mut Package,
        cache: &PackageCache,
        platform: &PlatformKind,
    ) -> Result<(), UpdateError> {
        Self::activate_inner(package, cache, platform, None)
    }

    fn activate_inner(
        package: &mut Package,
        cache: &PackageCache,
        platform: &PlatformKind,
        fail_at: Option<ActivationFailPoint>,
    ) -> Result<(), UpdateError> {
        cache.recover_interrupted_transaction()?;
        validate_manifest_paths_once(&package.manifest)?;
        let pkg_id = package.package_id().to_string();
        validate_package_id(&pkg_id)?;
        let staged_dir = cache.staged_dir(&pkg_id)?;
        let active_dir = cache.active_dir(&pkg_id)?;

        if !staged_dir.exists() {
            return Err(UpdateError::ActivationFailed(format!(
                "staged directory not found: {:?}",
                staged_dir
            )));
        }
        ensure_tree_has_no_links(&staged_dir, "staged package")?;
        validate_activation_payloads(&package.manifest, &staged_dir, platform)?;

        let previous_id = cache.active_package_id()?;
        if previous_id.as_deref() == Some(pkg_id.as_str()) {
            return Err(UpdateError::ActivationFailed(format!(
                "package {pkg_id} is already active"
            )));
        }
        if let Some(previous_id) = &previous_id {
            let previous_active = cache.active_dir(previous_id)?;
            if !previous_active.is_dir() {
                return Err(UpdateError::CacheCorrupt(format!(
                    "active pointer {previous_id} has no immutable payload directory"
                )));
            }
            ensure_tree_has_no_links(&previous_active, "current active package")?;
            cache.read_state(previous_id)?;
            ensure_previous_is_known_good(cache, previous_id)?;
        }

        let move_staged = !active_dir.exists();
        if !move_staged {
            ensure_tree_has_no_links(&active_dir, "existing immutable active package")?;
            validate_activation_payloads(&package.manifest, &active_dir, platform)?;
            Verifier::verify_payload_hashes(&package.manifest, &active_dir, platform).map_err(
                |errors| {
                    UpdateError::ActivationFailed(format!(
                        "existing immutable active payload failed verification: {}",
                        errors
                            .into_iter()
                            .map(|error| error.to_string())
                            .collect::<Vec<_>>()
                            .join("; ")
                    ))
                },
            )?;
        }

        let transaction = ActivationTransaction {
            version: ACTIVATION_FORMAT_VERSION,
            operation: TransactionOperation::Activate,
            activated_id: pkg_id.clone(),
            previous_id: previous_id.clone(),
            moved_staged_to_active: move_staged,
        };
        cache.write_transaction(&transaction)?;
        fail_activation_if_requested(
            cache,
            &transaction,
            fail_at,
            ActivationFailPoint::AfterJournal,
        )?;

        if move_staged {
            if let Err(error) = durable_rename_directory(&staged_dir, &active_dir) {
                return abort_activation(
                    cache,
                    &transaction,
                    UpdateError::ActivationFailed(format!(
                        "failed to prepare immutable active payload: {error}"
                    )),
                );
            }
        }
        fail_activation_if_requested(
            cache,
            &transaction,
            fail_at,
            ActivationFailPoint::AfterPayloadPrepared,
        )?;

        package.state = PackageState::Active;
        package.active_path = active_dir.clone();
        package.staged_path = staged_dir.clone();
        if let Err(error) = cache.write_state(package) {
            let result = abort_activation(cache, &transaction, error);
            restore_package_view(package, &staged_dir);
            return result;
        }
        if let Err(error) = fail_activation_if_requested(
            cache,
            &transaction,
            fail_at,
            ActivationFailPoint::AfterStatePrepared,
        ) {
            restore_package_view(package, &staged_dir);
            return Err(error);
        }

        let pending_record =
            ActivationRecord::new(pkg_id.clone(), previous_id, ActivationPhase::BootPending);
        if let Err(error) = cache.write_boot_marker(&pending_record) {
            let result = abort_activation(cache, &transaction, error);
            restore_package_view(package, &staged_dir);
            return result;
        }
        if let Err(error) = fail_activation_if_requested(
            cache,
            &transaction,
            fail_at,
            ActivationFailPoint::AfterBootMarker,
        ) {
            restore_package_view(package, &staged_dir);
            return Err(error);
        }
        let pointer_result = if fail_at == Some(ActivationFailPoint::PointerReplaceFailure) {
            Err(UpdateError::Io(std::io::Error::other(
                "injected atomic pointer replacement failure",
            )))
        } else {
            cache.set_active_pointer(&pkg_id)
        };
        if let Err(error) = pointer_result {
            if cache.active_package_id()?.as_deref() != Some(pkg_id.as_str()) {
                let result = abort_activation(
                    cache,
                    &transaction,
                    UpdateError::ActivationFailed(format!(
                        "atomic pointer replacement failed: {error}"
                    )),
                );
                restore_package_view(package, &staged_dir);
                return result;
            }
        }

        // The pointer now names the new package. Never turn a committed
        // activation into a reported failure: recovery can finish metadata.
        if let Err(error) = cache.finish_committed_activation(&transaction) {
            warn!("activation committed; metadata cleanup deferred: {error}");
        }
        if !move_staged {
            let _ = remove_dir_all_safe(&staged_dir, "redundant staged package");
        }

        package.state = PackageState::Active;
        package.active_path = active_dir;
        package.staged_path = staged_dir;

        info!(package_id = %pkg_id, "package activated");
        Ok(())
    }

    /// Mark the current active package as having failed boot.
    ///
    /// The pending marker and versioned activation record are retained so a
    /// later [`RollbackManager`](crate::rollback::RollbackManager) call has
    /// an exact, deterministic target.
    pub fn mark_failed_boot(cache: &PackageCache) -> Result<(), UpdateError> {
        cache.recover_interrupted_transaction()?;
        let active_id = cache.active_package_id()?.ok_or_else(|| {
            UpdateError::CacheCorrupt("cannot mark boot failure without an active package".into())
        })?;
        let mut record = cache.read_activation_record()?.ok_or_else(|| {
            UpdateError::CacheCorrupt("active package has no activation record".into())
        })?;
        if record.activated_id != active_id || record.phase == ActivationPhase::RolledBack {
            return Err(UpdateError::CacheCorrupt(
                "activation record does not describe the active package".into(),
            ));
        }
        record.phase = ActivationPhase::BootFailed;
        cache.write_activation_record(&record)?;
        cache.write_boot_marker(&record)?;
        atomic_write(&cache.boot_failed_path()?, b"failed")?;
        info!(package_id = %active_id, "active package marked as failed boot");
        Ok(())
    }

    /// Confirm that the active package booted successfully.
    pub fn mark_boot_successful(cache: &PackageCache) -> Result<(), UpdateError> {
        cache.recover_interrupted_transaction()?;
        let active_id = cache.active_package_id()?.ok_or_else(|| {
            UpdateError::CacheCorrupt("cannot confirm boot without an active package".into())
        })?;
        let mut record = cache.read_activation_record()?.ok_or_else(|| {
            UpdateError::CacheCorrupt("active package has no activation record".into())
        })?;
        if record.activated_id != active_id || record.phase == ActivationPhase::RolledBack {
            return Err(UpdateError::CacheCorrupt(
                "activation record does not describe the active package".into(),
            ));
        }
        record.phase = ActivationPhase::BootSuccessful;
        cache.write_activation_record(&record)?;
        cache.clear_boot_markers()?;
        info!(package_id = %active_id, "active package boot confirmed");
        Ok(())
    }
}

fn validate_activation_payloads(
    manifest: &HotUpdateManifest,
    root: &Path,
    platform: &PlatformKind,
) -> Result<(), UpdateError> {
    for payload in &manifest.payload_hashes {
        let path = safe_join(root, &payload.path, "activation payload")?;
        if payload.platform.applies_to(*platform) {
            if !path.is_file() {
                return Err(UpdateError::ActivationFailed(format!(
                    "selected payload is missing: {}",
                    payload.path
                )));
            }
        } else if path.exists() {
            return Err(UpdateError::ActivationFailed(format!(
                "payload for {:?} must not be activated on {:?}: {}",
                payload.platform, platform, payload.path
            )));
        }
    }
    Ok(())
}

fn ensure_previous_is_known_good(
    cache: &PackageCache,
    previous_id: &str,
) -> Result<(), UpdateError> {
    let record = cache.read_activation_record()?.ok_or_else(|| {
        UpdateError::CacheCorrupt(format!(
            "active package {previous_id} has no activation record"
        ))
    })?;

    let describes_confirmed_active = match record.phase {
        ActivationPhase::BootSuccessful => record.activated_id == previous_id,
        // A rollback target was required to be known-good when the failed
        // package was activated, so it remains a valid base after rollback.
        ActivationPhase::RolledBack => record.previous_id.as_deref() == Some(previous_id),
        ActivationPhase::BootPending | ActivationPhase::BootFailed => false,
    };
    if describes_confirmed_active {
        return Ok(());
    }

    match record.phase {
        ActivationPhase::BootPending | ActivationPhase::BootFailed => {
            Err(UpdateError::ActivationFailed(format!(
                "active package {previous_id} has not completed a successful boot ({:?})",
                record.phase
            )))
        }
        ActivationPhase::BootSuccessful | ActivationPhase::RolledBack => {
            Err(UpdateError::CacheCorrupt(format!(
                "activation record does not describe active package {previous_id}"
            )))
        }
    }
}

fn restore_package_view(package: &mut Package, staged_dir: &Path) {
    package.state = PackageState::Staged;
    package.staged_path = staged_dir.to_path_buf();
    package.active_path = package.active_dir();
}

fn abort_activation<T>(
    cache: &PackageCache,
    transaction: &ActivationTransaction,
    error: UpdateError,
) -> Result<T, UpdateError> {
    match cache.recover_interrupted_transaction() {
        Ok(()) => Err(error),
        Err(recovery_error) => Err(UpdateError::ActivationFailed(format!(
            "{error}; recovery also failed: {recovery_error}; transaction for {} remains on disk",
            transaction.activated_id
        ))),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivationFailPoint {
    AfterJournal,
    AfterPayloadPrepared,
    AfterStatePrepared,
    AfterBootMarker,
    PointerReplaceFailure,
}

fn fail_activation_if_requested(
    cache: &PackageCache,
    transaction: &ActivationTransaction,
    fail_at: Option<ActivationFailPoint>,
    point: ActivationFailPoint,
) -> Result<(), UpdateError> {
    if fail_at == Some(point) {
        return abort_activation(
            cache,
            transaction,
            UpdateError::ActivationFailed(format!("injected failure at {point:?}")),
        );
    }
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Recursively copy a directory.
#[cfg(all(test, unix))]
fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), UpdateError> {
    ensure_tree_has_no_links(src, "copy source tree")?;
    crate::path_safety::ensure_no_links_in_path(dst, "copy destination tree")?;
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        let name = entry.file_name();
        let src_path = entry.path();
        let name = name.to_str().ok_or_else(|| UpdateError::UnsafePath {
            field: "copy source tree".into(),
            path: src_path.display().to_string(),
            reason: "non-UTF-8 file names are forbidden".into(),
        })?;
        let dst_path = safe_join(dst, name, "copy destination tree")?;

        if metadata.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else if metadata.is_file() {
            std::fs::copy(&src_path, &dst_path)?;
        } else {
            return Err(UpdateError::UnsafePath {
                field: "copy source tree".into(),
                path: src_path.display().to_string(),
                reason: "non-regular files are forbidden".into(),
            });
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
                platform: PlatformKind::Desktop,
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

        let pkg =
            Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();
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

        let pkg =
            Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();

        // Verify state was persisted.
        let loaded = cache.get_package(pkg.package_id()).unwrap();
        assert_eq!(loaded.state, PackageState::Staged);
    }

    #[test]
    fn installer_stage_only_contains_current_platform_and_all_payloads() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![
            PayloadHash {
                platform: PlatformKind::Desktop,
                path: "desktop.bin".into(),
                algorithm: "sha256".into(),
                hash: [1; 32],
            },
            PayloadHash {
                platform: PlatformKind::Android,
                path: "android.bin".into(),
                algorithm: "sha256".into(),
                hash: [2; 32],
            },
            PayloadHash {
                platform: PlatformKind::All,
                path: "common.bin".into(),
                algorithm: "sha256".into(),
                hash: [3; 32],
            },
        ];
        let download = tmp.path().join("platform-download");
        std::fs::create_dir_all(&download).unwrap();
        std::fs::write(download.join("desktop.bin"), b"desktop").unwrap();
        std::fs::write(download.join("common.bin"), b"common").unwrap();
        // android.bin is deliberately absent and must not be required.

        let package =
            Installer::stage(&manifest, &download, &cache, &PlatformKind::Desktop).unwrap();

        assert!(package.staging_dir().join("desktop.bin").is_file());
        assert!(package.staging_dir().join("common.bin").is_file());
        assert!(!package.staging_dir().join("android.bin").exists());
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

        let mut pkg =
            Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();
        Installer::activate(&mut pkg, &cache, &PlatformKind::Desktop).unwrap();

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

        let mut pkg =
            Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();
        Installer::activate(&mut pkg, &cache, &PlatformKind::Desktop).unwrap();

        assert!(cache.boot_marker_path().exists());
    }

    #[test]
    fn installer_activate_fails_without_staged_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let mut pkg = Package::new(manifest, tmp.path());

        let result = Installer::activate(&mut pkg, &cache, &PlatformKind::Desktop);
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
        let mut pkg1 = Installer::stage(&m1, &s1, &cache, &PlatformKind::Desktop).unwrap();
        Installer::activate(&mut pkg1, &cache, &PlatformKind::Desktop).unwrap();
        Installer::mark_boot_successful(&cache).unwrap();

        // Create and activate second package.
        let mut m2 = sample_manifest();
        m2.created_at = "2026-06-01T00:00:00Z".into();
        let s2 = tmp.path().join("dl_second");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s2.join("data.bin"), b"second").unwrap();
        let mut pkg2 = Installer::stage(&m2, &s2, &cache, &PlatformKind::Desktop).unwrap();
        Installer::activate(&mut pkg2, &cache, &PlatformKind::Desktop).unwrap();

        // Activated payloads are immutable and retained under active/<id>.
        let prev_path = cache.base_dir.join("active").join(pkg1.package_id());
        let prev_content = std::fs::read(prev_path.join("data.bin")).unwrap();
        assert_eq!(prev_content, b"first");
    }

    #[test]
    fn installer_mark_failed_boot_creates_fail_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let manifest = sample_manifest();
        let staging_dir = tmp.path().join("download_failed_boot");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("data.bin"), b"boot").unwrap();
        let mut package =
            Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();
        Installer::activate(&mut package, &cache, &PlatformKind::Desktop).unwrap();

        Installer::mark_failed_boot(&cache).unwrap();

        // Both records remain so restart has deterministic rollback metadata.
        assert!(cache.boot_marker_path().exists());
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
        let mut p1 = Installer::stage(&m1, &s1, &cache, &PlatformKind::Desktop).unwrap();
        Installer::activate(&mut p1, &cache, &PlatformKind::Desktop).unwrap();
        let id1 = p1.package_id().to_string();
        Installer::mark_boot_successful(&cache).unwrap();

        // Second package.
        let mut m2 = sample_manifest();
        m2.created_at = "2026-07-01T00:00:00Z".into();
        let s2 = tmp.path().join("dl_rep2");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s2.join("data.bin"), b"v2").unwrap();
        let mut p2 = Installer::stage(&m2, &s2, &cache, &PlatformKind::Desktop).unwrap();
        Installer::activate(&mut p2, &cache, &PlatformKind::Desktop).unwrap();

        // Active pointer should now point to p2.
        let active = cache.active_package().unwrap();
        assert_eq!(active.package_id(), p2.package_id());

        // p1 remains immutable in active so pointer rollback needs no move.
        let p1_active = cache.base_dir.join("active").join(&id1);
        assert!(p1_active.exists());
    }

    #[test]
    fn installer_rejects_manifest_before_replacing_existing_stage() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(tmp.path());
        cache.initialize().unwrap();

        let mut manifest = sample_manifest();
        manifest.payload_hashes[0].path = "../escape.bin".into();
        let package = Package::new(manifest.clone(), tmp.path());
        let existing_stage = package.staging_dir();
        std::fs::create_dir_all(&existing_stage).unwrap();
        let sentinel = existing_stage.join("sentinel.txt");
        std::fs::write(&sentinel, b"keep").unwrap();

        let download = tmp.path().join("download-malicious");
        std::fs::create_dir_all(&download).unwrap();
        std::fs::write(download.join("safe.bin"), b"data").unwrap();

        let error =
            Installer::stage(&manifest, &download, &cache, &PlatformKind::Desktop).unwrap_err();
        assert!(matches!(error, UpdateError::UnsafePath { .. }));
        assert_eq!(std::fs::read(sentinel).unwrap(), b"keep");
        assert!(download.exists());
    }

    #[test]
    fn activation_rejects_pending_and_failed_previous_until_boot_is_confirmed() {
        let temp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(temp.path());
        cache.initialize().unwrap();

        let first_manifest = sample_manifest();
        let mut first =
            stage_test_package(&cache, temp.path(), &first_manifest, "known-good-first");
        Installer::activate(&mut first, &cache, &PlatformKind::Desktop).unwrap();

        let mut second_manifest = sample_manifest();
        second_manifest.created_at = "2026-07-15T00:00:00Z".into();
        let mut second =
            stage_test_package(&cache, temp.path(), &second_manifest, "known-good-second");

        let pending_error =
            Installer::activate(&mut second, &cache, &PlatformKind::Desktop).unwrap_err();
        assert!(matches!(pending_error, UpdateError::ActivationFailed(_)));
        assert_eq!(
            cache.active_package_id().unwrap().as_deref(),
            Some(first.package_id())
        );

        Installer::mark_failed_boot(&cache).unwrap();
        let failed_error =
            Installer::activate(&mut second, &cache, &PlatformKind::Desktop).unwrap_err();
        assert!(matches!(failed_error, UpdateError::ActivationFailed(_)));

        Installer::mark_boot_successful(&cache).unwrap();
        Installer::activate(&mut second, &cache, &PlatformKind::Desktop).unwrap();
        assert_eq!(
            cache.active_package_id().unwrap().as_deref(),
            Some(second.package_id())
        );
    }

    fn stage_test_package(
        cache: &PackageCache,
        root: &Path,
        manifest: &HotUpdateManifest,
        name: &str,
    ) -> Package {
        let download = root.join(name);
        std::fs::create_dir_all(&download).unwrap();
        std::fs::write(download.join("data.bin"), b"test payload").unwrap();
        Installer::stage(manifest, &download, cache, &PlatformKind::Desktop).unwrap()
    }

    fn install_confirmed_base(cache: &PackageCache, root: &Path) -> Package {
        let manifest = sample_manifest();
        let mut package = stage_test_package(cache, root, &manifest, "confirmed-base");
        Installer::activate(&mut package, cache, &PlatformKind::Desktop).unwrap();
        Installer::mark_boot_successful(cache).unwrap();
        package
    }

    #[test]
    fn every_precommit_failure_restores_old_pointer_payload_state_and_markers() {
        let temp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(temp.path());
        cache.initialize().unwrap();
        let old = install_confirmed_base(&cache, temp.path());

        let mut manifest = sample_manifest();
        manifest.created_at = "2026-08-01T00:00:00Z".into();
        let mut next = stage_test_package(&cache, temp.path(), &manifest, "failure-next");
        let fail_points = [
            ActivationFailPoint::AfterJournal,
            ActivationFailPoint::AfterPayloadPrepared,
            ActivationFailPoint::AfterStatePrepared,
            ActivationFailPoint::AfterBootMarker,
            ActivationFailPoint::PointerReplaceFailure,
        ];

        for fail_point in fail_points {
            let error = Installer::activate_inner(
                &mut next,
                &cache,
                &PlatformKind::Desktop,
                Some(fail_point),
            )
            .unwrap_err();
            assert!(matches!(error, UpdateError::ActivationFailed(_)));
            assert_eq!(
                cache.active_package_id().unwrap().as_deref(),
                Some(old.package_id())
            );
            assert!(cache.active_dir(old.package_id()).unwrap().is_dir());
            assert!(!cache.active_dir(next.package_id()).unwrap().exists());
            assert!(cache.staged_dir(next.package_id()).unwrap().is_dir());
            assert_eq!(
                cache.read_state(next.package_id()).unwrap().state,
                PackageState::Staged
            );
            assert_eq!(next.state, PackageState::Staged);
            assert!(!cache.boot_marker_path().exists());
            assert!(!cache.transaction_path().unwrap().exists());
            let record = cache.read_activation_record().unwrap().unwrap();
            assert_eq!(record.activated_id, old.package_id());
            assert_eq!(record.phase, ActivationPhase::BootSuccessful);
        }
    }

    #[test]
    fn startup_recovers_crash_before_pointer_commit() {
        let temp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(temp.path());
        cache.initialize().unwrap();
        let old = install_confirmed_base(&cache, temp.path());

        let mut manifest = sample_manifest();
        manifest.created_at = "2026-08-02T00:00:00Z".into();
        let mut next = stage_test_package(&cache, temp.path(), &manifest, "crash-before-next");
        let transaction = ActivationTransaction {
            version: ACTIVATION_FORMAT_VERSION,
            operation: TransactionOperation::Activate,
            activated_id: next.package_id().to_string(),
            previous_id: Some(old.package_id().to_string()),
            moved_staged_to_active: true,
        };
        cache.write_transaction(&transaction).unwrap();
        std::fs::rename(next.staging_dir(), next.active_dir()).unwrap();
        next.state = PackageState::Active;
        cache.write_state(&next).unwrap();
        cache
            .write_boot_marker(&ActivationRecord::new(
                next.package_id().to_string(),
                Some(old.package_id().to_string()),
                ActivationPhase::BootPending,
            ))
            .unwrap();

        let restarted = PackageCache::new(temp.path());
        restarted.initialize().unwrap();
        assert_eq!(
            restarted.active_package_id().unwrap().as_deref(),
            Some(old.package_id())
        );
        assert!(restarted.staged_dir(next.package_id()).unwrap().is_dir());
        assert!(!restarted.active_dir(next.package_id()).unwrap().exists());
        assert_eq!(
            restarted.read_state(next.package_id()).unwrap().state,
            PackageState::Staged
        );
        assert!(!restarted.transaction_path().unwrap().exists());
        assert!(!restarted.boot_marker_path().exists());
    }

    #[test]
    fn startup_finishes_crash_after_pointer_commit_without_rollback() {
        let temp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(temp.path());
        cache.initialize().unwrap();
        let old = install_confirmed_base(&cache, temp.path());

        let mut manifest = sample_manifest();
        manifest.created_at = "2026-08-03T00:00:00Z".into();
        let mut next = stage_test_package(&cache, temp.path(), &manifest, "crash-after-next");
        let transaction = ActivationTransaction {
            version: ACTIVATION_FORMAT_VERSION,
            operation: TransactionOperation::Activate,
            activated_id: next.package_id().to_string(),
            previous_id: Some(old.package_id().to_string()),
            moved_staged_to_active: true,
        };
        cache.write_transaction(&transaction).unwrap();
        std::fs::rename(next.staging_dir(), next.active_dir()).unwrap();
        next.state = PackageState::Active;
        cache.write_state(&next).unwrap();
        cache.set_active_pointer(next.package_id()).unwrap();

        let restarted = PackageCache::new(temp.path());
        restarted.initialize().unwrap();
        assert_eq!(
            restarted.active_package_id().unwrap().as_deref(),
            Some(next.package_id())
        );
        assert!(restarted.active_dir(old.package_id()).unwrap().is_dir());
        assert!(restarted.active_dir(next.package_id()).unwrap().is_dir());
        assert!(!restarted.transaction_path().unwrap().exists());
        assert!(restarted.boot_marker_path().exists());
        let record = restarted.read_activation_record().unwrap().unwrap();
        assert_eq!(record.activated_id, next.package_id());
        assert_eq!(record.previous_id.as_deref(), Some(old.package_id()));
        assert_eq!(record.phase, ActivationPhase::BootPending);
    }

    #[test]
    fn startup_resolves_rollback_crashes_from_authoritative_pointer() {
        for committed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let cache = PackageCache::new(temp.path());
            cache.initialize().unwrap();
            let old = install_confirmed_base(&cache, temp.path());

            let mut manifest = sample_manifest();
            manifest.created_at = if committed {
                "2026-08-05T00:00:00Z".into()
            } else {
                "2026-08-06T00:00:00Z".into()
            };
            let mut next = stage_test_package(&cache, temp.path(), &manifest, "rollback-crash");
            Installer::activate(&mut next, &cache, &PlatformKind::Desktop).unwrap();
            let transaction = ActivationTransaction {
                version: ACTIVATION_FORMAT_VERSION,
                operation: TransactionOperation::Rollback,
                activated_id: next.package_id().to_string(),
                previous_id: Some(old.package_id().to_string()),
                moved_staged_to_active: false,
            };
            cache.write_transaction(&transaction).unwrap();
            if committed {
                cache.set_active_pointer(old.package_id()).unwrap();
            }

            let restarted = PackageCache::new(temp.path());
            restarted.initialize().unwrap();
            assert!(!restarted.transaction_path().unwrap().exists());
            let record = restarted.read_activation_record().unwrap().unwrap();
            if committed {
                assert_eq!(
                    restarted.active_package_id().unwrap().as_deref(),
                    Some(old.package_id())
                );
                assert_eq!(record.phase, ActivationPhase::RolledBack);
                assert!(!restarted.boot_marker_path().exists());
            } else {
                assert_eq!(
                    restarted.active_package_id().unwrap().as_deref(),
                    Some(next.package_id())
                );
                assert_eq!(record.phase, ActivationPhase::BootPending);
                assert!(restarted.boot_marker_path().exists());
            }
        }
    }

    #[test]
    fn rollback_uses_record_even_with_multiple_unrelated_legacy_directories() {
        let temp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(temp.path());
        cache.initialize().unwrap();
        let old = install_confirmed_base(&cache, temp.path());

        let mut manifest = sample_manifest();
        manifest.created_at = "2026-08-04T00:00:00Z".into();
        let mut next = stage_test_package(&cache, temp.path(), &manifest, "rollback-next");
        Installer::activate(&mut next, &cache, &PlatformKind::Desktop).unwrap();
        std::fs::create_dir_all(temp.path().join("previous").join("a".repeat(64))).unwrap();
        std::fs::create_dir_all(temp.path().join("previous").join("b".repeat(64))).unwrap();

        let rolled_back = crate::rollback::RollbackManager::rollback(&cache).unwrap();
        assert_eq!(rolled_back.package_id(), old.package_id());
        assert_eq!(
            cache.active_package_id().unwrap().as_deref(),
            Some(old.package_id())
        );
    }

    #[test]
    fn successful_boot_survives_restart_and_first_install_has_no_rollback_target() {
        let temp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(temp.path());
        cache.initialize().unwrap();
        let package = install_confirmed_base(&cache, temp.path());
        assert!(!crate::rollback::RollbackManager::needs_rollback(&cache));
        assert!(crate::rollback::RollbackManager::rollback(&cache).is_err());

        let restarted = PackageCache::new(temp.path());
        restarted.initialize().unwrap();
        assert_eq!(
            restarted.active_package_id().unwrap().as_deref(),
            Some(package.package_id())
        );
        assert!(!crate::rollback::RollbackManager::needs_rollback(
            &restarted
        ));
    }

    #[cfg(unix)]
    #[test]
    fn directory_copy_rejects_symlinks_before_creating_destination() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        let destination = tmp.path().join("destination");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(tmp.path().join("outside.bin"), b"outside").unwrap();
        symlink(tmp.path().join("outside.bin"), source.join("link.bin")).unwrap();

        let error = copy_dir_all(&source, &destination).unwrap_err();
        assert!(matches!(error, UpdateError::UnsafePath { .. }));
        assert!(!destination.exists());
    }
}
