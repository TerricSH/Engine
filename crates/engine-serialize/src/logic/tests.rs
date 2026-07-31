use super::*;
use serde::Serialize;
use std::collections::BTreeMap;

fn canonical_asset() -> LogicAsset {
    LogicAsset {
        schema_version: LOGIC_ASSET_SCHEMA_V2,
        asset_id: "logic.sample".into(),
        kind: LogicKind::BehaviorTree,
        nodes: vec![
            LogicNode {
                id: "root".into(),
                node_type: "sequence".into(),
                label: Some("Root".into()),
                transitions: vec![LogicTransition {
                    target_node: "action".into(),
                    condition: Some(LogicCondition::BoolParam("enabled".into())),
                    priority: 2,
                }],
                properties: BTreeMap::new(),
                children: vec!["action".into()],
            },
            LogicNode {
                id: "action".into(),
                node_type: "action".into(),
                label: None,
                transitions: Vec::new(),
                properties: BTreeMap::from([(
                    "target".into(),
                    LogicValue::AssetRef(AssetId::new("target.asset")),
                )]),
                children: Vec::new(),
            },
        ],
        entry_node: Some("root".into()),
        parameters: BTreeMap::from([(
            "enabled".into(),
            LogicParameter {
                name: "enabled".into(),
                param_type: LogicParameterType::Bool,
                default: Some(LogicValue::Bool(true)),
                description: None,
            },
        )]),
        metadata: LogicMetadata::default(),
    }
}

fn production_v1_json() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": {"major": 0, "minor": 1, "patch": 0},
        "asset_id": "legacy.production",
        "kind": "BehaviorTree",
        "nodes": [{
            "id": "root",
            "node_type": "action",
            "label": "Root",
            "transitions": [],
            "properties": {
                "target": {"AssetRef": {"id": "asset.target", "logical_path": null}}
            },
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
            "parameters": [{"name": "speed", "value": {"Float": 2.5}}],
            "transitions": [
                {
                    "target_node": "root",
                    "condition": {"FloatCompare": {
                        "field": "speed",
                        "op": "Greater",
                        "value": 10.0
                    }}
                },
                {
                    "target_node": "root",
                    "condition": {"FloatCompare": {
                        "field": "speed",
                        "op": "Less",
                        "value": 100.0
                    }}
                },
                {
                    "target_node": "root",
                    "condition": {"HasAsset": {
                        "asset": {"id": "sense.asset", "logical_path": null}
                    }}
                }
            ],
            "children": []
        }],
        "entry_node": "root"
    }))
    .unwrap()
}

#[test]
fn canonical_json_roundtrip_and_validation() {
    let asset = canonical_asset();
    assert!(asset.validate().is_empty());
    assert_eq!(
        asset.entry_node().map(|node| node.id.as_str()),
        Some("root")
    );
    let json = serde_json::to_vec(&asset).unwrap();
    let parsed = parse_logic_asset_json_compatible(&json).unwrap();
    assert_eq!(parsed.source_schema, LogicAssetSourceSchema::CanonicalV2);
    assert_eq!(parsed.asset, asset);
}

#[test]
fn production_v1_json_migrates_without_losing_asset_fields() {
    let migrated = parse_logic_asset_json_compatible(&production_v1_json()).unwrap();
    assert_eq!(migrated.source_schema, LogicAssetSourceSchema::ProductionV1);
    assert_eq!(migrated.asset.schema_version, LOGIC_ASSET_SCHEMA_V2);
    assert_eq!(migrated.asset.asset_id, "legacy.production");
    assert!(matches!(
        migrated.asset.nodes[0].properties["target"],
        LogicValue::AssetRef(_)
    ));
    assert!(migrated.asset.validate().is_empty());
}

