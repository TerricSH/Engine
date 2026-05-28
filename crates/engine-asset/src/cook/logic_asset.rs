//! Gate 7 Logic Asset Model.
//!
//! Defines the first mobile-safe hot-updatable logic asset type: behavior graphs,
//! state machines, skill graphs, and quest/dialogue trees.  These are pure data
//! structures — no runtime interpreter is provided by this module.
//!
//! # Architecture
//!
//! ```text
//! LogicAsset  ──→  validate()  ──→  cook_logic_asset()
//!                                         │
//!                                    CookedAssetHeader
//!                                         │
//!                                    .cooked artifact
//! ```

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use engine_serialize::{AssetId, SchemaVersion};
use serde::{Deserialize, Serialize};

use super::error::CookError;
use super::{write_cooked_artifact, AssetType, CookResult};

// ── Top-level container ───────────────────────────────────────────────────

/// Top-level logic asset container.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicAsset {
    /// Schema version for forward-compatibility.
    pub schema_version: SchemaVersion,
    /// Human-readable asset identifier.
    pub asset_id: String,
    /// Kind of logic graph this asset represents.
    pub kind: LogicAssetKind,
    /// Nodes in the graph.
    pub nodes: Vec<LogicNode>,
    /// Named parameters exposed by this logic asset.
    pub parameters: BTreeMap<String, LogicParam>,
    /// Metadata (author, description, tags, version).
    pub metadata: LogicMetadata,
}

// ── LogicAssetKind ────────────────────────────────────────────────────────

/// Kind of logic asset.
///
/// This enum is `#[non_exhaustive]` so new kinds can be added without a
/// breaking change.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum LogicAssetKind {
    /// A behaviour tree (composite/decorator/action nodes).
    BehaviorTree,
    /// A hierarchical or flat state machine.
    StateMachine,
    /// A skill/ability graph (e.g. for an ability system).
    SkillGraph,
    /// A quest or dialogue tree.
    QuestDialogue,
}

// ── Nodes ─────────────────────────────────────────────────────────────────

/// A single node in the logic graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicNode {
    /// Unique identifier for this node within the asset.
    pub id: String,
    /// Type discriminator (e.g. "sequence", "selector", "state", "action", …).
    pub node_type: String,
    /// Optional human-readable label.
    pub label: Option<String>,
    /// Outgoing transitions/links to other nodes.
    pub transitions: Vec<LogicTransition>,
    /// Node-scoped properties.
    pub properties: BTreeMap<String, LogicValue>,
    /// Child node IDs (for hierarchical graphs like behaviour trees).
    pub children: Vec<String>,
}

// ── Transitions ───────────────────────────────────────────────────────────

/// A transition between nodes (for state machines) or a sequence link (for
/// behaviour trees).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicTransition {
    /// The target node identifier.
    pub target_node: String,
    /// Optional condition that must be satisfied for the transition to fire.
    pub condition: Option<LogicCondition>,
    /// Priority for resolving conflicting transitions (higher = preferred).
    pub priority: i32,
}

// ── Conditions ────────────────────────────────────────────────────────────

/// A condition that gates a transition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LogicCondition {
    /// Always fires.
    Always,
    /// Never fires.
    Never,
    /// Evaluate a boolean parameter by name.
    BoolParam(String),
    /// Compare a parameter against a value.
    Comparison {
        param: String,
        op: ComparisonOp,
        value: LogicValue,
    },
    /// Logical AND of sub-conditions.
    And(Vec<LogicCondition>),
    /// Logical OR of sub-conditions.
    Or(Vec<LogicCondition>),
    /// Logical NOT of a sub-condition.
    Not(Box<LogicCondition>),
}

/// Comparison operators for conditions.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ComparisonOp {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

// ── Values ────────────────────────────────────────────────────────────────

/// A typed parameter value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LogicValue {
    /// Boolean.
    Bool(bool),
    /// Signed 64-bit integer.
    Int(i64),
    /// 64-bit floating point.
    Float(f64),
    /// UTF-8 string.
    String(String),
    /// Reference to another asset by its [`AssetId`].
    AssetRef(AssetId),
    /// Reference to an entity by string identifier.
    EntityRef(String),
}

