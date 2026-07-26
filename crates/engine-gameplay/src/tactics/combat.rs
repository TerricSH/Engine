use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::grid::CoverKind;
use super::types::TacticalEntityId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatantStats {
    pub max_health: i32,
    pub aim: i16,
    pub defense: i16,
    pub armor: i16,
    pub mobility: u16,
    pub will: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combatant {
    pub entity: TacticalEntityId,
    pub stats: CombatantStats,
    pub health: i32,
    pub statuses: BTreeMap<String, StatusEffect>,
    pub cooldowns: BTreeMap<String, u16>,
}

impl Combatant {
    pub fn new(entity: impl Into<TacticalEntityId>, stats: CombatantStats) -> Self {
        Self {
            entity: entity.into(),
            health: stats.max_health,
            stats,
            statuses: BTreeMap::new(),
            cooldowns: BTreeMap::new(),
        }
    }

    pub fn is_alive(&self) -> bool {
        self.health > 0
    }

    pub fn tick_round(&mut self) {
        self.cooldowns.retain(|_, rounds| {
            *rounds = rounds.saturating_sub(1);
            *rounds > 0
        });
        self.statuses.retain(|_, status| {
            status.remaining_rounds = status.remaining_rounds.saturating_sub(1);
            status.remaining_rounds > 0
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEffect {
    pub key: String,
    pub remaining_rounds: u16,
    pub stacks: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponProfile {
    pub base_damage: i32,
    pub armor_piercing: i16,
    pub range: u16,
    pub accuracy: i16,
    pub critical_chance_basis_points: u16,
    pub critical_bonus_damage: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilitySpec {
    pub key: String,
    pub action_point_cost: u16,
    pub cooldown_rounds: u16,
    pub range: u16,
    pub requires_line_of_sight: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackContext {
    pub distance: u16,
    pub target_cover: CoverKind,
    pub elevation_delta: i16,
}

pub trait HitChancePolicy {
    fn hit_chance_basis_points(
        &self,
        attacker: &Combatant,
        target: &Combatant,
        weapon: WeaponProfile,
        context: AttackContext,
    ) -> u16;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultHitChancePolicy;

impl HitChancePolicy for DefaultHitChancePolicy {
    fn hit_chance_basis_points(
        &self,
        attacker: &Combatant,
        target: &Combatant,
        weapon: WeaponProfile,
        context: AttackContext,
    ) -> u16 {
        let range_penalty = context
            .distance
            .saturating_sub(weapon.range)
            .saturating_mul(5) as i32;
        let cover_penalty = match context.target_cover {
            CoverKind::None => 0,
            CoverKind::Half => 20,
            CoverKind::Full => 40,
        };
        let elevation_bonus = i32::from(context.elevation_delta).clamp(-2, 2) * 10;
        let percent = 65 + i32::from(attacker.stats.aim) + i32::from(weapon.accuracy)
            - i32::from(target.stats.defense)
            - cover_penalty
            - range_penalty
            + elevation_bonus;
        (percent.clamp(0, 100) * 100) as u16
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatOutcome {
    pub hit: bool,
    pub critical: bool,
    pub hit_chance_basis_points: u16,
    pub roll: u16,
    pub damage: i32,
    pub target_health: i32,
    pub target_killed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    pub fn roll_basis_points(&mut self) -> u16 {
        (self.next_u64() % 10_000) as u16
    }
}

pub struct CombatResolver<P = DefaultHitChancePolicy> {
    policy: P,
}

impl Default for CombatResolver<DefaultHitChancePolicy> {
    fn default() -> Self {
        Self {
            policy: DefaultHitChancePolicy,
        }
    }
}

impl<P: HitChancePolicy> CombatResolver<P> {
    pub fn new(policy: P) -> Self {
        Self { policy }
    }

    pub fn resolve_attack(
        &self,
        attacker: &Combatant,
        target: &mut Combatant,
        weapon: WeaponProfile,
        context: AttackContext,
        rng: &mut DeterministicRng,
    ) -> CombatOutcome {
        let hit_chance = self
            .policy
            .hit_chance_basis_points(attacker, target, weapon, context);
        let roll = rng.roll_basis_points();
        let hit = roll < hit_chance;
        let critical = hit && rng.roll_basis_points() < weapon.critical_chance_basis_points;
        let raw_damage = if hit {
            weapon.base_damage
                + if critical {
                    weapon.critical_bonus_damage
                } else {
                    0
                }
        } else {
            0
        };
        let mitigated_armor =
            (i32::from(target.stats.armor) - i32::from(weapon.armor_piercing)).max(0);
        let damage = if hit {
            (raw_damage - mitigated_armor).max(1)
        } else {
            0
        };
        target.health = target.health.saturating_sub(damage).max(0);
        CombatOutcome {
            hit,
            critical,
            hit_chance_basis_points: hit_chance,
            roll,
            damage,
            target_health: target.health,
            target_killed: !target.is_alive(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: &str) -> Combatant {
        Combatant::new(
            id,
            CombatantStats {
                max_health: 10,
                aim: 0,
                defense: 0,
                armor: 0,
                mobility: 12,
                will: 0,
            },
        )
    }

    #[test]
    fn seeded_combat_is_replayable() {
        let attacker = unit("a");
        let mut first_target = unit("b");
        let mut second_target = unit("b");
        let weapon = WeaponProfile {
            base_damage: 4,
            armor_piercing: 0,
            range: 5,
            accuracy: 0,
            critical_chance_basis_points: 1_000,
            critical_bonus_damage: 2,
        };
        let context = AttackContext {
            distance: 3,
            target_cover: CoverKind::Half,
            elevation_delta: 0,
        };
        let mut first_rng = DeterministicRng::new(42);
        let mut second_rng = DeterministicRng::new(42);
        let first = CombatResolver::default().resolve_attack(
            &attacker,
            &mut first_target,
            weapon,
            context,
            &mut first_rng,
        );
        let second = CombatResolver::default().resolve_attack(
            &attacker,
            &mut second_target,
            weapon,
            context,
            &mut second_rng,
        );
        assert_eq!(first, second);
    }

    #[test]
    fn armor_reduces_but_does_not_eliminate_a_hit() {
        let attacker = unit("a");
        let mut target = unit("b");
        target.stats.armor = 99;
        let weapon = WeaponProfile {
            base_damage: 4,
            armor_piercing: 0,
            range: 5,
            accuracy: 100,
            critical_chance_basis_points: 0,
            critical_bonus_damage: 0,
        };
        let outcome = CombatResolver::default().resolve_attack(
            &attacker,
            &mut target,
            weapon,
            AttackContext {
                distance: 1,
                target_cover: CoverKind::None,
                elevation_delta: 0,
            },
            &mut DeterministicRng::new(1),
        );
        assert!(outcome.hit);
        assert_eq!(outcome.damage, 1);
    }

    #[test]
    fn statuses_and_cooldowns_expire_by_round() {
        let mut combatant = unit("a");
        combatant.cooldowns.insert("grenade".to_string(), 1);
        combatant.statuses.insert(
            "burning".to_string(),
            StatusEffect {
                key: "burning".to_string(),
                remaining_rounds: 1,
                stacks: 1,
            },
        );
        combatant.tick_round();
        assert!(combatant.cooldowns.is_empty());
        assert!(combatant.statuses.is_empty());
    }
}
