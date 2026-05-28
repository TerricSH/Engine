use std::io::Read;
use std::path::Path;

use engine_serialize::{HotUpdateManifest, PlatformKind};
use tracing::{debug, warn};

use crate::error::UpdateError;
use crate::package::sha256_hash;

// ---------------------------------------------------------------------------
// Verifier
// ---------------------------------------------------------------------------

/// Verification pipeline for hot-update packages.
///
/// Verify runs after download and before staging.  It checks:
/// 1. Signature (placeholder — real crypto deferred to Gate 19).
/// 2. Payload hashes against the manifest.
/// 3. Engine & script API compatibility.
/// 4. Platform-specific rules (e.g. iOS → no assemblies).
/// 5. Cooked-asset header integrity for every `.cooked` payload.
pub struct Verifier;

impl Verifier {
    /// Run the full verification suite against a complete package.
    ///
    /// Returns `Ok(())` on success, or `Err(Vec<UpdateError>)` collecting all
    /// failures so the caller can inspect every problem at once.
    pub fn verify(
        manifest: &HotUpdateManifest,
        staged_dir: &Path,
        platform: &PlatformKind,
        engine_ver: &str,
        script_api_ver: (u16, u16),
    ) -> Result<(), Vec<UpdateError>> {
        let mut errors: Vec<UpdateError> = Vec::new();

        // 1. Signature (placeholder).
        if let Err(e) = Self::verify_signature(manifest) {
            errors.push(e);
        }

        // 2. Payload hashes.
        if let Err(mut hash_errors) = Self::verify_payload_hashes(manifest, staged_dir) {
            errors.append(&mut hash_errors);
        }

        // 3. Compatibility.
        if let Err(e) =
            Self::verify_compatibility(manifest, engine_ver, script_api_ver)
        {
            errors.push(e);
        }

        // 4. Platform rules.
        if let Err(e) = Self::verify_platform_rules(manifest, platform) {
            errors.push(e);
        }

        // 5. Cooked headers.
        if let Err(mut header_errors) = Self::verify_cooked_headers(manifest, staged_dir) {
            errors.append(&mut header_errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Verify the manifest signature.
    ///
    /// # Placeholder
    ///
    /// Real cryptographic signature verification is deferred to Gate 19.
    /// For v0 this method only blocks manifests that claim to have a
    /// signature (i.e. `signature.is_some()`) but where the algorithm is
    /// unrecognised.  Dev-mode manifests with no signature at all are
    /// accepted.
    pub fn verify_signature(
        manifest: &HotUpdateManifest,
    ) -> Result<(), UpdateError> {
        match &manifest.signature {
            None => {
                // No signature — dev mode; accept.
                debug!("manifest has no signature (dev mode, accepted)");
                Ok(())
            }
            Some(sig) => {
                match sig.algorithm.as_str() {
                    // Placeholder: accept any key length for now.
                    "ed25519" | "rsa-sha256" => {
                        debug!(
                            algorithm = %sig.algorithm,
                            key_id = %sig.key_id,
                            "manifest signature present (placeholder verify)"
                        );
                        Ok(())
                    }
                    other => Err(UpdateError::SignatureInvalid(format!(
                        "unrecognised signature algorithm '{other}'"
                    ))),
                }
            }
        }
    }

    /// Verify all payload hashes against the manifest.
    ///
    /// Reads every file listed in `payload_hashes` from `staged_dir` and
    /// checks its SHA-256 matches the manifest entry.
    pub fn verify_payload_hashes(
        manifest: &HotUpdateManifest,
        staged_dir: &Path,
    ) -> Result<(), Vec<UpdateError>> {
        let mut errors: Vec<UpdateError> = Vec::new();

        for ph in &manifest.payload_hashes {
            let file_path = staged_dir.join(&ph.path);
            let data = match std::fs::read(&file_path) {
                Ok(d) => d,
                Err(e) => {
                    errors.push(UpdateError::HashMismatch {
                        path: ph.path.clone(),
                        expected: ph.hash,
                        actual: [0u8; 32],
                    });
                    warn!("cannot read payload for hash verify: {file_path:?}: {e}");
                    continue;
                }
            };

            let computed = sha256_hash(&data);
            if computed != ph.hash {
                errors.push(UpdateError::HashMismatch {
                    path: ph.path.clone(),
                    expected: ph.hash,
                    actual: computed,
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Verify compatibility against current engine and script API versions.
    ///
    /// Delegates to [`HotUpdateManifest::check_compatibility`].
    pub fn verify_compatibility(
        manifest: &HotUpdateManifest,
        engine_ver: &str,
        script_api_ver: (u16, u16),
    ) -> Result<(), UpdateError> {
        // We use PlatformKind::All here because compatibility is about
        // engine/api versions, not platform payload rules (that's
        // verify_platform_rules).
        let result = manifest.check_compatibility(engine_ver, script_api_ver, PlatformKind::All);
        match result {
            engine_serialize::CompatibilityResult::Compatible => Ok(()),
            engine_serialize::CompatibilityResult::Incompatible { reasons } => {
                Err(UpdateError::IncompatibleVersion(reasons.join("; ")))
            }
        }
    }

    /// Verify platform-specific rules.
    ///
    /// - iOS: rejects any payload with `optional_assembly`.
    /// - Android: `optional_assembly` is allowed.
    /// - Desktop/All: assembly payloads are ignored.
    pub fn verify_platform_rules(
        manifest: &HotUpdateManifest,
        platform: &PlatformKind,
    ) -> Result<(), UpdateError> {
        let result = manifest.check_compatibility(
            &manifest.engine_version,
            manifest.script_api_version,
            platform.clone(),
        );
        match result {
            engine_serialize::CompatibilityResult::Compatible => {
                // Extra check: ensure iOS payloads never contain assemblies.
                if *platform == PlatformKind::Ios {
                    for payload in &manifest.platform_payloads {
                        if (payload.platform == PlatformKind::Ios
                            || payload.platform == PlatformKind::All)
                            && payload.optional_assembly.is_some()
                        {
                            return Err(UpdateError::PlatformRejected(
                                "iOS platform payload must not contain optional_assembly".into(),
                            ));
                        }
                    }
                }
                Ok(())
            }
            engine_serialize::CompatibilityResult::Incompatible { reasons } => {
                Err(UpdateError::PlatformRejected(reasons.join("; ")))
            }
        }
    }

    /// Verify that every cooked payload file has a valid
    /// [`CookedAssetHeader`] (per FD-006).
    ///
    /// Skips payloads whose file extension is not `.cooked`.  For each
    /// `.cooked` file the magic, header_version, and content_hash fields
    /// are validated.
    pub fn verify_cooked_headers(
        manifest: &HotUpdateManifest,
        staged_dir: &Path,
    ) -> Result<(), Vec<UpdateError>> {
        let mut errors: Vec<UpdateError> = Vec::new();

        for ph in &manifest.payload_hashes {
            let file_path = staged_dir.join(&ph.path);

            // Only verify .cooked files.
            if file_path.extension().and_then(|e| e.to_str()) != Some("cooked") {
                continue;
            }

            let mut file = match std::fs::File::open(&file_path) {
                Ok(f) => f,
                Err(e) => {
                    errors.push(UpdateError::CacheCorrupt(format!(
                        "cannot open cooked file {}: {e}",
                        ph.path
                    )));
                    continue;
                }
            };

            // Read enough for CookedAssetHeader (bincode-serialized).
            // The header is at most 256 bytes, we read a generous buffer.
            let mut buf = Vec::new();
            if let Err(e) = file.read_to_end(&mut buf) {
                errors.push(UpdateError::CacheCorrupt(format!(
                    "cannot read cooked file {}: {e}",
                    ph.path
                )));
                continue;
            }

            let header: engine_asset::cook::CookedAssetHeader =
                match bincode::deserialize(&buf) {
                    Ok(h) => h,
                    Err(e) => {
                        errors.push(UpdateError::CacheCorrupt(format!(
                            "invalid cooked header in {}: {e}",
                            ph.path
                        )));
                        continue;
                    }
                };

            if !header.is_valid() {
                errors.push(UpdateError::CacheCorrupt(format!(
                    "cooked file {} has invalid header (bad magic or version)",
                    ph.path
                )));
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

/// Verify that the data at `staged_dir` matches the manifest's payload
/// hashes for the given platform.  Returns the list of payload paths that
/// exist on disk.
pub(crate) fn verify_and_collect_payloads(
    manifest: &HotUpdateManifest,
    staged_dir: &Path,
) -> Result<Vec<String>, Vec<UpdateError>> {
    let mut errors = Vec::new();
    let mut present = Vec::new();

    for ph in &manifest.payload_hashes {
        let file_path = staged_dir.join(&ph.path);
        if !file_path.exists() {
            errors.push(UpdateError::HashMismatch {
                path: ph.path.clone(),
                expected: ph.hash,
                actual: [0u8; 32],
            });
            continue;
        }

        let data = match std::fs::read(&file_path) {
            Ok(d) => d,
            Err(e) => {
                errors.push(UpdateError::CacheCorrupt(format!(
                    "cannot read {}: {e}",
                    ph.path
                )));
                continue;
            }
        };

        let computed = sha256_hash(&data);
        if computed != ph.hash {
            errors.push(UpdateError::HashMismatch {
                path: ph.path.clone(),
                expected: ph.hash,
                actual: computed,
            });
        } else {
            present.push(ph.path.clone());
        }
    }

        if errors.is_empty() {
            Ok(present)
        } else {
            Err(errors)
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::{
        AssetId, HashDigest, HotUpdateManifest, ManifestSignature, PayloadHash, PlatformPayload,
        RollbackMetadata, SchemaVersion,
    };
    use sha2::{Digest, Sha256};

    // ── Helpers ───────────────────────────────────────────────────────────

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
            payload_hashes: vec![],
            signature: None,
            rollback: RollbackMetadata {
                previous_manifest_hash: None,
                fallback_manifest_path: None,
                min_safe_engine_version: "1.4.0".into(),
            },
            created_at: "2026-05-29T12:00:00Z".into(),
        }
    }

    fn create_temp_payload(dir: &std::path::Path, rel: &str, data: &[u8]) -> std::path::PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, data).unwrap();
        path
    }

    // ── Signature tests ─────────────────────────────────────────────────

    #[test]
    fn verify_signature_none_accepted() {
        let manifest = sample_manifest();
        assert!(Verifier::verify_signature(&manifest).is_ok());
    }

    #[test]
    fn verify_signature_ed25519_accepted() {
        let mut manifest = sample_manifest();
        manifest.signature = Some(ManifestSignature {
            algorithm: "ed25519".into(),
            value: vec![0u8; 64],
            key_id: "key-01".into(),
            signed_at: "2026-05-29T12:00:00Z".into(),
        });
        assert!(Verifier::verify_signature(&manifest).is_ok());
    }

    #[test]
    fn verify_signature_rsa_accepted() {
        let mut manifest = sample_manifest();
        manifest.signature = Some(ManifestSignature {
            algorithm: "rsa-sha256".into(),
            value: vec![0u8; 256],
            key_id: "key-02".into(),
            signed_at: "2026-05-29T12:00:00Z".into(),
        });
        assert!(Verifier::verify_signature(&manifest).is_ok());
    }

    #[test]
    fn verify_signature_unknown_algorithm_rejected() {
        let mut manifest = sample_manifest();
        manifest.signature = Some(ManifestSignature {
            algorithm: "hmac-sha1".into(),
            value: vec![0u8; 20],
            key_id: "key-03".into(),
            signed_at: "2026-05-29T12:00:00Z".into(),
        });
        assert!(Verifier::verify_signature(&manifest).is_err());
    }

    // ── Payload hash tests ──────────────────────────────────────────────

    #[test]
    fn verify_payload_hashes_all_match() {
        let mut manifest = sample_manifest();
        let data = b"hello payload";
        let hash: HashDigest = Sha256::digest(data).into();

        manifest.payload_hashes = vec![PayloadHash {
            path: "patch.bundle".into(),
            algorithm: "sha256".into(),
            hash,
        }];

        let dir = std::env::temp_dir().join("verify_hash_ok");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "patch.bundle", data);

        assert!(Verifier::verify_payload_hashes(&manifest, &dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_payload_hashes_mismatch() {
        let mut manifest = sample_manifest();
        let data = b"hello payload";
        let hash: HashDigest = Sha256::digest(data).into();

        manifest.payload_hashes = vec![PayloadHash {
            path: "patch.bundle".into(),
            algorithm: "sha256".into(),
            hash,
        }];

        let dir = std::env::temp_dir().join("verify_hash_bad");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "patch.bundle", b"tampered data");

        let result = Verifier::verify_payload_hashes(&manifest, &dir);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_payload_hashes_missing_file() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![PayloadHash {
            path: "missing.bundle".into(),
            algorithm: "sha256".into(),
            hash: [0u8; 32],
        }];

        let dir = std::env::temp_dir().join("verify_hash_miss");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let result = Verifier::verify_payload_hashes(&manifest, &dir);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_payload_hashes_multiple_errors() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![
            PayloadHash {
                path: "a.bundle".into(),
                algorithm: "sha256".into(),
                hash: [1u8; 32],
            },
            PayloadHash {
                path: "b.bundle".into(),
                algorithm: "sha256".into(),
                hash: [2u8; 32],
            },
        ];

        let dir = std::env::temp_dir().join("verify_hash_multi");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "a.bundle", b"data");

        let result = Verifier::verify_payload_hashes(&manifest, &dir);
        assert!(result.is_err());
        // Should have at least one error (b.bundle missing)
        assert!(result.unwrap_err().len() >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Compatibility tests ─────────────────────────────────────────────

    #[test]
    fn verify_compatibility_accepts() {
        let manifest = sample_manifest();
        assert!(Verifier::verify_compatibility(&manifest, "1.5.0", (1, 5)).is_ok());
    }

    #[test]
    fn verify_compatibility_rejects_engine_mismatch() {
        let manifest = sample_manifest();
        let result = Verifier::verify_compatibility(&manifest, "2.0.0", (1, 5));
        assert!(result.is_err());
        assert!(matches!(result, Err(UpdateError::IncompatibleVersion(_))));
    }

    #[test]
    fn verify_compatibility_rejects_script_api() {
        let mut manifest = sample_manifest();
        manifest.script_api_version = (5, 0);
        let result = Verifier::verify_compatibility(&manifest, "1.5.0", (1, 5));
        assert!(result.is_err());
    }

    // ── Platform rule tests ─────────────────────────────────────────────

    #[test]
    fn verify_platform_rules_desktop_accepted() {
        let manifest = sample_manifest();
        assert!(Verifier::verify_platform_rules(&manifest, &PlatformKind::Desktop).is_ok());
    }

    #[test]
    fn verify_platform_rules_ios_rejects_assemblies() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads.push(PlatformPayload {
            platform: PlatformKind::Ios,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(engine_serialize::AssemblyPayload {
                path: "ios/asm.dll".into(),
                size_bytes: 100,
                hash: [0xBB; 32],
                min_engine_version: "1.5.0".into(),
            }),
        });
        let result = Verifier::verify_platform_rules(&manifest, &PlatformKind::Ios);
        assert!(result.is_err());
        assert!(matches!(result, Err(UpdateError::PlatformRejected(_))));
    }

    #[test]
    fn verify_platform_rules_android_allows_assemblies() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads.push(PlatformPayload {
            platform: PlatformKind::Android,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(engine_serialize::AssemblyPayload {
                path: "android/asm.dll".into(),
                size_bytes: 100,
                hash: [0xCC; 32],
                min_engine_version: "1.5.0".into(),
            }),
        });
        assert!(Verifier::verify_platform_rules(&manifest, &PlatformKind::Android).is_ok());
    }

    #[test]
    fn verify_platform_rules_all_platform_no_assembly() {
        let manifest = sample_manifest();
        assert!(Verifier::verify_platform_rules(&manifest, &PlatformKind::All).is_ok());
    }

    // ── Cooked header tests ─────────────────────────────────────────────

    #[test]
    fn verify_cooked_headers_skips_non_cooked() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![PayloadHash {
            path: "data.bin".into(),
            algorithm: "sha256".into(),
            hash: [0u8; 32],
        }];

        let dir = std::env::temp_dir().join("verify_cooked_skip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "data.bin", b"not a cooked file");

        // Should pass because we skip non-.cooked files.
        assert!(Verifier::verify_cooked_headers(&manifest, &dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_cooked_headers_valid() {
        use engine_asset::cook::write_cooked_artifact;
        use engine_serialize::SchemaVersion;

        let mut manifest = sample_manifest();
        let hash: HashDigest = Sha256::digest(b"payload data").into();
        manifest.payload_hashes = vec![PayloadHash {
            path: "asset.cooked".into(),
            algorithm: "sha256".into(),
            hash,
        }];

        let dir = std::env::temp_dir().join("verify_cooked_ok");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Write a valid cooked artifact.
        write_cooked_artifact(
            &dir.join("asset.cooked"),
            1,
            b"payload data",
            SchemaVersion::new(0, 1, 0),
        )
        .unwrap();

        assert!(Verifier::verify_cooked_headers(&manifest, &dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_cooked_headers_invalid_magic() {
        let mut manifest = sample_manifest();
        let hash: HashDigest = Sha256::digest(b"bad data").into();
        manifest.payload_hashes = vec![PayloadHash {
            path: "bad.cooked".into(),
            algorithm: "sha256".into(),
            hash,
        }];

        let dir = std::env::temp_dir().join("verify_cooked_bad");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Write garbage instead of a valid cooked file.
        create_temp_payload(&dir, "bad.cooked", b"garbage data");

        let result = Verifier::verify_cooked_headers(&manifest, &dir);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Full verify tests ───────────────────────────────────────────────

    #[test]
    fn verify_full_pipeline_accepts_valid_package() {
        use engine_asset::cook::write_cooked_artifact;
        use engine_serialize::SchemaVersion;
        use sha2::{Digest, Sha256};

        let mut manifest = sample_manifest();
        let payload_data = b"cooked content";

        let dir = std::env::temp_dir().join("verify_full_ok");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_cooked_artifact(
            &dir.join("mesh.cooked"),
            1,
            payload_data,
            SchemaVersion::new(0, 1, 0),
        )
        .unwrap();

        // Hash must be computed from the entire written file (header + payload).
        let file_data = std::fs::read(&dir.join("mesh.cooked")).unwrap();
        let hash: HashDigest = Sha256::digest(&file_data).into();

        manifest.payload_hashes = vec![PayloadHash {
            path: "mesh.cooked".into(),
            algorithm: "sha256".into(),
            hash,
        }];

        let result = Verifier::verify(
            &manifest,
            &dir,
            &PlatformKind::Desktop,
            "1.5.0",
            (1, 5),
        );
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_full_pipeline_rejects_bad_hash() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![PayloadHash {
            path: "data.bin".into(),
            algorithm: "sha256".into(),
            hash: [0xAA; 32],
        }];

        let dir = std::env::temp_dir().join("verify_full_bad");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "data.bin", b"does not match");

        let result = Verifier::verify(
            &manifest,
            &dir,
            &PlatformKind::Desktop,
            "1.5.0",
            (1, 5),
        );
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
