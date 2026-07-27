use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Stats shared by field, menu, and battle gameplay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatBlock {
    pub max_hp: i32,
    pub max_mp: i32,
    pub attack: i32,
    pub defense: i32,
    pub magic: i32,
    pub resistance: i32,
    pub speed: i32,
    pub luck: i32,
}

impl StatBlock {
    pub fn clamped_non_negative(self) -> Self {
        Self {
            max_hp: self.max_hp.max(1),
            max_mp: self.max_mp.max(0),
            attack: self.attack.max(0),
            defense: self.defense.max(0),
            magic: self.magic.max(0),
            resistance: self.resistance.max(0),
            speed: self.speed.max(0),
            luck: self.luck.max(0),
        }
    }

    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            max_hp: self.max_hp.saturating_add(other.max_hp),
            max_mp: self.max_mp.saturating_add(other.max_mp),
            attack: self.attack.saturating_add(other.attack),
            defense: self.defense.saturating_add(other.defense),
            magic: self.magic.saturating_add(other.magic),
            resistance: self.resistance.saturating_add(other.resistance),
            speed: self.speed.saturating_add(other.speed),
            luck: self.luck.saturating_add(other.luck),
        }
        .clamped_non_negative()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatModifier {
    pub source: String,
    #[serde(default)]
    pub flat: StatBlock,
    /// Per-stat multiplier in basis points; 10_000 means no change.
    #[serde(default = "default_multiplier")]
    pub multiplier_basis_points: StatBlock,
}

fn default_multiplier() -> StatBlock {
    StatBlock {
        max_hp: 10_000,
        max_mp: 10_000,
        attack: 10_000,
        defense: 10_000,
        magic: 10_000,
        resistance: 10_000,
        speed: 10_000,
        luck: 10_000,
    }
}

impl StatModifier {
    pub fn flat(source: impl Into<String>, stats: StatBlock) -> Self {
        Self {
            source: source.into(),
            flat: stats,
            multiplier_basis_points: default_multiplier(),
        }
    }
}

/// Strategy boundary for project-specific level curves.
pub trait ExperienceCurve {
    fn total_experience_for_level(&self, level: u16) -> u64;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuadraticExperienceCurve {
    pub base: u64,
    pub growth: u64,
}

impl Default for QuadraticExperienceCurve {
    fn default() -> Self {
        Self {
            base: 100,
            growth: 25,
        }
    }
}

impl ExperienceCurve for QuadraticExperienceCurve {
    fn total_experience_for_level(&self, level: u16) -> u64 {
        let completed = u64::from(level.saturating_sub(1));
        self.base.saturating_mul(completed).saturating_add(
            self.growth
                .saturating_mul(completed.saturating_mul(completed.saturating_sub(1)) / 2),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelGain {
    pub old_level: u16,
    pub new_level: u16,
    pub gained_experience: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterProgress {
    pub actor_id: String,
    pub display_name: String,
    pub level: u16,
    pub experience: u64,
    pub skill_points: u32,
    pub base_stats: StatBlock,
    #[serde(default)]
    pub learned_abilities: BTreeSet<String>,
    #[serde(default)]
    pub equipment: BTreeMap<String, String>,
}

impl CharacterProgress {
    pub fn new(
        actor_id: impl Into<String>,
        display_name: impl Into<String>,
        base_stats: StatBlock,
    ) -> Self {
        Self {
            actor_id: actor_id.into(),
            display_name: display_name.into(),
            level: 1,
            experience: 0,
            skill_points: 0,
            base_stats: base_stats.clamped_non_negative(),
            learned_abilities: BTreeSet::new(),
            equipment: BTreeMap::new(),
        }
    }

    pub fn grant_experience<C: ExperienceCurve>(
        &mut self,
        amount: u64,
        level_cap: u16,
        curve: &C,
    ) -> LevelGain {
        let old_level = self.level;
        self.experience = self.experience.saturating_add(amount);
        while self.level < level_cap
            && self.experience >= curve.total_experience_for_level(self.level.saturating_add(1))
        {
            self.level = self.level.saturating_add(1);
            self.skill_points = self.skill_points.saturating_add(1);
        }
        LevelGain {
            old_level,
            new_level: self.level,
            gained_experience: amount,
        }
    }

    pub fn resolved_stats<'a>(
        &self,
        modifiers: impl IntoIterator<Item = &'a StatModifier>,
    ) -> StatBlock {
        let mut stats = self.base_stats;
        for modifier in modifiers {
            stats = stats.saturating_add(modifier.flat);
            stats = multiply_stats(stats, modifier.multiplier_basis_points);
        }
        stats.clamped_non_negative()
    }
}

fn multiply_stats(stats: StatBlock, basis_points: StatBlock) -> StatBlock {
    fn scale(value: i32, multiplier: i32) -> i32 {
        let product = i64::from(value).saturating_mul(i64::from(multiplier.max(0)));
        (product / 10_000).clamp(0, i64::from(i32::MAX)) as i32
    }
    StatBlock {
        max_hp: scale(stats.max_hp, basis_points.max_hp),
        max_mp: scale(stats.max_mp, basis_points.max_mp),
        attack: scale(stats.attack, basis_points.attack),
        defense: scale(stats.defense, basis_points.defense),
        magic: scale(stats.magic, basis_points.magic),
        resistance: scale(stats.resistance, basis_points.resistance),
        speed: scale(stats.speed, basis_points.speed),
        luck: scale(stats.luck, basis_points.luck),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experience_gain_handles_multiple_levels() {
        let mut actor = CharacterProgress::new(
            "hero",
            "Hero",
            StatBlock {
                max_hp: 100,
                ..StatBlock::default()
            },
        );
        let gain = actor.grant_experience(500, 99, &QuadraticExperienceCurve::default());
        assert!(gain.new_level > gain.old_level);
        assert_eq!(actor.skill_points, u32::from(actor.level - 1));
    }

    #[test]
    fn modifiers_are_applied_in_stable_order() {
        let actor = CharacterProgress::new(
            "mage",
            "Mage",
            StatBlock {
                max_hp: 100,
                magic: 20,
                ..StatBlock::default()
            },
        );
        let flat = StatModifier::flat(
            "staff",
            StatBlock {
                magic: 10,
                ..StatBlock::default()
            },
        );
        let percent = StatModifier {
            source: "buff".into(),
            flat: StatBlock::default(),
            multiplier_basis_points: StatBlock {
                magic: 15_000,
                ..default_multiplier()
            },
        };
        assert_eq!(actor.resolved_stats([&flat, &percent]).magic, 45);
    }
}
