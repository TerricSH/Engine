use super::{
    LogicAsset, LogicCondition, LogicNode, LogicParameter, LogicParameterType, LogicValue,
    LOGIC_ASSET_SCHEMA_V2,
};
use std::collections::{BTreeMap, HashMap, HashSet};

impl LogicAsset {
    /// Validate the unified graph, parameter, and hierarchy contract.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.schema_version != LOGIC_ASSET_SCHEMA_V2 {
            errors.push(format!(
                "LogicAsset schema must be {}.{}.{} after migration, got {}.{}.{}",
                LOGIC_ASSET_SCHEMA_V2.major,
                LOGIC_ASSET_SCHEMA_V2.minor,
                LOGIC_ASSET_SCHEMA_V2.patch,
                self.schema_version.major,
                self.schema_version.minor,
                self.schema_version.patch
            ));
        }
        if self.nodes.is_empty() {
            errors.push("Asset must contain at least one node".into());
            return errors;
        }

        let mut node_ids = HashSet::new();
        for node in &self.nodes {
            if node.id.is_empty() {
                errors.push("Logic node ID must not be empty".into());
            } else if !node_ids.insert(node.id.as_str()) {
                errors.push(format!("Duplicate node ID: '{}'", node.id));
            }
            if node.node_type.is_empty() {
                errors.push(format!("Node '{}' has an empty node_type", node.id));
            }
            for (name, value) in &node.properties {
                validate_value(
                    value,
                    &format!("Node '{}' property '{name}'", node.id),
                    &mut errors,
                );
            }
        }

        if let Some(entry) = &self.entry_node {
            if !node_ids.contains(entry.as_str()) {
                errors.push(format!(
                    "Entry node '{entry}' does not exist in the node list"
                ));
            }
        }

        for (name, parameter) in &self.parameters {
            if name.is_empty() || parameter.name.is_empty() {
                errors.push("Logic parameter names must not be empty".into());
            }
            if parameter.name != *name {
                errors.push(format!(
                    "Parameter map key '{name}' does not match declaration name '{}'",
                    parameter.name
                ));
            }
            if let Some(default) = &parameter.default {
                validate_value(default, &format!("Parameter '{name}' default"), &mut errors);
                if !value_matches_parameter_type(default, parameter.param_type) {
                    errors.push(format!(
                        "Parameter '{name}' default does not match {:?}",
                        parameter.param_type
                    ));
                }
            }
        }

        for node in &self.nodes {
            for (index, transition) in node.transitions.iter().enumerate() {
                if !node_ids.contains(transition.target_node.as_str()) {
                    errors.push(format!(
                        "Node '{}', transition {index}: target node '{}' does not exist",
                        node.id, transition.target_node
                    ));
                }
                if let Some(condition) = &transition.condition {
                    validate_condition(condition, &self.parameters, &node.id, &mut errors);
                }
            }
            for child in &node.children {
                if !node_ids.contains(child.as_str()) {
                    errors.push(format!(
                        "Node '{}': child node '{}' does not exist",
                        node.id, child
                    ));
                }
            }
        }

        detect_child_cycles(&self.nodes, &node_ids, &mut errors);
        errors
    }
}

fn value_matches_parameter_type(value: &LogicValue, expected: LogicParameterType) -> bool {
    matches!(
        (value, expected),
        (LogicValue::Bool(_), LogicParameterType::Bool)
            | (LogicValue::Int(_), LogicParameterType::Int)
            | (LogicValue::Float(_), LogicParameterType::Float)
            | (LogicValue::String(_), LogicParameterType::String)
            | (LogicValue::AssetRef(_), LogicParameterType::AssetRef)
            | (LogicValue::EntityRef(_), LogicParameterType::EntityRef)
    )
}

