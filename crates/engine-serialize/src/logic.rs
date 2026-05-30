//! Gate 7 — Session 7B: Interpreted Logic Asset Model.
//!
//! Defines the first mobile-safe hot-updatable logic asset type (behavior graph)
//! with schema, serialization, parsing, and validation.  These are pure data
//! structures — no runtime interpreter is provided.
//!
//! # Types
//!
//! | Type               | Purpose                                          |
//! |--------------------|--------------------------------------------------|
//! | [`LogicAsset`]     | Top-level container with version, kind, nodes    |
//! | [`LogicKind`]      | Discriminator: behavior tree, state machine, …   |
//! | [`LogicNode`]      | A single node in the graph                       |
//! | [`LogicParameter`] | A typed parameter bound to a node                |
//! | [`LogicTransition`]| Edge to another node, optionally gated           |
//! | [`LogicCondition`] | Gate condition enum                              |
//! | [`CompareOp`]      | Comparison operators for float conditions        |
//! | [`LogicValue`]     | Typed value (bool, int, float, string, asset)    |

use crate::{AssetId, SchemaVersion};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ── Top-level container ───────────────────────────────────────────────────

/// A typed logic asset (behavior graph / state machine).
///
/// Hot-updatable on mobile without executable code.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicAsset {
    /// Schema version for forward-compatibility.
    pub schema_version: SchemaVersion,
    /// Kind of logic graph this asset represents.
    pub kind: LogicKind,
    /// Nodes in the graph.
    pub nodes: Vec<LogicNode>,
    /// ID of the entry node.  `None` means the first node is the entry point.
    pub entry_node: Option<String>,
}

// ── LogicKind ─────────────────────────────────────────────────────────────

/// The kind of logic graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicKind {
    /// A behaviour tree (composite/decorator/action nodes).
    BehaviorTree,
    /// A hierarchical or flat state machine.
    StateMachine,
    /// A skill/ability graph.
    SkillGraph,
    /// A quest or dialogue tree.
    QuestDialogue,
    /// An AI behaviour tree with environment sensing.
    AITree,
}

// ── Nodes ─────────────────────────────────────────────────────────────────

/// A single node in the logic graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicNode {
    /// Unique identifier for this node within the asset.
    pub id: String,
    /// Type discriminator (e.g. "sequence", "selector", "condition",
    /// "action", "wait", "state", …).
    pub node_type: String,
    /// Typed parameters for this node.
    pub parameters: Vec<LogicParameter>,
    /// Outgoing transitions / links to other nodes.
    pub transitions: Vec<LogicTransition>,
    /// Child node IDs (for hierarchical graphs like behaviour trees).
    pub children: Vec<String>,
}

// ── Parameters ────────────────────────────────────────────────────────────

/// A typed parameter bound to a node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicParameter {
    /// Parameter name.
    pub name: String,
    /// Parameter value.
    pub value: LogicValue,
}

// ── Transitions ───────────────────────────────────────────────────────────

/// A transition between nodes (for state machines) or a sequence link
/// (for behaviour trees).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicTransition {
    /// The target node identifier.
    pub target_node: String,
    /// Optional condition that must be satisfied for the transition to fire.
    /// `None` is equivalent to `Always` (unconditional).
    pub condition: Option<LogicCondition>,
}

// ── Conditions ────────────────────────────────────────────────────────────

/// A condition that gates a transition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LogicCondition {
    /// Always fires (unconditional).
    Always,
    /// Compare a boolean field against an expected value.
    BoolCompare { field: String, expected: bool },
    /// Compare a float field against a value using an operator.
    FloatCompare {
        field: String,
        op: CompareOp,
        value: f32,
    },
    /// True when the runtime has a specific asset loaded.
    HasAsset { asset: AssetId },
}

// ── CompareOp ─────────────────────────────────────────────────────────────

/// Comparison operators for [`LogicCondition::FloatCompare`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    Equal,
    NotEqual,
    Less,
    Greater,
    LessOrEqual,
    GreaterOrEqual,
}

// ── LogicValue ────────────────────────────────────────────────────────────

/// A typed value used in parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LogicValue {
    /// Boolean.
    Bool(bool),
    /// Signed 64-bit integer.
    Int(i64),
    /// 64-bit floating point.
    Float(f64),
    /// UTF-8 string.
    Str(String),
    /// Reference to another asset by its [`AssetId`].
    Asset(AssetId),
}

// ── Validation ────────────────────────────────────────────────────────────

impl LogicAsset {
    /// Validate the logic asset structure.
    ///
    /// Returns a list of human-readable error strings.  An empty `Vec` means
    /// the asset is structurally valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();

        // At least one node must exist.
        if self.nodes.is_empty() {
            errors.push("Asset must contain at least one node".into());
            return errors;
        }

