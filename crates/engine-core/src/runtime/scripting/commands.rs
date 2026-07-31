use super::world::*;
use crate::*;

impl EngineRuntime {
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn apply_script_gameplay_commands(
        &mut self,
        commands: Vec<engine_script::OwnedGameplayCommand>,
    ) -> Vec<Diagnostic> {
        self.apply_script_gameplay_commands_with_depth(commands, 0)
    }

    /// Apply validated script commands at the frame boundary.
    ///
    /// `depth` bounds the synchronous `Scene.Spawn` → `OnCreate` → command
    /// chain: prefabs spawned from a spawned script's `OnCreate` are applied
    /// recursively so each new instance completes its lifecycle within the
    /// same frame boundary, up to [`MAX_SCRIPT_SPAWN_DEPTH`].
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn apply_script_gameplay_commands_with_depth(
        &mut self,
        commands: Vec<engine_script::OwnedGameplayCommand>,
        depth: usize,
    ) -> Vec<Diagnostic> {
        if commands.is_empty() {
            return Vec::new();
        }
        if self.world_slot.with_world(|_| ()).is_none() {
            return vec![Diagnostic::new(
                "SCRIPT_WORLD_MISSING",
                DiagnosticSeverity::Error,
                "script",
                "gameplay commands could not be applied because no World is active",
            )];
        }

        let mut diagnostics = Vec::new();
        let mut scene_request: Option<SceneLoadRequest> = None;
        for engine_script::OwnedGameplayCommand { entity_id, command } in commands {
            match command {
                GameplayCommand::SetTransform { transform } => {
                    apply_script_transform_command(
                        &self.world_slot,
                        &entity_id,
                        &entity_id,
                        transform,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::SetEntityTransform {
                    entity_id: target_id,
                    transform,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "set another entity's Transform",
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_entity_id(&target_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_ENTITY_TARGET_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested an invalid Transform target: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    apply_script_transform_command(
                        &self.world_slot,
                        &entity_id,
                        &target_id,
                        transform,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::CreateEntity {
                    entity_id: target_id,
                    transform,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "create a persistent entity",
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_entity_id(&target_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_ENTITY_CREATE_ID_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested an invalid entity creation target: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    create_script_entity(
                        &self.world_slot,
                        &entity_id,
                        &target_id,
                        transform,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::DestroySelf => {
                    destroy_script_entity(
                        &self.world_slot,
                        &mut self.scripting.engine,
                        &entity_id,
                        &entity_id,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::DestroyEntity {
                    entity_id: target_id,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "destroy another entity",
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_entity_id(&target_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_ENTITY_TARGET_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested an invalid destroy target: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    destroy_script_entity(
                        &self.world_slot,
                        &mut self.scripting.engine,
                        &entity_id,
                        &target_id,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::LoadScene { scene_id } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            &format!("request scene '{scene_id}'"),
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_scene_id(&scene_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_SCENE_REQUEST_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!("script entity '{entity_id}' {reason}"),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    let request = SceneLoadRequest {
                        scene_id,
                        requested_by: entity_id.clone(),
                    };
                    if let Some(existing) = &scene_request {
                        if existing != &request {
                            let mut diagnostic = Diagnostic::new(
                                "SCRIPT_SCENE_REQUEST_CONFLICT",
                                DiagnosticSeverity::Error,
                                "script",
                                format!(
                                    "script entity '{}' requested scene '{}' after '{}' already requested '{}'; the first request wins",
                                    request.requested_by,
                                    request.scene_id,
                                    existing.requested_by,
                                    existing.scene_id,
                                ),
                            );
                            diagnostic.entity = Some(request.requested_by);
                            diagnostics.push(diagnostic);
                        }
                    } else {
                        scene_request = Some(request);
                    }
                }
                GameplayCommand::SpawnPrefab {
                    prefab_id,
                    translation,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            &format!("spawn prefab '{prefab_id}'"),
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_prefab_id(&prefab_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PREFAB_ID_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!("script entity '{entity_id}' {reason}"),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    if translation.is_some_and(|translation| {
                        !translation.iter().all(|value| value.is_finite())
                    }) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PREFAB_TRANSFORM_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested prefab '{prefab_id}' with a non-finite spawn translation"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    self.spawn_script_prefab(
                        &entity_id,
                        &prefab_id,
                        translation,
                        &mut diagnostics,
                        depth,
                    );
                }
                GameplayCommand::ApplyDamage {
                    entity_id: target_id,
                    amount,
                    damage_kind,
                    hit_position,
                    impulse,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics
                            .push(script_owner_missing_diagnostic(&entity_id, "apply damage"));
                        continue;
                    }
                    let command = GameplayCommand::ApplyDamage {
                        entity_id: target_id.clone(),
                        amount,
                        damage_kind,
                        hit_position,
                        impulse,
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_DAMAGE_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid damage request: {reason}"
                            ),
                        ));
                        continue;
                    }
                    let target_exists = self
                        .world_slot
                        .with_world(|world| world.entity_by_persistent_id(&target_id).is_some())
                        .unwrap_or(false);
                    if !target_exists {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_DAMAGE_TARGET_MISSING",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' requested damage for unknown entity '{target_id}'"
                            ),
                        ));
                        continue;
                    }
                    if self.scripting.pending_damage_requests.len()
                        >= engine_script::MAX_PENDING_DAMAGE_REQUESTS
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_DAMAGE_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending damage budget of {} per frame",
                                engine_script::MAX_PENDING_DAMAGE_REQUESTS
                            ),
                        ));
                        continue;
                    }
                    self.scripting.pending_damage_requests.push(
                        engine_script::OwnedGameplayDamageRequest {
                            owner_entity_id: entity_id,
                            target_entity_id: target_id,
                            amount,
                            damage_kind,
                            hit_position,
                            impulse,
                        },
                    );
                }
                GameplayCommand::SetRagdoll {
                    entity_id: target_id,
                    active,
                    recovery_duration,
                    impulse,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "change ragdoll ownership",
                        ));
                        continue;
                    }
                    let command = GameplayCommand::SetRagdoll {
                        entity_id: target_id.clone(),
                        active,
                        recovery_duration,
                        impulse,
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_RAGDOLL_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid ragdoll request: {reason}"
                            ),
                        ));
                        continue;
                    }
                    let target_exists = self
                        .world_slot
                        .with_world(|world| world.entity_by_persistent_id(&target_id).is_some())
                        .unwrap_or(false);
                    if !target_exists {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_RAGDOLL_TARGET_MISSING",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' requested ragdoll ownership for unknown entity '{target_id}'"
                            ),
                        ));
                        continue;
                    }
                    if self.scripting.pending_ragdoll_requests.len()
                        >= engine_script::MAX_PENDING_RAGDOLL_REQUESTS
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_RAGDOLL_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending ragdoll budget of {} per frame",
                                engine_script::MAX_PENDING_RAGDOLL_REQUESTS
                            ),
                        ));
                        continue;
                    }
                    self.scripting.pending_ragdoll_requests.push(
                        engine_script::OwnedGameplayRagdollRequest {
                            owner_entity_id: entity_id,
                            target_entity_id: target_id,
                            active,
                            recovery_duration,
                            impulse,
                        },
                    );
                }
                GameplayCommand::CharacterControl {
                    entity_id: target_id,
                    direction,
                    jump,
                    speed,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "control a character",
                        ));
                        continue;
                    }
                    let command = GameplayCommand::CharacterControl {
                        entity_id: target_id.clone(),
                        direction,
                        jump,
                        speed,
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_CHARACTER_CONTROL_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced invalid character control: {reason}"
                            ),
                        ));
                        continue;
                    }
                    let applied = self
                        .world_slot
                        .with_world_mut(|world| {
                            let Some(target) = world.entity_by_persistent_id(&target_id) else {
                                return false;
                            };
                            let Some(controller) =
                                world.get_mut::<engine_character::CharacterController>(target)
                            else {
                                return false;
                            };
                            controller.push_command(engine_character::CharacterCommand {
                                direction: glam::Vec3::from(direction),
                                desired_speed: speed.unwrap_or(0.0),
                                jump_requested: jump,
                            });
                            true
                        })
                        .unwrap_or(false);
                    if !applied {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_CHARACTER_CONTROL_TARGET_MISSING",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' requested character control for '{target_id}', but it has no CharacterController"
                            ),
                        ));
                    }
                }
                GameplayCommand::Ui { command } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "mutate runtime UI",
                        ));
                        continue;
                    }
                    #[cfg(feature = "subsystem-ui")]
                    apply_script_ui_command(
                        &self.world_slot,
                        &entity_id,
                        command,
                        &mut diagnostics,
                    );
                    #[cfg(not(feature = "subsystem-ui"))]
                    {
                        let _ = command;
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_UI_UNAVAILABLE",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested runtime UI, but engine-core was built without subsystem-ui"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                    }
                }
                GameplayCommand::PhysicsQuery { query } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "request a physics query",
                        ));
                        continue;
                    }
                    if let Err(reason) = query.validate() {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_QUERY_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' produced an invalid physics query: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    // `GameplayPhysicsQuery::validate` cannot know the world;
                    // reject exclusion targets that name no existing entity.
                    if let Some(excluded) = query
                        .filter()
                        .and_then(|filter| filter.exclude_entity.as_deref())
                    {
                        let excluded_exists = self
                            .world_slot
                            .with_world(|world| world.entity_by_persistent_id(excluded).is_some())
                            .unwrap_or(false);
                        if !excluded_exists {
                            let mut diagnostic = Diagnostic::new(
                                "SCRIPT_PHYSICS_QUERY_INVALID",
                                DiagnosticSeverity::Error,
                                "script",
                                format!(
                                    "script entity '{entity_id}' produced an invalid physics query: unknown exclude_entity id '{excluded}'"
                                ),
                            );
                            diagnostic.entity = Some(entity_id);
                            diagnostics.push(diagnostic);
                            continue;
                        }
                    }
                    if self.scripting.pending_physics_queries.len()
                        >= engine_script::MAX_PENDING_PHYSICS_QUERIES
                    {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_QUERY_OVERFLOW",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' exceeded the pending physics query budget of {} per frame",
                                engine_script::MAX_PENDING_PHYSICS_QUERIES
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    self.scripting
                        .pending_physics_queries
                        .push(engine_script::OwnedGameplayPhysicsQuery { entity_id, query });
                }
                GameplayCommand::PhysicsMutation { mutation } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "mutate a rigid body",
                        ));
                        continue;
                    }
                    if let Err(reason) = mutation.validate() {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_MUTATION_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' produced an invalid physics mutation: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    let missing_target = self
                        .world_slot
                        .with_world(|world| {
                            mutation
                                .required_existing_entity_ids()
                                .into_iter()
                                .find(|target_id| {
                                    world.entity_by_persistent_id(target_id).is_none()
                                })
                                .map(str::to_owned)
                        })
                        .flatten();
                    if let Some(target_id) = missing_target {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_MUTATION_TARGET_MISSING",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested a physics mutation for unknown entity '{target_id}'"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    if self.scripting.pending_physics_mutations.len()
                        >= engine_script::MAX_PENDING_PHYSICS_MUTATIONS
                    {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_MUTATION_OVERFLOW",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' exceeded the pending physics mutation budget of {} per frame",
                                engine_script::MAX_PENDING_PHYSICS_MUTATIONS
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    self.scripting.pending_physics_mutations.push(
                        engine_script::OwnedGameplayPhysicsMutation {
                            owner_entity_id: entity_id,
                            mutation,
                        },
                    );
                }
                GameplayCommand::ComponentQuery { query } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "query a component",
                        ));
                        continue;
                    }
                    if let Err(reason) = query.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_COMPONENT_QUERY_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid component query: {reason}"
                            ),
                        ));
                        continue;
                    }
                    // Registry-driven access check. `None` means no World is
                    // active; the query is still queued and the executor
                    // reports SCRIPT_WORLD_MISSING instead.
                    let resolution = self.world_slot.with_world(|world| {
                        (
                            script_components::resolve_script_component(
                                world,
                                &query.component_type,
                            ),
                            script_components::supported_script_component_types(world),
                        )
                    });
                    if let Some((
                        script_components::ScriptComponentResolution::Unsupported,
                        supported,
                    )) = resolution
                    {
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
                    if self.scripting.pending_component_queries.len()
                        >= engine_script::MAX_PENDING_COMPONENT_QUERIES
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_COMPONENT_QUERY_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending component query budget of {} per frame",
                                engine_script::MAX_PENDING_COMPONENT_QUERIES
                            ),
                        ));
                        continue;
                    }
                    self.scripting
                        .pending_component_queries
                        .push(engine_script::OwnedGameplayComponentQuery { entity_id, query });
                }
                GameplayCommand::SetComponent {
                    entity_id: target_id,
                    component_type,
                    fields,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "write a component",
                        ));
                        continue;
                    }
                    let wire_validation = engine_script::validate_entity_id(&target_id)
                        .and_then(|_| engine_script::validate_component_type_key(&component_type))
                        .and_then(|_| engine_script::validate_component_fields(&fields));
                    if let Err(reason) = wire_validation {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_COMPONENT_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid set_component for entity '{target_id}': {reason}"
                            ),
                        ));
                        continue;
                    }
                    let resolution = self.world_slot.with_world(|world| {
                        (
                            script_components::resolve_script_component(world, &component_type),
                            script_components::supported_script_component_types(world),
                        )
                    });
                    match resolution {
                        Some((script_components::ScriptComponentResolution::ReadWrite, _)) => {}
                        Some((script_components::ScriptComponentResolution::ReadOnly, _)) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_READ_ONLY",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' tried to write component '{component_type}' on entity '{target_id}', but that component is read-only for scripts; query it with Components.Query instead"
                                ),
                            ));
                            continue;
                        }
                        Some((
                            script_components::ScriptComponentResolution::Unsupported,
                            supported,
                        )) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_UNKNOWN",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' tried to write component '{component_type}' on entity '{target_id}', but that type is not script-accessible; {}",
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
                                    "script entity '{entity_id}' component write for entity '{target_id}' could not be applied because no World is active"
                                ),
                            ));
                            continue;
                        }
                    }
                    let outcome = self.world_slot.with_world_mut(|world| {
                        script_components::apply_script_component_write(
                            world,
                            &target_id,
                            &component_type,
                            &fields,
                        )
                    });
                    match outcome {
                        Some(Ok(())) => {}
                        Some(Err(script_components::ScriptComponentWriteError::UnknownEntity)) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_TARGET_MISSING",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' wrote component '{component_type}' for entity '{target_id}', but that entity does not exist"
                                ),
                            ));
                        }
                        Some(Err(script_components::ScriptComponentWriteError::ReadOnly)) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_READ_ONLY",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' tried to write component '{component_type}' on entity '{target_id}', but that component is read-only for scripts; query it with Components.Query instead"
                                ),
                            ));
                        }
                        Some(Err(
                            script_components::ScriptComponentWriteError::PayloadRejected {
                                rejected,
                                known,
                            },
                        )) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_PAYLOAD_INVALID",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' wrote component '{component_type}' on entity '{target_id}' with fields the component rejected: {}; known fields: {}",
                                    rejected.join(", "),
                                    known.join(", ")
                                ),
                            ));
                        }
                        Some(Err(
                            script_components::ScriptComponentWriteError::ValidationFailed {
                                message,
                            },
                        )) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_VALIDATION_FAILED",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' wrote invalid component '{component_type}' parameters on entity '{target_id}': {message}"
                                ),
                            ));
                        }
                        Some(Err(script_components::ScriptComponentWriteError::Unsupported)) => {
                            let supported = self
                                .world_slot
                                .with_world(script_components::supported_script_component_types)
                                .unwrap_or_default();
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_UNKNOWN",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' tried to write component '{component_type}' on entity '{target_id}', but that type is not script-accessible; {}",
                                    supported_script_component_description(&supported)
                                ),
                            ));
                        }
                        Some(Err(script_components::ScriptComponentWriteError::ApplyFailed)) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_APPLY_FAILED",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' wrote component '{component_type}' on entity '{target_id}', but the validated component could not be committed to storage"
                                ),
                            ));
                        }
                        None => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_WORLD_MISSING",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' component write for entity '{target_id}' could not be applied because no World is active"
                                ),
                            ));
                        }
                    }
                }
                command @ (GameplayCommand::PlayAnimation { .. }
                | GameplayCommand::SetAnimationParameter { .. }
                | GameplayCommand::TransitionAnimationState { .. }
                | GameplayCommand::SetAnimationPlaying { .. }
                | GameplayCommand::SetMorphWeights { .. }
                | GameplayCommand::SaveCheckpoint { .. }
                | GameplayCommand::LoadCheckpoint { .. }
                | GameplayCommand::QueryLogicAsset { .. }) => {
                    self.apply_script_extended_command(entity_id, command, &mut diagnostics);
                }
            }
        }

        if let Some(scene_request) = scene_request {
            if let Some(existing) = &self.scripting.pending_scene_request {
                if existing != &scene_request {
                    let mut diagnostic = Diagnostic::new(
                        "SCRIPT_SCENE_REQUEST_CONFLICT",
                        DiagnosticSeverity::Error,
                        "script",
                        format!(
                            "script entity '{}' requested scene '{}' while '{}' already has a pending request for '{}'; the first request wins",
                            scene_request.requested_by,
                            scene_request.scene_id,
                            existing.requested_by,
                            existing.scene_id,
                        ),
                    );
                    diagnostic.entity = Some(scene_request.requested_by);
                    diagnostics.push(diagnostic);
                }
            } else {
                self.scripting.pending_scene_request = Some(scene_request);
            }
        }
        diagnostics
    }
}
