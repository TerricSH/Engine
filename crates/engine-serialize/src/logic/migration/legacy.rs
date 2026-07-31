use super::LogicAssetMigrationError;
use crate::logic::{
    ComparisonOp, LogicAsset, LogicCondition, LogicKind, LogicMetadata, LogicNode, LogicParameter,
    LogicParameterType, LogicTransition, LogicValue, LOGIC_ASSET_SCHEMA_V2,
};
use crate::{AssetId, SchemaVersion};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Former engine-asset production schema. Field and variant order must remain
// exact because unprefixed V1 cooked payloads used bincode's sequence layout.

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyProductionLogicAssetV1 {
    pub(super) schema_version: SchemaVersion,
    asset_id: String,
    kind: LegacyProductionLogicKindV1,
    nodes: Vec<LegacyProductionLogicNodeV1>,
    parameters: BTreeMap<String, LegacyProductionLogicParamV1>,
    metadata: LegacyProductionLogicMetadataV1,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum LegacyProductionLogicKindV1 {
    BehaviorTree,
    StateMachine,
    SkillGraph,
    QuestDialogue,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProductionLogicNodeV1 {
    id: String,
    node_type: String,
    label: Option<String>,
    transitions: Vec<LegacyProductionLogicTransitionV1>,
    properties: BTreeMap<String, LegacyProductionLogicValueV1>,
    children: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProductionLogicTransitionV1 {
    target_node: String,
    condition: Option<LegacyProductionLogicConditionV1>,
    priority: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum LegacyProductionLogicConditionV1 {
    Always,
    Never,
    BoolParam(String),
    Comparison {
        param: String,
        op: LegacyProductionComparisonOpV1,
        value: LegacyProductionLogicValueV1,
    },
    And(Vec<LegacyProductionLogicConditionV1>),
    Or(Vec<LegacyProductionLogicConditionV1>),
    Not(Box<LegacyProductionLogicConditionV1>),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum LegacyProductionComparisonOpV1 {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum LegacyProductionLogicValueV1 {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    AssetRef(AssetId),
    EntityRef(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProductionLogicParamV1 {
    name: String,
    param_type: LegacyProductionLogicParamTypeV1,
    default: Option<LegacyProductionLogicValueV1>,
    description: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum LegacyProductionLogicParamTypeV1 {
    Bool,
    Int,
    Float,
    String,
    AssetRef,
    EntityRef,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProductionLogicMetadataV1 {
    author: Option<String>,
    description: Option<String>,
    tags: Vec<String>,
    version: String,
}

pub(super) fn migrate_production_v1(legacy: LegacyProductionLogicAssetV1) -> LogicAsset {
    LogicAsset {
        schema_version: LOGIC_ASSET_SCHEMA_V2,
        asset_id: legacy.asset_id,
        kind: match legacy.kind {
            LegacyProductionLogicKindV1::BehaviorTree => LogicKind::BehaviorTree,
            LegacyProductionLogicKindV1::StateMachine => LogicKind::StateMachine,
            LegacyProductionLogicKindV1::SkillGraph => LogicKind::SkillGraph,
            LegacyProductionLogicKindV1::QuestDialogue => LogicKind::QuestDialogue,
        },
        nodes: legacy
            .nodes
            .into_iter()
            .map(|node| LogicNode {
                id: node.id,
                node_type: node.node_type,
                label: node.label,
                transitions: node
                    .transitions
                    .into_iter()
                    .map(|transition| LogicTransition {
                        target_node: transition.target_node,
                        condition: transition.condition.map(migrate_production_condition_v1),
                        priority: transition.priority,
                    })
                    .collect(),
                properties: node
                    .properties
                    .into_iter()
                    .map(|(name, value)| (name, migrate_production_value_v1(value)))
                    .collect(),
                children: node.children,
            })
            .collect(),
        entry_node: None,
        parameters: legacy
            .parameters
            .into_iter()
            .map(|(name, parameter)| {
                (
                    name,
                    LogicParameter {
                        name: parameter.name,
                        param_type: match parameter.param_type {
                            LegacyProductionLogicParamTypeV1::Bool => LogicParameterType::Bool,
                            LegacyProductionLogicParamTypeV1::Int => LogicParameterType::Int,
                            LegacyProductionLogicParamTypeV1::Float => LogicParameterType::Float,
                            LegacyProductionLogicParamTypeV1::String => LogicParameterType::String,
                            LegacyProductionLogicParamTypeV1::AssetRef => {
                                LogicParameterType::AssetRef
                            }
                            LegacyProductionLogicParamTypeV1::EntityRef => {
                                LogicParameterType::EntityRef
                            }
                        },
                        default: parameter.default.map(migrate_production_value_v1),
                        description: parameter.description,
                    },
                )
            })
            .collect(),
        metadata: LogicMetadata {
            author: legacy.metadata.author,
            description: legacy.metadata.description,
            tags: legacy.metadata.tags,
            version: legacy.metadata.version,
        },
    }
}

fn migrate_production_condition_v1(condition: LegacyProductionLogicConditionV1) -> LogicCondition {
    match condition {
        LegacyProductionLogicConditionV1::Always => LogicCondition::Always,
        LegacyProductionLogicConditionV1::Never => LogicCondition::Never,
        LegacyProductionLogicConditionV1::BoolParam(name) => LogicCondition::BoolParam(name),
        LegacyProductionLogicConditionV1::Comparison { param, op, value } => {
            LogicCondition::Comparison {
                param,
                op: match op {
                    LegacyProductionComparisonOpV1::Equal => ComparisonOp::Equal,
                    LegacyProductionComparisonOpV1::NotEqual => ComparisonOp::NotEqual,
                    LegacyProductionComparisonOpV1::Less => ComparisonOp::Less,
                    LegacyProductionComparisonOpV1::LessOrEqual => ComparisonOp::LessOrEqual,
                    LegacyProductionComparisonOpV1::Greater => ComparisonOp::Greater,
                    LegacyProductionComparisonOpV1::GreaterOrEqual => ComparisonOp::GreaterOrEqual,
                },
                value: migrate_production_value_v1(value),
            }
        }
        LegacyProductionLogicConditionV1::And(conditions) => LogicCondition::And(
            conditions
                .into_iter()
                .map(migrate_production_condition_v1)
                .collect(),
        ),
        LegacyProductionLogicConditionV1::Or(conditions) => LogicCondition::Or(
            conditions
                .into_iter()
                .map(migrate_production_condition_v1)
                .collect(),
        ),
        LegacyProductionLogicConditionV1::Not(condition) => {
            LogicCondition::Not(Box::new(migrate_production_condition_v1(*condition)))
        }
    }
}

fn migrate_production_value_v1(value: LegacyProductionLogicValueV1) -> LogicValue {
    match value {
        LegacyProductionLogicValueV1::Bool(value) => LogicValue::Bool(value),
        LegacyProductionLogicValueV1::Int(value) => LogicValue::Int(value),
        LegacyProductionLogicValueV1::Float(value) => LogicValue::Float(value),
        LegacyProductionLogicValueV1::String(value) => LogicValue::String(value),
        LegacyProductionLogicValueV1::AssetRef(value) => LogicValue::AssetRef(value),
        LegacyProductionLogicValueV1::EntityRef(value) => LogicValue::EntityRef(value),
    }
}

// Former engine-serialize behavior-graph JSON schema.

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacySerializeLogicAssetV1 {
    schema_version: SchemaVersion,
    kind: LegacySerializeLogicKindV1,
    nodes: Vec<LegacySerializeLogicNodeV1>,
    entry_node: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum LegacySerializeLogicKindV1 {
    BehaviorTree,
    StateMachine,
    SkillGraph,
    QuestDialogue,
    AITree,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySerializeLogicNodeV1 {
    id: String,
    node_type: String,
    parameters: Vec<LegacySerializeLogicParameterV1>,
    transitions: Vec<LegacySerializeLogicTransitionV1>,
    children: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySerializeLogicParameterV1 {
    name: String,
    value: LegacySerializeLogicValueV1,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySerializeLogicTransitionV1 {
    target_node: String,
    condition: Option<LegacySerializeLogicConditionV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum LegacySerializeLogicConditionV1 {
    Always,
    BoolCompare {
        field: String,
        expected: bool,
    },
    FloatCompare {
        field: String,
        op: LegacySerializeCompareOpV1,
        value: f32,
    },
    HasAsset {
        asset: AssetId,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum LegacySerializeCompareOpV1 {
    Equal,
    NotEqual,
    Less,
    Greater,
    LessOrEqual,
    GreaterOrEqual,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum LegacySerializeLogicValueV1 {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Asset(AssetId),
}

pub(super) fn migrate_serialize_v1(
    legacy: LegacySerializeLogicAssetV1,
) -> Result<LogicAsset, LogicAssetMigrationError> {
    let mut parameters = BTreeMap::new();
    for node in &legacy.nodes {
        for parameter in &node.parameters {
            let value = migrate_serialize_value_v1(parameter.value.clone());
            merge_migrated_parameter(
                &mut parameters,
                &parameter.name,
                logic_value_parameter_type(&value),
            )?;
        }
        for transition in &node.transitions {
            if let Some(condition) = &transition.condition {
                collect_legacy_condition_parameters(condition, &mut parameters)?;
            }
        }
    }

    let nodes = legacy
        .nodes
        .into_iter()
        .map(|node| {
            let properties = node
                .parameters
                .into_iter()
                .map(|parameter| (parameter.name, migrate_serialize_value_v1(parameter.value)))
                .collect();
            LogicNode {
                id: node.id,
                node_type: node.node_type,
                label: None,
                transitions: node
                    .transitions
                    .into_iter()
                    .map(|transition| LogicTransition {
                        target_node: transition.target_node,
                        condition: transition.condition.map(migrate_serialize_condition_v1),
                        priority: 0,
                    })
                    .collect(),
                properties,
                children: node.children,
            }
        })
        .collect();

    Ok(LogicAsset {
        schema_version: LOGIC_ASSET_SCHEMA_V2,
        asset_id: String::new(),
        kind: match legacy.kind {
            LegacySerializeLogicKindV1::BehaviorTree => LogicKind::BehaviorTree,
            LegacySerializeLogicKindV1::StateMachine => LogicKind::StateMachine,
            LegacySerializeLogicKindV1::SkillGraph => LogicKind::SkillGraph,
            LegacySerializeLogicKindV1::QuestDialogue => LogicKind::QuestDialogue,
            LegacySerializeLogicKindV1::AITree => LogicKind::AITree,
        },
        nodes,
        entry_node: legacy.entry_node,
        parameters,
        metadata: LogicMetadata::default(),
    })
}

fn merge_migrated_parameter(
    parameters: &mut BTreeMap<String, LogicParameter>,
    name: &str,
    param_type: LogicParameterType,
) -> Result<(), LogicAssetMigrationError> {
    match parameters.get(name) {
        Some(existing) if existing.param_type != param_type => Err(
            LogicAssetMigrationError::ConflictingLegacyParameter(name.to_string()),
        ),
        Some(_) => Ok(()),
        None => {
            parameters.insert(
                name.to_string(),
                LogicParameter {
                    name: name.to_string(),
                    param_type,
                    default: None,
                    description: None,
                },
            );
            Ok(())
        }
    }
}

fn collect_legacy_condition_parameters(
    condition: &LegacySerializeLogicConditionV1,
    parameters: &mut BTreeMap<String, LogicParameter>,
) -> Result<(), LogicAssetMigrationError> {
    match condition {
        LegacySerializeLogicConditionV1::Always
        | LegacySerializeLogicConditionV1::HasAsset { .. } => Ok(()),
        LegacySerializeLogicConditionV1::BoolCompare { field, .. } => {
            merge_migrated_parameter(parameters, field, LogicParameterType::Bool)
        }
        LegacySerializeLogicConditionV1::FloatCompare { field, .. } => {
            merge_migrated_parameter(parameters, field, LogicParameterType::Float)
        }
    }
}

fn migrate_serialize_condition_v1(condition: LegacySerializeLogicConditionV1) -> LogicCondition {
    match condition {
        LegacySerializeLogicConditionV1::Always => LogicCondition::Always,
        LegacySerializeLogicConditionV1::BoolCompare { field, expected } => {
            LogicCondition::Comparison {
                param: field,
                op: ComparisonOp::Equal,
                value: LogicValue::Bool(expected),
            }
        }
        LegacySerializeLogicConditionV1::FloatCompare { field, op, value } => {
            LogicCondition::Comparison {
                param: field,
                op: match op {
                    LegacySerializeCompareOpV1::Equal => ComparisonOp::Equal,
                    LegacySerializeCompareOpV1::NotEqual => ComparisonOp::NotEqual,
                    LegacySerializeCompareOpV1::Less => ComparisonOp::Less,
                    LegacySerializeCompareOpV1::Greater => ComparisonOp::Greater,
                    LegacySerializeCompareOpV1::LessOrEqual => ComparisonOp::LessOrEqual,
                    LegacySerializeCompareOpV1::GreaterOrEqual => ComparisonOp::GreaterOrEqual,
                },
                value: LogicValue::Float(f64::from(value)),
            }
        }
        LegacySerializeLogicConditionV1::HasAsset { asset } => LogicCondition::HasAsset { asset },
    }
}

fn migrate_serialize_value_v1(value: LegacySerializeLogicValueV1) -> LogicValue {
    match value {
        LegacySerializeLogicValueV1::Bool(value) => LogicValue::Bool(value),
        LegacySerializeLogicValueV1::Int(value) => LogicValue::Int(value),
        LegacySerializeLogicValueV1::Float(value) => LogicValue::Float(value),
        LegacySerializeLogicValueV1::Str(value) => LogicValue::String(value),
        LegacySerializeLogicValueV1::Asset(value) => LogicValue::AssetRef(value),
    }
}

fn logic_value_parameter_type(value: &LogicValue) -> LogicParameterType {
    match value {
        LogicValue::Bool(_) => LogicParameterType::Bool,
        LogicValue::Int(_) => LogicParameterType::Int,
        LogicValue::Float(_) => LogicParameterType::Float,
        LogicValue::String(_) => LogicParameterType::String,
        LogicValue::AssetRef(_) => LogicParameterType::AssetRef,
        LogicValue::EntityRef(_) => LogicParameterType::EntityRef,
    }
}
