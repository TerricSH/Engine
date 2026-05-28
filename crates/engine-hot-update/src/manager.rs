use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

use crate::manifest::{HotUpdateError, InstallationSnapshot, PackageManifest, PackageState};
use crate::verify::compute_hash;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// A single file parsed from the raw package blob.
#[derive(Debug, Clone)]
struct ParsedFile {
    relative_path: String,
    data: Vec<u8>,
}

/// Internal bookkeeping for each registered package.
#[derive(Debug, Clone)]
struct PackageEntry {
    manifest: PackageManifest,
    state: PackageState,
    snapshot: Option<InstallationSnapshot>,
    files: Vec<ParsedFile>,
    /// Raw blob retained for content-hash verification.
    package_data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// PackageManager
// ---------------------------------------------------------------------------

pub struct PackageManager {
    base_path: PathBuf,
    packages: BTreeMap<String, PackageEntry>,
}

impl PackageManager {
    pub fn new(base_path: &Path) -> Self {
        info!("PackageManager::new(base_path: {:?})", base_path);
        Self {
            base_path: base_path.to_path_buf(),
            packages: BTreeMap::new(),
        }
    }

    /// Register a package from a manifest and its raw blob data.
    ///
    /// The `data` blob must follow the package binary format:
    /// - first 8 bytes: file count (u64 LE)
    /// - per file:
    ///   - 8 bytes: path length (u64 LE)
    ///   - N bytes: UTF-8 relative path
    ///   - 8 bytes: content length (u64 LE)
    ///   - M bytes: file content
    pub fn register_package(
        &mut self,
        manifest: PackageManifest,
        data: &[u8],
    ) -> Result<(), HotUpdateError> {
        let package_id = manifest.package_id.clone();
        info!("registering package: {}", package_id);

        let files = parse_package_blob(data)?;

        let entry = PackageEntry {
            manifest,
            state: PackageState::ReadyToInstall,
            snapshot: None,
            files,
            package_data: data.to_vec(),
        };

        self.packages.insert(package_id, entry);
        debug!("package registered successfully");
        Ok(())
    }

    /// Install a previously registered package.
    ///
    /// Before extracting files the content hash stored in the manifest is
    /// verified against the actual SHA-256 of the raw blob.  Existing files
    /// at `base_path/installed/<id>/` are backed up to
    /// `base_path/backups/<id>/<timestamp>/`.
    pub fn install(&mut self, package_id: &str) -> Result<(), HotUpdateError> {
        info!("installing package: {}", package_id);

        // Check existence.
        if !self.packages.contains_key(package_id) {
            return Err(HotUpdateError::PackageNotFound(package_id.to_string()));
        }

        // Verify content hash (immutable borrow first).
        let (files, manifest_version) = {
            let entry = self.packages.get(package_id).ok_or_else(|| {
                HotUpdateError::PackageNotFound(package_id.to_string())
            })?;
            let actual_hash = compute_hash(&entry.package_data);
            if actual_hash != entry.manifest.content_hash {
                return Err(HotUpdateError::VerificationFailed {
                    expected: entry.manifest.content_hash,
                    actual: actual_hash,
                });
            }
            (entry.files.clone(), entry.manifest.version.clone())
        };

        let entry = self.packages.get_mut(package_id).ok_or_else(|| {
            HotUpdateError::PackageNotFound(package_id.to_string())
        })?;

        let install_dir = self.base_path.join("installed").join(package_id);
        let timestamp = current_timestamp();
        let backup_dir = self
            .base_path
            .join("backups")
            .join(package_id)
            .join(&timestamp);

        // Back up any previously installed files.
        if install_dir.exists() {
            let has_files =
                fs::read_dir(&install_dir)
                    .map(|mut it| it.next().is_some())
                    .unwrap_or(false);
            if has_files {
                fs::create_dir_all(&backup_dir)?;
                copy_dir_contents(&install_dir, &backup_dir)?;
            }
            fs::remove_dir_all(&install_dir)?;
        }
        fs::create_dir_all(&install_dir)?;

        // Extract package files.
        for pf in &files {
            let file_path = install_dir.join(&pf.relative_path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&file_path, &pf.data)?;
        }

        let previous_state = entry.state.clone();
        entry.state = PackageState::Installed {
            version: manifest_version,
            installed_at: timestamp.clone(),
        };
        entry.snapshot = Some(InstallationSnapshot {
            package_id: package_id.to_string(),
            previous_state,
            backup_path: Some(backup_dir.to_string_lossy().to_string()),
            timestamp,
        });

        info!("package installed successfully: {}", package_id);
        Ok(())
    }

    /// Roll back an installed package to its previous state.
    ///
    /// Files are restored from the backup directory created during the last
    /// `install()` call.
    pub fn rollback(&mut self, package_id: &str) -> Result<(), HotUpdateError> {
        info!("rolling back package: {}", package_id);

        let entry = self.packages.get_mut(package_id).ok_or_else(|| {
            HotUpdateError::PackageNotFound(package_id.to_string())
        })?;

        let snapshot = entry
            .snapshot
            .as_ref()
            .ok_or_else(|| HotUpdateError::RollbackFailed("no snapshot available".to_string()))?;

        let backup_path = snapshot
            .backup_path
            .as_ref()
            .ok_or_else(|| HotUpdateError::RollbackFailed("no backup path in snapshot".to_string()))?;

        let backup_dir = Path::new(backup_path);
        if !backup_dir.exists() {
            return Err(HotUpdateError::RollbackFailed(format!(
                "backup directory not found: {}",
                backup_path
            )));
        }

        // Replace installed directory with backup contents.
        let install_dir = self.base_path.join("installed").join(package_id);
        if install_dir.exists() {
            fs::remove_dir_all(&install_dir)?;
        }
        fs::create_dir_all(&install_dir)?;
        copy_dir_contents(backup_dir, &install_dir)?;

        let prev_version = entry.manifest.version.clone();
        entry.state = PackageState::RolledBack {
            previous_version: prev_version,
        };
        entry.snapshot = None;

        info!("package rolled back successfully: {}", package_id);
        Ok(())
    }