// ── Parameters ────────────────────────────────────────────────────────────

/// A named parameter declaration for a logic asset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicParam {
    /// Parameter name.
    pub name: String,
    /// The type of the parameter.
    pub param_type: LogicParamType,
    /// Optional default value.
    pub default: Option<LogicValue>,
    /// Optional human-readable description.
    pub description: Option<String>,
}

/// Supported parameter types.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LogicParamType {
    Bool,
    Int,
    Float,
    String,
    AssetRef,
    EntityRef,
}

// ── Metadata ──────────────────────────────────────────────────────────────

/// Metadata attached to a logic asset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicMetadata {
    /// Author name or identifier.
    pub author: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
    /// Search/filter tags.
    pub tags: Vec<String>,
    /// Asset version string (semver recommended).
    pub version: String,
}

// ── Validation ────────────────────────────────────────────────────────────

impl LogicAsset {
    /// Validate the logic asset structure.
    ///
    /// Returns a list of human-readable error strings.  An empty `Vec` means
    /// the asset is structurally valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();

        // Collect all node IDs.
        let mut node_ids: HashSet<&str> = HashSet::new();
        for node in &self.nodes {
            if !node_ids.insert(node.id.as_str()) {
                errors.push(format!("Duplicate node ID: '{}'", node.id));
            }
        }

        // Collect all parameter names.
        let param_names: HashSet<&str> = self.parameters.keys().map(|s| s.as_str()).collect();

        // Check each node's transitions target existing nodes.
        for node in &self.nodes {
            for (i, transition) in node.transitions.iter().enumerate() {
                if !node_ids.contains(transition.target_node.as_str()) {
                    errors.push(format!(
                        "Node '{}', transition {}: target node '{}' does not exist",
                        node.id, i, transition.target_node
                    ));
                }

                // Check condition parameters reference existing parameters.
                if let Some(ref cond) = transition.condition {
                    self.collect_param_refs(cond, &mut errors, &param_names, &node.id);
                }
            }

            // Check child node IDs exist.
            for child_id in &node.children {
                if !node_ids.contains(child_id.as_str()) {
                    errors.push(format!(
                        "Node '{}': child node '{}' does not exist",
                        node.id, child_id
                    ));
                }
            }
        }

        // Check for cycles in state machines.
        if matches!(self.kind, LogicAssetKind::StateMachine) {
            self.detect_cycles(&node_ids, &mut errors);
        }

