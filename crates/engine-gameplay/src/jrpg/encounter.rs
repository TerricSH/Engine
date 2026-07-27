use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::DeterministicRng;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnemyFormation {
    pub id: String,
    pub enemy_ids: Vec<String>,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default)]
    pub battle_scene: Option<String>,
    #[serde(default)]
    pub battle_music: Option<String>,
}

fn default_weight() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterTable {
    pub id: String,
    #[serde(default)]
    pub formations: Vec<EnemyFormation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EncounterError {
    #[error("encounter table '{0}' has no positive-weight formation")]
    EmptyTable(String),
    #[error("encounter meter threshold must be greater than zero")]
    InvalidThreshold,
}

impl EncounterTable {
    pub fn choose<'a>(
        &'a self,
        random: &mut DeterministicRng,
    ) -> Result<&'a EnemyFormation, EncounterError> {
        let total = self
            .formations
            .iter()
            .fold(0_u32, |sum, formation| sum.saturating_add(formation.weight));
        if total == 0 {
            return Err(EncounterError::EmptyTable(self.id.clone()));
        }
        let mut roll = random.range_u32(total);
        self.formations
            .iter()
            .find(|formation| {
                if roll < formation.weight {
                    true
                } else {
                    roll -= formation.weight;
                    false
                }
            })
            .ok_or_else(|| EncounterError::EmptyTable(self.id.clone()))
    }
}

/// Serializable field-encounter accumulator. Projects may feed distance,
/// danger, stealth, or scripted threat into the same deterministic meter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterMeter {
    pub progress: u32,
    pub threshold: u32,
    #[serde(default)]
    pub disabled: bool,
}

impl Default for EncounterMeter {
    fn default() -> Self {
        Self {
            progress: 0,
            threshold: 10_000,
            disabled: false,
        }
    }
}

impl EncounterMeter {
    pub fn advance(&mut self, amount: u32) -> Result<bool, EncounterError> {
        if self.threshold == 0 {
            return Err(EncounterError::InvalidThreshold);
        }
        if self.disabled {
            return Ok(false);
        }
        self.progress = self.progress.saturating_add(amount);
        if self.progress >= self.threshold {
            self.progress %= self.threshold;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn reset(&mut self) {
        self.progress = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_selection_is_deterministic() {
        let table = EncounterTable {
            id: "field".into(),
            formations: vec![
                EnemyFormation {
                    id: "slime".into(),
                    enemy_ids: vec!["slime".into()],
                    weight: 1,
                    battle_scene: None,
                    battle_music: None,
                },
                EnemyFormation {
                    id: "wolves".into(),
                    enemy_ids: vec!["wolf".into(), "wolf".into()],
                    weight: 3,
                    battle_scene: None,
                    battle_music: None,
                },
            ],
        };
        let mut first = DeterministicRng::new(5);
        let mut second = DeterministicRng::new(5);
        for _ in 0..8 {
            assert_eq!(
                table.choose(&mut first).unwrap().id,
                table.choose(&mut second).unwrap().id
            );
        }
    }

    #[test]
    fn encounter_meter_preserves_overflow() {
        let mut meter = EncounterMeter {
            threshold: 100,
            ..EncounterMeter::default()
        };
        assert!(meter.advance(125).unwrap());
        assert_eq!(meter.progress, 25);
    }
}