#[test]
fn serialize_v1_migrates_entry_ai_kind_properties_and_conditions() {
    let migrated = parse_logic_asset_json_compatible(&serialize_v1_json()).unwrap();
    assert_eq!(migrated.source_schema, LogicAssetSourceSchema::SerializeV1);
    assert_eq!(migrated.asset.entry_node.as_deref(), Some("root"));
    assert_eq!(migrated.asset.kind, LogicKind::AITree);
    assert_eq!(
        migrated.asset.nodes[0].properties["speed"],
        LogicValue::Float(2.5)
    );
    assert_eq!(
        migrated.asset.parameters["speed"].param_type,
        LogicParameterType::Float
    );
    assert_eq!(migrated.asset.parameters["speed"].default, None);
    assert!(matches!(
        migrated.asset.nodes[0].transitions[2].condition,
        Some(LogicCondition::HasAsset { .. })
    ));
    assert!(migrated.asset.validate().is_empty());
}

#[test]
fn incompatible_serialize_v1_parameter_types_are_rejected() {
    let source = serde_json::json!({
        "schema_version": {"major": 0, "minor": 1, "patch": 0},
        "kind": "AITree",
        "nodes": [{
            "id": "root",
            "node_type": "sense",
            "parameters": [{"name": "value", "value": {"Float": 2.5}}],
            "transitions": [{
                "target_node": "root",
                "condition": {"BoolCompare": {"field": "value", "expected": true}}
            }],
            "children": []
        }],
        "entry_node": "root"
    });
    assert_eq!(
        parse_logic_asset_json_compatible(&serde_json::to_vec(&source).unwrap()).unwrap_err(),
        LogicAssetMigrationError::ConflictingLegacyParameter("value".into())
    );
}

#[test]
fn ambiguous_v1_json_is_rejected_instead_of_dropping_fields() {
    let json = serde_json::json!({
        "schema_version": {"major": 0, "minor": 1, "patch": 0},
        "asset_id": "ambiguous",
        "kind": "BehaviorTree",
        "nodes": [{
            "id": "root",
            "node_type": "action",
            "parameters": [],
            "transitions": [],
            "children": []
        }],
        "entry_node": "root",
        "parameters": {},
        "metadata": {"author": null, "description": null, "tags": [], "version": "1"}
    });
    assert_eq!(
        parse_logic_asset_json_compatible(&serde_json::to_vec(&json).unwrap()).unwrap_err(),
        LogicAssetMigrationError::AmbiguousV1
    );
}

#[test]
fn canonical_v2_rejects_legacy_node_parameters() {
    let mut json = serde_json::to_value(canonical_asset()).unwrap();
    json["nodes"][0]["parameters"] = serde_json::json!([]);
    assert!(matches!(
        parse_logic_asset_json_compatible(&serde_json::to_vec(&json).unwrap()),
        Err(LogicAssetMigrationError::Json(_))
    ));
}

#[test]
fn validation_merges_entry_parameter_and_child_cycle_checks() {
    let mut asset = canonical_asset();
    asset.entry_node = Some("missing".into());
    asset.nodes[0].transitions[0].condition =
        Some(LogicCondition::BoolParam("missing_parameter".into()));
    asset.nodes[1].children.push("root".into());
    let errors = asset.validate();
    assert!(errors.iter().any(|error| error.contains("Entry node")));
    assert!(errors
        .iter()
        .any(|error| error.contains("missing_parameter")));
    assert!(errors
        .iter()
        .any(|error| error.contains("Circular child dependency")));
}

#[test]
fn cooked_v2_requires_magic_and_roundtrips() {
    let asset = canonical_asset();
    let bytes = encode_logic_asset_cooked_v2(&asset).unwrap();
    assert!(bytes.starts_with(LOGIC_ASSET_COOKED_V2_MAGIC));
    let decoded = decode_logic_asset_cooked_compatible(&bytes).unwrap();
    assert_eq!(decoded.source_schema, LogicAssetSourceSchema::CanonicalV2);
    assert_eq!(decoded.asset, asset);

    let mut corrupt = LOGIC_ASSET_COOKED_V2_MAGIC.to_vec();
    corrupt.extend([1, 2, 3]);
    assert!(decode_logic_asset_cooked_compatible(&corrupt).is_err());
}

