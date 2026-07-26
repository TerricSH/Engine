use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::grid::GridCoord;
use super::types::{ActionId, FactionId, TacticalEntityId};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnPhase {
    #[default]
    NotStarted,
    RoundStart,
    Acting,
    Resolving,
    Reaction,
    RoundEnd,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnParticipant {
    pub entity: TacticalEntityId,
    pub faction: FactionId,
    pub initiative: i32,
    pub max_action_points: u16,
    pub action_points: u16,
    pub active: bool,
}

impl TurnParticipant {
    pub fn new(
        entity: impl Into<TacticalEntityId>,
        faction: FactionId,
        initiative: i32,
        action_points: u16,
    ) -> Self {
        Self {
            entity: entity.into(),
            faction,
            initiative,
            max_action_points: action_points,
            action_points,
            active: true,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionTarget {
    #[default]
    None,
    Cell(GridCoord),
    Entity(TacticalEntityId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TacticalAction {
    pub id: ActionId,
    pub actor: TacticalEntityId,
    pub kind: String,
    pub action_point_cost: u16,
    pub target: ActionTarget,
    pub payload: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnCommand {
    Queue(TacticalAction),
    QueueReaction(TacticalAction),
    EndTurn { actor: TacticalEntityId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TacticalEvent {
    EncounterStarted,
    RoundStarted { round: u32 },
    TurnStarted { actor: TacticalEntityId },
    ActionQueued { action: ActionId },
    ActionResolved { action: ActionId },
    TurnEnded { actor: TacticalEntityId },
    RoundEnded { round: u32 },
    EncounterCompleted,
    Custom { key: String, value: String },
}

pub trait TacticalActionExecutor {
    fn execute(&mut self, action: &TacticalAction) -> Result<Vec<TacticalEvent>, String>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TurnError {
    #[error("the tactical encounter has not started")]
    NotStarted,
    #[error("there is no active participant")]
    NoActiveParticipant,
    #[error("{0} is not the active participant")]
    NotActiveActor(TacticalEntityId),
    #[error("participant {0} does not exist")]
    MissingParticipant(TacticalEntityId),
    #[error("participant {actor} needs {required} AP but only has {available}")]
    InsufficientActionPoints {
        actor: TacticalEntityId,
        required: u16,
        available: u16,
    },
    #[error("action execution failed: {0}")]
    Execution(String),
}

/// Deterministic turn coordinator using the Command pattern.
///
/// Commands are validated here, but their game-specific effects are delegated
/// to a `TacticalActionExecutor`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnDirector {
    participants: BTreeMap<TacticalEntityId, TurnParticipant>,
    order: Vec<TacticalEntityId>,
    active_index: usize,
    round: u32,
    phase: TurnPhase,
    queued_actions: VecDeque<TacticalAction>,
    reactions: VecDeque<TacticalAction>,
}

impl TurnDirector {
    pub fn add_participant(&mut self, participant: TurnParticipant) {
        self.participants
            .insert(participant.entity.clone(), participant);
    }

    pub fn participant(&self, entity: &str) -> Option<&TurnParticipant> {
        self.participants.get(entity)
    }

    pub fn participant_mut(&mut self, entity: &str) -> Option<&mut TurnParticipant> {
        self.participants.get_mut(entity)
    }

    pub fn phase(&self) -> TurnPhase {
        self.phase
    }

    pub fn round(&self) -> u32 {
        self.round
    }

    pub fn active_actor(&self) -> Option<&str> {
        self.order.get(self.active_index).map(String::as_str)
    }

    pub fn start(&mut self) -> Vec<TacticalEvent> {
        self.order = self
            .participants
            .values()
            .filter(|participant| participant.active)
            .map(|participant| participant.entity.clone())
            .collect();
        self.order.sort_by(|left, right| {
            let left = &self.participants[left];
            let right = &self.participants[right];
            right
                .initiative
                .cmp(&left.initiative)
                .then_with(|| left.entity.cmp(&right.entity))
        });
        self.active_index = 0;
        self.round = 1;
        self.phase = if self.order.is_empty() {
            TurnPhase::Complete
        } else {
            TurnPhase::Acting
        };
        self.refresh_action_points();
        let mut events = vec![TacticalEvent::EncounterStarted];
        if let Some(actor) = self.active_actor() {
            events.push(TacticalEvent::RoundStarted { round: self.round });
            events.push(TacticalEvent::TurnStarted {
                actor: actor.to_string(),
            });
        } else {
            events.push(TacticalEvent::EncounterCompleted);
        }
        events
    }

    pub fn submit(&mut self, command: TurnCommand) -> Result<Vec<TacticalEvent>, TurnError> {
        if matches!(self.phase, TurnPhase::NotStarted | TurnPhase::Complete) {
            return Err(TurnError::NotStarted);
        }
        match command {
            TurnCommand::Queue(action) => self.queue_action(action, false),
            TurnCommand::QueueReaction(action) => self.queue_action(action, true),
            TurnCommand::EndTurn { actor } => self.end_turn(&actor),
        }
    }

    pub fn resolve_next<E: TacticalActionExecutor>(
        &mut self,
        executor: &mut E,
    ) -> Result<Vec<TacticalEvent>, TurnError> {
        let (action, was_reaction) = if let Some(action) = self.reactions.pop_front() {
            (action, true)
        } else if let Some(action) = self.queued_actions.pop_front() {
            (action, false)
        } else {
            self.phase = TurnPhase::Acting;
            return Ok(Vec::new());
        };
        self.phase = if was_reaction {
            TurnPhase::Reaction
        } else {
            TurnPhase::Resolving
        };
        let mut events = executor.execute(&action).map_err(TurnError::Execution)?;
        events.push(TacticalEvent::ActionResolved { action: action.id });
        if self.reactions.is_empty() && self.queued_actions.is_empty() {
            self.phase = TurnPhase::Acting;
        }
        Ok(events)
    }

    pub fn complete(&mut self) -> TacticalEvent {
        self.phase = TurnPhase::Complete;
        self.queued_actions.clear();
        self.reactions.clear();
        TacticalEvent::EncounterCompleted
    }

    fn queue_action(
        &mut self,
        action: TacticalAction,
        reaction: bool,
    ) -> Result<Vec<TacticalEvent>, TurnError> {
        let active = self.active_actor().ok_or(TurnError::NoActiveParticipant)?;
        if !reaction && active != action.actor {
            return Err(TurnError::NotActiveActor(action.actor));
        }
        let participant = self
            .participants
            .get_mut(&action.actor)
            .ok_or_else(|| TurnError::MissingParticipant(action.actor.clone()))?;
        if participant.action_points < action.action_point_cost {
            return Err(TurnError::InsufficientActionPoints {
                actor: action.actor,
                required: action.action_point_cost,
                available: participant.action_points,
            });
        }
        participant.action_points -= action.action_point_cost;
        let id = action.id;
        if reaction {
            self.reactions.push_back(action);
        } else {
            self.queued_actions.push_back(action);
        }
        Ok(vec![TacticalEvent::ActionQueued { action: id }])
    }

    fn end_turn(&mut self, actor: &str) -> Result<Vec<TacticalEvent>, TurnError> {
        let active = self.active_actor().ok_or(TurnError::NoActiveParticipant)?;
        if active != actor {
            return Err(TurnError::NotActiveActor(actor.to_string()));
        }
        let mut events = vec![TacticalEvent::TurnEnded {
            actor: actor.to_string(),
        }];
        self.active_index += 1;
        if self.active_index >= self.order.len() {
            events.push(TacticalEvent::RoundEnded { round: self.round });
            self.round = self.round.saturating_add(1);
            self.active_index = 0;
            self.refresh_action_points();
            events.push(TacticalEvent::RoundStarted { round: self.round });
        }
        if let Some(next_actor) = self.active_actor() {
            events.push(TacticalEvent::TurnStarted {
                actor: next_actor.to_string(),
            });
        }
        Ok(events)
    }

    fn refresh_action_points(&mut self) {
        for participant in self.participants.values_mut() {
            participant.action_points = participant.max_action_points;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(id: u64, actor: &str, cost: u16) -> TacticalAction {
        TacticalAction {
            id: ActionId(id),
            actor: actor.to_string(),
            kind: "move".to_string(),
            action_point_cost: cost,
            target: ActionTarget::None,
            payload: BTreeMap::new(),
        }
    }

    #[test]
    fn initiative_then_id_produces_stable_order() {
        let mut turns = TurnDirector::default();
        turns.add_participant(TurnParticipant::new("bravo", FactionId(1), 10, 2));
        turns.add_participant(TurnParticipant::new("alpha", FactionId(1), 10, 2));
        turns.start();
        assert_eq!(turns.active_actor(), Some("alpha"));
    }

    #[test]
    fn commands_spend_action_points() {
        let mut turns = TurnDirector::default();
        turns.add_participant(TurnParticipant::new("alpha", FactionId(1), 10, 2));
        turns.start();
        turns
            .submit(TurnCommand::Queue(action(1, "alpha", 1)))
            .unwrap();
        assert_eq!(turns.participant("alpha").unwrap().action_points, 1);
        assert!(matches!(
            turns.submit(TurnCommand::Queue(action(2, "alpha", 2))),
            Err(TurnError::InsufficientActionPoints { .. })
        ));
    }

    #[test]
    fn ending_last_turn_starts_new_round() {
        let mut turns = TurnDirector::default();
        turns.add_participant(TurnParticipant::new("alpha", FactionId(1), 10, 2));
        turns.start();
        let events = turns
            .submit(TurnCommand::EndTurn {
                actor: "alpha".to_string(),
            })
            .unwrap();
        assert!(events.contains(&TacticalEvent::RoundStarted { round: 2 }));
        assert_eq!(turns.round(), 2);
    }
}
