use super::*;
use engine_serialize::{AssetId, PayloadHash, PlatformPayload, RollbackMetadata, SchemaVersion};
use ring::signature::KeyPair;
use sha2::{Digest, Sha256};

fn sample_manifest() -> HotUpdateManifest {
    let data = b"test payload";
    let hash: [u8; 32] = Sha256::digest(data).into();
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
            platform: PlatformKind::Desktop,
            path: "data.bin".into(),
            algorithm: "sha256".into(),
            hash,
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

fn setup_manager() -> (PackageManager, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let manager =
        PackageManager::new_development(tmp.path(), PlatformKind::Desktop, "1.5.0", (1, 5))
            .unwrap();
    (manager, tmp)
}

// ── install_local tests ────────────────────────────────────────────

include!("manager_tests/install.rs");
include!("manager_tests/boot.rs");
include!("manager_tests/state.rs");