        errors
    }

    /// Recursively collect parameter references from a condition tree and
    /// emit errors for any that don't exist in `param_names`.
    fn collect_param_refs(
        &self,
        cond: &LogicCondition,
        errors: &mut Vec<String>,
        param_names: &HashSet<&str>,
        node_id: &str,
    ) {
        match cond {
            LogicCondition::BoolParam(name) => {
                if !param_names.contains(name.as_str()) {
                    errors.push(format!(
                        "Node '{}': condition references undefined bool parameter '{}'",
                        node_id, name
                    ));
                }
            }
            LogicCondition::Comparison { param, .. } => {
                if !param_names.contains(param.as_str()) {
                    errors.push(format!(
                        "Node '{}': condition references undefined parameter '{}'",
                        node_id, param
                    ));
                }
            }
            LogicCondition::And(conds) | LogicCondition::Or(conds) => {
                for c in conds {
                    self.collect_param_refs(c, errors, param_names, node_id);
                }
            }
            LogicCondition::Not(inner) => {
                self.collect_param_refs(inner, errors, param_names, node_id);
            }
            // Always / Never have no parameter references.
            LogicCondition::Always | LogicCondition::Never => {}
        }
    }

    /// Simple cycle detection for state machines using DFS.
    fn detect_cycles(&self, node_ids: &HashSet<&str>, errors: &mut Vec<String>) {
        // Build adjacency list: node_id → list of target_node IDs from transitions.
        let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for id in node_ids.iter() {
            adj.entry(id).or_default();
        }
        for node in &self.nodes {
            for t in &node.transitions {
                if node_ids.contains(t.target_node.as_str()) {
                    adj.entry(node.id.as_str())
                        .or_default()
                        .push(t.target_node.as_str());
                }
            }
        }

        // Standard DFS cycle detection with three-colour marking.
        // Uses indices into a Vec rather than lifetime-heavy BTreeMap borrows
        // to avoid nested-function lifetime issues.
        let node_list: Vec<&str> = node_ids.iter().copied().collect();

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Color {
            White,
            Gray,
            Black,
        }

        let mut color: Vec<Color> = vec![Color::White; node_list.len()];
        let mut dfs_path: Vec<String> = Vec::new();

        // Build adjacency list as index-based Vec.
        let mut adj_idx: Vec<Vec<usize>> = vec![Vec::new(); node_list.len()];
        for node in &self.nodes {
            if let Some(from) = node_list.iter().position(|&id| id == node.id) {
                for t in &node.transitions {
                    if let Some(to) = node_list.iter().position(|&id| *id == t.target_node) {
                        if from != to {
                            adj_idx[from].push(to);
                        }
                    }
                }
            }
        }

        fn dfs_idx(
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
                        // Found a cycle — report it.
                        let cycle_start = path
                            .iter()
                            .position(|n| n.as_str() == node_list[v])
                            .unwrap_or(0);
                        let cycle_path: Vec<&str> = path[cycle_start..]
                            .iter()
                            .map(|s| s.as_str())
                            .collect();
                        errors.push(format!(
                            "StateMachine cycle detected: {}",
                            cycle_path.join(" → ")
                        ));
                    }
                    Color::White => {
                        dfs_idx(v, adj, color, path, node_list, errors);
                    }
                    Color::Black => {}
                }
            }

            path.pop();
            color[u] = Color::Black;
        }

        for i in 0..color.len() {
            if color[i] == Color::White {
                dfs_idx(i, &adj_idx, &mut color, &mut dfs_path, &node_list, errors);
            }
        }
    }
}

// ── Cook entry point ─────────────────────────────────────────────────────

