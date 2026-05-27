// ---------------------------------------------------------------------------
// HotUpdateError
// ---------------------------------------------------------------------------

use engine_serialize::HashDigest;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HotUpdateError {
    #[error("package not found: {0}")]
    PackageNotFound(String),

    #[error("verification failed: expected {expected:?}, actual {actual:?}")]
    VerificationFailed {
        expected: HashDigest,
        actual: HashDigest,
    },

    #[error("install failed: {0}")]
    InstallFailed(String),

    #[error("rollback failed: {0}")]
    RollbackFailed(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("invalid package: {0}")]
    InvalidPackage(String),
}

impl From<std::io::Error> for HotUpdateError {
    fn from(err: std::io::Error) -> Self {
        HotUpdateError::Io(err.to_string())
    }
}

// ---------------------------------------------------------------------------
// PackageState
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PackageState {
    NotInstalled,
    Downloading {
        progress: f32,
    },
    ReadyToInstall,
    Installed {
        version: String,
        installed_at: String,
    },
    InstallFailed {
        error: String,
    },
    RolledBack {
        previous_version: String,
    },
}

// ---------------------------------------------------------------------------
// PackageFileEntry
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageFileEntry {
    pub relative_path: String,
    pub size_bytes: u64,
    pub hash: HashDigest,
}

// ---------------------------------------------------------------------------
// PackageManifest
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageManifest {
    pub package_id: String,
    pub version: String,
    pub required_engine_version: String,
    pub files: Vec<PackageFileEntry>,
    pub content_hash: HashDigest,
    pub dependencies: Vec<String>,
    pub description: String,
    pub release_notes: String,
}

// ---------------------------------------------------------------------------
// InstallationSnapshot
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct InstallationSnapshot {
    pub package_id: String,
    pub previous_state: PackageState,
    pub backup_path: Option<String>,
    pub timestamp: String,
}
