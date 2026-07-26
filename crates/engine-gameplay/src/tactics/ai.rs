use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::grid::GridCoord;
use super::types::TacticalEntityId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UtilityOption {
    pub key: String,
    pub actor: TacticalEntityId,
    pub target_entity: Option<TacticalEntityId>,
    pub target_cell: Option<GridCoord>,
    pub base_score: f32,
    pub facts: BTreeMap<String, f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredOption {
    pub option: UtilityOption,
    pub score: f32,
    pub considerations: BTreeMap<String, f32>,
}

pub trait UtilityConsideration {
    fn key(&self) -> &str;
    fn evaluate(&self, option: &UtilityOption) -> f32;
}

/// Composable utility-AI selector. Each consideration is a Strategy and the
/// stable key tie-break keeps decisions deterministic across replays.
#[derive(Default)]
pub struct UtilityBrain {
    considerations: Vec<Box<dyn UtilityConsideration + Send + Sync>>,
}

impl UtilityBrain {
    pub fn add_consideration(
        &mut self,
        consideration: impl UtilityConsideration + Send + Sync + 'static,
    ) {
        self.considerations.push(Box::new(consideration));
    }

    pub fn score(&self, option: UtilityOption) -> ScoredOption {
        let mut score = option.base_score;
        let mut considerations = BTreeMap::new();
        for consideration in &self.considerations {
            let value = consideration.evaluate(&option).clamp(0.0, 1.0);
            score *= value;
            considerations.insert(consideration.key().to_string(), value);
        }
        ScoredOption {
            option,
            score,
            considerations,
        }
    }

    pub fn choose(&self, options: impl IntoIterator<Item = UtilityOption>) -> Option<ScoredOption> {
        options
            .into_iter()
            .map(|option| self.score(option))
            .max_by(|left, right| {
                left.score
                    .partial_cmp(&right.score)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| right.option.key.cmp(&left.option.key))
            })
    }
}

#[derive(Debug, Clone)]
pub struct FactCurve {
    pub key: String,
    pub fact: String,
    pub input_min: f32,
    pub input_max: f32,
    pub invert: bool,
}

impl UtilityConsideration for FactCurve {
    fn key(&self) -> &str {
        &self.key
    }

    fn evaluate(&self, option: &UtilityOption) -> f32 {
        let value = option.facts.get(&self.fact).copied().unwrap_or_default();
        let range = self.input_max - self.input_min;
        let normalized = if range.abs() <= f32::EPSILON {
            0.0
        } else {
            ((value - self.input_min) / range).clamp(0.0, 1.0)
        };
        if self.invert {
            1.0 - normalized
        } else {
            normalized
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(key: &str, danger: f32) -> UtilityOption {
        UtilityOption {
            key: key.to_string(),
            actor: "alien".to_string(),
            target_entity: None,
            target_cell: None,
            base_score: 1.0,
            facts: BTreeMap::from([("danger".to_string(), danger)]),
        }
    }

    #[test]
    fn considerations_are_composable() {
        let mut brain = UtilityBrain::default();
        brain.add_consideration(FactCurve {
            key: "safety".to_string(),
            fact: "danger".to_string(),
            input_min: 0.0,
            input_max: 10.0,
            invert: true,
        });
        assert_eq!(
            brain
                .choose([option("safe", 1.0), option("risky", 9.0)])
                .unwrap()
                .option
                .key,
            "safe"
        );
    }

    #[test]
    fn equal_scores_use_stable_key_order() {
        let brain = UtilityBrain::default();
        assert_eq!(
            brain
                .choose([option("bravo", 0.0), option("alpha", 0.0)])
                .unwrap()
                .option
                .key,
            "alpha"
        );
    }
}
