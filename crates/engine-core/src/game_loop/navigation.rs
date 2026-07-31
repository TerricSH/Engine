use super::*;

impl GameLoop {
    /// Evaluate navigation agents and queue their movement intent on the
    /// CharacterController attached to the same entity. The primary player
    /// mirror is refreshed so its normal update consumes the queued command.
    #[cfg(feature = "subsystem-navigation")]
    pub(super) fn queue_runtime_navigation(&mut self, dt: f32) {
        let navmeshes = self
            .runtime
            .asset_registry()
            .cached_ids()
            .into_iter()
            .filter_map(|id| {
                self.runtime
                    .extension_asset::<engine_nav::NavMesh>("navmesh", &id)
                    .map(|handle| (id.id, handle.get().clone()))
            })
            .collect::<std::collections::HashMap<_, _>>();
        let primary = self.character_entity;
        let updated_primary = self
            .runtime
            .with_world_mut(|world| {
                // Path queries run in the navmesh's authored (logical)
                // space; agent and controller state stays origin-relative.
                let origin = {
                    let origin = world.world_origin();
                    Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32)
                };
                let entities = world
                    .query::<engine_nav::AiAgent>()
                    .map(|(entity, _)| entity)
                    .collect::<Vec<_>>();
                let mut updated_primary = None;
                for entity in entities {
                    let Some(mut agent) = world.get::<engine_nav::AiAgent>(entity).cloned() else {
                        continue;
                    };
                    // Persistent scene data cannot safely encode a raw ECS
                    // generation. Zero therefore means the supported and
                    // portable same-entity controller binding.
                    if agent.controller_entity_id != 0 {
                        continue;
                    }
                    let Some(navmesh_id) = agent.navmesh_ref.as_deref() else {
                        continue;
                    };
                    let Some(navmesh) = navmeshes.get(navmesh_id) else {
                        continue;
                    };
                    let Some(mut controller) = world.get::<CharacterController>(entity).cloned()
                    else {
                        continue;
                    };
                    engine_nav::update_ai_agent_with_world_origin(
                        &mut agent,
                        &mut controller,
                        navmesh,
                        dt,
                        origin,
                    );
                    if let Some(component) = world.get_mut::<engine_nav::AiAgent>(entity) {
                        *component = agent;
                    }
                    if let Some(component) = world.get_mut::<CharacterController>(entity) {
                        *component = controller.clone();
                    }
                    if Some(entity) == primary {
                        updated_primary = Some(controller);
                    }
                }
                updated_primary
            })
            .flatten();
        if let Some(controller) = updated_primary {
            self.character = Some(controller);
        }
    }
}
