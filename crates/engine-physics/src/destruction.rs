use engine_serialize::AssetId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Component, Entity, World};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum DamageKind {
    #[default]
    Generic,
    Impact,
    Bullet,
    Blast,
    Fire,
}

/// Serializable health and fracture policy for a breakable entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Destructible {
    pub enabled: bool,
    pub max_health: f32,
    pub health: f32,
    pub minimum_damage: f32,
    pub damage_scale: f32,
    pub replacement_prefab: Option<AssetId>,
    pub destroy_on_break: bool,
    pub inherit_velocity: bool,
    pub fracture_impulse_scale: f32,
    pub broken: bool,
}

impl Default for Destructible {
    fn default() -> Self {
        Self {
            enabled: true,
            max_health: 100.0,
            health: 100.0,
            minimum_damage: 0.0,
            damage_scale: 1.0,
            replacement_prefab: None,
            destroy_on_break: true,
            inherit_velocity: true,
            fracture_impulse_scale: 1.0,
            broken: false,
        }
    }
}

impl Component for Destructible {
    const TYPE_ID: &'static str = "engine.physics.destructible";
}

impl Destructible {
    pub fn validate(&self) -> Result<(), String> {
        if ![
            self.max_health,
            self.health,
            self.minimum_damage,
            self.damage_scale,
            self.fracture_impulse_scale,
        ]
        .iter()
        .all(|value| value.is_finite())
        {
            return Err("destructible values must be finite".into());
        }
        if self.max_health <= 0.0 {
            return Err("destructible max_health must be greater than zero".into());
        }
        if self.health < 0.0 || self.health > self.max_health {
            return Err("destructible health must be between zero and max_health".into());
        }
        if self.minimum_damage < 0.0 || self.damage_scale < 0.0 || self.fracture_impulse_scale < 0.0
        {
            return Err(
                "destructible damage thresholds and multipliers must be non-negative".into(),
            );
        }
        if self.broken && self.health > 0.0 {
            return Err("a broken destructible must have zero health".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DamageRequest {
    pub source: Option<Entity>,
    pub amount: f32,
    pub kind: DamageKind,
    pub hit_position: Option<[f32; 3]>,
    pub impulse: [f32; 3],
}

impl DamageRequest {
    pub fn validate(&self) -> Result<(), DamageError> {
        if !self.amount.is_finite() || self.amount <= 0.0 {
            return Err(DamageError::InvalidRequest(
                "damage amount must be finite and greater than zero".into(),
            ));
        }
        if !self
            .hit_position
            .iter()
            .flatten()
            .chain(self.impulse.iter())
            .all(|value| value.is_finite())
        {
            return Err(DamageError::InvalidRequest(
                "damage position and impulse must be finite".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DestructibleDamageEvent {
    pub target: Entity,
    pub source: Option<Entity>,
    pub kind: DamageKind,
    pub raw_damage: f32,
    pub applied_damage: f32,
    pub remaining_health: f32,
    pub hit_position: Option<[f32; 3]>,
    pub impulse: [f32; 3],
    pub broke: bool,
    pub replacement_prefab: Option<AssetId>,
    pub destroy_on_break: bool,
    pub inherit_velocity: bool,
    pub fracture_impulse_scale: f32,
}

#[derive(Debug, Error, PartialEq)]
pub enum DamageError {
    #[error("damage target does not exist")]
    MissingTarget,
    #[error("damage target has no Destructible component")]
    NotDestructible,
    #[error("invalid damage request: {0}")]
    InvalidRequest(String),
    #[error("invalid Destructible target state: {0}")]
    InvalidTarget(String),
}

/// Apply one request and return an event when the hit was accepted.
pub fn apply_damage(
    world: &mut World,
    target: Entity,
    request: &DamageRequest,
) -> Result<Option<DestructibleDamageEvent>, DamageError> {
    request.validate()?;
    if !world.is_alive(target) {
        return Err(DamageError::MissingTarget);
    }
    let destructible = world
        .get_mut::<Destructible>(target)
        .ok_or(DamageError::NotDestructible)?;
    destructible
        .validate()
        .map_err(DamageError::InvalidTarget)?;
    if !destructible.enabled || destructible.broken || request.amount < destructible.minimum_damage
    {
        return Ok(None);
    }

    let applied_damage = (request.amount * destructible.damage_scale).min(destructible.health);
    if applied_damage <= 0.0 {
        return Ok(None);
    }
    destructible.health = (destructible.health - applied_damage).max(0.0);
    let broke = destructible.health <= 0.0;
    if broke {
        destructible.health = 0.0;
        destructible.broken = true;
    }

    Ok(Some(DestructibleDamageEvent {
        target,
        source: request.source,
        kind: request.kind,
        raw_damage: request.amount,
        applied_damage,
        remaining_health: destructible.health,
        hit_position: request.hit_position,
        impulse: request.impulse,
        broke,
        replacement_prefab: destructible.replacement_prefab.clone(),
        destroy_on_break: destructible.destroy_on_break,
        inherit_velocity: destructible.inherit_velocity,
        fracture_impulse_scale: destructible.fracture_impulse_scale,
    }))
}
