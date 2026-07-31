use crate::*;

impl EngineRuntime {
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn script_gameplay_contexts(
        &self,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        input_transitions: &GameplayInputTransitions,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
        ui_events: &[GameplayUiEvent],
        physics_query_results: &std::collections::BTreeMap<
            String,
            Vec<engine_script::GameplayPhysicsQueryResult>,
        >,
    ) -> std::collections::BTreeMap<String, GameplayContext> {
        let entity_ids = self
            .scripting
            .engine
            .managers()
            .iter()
            .flat_map(|manager| manager.iter_instances().map(|(entity_id, _, _)| entity_id))
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();

        let entities = self.script_gameplay_entity_snapshots();
        let world_origin = self
            .world_slot
            .with_world(|world| world.world_origin())
            .unwrap_or([0.0; 3]);

        entity_ids
            .into_iter()
            .map(|entity_id| {
                let context = GameplayContext {
                    script_api: engine_script::GAMEPLAY_SCRIPT_API_SCHEMA.to_owned(),
                    transform: entities
                        .get(&entity_id)
                        .and_then(|snapshot| snapshot.transform.clone()),
                    entity_id: entity_id.clone(),
                    world_origin,
                    input_actions: input_actions.clone(),
                    input_transitions: input_transitions.clone(),
                    pointer: self.scripting.pointer.clone(),
                    camera: self.scripting.camera.clone(),
                    save_events: self
                        .scripting
                        .save_events
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    logic_asset_results: self
                        .scripting
                        .logic_asset_results
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    physics_events: physics_events.get(&entity_id).cloned().unwrap_or_default(),
                    damage_events: self
                        .scripting
                        .damage_events
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    ragdoll_events: self
                        .scripting
                        .ragdoll_events
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    physics_query_results: physics_query_results
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    component_query_results: self
                        .scripting
                        .component_query_results
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    ui_events: ui_events.to_vec(),
                    entities: entities.clone(),
                };
                (entity_id, context)
            })
            .collect()
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn script_gameplay_entity_snapshots(
        &self,
    ) -> std::collections::BTreeMap<String, GameplayEntitySnapshot> {
        self.world_slot
            .with_world(|world| {
                world
                    .persistent_entities()
                    .map(|(entity_id, entity)| {
                        let transform = world
                            .get::<engine_scene::components::Transform>(entity)
                            .map(|transform| ScriptTransform {
                                translation: transform.translation.to_array(),
                                rotation: transform.rotation.to_array(),
                                scale: transform.scale.to_array(),
                            });
                        (entity_id.to_owned(), GameplayEntitySnapshot { transform })
                    })
                    .collect::<std::collections::BTreeMap<_, _>>()
            })
            .unwrap_or_default()
    }
}
