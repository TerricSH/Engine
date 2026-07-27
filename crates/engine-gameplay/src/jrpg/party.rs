use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::progression::CharacterProgress;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PartyError {
    #[error("actor '{0}' already belongs to the party")]
    DuplicateActor(String),
    #[error("actor '{0}' does not belong to the party")]
    UnknownActor(String),
    #[error("active party limit must be at least one")]
    InvalidActiveLimit,
    #[error("active party would exceed its {0}-member limit")]
    ActiveLimit(usize),
    #[error("currency operation overflowed")]
    CurrencyOverflow,
    #[error("not enough currency: requested {requested}, available {available}")]
    InsufficientCurrency { requested: u64, available: u64 },
    #[error("invalid party snapshot: {0}")]
    InvalidSnapshot(String),
}

/// Aggregate root for roster ordering and shared currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Party {
    #[serde(default = "default_active_limit")]
    pub active_limit: usize,
    #[serde(default)]
    pub currency: u64,
    #[serde(default)]
    actors: BTreeMap<String, CharacterProgress>,
    #[serde(default)]
    active: Vec<String>,
    #[serde(default)]
    reserve: Vec<String>,
}

fn default_active_limit() -> usize {
    4
}

impl Default for Party {
    fn default() -> Self {
        Self {
            active_limit: default_active_limit(),
            currency: 0,
            actors: BTreeMap::new(),
            active: Vec::new(),
            reserve: Vec::new(),
        }
    }
}

impl Party {
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.active_limit == 0 {
            return Err(PartyError::InvalidActiveLimit);
        }
        if self.active.len() > self.active_limit {
            return Err(PartyError::ActiveLimit(self.active_limit));
        }
        for (id, actor) in &self.actors {
            if id != &actor.actor_id {
                return Err(PartyError::InvalidSnapshot(format!(
                    "actor map key '{id}' does not match '{}'",
                    actor.actor_id
                )));
            }
        }
        let mut roster = self
            .active
            .iter()
            .chain(self.reserve.iter())
            .cloned()
            .collect::<Vec<_>>();
        let listed_count = roster.len();
        roster.sort();
        roster.dedup();
        if roster.len() != listed_count {
            return Err(PartyError::InvalidSnapshot(
                "active/reserve roster contains duplicates".into(),
            ));
        }
        let actor_ids = self.actors.keys().cloned().collect::<Vec<_>>();
        if roster != actor_ids {
            return Err(PartyError::InvalidSnapshot(
                "active/reserve roster does not match actor map".into(),
            ));
        }
        Ok(())
    }

    pub fn actors(&self) -> impl Iterator<Item = &CharacterProgress> {
        self.actors.values()
    }

    pub fn actor(&self, actor_id: &str) -> Option<&CharacterProgress> {
        self.actors.get(actor_id)
    }

    pub fn actor_mut(&mut self, actor_id: &str) -> Option<&mut CharacterProgress> {
        self.actors.get_mut(actor_id)
    }

    pub fn active_ids(&self) -> &[String] {
        &self.active
    }

    pub fn reserve_ids(&self) -> &[String] {
        &self.reserve
    }

    pub fn recruit(
        &mut self,
        actor: CharacterProgress,
        prefer_active: bool,
    ) -> Result<(), PartyError> {
        if self.actors.contains_key(&actor.actor_id) {
            return Err(PartyError::DuplicateActor(actor.actor_id));
        }
        let id = actor.actor_id.clone();
        self.actors.insert(id.clone(), actor);
        if prefer_active && self.active.len() < self.active_limit {
            self.active.push(id);
        } else {
            self.reserve.push(id);
        }
        Ok(())
    }

    pub fn dismiss(&mut self, actor_id: &str) -> Result<CharacterProgress, PartyError> {
        let actor = self
            .actors
            .remove(actor_id)
            .ok_or_else(|| PartyError::UnknownActor(actor_id.into()))?;
        self.active.retain(|id| id != actor_id);
        self.reserve.retain(|id| id != actor_id);
        Ok(actor)
    }

    /// Replaces the active lineup after validating uniqueness and membership.
    pub fn set_active(&mut self, actor_ids: Vec<String>) -> Result<(), PartyError> {
        if self.active_limit == 0 {
            return Err(PartyError::InvalidActiveLimit);
        }
        if actor_ids.len() > self.active_limit {
            return Err(PartyError::ActiveLimit(self.active_limit));
        }
        let mut unique = actor_ids.clone();
        unique.sort();
        unique.dedup();
        if unique.len() != actor_ids.len() {
            return Err(PartyError::DuplicateActor(
                "active lineup contains a duplicate".into(),
            ));
        }
        for id in &actor_ids {
            if !self.actors.contains_key(id) {
                return Err(PartyError::UnknownActor(id.clone()));
            }
        }
        self.active = actor_ids;
        self.reserve = self
            .actors
            .keys()
            .filter(|id| !self.active.contains(id))
            .cloned()
            .collect();
        Ok(())
    }

    pub fn grant_currency(&mut self, amount: u64) -> Result<u64, PartyError> {
        self.currency = self
            .currency
            .checked_add(amount)
            .ok_or(PartyError::CurrencyOverflow)?;
        Ok(self.currency)
    }

    pub fn spend_currency(&mut self, amount: u64) -> Result<u64, PartyError> {
        if amount > self.currency {
            return Err(PartyError::InsufficientCurrency {
                requested: amount,
                available: self.currency,
            });
        }
        self.currency -= amount;
        Ok(self.currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jrpg::StatBlock;

    fn actor(id: &str) -> CharacterProgress {
        CharacterProgress::new(id, id, StatBlock::default())
    }

    #[test]
    fn lineup_update_is_validated_before_commit() {
        let mut party = Party::default();
        party.recruit(actor("a"), true).unwrap();
        party.recruit(actor("b"), true).unwrap();
        assert!(party
            .set_active(vec!["a".into(), "missing".into()])
            .is_err());
        assert_eq!(party.active_ids(), &["a", "b"]);
    }

    #[test]
    fn currency_cannot_go_negative() {
        let mut party = Party::default();
        party.grant_currency(50).unwrap();
        assert!(party.spend_currency(51).is_err());
        assert_eq!(party.currency, 50);
    }
}
