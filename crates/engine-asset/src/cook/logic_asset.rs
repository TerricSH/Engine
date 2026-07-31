//! Logic-asset cooking and runtime loading.
//!
//! The data model is owned by `engine-serialize`. This module intentionally
//! contains only the asset-pipeline adapter plus compatibility re-exports for
//! callers that used the former `engine_asset::cook::logic_asset` paths.

use std::path::Path;

pub use engine_serialize::{
    decode_logic_asset_cooked_compatible, encode_logic_asset_cooked_v2,
    parse_logic_asset_json_compatible, CompareOp, ComparisonOp, LogicAsset, LogicAssetKind,
    LogicAssetMigration, LogicAssetMigrationError, LogicAssetSourceSchema, LogicCondition,
    LogicKind, LogicMetadata, LogicNode, LogicParam, LogicParamType, LogicParameter,
    LogicParameterType, LogicTransition, LogicValue, LOGIC_ASSET_COOKED_V2_MAGIC,
    LOGIC_ASSET_SCHEMA_V1, LOGIC_ASSET_SCHEMA_V2,
};

use super::error::CookError;
use super::{write_cooked_artifact, AssetType, CookResult};

pub const LOGIC_ASSET_TYPE_ID: &str = "logic";

fn parse_and_validate(source: &[u8]) -> Result<LogicAsset, String> {
    let migration = parse_logic_asset_json_compatible(source).map_err(|error| error.to_string())?;
    validate_asset(migration.asset)
}

fn decode_and_validate(cooked: &[u8]) -> Result<LogicAsset, String> {
    let migration =
        decode_logic_asset_cooked_compatible(cooked).map_err(|error| error.to_string())?;
    validate_asset(migration.asset)
}

fn validate_asset(asset: LogicAsset) -> Result<LogicAsset, String> {
    let errors = asset.validate();
    if errors.is_empty() {
        Ok(asset)
    } else {
        Err(format!(
            "LogicAsset validation failed: {}",
            errors.join("; ")
        ))
    }
}

/// Registry cooker used by tooling that routes source bytes through the
/// extension pipeline.
///
/// Both former V1 JSON layouts are migrated explicitly. Output is always the
/// canonical V2 cooked layout and carries the V2 magic prefix.
pub fn logic_asset_cooker(source: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
    let asset = parse_and_validate(source)?;
    let payload = encode_logic_asset_cooked_v2(&asset).map_err(|error| error.to_string())?;
    output.extend_from_slice(&payload);
    Ok(())
}

/// Runtime loader for canonical V2 cooked logic graphs.
///
/// Exact legacy production-V1 bincode payloads are migrated by
/// `engine-serialize`; malformed V2-prefixed data never falls back to the
/// legacy decoder.
pub fn logic_asset_loader(cooked: &[u8]) -> Result<Box<dyn std::any::Any + Send + Sync>, String> {
    Ok(Box::new(decode_and_validate(cooked)?))
}

pub fn register_logic_asset_type(registry: &mut engine_scene::registry::AssetTypeRegistry) {
    use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

    let _ = registry.register(AssetTypeExtension {
        meta: AssetTypeMeta {
            type_id: LOGIC_ASSET_TYPE_ID,
            source_extensions: vec!["logic.json"],
            display_name: "Logic Graph",
        },
        cooker: Some(logic_asset_cooker),
        loader: Some(logic_asset_loader),
    });
}

