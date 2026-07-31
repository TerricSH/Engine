use super::*;
use engine_serialize::{
    AssetId, PayloadHash, PlatformKind, PlatformPayload, RollbackMetadata, SchemaVersion,
};
use sha2::{Digest, Sha256};

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
            platform: PlatformKind::Desktop,
            path: "data.bin".into(),
            algorithm: "sha256".into(),
            hash: {
                let h = Sha256::digest(b"test payload");
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&h);
                arr
            },
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

include!("install_tests/basic.rs");
include!("install_tests/recovery.rs");
