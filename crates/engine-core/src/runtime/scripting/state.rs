use std::collections::BTreeMap;

use engine_script::{
    GameplayCameraSnapshot, GameplayComponentQueryResult, GameplayDamageEvent, GameplayInputValue,
    GameplayLogicAssetResult, GameplayPointerSnapshot, GameplayRagdollEvent,
    GameplayRuntimeAssetResult, GameplaySaveEvent, OwnedGameplayComponentQuery,
    OwnedGameplayDamageRequest, OwnedGameplayPhysicsMutation, OwnedGameplayPhysicsQuery,
    OwnedGameplayRagdollRequest, OwnedGameplaySaveRequest, OwnedGameplayTerrainBrushRequest,
    ScriptEngine,
};

use crate::SceneLoadRequest;

/// Cohesive state owned by the managed-gameplay adapter.
///
/// Keeping this behind one leaf-feature field prevents [`crate::EngineRuntime`]
/// from accumulating one conditional field per scripting protocol queue.
pub(crate) struct ScriptRuntimeState {
    pub(crate) engine: ScriptEngine,
    pub(crate) host_name: String,
    pub(crate) input_actions: BTreeMap<String, GameplayInputValue>,
    pub(crate) pointer: GameplayPointerSnapshot,
    pub(crate) camera: Option<GameplayCameraSnapshot>,
    pub(crate) pending_save_requests: Vec<OwnedGameplaySaveRequest>,
    pub(crate) save_events: BTreeMap<String, Vec<GameplaySaveEvent>>,
    pub(crate) logic_asset_results: BTreeMap<String, Vec<GameplayLogicAssetResult>>,
    pub(crate) runtime_asset_results: BTreeMap<String, Vec<GameplayRuntimeAssetResult>>,
    pub(crate) pending_terrain_brushes: Vec<OwnedGameplayTerrainBrushRequest>,
    pub(crate) pending_scene_request: Option<SceneLoadRequest>,
    pub(crate) pending_physics_queries: Vec<OwnedGameplayPhysicsQuery>,
    pub(crate) pending_physics_mutations: Vec<OwnedGameplayPhysicsMutation>,
    pub(crate) pending_damage_requests: Vec<OwnedGameplayDamageRequest>,
    pub(crate) damage_events: BTreeMap<String, Vec<GameplayDamageEvent>>,
    pub(crate) pending_ragdoll_requests: Vec<OwnedGameplayRagdollRequest>,
    pub(crate) ragdoll_events: BTreeMap<String, Vec<GameplayRagdollEvent>>,
    pub(crate) pending_component_queries: Vec<OwnedGameplayComponentQuery>,
    pub(crate) component_query_results: BTreeMap<String, Vec<GameplayComponentQueryResult>>,
}

impl Default for ScriptRuntimeState {
    fn default() -> Self {
        Self {
            engine: ScriptEngine::new(),
            host_name: "dotnet".to_string(),
            input_actions: BTreeMap::new(),
            pointer: GameplayPointerSnapshot::default(),
            camera: None,
            pending_save_requests: Vec::new(),
            save_events: BTreeMap::new(),
            logic_asset_results: BTreeMap::new(),
            runtime_asset_results: BTreeMap::new(),
            pending_terrain_brushes: Vec::new(),
            pending_scene_request: None,
            pending_physics_queries: Vec::new(),
            pending_physics_mutations: Vec::new(),
            pending_damage_requests: Vec::new(),
            damage_events: BTreeMap::new(),
            pending_ragdoll_requests: Vec::new(),
            ragdoll_events: BTreeMap::new(),
            pending_component_queries: Vec::new(),
            component_query_results: BTreeMap::new(),
        }
    }
}
