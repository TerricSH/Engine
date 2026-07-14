use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::Path;

use engine_serialize::{HotUpdateManifest, ManifestSignature, PlatformKind};
use ring::signature;
use tracing::{debug, warn};

use crate::error::UpdateError;
use crate::package::sha256_hash;
use crate::path_safety::{safe_join, validate_manifest_paths};

// ---------------------------------------------------------------------------
// Verifier
// ---------------------------------------------------------------------------

/// Domain separator for the v2 manifest signing format.
///
/// It prevents a valid manifest signature from being reused for a different
/// protocol that happens to serialize the same bytes. V2 adds the platform
/// field to each payload hash; v1 signatures must therefore be reissued.
const MANIFEST_SIGNATURE_DOMAIN_V2: &[u8] = b"engine-hot-update/manifest-signature/v2\0";

/// Policy governing whether a manifest may omit its signature.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SignaturePolicy {
    /// Production-safe default: every manifest must have a valid signature
    /// from a configured trusted Ed25519 key.
    #[default]
    Production,
    /// Development mode: unsigned manifests are accepted.
    ///
    /// Manifests that do contain a signature are still verified strictly.
    Development,
    /// Explicitly allow unsigned manifests outside the named development
    /// mode. Signed manifests are still verified strictly.
    AllowUnsigned,
}

impl SignaturePolicy {
    fn allows_unsigned(self) -> bool {
        matches!(self, Self::Development | Self::AllowUnsigned)
    }
}

/// Return the stable bytes covered by a hot-update manifest signature.
///
/// The format is the v2 domain separator followed by compact UTF-8 JSON of a
/// cloned manifest whose `signature` field is set to `None`. The manifest
/// schema contains ordered structs and vectors, so serde's struct field order
/// makes this deterministic. Any future signing-format change must use a new
/// domain/version instead of silently changing these bytes.
pub fn canonical_manifest_bytes(manifest: &HotUpdateManifest) -> Result<Vec<u8>, UpdateError> {
    let mut unsigned = manifest.clone();
    unsigned.signature = None;

    let json =
        serde_json::to_vec(&unsigned).map_err(|_| UpdateError::ManifestCanonicalizationFailed)?;
    let mut canonical = Vec::with_capacity(MANIFEST_SIGNATURE_DOMAIN_V2.len() + json.len());
    canonical.extend_from_slice(MANIFEST_SIGNATURE_DOMAIN_V2);
    canonical.extend_from_slice(&json);
    Ok(canonical)
}

/// Sign a manifest with an Ed25519 PKCS#8 private key for packaging tools.
///
/// The private key is never retained and parse failures intentionally do not
/// include key material in the returned error.
pub fn sign_manifest_ed25519(
    manifest: &mut HotUpdateManifest,
    key_id: impl Into<String>,
    signed_at: impl Into<String>,
    private_key_pkcs8: &[u8],
) -> Result<(), UpdateError> {
    let key_id = key_id.into();
    if key_id.is_empty() {
        return Err(UpdateError::TrustedKeyInvalid { key_id });
    }

    let key_pair = signature::Ed25519KeyPair::from_pkcs8(private_key_pkcs8)
        .map_err(|_| UpdateError::SigningKeyInvalid)?;
    let canonical = canonical_manifest_bytes(manifest)?;
    let signature = key_pair.sign(&canonical);
    manifest.signature = Some(ManifestSignature {
        algorithm: "ed25519".into(),
        value: signature.as_ref().to_vec(),
        key_id,
        signed_at: signed_at.into(),
    });
    Ok(())
}

/// Verification pipeline for hot-update packages.
///
/// Verify runs after download and before staging. It checks the configured
/// Ed25519 signature policy, then payload integrity and compatibility.
#[derive(Clone)]
pub struct Verifier {
    policy: SignaturePolicy,
    trusted_ed25519_keys: BTreeMap<String, [u8; 32]>,
}

