use tracing::{debug, info};

use crate::cache::PackageCache;
use crate::error::UpdateError;
use crate::package::{Package, PackageState};
use crate::path_safety::{
    ensure_tree_has_no_links, remove_dir_all_safe, safe_package_path, validate_package_id,
};
use crate::transaction::{
    ActivationPhase, ActivationTransaction, TransactionOperation, ACTIVATION_FORMAT_VERSION,
};

/// Manages deterministic rollback to the previous known-good package.
pub struct RollbackManager;

impl RollbackManager {
    /// Atomically switch to the exact package named by the activation record.
    ///
    /// Activated payload directories are immutable and retained. The active
    /// pointer is therefore the sole commit point; filesystem enumeration is
    /// never used to select a rollback target.
    pub fn rollback(cache: &PackageCache) -> Result<Package, UpdateError> {
        cache.recover_interrupted_transaction()?;
        let record = cache.read_activation_record()?.ok_or_else(|| {
            UpdateError::RollbackFailed("no deterministic activation record found".into())
        })?;
        if record.phase == ActivationPhase::RolledBack {
            return Err(UpdateError::RollbackFailed(
                "the most recent activation has already been rolled back".into(),
            ));
        }

        let current_id = cache.active_package_id()?.ok_or_else(|| {
            UpdateError::RollbackFailed("there is no active package to roll back".into())
        })?;
        if current_id != record.activated_id {
            return Err(UpdateError::CacheCorrupt(format!(
                "active pointer {current_id} does not match activation record {}",
                record.activated_id
            )));
        }
        let previous_id = record.previous_id.clone().ok_or_else(|| {
            UpdateError::RollbackFailed("the active package has no previous version".into())
        })?;
        let previous_dir = cache.active_dir(&previous_id)?;
        if !previous_dir.is_dir() {
            return Err(UpdateError::RollbackFailed(format!(
                "immutable rollback payload is missing for {previous_id}"
            )));
        }
        ensure_tree_has_no_links(&previous_dir, "immutable rollback package")?;
        let mut previous_package = cache.read_state(&previous_id).map_err(|error| {
            UpdateError::RollbackFailed(format!(
                "cannot read rollback package {previous_id}: {error}"
            ))
        })?;

        let transaction = ActivationTransaction {
            version: ACTIVATION_FORMAT_VERSION,
            operation: TransactionOperation::Rollback,
            activated_id: current_id,
            previous_id: Some(previous_id.clone()),
            moved_staged_to_active: false,
        };
        cache.write_transaction(&transaction)?;
        if let Err(error) = cache.set_active_pointer(&previous_id) {
            // A replace API may theoretically report an error after making
            // the replacement. Inspect the authoritative pointer before
            // deciding whether this was a committed rollback.
            if cache.active_package_id()?.as_deref() != Some(previous_id.as_str()) {
                let _ = cache.clear_transaction();
                return Err(UpdateError::RollbackFailed(format!(
                    "atomic pointer switch failed: {error}"
                )));
            }
        }

        // No fallible cleanup is allowed to turn a committed pointer switch
        // into a reported failure. A surviving journal is finalized at init.
        if let Err(error) = cache.finish_committed_rollback(&transaction) {
            debug!("rollback committed; metadata cleanup deferred: {error}");
        }
        previous_package.state = PackageState::Active;
        previous_package.active_path = previous_dir;

        info!(package_id = %previous_id, "rollback completed");
        Ok(previous_package)
    }

    /// Whether a pending or explicitly failed boot requires rollback.
    pub fn needs_rollback(cache: &PackageCache) -> bool {
        let marker = match cache.checked_boot_marker_path() {
            Ok(path) => path,
            Err(_) => return true,
        };
        if marker.exists() {
            debug!("boot marker present; rollback may be required");
            return true;
        }
        cache
            .boot_failed_path()
            .map(|path| path.exists())
            .unwrap_or(true)
    }

    /// Remove a package unless it is active or is the deterministic rollback
    /// target retained by the latest activation record.
    pub fn remove_package(cache: &PackageCache, package_id: &str) -> Result<(), UpdateError> {
        validate_package_id(package_id)?;
        if cache.active_package_id()?.as_deref() == Some(package_id) {
            return Err(UpdateError::RollbackFailed(
                "cannot remove the active package".into(),
            ));
        }
        if cache
            .read_activation_record()?
            .and_then(|record| record.previous_id)
            .as_deref()
            == Some(package_id)
        {
            return Err(UpdateError::RollbackFailed(
                "cannot remove the retained rollback target".into(),
            ));
        }

        let meta_dir = safe_package_path(&cache.base_dir, "packages", package_id)?;
        let staged_dir = safe_package_path(&cache.base_dir, "staged", package_id)?;
        let active_dir = safe_package_path(&cache.base_dir, "active", package_id)?;
        let previous_dir = safe_package_path(&cache.base_dir, "previous", package_id)?;

        for (path, field) in [
            (&meta_dir, "package metadata"),
            (&staged_dir, "staged package"),
            (&active_dir, "active package"),
            (&previous_dir, "legacy previous package"),
        ] {
            if path.exists() {
                ensure_tree_has_no_links(path, field)?;
            }
        }

        for (path, field) in [
            (&meta_dir, "package metadata"),
            (&staged_dir, "staged package"),
            (&active_dir, "inactive immutable package"),
            (&previous_dir, "legacy previous package"),
        ] {
            if path.exists() {
                remove_dir_all_safe(path, field)?;
            }
        }
        debug!("package {package_id} removed");
        Ok(())
    }
}