/// Cook a logic asset source into a standard engine cooked artifact.
///
/// Source may use canonical V2 JSON or either explicitly recognized V1 JSON
/// layout. The artifact header and payload are always written as V2.
pub fn cook_logic_asset(source: &Path, output: &Path) -> Result<CookResult, CookError> {
    let source_bytes = std::fs::read(source)?;
    let asset = parse_logic_asset_json_compatible(&source_bytes)
        .map_err(|error| CookError::Parse(error.to_string()))?
        .asset;

    let validation_errors = asset.validate();
    if !validation_errors.is_empty() {
        return Err(CookError::InvalidAsset(format!(
            "logic asset '{}' validation failed:\n  - {}",
            asset.asset_id,
            validation_errors.join("\n  - ")
        )));
    }

    let payload = encode_logic_asset_cooked_v2(&asset)
        .map_err(|error| CookError::InvalidAsset(error.to_string()))?;
    write_cooked_artifact(
        output,
        AssetType::Logic.kind_code(),
        &payload,
        LOGIC_ASSET_SCHEMA_V2,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::read_cooked_artifact;
    use engine_serialize::{AssetId, SchemaVersion};
    use std::collections::BTreeMap;

    fn canonical_asset() -> LogicAsset {
        LogicAsset {
            schema_version: LOGIC_ASSET_SCHEMA_V2,
            asset_id: "logic.canonical".into(),
            kind: LogicKind::BehaviorTree,
            nodes: vec![LogicNode {
                id: "root".into(),
                node_type: "action".into(),
                label: None,
                transitions: Vec::new(),
                properties: BTreeMap::from([(
                    "target".into(),
                    LogicValue::AssetRef(AssetId::new("asset.target")),
                )]),
                children: Vec::new(),
            }],
            entry_node: Some("root".into()),
            parameters: BTreeMap::new(),
            metadata: LogicMetadata::default(),
        }
    }

    fn production_v1_json() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": {"major": 0, "minor": 1, "patch": 0},
            "asset_id": "logic.production-v1",
            "kind": "BehaviorTree",
            "nodes": [{
                "id": "root",
                "node_type": "action",
                "label": null,
                "transitions": [],
                "properties": {"enabled": {"Bool": true}},
                "children": []
            }],
            "parameters": {},
            "metadata": {
                "author": null,
                "description": null,
                "tags": ["legacy"],
                "version": "1.0.0"
            }
        }))
        .unwrap()
    }

    fn serialize_v1_json() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": {"major": 0, "minor": 1, "patch": 0},
            "kind": "AITree",
            "nodes": [{
                "id": "root",
                "node_type": "sense",
                "parameters": [{"name": "radius", "value": {"Float": 25.0}}],
                "transitions": [{
                    "target_node": "root",
                    "condition": {"HasAsset": {
                        "asset": {"id": "asset.sensor", "logical_path": null}
                    }}
                }],
                "children": []
            }],
            "entry_node": "root"
        }))
        .unwrap()
    }

    #[test]
    fn canonical_source_cooks_and_loads_as_engine_serialize_type() {
        let source = serde_json::to_vec(&canonical_asset()).unwrap();
        let mut cooked = Vec::new();
        logic_asset_cooker(&source, &mut cooked).unwrap();
        assert!(cooked.starts_with(LOGIC_ASSET_COOKED_V2_MAGIC));

        let loaded = logic_asset_loader(&cooked).unwrap();
        let loaded = loaded
            .downcast_ref::<engine_serialize::LogicAsset>()
            .expect("loader must return the canonical engine-serialize type");
        assert_eq!(loaded.asset_id, "logic.canonical");
        assert_eq!(loaded.schema_version, LOGIC_ASSET_SCHEMA_V2);
    }

    #[test]
    fn both_v1_json_layouts_are_migrated_before_cooking() {
        for (source, expected_schema, expected_kind) in [
            (
                production_v1_json(),
                LogicAssetSourceSchema::ProductionV1,
                LogicKind::BehaviorTree,
            ),
            (
                serialize_v1_json(),
                LogicAssetSourceSchema::SerializeV1,
                LogicKind::AITree,
            ),
        ] {
            let migration = parse_logic_asset_json_compatible(&source).unwrap();
            assert_eq!(migration.source_schema, expected_schema);
            assert_eq!(migration.asset.kind, expected_kind);

            let mut cooked = Vec::new();
            logic_asset_cooker(&source, &mut cooked).unwrap();
            let loaded = logic_asset_loader(&cooked).unwrap();
            let loaded = loaded
                .downcast_ref::<engine_serialize::LogicAsset>()
                .unwrap();
            assert_eq!(loaded.schema_version, LOGIC_ASSET_SCHEMA_V2);
            assert!(loaded.validate().is_empty());
        }
    }

    #[test]
    fn malformed_v2_payload_rejects_legacy_fallback() {
        let mut corrupt = LOGIC_ASSET_COOKED_V2_MAGIC.to_vec();
        corrupt.extend([1, 2, 3]);
        let error = logic_asset_loader(&corrupt).unwrap_err();
        assert!(error.contains("bincode"));
    }

    #[test]
    fn invalid_graph_is_rejected_before_cooking() {
        let mut asset = canonical_asset();
        asset.nodes[0].transitions.push(LogicTransition {
            target_node: "missing".into(),
            condition: None,
            priority: 0,
        });
        let source = serde_json::to_vec(&asset).unwrap();
        let error = logic_asset_cooker(&source, &mut Vec::new()).unwrap_err();
        assert!(error.contains("target node 'missing' does not exist"));
    }

    #[test]
    fn file_cooker_writes_v2_header_and_prefixed_payload() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("sample.logic.json");
        let output = directory.path().join("sample.cooked");
        std::fs::write(&source, production_v1_json()).unwrap();

        cook_logic_asset(&source, &output).unwrap();
        let artifact = read_cooked_artifact(&output).unwrap();
        assert_eq!(artifact.header.asset_kind, AssetType::Logic.kind_code());
        assert_eq!(artifact.header.schema_version, LOGIC_ASSET_SCHEMA_V2);
        assert!(artifact.payload.starts_with(LOGIC_ASSET_COOKED_V2_MAGIC));

        let loaded = logic_asset_loader(&artifact.payload).unwrap();
        let loaded = loaded
            .downcast_ref::<engine_serialize::LogicAsset>()
            .unwrap();
        assert_eq!(loaded.asset_id, "logic.production-v1");
    }

    #[test]
    fn compatibility_names_are_aliases_not_parallel_types() {
        use std::any::TypeId;

        assert_eq!(
            TypeId::of::<LogicAsset>(),
            TypeId::of::<engine_serialize::LogicAsset>()
        );
        assert_eq!(TypeId::of::<LogicAssetKind>(), TypeId::of::<LogicKind>());
        assert_eq!(TypeId::of::<LogicParam>(), TypeId::of::<LogicParameter>());
        assert_eq!(
            TypeId::of::<LogicParamType>(),
            TypeId::of::<LogicParameterType>()
        );
        assert_eq!(TypeId::of::<CompareOp>(), TypeId::of::<ComparisonOp>());
    }

    #[test]
    fn registration_exposes_canonical_cooker_and_loader() {
        let mut registry = engine_scene::registry::AssetTypeRegistry::new();
        register_logic_asset_type(&mut registry);
        let extension = registry.get(LOGIC_ASSET_TYPE_ID).unwrap();
        assert!(extension.cooker.is_some());
        assert!(extension.loader.is_some());
    }

    #[test]
    fn public_v1_schema_constant_preserves_the_old_contract() {
        assert_eq!(LOGIC_ASSET_SCHEMA_V1, SchemaVersion::new(0, 1, 0));
    }
}
