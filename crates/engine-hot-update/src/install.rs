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
#[path = "install_tests.rs"]
mod tests;
