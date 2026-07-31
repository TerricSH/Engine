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
#[path = "verify_tests.rs"]
mod tests;
