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

include!("verify_tests/signatures.rs");
include!("verify_tests/hashes.rs");
include!("verify_tests/compatibility.rs");
include!("verify_tests/cooked.rs");
