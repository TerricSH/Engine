use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    AbilityDefinition, BattleEffect, DialogueGraph, DialogueNode, EncounterTable, ItemDefinition,
    LocalizationCatalog, NarrativeCommand, QuestDefinition, SequenceCommand, SequenceDefinition,
    SequenceStep, StatBlock, StatusDefinition,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorDefinition {
    pub id: String,
    pub display_name_key: String,
    pub base_stats: StatBlock,
    #[serde(default)]
    pub starting_abilities: Vec<String>,
    #[serde(default)]
    pub field_prefab: Option<String>,
    #[serde(default)]
    pub battle_prefab: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnemyDefinition {
    pub id: String,
    pub display_name_key: String,
    pub stats: StatBlock,
    #[serde(default)]
    pub abilities: Vec<String>,
    #[serde(default)]
    pub reward_experience: u64,
    #[serde(default)]
    pub reward_currency: u64,
    #[serde(default)]
    pub battle_prefab: Option<String>,
}

/// Typed, validated view over project-authored Logic assets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JrpgDatabase {
    #[serde(default)]
    pub actors: BTreeMap<String, ActorDefinition>,
    #[serde(default)]
    pub enemies: BTreeMap<String, EnemyDefinition>,
    #[serde(default)]
    pub items: BTreeMap<String, ItemDefinition>,
    #[serde(default)]
    pub abilities: BTreeMap<String, AbilityDefinition>,
    #[serde(default)]
    pub statuses: BTreeMap<String, StatusDefinition>,
    #[serde(default)]
    pub quests: BTreeMap<String, QuestDefinition>,
    #[serde(default)]
    pub dialogues: BTreeMap<String, DialogueGraph>,
    #[serde(default)]
    pub encounters: BTreeMap<String, EncounterTable>,
    #[serde(default)]
    pub localizations: BTreeMap<String, LocalizationCatalog>,
    #[serde(default)]
    pub sequences: BTreeMap<String, SequenceDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DatabaseError {
    #[error("{kind} map key '{key}' does not match definition id '{id}'")]
    KeyMismatch {
        kind: &'static str,
        key: String,
        id: String,
    },
    #[error("{owner} references missing {kind} '{id}'")]
    MissingReference {
        owner: String,
        kind: &'static str,
        id: String,
    },
    #[error("invalid definition: {0}")]
    Invalid(String),
    #[error("invalid JRPG database JSON: {0}")]
    InvalidJson(String),
}

impl JrpgDatabase {
    pub fn from_json(json: &str) -> Result<Self, DatabaseError> {
        let database: Self = serde_json::from_str(json)
            .map_err(|error| DatabaseError::InvalidJson(error.to_string()))?;
        database.validate()?;
        Ok(database)
    }

    pub fn to_json(&self) -> Result<String, DatabaseError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|error| DatabaseError::InvalidJson(error.to_string()))
    }

    pub fn validate(&self) -> Result<(), DatabaseError> {
        validate_keys("actor", &self.actors, |definition| &definition.id)?;
        validate_keys("enemy", &self.enemies, |definition| &definition.id)?;
        validate_keys("item", &self.items, |definition| &definition.id)?;
        validate_keys("ability", &self.abilities, |definition| &definition.id)?;
        validate_keys("status", &self.statuses, |definition| &definition.id)?;
        validate_keys("quest", &self.quests, |definition| &definition.id)?;
        validate_keys("dialogue", &self.dialogues, |definition| &definition.id)?;
        validate_keys("encounter", &self.encounters, |definition| &definition.id)?;
        validate_keys("sequence", &self.sequences, |definition| &definition.id)?;

        for item in self.items.values() {
            item.validate()
                .map_err(|error| DatabaseError::Invalid(error.to_string()))?;
            if let Some(ability) = &item.consumable_ability {
                require_reference(
                    &self.abilities,
                    &format!("item '{}'", item.id),
                    "ability",
                    ability,
                )?;
            }
            if let Some(equipment) = &item.equipment {
                for ability in &equipment.granted_abilities {
                    require_reference(
                        &self.abilities,
                        &format!("item '{}'", item.id),
                        "ability",
                        ability,
                    )?;
                }
            }
        }
        for ability in self.abilities.values() {
            ability
                .validate()
                .map_err(|error| DatabaseError::Invalid(error.to_string()))?;
            for effect in &ability.effects {
                validate_battle_effect(
                    effect,
                    &format!("ability '{}'", ability.id),
                    &self.statuses,
                )?;
            }
        }
        for status in self.statuses.values() {
            if status.default_duration == 0 || status.max_stacks == 0 {
                return Err(DatabaseError::Invalid(format!(
                    "status '{}' must have positive duration and stack limit",
                    status.id
                )));
            }
            for effect in &status.periodic_effects {
                validate_battle_effect(effect, &format!("status '{}'", status.id), &self.statuses)?;
            }
        }
        for actor in self.actors.values() {
            for ability in &actor.starting_abilities {
                require_reference(
                    &self.abilities,
                    &format!("actor '{}'", actor.id),
                    "ability",
                    ability,
                )?;
            }
        }
        for enemy in self.enemies.values() {
            for ability in &enemy.abilities {
                require_reference(
                    &self.abilities,
                    &format!("enemy '{}'", enemy.id),
                    "ability",
                    ability,
                )?;
            }
        }
        for table in self.encounters.values() {
            let mut formation_ids = BTreeSet::new();
            for formation in &table.formations {
                if formation.id.is_empty()
                    || !formation_ids.insert(formation.id.clone())
                    || formation.weight == 0
                    || formation.enemy_ids.is_empty()
                {
                    return Err(DatabaseError::Invalid(format!(
                        "formation '{}' must have a unique id, enemies, and positive weight",
                        formation.id
                    )));
                }
                for enemy in &formation.enemy_ids {
                    require_reference(
                        &self.enemies,
                        &format!("formation '{}'", formation.id),
                        "enemy",
                        enemy,
                    )?;
                }
            }
        }
        for quest in self.quests.values() {
            let mut objective_ids = BTreeSet::new();
            for objective in &quest.objectives {
                if objective.id.is_empty()
                    || objective.target_count == 0
                    || !objective_ids.insert(objective.id.clone())
                {
                    return Err(DatabaseError::Invalid(format!(
                        "quest '{}' objective '{}' must be unique and have a positive target",
                        quest.id, objective.id
                    )));
                }
            }
            for command in &quest.completion_commands {
                if let NarrativeCommand::GrantItem { item_id, .. } = command {
                    require_reference(
                        &self.items,
                        &format!("quest '{}'", quest.id),
                        "item",
                        item_id,
                    )?;
                }
            }
        }
        for graph in self.dialogues.values() {
            if !graph.nodes.contains_key(&graph.entry) {
                return Err(DatabaseError::Invalid(format!(
                    "dialogue '{}' entry '{}' does not exist",
                    graph.id, graph.entry
                )));
            }
            for (node_id, node) in &graph.nodes {
                let referenced = match node {
                    DialogueNode::Line { next, .. } | DialogueNode::Effects { next, .. } => {
                        vec![next]
                    }
                    DialogueNode::Choice { choices, .. } => {
                        choices.iter().map(|choice| &choice.next).collect()
                    }
                    DialogueNode::Branch { branches, fallback } => branches
                        .iter()
                        .map(|(_, next)| next)
                        .chain(std::iter::once(fallback))
                        .collect(),
                    DialogueNode::End => Vec::new(),
                };
                for next in referenced {
                    if !graph.nodes.contains_key(next) {
                        return Err(DatabaseError::Invalid(format!(
                            "dialogue '{}' node '{}' references missing node '{}'",
                            graph.id, node_id, next
                        )));
                    }
                }
            }
        }
        for (catalog_id, catalog) in &self.localizations {
            if catalog.fallback_locale.is_empty()
                || !catalog.locales.contains_key(&catalog.fallback_locale)
            {
                return Err(DatabaseError::Invalid(format!(
                    "localization catalog '{catalog_id}' has no fallback locale '{}'",
                    catalog.fallback_locale
                )));
            }
        }
        for sequence in self.sequences.values() {
            for step in &sequence.steps {
                if let SequenceStep::Emit { command } = step {
                    validate_sequence_command(command, &sequence.id)?;
                }
            }
        }
        Ok(())
    }
}

