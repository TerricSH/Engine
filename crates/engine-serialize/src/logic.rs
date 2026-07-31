//! Canonical, interpreted logic-asset contract.
//!
//! Version 2 unifies the former `engine-asset` production schema and the older
//! `engine-serialize` behavior-graph schema. Legacy data is recognized before
//! deserialization and migrated explicitly by the compatibility helpers.

use crate::{AssetId, SchemaVersion};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod migration;
mod validation;

pub use migration::{
    decode_logic_asset_cooked_compatible, encode_logic_asset_cooked_v2,
    parse_logic_asset_json_compatible, LogicAssetMigration, LogicAssetMigrationError,
    LogicAssetSourceSchema,
};

pub const LOGIC_ASSET_SCHEMA_V1: SchemaVersion = SchemaVersion::new(0, 1, 0);
pub const LOGIC_ASSET_SCHEMA_V2: SchemaVersion = SchemaVersion::new(0, 2, 0);
pub const LOGIC_ASSET_COOKED_V2_MAGIC: &[u8; 8] = b"LOGICV2\0";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicAsset {
    pub schema_version: SchemaVersion,
    #[serde(default)]
    pub asset_id: String,
    pub kind: LogicKind,
    pub nodes: Vec<LogicNode>,
    #[serde(default)]
    pub entry_node: Option<String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, LogicParameter>,
    #[serde(default)]
    pub metadata: LogicMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum LogicKind {
    BehaviorTree,
    StateMachine,
    SkillGraph,
    QuestDialogue,
    AITree,
}

/// Backwards-compatible production-schema name for [`LogicKind`].
pub type LogicAssetKind = LogicKind;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicNode {
    pub id: String,
    pub node_type: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub transitions: Vec<LogicTransition>,
    #[serde(default)]
    pub properties: BTreeMap<String, LogicValue>,
    #[serde(default)]
    pub children: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicTransition {
    pub target_node: String,
    #[serde(default)]
    pub condition: Option<LogicCondition>,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LogicCondition {
    Always,
    Never,
    BoolParam(String),
    Comparison {
        param: String,
        op: ComparisonOp,
        value: LogicValue,
    },
    And(Vec<LogicCondition>),
    Or(Vec<LogicCondition>),
    Not(Box<LogicCondition>),
    HasAsset {
        asset: AssetId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonOp {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

/// Backwards-compatible serialize-schema name for [`ComparisonOp`].
pub type CompareOp = ComparisonOp;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LogicValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    AssetRef(AssetId),
    EntityRef(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicParameter {
    pub name: String,
    pub param_type: LogicParameterType,
    #[serde(default)]
    pub default: Option<LogicValue>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Backwards-compatible production-schema name for [`LogicParameter`].
pub type LogicParam = LogicParameter;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicParameterType {
    Bool,
    Int,
    Float,
    String,
    AssetRef,
    EntityRef,
}

/// Backwards-compatible production-schema name for [`LogicParameterType`].
pub type LogicParamType = LogicParameterType;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicMetadata {
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_logic_metadata_version")]
    pub version: String,
}

fn default_logic_metadata_version() -> String {
    "2.0.0".to_string()
}

impl Default for LogicMetadata {
    fn default() -> Self {
        Self {
            author: None,
            description: None,
            tags: Vec::new(),
            version: default_logic_metadata_version(),
        }
    }
}

impl LogicAsset {
    pub fn find_node(&self, id: &str) -> Option<&LogicNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn entry_node(&self) -> Option<&LogicNode> {
        match &self.entry_node {
            Some(id) => self.find_node(id),
            None => self.nodes.first(),
        }
    }
}

#[cfg(test)]
mod tests;
