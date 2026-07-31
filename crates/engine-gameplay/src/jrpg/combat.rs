use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::DeterministicRng;

use super::progression::{StatBlock, StatModifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BattleSide {
    Party,
    Enemy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatDamageKind {
    Physical,
    Magical,
    Direct,
}

/// Backwards-compatible name for [`CombatDamageKind`].
///
/// New code should use the domain-qualified name to distinguish combat
/// formula selection from physics/destruction damage sources.
pub type DamageKind = CombatDamageKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Element {
    Neutral,
    Fire,
    Ice,
    Lightning,
    Water,
    Wind,
    Earth,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRule {
    SelfUnit,
    SingleAlly,
    AllAllies,
    SingleEnemy,
    AllEnemies,
    AnySingle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BattleEffect {
    Damage {
        power: i32,
        kind: CombatDamageKind,
        #[serde(default = "neutral_element")]
        element: Element,
    },
    Heal {
        power: i32,
    },
    RestoreMp {
        amount: i32,
    },
    AddStatus {
        status_id: String,
        #[serde(default = "guaranteed_chance")]
        chance_basis_points: u16,
    },
    RemoveStatus {
        status_id: String,
    },
    Revive {
        health_basis_points: u16,
    },
}

fn neutral_element() -> Element {
    Element::Neutral
}

fn guaranteed_chance() -> u16 {
    10_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityDefinition {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub mp_cost: i32,
    #[serde(default = "guaranteed_chance")]
    pub accuracy_basis_points: u16,
    #[serde(default)]
    pub critical_basis_points: u16,
    pub target_rule: TargetRule,
    #[serde(default)]
    pub effects: Vec<BattleEffect>,
}

impl AbilityDefinition {
    pub fn validate(&self) -> Result<(), BattleError> {
        if self.id.is_empty() || self.effects.is_empty() {
            return Err(BattleError::InvalidDefinition(self.id.clone()));
        }
        if self.mp_cost < 0
            || self.accuracy_basis_points > 10_000
            || self.critical_basis_points > 10_000
        {
            return Err(BattleError::InvalidDefinition(self.id.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusDefinition {
    pub id: String,
    #[serde(default = "default_status_duration")]
    pub default_duration: u16,
    #[serde(default = "default_max_stacks")]
    pub max_stacks: u8,
    #[serde(default)]
    pub modifiers: Vec<StatModifier>,
    #[serde(default)]
    pub periodic_effects: Vec<BattleEffect>,
}

fn default_status_duration() -> u16 {
    3
}

fn default_max_stacks() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveStatus {
    pub remaining_turns: u16,
    pub stacks: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleUnit {
    pub id: String,
    pub source_id: String,
    pub side: BattleSide,
    pub stats: StatBlock,
    pub hp: i32,
    pub mp: i32,
    #[serde(default)]
    pub elemental_affinity_basis_points: BTreeMap<Element, i32>,
    #[serde(default)]
    pub statuses: BTreeMap<String, ActiveStatus>,
    #[serde(default)]
    pub atb: u32,
    #[serde(default)]
    pub defending: bool,
    #[serde(default)]
    pub reward_experience: u64,
    #[serde(default)]
    pub reward_currency: u64,
}

impl BattleUnit {
    pub fn new(
        id: impl Into<String>,
        source_id: impl Into<String>,
        side: BattleSide,
        stats: StatBlock,
    ) -> Self {
        let stats = stats.clamped_non_negative();
        Self {
            id: id.into(),
            source_id: source_id.into(),
            side,
            hp: stats.max_hp,
            mp: stats.max_mp,
            stats,
            elemental_affinity_basis_points: BTreeMap::new(),
            statuses: BTreeMap::new(),
            atb: 0,
            defending: false,
            reward_experience: 0,
            reward_currency: 0,
        }
    }

    pub fn alive(&self) -> bool {
        self.hp > 0
    }

    pub fn resolved_stats(&self, definitions: &BTreeMap<String, StatusDefinition>) -> StatBlock {
        let modifiers = self.statuses.iter().flat_map(|(id, active)| {
            definitions.get(id).into_iter().flat_map(move |definition| {
                definition
                    .modifiers
                    .iter()
                    .cycle()
                    .take(definition.modifiers.len() * usize::from(active.stacks))
            })
        });
        let mut stats = self.stats;
        for modifier in modifiers {
            stats = stats.saturating_add(modifier.flat);
        }
        stats
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleCommand {
    pub actor_id: String,
    pub ability_id: String,
    pub target_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BattlePhase {
    Running,
    PartyVictory,
    EnemyVictory,
    Escaped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BattleEvent {
    UnitReady {
        unit_id: String,
    },
    CommandQueued {
        unit_id: String,
        ability_id: String,
    },
    MpSpent {
        unit_id: String,
        amount: i32,
    },
    Missed {
        source_id: String,
        target_id: String,
    },
    Damaged {
        source_id: String,
        target_id: String,
        amount: i32,
        critical: bool,
        element: Element,
    },
    Healed {
        source_id: String,
        target_id: String,
        amount: i32,
    },
    StatusAdded {
        target_id: String,
        status_id: String,
    },
    StatusRemoved {
        target_id: String,
        status_id: String,
    },
    UnitDefeated {
        unit_id: String,
    },
    PhaseChanged {
        phase: BattlePhase,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BattleError {
    #[error("battle is no longer running")]
    Finished,
    #[error("unknown battle unit '{0}'")]
    UnknownUnit(String),
    #[error("unknown ability '{0}'")]
    UnknownAbility(String),
    #[error("unknown status '{0}'")]
    UnknownStatus(String),
    #[error("unit '{0}' is not ready")]
    UnitNotReady(String),
    #[error("unit '{0}' is defeated")]
    UnitDefeated(String),
    #[error("unit '{unit}' needs {required} MP but has {available}")]
    InsufficientMp {
        unit: String,
        required: i32,
        available: i32,
    },
    #[error("invalid targets for ability '{0}'")]
    InvalidTargets(String),
    #[error("invalid battle definition '{0}'")]
    InvalidDefinition(String),
}

/// Strategy boundary for a project's damage formula.
pub trait BattleFormula {
    fn damage(
        &self,
        attacker: StatBlock,
        target: StatBlock,
        power: i32,
        kind: CombatDamageKind,
    ) -> i32;

    fn healing(&self, caster: StatBlock, power: i32) -> i32;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ClassicBattleFormula;

impl BattleFormula for ClassicBattleFormula {
    fn damage(
        &self,
        attacker: StatBlock,
        target: StatBlock,
        power: i32,
        kind: CombatDamageKind,
    ) -> i32 {
        let (offense, defense) = match kind {
            CombatDamageKind::Physical => (attacker.attack, target.defense),
            CombatDamageKind::Magical => (attacker.magic, target.resistance),
            CombatDamageKind::Direct => (power, 0),
        };
        if kind == CombatDamageKind::Direct {
            return power.max(0);
        }
        offense
            .saturating_mul(power.max(0))
            .saturating_div(100)
            .saturating_sub(defense / 2)
            .max(1)
    }

    fn healing(&self, caster: StatBlock, power: i32) -> i32 {
        caster
            .magic
            .saturating_mul(power.max(0))
            .saturating_div(100)
            .max(1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleSession {
    pub phase: BattlePhase,
    #[serde(default = "default_atb_threshold")]
    pub atb_threshold: u32,
    pub units: BTreeMap<String, BattleUnit>,
    #[serde(default)]
    ready: BTreeSet<String>,
    #[serde(default)]
    command_queue: VecDeque<BattleCommand>,
}

fn default_atb_threshold() -> u32 {
    10_000
}

impl BattleSession {
    pub fn new(units: impl IntoIterator<Item = BattleUnit>) -> Result<Self, BattleError> {
        let mut indexed = BTreeMap::new();
        for unit in units {
            if unit.id.is_empty() || indexed.insert(unit.id.clone(), unit).is_some() {
                return Err(BattleError::InvalidDefinition(
                    "battle unit ids must be non-empty and unique".into(),
                ));
            }
        }
        if !indexed.values().any(|unit| unit.side == BattleSide::Party)
            || !indexed.values().any(|unit| unit.side == BattleSide::Enemy)
        {
            return Err(BattleError::InvalidDefinition(
                "battle requires party and enemy units".into(),
            ));
        }
        Ok(Self {
            phase: BattlePhase::Running,
            atb_threshold: default_atb_threshold(),
            units: indexed,
            ready: BTreeSet::new(),
            command_queue: VecDeque::new(),
        })
    }

    pub fn ready_units(&self) -> impl Iterator<Item = &str> {
        self.ready.iter().map(String::as_str)
    }

    pub fn queued_commands(&self) -> impl Iterator<Item = &BattleCommand> {
        self.command_queue.iter()
    }

    /// Advances an Active-Time-Battle gauge using integer milliseconds.
    pub fn advance_time(&mut self, delta_millis: u32) -> Vec<BattleEvent> {
        if self.phase != BattlePhase::Running {
            return Vec::new();
        }
        let mut events = Vec::new();
        for unit in self.units.values_mut() {
            if !unit.alive() || self.ready.contains(&unit.id) {
                continue;
            }
            let gain = u32::try_from(unit.stats.speed.max(1))
                .unwrap_or(u32::MAX)
                .saturating_mul(delta_millis);
            unit.atb = unit.atb.saturating_add(gain).min(self.atb_threshold);
            if unit.atb >= self.atb_threshold {
                self.ready.insert(unit.id.clone());
                events.push(BattleEvent::UnitReady {
                    unit_id: unit.id.clone(),
                });
            }
        }
        events
    }

    /// Makes a unit ready immediately, supporting classic turn-based projects.
    pub fn grant_turn(&mut self, unit_id: &str) -> Result<BattleEvent, BattleError> {
        let unit = self
            .units
            .get_mut(unit_id)
            .ok_or_else(|| BattleError::UnknownUnit(unit_id.into()))?;
        if !unit.alive() {
            return Err(BattleError::UnitDefeated(unit_id.into()));
        }
        unit.atb = self.atb_threshold;
        self.ready.insert(unit_id.into());
        Ok(BattleEvent::UnitReady {
            unit_id: unit_id.into(),
        })
    }

    pub fn queue_command(
        &mut self,
        command: BattleCommand,
        abilities: &BTreeMap<String, AbilityDefinition>,
    ) -> Result<BattleEvent, BattleError> {
        if self.phase != BattlePhase::Running {
            return Err(BattleError::Finished);
        }
        let actor = self
            .units
            .get(&command.actor_id)
            .ok_or_else(|| BattleError::UnknownUnit(command.actor_id.clone()))?;
        if !actor.alive() {
            return Err(BattleError::UnitDefeated(actor.id.clone()));
        }
        if !self.ready.contains(&actor.id) {
            return Err(BattleError::UnitNotReady(actor.id.clone()));
        }
        let ability = abilities
            .get(&command.ability_id)
            .ok_or_else(|| BattleError::UnknownAbility(command.ability_id.clone()))?;
        ability.validate()?;
        if actor.mp < ability.mp_cost {
            return Err(BattleError::InsufficientMp {
                unit: actor.id.clone(),
                required: ability.mp_cost,
                available: actor.mp,
            });
        }
        if !self.targets_are_valid(actor, ability.target_rule, &command.target_ids) {
            return Err(BattleError::InvalidTargets(ability.id.clone()));
        }
        self.ready.remove(&actor.id);
        self.command_queue.push_back(command.clone());
        Ok(BattleEvent::CommandQueued {
            unit_id: actor.id.clone(),
            ability_id: ability.id.clone(),
        })
    }

    pub fn execute_next<F: BattleFormula>(
        &mut self,
        abilities: &BTreeMap<String, AbilityDefinition>,
        statuses: &BTreeMap<String, StatusDefinition>,
        formula: &F,
        random: &mut DeterministicRng,
    ) -> Result<Vec<BattleEvent>, BattleError> {
        if self.phase != BattlePhase::Running {
            return Err(BattleError::Finished);
        }
        let Some(command) = self.command_queue.pop_front() else {
            return Ok(Vec::new());
        };
        let ability = abilities
            .get(&command.ability_id)
            .ok_or_else(|| BattleError::UnknownAbility(command.ability_id.clone()))?;
        let actor = self
            .units
            .get(&command.actor_id)
            .ok_or_else(|| BattleError::UnknownUnit(command.actor_id.clone()))?;
        if !actor.alive() {
            return Err(BattleError::UnitDefeated(actor.id.clone()));
        }
        let actor_stats = actor.resolved_stats(statuses);
        let actor_id = actor.id.clone();
        let mp_cost = ability.mp_cost;
        self.units.get_mut(&actor_id).unwrap().mp -= mp_cost;

        let mut events = if mp_cost == 0 {
            Vec::new()
        } else {
            vec![BattleEvent::MpSpent {
                unit_id: actor_id.clone(),
                amount: mp_cost,
            }]
        };

        for target_id in command.target_ids {
            for effect in &ability.effects {
                self.apply_effect(
                    &actor_id,
                    actor_stats,
                    &target_id,
                    ability,
                    effect,
                    statuses,
                    formula,
                    random,
                    &mut events,
                )?;
            }
        }
        if let Some(actor) = self.units.get_mut(&actor_id) {
            actor.atb = 0;
            actor.defending = false;
        }
        self.tick_statuses(&actor_id, statuses, formula, random, &mut events)?;
        self.refresh_phase(&mut events);
        Ok(events)
    }

    fn tick_statuses<F: BattleFormula>(
        &mut self,
        unit_id: &str,
        statuses: &BTreeMap<String, StatusDefinition>,
        formula: &F,
        random: &mut DeterministicRng,
        events: &mut Vec<BattleEvent>,
    ) -> Result<(), BattleError> {
        let active = self
            .units
            .get(unit_id)
            .ok_or_else(|| BattleError::UnknownUnit(unit_id.into()))?
            .statuses
            .clone();
        for (status_id, status) in active {
            let definition = statuses
                .get(&status_id)
                .ok_or_else(|| BattleError::UnknownStatus(status_id.clone()))?;
            let caster_stats = self.units[unit_id].resolved_stats(statuses);
            let periodic_ability = AbilityDefinition {
                id: format!("status.{status_id}"),
                display_name: status_id.clone(),
                mp_cost: 0,
                accuracy_basis_points: 10_000,
                critical_basis_points: 0,
                target_rule: TargetRule::SelfUnit,
                effects: definition.periodic_effects.clone(),
            };
            for _ in 0..status.stacks {
                for effect in &definition.periodic_effects {
                    self.apply_effect(
                        unit_id,
                        caster_stats,
                        unit_id,
                        &periodic_ability,
                        effect,
                        statuses,
                        formula,
                        random,
                        events,
                    )?;
                }
            }
            let Some(unit) = self.units.get_mut(unit_id) else {
                continue;
            };
            let Some(active_status) = unit.statuses.get_mut(&status_id) else {
                continue;
            };
            active_status.remaining_turns = active_status.remaining_turns.saturating_sub(1);
            if active_status.remaining_turns == 0 {
                unit.statuses.remove(&status_id);
                events.push(BattleEvent::StatusRemoved {
                    target_id: unit_id.into(),
                    status_id,
                });
            }
        }
        Ok(())
    }

    fn targets_are_valid(&self, actor: &BattleUnit, rule: TargetRule, targets: &[String]) -> bool {
        if targets.is_empty() {
            return false;
        }
        let resolved: Option<Vec<&BattleUnit>> =
            targets.iter().map(|id| self.units.get(id)).collect();
        let Some(resolved) = resolved else {
            return false;
        };
        let unique = targets.iter().collect::<BTreeSet<_>>().len() == targets.len();
        if !unique {
            return false;
        }
        match rule {
            TargetRule::SelfUnit => targets.len() == 1 && targets[0] == actor.id,
            TargetRule::SingleAlly => {
                resolved.len() == 1 && resolved[0].side == actor.side && resolved[0].alive()
            }
            TargetRule::AllAllies => {
                resolved
                    .iter()
                    .all(|target| target.side == actor.side && target.alive())
                    && resolved.len()
                        == self
                            .units
                            .values()
                            .filter(|unit| unit.side == actor.side && unit.alive())
                            .count()
            }
            TargetRule::SingleEnemy => {
                resolved.len() == 1 && resolved[0].side != actor.side && resolved[0].alive()
            }
            TargetRule::AllEnemies => {
                resolved
                    .iter()
                    .all(|target| target.side != actor.side && target.alive())
                    && resolved.len()
                        == self
                            .units
                            .values()
                            .filter(|unit| unit.side != actor.side && unit.alive())
                            .count()
            }
            TargetRule::AnySingle => resolved.len() == 1,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_effect<F: BattleFormula>(
        &mut self,
        actor_id: &str,
        actor_stats: StatBlock,
        target_id: &str,
        ability: &AbilityDefinition,
        effect: &BattleEffect,
        statuses: &BTreeMap<String, StatusDefinition>,
        formula: &F,
        random: &mut DeterministicRng,
        events: &mut Vec<BattleEvent>,
    ) -> Result<(), BattleError> {
        let target = self
            .units
            .get_mut(target_id)
            .ok_or_else(|| BattleError::UnknownUnit(target_id.into()))?;
        match effect {
            BattleEffect::Damage {
                power,
                kind,
                element,
            } => {
                if random.roll_basis_points() >= ability.accuracy_basis_points {
                    events.push(BattleEvent::Missed {
                        source_id: actor_id.into(),
                        target_id: target_id.into(),
                    });
                    return Ok(());
                }
                let critical = random.roll_basis_points() < ability.critical_basis_points;
                let mut amount =
                    formula.damage(actor_stats, target.resolved_stats(statuses), *power, *kind);
                let affinity = target
                    .elemental_affinity_basis_points
                    .get(element)
                    .copied()
                    .unwrap_or(10_000)
                    .clamp(-20_000, 20_000);
                amount = ((i64::from(amount) * i64::from(affinity)) / 10_000)
                    .clamp(i64::from(i32::MIN), i64::from(i32::MAX))
                    as i32;
                if critical {
                    amount = amount.saturating_mul(3) / 2;
                }
                if target.defending {
                    amount /= 2;
                }
                if amount >= 0 {
                    target.hp = target.hp.saturating_sub(amount).max(0);
                } else {
                    target.hp = target.hp.saturating_add(-amount).min(target.stats.max_hp);
                }
                events.push(BattleEvent::Damaged {
                    source_id: actor_id.into(),
                    target_id: target_id.into(),
                    amount,
                    critical,
                    element: *element,
                });
                if target.hp == 0 {
                    target.atb = 0;
                    self.ready.remove(target_id);
                    events.push(BattleEvent::UnitDefeated {
                        unit_id: target_id.into(),
                    });
                }
            }
            BattleEffect::Heal { power } => {
                let before = target.hp;
                target.hp = target
                    .hp
                    .saturating_add(formula.healing(actor_stats, *power))
                    .min(target.stats.max_hp);
                events.push(BattleEvent::Healed {
                    source_id: actor_id.into(),
                    target_id: target_id.into(),
                    amount: target.hp - before,
                });
            }
            BattleEffect::RestoreMp { amount } => {
                target.mp = target
                    .mp
                    .saturating_add(*amount)
                    .clamp(0, target.stats.max_mp);
            }
            BattleEffect::AddStatus {
                status_id,
                chance_basis_points,
            } => {
                let definition = statuses
                    .get(status_id)
                    .ok_or_else(|| BattleError::UnknownStatus(status_id.clone()))?;
                if random.roll_basis_points() < (*chance_basis_points).min(10_000) {
                    let active = target
                        .statuses
                        .entry(status_id.clone())
                        .or_insert(ActiveStatus {
                            remaining_turns: definition.default_duration,
                            stacks: 0,
                        });
                    active.remaining_turns = definition.default_duration;
                    active.stacks = active
                        .stacks
                        .saturating_add(1)
                        .min(definition.max_stacks.max(1));
                    events.push(BattleEvent::StatusAdded {
                        target_id: target_id.into(),
                        status_id: status_id.clone(),
                    });
                }
            }
            BattleEffect::RemoveStatus { status_id } => {
                if target.statuses.remove(status_id).is_some() {
                    events.push(BattleEvent::StatusRemoved {
                        target_id: target_id.into(),
                        status_id: status_id.clone(),
                    });
                }
            }
            BattleEffect::Revive {
                health_basis_points,
            } => {
                if target.hp == 0 {
                    target.hp = ((i64::from(target.stats.max_hp)
                        * i64::from((*health_basis_points).min(10_000)))
                        / 10_000)
                        .max(1) as i32;
                    events.push(BattleEvent::Healed {
                        source_id: actor_id.into(),
                        target_id: target_id.into(),
                        amount: target.hp,
                    });
                }
            }
        }
        Ok(())
    }

    fn refresh_phase(&mut self, events: &mut Vec<BattleEvent>) {
        let party_alive = self
            .units
            .values()
            .any(|unit| unit.side == BattleSide::Party && unit.alive());
        let enemies_alive = self
            .units
            .values()
            .any(|unit| unit.side == BattleSide::Enemy && unit.alive());
        let next = match (party_alive, enemies_alive) {
            (true, true) => BattlePhase::Running,
            (true, false) => BattlePhase::PartyVictory,
            _ => BattlePhase::EnemyVictory,
        };
        if next != self.phase {
            self.phase = next;
            events.push(BattleEvent::PhaseChanged { phase: next });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_damage_kind_is_the_combat_damage_kind() {
        assert_eq!(
            std::any::TypeId::of::<DamageKind>(),
            std::any::TypeId::of::<CombatDamageKind>()
        );
        assert_eq!(
            serde_json::to_string(&CombatDamageKind::Physical).unwrap(),
            "\"physical\""
        );
    }

    fn stats(attack: i32, defense: i32, speed: i32) -> StatBlock {
        StatBlock {
            max_hp: 100,
            max_mp: 20,
            attack,
            defense,
            magic: attack,
            resistance: defense,
            speed,
            luck: 0,
        }
    }

    fn attack() -> AbilityDefinition {
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
        }
    }

    #[test]
    fn command_pipeline_validates_then_resolves() {
        let mut battle = BattleSession::new([
            BattleUnit::new("hero", "hero", BattleSide::Party, stats(20, 5, 10)),
            BattleUnit::new("slime", "slime", BattleSide::Enemy, stats(5, 4, 5)),
        ])
        .unwrap();
        battle.grant_turn("hero").unwrap();
        let abilities = BTreeMap::from([("attack".into(), attack())]);
        battle
            .queue_command(
                BattleCommand {
                    actor_id: "hero".into(),
                    ability_id: "attack".into(),
                    target_ids: vec!["slime".into()],
                },
                &abilities,
            )
            .unwrap();
        let events = battle
            .execute_next(
                &abilities,
                &BTreeMap::new(),
                &ClassicBattleFormula,
                &mut DeterministicRng::new(1),
            )
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, BattleEvent::Damaged { .. })));
        assert!(battle.units["slime"].hp < 100);
    }

    #[test]
    fn atb_order_comes_from_speed() {
        let mut battle = BattleSession::new([
            BattleUnit::new("hero", "hero", BattleSide::Party, stats(20, 5, 100)),
            BattleUnit::new("slime", "slime", BattleSide::Enemy, stats(5, 4, 1)),
        ])
        .unwrap();
        let events = battle.advance_time(100);
        assert_eq!(
            events,
            vec![BattleEvent::UnitReady {
                unit_id: "hero".into()
            }]
        );
    }

    #[test]
    fn battle_snapshot_preserves_queue_and_random_independently() {
        let mut battle = BattleSession::new([
            BattleUnit::new("hero", "hero", BattleSide::Party, stats(20, 5, 10)),
            BattleUnit::new("slime", "slime", BattleSide::Enemy, stats(5, 4, 5)),
        ])
        .unwrap();
        battle.grant_turn("hero").unwrap();
        battle
            .queue_command(
                BattleCommand {
                    actor_id: "hero".into(),
                    ability_id: "attack".into(),
                    target_ids: vec!["slime".into()],
                },
                &BTreeMap::from([("attack".into(), attack())]),
            )
            .unwrap();
        let json = serde_json::to_string(&battle).unwrap();
        let restored: BattleSession = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, battle);
    }
}
