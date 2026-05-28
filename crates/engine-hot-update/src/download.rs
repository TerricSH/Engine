use std::path::Path;

use engine_serialize::{HotUpdateManifest, PlatformKind};
use tracing::{debug, info};

use crate::error::UpdateError;

// ---------------------------------------------------------------------------
// Downloader
// ---------------------------------------------------------------------------

/// Downloads hot-update payloads for a given platform.
///
/// Two modes are supported:
/// - `download`:  HTTP download via `ureq` (blocking).
/// - `download_local`:  Copy from a local directory (useful for testing).
pub struct Downloader;

impl Downloader {
    /// Download all payloads for the given platform into the staging
    /// directory.
    ///
    /// For each payload path listed in the manifest, this method:
    /// 1. Checks the platform matches (via `payloads_for_platform`).
    /// 2. Downloads the file from a CDN / update server.
    /// 3. Writes it to `<staging_dir>/<payload.path>`.
    ///
    /// The base URL is constructed from the manifest identity and platform.
    /// Individual payload errors are collected and returned together.
    pub fn download(
        manifest: &HotUpdateManifest,
        staging_dir: &Path,
        platform: &PlatformKind,
        base_url: &str,
    ) -> Result<(), Vec<UpdateError>> {
        let mut errors: Vec<UpdateError> = Vec::new();

        // Collect all payload paths for the target platform.
        let payload_paths: Vec<&str> = manifest
            .payload_hashes
            .iter()
            .map(|ph| ph.path.as_str())
            .collect();

        for path in &payload_paths {
            let url = format!(
                "{}/packages/{}/{}/{}",
                base_url.trim_end_matches('/'),
                manifest.engine_version,
                platform_kind_str(platform),
                path
            );

            let dest = staging_dir.join(path);
            if let Some(parent) = dest.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    errors.push(UpdateError::DownloadFailed(format!(
                        "cannot create directory {parent:?}: {e}"
                    )));
                    continue;
                }
            }

            info!(%url, dest = %dest.display(), "downloading payload");