fn validate_keys<T>(
    kind: &'static str,
    values: &BTreeMap<String, T>,
    id: impl Fn(&T) -> &String,
) -> Result<(), DatabaseError> {
    for (key, value) in values {
        if key.is_empty() || id(value).is_empty() || key != id(value) {
            return Err(DatabaseError::KeyMismatch {
                kind,
                key: key.clone(),
                id: id(value).clone(),
            });
        }
    }
    Ok(())
}

fn validate_battle_effect(
    effect: &BattleEffect,
    owner: &str,
    statuses: &BTreeMap<String, StatusDefinition>,
) -> Result<(), DatabaseError> {
    match effect {
        BattleEffect::Damage { power, .. } | BattleEffect::Heal { power } if *power <= 0 => Err(
            DatabaseError::Invalid(format!("{owner} has a non-positive effect power")),
        ),
        BattleEffect::AddStatus {
            status_id,
            chance_basis_points,
        } => {
            if *chance_basis_points > 10_000 {
                return Err(DatabaseError::Invalid(format!(
                    "{owner} status chance exceeds 10000 basis points"
                )));
            }
            require_reference(statuses, owner, "status", status_id)
        }
        BattleEffect::RemoveStatus { status_id } => {
            require_reference(statuses, owner, "status", status_id)
        }
        BattleEffect::Revive {
            health_basis_points,
        } if *health_basis_points == 0 || *health_basis_points > 10_000 => Err(
            DatabaseError::Invalid(format!("{owner} revive percentage must be in 1..=10000")),
        ),
        _ => Ok(()),
    }
}