    /// Verify the integrity of installed files.
    ///
    /// Reads each file from disk and compares its SHA-256 hash against the
    /// hash stored in the manifest.
    pub fn verify(&self, package_id: &str) -> Result<bool, HotUpdateError> {
        debug!("verifying package: {}", package_id);

        let entry = self.packages.get(package_id).ok_or_else(|| {
            HotUpdateError::PackageNotFound(package_id.to_string())
        })?;

        let install_dir = self.base_path.join("installed").join(package_id);
        if !install_dir.exists() {
            return Ok(false);
        }

        for file_entry in &entry.manifest.files {
            let file_path = install_dir.join(&file_entry.relative_path);
            let data = match fs::read(&file_path) {
                Ok(d) => d,
                Err(_) => return Ok(false),
            };
            if compute_hash(&data) != file_entry.hash {
                return Ok(false);
            }
        }

        debug!("package verification passed: {}", package_id);
        Ok(true)
    }

    /// Returns the current state of a package, or `None` if unknown.
    pub fn state(&self, package_id: &str) -> Option<&PackageState> {
        self.packages.get(package_id).map(|e| &e.state)
    }

    /// Returns manifests for all registered packages.
    pub fn list_packages(&self) -> Vec<&PackageManifest> {
        self.packages.values().map(|e| &e.manifest).collect()
    }

    /// Returns manifests for packages in the `Installed` state.
    pub fn installed_packages(&self) -> Vec<&PackageManifest> {
        self.packages
            .values()
            .filter(|e| matches!(e.state, PackageState::Installed { .. }))
            .map(|e| &e.manifest)
            .collect()
    }

    /// Remove a package from the registry. Returns `true` if the package was
    /// known.
    pub fn unregister(&mut self, package_id: &str) -> bool {
        info!("unregistering package: {}", package_id);
        self.packages.remove(package_id).is_some()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse a package blob into its constituent files.
///
/// Format:
///   [file_count: u64 LE]
///   [for each file:
///       path_len: u64 LE
///       path:      UTF-8 bytes (path_len bytes)
///       content_len: u64 LE
///       content:   raw bytes (content_len bytes) ]
fn parse_package_blob(data: &[u8]) -> Result<Vec<ParsedFile>, HotUpdateError> {
    if data.len() < 8 {
        return Err(HotUpdateError::InvalidPackage(
            "data too short for header".to_string(),
        ));
    }

    let (count_bytes, mut remaining) = data.split_at(8);
    let file_count = u64::from_le_bytes(
        count_bytes
            .try_into()
            .map_err(|_| HotUpdateError::InvalidPackage("header: failed to parse file count".to_string()))?,
    ) as usize;

    let mut files = Vec::with_capacity(file_count);

    for i in 0..file_count {
        // path_len
        if remaining.len() < 8 {
            return Err(HotUpdateError::InvalidPackage(format!(
                "file {}: missing path_len field",
                i
            )));
        }
        let (path_len_bytes, after_path_len) = remaining.split_at(8);
        let path_len = u64::from_le_bytes(
            path_len_bytes.try_into().map_err(|_| {
                HotUpdateError::InvalidPackage(format!(
                    "file {}: failed to parse path length",
                    i
                ))
            })?,
        ) as usize;

        // path
        if after_path_len.len() < path_len {
            return Err(HotUpdateError::InvalidPackage(format!(
                "file {}: path data truncated (expected {} bytes, got {})",
                i,
                path_len,
                after_path_len.len()
            )));
        }
        let (path_bytes, after_path) = after_path_len.split_at(path_len);
        let relative_path = String::from_utf8(path_bytes.to_vec()).map_err(|_| {
            HotUpdateError::InvalidPackage(format!(
                "file {}: path is not valid UTF-8",
                i
            ))
        })?;

        // content_len
        if after_path.len() < 8 {
            return Err(HotUpdateError::InvalidPackage(format!(
                "file {}: missing content_len field",
                i
            )));
        }
        let (content_len_bytes, after_content_len) = after_path.split_at(8);
        let content_len = u64::from_le_bytes(
            content_len_bytes.try_into().map_err(|_| {
                HotUpdateError::InvalidPackage(format!(
                    "file {}: failed to parse content length",
                    i
                ))
            })?,
        ) as usize;

        // content
        if after_content_len.len() < content_len {
            return Err(HotUpdateError::InvalidPackage(format!(
                "file {}: content data truncated (expected {} bytes, got {})",
                i,
                content_len,
                after_content_len.len()
            )));
        }
        let (content, after_content) = after_content_len.split_at(content_len);

        files.push(ParsedFile {
            relative_path,
            data: content.to_vec(),
        });

        remaining = after_content;
    }

    Ok(files)
}

/// Recursively copy the contents of `src` into `dst`.  `dst` must already
/// exist.
fn copy_dir_contents(src: &Path, dst: &Path) -> Result<(), HotUpdateError> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let dest_path = dst.join(&name);

        if file_type.is_dir() {
            fs::create_dir_all(&dest_path)?;
            copy_dir_contents(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

/// Return an ISO-8601-ish timestamp string based on the system clock.
fn current_timestamp() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:09}", dur.as_secs(), dur.subsec_nanos())
}
