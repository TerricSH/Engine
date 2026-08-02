use crate::EngineRuntime;

impl EngineRuntime {
    pub(crate) fn set_script_network_snapshot(
        &mut self,
        snapshot: engine_script::GameplayNetworkSnapshot,
    ) {
        self.scripting.network_snapshot = snapshot;
    }

    pub(crate) fn take_pending_network_commands(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayNetworkCommand> {
        std::mem::take(&mut self.scripting.pending_network_commands)
    }

    pub(crate) fn push_script_network_result(
        &mut self,
        owner_entity_id: String,
        result: engine_script::GameplayNetworkOperationResult,
    ) {
        self.scripting
            .network_operation_results
            .entry(owner_entity_id)
            .or_default()
            .push(result);
    }

    pub(crate) fn set_script_xr_snapshot(&mut self, snapshot: engine_script::GameplayXrSnapshot) {
        self.scripting.xr_snapshot = snapshot;
    }
}
