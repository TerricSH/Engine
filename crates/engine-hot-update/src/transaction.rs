use serde::{Deserialize, Serialize};

use crate::atomic_fs::{atomic_write, durable_rename_directory, remove_file_if_exists};
use crate::cache::PackageCache;
use crate::error::UpdateError;
use crate::package::PackageState;
use crate::path_safety::{safe_join, validate_package_id};

pub(crate) const ACTIVATION_FORMAT_VERSION: u16 = 1;

/// Durable state of the most recently committed activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationPhase {
    /// The active package has not yet been confirmed by the game.
    BootPending,
    /// The game explicitly reported that the active package failed to boot.
    BootFailed,
    /// The game confirmed that the active package booted successfully.
    BootSuccessful,
    /// The pointer was atomically switched back to `previous_id`.
    RolledBack,
}

/// Versioned, deterministic rollback metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationRecord {
    pub version: u16,
    pub activated_id: String,
    pub previous_id: Option<String>,
    pub phase: ActivationPhase,
}

impl ActivationRecord {
    pub(crate) fn new(
        activated_id: String,
        previous_id: Option<String>,
        phase: ActivationPhase,
    ) -> Self {
        Self {
            version: ACTIVATION_FORMAT_VERSION,
            activated_id,
            previous_id,
            phase,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), UpdateError> {
        if self.version != ACTIVATION_FORMAT_VERSION {
            return Err(UpdateError::CacheCorrupt(format!(
                "unsupported activation record version {}",
                self.version
            )));
        }
        validate_package_id(&self.activated_id).map_err(|error| {
            UpdateError::CacheCorrupt(format!("invalid activated package id: {error}"))
        })?;
        if let Some(previous_id) = &self.previous_id {
            validate_package_id(previous_id).map_err(|error| {
                UpdateError::CacheCorrupt(format!("invalid previous package id: {error}"))
            })?;
            if previous_id == &self.activated_id {
                return Err(UpdateError::CacheCorrupt(
                    "activation record points to the same active and previous package".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransactionOperation {
    Activate,
    Rollback,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ActivationTransaction {
    pub version: u16,
    pub operation: TransactionOperation,
    pub activated_id: String,
    pub previous_id: Option<String>,
    pub moved_staged_to_active: bool,
}

impl ActivationTransaction {
    pub(crate) fn validate(&self) -> Result<(), UpdateError> {
        if self.version != ACTIVATION_FORMAT_VERSION {
            return Err(UpdateError::CacheCorrupt(format!(
                "unsupported activation transaction version {}",
                self.version
            )));
        }
        validate_package_id(&self.activated_id).map_err(|error| {
            UpdateError::CacheCorrupt(format!("invalid transaction package id: {error}"))
        })?;
        if let Some(previous_id) = &self.previous_id {
            validate_package_id(previous_id).map_err(|error| {
                UpdateError::CacheCorrupt(format!("invalid transaction previous id: {error}"))
            })?;
        }
        Ok(())
    }
}

impl PackageCache {
    pub(crate) fn activation_record_path(&self) -> Result<std::path::PathBuf, UpdateError> {
        safe_join(
            &self.base_dir,
            "activation_record.json",
            "activation record",
        )
    }

    pub(crate) fn transaction_path(&self) -> Result<std::path::PathBuf, UpdateError> {
        safe_join(
            &self.base_dir,
            "activation_transaction.json",
            "activation transaction",
        )
    }

    pub(crate) fn boot_failed_path(&self) -> Result<std::path::PathBuf, UpdateError> {
        safe_join(&self.base_dir, "boot_failed", "boot failure marker")
    }

    pub(crate) fn read_activation_record(&self) -> Result<Option<ActivationRecord>, UpdateError> {
        let path = self.activation_record_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let record: ActivationRecord =
            serde_json::from_slice(&std::fs::read(&path)?).map_err(|error| {
                UpdateError::CacheCorrupt(format!("invalid activation record: {error}"))
            })?;
        record.validate()?;
        Ok(Some(record))
    }

    pub(crate) fn write_activation_record(
        &self,
        record: &ActivationRecord,
    ) -> Result<(), UpdateError> {
        record.validate()?;
        let bytes = serde_json::to_vec_pretty(record).map_err(|error| {
            UpdateError::CacheCorrupt(format!("cannot encode activation record: {error}"))
        })?;
        atomic_write(&self.activation_record_path()?, &bytes)
    }

    pub(crate) fn read_transaction(&self) -> Result<Option<ActivationTransaction>, UpdateError> {
        let path = self.transaction_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let transaction: ActivationTransaction = serde_json::from_slice(&std::fs::read(&path)?)
            .map_err(|error| {
                UpdateError::CacheCorrupt(format!("invalid activation transaction: {error}"))
            })?;
        transaction.validate()?;
        Ok(Some(transaction))
    }

    pub(crate) fn write_transaction(
        &self,
        transaction: &ActivationTransaction,
    ) -> Result<(), UpdateError> {
        transaction.validate()?;
        let bytes = serde_json::to_vec_pretty(transaction).map_err(|error| {
            UpdateError::CacheCorrupt(format!("cannot encode activation transaction: {error}"))
        })?;
        atomic_write(&self.transaction_path()?, &bytes)
    }

    pub(crate) fn clear_transaction(&self) -> Result<(), UpdateError> {
        remove_file_if_exists(&self.transaction_path()?)
    }

    pub(crate) fn write_boot_marker(&self, record: &ActivationRecord) -> Result<(), UpdateError> {
        record.validate()?;
        let bytes = serde_json::to_vec_pretty(record).map_err(|error| {
            UpdateError::CacheCorrupt(format!("cannot encode boot marker: {error}"))
        })?;
        atomic_write(&self.checked_boot_marker_path()?, &bytes)
    }

    pub(crate) fn clear_boot_markers(&self) -> Result<(), UpdateError> {
        remove_file_if_exists(&self.checked_boot_marker_path()?)?;
        remove_file_if_exists(&self.boot_failed_path()?)
    }

    /// Recover a journal left by a process interruption. The atomically
    /// replaced active pointer is authoritative: an old pointer means the
    /// operation did not commit, while the requested pointer means it did.
    pub(crate) fn recover_interrupted_transaction(&self) -> Result<(), UpdateError> {
        let Some(transaction) = self.read_transaction()? else {
            return self.migrate_legacy_rollback_record();
        };
        let pointer = self.active_package_id()?;

        match transaction.operation {
            TransactionOperation::Activate => {
                if pointer.as_deref() == Some(transaction.activated_id.as_str()) {
                    self.finish_committed_activation(&transaction)?;
                } else if pointer == transaction.previous_id {
                    self.revert_uncommitted_activation(&transaction)?;
                } else {
                    return Err(UpdateError::CacheCorrupt(format!(
                        "activation transaction expected pointer {:?} or {}, found {:?}",
                        transaction.previous_id, transaction.activated_id, pointer
                    )));
                }
            }
            TransactionOperation::Rollback => {
                let previous_id = transaction.previous_id.as_deref().ok_or_else(|| {
                    UpdateError::CacheCorrupt(
                        "rollback transaction has no previous package id".into(),
                    )
                })?;
                if pointer.as_deref() == Some(previous_id) {
                    self.finish_committed_rollback(&transaction)?;
                } else if pointer.as_deref() == Some(transaction.activated_id.as_str()) {
                    self.clear_transaction()?;
                } else {
                    return Err(UpdateError::CacheCorrupt(format!(
                        "rollback transaction expected pointer {} or {}, found {:?}",
                        transaction.activated_id, previous_id, pointer
                    )));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn finish_committed_activation(
        &self,
        transaction: &ActivationTransaction,
    ) -> Result<(), UpdateError> {
        let active_dir = self.active_dir(&transaction.activated_id)?;
        if !active_dir.is_dir() {
            return Err(UpdateError::CacheCorrupt(format!(
                "committed active package has no payload directory: {}",
                transaction.activated_id
            )));
        }

        let record = ActivationRecord::new(
            transaction.activated_id.clone(),
            transaction.previous_id.clone(),
            ActivationPhase::BootPending,
        );
        self.write_activation_record(&record)?;
        self.write_boot_marker(&record)?;
        remove_file_if_exists(&self.boot_failed_path()?)?;

        if let Ok(mut package) = self.read_state(&transaction.activated_id) {
            package.state = PackageState::Active;
            self.write_state(&package)?;
        }
        self.clear_transaction()
    }

    pub(crate) fn finish_committed_rollback(
        &self,
        transaction: &ActivationTransaction,
    ) -> Result<(), UpdateError> {
        let previous_id = transaction.previous_id.as_deref().ok_or_else(|| {
            UpdateError::CacheCorrupt("committed rollback has no previous package id".into())
        })?;
        if !self.active_dir(previous_id)?.is_dir() {
            return Err(UpdateError::CacheCorrupt(format!(
                "rollback target has no immutable active directory: {previous_id}"
            )));
        }

        let record = ActivationRecord::new(
            transaction.activated_id.clone(),
            transaction.previous_id.clone(),
            ActivationPhase::RolledBack,
        );
        self.write_activation_record(&record)?;
        self.clear_boot_markers()?;
        if let Ok(mut active_package) = self.read_state(previous_id) {
            active_package.state = PackageState::Active;
            self.write_state(&active_package)?;
        }
        if let Ok(mut rolled_back_package) = self.read_state(&transaction.activated_id) {
            rolled_back_package.state = PackageState::RolledBack;
            self.write_state(&rolled_back_package)?;
        }
        self.clear_transaction()
    }

    fn revert_uncommitted_activation(
        &self,
        transaction: &ActivationTransaction,
    ) -> Result<(), UpdateError> {
        if transaction.moved_staged_to_active {
            let active_dir = self.active_dir(&transaction.activated_id)?;
            let staged_dir = self.staged_dir(&transaction.activated_id)?;
            match (active_dir.exists(), staged_dir.exists()) {
                (true, false) => durable_rename_directory(&active_dir, &staged_dir)?,
                (false, true) => {}
                (true, true) => {
                    return Err(UpdateError::CacheCorrupt(format!(
                        "both active and staged payloads exist while reverting {}",
                        transaction.activated_id
                    )));
                }
                (false, false) => {
                    return Err(UpdateError::CacheCorrupt(format!(
                        "payloads disappeared while reverting {}",
                        transaction.activated_id
                    )));
                }
            }
        }

        if let Ok(mut package) = self.read_state(&transaction.activated_id) {
            package.state = PackageState::Staged;
            self.write_state(&package)?;
        }

        self.clear_boot_markers()?;
        if let Some(record) = self.read_activation_record()? {
            if transaction.previous_id.as_deref() == Some(record.activated_id.as_str()) {
                match record.phase {
                    ActivationPhase::BootPending => self.write_boot_marker(&record)?,
                    ActivationPhase::BootFailed => {
                        self.write_boot_marker(&record)?;
                        atomic_write(&self.boot_failed_path()?, b"failed")?;
                    }
                    ActivationPhase::BootSuccessful | ActivationPhase::RolledBack => {}
                }
            }
        }
        self.clear_transaction()
    }

    /// Migrate the old `previous/<id>` layout only when it is unambiguous.
    /// Extra legacy directories without a durable record are corruption,
    /// never an invitation to choose a filesystem iteration order.
    fn migrate_legacy_rollback_record(&self) -> Result<(), UpdateError> {
        if let Some(record) = self.read_activation_record()? {
            let pointer = self.active_package_id()?;
            let expected_pointer = match record.phase {
                ActivationPhase::RolledBack => record.previous_id.as_deref(),
                ActivationPhase::BootPending
                | ActivationPhase::BootFailed
                | ActivationPhase::BootSuccessful => Some(record.activated_id.as_str()),
            };
            if pointer.as_deref() != expected_pointer {
                return Err(UpdateError::CacheCorrupt(format!(
                    "activation record phase {:?} disagrees with active pointer {:?}",
                    record.phase, pointer
                )));
            }
            match record.phase {
                ActivationPhase::BootPending => self.write_boot_marker(&record)?,
                ActivationPhase::BootFailed => {
                    self.write_boot_marker(&record)?;
                    atomic_write(&self.boot_failed_path()?, b"failed")?;
                }
                ActivationPhase::BootSuccessful | ActivationPhase::RolledBack => {
                    self.clear_boot_markers()?;
                }
            }
            return Ok(());
        }
        let previous_root = safe_join(&self.base_dir, "previous", "legacy previous root")?;
        let mut previous_ids = Vec::new();
        for entry in std::fs::read_dir(&previous_root)? {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if !metadata.is_dir() || crate::path_safety::is_link_or_reparse(&metadata) {
                return Err(UpdateError::CacheCorrupt(format!(
                    "unexpected entry in legacy previous directory: {}",
                    entry.path().display()
                )));
            }
            let package_id = entry
                .file_name()
                .into_string()
                .map_err(|_| UpdateError::CacheCorrupt("non-UTF-8 legacy package id".into()))?;
            validate_package_id(&package_id).map_err(|error| {
                UpdateError::CacheCorrupt(format!("invalid legacy package id: {error}"))
            })?;
            previous_ids.push(package_id);
        }
        if previous_ids.is_empty() {
            let Some(activated_id) = self.active_package_id()? else {
                return Ok(());
            };
            if !self.active_dir(&activated_id)?.is_dir() {
                return Err(UpdateError::CacheCorrupt(format!(
                    "legacy active pointer has no payload directory: {activated_id}"
                )));
            }
            self.read_state(&activated_id)?;
            let phase = self.legacy_boot_phase(&activated_id)?;
            let record = ActivationRecord::new(activated_id, None, phase);
            self.write_activation_record(&record)?;
            if phase == ActivationPhase::BootPending {
                self.write_boot_marker(&record)?;
            }
            return Ok(());
        }
        if previous_ids.len() != 1 {
            return Err(UpdateError::CacheCorrupt(format!(
                "legacy rollback cache is ambiguous: found {} previous package directories",
                previous_ids.len()
            )));
        }

        let activated_id = self.active_package_id()?.ok_or_else(|| {
            UpdateError::CacheCorrupt(
                "legacy previous package exists without an active pointer".into(),
            )
        })?;
        let previous_id = previous_ids.pop().expect("length checked");
        if activated_id == previous_id {
            return Err(UpdateError::CacheCorrupt(
                "legacy active and previous package IDs are identical".into(),
            ));
        }
        let legacy_dir = self.previous_dir(&previous_id)?;
        let immutable_dir = self.active_dir(&previous_id)?;
        if immutable_dir.exists() {
            return Err(UpdateError::CacheCorrupt(format!(
                "legacy rollback target already has an active directory: {previous_id}"
            )));
        }
        durable_rename_directory(&legacy_dir, &immutable_dir)?;

        let phase = self.legacy_boot_phase(&activated_id)?;
        let record = ActivationRecord::new(activated_id, Some(previous_id), phase);
        self.write_activation_record(&record)?;
        if phase == ActivationPhase::BootPending {
            self.write_boot_marker(&record)?;
        }
        Ok(())
    }

    fn legacy_boot_phase(&self, activated_id: &str) -> Result<ActivationPhase, UpdateError> {
        let marker_path = self.checked_boot_marker_path()?;
        if !marker_path.exists() {
            return Ok(ActivationPhase::BootSuccessful);
        }
        let marker = std::fs::read_to_string(&marker_path)?;
        if marker.trim() != activated_id {
            return Err(UpdateError::CacheCorrupt(
                "legacy boot marker does not match the active pointer".into(),
            ));
        }
        Ok(ActivationPhase::BootPending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;
    use engine_serialize::{HotUpdateManifest, RollbackMetadata, SchemaVersion};

    fn manifest(created_at: &str) -> HotUpdateManifest {
        HotUpdateManifest {
            manifest_version: SchemaVersion::new(0, 1, 0),
            engine_version: "1.5.0".into(),
            script_api_version: (1, 5),
            content_schema_version: SchemaVersion::new(1, 0, 0),
            logic_asset_schema_version: SchemaVersion::new(1, 0, 0),
            platform_payloads: Vec::new(),
            payload_hashes: Vec::new(),
            signature: None,
            rollback: RollbackMetadata {
                previous_manifest_hash: None,
                fallback_manifest_path: None,
                min_safe_engine_version: "1.4.0".into(),
            },
            created_at: created_at.into(),
        }
    }

    #[test]
    fn unambiguous_legacy_previous_layout_is_migrated() {
        let temp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(temp.path());
        cache.initialize().unwrap();
        let current = Package::new(manifest("2026-01-02T00:00:00Z"), temp.path());
        let previous = Package::new(manifest("2026-01-01T00:00:00Z"), temp.path());
        cache.write_state(&current).unwrap();
        cache.write_state(&previous).unwrap();
        std::fs::create_dir_all(cache.active_dir(current.package_id()).unwrap()).unwrap();
        std::fs::create_dir_all(cache.previous_dir(previous.package_id()).unwrap()).unwrap();
        cache.set_active_pointer(current.package_id()).unwrap();
        std::fs::write(cache.boot_marker_path(), current.package_id()).unwrap();

        let restarted = PackageCache::new(temp.path());
        restarted.initialize().unwrap();
        assert!(restarted
            .active_dir(previous.package_id())
            .unwrap()
            .is_dir());
        assert!(!restarted
            .previous_dir(previous.package_id())
            .unwrap()
            .exists());
        let record = restarted.read_activation_record().unwrap().unwrap();
        assert_eq!(record.activated_id, current.package_id());
        assert_eq!(record.previous_id.as_deref(), Some(previous.package_id()));
        assert_eq!(record.phase, ActivationPhase::BootPending);
    }

    #[test]
    fn ambiguous_legacy_previous_layout_is_cache_corruption() {
        let temp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(temp.path());
        cache.initialize().unwrap();
        std::fs::create_dir_all(temp.path().join("previous").join("a".repeat(64))).unwrap();
        std::fs::create_dir_all(temp.path().join("previous").join("b".repeat(64))).unwrap();

        let restarted = PackageCache::new(temp.path());
        assert!(matches!(
            restarted.initialize(),
            Err(UpdateError::CacheCorrupt(_))
        ));
    }
}
