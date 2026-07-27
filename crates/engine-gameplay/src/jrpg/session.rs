use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::DeterministicRng;

use super::{
    BattleError, BattlePhase, BattleSession, BattleSide, BattleUnit, CharacterProgress,
    DatabaseError, EncounterError, EncounterMeter, Inventory, InventoryError, JrpgDatabase,
    NarrativeCommand, Party, PartyError, QuadraticExperienceCurve, StatModifier, StoryState,
};

pub const JRPG_SESSION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleRewards {
    pub experience: u64,
    pub currency: u64,
    #[serde(default)]
    pub items: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JrpgSession {
    pub schema_version: u32,
    pub party: Party,
    pub inventory: Inventory,
    pub story: StoryState,
    #[serde(default)]
    pub encounter_meters: BTreeMap<String, EncounterMeter>,
    #[serde(default)]
    pub battle: Option<BattleSession>,
    pub random: DeterministicRng,
}

impl JrpgSession {
    pub fn new(seed: u64) -> Self {
        Self {
            schema_version: JRPG_SESSION_SCHEMA_VERSION,
            party: Party::default(),
            inventory: Inventory::default(),
            story: StoryState::default(),
            encounter_meters: BTreeMap::new(),
            battle: None,
            random: DeterministicRng::new(seed),
        }
    }

    pub fn to_json(&self) -> Result<String, JrpgSessionError> {
        serde_json::to_string(self).map_err(|error| JrpgSessionError::Json(error.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, JrpgSessionError> {
        let session: Self = serde_json::from_str(json)
            .map_err(|error| JrpgSessionError::Json(error.to_string()))?;
        if session.schema_version != JRPG_SESSION_SCHEMA_VERSION {
            return Err(JrpgSessionError::UnsupportedSchema(session.schema_version));
        }
        session.party.validate()?;
        Ok(session)
    }

    pub fn recruit(
        &mut self,
        database: &JrpgDatabase,
        actor_id: &str,
        prefer_active: bool,
    ) -> Result<(), JrpgSessionError> {
        let definition = database
            .actors
            .get(actor_id)
            .ok_or_else(|| JrpgSessionError::UnknownActor(actor_id.into()))?;
        let mut actor = CharacterProgress::new(
            definition.id.clone(),
            definition.display_name_key.clone(),
            definition.base_stats,
        );
        actor
            .learned_abilities
            .extend(definition.starting_abilities.iter().cloned());
        self.party.recruit(actor, prefer_active)?;
        Ok(())
    }

    pub fn roll_encounter(
        &mut self,
        database: &JrpgDatabase,
        table_id: &str,
        amount: u32,
    ) -> Result<Option<String>, JrpgSessionError> {
        let table = database
            .encounters
            .get(table_id)
            .ok_or_else(|| JrpgSessionError::UnknownEncounter(table_id.into()))?;
        let meter = self.encounter_meters.entry(table_id.into()).or_default();
        if !meter.advance(amount)? {
            return Ok(None);
        }
        Ok(Some(table.choose(&mut self.random)?.id.clone()))
    }

    pub fn begin_formation(
        &mut self,
        database: &JrpgDatabase,
        table_id: &str,
        formation_id: &str,
    ) -> Result<(), JrpgSessionError> {
        if self.battle.is_some() {
            return Err(JrpgSessionError::BattleAlreadyActive);
        }
        let table = database
            .encounters
            .get(table_id)
            .ok_or_else(|| JrpgSessionError::UnknownEncounter(table_id.into()))?;
        let formation = table
            .formations
            .iter()
            .find(|formation| formation.id == formation_id)
            .ok_or_else(|| JrpgSessionError::UnknownFormation(formation_id.into()))?;

        let mut units = Vec::new();
        for actor_id in self.party.active_ids() {
            let actor = self
                .party
                .actor(actor_id)
                .ok_or_else(|| PartyError::UnknownActor(actor_id.clone()))?;
            let modifiers = actor.equipment.values().filter_map(|item_id| {
                database
                    .items
                    .get(item_id)
                    .and_then(|item| item.equipment.as_ref())
            });
            let flat_modifiers: Vec<&StatModifier> = modifiers
                .flat_map(|equipment| equipment.modifiers.iter())
                .collect();
            units.push(BattleUnit::new(
                actor.actor_id.clone(),
                actor.actor_id.clone(),
                BattleSide::Party,
                actor.resolved_stats(flat_modifiers),
            ));
        }
        let mut instance_counts = BTreeMap::<String, u32>::new();
        for enemy_id in &formation.enemy_ids {
            let enemy = database
                .enemies
                .get(enemy_id)
                .ok_or_else(|| JrpgSessionError::UnknownEnemy(enemy_id.clone()))?;
            let index = instance_counts.entry(enemy_id.clone()).or_default();
            *index = index.saturating_add(1);
            let instance_id = format!("enemy.{enemy_id}.{}", *index);
            let mut unit = BattleUnit::new(
                instance_id,
                enemy.id.clone(),
                BattleSide::Enemy,
                enemy.stats,
            );
            unit.reward_experience = enemy.reward_experience;
            unit.reward_currency = enemy.reward_currency;
            units.push(unit);
        }
        self.battle = Some(BattleSession::new(units)?);
        Ok(())
    }

    pub fn claim_battle_rewards(&mut self) -> Result<BattleRewards, JrpgSessionError> {
        let battle = self
            .battle
            .as_ref()
            .ok_or(JrpgSessionError::NoActiveBattle)?;
        if battle.phase != BattlePhase::PartyVictory {
            return Err(JrpgSessionError::BattleNotWon);
        }
        let rewards = BattleRewards {
            experience: battle
                .units
                .values()
                .filter(|unit| unit.side == BattleSide::Enemy)
                .fold(0_u64, |sum, unit| {
                    sum.saturating_add(unit.reward_experience)
                }),
            currency: battle
                .units
                .values()
                .filter(|unit| unit.side == BattleSide::Enemy)
                .fold(0_u64, |sum, unit| sum.saturating_add(unit.reward_currency)),
            items: BTreeMap::new(),
        };
        self.party.grant_currency(rewards.currency)?;
        let active = self.party.active_ids().to_vec();
        for actor_id in active {
            if let Some(actor) = self.party.actor_mut(&actor_id) {
                let _ = actor.grant_experience(
                    rewards.experience,
                    99,
                    &QuadraticExperienceCurve::default(),
                );
            }
        }
        self.battle = None;
        Ok(rewards)
    }

    /// Applies commands owned by the generic session and returns project/
    /// runtime commands (scene loads, sequences, custom hooks) to the caller.
    pub fn apply_narrative_commands(
        &mut self,
        database: &JrpgDatabase,
        commands: impl IntoIterator<Item = NarrativeCommand>,
    ) -> Result<Vec<NarrativeCommand>, JrpgSessionError> {
        let mut external = Vec::new();
        for command in commands {
            match command {
                NarrativeCommand::GrantItem { item_id, quantity } => {
                    let definition = database
                        .items
                        .get(&item_id)
                        .ok_or_else(|| InventoryError::UnknownItem(item_id.clone()))?;
                    let _ = self.inventory.add(definition, quantity)?;
                }
                NarrativeCommand::GrantCurrency { amount } => {
                    let _ = self.party.grant_currency(amount)?;
                }
                NarrativeCommand::GrantExperience { amount } => {
                    let active = self.party.active_ids().to_vec();
                    for actor_id in active {
                        if let Some(actor) = self.party.actor_mut(&actor_id) {
                            let _ = actor.grant_experience(
                                amount,
                                99,
                                &QuadraticExperienceCurve::default(),
                            );
                        }
                    }
                }
                other => external.push(other),
            }
        }
        Ok(external)
    }
}

impl Default for JrpgSession {
    fn default() -> Self {
        Self::new(0)
    }
}

#[derive(Debug, Error)]
pub enum JrpgSessionError {
    #[error("unsupported JRPG session schema version {0}")]
    UnsupportedSchema(u32),
    #[error("JRPG session JSON error: {0}")]
    Json(String),
    #[error("unknown actor '{0}'")]
    UnknownActor(String),
    #[error("unknown enemy '{0}'")]
    UnknownEnemy(String),
    #[error("unknown encounter table '{0}'")]
    UnknownEncounter(String),
    #[error("unknown formation '{0}'")]
    UnknownFormation(String),
    #[error("a battle is already active")]
    BattleAlreadyActive,
    #[error("there is no active battle")]
    NoActiveBattle,
    #[error("battle rewards require a party victory")]
    BattleNotWon,
    #[error(transparent)]
    Party(#[from] PartyError),
    #[error(transparent)]
    Inventory(#[from] InventoryError),
    #[error(transparent)]
    Encounter(#[from] EncounterError),
    #[error(transparent)]
    Battle(#[from] BattleError),
    #[error(transparent)]
    Database(#[from] DatabaseError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jrpg::{
        ActorDefinition, EncounterTable, EnemyDefinition, EnemyFormation, StatBlock,
    };

    fn database() -> JrpgDatabase {
        JrpgDatabase {
            actors: BTreeMap::from([(
                "hero".into(),
                ActorDefinition {
                    id: "hero".into(),
                    display_name_key: "actor.hero".into(),
                    base_stats: StatBlock {
                        max_hp: 100,
                        speed: 10,
                        ..StatBlock::default()
                    },
                    starting_abilities: vec![],
                    field_prefab: None,
                    battle_prefab: None,
                },
            )]),
            enemies: BTreeMap::from([(
                "slime".into(),
                EnemyDefinition {
                    id: "slime".into(),
                    display_name_key: "enemy.slime".into(),
                    stats: StatBlock {
                        max_hp: 10,
                        ..StatBlock::default()
                    },
                    abilities: vec![],
                    reward_experience: 12,
                    reward_currency: 3,
                    battle_prefab: None,
                },
            )]),
            encounters: BTreeMap::from([(
                "field".into(),
                EncounterTable {
                    id: "field".into(),
                    formations: vec![EnemyFormation {
                        id: "slimes".into(),
                        enemy_ids: vec!["slime".into()],
                        weight: 1,
                        battle_scene: None,
                        battle_music: None,
                    }],
                },
            )]),
            ..JrpgDatabase::default()
        }
    }

    #[test]
    fn session_builds_battle_from_database_and_round_trips() {
        let database = database();
        let mut session = JrpgSession::new(7);
        session.recruit(&database, "hero", true).unwrap();
        session
            .begin_formation(&database, "field", "slimes")
            .unwrap();
        assert_eq!(session.battle.as_ref().unwrap().units.len(), 2);
        let json = session.to_json().unwrap();
        assert_eq!(JrpgSession::from_json(&json).unwrap(), session);
    }
}
