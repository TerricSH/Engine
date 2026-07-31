use std::collections::BTreeMap;

use engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA;
use serde::{Deserialize, Serialize};

use super::components::GameplayComponentQueryResult;
use super::events::{GameplayDamageEvent, GameplayPhysicsEvent, GameplayRagdollEvent};
use super::physics_query::GameplayPhysicsQueryResult;
use super::snapshots::{
    GameplayCameraSnapshot, GameplayEntitySnapshot, GameplayInputTransitions, GameplayInputValue,
    GameplayLogicAssetResult, GameplayPointerSnapshot, GameplaySaveEvent, ScriptTransform,
};
use super::ui::GameplayUiEvent;

fn default_gameplay_script_api_schema() -> String {
    GAMEPLAY_SCRIPT_API_SCHEMA.to_owned()
}
/// Frame-local data made available to one script instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayContext {
    /// Versioned boundary implemented by the engine-owned generated C# API.
    ///
    /// The default preserves compatibility with contexts recorded before the
    /// explicit handshake was added. New runtimes always serialize this field,
    /// and managed hosts reject a different schema before invoking game code.
    #[serde(default = "default_gameplay_script_api_schema")]
    pub script_api: String,
    pub entity_id: String,
    pub transform: Option<ScriptTransform>,
    /// Current runtime world origin (ENG-01 Phase 2).
    ///
    /// Every `ScriptTransform` a script sees is **relative** to this origin:
    /// the logical position of an entity is `world_origin + translation`.
    /// Read-only for scripts; the origin changes only through the periodic
    /// world-origin shift at the frame boundary. `default` keeps contexts
    /// produced before origin shifting existed compatible (zero origin).
    #[serde(default)]
    pub world_origin: [f64; 3],
    pub input_actions: BTreeMap<String, GameplayInputValue>,
    #[serde(default)]
    pub input_transitions: GameplayInputTransitions,
    /// Pointing-device state and the current cursor ray.
    #[serde(default)]
    pub pointer: GameplayPointerSnapshot,
    /// Renderer-consistent active camera data.
    #[serde(default)]
    pub camera: Option<GameplayCameraSnapshot>,
    /// Results of save/load requests issued by this script instance.
    #[serde(default)]
    pub save_events: Vec<GameplaySaveEvent>,
    /// Deferred data-driven logic graph query results.
    #[serde(default)]
    pub logic_asset_results: Vec<GameplayLogicAssetResult>,
    /// Collision and trigger events involving the owning entity this frame.
    #[serde(default)]
    pub physics_events: Vec<GameplayPhysicsEvent>,
    /// Damage accepted for the owning entity or issued by it in the previous
    /// frame. Events are delivered exactly once.
    #[serde(default)]
    pub damage_events: Vec<GameplayDamageEvent>,
    /// Confirmed ragdoll ownership changes from the previous frame.
    #[serde(default)]
    pub ragdoll_events: Vec<GameplayRagdollEvent>,
    /// Results of physics queries issued by the owning script instance in a
    /// previous frame.
    ///
    /// Queries are deferred commands: the engine executes them at the frame
    /// boundary and answers with the next frame's snapshot. `default` keeps
    /// contexts produced before physics queries existed compatible with the
    /// current script hosts.
    #[serde(default)]
    pub physics_query_results: Vec<GameplayPhysicsQueryResult>,
    /// Results of component queries issued by the owning script instance in a
    /// previous frame.
    ///
    /// Component queries follow the same deferred, frame-local contract as
    /// physics queries: the engine snapshots the requested component at the
    /// frame boundary and answers with exactly one following snapshot.
    /// `default` keeps contexts produced before component access existed
    /// compatible with the current script hosts.
    #[serde(default)]
    pub component_query_results: Vec<GameplayComponentQueryResult>,
    /// Runtime UI clicks delivered during this frame.
    ///
    /// `default` keeps contexts produced before gameplay UI events were added
    /// compatible with current script hosts.
    #[serde(default)]
    pub ui_events: Vec<GameplayUiEvent>,
    /// Snapshot of every persistent entity in the active World.
    ///
    /// `default` keeps contexts produced by older runtimes deserializable by
    /// the current API assembly.
    #[serde(default)]
    pub entities: BTreeMap<String, GameplayEntitySnapshot>,
}