        // Collect all node IDs.
        let mut node_ids: HashSet<&str> = HashSet::new();
        for node in &self.nodes {
            if !node_ids.insert(node.id.as_str()) {
                errors.push(format!("Duplicate node ID: '{}'", node.id));
            }
        }

        // Check entry node exists.
        if let Some(ref entry) = self.entry_node {
            if !node_ids.contains(entry.as_str()) {
                errors.push(format!(
                    "Entry node '{}' does not exist in the node list",
                    entry
                ));
            }
        }

        // Check transition targets reference existing nodes.
        for node in &self.nodes {
            for (i, transition) in node.transitions.iter().enumerate() {
                if !node_ids.contains(transition.target_node.as_str()) {
                    errors.push(format!(
                        "Node '{}', transition {}: target node '{}' does not exist",
                        node.id, i, transition.target_node
                    ));
                }
            }

            // Check child node IDs reference existing nodes.
            for child_id in &node.children {
                if !node_ids.contains(child_id.as_str()) {
                    errors.push(format!(
                        "Node '{}': child node '{}' does not exist",
                        node.id, child_id
                    ));
                }
            }

            // Validate parameter types.
            for (i, param) in node.parameters.iter().enumerate() {
                if param.name.is_empty() {
                    errors.push(format!(
                        "Node '{}', parameter {}: parameter name is empty",
                        node.id, i
                    ));
                }
            }
        }

        // Detect circular dependencies in node children (DFS).
        self.detect_child_cycles(&node_ids, &mut errors);