fn validate_value(value: &LogicValue, context: &str, errors: &mut Vec<String>) {
    match value {
        LogicValue::Float(value) if !value.is_finite() => {
            errors.push(format!("{context} contains a non-finite float"));
        }
        LogicValue::AssetRef(asset) if asset.id.is_empty() => {
            errors.push(format!("{context} contains an empty asset reference"));
        }
        LogicValue::EntityRef(entity) if entity.is_empty() => {
            errors.push(format!("{context} contains an empty entity reference"));
        }
        _ => {}
    }
}

fn validate_condition(
    condition: &LogicCondition,
    parameters: &BTreeMap<String, LogicParameter>,
    node_id: &str,
    errors: &mut Vec<String>,
) {
    match condition {
        LogicCondition::Always | LogicCondition::Never => {}
        LogicCondition::BoolParam(name) => match parameters.get(name) {
            None => errors.push(format!(
                "Node '{node_id}': condition references undefined bool parameter '{name}'"
            )),
            Some(parameter) if parameter.param_type != LogicParameterType::Bool => errors.push(
                format!("Node '{node_id}': condition parameter '{name}' is not Bool"),
            ),
            Some(_) => {}
        },
        LogicCondition::Comparison { param, value, .. } => match parameters.get(param) {
            None => errors.push(format!(
                "Node '{node_id}': condition references undefined parameter '{param}'"
            )),
            Some(parameter) if !value_matches_parameter_type(value, parameter.param_type) => {
                errors.push(format!(
                    "Node '{node_id}': comparison value for '{param}' does not match {:?}",
                    parameter.param_type
                ));
            }
            Some(_) => validate_value(
                value,
                &format!("Node '{node_id}' comparison '{param}'"),
                errors,
            ),
        },
        LogicCondition::And(conditions) | LogicCondition::Or(conditions) => {
            if conditions.is_empty() {
                errors.push(format!(
                    "Node '{node_id}': logical condition must not be empty"
                ));
            }
            for condition in conditions {
                validate_condition(condition, parameters, node_id, errors);
            }
        }
        LogicCondition::Not(condition) => {
            validate_condition(condition, parameters, node_id, errors);
        }
        LogicCondition::HasAsset { asset } => {
            if asset.id.is_empty() {
                errors.push(format!(
                    "Node '{node_id}': HasAsset condition contains an empty asset reference"
                ));
            }
        }
    }
}

fn detect_child_cycles(nodes: &[LogicNode], node_ids: &HashSet<&str>, errors: &mut Vec<String>) {
    let index_by_id: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect();
    let mut colors = vec![0_u8; nodes.len()];
    let mut path = Vec::new();

    fn visit(
        index: usize,
        nodes: &[LogicNode],
        node_ids: &HashSet<&str>,
        index_by_id: &HashMap<&str, usize>,
        colors: &mut [u8],
        path: &mut Vec<String>,
        errors: &mut Vec<String>,
    ) {
        colors[index] = 1;
        path.push(nodes[index].id.clone());
        for child in &nodes[index].children {
            if !node_ids.contains(child.as_str()) {
                continue;
            }
            let Some(&child_index) = index_by_id.get(child.as_str()) else {
                continue;
            };
            match colors[child_index] {
                0 => visit(
                    child_index,
                    nodes,
                    node_ids,
                    index_by_id,
                    colors,
                    path,
                    errors,
                ),
                1 => {
                    let start = path
                        .iter()
                        .position(|node| node == child)
                        .unwrap_or_default();
                    let mut cycle = path[start..].to_vec();
                    cycle.push(child.clone());
                    errors.push(format!(
                        "Circular child dependency detected: {}",
                        cycle.join(" -> ")
                    ));
                }
                _ => {}
            }
        }
        path.pop();
        colors[index] = 2;
    }

    for index in 0..nodes.len() {
        if colors[index] == 0 {
            visit(
                index,
                nodes,
                node_ids,
                &index_by_id,
                &mut colors,
                &mut path,
                errors,
            );
        }
    }
}
