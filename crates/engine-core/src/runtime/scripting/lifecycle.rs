use crate::*;

impl EngineRuntime {
    /// Register a script backend host (e.g. `ProcessHost` for C#).
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn register_script_host(&mut self, host: Box<dyn ScriptHost>) {
        self.scripting.engine.register_host(host);
    }

    /// Load a script assembly through the named host.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn load_script_assembly(
        &mut self,
        id: &str,
        host_name: &str,
        data: &[u8],
    ) -> Result<(), ScriptError> {
        self.scripting.engine.load_script(id, host_name, data)?;
        Ok(())
    }

    /// Return the concrete managed behaviour classes verified by the active
    /// script hosts when their assemblies were loaded.
    ///
    /// This is the sole editor-facing discovery path: callers receive no
    /// source-derived or conventional default class names.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn verified_script_classes(&self) -> Vec<engine_script::VerifiedScriptClass> {
        self.scripting.engine.verified_classes()
    }

    /// Direct access to the script engine.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_engine(&self) -> &ScriptEngine {
        &self.scripting.engine
    }

    /// Mutable access to the script engine.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_engine_mut(&mut self) -> &mut ScriptEngine {
        &mut self.scripting.engine
    }

    /// Atomically replace the complete script runtime after a caller has
    /// prepared its host and assemblies in isolation.
    ///
    /// The candidate must contain exactly one host with `host_name`.  This is
    /// checked before the active engine is touched, so a malformed candidate
    /// leaves the previous host, assemblies, and instances available.  A
    /// successful replacement cannot accumulate duplicate hosts from prior
    /// reloads because the complete [`ScriptEngine`] is swapped at once.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn replace_script_engine(
        &mut self,
        candidate: ScriptEngine,
        host_name: impl Into<String>,
    ) -> Result<(), ScriptError> {
        let host_name = host_name.into();
        let matching_hosts = candidate
            .managers()
            .iter()
            .filter(|manager| manager.host_name == host_name)
            .count();
        if matching_hosts != 1 {
            return Err(ScriptError::HostError(format!(
                "replacement script engine must contain exactly one host named '{host_name}', found {matching_hosts}"
            )));
        }

        self.clear_scene_script_instances();
        self.scripting.engine = candidate;
        self.scripting.host_name = host_name;
        self.scripting.pending_scene_request = None;
        self.scripting.pending_physics_queries.clear();
        self.scripting.pending_physics_mutations.clear();
        self.scripting.pending_damage_requests.clear();
        self.scripting.damage_events.clear();
        self.scripting.pending_ragdoll_requests.clear();
        self.scripting.ragdoll_events.clear();
        self.scripting.pending_component_queries.clear();
        self.scripting.component_query_results.clear();
        self.scripting.pending_save_requests.clear();
        self.scripting.save_events.clear();
        self.scripting.logic_asset_results.clear();
        self.scripting.runtime_asset_results.clear();
        self.scripting.pending_terrain_brushes.clear();
        self.scripting.pending_network_commands.clear();
        self.scripting.network_operation_results.clear();
        self.scripting.network_snapshot = engine_script::GameplayNetworkSnapshot::default();
        self.scripting.xr_snapshot = engine_script::GameplayXrSnapshot::default();
        Ok(())
    }

    /// Set the script host name used for scene-attached scripts.
    ///
    /// Must match the [`name`](ScriptHost::name) of a registered host.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_host_name(&mut self, name: impl Into<String>) {
        self.scripting.host_name = name.into();
    }

    /// Store the resolved input values used by the next script lifecycle call.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_input_actions(
        &mut self,
        input_actions: std::collections::BTreeMap<String, GameplayInputValue>,
    ) {
        self.scripting.input_actions = input_actions;
    }

    /// Set renderer-consistent pointer and camera data for the next script
    /// lifecycle call.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_view_context(
        &mut self,
        pointer: GameplayPointerSnapshot,
        camera: Option<GameplayCameraSnapshot>,
    ) {
        self.scripting.pointer = pointer;
        self.scripting.camera = camera;
    }

    /// Take the validated physics queries drained from scripts during the
    /// current update, leaving the queue empty.
    ///
    /// The owning game loop executes these against its physics world at the
    /// frame boundary and delivers results in the next frame snapshot.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_physics_queries(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayPhysicsQuery> {
        std::mem::take(&mut self.scripting.pending_physics_queries)
    }

    /// Take the validated forces and impulses drained from scripts during the
    /// current update, leaving the queue empty.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_physics_mutations(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayPhysicsMutation> {
        std::mem::take(&mut self.scripting.pending_physics_mutations)
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_damage_requests(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayDamageRequest> {
        std::mem::take(&mut self.scripting.pending_damage_requests)
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    pub(crate) fn push_script_damage_event(
        &mut self,
        entity_id: String,
        event: GameplayDamageEvent,
    ) {
        self.scripting
            .damage_events
            .entry(entity_id)
            .or_default()
            .push(event);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_ragdoll_requests(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayRagdollRequest> {
        std::mem::take(&mut self.scripting.pending_ragdoll_requests)
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_save_requests(&mut self) -> Vec<engine_script::OwnedGameplaySaveRequest> {
        std::mem::take(&mut self.scripting.pending_save_requests)
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn push_script_save_event(&mut self, entity_id: String, event: GameplaySaveEvent) {
        self.scripting
            .save_events
            .entry(entity_id)
            .or_default()
            .push(event);
    }

    #[cfg(all(
        feature = "subsystem-scripting-csharp",
        feature = "subsystem-physics",
        feature = "subsystem-animation"
    ))]
    pub(crate) fn push_script_ragdoll_event(
        &mut self,
        entity_id: String,
        event: GameplayRagdollEvent,
    ) {
        self.scripting
            .ragdoll_events
            .entry(entity_id)
            .or_default()
            .push(event);
    }

    /// Tick all scripts — call this each frame before `render_frame`.
    ///
    /// Dispatches completed async callbacks, advances native coroutine state,
    /// then calls `OnStart`/`OnUpdate(dt)` on every active script instance.
    /// Resulting script diagnostics are pushed into the collector.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts(&mut self, dt: f32) {
        self.tick_scripts_with_input(dt, &std::collections::BTreeMap::new());
    }

    /// Tick scripts with the resolved project input snapshot for this frame.
    ///
    /// Process hosts receive entity Transform and input data before lifecycle
    /// methods run. Their queued Transform writes are validated and committed
    /// to the ECS world after every script has completed its update.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_input(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
    ) {
        self.tick_scripts_with_input_and_physics(
            dt,
            input_actions,
            &std::collections::BTreeMap::new(),
        );
    }

    /// Tick scripts with input and entity-relative physics events.
    ///
    /// Physics events are frame snapshots. Callers must pass an empty map on
    /// frames without a physics step so stale contacts cannot be observed.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_input_and_physics(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
    ) {
        self.tick_scripts_with_frame_input(
            dt,
            input_actions,
            &GameplayInputTransitions::default(),
            physics_events,
        );
    }

    /// Tick scripts with the complete resolved frame-input snapshot.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_frame_input(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        input_transitions: &GameplayInputTransitions,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
    ) {
        self.tick_scripts_with_frame_input_and_ui(
            dt,
            input_actions,
            input_transitions,
            physics_events,
            &[],
        );
    }

    /// Tick scripts with the complete frame snapshot, including retained UI
    /// clicks drained by the owning [`GameLoop`](crate::game_loop::GameLoop).
    /// The same immutable event slice is copied into every script context.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_frame_input_and_ui(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        input_transitions: &GameplayInputTransitions,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
        ui_events: &[GameplayUiEvent],
    ) {
        self.tick_scripts_with_frame_input_ui_and_physics_queries(
            dt,
            input_actions,
            input_transitions,
            physics_events,
            ui_events,
            &std::collections::BTreeMap::new(),
        );
    }

    /// Tick scripts with the complete frame snapshot, including retained UI
    /// clicks and the physics query results computed by the owning
    /// [`GameLoop`](crate::game_loop::GameLoop) after the previous update.
    ///
    /// Query results are frame snapshots. Callers must pass an empty map on
    /// frames without freshly computed results so stale answers cannot be
    /// observed.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_frame_input_ui_and_physics_queries(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        input_transitions: &GameplayInputTransitions,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
        ui_events: &[GameplayUiEvent],
        physics_query_results: &std::collections::BTreeMap<
            String,
            Vec<engine_script::GameplayPhysicsQueryResult>,
        >,
    ) {
        self.scripting.input_actions.clone_from(input_actions);
        self.activate_ffi_coroutine_runtime();
        engine_ffi::r#async::dispatch_main_thread_callbacks();
        engine_ffi::coroutine::tick_managed_coroutines(dt);
        let contexts = self.script_gameplay_contexts(
            input_actions,
            input_transitions,
            physics_events,
            ui_events,
            physics_query_results,
        );
        let mut diagnostics = self.scripting.engine.set_gameplay_contexts(&contexts);
        self.scripting.damage_events.clear();
        self.scripting.ragdoll_events.clear();
        self.scripting.save_events.clear();
        self.scripting.logic_asset_results.clear();
        self.scripting.runtime_asset_results.clear();
        self.scripting.network_operation_results.clear();
        diagnostics.extend(self.scripting.engine.update(dt));
        let (commands, command_diagnostics) = self.scripting.engine.drain_gameplay_commands();
        diagnostics.extend(command_diagnostics);
        diagnostics.extend(self.apply_script_gameplay_commands(commands));
        // Component queries issued during this update snapshot the world
        // after this frame's commands applied, so next frame's results
        // observe same-frame component writes.
        diagnostics.extend(self.execute_script_component_queries());
        self.collector.push_script_diags(diagnostics);
    }

    // ── Private helpers ─────────────────────────────────────────────────

    /// Destroy and remove all script instances attached to the active scene.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn clear_scene_script_instances(&mut self) {
        // OnDestroy must run while the previous World is still active.  The
        // manager then needs an explicit clear because destroy() deliberately
        // preserves instance records for lifecycle inspection.
        let destroy_diags = self.scripting.engine.destroy_instances();
        self.collector.push_script_diags(destroy_diags);
        for manager in self.scripting.engine.managers_mut() {
            manager.clear();
        }
    }

    /// Iterate scene entities and attach any `"engine.script"` components.
    ///
    /// Whole-scene loading and additive partition-cell loading share this
    /// path. Callers must first materialise all owning entities in the active
    /// World so OnCreate receives a complete gameplay context.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn attach_scene_scripts(&mut self, scene: &Scene) {
        let scripts = collect_scene_scripts(scene);
        let host_name = &self.scripting.host_name;
        for (entity_id, component) in &scripts {
            // The assembly must have been loaded externally (e.g. via
            // `load_script_assembly`). If it hasn't, the attach will
            // produce a ScriptError and we push a diagnostic.
            match self
                .scripting
                .engine
                .attach_script(entity_id, host_name, component)
            {
                Ok(()) => {}
                Err(e) => {
                    let diag = Diagnostic::new(
                        "SCR_ATTACH_FAILED",
                        DiagnosticSeverity::Error,
                        "engine-core",
                        format!(
                            "Failed to attach script '{}' to entity '{}': {e}",
                            component.class_name, entity_id
                        ),
                    );
                    self.collector.push_script_diags(vec![diag]);
                }
            }
        }

        // OnCreate receives the owning entity's Transform. Input actions are
        // frame data and start empty until GameLoop performs its first update.
        let contexts = self.script_gameplay_contexts(
            &self.scripting.input_actions,
            &GameplayInputTransitions::default(),
            &std::collections::BTreeMap::new(),
            &[],
            &std::collections::BTreeMap::new(),
        );
        let context_diags = self.scripting.engine.set_gameplay_contexts(&contexts);
        self.collector.push_script_diags(context_diags);

        // Call OnCreate on all newly-attached instances
        let create_diags = self.scripting.engine.create_instances();
        self.collector.push_script_diags(create_diags);

        // OnCreate may change Transform. Commit those commands immediately so
        // the first-frame context does not overwrite the managed change.
        let (commands, mut command_diags) = self.scripting.engine.drain_gameplay_commands();
        command_diags.extend(self.apply_script_gameplay_commands(commands));
        // Component queries issued from OnCreate are answered with the first
        // OnUpdate snapshot instead of waiting for a full extra frame.
        command_diags.extend(self.execute_script_component_queries());
        self.collector.push_script_diags(command_diags);
    }

    /// Run `OnDestroy` and detach scripts owned by streamed entities while
    /// their ECS records are still live. Resident entity IDs must be filtered
    /// by the cell driver before calling this method.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn destroy_streamed_script_instances(
        &mut self,
        entity_ids: &[engine_serialize::PersistentId],
    ) {
        let mut diagnostics = Vec::new();
        // Children conventionally appear after parents in authored scenes.
        // Destroy in reverse order so dependent child behaviours tear down
        // before their owning hierarchy roots.
        for entity_id in entity_ids.iter().rev() {
            diagnostics.extend(
                self.scripting
                    .engine
                    .destroy_entity_instances(entity_id.as_str()),
            );
        }
        self.collector.push_script_diags(diagnostics);
    }
}