        errors
    }

    /// Find a node by its ID.
    pub fn find_node(&self, id: &str) -> Option<&LogicNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Return the entry node, or the first node if no explicit entry is set.
    pub fn entry_node(&self) -> Option<&LogicNode> {
        match self.entry_node {
            Some(ref id) => self.find_node(id),
            None => self.nodes.first(),
        }
    }

    // ── Internal helpers ───────────────────────────────────────────────

    /// Detect cycles in the child-node graph using DFS.
    fn detect_child_cycles(&self, node_ids: &HashSet<&str>, errors: &mut Vec<String>) {
        // Build adjacency: parent → children (only edges to existing nodes).
        let node_list: Vec<&str> = node_ids.iter().copied().collect();
        let n = node_list.len();

        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for node in &self.nodes {
            if let Some(from) = node_list.iter().position(|&id| id == node.id) {
                for child_id in &node.children {
                    if let Some(to) = node_list.iter().position(|&id| id == child_id.as_str()) {
                        if from != to {
                            adj[from].push(to);
                        }
                    }
                }
            }
        }

        // Three-colour DFS.
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Color {
            White,
            Gray,
            Black,
        }

        let mut color = vec![Color::White; n];
        let mut path: Vec<String> = Vec::new();

        fn dfs(
            u: usize,
            adj: &[Vec<usize>],
            color: &mut Vec<Color>,
            path: &mut Vec<String>,
            node_list: &[&str],
            errors: &mut Vec<String>,
        ) {
            color[u] = Color::Gray;
            path.push(node_list[u].to_string());

            for &v in &adj[u] {
                match color[v] {
                    Color::Gray => {
                        let cycle_start = path
                            .iter()
                            .position(|n| n.as_str() == node_list[v])
                            .unwrap_or(0);
                        let cycle: Vec<&str> =
                            path[cycle_start..].iter().map(|s| s.as_str()).collect();
                        errors.push(format!(
                            "Circular child dependency detected: {}",
                            cycle.join(" → ")
                        ));
                    }
                    Color::White => {
                        dfs(v, adj, color, path, node_list, errors);
                    }
                    Color::Black => {}
                }
            }

            path.pop();
            color[u] = Color::Black;
        }

        for i in 0..n {
            if color[i] == Color::White {
                dfs(i, &adj, &mut color, &mut path, &node_list, errors);
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SchemaVersion;

    // ── Helpers ────────────────────────────────────────────────────────

    fn sample_behavior_tree() -> LogicAsset {
        LogicAsset {
            schema_version: SchemaVersion::new(0, 1, 0),
            kind: LogicKind::BehaviorTree,
            entry_node: Some("root".into()),
            nodes: vec![
                LogicNode {
                    id: "root".into(),
                    node_type: "sequence".into(),
                    parameters: vec![],
                    transitions: vec![],
                    children: vec!["move_to_point".into(), "wait".into()],
                },
                LogicNode {
                    id: "move_to_point".into(),
                    node_type: "action".into(),
                    parameters: vec![LogicParameter {
                        name: "speed".into(),
                        value: LogicValue::Float(2.5),
                    }],
                    transitions: vec![LogicTransition {
                        target_node: "root".into(),
                        condition: Some(LogicCondition::Always),
                    }],
                    children: vec![],
                },
                LogicNode {
                    id: "wait".into(),
                    node_type: "action".into(),
                    parameters: vec![LogicParameter {
                        name: "duration".into(),
                        value: LogicValue::Float(3.0),
                    }],
                    transitions: vec![LogicTransition {
                        target_node: "root".into(),
                        condition: Some(LogicCondition::BoolCompare {
                            field: "has_waited".into(),
                            expected: true,
                        }),
                    }],
                    children: vec![],
                },
            ],
        }
    }

    fn sample_state_machine() -> LogicAsset {
        LogicAsset {
            schema_version: SchemaVersion::new(0, 1, 0),
            kind: LogicKind::StateMachine,
            entry_node: Some("closed".into()),
            nodes: vec![
                LogicNode {
                    id: "closed".into(),
                    node_type: "state".into(),
                    parameters: vec![],
                    transitions: vec![LogicTransition {
                        target_node: "opening".into(),
                        condition: Some(LogicCondition::BoolCompare {
                            field: "button_pressed".into(),
                            expected: true,
                        }),
                    }],
                    children: vec![],
                },
                LogicNode {
                    id: "opening".into(),
                    node_type: "state".into(),
                    parameters: vec![],
                    transitions: vec![LogicTransition {
                        target_node: "open".into(),
                        condition: Some(LogicCondition::Always),
                    }],
                    children: vec![],
                },
                LogicNode {
                    id: "open".into(),
                    node_type: "state".into(),
                    parameters: vec![LogicParameter {
                        name: "open_time".into(),
                        value: LogicValue::Float(0.0),
                    }],
                    transitions: vec![LogicTransition {
                        target_node: "closing".into(),
                        condition: Some(LogicCondition::FloatCompare {
                            field: "open_time".into(),
                            op: CompareOp::GreaterOrEqual,
                            value: 5.0,
                        }),
                    }],
                    children: vec![],
                },
                LogicNode {
                    id: "closing".into(),
                    node_type: "state".into(),
                    parameters: vec![],
                    transitions: vec![LogicTransition {
                        target_node: "closed".into(),
                        condition: Some(LogicCondition::Always),
                    }],
                    children: vec![],
                },
            ],
        }
    }

    // ── Round-trip tests ───────────────────────────────────────────────

    #[test]
    fn logic_asset_behavior_tree_roundtrip() {
        let asset = sample_behavior_tree();
        let json = serde_json::to_string_pretty(&asset).unwrap();
        let restored: LogicAsset = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.schema_version, SchemaVersion::new(0, 1, 0));
        assert!(matches!(restored.kind, LogicKind::BehaviorTree));
        assert_eq!(restored.entry_node.as_deref(), Some("root"));
        assert_eq!(restored.nodes.len(), 3);

        // Verify a parameter round-trip.
        let move_node = restored
            .nodes
            .iter()
            .find(|n| n.id == "move_to_point")
            .expect("move_to_point node missing");
        assert_eq!(move_node.parameters.len(), 1);
        assert_eq!(move_node.parameters[0].name, "speed");
        assert!(matches!(
            move_node.parameters[0].value,
            LogicValue::Float(v) if (v - 2.5).abs() < 1e-10
        ));
    }

    #[test]
    fn logic_asset_state_machine_roundtrip() {
        let asset = sample_state_machine();
        let json = serde_json::to_string_pretty(&asset).unwrap();
        let restored: LogicAsset = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.schema_version, SchemaVersion::new(0, 1, 0));
        assert!(matches!(restored.kind, LogicKind::StateMachine));
        assert_eq!(restored.entry_node.as_deref(), Some("closed"));
        assert_eq!(restored.nodes.len(), 4);

        // Verify a transition condition round-trip.
        let closed = restored
            .nodes
            .iter()
            .find(|n| n.id == "closed")
            .expect("closed node missing");
        assert_eq!(closed.transitions.len(), 1);
        assert!(matches!(
            closed.transitions[0].condition,
            Some(LogicCondition::BoolCompare { ref field, expected: true })
                if field == "button_pressed"
        ));
    }

    #[test]
    fn logic_asset_serde_bincode() {
        let asset = sample_behavior_tree();
        let bytes = bincode::serialize(&asset).unwrap();
        let restored: LogicAsset = bincode::deserialize(&bytes).unwrap();

        assert_eq!(restored.schema_version, asset.schema_version);
        assert_eq!(restored.nodes.len(), asset.nodes.len());
        assert_eq!(restored.entry_node, asset.entry_node);
        assert!(matches!(restored.kind, LogicKind::BehaviorTree));

        // Verify a specific node round-tripped via bincode.
        let move_node = restored
            .nodes
            .iter()
            .find(|n| n.id == "move_to_point")
            .expect("move_to_point node missing");
        assert_eq!(move_node.node_type, "action");
    }

    // ── Validation tests ───────────────────────────────────────────────

    #[test]
    fn logic_asset_validation_passes() {
        let bt = sample_behavior_tree();
        let errors = bt.validate();
        assert!(
            errors.is_empty(),
            "expected no validation errors for valid BT, got: {errors:?}"
        );

        let fsm = sample_state_machine();
        let errors = fsm.validate();
        assert!(
            errors.is_empty(),
            "expected no validation errors for valid FSM, got: {errors:?}"
        );
    }

    #[test]
    fn logic_asset_validation_fails_missing_entry() {
        let mut asset = sample_behavior_tree();
        asset.entry_node = Some("nonexistent_entry".into());
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("nonexistent_entry")),
            "expected error about missing entry node, got: {errors:?}"
        );
    }

    #[test]
    fn logic_asset_validation_fails_broken_transition() {
        let mut asset = sample_behavior_tree();
        // Point the "move_to_point" transition to a non-existent node.
        if let Some(node) = asset.nodes.iter_mut().find(|n| n.id == "move_to_point") {
            if let Some(t) = node.transitions.first_mut() {
                t.target_node = "ghost_node".into();
            }
        }
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("ghost_node")),
            "expected error about missing transition target, got: {errors:?}"
        );
    }

    #[test]
    fn logic_asset_validation_fails_empty_nodes() {
        let asset = LogicAsset {
            schema_version: SchemaVersion::new(0, 1, 0),
            kind: LogicKind::BehaviorTree,
            entry_node: None,
            nodes: vec![],
        };
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("at least one node")),
            "expected error about empty nodes, got: {errors:?}"
        );
    }

    #[test]
    fn logic_asset_validation_fails_duplicate_node_id() {
        let mut asset = sample_behavior_tree();
        // Add a second node with id "root".
        asset.nodes.push(LogicNode {
            id: "root".into(),
            node_type: "action".into(),
            parameters: vec![],
            transitions: vec![],
            children: vec![],
        });
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("Duplicate node ID")),
            "expected error about duplicate node ID, got: {errors:?}"
        );
    }

    #[test]
    fn logic_asset_validation_fails_broken_child() {
        let mut asset = sample_behavior_tree();
        if let Some(node) = asset.nodes.iter_mut().find(|n| n.id == "root") {
            node.children.push("missing_child".into());
        }
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("missing_child")),
            "expected error about missing child node, got: {errors:?}"
        );
    }

    #[test]
    fn logic_asset_entry_node_returns_first_when_none() {
        let asset = sample_behavior_tree();
        let entry = asset.entry_node();
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().id, "root");
    }

    #[test]
    fn logic_asset_find_node_returns_none_for_missing() {
        let asset = sample_behavior_tree();
        assert!(asset.find_node("does_not_exist").is_none());
    }

    #[test]
    fn logic_asset_find_node_finds_existing() {
        let asset = sample_behavior_tree();
        let node = asset.find_node("wait");
        assert!(node.is_some());
        assert_eq!(node.unwrap().node_type, "action");
    }

    #[test]
    fn logic_value_variants_roundtrip() {
        let values = vec![
            LogicValue::Bool(true),
            LogicValue::Int(-42),
            LogicValue::Float(std::f64::consts::PI),
            LogicValue::Str("hello".into()),
            LogicValue::Asset(AssetId::new("some_asset")),
        ];

        for value in values {
            let json = serde_json::to_string(&value).unwrap();
            let restored: LogicValue = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&restored).unwrap();
            assert_eq!(json, json2, "LogicValue roundtrip mismatch");
        }
    }

    #[test]
    fn logic_kind_variants_roundtrip() {
        let kinds = vec![
            LogicKind::BehaviorTree,
            LogicKind::StateMachine,
            LogicKind::SkillGraph,
            LogicKind::QuestDialogue,
            LogicKind::AITree,
        ];
        for kind in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            let restored: LogicKind = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&restored).unwrap();
            assert_eq!(json, json2, "LogicKind roundtrip mismatch");
        }
    }

    #[test]
    fn compare_op_variants_roundtrip() {
        let ops = vec![
            CompareOp::Equal,
            CompareOp::NotEqual,
            CompareOp::Less,
            CompareOp::Greater,
            CompareOp::LessOrEqual,
            CompareOp::GreaterOrEqual,
        ];
        for op in ops {
            let json = serde_json::to_string(&op).unwrap();
            let restored: CompareOp = serde_json::from_str(&json).unwrap();
            assert_eq!(op, restored);
        }
    }
}
