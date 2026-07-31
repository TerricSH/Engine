use super::world::*;
use crate::*;

impl EngineRuntime {
    /// Execute the component queries drained from the latest script update
    /// and stage the results for the next frame snapshot.
    ///
    /// Queries run against the active World after this frame's commands
    /// apply, so answers observe same-frame component writes. Results are
    /// frame-local: they replace the previous staging map and are consumed by
    /// exactly one following script tick.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn execute_script_component_queries(&mut self) -> Vec<Diagnostic> {
        let pending = std::mem::take(&mut self.scripting.pending_component_queries);
        if pending.is_empty() {
            self.scripting.component_query_results.clear();
            return Vec::new();
        }
        let mut diagnostics = Vec::new();
        let mut results: std::collections::BTreeMap<
            String,
            Vec<engine_script::GameplayComponentQueryResult>,
        > = std::collections::BTreeMap::new();
        for engine_script::OwnedGameplayComponentQuery { entity_id, query } in pending {
            let outcome = self.world_slot.with_world(|world| {
                (
                    script_components::read_script_component(
                        world,
                        &query.entity_id,
                        &query.component_type,
                    ),
                    script_components::supported_script_component_types(world),
                )
            });
            use engine_script::GameplayComponentQueryResult as QueryResult;
            use script_components::ScriptComponentRead as Read;
            let result = match outcome {
                Some((Read::Snapshot(fields), _)) => QueryResult::Snapshot {
                    query_id: query.query_id,
                    entity_id: query.entity_id,
                    component_type: query.component_type,
                    fields,
                },
                Some((Read::Missing, _)) => QueryResult::Missing {
                    query_id: query.query_id,
                    entity_id: query.entity_id,
                    component_type: query.component_type,
                },
                Some((Read::Unsupported, supported)) => {
                    diagnostics.push(script_component_diagnostic(
                        "SCRIPT_COMPONENT_UNKNOWN",
                        &entity_id,
                        format!(
                            "script entity '{entity_id}' queried component '{}' on entity '{}', but that type is not script-accessible; {}",
                            query.component_type,
                            query.entity_id,
                            supported_script_component_description(&supported)
                        ),
                    ));
                    continue;
                }
                None => {
                    diagnostics.push(script_component_diagnostic(
                        "SCRIPT_WORLD_MISSING",
                        &entity_id,
                        format!(
                            "script entity '{entity_id}' component query on entity '{}' could not run because no World is active",
                            query.entity_id
                        ),
                    ));
                    continue;
                }
            };
            results.entry(entity_id).or_default().push(result);
        }
        self.scripting.component_query_results = results;
        diagnostics
    }

    /// Instantiate one cooked prefab for a `Scene.Spawn` command.
    ///
    /// The prefab is resolved from the runtime's cooked `prefab` extension
    /// assets (never from script-supplied paths), instantiated transactionally
    /// through the same component restoration path scenes use, and assigned
    /// deterministic persistent IDs. `engine.script` records ride along as
    /// scene-only metadata and are attached to the script engine with the same
    /// lifecycle as scene-authored scripts: `OnCreate` runs immediately, and
    /// its commands are applied recursively within this frame boundary.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn spawn_script_prefab(
        &mut self,
        requested_by: &str,
        prefab_id: &str,
        translation: Option<[f32; 3]>,
        diagnostics: &mut Vec<Diagnostic>,
        depth: usize,
    ) {
        let asset_id = AssetId::new(prefab_id);
        let Some(root_handle) = self.extension_asset::<engine_scene::Prefab>("prefab", &asset_id)
        else {
            let mut diagnostic = Diagnostic::new(
                "SCRIPT_PREFAB_UNKNOWN",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "script entity '{requested_by}' requested unknown prefab '{prefab_id}'; {}",
                    self.available_prefab_description()
                ),
            );
            diagnostic.entity = Some(requested_by.to_owned());
            diagnostics.push(diagnostic);
            return;
        };
        let root_prefab = root_handle.get().clone();

        // Nested prefab references resolve against the same cooked batch.
        let mut resolver = engine_scene::PrefabRegistry::new();
        let mut visiting = std::collections::BTreeSet::new();
        if let Err(missing) =
            self.collect_prefab_graph(&asset_id, &root_prefab, &mut resolver, &mut visiting)
        {
            let mut diagnostic = Diagnostic::new(
                "SCRIPT_PREFAB_GRAPH_INCOMPLETE",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "script entity '{requested_by}' requested prefab '{prefab_id}', but its nested prefab '{missing}' is not loaded; declare and cook every referenced prefab asset"
                ),
            );
            diagnostic.entity = Some(requested_by.to_owned());
            diagnostics.push(diagnostic);
            return;
        }

        let outcome = self.world_slot.with_world_mut(|world| {
            match engine_scene::instantiate_prefab(world, &root_prefab, Some(&resolver)) {
                Ok(result) => {
                    match assign_spawned_persistent_ids(world, prefab_id, &result) {
                        Ok(assigned) => {
                            if let Some(translation) = translation {
                                apply_spawn_translation(world, result.root_entity, translation);
                            }
                            Ok((result, assigned))
                        }
                        Err(reason) => {
                            // Roll the whole instance back so a failed spawn
                            // cannot leave anonymous or partially named
                            // entities behind.
                            for entity in result.all_entities.iter().rev() {
                                let _ = world.destroy_entity(*entity);
                            }
                            Err(reason)
                        }
                    }
                }
                Err(error) => Err(error.to_string()),
            }
        });

        let (result, assigned) = match outcome {
            Some(Ok(spawned)) => spawned,
            Some(Err(reason)) => {
                let mut diagnostic = Diagnostic::new(
                    "SCRIPT_PREFAB_SPAWN_FAILED",
                    DiagnosticSeverity::Error,
                    "script",
                    format!(
                        "script entity '{requested_by}' could not spawn prefab '{prefab_id}': {reason}"
                    ),
                );
                diagnostic.entity = Some(requested_by.to_owned());
                diagnostics.push(diagnostic);
                return;
            }
            None => {
                diagnostics.push(Diagnostic::new(
                    "SCRIPT_WORLD_MISSING",
                    DiagnosticSeverity::Error,
                    "script",
                    format!(
                        "script entity '{requested_by}' could not spawn prefab '{prefab_id}' because no World is active"
                    ),
                ));
                return;
            }
        };

        // Attach scene-only `engine.script` records with the same lifecycle
        // scene-authored scripts receive.
        let id_by_entity: std::collections::HashMap<engine_scene::Entity, String> =
            assigned.into_iter().collect();
        let mut attached_any = false;
        for (entity, component_type_id, record) in &result.scene_only_components {
            if component_type_id != script::SCRIPT_COMPONENT_TYPE {
                continue;
            }
            let Some(entity_id) = id_by_entity.get(entity) else {
                continue;
            };
            let Some(component) = script::extract_script_component_from_record(record) else {
                let mut diagnostic = Diagnostic::new(
                    "SCRIPT_SPAWN_ATTACH_FAILED",
                    DiagnosticSeverity::Error,
                    "script",
                    format!(
                        "spawned entity '{entity_id}' has an invalid engine.script record: 'assembly_id' and 'class_name' strings are required"
                    ),
                );
                diagnostic.entity = Some(entity_id.clone());
                diagnostics.push(diagnostic);
                continue;
            };
            match self.scripting.engine.attach_script(
                entity_id,
                &self.scripting.host_name.clone(),
                &component,
            ) {
                Ok(()) => attached_any = true,
                Err(error) => {
                    let mut diagnostic = Diagnostic::new(
                        "SCRIPT_SPAWN_ATTACH_FAILED",
                        DiagnosticSeverity::Error,
                        "script",
                        format!(
                            "failed to attach script '{}' to spawned entity '{entity_id}': {error}",
                            component.class_name
                        ),
                    );
                    diagnostic.entity = Some(entity_id.clone());
                    diagnostics.push(diagnostic);
                }
            }
        }
        if !attached_any {
            return;
        }

        // Run OnCreate for the newly attached instances and apply the
        // commands they enqueue (including further spawns) at this same frame
        // boundary, bounded by MAX_SCRIPT_SPAWN_DEPTH.
        let contexts = self.script_gameplay_contexts(
            &self.scripting.input_actions.clone(),
            &GameplayInputTransitions::default(),
            &std::collections::BTreeMap::new(),
            &[],
            &std::collections::BTreeMap::new(),
        );
        diagnostics.extend(self.scripting.engine.set_gameplay_contexts(&contexts));
        diagnostics.extend(self.scripting.engine.create_instances());
        let (commands, command_diagnostics) = self.scripting.engine.drain_gameplay_commands();
        diagnostics.extend(command_diagnostics);
        if commands.is_empty() {
            return;
        }
        if depth >= MAX_SCRIPT_SPAWN_DEPTH {
            diagnostics.push(Diagnostic::new(
                "SCRIPT_SPAWN_DEPTH_EXCEEDED",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "prefab spawn chains from OnCreate callbacks exceeded the depth budget of {MAX_SCRIPT_SPAWN_DEPTH}; remaining commands were deferred"
                ),
            ));
            return;
        }
        diagnostics.extend(self.apply_script_gameplay_commands_with_depth(commands, depth + 1));
    }

    /// Human-readable list of the loaded cooked prefab assets for actionable
    /// `Scene.Spawn` error messages.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn available_prefab_description(&self) -> String {
        let available = self
            .loaded_extension_asset_ids
            .get("prefab")
            .map(|ids| {
                ids.iter()
                    .map(|id| id.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        if available.is_empty() {
            "no prefab assets are loaded; declare .prefab.ron sources in the project's source manifest and cook the project".to_string()
        } else {
            format!("loaded prefabs: {available}")
        }
    }

    /// Register the reachable nested-prefab graph of `root` with the resolver.
    ///
    /// Cycles are left to the instantiation validator, which reports them as
    /// structured errors; this walk only proves that every referenced child is
    /// present in the cooked batch.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn collect_prefab_graph(
        &self,
        asset_id: &AssetId,
        prefab: &engine_scene::Prefab,
        resolver: &mut engine_scene::PrefabRegistry,
        visiting: &mut std::collections::BTreeSet<String>,
    ) -> Result<(), String> {
        if !visiting.insert(asset_id.id.clone()) {
            return Ok(());
        }
        for child_ref in &prefab.child_prefab_refs {
            let child_id = &child_ref.prefab_asset;
            let child = self
                .extension_asset::<engine_scene::Prefab>("prefab", child_id)
                .ok_or_else(|| child_id.id.clone())?;
            let child = child.get().clone();
            self.collect_prefab_graph(child_id, &child, resolver, visiting)?;
            resolver.register(child_id.id.clone(), child);
        }
        Ok(())
    }
}
