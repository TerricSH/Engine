use engine_serialize::HashDigest;
use thiserror::Error;

/// Errors that can occur during the hot update lifecycle.
#[derive(Debug, Error)]
pub enum UpdateError {
    /// The manifest failed semantic validation.
    #[error("manifest parse error: {0}")]
    ManifestParse(String),

    /// The manifest was rejected by the verifier.
    #[error("manifest rejected: {0}")]
    ManifestRejected(String),

    /// A payload download failed.
    #[error("download failed: {0}")]
    DownloadFailed(String),

    /// A payload hash did not match the expected value.
    #[error("hash mismatch for {path}: expected {expected:?}, actual {actual:?}")]
    HashMismatch {
        path: String,
        expected: HashDigest,
        actual: HashDigest,
    },

    /// The manifest has no signature.
    #[error("signature missing")]
    SignatureMissing,

    /// The manifest signature is invalid.
    #[error("signature invalid: {0}")]
    SignatureInvalid(String),

    /// An I/O error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The package engine version is incompatible.
    #[error("incompatible version: {0}")]
    IncompatibleVersion(String),

    /// The package was rejected for the target platform.
    #[error("platform rejected: {0}")]
    PlatformRejected(String),

    /// The on-disk cache is corrupt.
    #[error("cache corrupt: {0}")]
    CacheCorrupt(String),

    /// Package activation (switch to staged) failed.
    #[error("activation failed: {0}")]
    ActivationFailed(String),

    /// Rollback to a previous version failed.
    #[error("rollback failed: {0}")]
    RollbackFailed(String),

    /// Applying resource or logic updates failed.
    #[error("apply failed: {0}")]
    ApplyFailed(String),
}

impl From<serde_json::Error> for UpdateError {
    fn from(err: serde_json::Error) -> Self {
        UpdateError::ManifestParse(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_error_manifest_parse_display() {
        let err = UpdateError::ManifestParse("missing field".into());
        assert_eq!(err.to_string(), "manifest parse error: missing field");
    }

    #[test]
    fn update_error_manifest_rejected_display() {
        let err = UpdateError::ManifestRejected("bad signature".into());
        assert_eq!(err.to_string(), "manifest rejected: bad signature");
    }

    #[test]
    fn update_error_download_failed_display() {
        let err = UpdateError::DownloadFailed("connection refused".into());
        assert_eq!(err.to_string(), "download failed: connection refused");
    }

    #[test]
    fn update_error_hash_mismatch_display() {
        let err = UpdateError::HashMismatch {
            path: "file.bin".into(),
            expected: [1u8; 32],
            actual: [2u8; 32],
        };
        let msg = err.to_string();
        assert!(msg.contains("hash mismatch"));
        assert!(msg.contains("file.bin"));
    }

    #[test]
    fn update_error_signature_missing_display() {
        let err = UpdateError::SignatureMissing;
        assert_eq!(err.to_string(), "signature missing");
    }

    #[test]
    fn update_error_signature_invalid_display() {
        let err = UpdateError::SignatureInvalid("bad key".into());
        assert_eq!(err.to_string(), "signature invalid: bad key");
    }

    #[test]
    fn update_error_io_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err = UpdateError::from(io_err);
        assert!(err.to_string().contains("io error"));
    }

    #[test]
    fn update_error_incompatible_version_display() {
        let err = UpdateError::IncompatibleVersion("engine 2.0 required".into());
        assert_eq!(err.to_string(), "incompatible version: engine 2.0 required");
    }

    #[test]
    fn update_error_platform_rejected_display() {
        let err = UpdateError::PlatformRejected("iOS no assemblies".into());
        assert_eq!(err.to_string(), "platform rejected: iOS no assemblies");
    }

    #[test]
    fn update_error_cache_corrupt_display() {
        let err = UpdateError::CacheCorrupt("bad state file".into());
        assert_eq!(err.to_string(), "cache corrupt: bad state file");
    }

    #[test]
    fn update_error_activation_failed_display() {
        let err = UpdateError::ActivationFailed("disk full".into());
        assert_eq!(err.to_string(), "activation failed: disk full");
    }

    #[test]
    fn update_error_rollback_failed_display() {
        let err = UpdateError::RollbackFailed("no previous version".into());
        assert_eq!(err.to_string(), "rollback failed: no previous version");
    }

    #[test]
    fn update_error_apply_failed_display() {
        let err = UpdateError::ApplyFailed("reload error".into());
        assert_eq!(err.to_string(), "apply failed: reload error");
    }

    #[test]
    fn update_error_from_serde_json_error() {
        let serde_err = serde_json::from_str::<()>("").unwrap_err();
        let err = UpdateError::from(serde_err);
        assert!(matches!(err, UpdateError::ManifestParse(_)));
    }

    #[test]
    fn update_error_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let err = UpdateError::from(io_err);
        assert!(matches!(err, UpdateError::Io(_)));
    }

    #[test]
    fn update_error_debug() {
        let err = UpdateError::ManifestParse("oops".into());
        let debug = format!("{err:?}");
        assert!(debug.contains("ManifestParse"));
    }
}