/// Cook a logic asset from a JSON source file.
///
/// 1. Load [`LogicAsset`] from a JSON source file.
/// 2. Validate the asset structure.
/// 3. Serialize with bincode and write a cooked artifact.
///
/// # Parameters
///
/// * `source` – path to the JSON source file.
/// * `output` – path for the cooked `.cooked` file.
///
/// # Returns
///
/// A [`CookResult`] on success, or a [`CookError`] on failure.
pub fn cook_logic_asset(source: &Path, output: &Path) -> Result<CookResult, CookError> {
    // 1. Load LogicAsset from JSON source.
    let source_bytes = std::fs::read_to_string(source)?;
    let asset: LogicAsset = serde_json::from_str(&source_bytes)
        .map_err(|e| CookError::Parse(format!("failed to parse logic asset JSON: {e}")))?;

    // 2. Validate.
    let validation_errors = asset.validate();
    if !validation_errors.is_empty() {
        let msg = format!(
            "logic asset '{}' validation failed:\n  - {}",
            asset.asset_id,
            validation_errors.join("\n  - ")
        );
        return Err(CookError::InvalidAsset(msg));
    }

    // 3. Serialize with bincode.
    let payload = bincode::serialize(&asset)
        .map_err(|e| CookError::InvalidAsset(format!("bincode serialization failed: {e}")))?;

    // 4. Write cooked artifact with header.
    let result = write_cooked_artifact(
        output,
        AssetType::Logic.kind_code(),
        &payload,
        asset.schema_version,
    )?;

    Ok(result)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    // ── Helpers ────────────────────────────────────────────────────────

    fn sample_behavior_tree() -> LogicAsset {
        LogicAsset {
            schema_version: SchemaVersion::new(0, 1, 0),
            asset_id: "bt_enemy_patrol".into(),
            kind: LogicAssetKind::BehaviorTree,
            nodes: vec![
                LogicNode {
                    id: "root".into(),
                    node_type: "sequence".into(),
                    label: Some("Patrol Sequence".into()),
                    transitions: vec![],
                    properties: BTreeMap::new(),
                    children: vec!["move_to_point".into(), "wait".into()],
                },
                LogicNode {
                    id: "move_to_point".into(),
                    node_type: "action".into(),
                    label: Some("Move to Patrol Point".into()),
                    transitions: vec![LogicTransition {
                        target_node: "root".into(),
                        condition: Some(LogicCondition::Always),
                        priority: 0,
                    }],
                    properties: {
                        let mut m = BTreeMap::new();
                        m.insert(
                            "speed".into(),
                            LogicValue::Float(2.5),
                        );
                        m
                    },
                    children: vec![],
                },
                LogicNode {
                    id: "wait".into(),
                    node_type: "action".into(),
                    label: Some("Wait at Point".into()),
                    transitions: vec![LogicTransition {
                        target_node: "root".into(),
                        condition: Some(LogicCondition::BoolParam("has_waited".into())),
                        priority: 0,
                    }],
                    properties: {
                        let mut m = BTreeMap::new();
                        m.insert("duration".into(), LogicValue::Float(3.0));
                        m
                    },
                    children: vec![],
                },
            ],
            parameters: {
                let mut m = BTreeMap::new();
                m.insert(
                    "has_waited".into(),
                    LogicParam {
                        name: "has_waited".into(),
                        param_type: LogicParamType::Bool,
                        default: Some(LogicValue::Bool(false)),
                        description: Some("Whether the wait action completed".into()),
                    },
                );
                m
            },
            metadata: LogicMetadata {
                author: Some("Engine Team".into()),
                description: Some("Enemy patrol behaviour tree".into()),
                tags: vec!["enemy".into(), "patrol".into(), "ai".into()],
                version: "1.0.0".into(),
            },
        }
    }

    fn sample_state_machine() -> LogicAsset {
        LogicAsset {
            schema_version: SchemaVersion::new(0, 1, 0),
            asset_id: "fsm_door".into(),
            kind: LogicAssetKind::StateMachine,
            nodes: vec![
                LogicNode {
                    id: "closed".into(),
                    node_type: "state".into(),
                    label: Some("Closed".into()),
                    transitions: vec![LogicTransition {
                        target_node: "opening".into(),
                        condition: Some(LogicCondition::BoolParam("button_pressed".into())),
                        priority: 0,
                    }],
                    properties: BTreeMap::new(),
                    children: vec![],
                },
                LogicNode {
                    id: "opening".into(),
                    node_type: "state".into(),
                    label: Some("Opening".into()),
                    transitions: vec![LogicTransition {
                        target_node: "open".into(),
                        condition: Some(LogicCondition::Always),
                        priority: 0,
                    }],
                    properties: BTreeMap::new(),
                    children: vec![],
                },
                LogicNode {
                    id: "open".into(),
                    node_type: "state".into(),
                    label: Some("Open".into()),
                    transitions: vec![LogicTransition {
                        target_node: "closing".into(),
                        condition: Some(LogicCondition::Comparison {
                            param: "open_time".into(),
                            op: ComparisonOp::GreaterOrEqual,
                            value: LogicValue::Float(5.0),
                        }),
                        priority: 0,
                    }],
                    properties: BTreeMap::new(),
                    children: vec![],
                },
                LogicNode {
                    id: "closing".into(),
                    node_type: "state".into(),
                    label: Some("Closing".into()),
                    transitions: vec![LogicTransition {
                        target_node: "closed".into(),
                        condition: Some(LogicCondition::Always),
                        priority: 0,
                    }],
                    properties: BTreeMap::new(),
                    children: vec![],
                },
            ],
            parameters: {
                let mut m = BTreeMap::new();
                m.insert(
                    "button_pressed".into(),
                    LogicParam {
                        name: "button_pressed".into(),
                        param_type: LogicParamType::Bool,
                        default: Some(LogicValue::Bool(false)),
                        description: Some("Whether the door button was pressed".into()),
                    },
                );
                m.insert(
                    "open_time".into(),
                    LogicParam {
                        name: "open_time".into(),
                        param_type: LogicParamType::Float,
                        default: Some(LogicValue::Float(0.0)),
                        description: Some("Seconds the door has been open".into()),
                    },
                );
                m
            },
            metadata: LogicMetadata {
                author: Some("Engine Team".into()),
                description: Some("Door state machine".into()),
                tags: vec!["door".into(), "fsm".into()],
                version: "1.0.0".into(),
            },
        }
    }

    // ── Round-trip tests ───────────────────────────────────────────────

    #[test]
    fn logic_asset_serde_roundtrip() {
        let asset = sample_behavior_tree();
        let json = serde_json::to_string_pretty(&asset).unwrap();
        let restored: LogicAsset = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.asset_id, "bt_enemy_patrol");
        assert!(matches!(restored.kind, LogicAssetKind::BehaviorTree));
        assert_eq!(restored.nodes.len(), 3);
        assert_eq!(restored.parameters.len(), 1);
        assert_eq!(restored.metadata.author.as_deref(), Some("Engine Team"));

        // Verify a property round-trip.
        let move_node = restored
            .nodes
            .iter()
            .find(|n| n.id == "move_to_point")
            .expect("move_to_point node missing");
        let speed = move_node.properties.get("speed").expect("speed property");
        assert!(matches!(speed, LogicValue::Float(v) if (*v - 2.5).abs() < 1e-10));
    }

    #[test]
    fn state_machine_serde_roundtrip() {
        let asset = sample_state_machine();
        let json = serde_json::to_string_pretty(&asset).unwrap();
        let restored: LogicAsset = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.asset_id, "fsm_door");
        assert!(matches!(restored.kind, LogicAssetKind::StateMachine));
        assert_eq!(restored.nodes.len(), 4);
        assert_eq!(restored.parameters.len(), 2);
    }

    // ── Validation tests ───────────────────────────────────────────────

    #[test]
    fn valid_behavior_tree_passes_validation() {
        let asset = sample_behavior_tree();
        let errors = asset.validate();
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn valid_state_machine_passes_validation() {
        let asset = sample_state_machine();
        let errors = asset.validate();
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn broken_transition_target_detected() {
        let mut asset = sample_behavior_tree();
        // Point the "move_to_point" transition to a non-existent node.
        asset.nodes[1].transitions[0].target_node = "non_existent_node".into();
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("non_existent_node")),
            "expected error about missing target node, got: {errors:?}"
        );
    }

    #[test]
    fn duplicate_node_id_detected() {
        let mut asset = sample_behavior_tree();
        // Add a duplicate "root" node.
        asset.nodes.push(LogicNode {
            id: "root".into(),
            node_type: "action".into(),
            label: None,
            transitions: vec![],
            properties: BTreeMap::new(),
            children: vec![],
        });
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("Duplicate node ID")),
            "expected error about duplicate node ID, got: {errors:?}"
        );
    }

    #[test]
    fn undefined_parameter_in_condition_detected() {
        let mut asset = sample_behavior_tree();
        // Reference a param that does not exist.
        asset.nodes[2].transitions[0].condition =
            Some(LogicCondition::BoolParam("nonexistent_param".into()));
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("nonexistent_param")),
            "expected error about undefined parameter, got: {errors:?}"
        );
    }

    #[test]
    fn broken_child_reference_detected() {
        let mut asset = sample_behavior_tree();
        asset.nodes[0].children.push("missing_child".into());
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("missing_child")),
            "expected error about missing child node, got: {errors:?}"
        );
    }

    #[test]
    fn state_machine_cycle_detected() {
        let mut asset = sample_state_machine();
        // Introduce a cycle: closed → opening → closed (direct back-edge).
        asset.nodes[1].transitions[0].target_node = "closed".into();
        let errors = asset.validate();
        assert!(
            errors.iter().any(|e| e.contains("cycle")),
            "expected cycle detection error, got: {errors:?}"
        );
    }

    #[test]
    fn non_state_machine_cycle_allowed() {
        // Behavior trees may have loops (they're not strictly acyclic).
        let asset = sample_behavior_tree();
        // Both move_to_point and wait already link back to root — that's fine.
        let errors = asset.validate();
        assert!(errors.is_empty(), "expected no errors for BT with back-edges, got: {errors:?}");
    }

    // ── Cook tests ─────────────────────────────────────────────────────

    #[test]
    fn cook_logic_asset_writes_valid_artifact() {
        let dir = std::env::temp_dir().join("cook_test_logic_asset");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let source_path = dir.join("bt_enemy_patrol.json");
        let cooked_path = dir.join("bt_enemy_patrol.cooked");

        // Write source JSON.
        let asset = sample_behavior_tree();
        let json = serde_json::to_string_pretty(&asset).unwrap();
        std::fs::write(&source_path, &json).unwrap();

        // Cook.
        let result = cook_logic_asset(&source_path, &cooked_path).unwrap();
        assert!(result.success);
        assert_eq!(result.asset_type, AssetType::Logic);

        // Read back and verify header + payload.
        let mut file = std::fs::File::open(&cooked_path).unwrap();
        let mut file_bytes = Vec::new();
        file.read_to_end(&mut file_bytes).unwrap();

        let header: crate::cook::CookedAssetHeader =
            bincode::deserialize(&file_bytes[..]).unwrap();
        assert!(header.is_valid());
        assert_eq!(header.asset_kind, AssetType::Logic.kind_code());

        // Deserialize payload.
        let header_size = bincode::serialized_size(&header).unwrap() as usize;
        let payload = &file_bytes[header_size..];
        let restored: LogicAsset = bincode::deserialize(payload).unwrap();
        assert_eq!(restored.asset_id, "bt_enemy_patrol");
        assert_eq!(restored.nodes.len(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cook_invalid_logic_asset_returns_error() {
        let dir = std::env::temp_dir().join("cook_test_invalid_logic");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let source_path = dir.join("invalid.json");
        let cooked_path = dir.join("invalid.cooked");

        // Write an asset with a broken transition target.
        let mut asset = sample_behavior_tree();
        asset.nodes[1].transitions[0].target_node = "ghost".into();
        let json = serde_json::to_string_pretty(&asset).unwrap();
        std::fs::write(&source_path, &json).unwrap();

        // Cooking should fail validation.
        let result = cook_logic_asset(&source_path, &cooked_path);
        assert!(result.is_err(), "expected cook to fail for invalid asset");
        match result {
            Err(CookError::InvalidAsset(msg)) => {
                assert!(msg.contains("ghost"), "error should mention missing node");
            }
            other => panic!("expected InvalidAsset error, got: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn logic_asset_kind_display() {
        // Verify the kinds serialize/deserialize with their variant names.
        let kinds = vec![
            LogicAssetKind::BehaviorTree,
            LogicAssetKind::StateMachine,
            LogicAssetKind::SkillGraph,
            LogicAssetKind::QuestDialogue,
        ];
        for kind in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            let restored: LogicAssetKind = serde_json::from_str(&json).unwrap();
            // Can't easily assert variant equality without PartialEq,
            // but we can serialize again and verify the string matches.
            let json2 = serde_json::to_string(&restored).unwrap();
            assert_eq!(json, json2, "kind roundtrip mismatch");
        }
    }

    #[test]
    fn logic_value_variants_roundtrip() {
        let values = vec![
            LogicValue::Bool(true),
            LogicValue::Int(-42),
            LogicValue::Float(3.14),
            LogicValue::String("hello".into()),
            LogicValue::AssetRef(AssetId::new("some_asset")),
            LogicValue::EntityRef("player".into()),
        ];

        for value in values {
            let json = serde_json::to_string(&value).unwrap();
            let restored: LogicValue = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&restored).unwrap();
            assert_eq!(json, json2, "LogicValue roundtrip mismatch");
        }
    }
}
