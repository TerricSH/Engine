//! Engine-level building blocks for deterministic, turn-based tactical games.
//!
//! This module contains rules and state only. Rendering, animation and project
//! specific abilities stay outside the domain and integrate through commands,
//! events and strategy traits.

mod ai;
mod combat;
mod grid;
mod pathfinding;
mod turns;
mod types;
mod visibility;

pub use crate::random::DeterministicRng;
pub use ai::{FactCurve, ScoredOption, UtilityBrain, UtilityConsideration, UtilityOption};
pub use combat::{
    AbilitySpec, AttackContext, CombatOutcome, CombatResolver, Combatant, CombatantStats,
    DefaultHitChancePolicy, HitChancePolicy, StatusEffect, WeaponProfile,
};
pub use grid::{
    BoardError, CardinalDirection, CoverEdges, CoverKind, GridCoord, TacticalBoard, TacticalCell,
};
pub use pathfinding::{
    DefaultMovementCostPolicy, MovementCostPolicy, ReachabilityMap, TacticalPath,
    TacticalPathfinder,
};
pub use turns::{
    ActionTarget, TacticalAction, TacticalActionExecutor, TacticalEvent, TurnCommand, TurnDirector,
    TurnError, TurnParticipant, TurnPhase,
};
pub use types::{ActionId, FactionId, TacticalEntityId};
pub use visibility::{GridLineOfSight, LineOfSightPolicy, VisibilityMap, VisibilityState};

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Serializable façade for a running tactical encounter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticalSession {
    pub board: TacticalBoard,
    pub turns: TurnDirector,
    pub visibility: VisibilityMap,
    pub combatants: BTreeMap<TacticalEntityId, Combatant>,
    pub random: DeterministicRng,
}

impl TacticalSession {
    pub fn new(seed: u64) -> Self {
        Self {
            board: TacticalBoard::default(),
            turns: TurnDirector::default(),
            visibility: VisibilityMap::default(),
            combatants: BTreeMap::new(),
            random: DeterministicRng::new(seed),
        }
    }

    pub fn add_unit(
        &mut self,
        combatant: Combatant,
        participant: TurnParticipant,
        coord: GridCoord,
    ) -> Result<(), BoardError> {
        self.board.occupy(combatant.entity.clone(), coord)?;
        self.turns.add_participant(participant);
        self.combatants.insert(combatant.entity.clone(), combatant);
        Ok(())
    }

    pub fn remove_unit(&mut self, entity: &str) -> Option<Combatant> {
        self.board.vacate(entity);
        self.combatants.remove(entity)
    }
}

impl Default for TacticalSession {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trips_through_json() {
        let mut session = TacticalSession::new(123);
        let coord = GridCoord::new(0, 0, 0);
        session
            .board
            .insert_cell(TacticalCell::walkable(coord, [0.0, 0.0, 0.0]));
        let stats = CombatantStats {
            max_health: 5,
            aim: 0,
            defense: 0,
            armor: 0,
            mobility: 6,
            will: 0,
        };
        session
            .add_unit(
                Combatant::new("unit", stats),
                TurnParticipant::new("unit", FactionId(1), 10, 2),
                coord,
            )
            .unwrap();
        let json = serde_json::to_string(&session).unwrap();
        let restored: TacticalSession = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, session);
    }
}