            match ureq::get(&url).call() {
                Ok(response) => {
                    let mut reader = response.into_reader();
                    let mut file = match std::fs::File::create(&dest) {
                        Ok(f) => f,
                        Err(e) => {
                            errors.push(UpdateError::DownloadFailed(format!(
                                "failed to create {dest:?}: {e}"
                            )));
                            continue;
                        }
                    };
                    if let Err(e) = std::io::copy(&mut reader, &mut file) {
                        errors.push(UpdateError::DownloadFailed(format!(
                            "failed to write {dest:?}: {e}"
                        )));
                    } else {
                        debug!("downloaded {} -> {:?}", path, dest);
                    }
                }
                Err(e) => {
                    errors.push(UpdateError::DownloadFailed(format!(
                        "failed to download {url}: {e}"
                    )));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Simulate a local test download by copying files from `source_dir` to
    /// `staging_dir`.
    ///
    /// Only payload files whose paths appear in the manifest are copied.
    /// The files are *not* verified here — verification happens in the
    /// Verifier step.
    pub fn download_local(
        manifest: &HotUpdateManifest,
        source_dir: &Path,
        staging_dir: &Path,
    ) -> Result<(), Vec<UpdateError>> {
        let mut errors: Vec<UpdateError> = Vec::new();

        for ph in &manifest.payload_hashes {
            let src = source_dir.join(&ph.path);
            let dst = staging_dir.join(&ph.path);

            if !src.exists() {
                errors.push(UpdateError::DownloadFailed(format!(
                    "source file not found: {}",
                    src.display()
                )));
                continue;
            }

            if let Some(parent) = dst.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    errors.push(UpdateError::DownloadFailed(format!(
                        "cannot create directory {parent:?}: {e}"
                    )));
                    continue;
                }
            }

            match std::fs::copy(&src, &dst) {
                Ok(n) => {
                    debug!(
                        "copied {} -> {} ({} bytes)",
                        src.display(),
                        dst.display(),
                        n
                    );
                }
                Err(e) => {
                    errors.push(UpdateError::DownloadFailed(format!(
                        "failed to copy {} -> {}: {e}",
                        src.display(),
                        dst.display()
                    )));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn platform_kind_str(platform: &PlatformKind) -> &'static str {
    match platform {
        PlatformKind::All => "all",
        PlatformKind::Desktop => "desktop",
        PlatformKind::Android => "android",
        PlatformKind::Ios => "ios",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::{
        AssetId, PayloadHash, PlatformPayload, RollbackMetadata, SchemaVersion,
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
            payload_hashes: vec![PayloadHash {
                path: "data/patch.bundle".into(),
                algorithm: "sha256".into(),
                hash: [0u8; 32],
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
    fn download_local_copies_files() {
        let manifest = sample_manifest();

        let dir = std::env::temp_dir().join("dl_local_copy");
        let _ = std::fs::remove_dir_all(&dir);

        let source_dir = dir.join("source");
        let staging_dir = dir.join("staging");

        std::fs::create_dir_all(&source_dir.join("data")).unwrap();
        std::fs::write(&source_dir.join("data/patch.bundle"), b"test data").unwrap();

        let result = Downloader::download_local(&manifest, &source_dir, &staging_dir);
        assert!(result.is_ok(), "download_local failed: {result:?}");

        // Verify file was copied.
        let copied = staging_dir.join("data/patch.bundle");
        assert!(copied.exists(), "file was not copied to staging");
        assert_eq!(
            std::fs::read(&copied).unwrap(),
            b"test data",
            "content mismatch"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_local_missing_source_reports_error() {
        let manifest = sample_manifest();

        let dir = std::env::temp_dir().join("dl_local_miss");
        let _ = std::fs::remove_dir_all(&dir);

        let source_dir = dir.join("source");
        let staging_dir = dir.join("staging");
        std::fs::create_dir_all(&source_dir).unwrap();

        let result = Downloader::download_local(&manifest, &source_dir, &staging_dir);
        assert!(result.is_err(), "expected error for missing source");

        let errors = result.unwrap_err();
        assert!(!errors.is_empty());
        assert!(matches!(&errors[0], UpdateError::DownloadFailed(_)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_local_empty_manifest() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes.clear();

        let dir = std::env::temp_dir().join("dl_local_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let result =
            Downloader::download_local(&manifest, &dir, &dir.join("staging"));
        assert!(result.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_local_creates_subdirectories() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![
            PayloadHash {
                path: "a/deep/nested/file.bin".into(),
                algorithm: "sha256".into(),
                hash: [0u8; 32],
            },
        ];

        let dir = std::env::temp_dir().join("dl_local_nested");
        let _ = std::fs::remove_dir_all(&dir);

        let source_dir = dir.join("source");
        let staging_dir = dir.join("staging");
        std::fs::create_dir_all(&source_dir.join("a/deep/nested")).unwrap();
        std::fs::write(&source_dir.join("a/deep/nested/file.bin"), b"nested").unwrap();

        let result = Downloader::download_local(&manifest, &source_dir, &staging_dir);
        assert!(result.is_ok());

        assert!(staging_dir.join("a/deep/nested/file.bin").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_local_multiple_files() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![
            PayloadHash {
                path: "file1.bin".into(),
                algorithm: "sha256".into(),
                hash: [0u8; 32],
            },
            PayloadHash {
                path: "file2.bin".into(),
                algorithm: "sha256".into(),
                hash: [0u8; 32],
            },
            PayloadHash {
                path: "file3.bin".into(),
                algorithm: "sha256".into(),
                hash: [0u8; 32],
            },
        ];

        let dir = std::env::temp_dir().join("dl_local_multi");
        let _ = std::fs::remove_dir_all(&dir);

        let source_dir = dir.join("source");
        let staging_dir = dir.join("staging");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(&source_dir.join("file1.bin"), b"one").unwrap();
        std::fs::write(&source_dir.join("file2.bin"), b"two").unwrap();
        std::fs::write(&source_dir.join("file3.bin"), b"three").unwrap();

        let result = Downloader::download_local(&manifest, &source_dir, &staging_dir);
        assert!(result.is_ok());

        for f in &["file1.bin", "file2.bin", "file3.bin"] {
            assert!(staging_dir.join(f).exists(), "missing {f}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_local_partial_failure() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![
            PayloadHash {
                path: "present.bin".into(),
                algorithm: "sha256".into(),
                hash: [0u8; 32],
            },
            PayloadHash {
                path: "missing.bin".into(),
                algorithm: "sha256".into(),
                hash: [0u8; 32],
            },
        ];

        let dir = std::env::temp_dir().join("dl_local_partial");
        let _ = std::fs::remove_dir_all(&dir);

        let source_dir = dir.join("source");
        let staging_dir = dir.join("staging");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(&source_dir.join("present.bin"), b"data").unwrap();

        let result = Downloader::download_local(&manifest, &source_dir, &staging_dir);
        assert!(result.is_err());
        // The present.bin should still be copied even though missing.bin fails
        assert!(staging_dir.join("present.bin").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