fn validate_sequence_command(
    command: &SequenceCommand,
    sequence_id: &str,
) -> Result<(), DatabaseError> {
    let valid = match command {
        SequenceCommand::PlayAnimation {
            entity_id,
            clip_asset,
            speed,
            ..
        } => !entity_id.is_empty() && !clip_asset.is_empty() && speed.is_finite() && *speed > 0.0,
        SequenceCommand::PlayAudio {
            entity_id,
            clip_asset,
            volume,
            ..
        } => {
            !entity_id.is_empty()
                && !clip_asset.is_empty()
                && volume.is_finite()
                && (0.0..=1.0).contains(volume)
        }
        SequenceCommand::ActivateCamera {
            camera_entity_id, ..
        } => !camera_entity_id.is_empty(),
        SequenceCommand::Fade { from, to, .. } => {
            from.is_finite()
                && to.is_finite()
                && (0.0..=1.0).contains(from)
                && (0.0..=1.0).contains(to)
        }
        SequenceCommand::StartDialogue { dialogue_id } => !dialogue_id.is_empty(),
        SequenceCommand::LoadScene { scene_id } => !scene_id.is_empty(),
        SequenceCommand::Custom { command, .. } => !command.is_empty(),
    };
    if valid {
        Ok(())
    } else {
        Err(DatabaseError::Invalid(format!(
            "sequence '{sequence_id}' contains an invalid command"
        )))
    }
}

fn require_reference<T>(
    values: &BTreeMap<String, T>,
    owner: &str,
    kind: &'static str,
    id: &str,
) -> Result<(), DatabaseError> {
    if values.contains_key(id) {
        Ok(())
    } else {
        Err(DatabaseError::MissingReference {
            owner: owner.into(),
            kind,
            id: id.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jrpg::{DamageKind, Element, TargetRule};

    #[test]
    fn database_rejects_dangling_references() {
        let database = JrpgDatabase {
            actors: BTreeMap::from([(
                "hero".into(),
                ActorDefinition {
                    id: "hero".into(),
                    display_name_key: "actor.hero".into(),
                    base_stats: StatBlock::default(),
                    starting_abilities: vec!["missing".into()],
                    field_prefab: None,
                    battle_prefab: None,
                },
            )]),
            ..JrpgDatabase::default()
        };
        assert!(matches!(
            database.validate(),
            Err(DatabaseError::MissingReference { .. })
        ));
    }

    #[test]
    fn valid_database_round_trips() {
        let database = JrpgDatabase {
            abilities: BTreeMap::from([(
                "attack".into(),
                AbilityDefinition {
                    id: "attack".into(),
                    display_name: "Attack".into(),
                    mp_cost: 0,
                    accuracy_basis_points: 10_000,
                    critical_basis_points: 0,
                    target_rule: TargetRule::SingleEnemy,
                    effects: vec![BattleEffect::Damage {
                        power: 100,
                        kind: DamageKind::Physical,
                        element: Element::Neutral,
                    }],
                },
            )]),
            ..JrpgDatabase::default()
        };
        let json = database.to_json().unwrap();
        assert_eq!(JrpgDatabase::from_json(&json).unwrap(), database);
    }
}
