//! Engine-level building blocks for data-driven, party-based role-playing games.
//!
//! The module owns deterministic rules and serializable state only. Rendering,
//! localization text, authored content, camera direction, and UI presentation
//! remain project responsibilities connected through definitions, commands,
//! and events.

mod combat;
mod database;
mod encounter;
mod inventory;
mod localization;
mod narrative;
mod party;
mod progression;
mod sequence;
mod session;

pub use combat::{
    AbilityDefinition, ActiveStatus, BattleCommand, BattleEffect, BattleError, BattleEvent,
    BattleFormula, BattlePhase, BattleSession, BattleSide, BattleUnit, ClassicBattleFormula,
    CombatDamageKind, DamageKind, Element, StatusDefinition, TargetRule,
};
pub use database::{ActorDefinition, DatabaseError, EnemyDefinition, JrpgDatabase};
pub use encounter::{EncounterError, EncounterMeter, EncounterTable, EnemyFormation};
pub use inventory::{EquipmentDefinition, Inventory, InventoryError, ItemDefinition, ItemKind};
pub use localization::{LocalizationCatalog, LocalizationError};
pub use narrative::{
    DialogueBeat, DialogueChoice, DialogueGraph, DialogueNode, DialogueRunner, NarrativeCommand,
    NarrativeError, QuestDefinition, QuestObjective, QuestProgress, QuestStatus, StoryCondition,
    StoryEffect, StoryState, StoryValue,
};
pub use party::{Party, PartyError};
pub use progression::{
    CharacterProgress, ExperienceCurve, LevelGain, QuadraticExperienceCurve, StatBlock,
    StatModifier,
};
pub use sequence::{SequenceCommand, SequenceDefinition, SequenceRunner, SequenceStep};
pub use session::{BattleRewards, JrpgSession, JrpgSessionError, JRPG_SESSION_SCHEMA_VERSION};