#[derive(Serialize)]
struct ProductionV1BincodeFixture {
    schema_version: SchemaVersion,
    asset_id: String,
    kind: ProductionV1Kind,
    nodes: Vec<ProductionV1Node>,
    parameters: BTreeMap<String, ProductionV1Parameter>,
    metadata: ProductionV1Metadata,
}

#[derive(Serialize)]
enum ProductionV1Kind {
    BehaviorTree,
}

#[derive(Serialize)]
struct ProductionV1Node {
    id: String,
    node_type: String,
    label: Option<String>,
    transitions: Vec<ProductionV1Transition>,
    properties: BTreeMap<String, ProductionV1Value>,
    children: Vec<String>,
}

#[derive(Serialize)]
struct ProductionV1Transition {
    target_node: String,
    condition: Option<ProductionV1Condition>,
    priority: i32,
}

#[derive(Serialize)]
enum ProductionV1Condition {
    Always,
}

#[derive(Serialize)]
enum ProductionV1Value {
    Bool(bool),
}

#[derive(Serialize)]
struct ProductionV1Parameter {
    name: String,
    param_type: ProductionV1ParameterType,
    default: Option<ProductionV1Value>,
    description: Option<String>,
}

#[derive(Serialize)]
enum ProductionV1ParameterType {
    Bool,
}

#[derive(Serialize)]
struct ProductionV1Metadata {
    author: Option<String>,
    description: Option<String>,
    tags: Vec<String>,
    version: String,
}

#[test]
fn production_v1_bincode_uses_explicit_legacy_decoder() {
    let legacy = ProductionV1BincodeFixture {
        schema_version: LOGIC_ASSET_SCHEMA_V1,
        asset_id: "legacy.production".into(),
        kind: ProductionV1Kind::BehaviorTree,
        nodes: vec![ProductionV1Node {
            id: "root".into(),
            node_type: "action".into(),
            label: Some("Root".into()),
            transitions: vec![ProductionV1Transition {
                target_node: "root".into(),
                condition: Some(ProductionV1Condition::Always),
                priority: 3,
            }],
            properties: BTreeMap::from([("enabled".into(), ProductionV1Value::Bool(true))]),
            children: Vec::new(),
        }],
        parameters: BTreeMap::from([(
            "enabled".into(),
            ProductionV1Parameter {
                name: "enabled".into(),
                param_type: ProductionV1ParameterType::Bool,
                default: Some(ProductionV1Value::Bool(true)),
                description: None,
            },
        )]),
        metadata: ProductionV1Metadata {
            author: None,
            description: None,
            tags: vec!["legacy".into()],
            version: "1.0.0".into(),
        },
    };
    let bytes = bincode::serialize(&legacy).unwrap();
    let decoded = decode_logic_asset_cooked_compatible(&bytes).unwrap();
    assert_eq!(decoded.source_schema, LogicAssetSourceSchema::ProductionV1);
    assert_eq!(decoded.asset.asset_id, "legacy.production");
    assert_eq!(decoded.asset.schema_version, LOGIC_ASSET_SCHEMA_V2);
}

#[test]
fn public_legacy_type_names_alias_the_canonical_types() {
    assert_eq!(
        std::any::TypeId::of::<LogicAssetKind>(),
        std::any::TypeId::of::<LogicKind>()
    );
    assert_eq!(
        std::any::TypeId::of::<LogicParam>(),
        std::any::TypeId::of::<LogicParameter>()
    );
    assert_eq!(
        std::any::TypeId::of::<LogicParamType>(),
        std::any::TypeId::of::<LogicParameterType>()
    );
    assert_eq!(
        std::any::TypeId::of::<CompareOp>(),
        std::any::TypeId::of::<ComparisonOp>()
    );
}