impl fmt::Debug for Verifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Verifier")
            .field("policy", &self.policy)
            .field(
                "trusted_key_ids",
                &self.trusted_ed25519_keys.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Default for Verifier {
    fn default() -> Self {
        Self::production()
    }
}

impl Verifier {
    /// Create a verifier with the production-safe signature policy.
    pub fn production() -> Self {
        Self::new(SignaturePolicy::Production)
    }

    /// Create a verifier that explicitly allows unsigned development packages.
    pub fn development() -> Self {
        Self::new(SignaturePolicy::Development)
    }

    /// Create a verifier with an explicit signature policy.
    pub fn new(policy: SignaturePolicy) -> Self {
        Self {
            policy,
            trusted_ed25519_keys: BTreeMap::new(),
        }
    }

    /// Return the configured signature policy.
    pub fn signature_policy(&self) -> SignaturePolicy {
        self.policy
    }

    /// Add or rotate a trusted Ed25519 public key under `key_id`.
    pub fn trust_ed25519_key(
        &mut self,
        key_id: impl Into<String>,
        public_key: &[u8],
    ) -> Result<(), UpdateError> {
        let key_id = key_id.into();
        let key = <[u8; 32]>::try_from(public_key).map_err(|_| UpdateError::TrustedKeyInvalid {
            key_id: key_id.clone(),
        })?;
        if key_id.is_empty() {
            return Err(UpdateError::TrustedKeyInvalid { key_id });
        }
        self.trusted_ed25519_keys.insert(key_id, key);
        Ok(())
    }

    /// Builder-style trusted-key registration.
    pub fn with_trusted_ed25519_key(
        mut self,
        key_id: impl Into<String>,
        public_key: &[u8],
    ) -> Result<Self, UpdateError> {
        self.trust_ed25519_key(key_id, public_key)?;
        Ok(self)
    }

    /// Run the full verification suite against a complete package.
    ///
    /// Returns `Ok(())` on success, or `Err(Vec<UpdateError>)` collecting all
    /// failures so the caller can inspect every problem at once.
    pub fn verify(
        &self,
        manifest: &HotUpdateManifest,
        staged_dir: &Path,
        platform: &PlatformKind,
        engine_ver: &str,
        script_api_ver: (u16, u16),
    ) -> Result<(), Vec<UpdateError>> {
        // Reject unsafe manifest-controlled paths before any file is opened.
        validate_manifest_paths(manifest)?;
        let mut errors: Vec<UpdateError> = Vec::new();

        // 1. Signature and trust policy.
        if let Err(e) = self.verify_signature(manifest) {
            errors.push(e);
        }

        // 2. Payload hashes.
        if let Err(mut hash_errors) = Self::verify_payload_hashes(manifest, staged_dir, platform) {
            errors.append(&mut hash_errors);
        }

        // 3. Compatibility.
        if let Err(e) = Self::verify_compatibility(manifest, engine_ver, script_api_ver) {
            errors.push(e);
        }

        // 4. Platform rules.
        if let Err(e) = Self::verify_platform_rules(manifest, platform) {
            errors.push(e);
        }

        // 5. Cooked headers.
        if let Err(mut header_errors) = Self::verify_cooked_headers(manifest, staged_dir, platform)
        {
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
    /// Unsigned manifests are rejected unless the verifier was explicitly
    /// constructed with a policy that permits them. A present signature can
    /// never bypass cryptographic verification, even in development mode.
    pub fn verify_signature(&self, manifest: &HotUpdateManifest) -> Result<(), UpdateError> {
        match &manifest.signature {
            None if self.policy.allows_unsigned() => {
                debug!(policy = ?self.policy, "unsigned manifest accepted by explicit policy");
                Ok(())
            }
            None => Err(UpdateError::SignatureMissing),
            Some(sig) => {
                if sig.algorithm != "ed25519" {
                    return Err(UpdateError::SignatureUnsupportedAlgorithm {
                        algorithm: sig.algorithm.clone(),
                    });
                }

                let public_key = self.trusted_ed25519_keys.get(&sig.key_id).ok_or_else(|| {
                    UpdateError::SignatureUnknownKey {
                        key_id: sig.key_id.clone(),
                    }
                })?;
                let canonical = canonical_manifest_bytes(manifest)?;
                signature::UnparsedPublicKey::new(&signature::ED25519, public_key)
                    .verify(&canonical, &sig.value)
                    .map_err(|_| UpdateError::SignatureInvalid {
                        key_id: sig.key_id.clone(),
                    })?;
                debug!(key_id = %sig.key_id, "manifest Ed25519 signature verified");
                Ok(())
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
        platform: &PlatformKind,
    ) -> Result<(), Vec<UpdateError>> {
        validate_manifest_paths(manifest)?;
        let selected = manifest.payload_hashes_for_platform(*platform);
        let mut prepared = Vec::with_capacity(selected.len());
        for payload in selected {
            if !payload.algorithm.eq_ignore_ascii_case("sha256") {
                return Err(vec![UpdateError::ManifestRejected(format!(
                    "unsupported payload hash algorithm `{}` for {}",
                    payload.algorithm, payload.path
                ))]);
            }
            match safe_join(staged_dir, &payload.path, "payload hash verification") {
                Ok(path) => prepared.push((payload, path)),
                Err(error) => return Err(vec![error]),
            }
        }

        let mut errors: Vec<UpdateError> = Vec::new();

        for (ph, file_path) in prepared {
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
            *platform,
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
                let selected_hashes = manifest.payload_hashes_for_platform(*platform);
                for payload in manifest.payloads_for_platform(*platform) {
                    let Some(assembly) = &payload.optional_assembly else {
                        continue;
                    };
                    let Some(payload_hash) = selected_hashes
                        .iter()
                        .find(|entry| entry.path == assembly.path)
                    else {
                        return Err(UpdateError::PlatformRejected(format!(
                            "selected assembly `{}` has no selected payload hash",
                            assembly.path
                        )));
                    };
                    if !payload_hash.algorithm.eq_ignore_ascii_case("sha256") {
                        return Err(UpdateError::PlatformRejected(format!(
                            "selected assembly `{}` must use sha256",
                            assembly.path
                        )));
                    }
                    if payload_hash.hash != assembly.hash {
                        return Err(UpdateError::PlatformRejected(format!(
                            "selected assembly `{}` hash disagrees with its payload hash",
                            assembly.path
                        )));
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
        platform: &PlatformKind,
    ) -> Result<(), Vec<UpdateError>> {
        validate_manifest_paths(manifest)?;
        let selected = manifest.payload_hashes_for_platform(*platform);
        let mut prepared = Vec::with_capacity(selected.len());
        for payload in selected {
            match safe_join(staged_dir, &payload.path, "cooked payload verification") {
                Ok(path) => prepared.push((payload, path)),
                Err(error) => return Err(vec![error]),
            }
        }

        let mut errors: Vec<UpdateError> = Vec::new();

        for (ph, file_path) in prepared {
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

            let header: engine_asset::cook::CookedAssetHeader = match bincode::deserialize(&buf) {
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
#[expect(dead_code)]
pub(crate) fn verify_and_collect_payloads(
    manifest: &HotUpdateManifest,
    staged_dir: &Path,
    platform: &PlatformKind,
) -> Result<Vec<String>, Vec<UpdateError>> {
    validate_manifest_paths(manifest)?;
    let selected = manifest.payload_hashes_for_platform(*platform);
    let mut prepared = Vec::with_capacity(selected.len());
    for payload in selected {
        match safe_join(staged_dir, &payload.path, "payload collection") {
            Ok(path) => prepared.push((payload, path)),
            Err(error) => return Err(vec![error]),
        }
    }

    let mut errors = Vec::new();
    let mut present = Vec::new();

    for (ph, file_path) in prepared {
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
    use ring::signature::KeyPair;
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

    fn signing_key() -> (ring::pkcs8::Document, Vec<u8>) {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let key_pair = signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        (pkcs8, key_pair.public_key().as_ref().to_vec())
    }

    fn signed_manifest(key_id: &str) -> (HotUpdateManifest, Verifier) {
        let (private_key, public_key) = signing_key();
        let mut manifest = sample_manifest();
        sign_manifest_ed25519(
            &mut manifest,
            key_id,
            "2026-05-29T12:00:00Z",
            private_key.as_ref(),
        )
        .unwrap();
        let verifier = Verifier::production()
            .with_trusted_ed25519_key(key_id, &public_key)
            .unwrap();
        (manifest, verifier)
    }

    #[test]
    fn unsigned_manifest_is_rejected_in_production() {
        let error = Verifier::production()
            .verify_signature(&sample_manifest())
            .unwrap_err();
        assert!(matches!(error, UpdateError::SignatureMissing));
    }

    #[test]
    fn unsigned_manifest_is_accepted_only_by_explicit_relaxed_policies() {
        let manifest = sample_manifest();
        assert!(Verifier::development().verify_signature(&manifest).is_ok());
        assert!(Verifier::new(SignaturePolicy::AllowUnsigned)
            .verify_signature(&manifest)
            .is_ok());
    }

    #[test]
    fn valid_ed25519_signature_is_accepted() {
        let (manifest, verifier) = signed_manifest("release-2026");
        assert!(verifier.verify_signature(&manifest).is_ok());
    }

    #[test]
    fn tampered_manifest_is_rejected() {
        let (mut manifest, verifier) = signed_manifest("release-2026");
        manifest.engine_version = "1.5.1".into();
        let error = verifier.verify_signature(&manifest).unwrap_err();
        assert!(matches!(error, UpdateError::SignatureInvalid { .. }));
    }

    #[test]
    fn signature_from_wrong_key_is_rejected() {
        let (manifest, _) = signed_manifest("release-2026");
        let (_, wrong_public_key) = signing_key();
        let verifier = Verifier::production()
            .with_trusted_ed25519_key("release-2026", &wrong_public_key)
            .unwrap();
        let error = verifier.verify_signature(&manifest).unwrap_err();
        assert!(matches!(error, UpdateError::SignatureInvalid { .. }));
    }

    #[test]
    fn unknown_key_id_is_rejected() {
        let (manifest, _) = signed_manifest("release-2026");
        let error = Verifier::production()
            .verify_signature(&manifest)
            .unwrap_err();
        assert!(matches!(
            error,
            UpdateError::SignatureUnknownKey { ref key_id } if key_id == "release-2026"
        ));
    }

    #[test]
    fn bad_signature_is_rejected_even_in_development() {
        let (mut manifest, production) = signed_manifest("release-2026");
        manifest.signature.as_mut().unwrap().value[0] ^= 0xff;
        let mut development = Verifier::development();
        development
            .trust_ed25519_key(
                "release-2026",
                &production.trusted_ed25519_keys["release-2026"],
            )
            .unwrap();
        let error = development.verify_signature(&manifest).unwrap_err();
        assert!(matches!(error, UpdateError::SignatureInvalid { .. }));
    }

    #[test]
    fn unsupported_algorithm_is_rejected() {
        let mut manifest = sample_manifest();
        manifest.signature = Some(ManifestSignature {
            algorithm: "rsa-sha256".into(),
            value: vec![0u8; 256],
            key_id: "key-02".into(),
            signed_at: "2026-05-29T12:00:00Z".into(),
        });
        let error = Verifier::development()
            .verify_signature(&manifest)
            .unwrap_err();
        assert!(matches!(
            error,
            UpdateError::SignatureUnsupportedAlgorithm { ref algorithm }
                if algorithm == "rsa-sha256"
        ));
    }

    #[test]
    fn canonical_bytes_are_stable_and_ignore_signature_metadata() {
        let mut manifest = sample_manifest();
        let first = canonical_manifest_bytes(&manifest).unwrap();
        let second = canonical_manifest_bytes(&manifest).unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with(MANIFEST_SIGNATURE_DOMAIN_V2));
        assert_eq!(
            crate::package::hex_encode(&Sha256::digest(&first)),
            "64de56d94b403f7cd9f15ea686ce47874c09dca033a92877c1d174419ca29d67"
        );

        manifest.signature = Some(ManifestSignature {
            algorithm: "anything".into(),
            value: vec![1, 2, 3],
            key_id: "anything".into(),
            signed_at: "anything".into(),
        });
        assert_eq!(first, canonical_manifest_bytes(&manifest).unwrap());
    }

    // ── Payload hash tests ──────────────────────────────────────────────

    #[test]
    fn verify_payload_hashes_all_match() {
        let mut manifest = sample_manifest();
        let data = b"hello payload";
        let hash: HashDigest = Sha256::digest(data).into();

        manifest.payload_hashes = vec![PayloadHash {
            platform: PlatformKind::Desktop,
            path: "patch.bundle".into(),
            algorithm: "sha256".into(),
            hash,
        }];

        let dir = std::env::temp_dir().join("verify_hash_ok");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "patch.bundle", data);

        assert!(Verifier::verify_payload_hashes(&manifest, &dir, &PlatformKind::Desktop).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_payload_hashes_ignores_missing_other_platform_but_checks_all() {
        let mut manifest = sample_manifest();
        let desktop_data = b"desktop";
        let common_data = b"common";
        manifest.payload_hashes = vec![
            PayloadHash {
                platform: PlatformKind::Desktop,
                path: "desktop.bin".into(),
                algorithm: "sha256".into(),
                hash: Sha256::digest(desktop_data).into(),
            },
            PayloadHash {
                platform: PlatformKind::Android,
                path: "android-missing.bin".into(),
                algorithm: "sha256".into(),
                hash: [7; 32],
            },
            PayloadHash {
                platform: PlatformKind::All,
                path: "common.bin".into(),
                algorithm: "sha256".into(),
                hash: Sha256::digest(common_data).into(),
            },
        ];
        let temp = tempfile::tempdir().unwrap();
        create_temp_payload(temp.path(), "desktop.bin", desktop_data);
        create_temp_payload(temp.path(), "common.bin", common_data);

        assert!(
            Verifier::verify_payload_hashes(&manifest, temp.path(), &PlatformKind::Desktop,)
                .is_ok()
        );
        assert!(
            Verifier::verify_payload_hashes(&manifest, temp.path(), &PlatformKind::Android,)
                .is_err()
        );
    }

    #[test]
    fn verify_payload_hashes_mismatch() {
        let mut manifest = sample_manifest();
        let data = b"hello payload";
        let hash: HashDigest = Sha256::digest(data).into();

        manifest.payload_hashes = vec![PayloadHash {
            platform: PlatformKind::Desktop,
            path: "patch.bundle".into(),
            algorithm: "sha256".into(),
            hash,
        }];

        let dir = std::env::temp_dir().join("verify_hash_bad");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "patch.bundle", b"tampered data");

        let result = Verifier::verify_payload_hashes(&manifest, &dir, &PlatformKind::Desktop);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_payload_hashes_missing_file() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![PayloadHash {
            platform: PlatformKind::Desktop,
            path: "missing.bundle".into(),
            algorithm: "sha256".into(),
            hash: [0u8; 32],
        }];

        let dir = std::env::temp_dir().join("verify_hash_miss");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let result = Verifier::verify_payload_hashes(&manifest, &dir, &PlatformKind::Desktop);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_payload_hashes_multiple_errors() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![
            PayloadHash {
                platform: PlatformKind::Desktop,
                path: "a.bundle".into(),
                algorithm: "sha256".into(),
                hash: [1u8; 32],
            },
            PayloadHash {
                platform: PlatformKind::Desktop,
                path: "b.bundle".into(),
                algorithm: "sha256".into(),
                hash: [2u8; 32],
            },
        ];

        let dir = std::env::temp_dir().join("verify_hash_multi");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "a.bundle", b"data");

        let result = Verifier::verify_payload_hashes(&manifest, &dir, &PlatformKind::Desktop);
        assert!(result.is_err());
        // Should have at least one error (b.bundle missing)
        assert!(!result.unwrap_err().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_rejects_payload_traversal_before_reading_outside_stage() {
        let mut manifest = sample_manifest();
        manifest.payload_hashes = vec![PayloadHash {
            platform: PlatformKind::Desktop,
            path: "../outside.bin".into(),
            algorithm: "sha256".into(),
            hash: [0u8; 32],
        }];
        let temp = tempfile::tempdir().unwrap();
        let staged = temp.path().join("staged");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(temp.path().join("outside.bin"), b"outside").unwrap();

        let errors = Verifier::verify_payload_hashes(&manifest, &staged, &PlatformKind::Desktop)
            .unwrap_err();
        assert!(matches!(errors[0], UpdateError::UnsafePath { .. }));
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
        manifest.payload_hashes.push(PayloadHash {
            platform: PlatformKind::Android,
            path: "android/asm.dll".into(),
            algorithm: "sha256".into(),
            hash: [0xCC; 32],
        });
        assert!(Verifier::verify_platform_rules(&manifest, &PlatformKind::Android).is_ok());
    }

    #[test]
    fn verify_platform_rules_rejects_selected_assembly_without_selected_hash() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads.push(PlatformPayload {
            platform: PlatformKind::Android,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(engine_serialize::AssemblyPayload {
                path: "android/unhashed.dll".into(),
                size_bytes: 100,
                hash: [0xDD; 32],
                min_engine_version: "1.5.0".into(),
            }),
        });

        let error = Verifier::verify_platform_rules(&manifest, &PlatformKind::Android).unwrap_err();
        assert!(
            matches!(error, UpdateError::PlatformRejected(message) if message.contains("no selected payload hash"))
        );
    }

    #[test]
    fn verify_platform_rules_rejects_assembly_hash_disagreement() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads.push(PlatformPayload {
            platform: PlatformKind::Android,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(engine_serialize::AssemblyPayload {
                path: "android/mismatch.dll".into(),
                size_bytes: 100,
                hash: [0xDD; 32],
                min_engine_version: "1.5.0".into(),
            }),
        });
        manifest.payload_hashes.push(PayloadHash {
            platform: PlatformKind::Android,
            path: "android/mismatch.dll".into(),
            algorithm: "sha256".into(),
            hash: [0xEE; 32],
        });

        let error = Verifier::verify_platform_rules(&manifest, &PlatformKind::Android).unwrap_err();
        assert!(
            matches!(error, UpdateError::PlatformRejected(message) if message.contains("disagrees"))
        );
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
            platform: PlatformKind::Desktop,
            path: "data.bin".into(),
            algorithm: "sha256".into(),
            hash: [0u8; 32],
        }];

        let dir = std::env::temp_dir().join("verify_cooked_skip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "data.bin", b"not a cooked file");

        // Should pass because we skip non-.cooked files.
        assert!(Verifier::verify_cooked_headers(&manifest, &dir, &PlatformKind::Desktop).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_cooked_headers_valid() {
        use engine_asset::cook::write_cooked_artifact;
        use engine_serialize::SchemaVersion;

        let mut manifest = sample_manifest();
        let hash: HashDigest = Sha256::digest(b"payload data").into();
        manifest.payload_hashes = vec![PayloadHash {
            platform: PlatformKind::Desktop,
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

        assert!(Verifier::verify_cooked_headers(&manifest, &dir, &PlatformKind::Desktop).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_cooked_headers_invalid_magic() {
        let mut manifest = sample_manifest();
        let hash: HashDigest = Sha256::digest(b"bad data").into();
        manifest.payload_hashes = vec![PayloadHash {
            platform: PlatformKind::Desktop,
            path: "bad.cooked".into(),
            algorithm: "sha256".into(),
            hash,
        }];

        let dir = std::env::temp_dir().join("verify_cooked_bad");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Write garbage instead of a valid cooked file.
        create_temp_payload(&dir, "bad.cooked", b"garbage data");

        let result = Verifier::verify_cooked_headers(&manifest, &dir, &PlatformKind::Desktop);
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
        let file_data = std::fs::read(dir.join("mesh.cooked")).unwrap();
        let hash: HashDigest = Sha256::digest(&file_data).into();

        manifest.payload_hashes = vec![PayloadHash {
            platform: PlatformKind::Desktop,
            path: "mesh.cooked".into(),
            algorithm: "sha256".into(),
            hash,
        }];

        let result = Verifier::development().verify(
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
            platform: PlatformKind::Desktop,
            path: "data.bin".into(),
            algorithm: "sha256".into(),
            hash: [0xAA; 32],
        }];

        let dir = std::env::temp_dir().join("verify_full_bad");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_temp_payload(&dir, "data.bin", b"does not match");

        let result = Verifier::development().verify(
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
