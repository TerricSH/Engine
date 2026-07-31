use crate::*;

/// Maximum synchronous `Scene.Spawn` → `OnCreate` recursion depth per frame
/// boundary. A script whose `OnCreate` spawns another scripted prefab cannot
/// recurse without bound inside one command drain.
#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) const MAX_SCRIPT_SPAWN_DEPTH: usize = 8;

/// Assign deterministic persistent IDs to a freshly instantiated prefab.
///
/// The root entity receives the first free id from `<prefabId>`,
/// `<prefabId>-2`, `<prefabId>-3`, …; every other spawned entity receives
/// `<rootId>.<prefab-local id>` with the same `-N` conflict suffix. The
/// result pairs each entity with its assigned id in spawn order, root first.
#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn assign_spawned_persistent_ids(
    world: &mut World,
    prefab_id: &str,
    result: &engine_scene::PrefabInstantiateResult,
) -> Result<Vec<(engine_scene::Entity, String)>, String> {
    let mut assigned: Vec<(engine_scene::Entity, String)> =
        Vec::with_capacity(result.all_entities.len());
    for entity in &result.all_entities {
        let base = if *entity == result.root_entity {
            prefab_id.to_string()
        } else {
            let local_id = world
                .get::<engine_scene::PrefabInstanceRef>(*entity)
                .map(|reference| reference.entity_persistent_id.clone())
                .unwrap_or_else(|| format!("entity-{}", entity.index()));
            let root_id = &assigned
                .first()
                .expect("the root entity is always assigned first")
                .1;
            format!("{root_id}.{local_id}")
        };
        let candidate = first_free_persistent_id(world, &base).ok_or_else(|| {
            format!("could not allocate a unique persistent entity id below '{base}'")
        })?;
        engine_script::validate_entity_id(&candidate).map_err(|reason| {
            format!("prefab '{prefab_id}' produced an unusable spawned entity id: {reason}")
        })?;
        world
            .assign_persistent_id(*entity, candidate.clone())
            .map_err(|error| error.to_string())?;
        assigned.push((*entity, candidate));
    }
    Ok(assigned)
}

/// First unused persistent id from `base`, `base-2`, `base-3`, … within the
/// 128-byte persistent-id budget.
#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn first_free_persistent_id(world: &World, base: &str) -> Option<String> {
    if world.entity_by_persistent_id(base).is_none() {
        return Some(base.to_string());
    }
    for suffix in 2_u64.. {
        let candidate = format!("{base}-{suffix}");
        if candidate.len() > 128 {
            return None;
        }
        if world.entity_by_persistent_id(&candidate).is_none() {
            return Some(candidate);
        }
    }
    unreachable!("the u64 suffix space cannot be exhausted in memory")
}

/// Apply the optional `Scene.Spawn` translation override to the spawned root.
/// A prefab root without a Transform gains one so the spawn position is never
/// silently dropped; rotation and scale from the prefab are preserved.
#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn apply_spawn_translation(
    world: &mut World,
    root: engine_scene::Entity,
    translation: [f32; 3],
) {
    let translation = glam::Vec3::from_array(translation);
    if let Some(transform) = world.get_mut::<engine_scene::components::Transform>(root) {
        transform.translation = translation;
    } else {
        world.add_component(
            root,
            engine_scene::components::Transform {
                translation,
                ..Default::default()
            },
        );
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn validate_script_transform(transform: &ScriptTransform) -> Result<(), &'static str> {
    engine_script::validate_script_transform(transform).map_err(|reason| match reason.as_str() {
        "translation, rotation, and scale must contain only finite values" => {
            "translation, rotation, and scale must contain only finite values"
        }
        "rotation quaternion must not be zero length" => {
            "rotation quaternion must not be zero length"
        }
        _ => "Transform is invalid",
    })
}

#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn script_command_owner_exists(world_slot: &WorldSlot, entity_id: &str) -> bool {
    world_slot
        .with_world(|world| world.entity_by_persistent_id(entity_id).is_some())
        .unwrap_or(false)
}

#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn script_owner_missing_diagnostic(entity_id: &str, action: &str) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        "SCRIPT_COMMAND_OWNER_MISSING",
        DiagnosticSeverity::Error,
        "script",
        format!("script entity '{entity_id}' no longer exists and cannot {action}"),
    );
    diagnostic.entity = Some(entity_id.to_owned());
    diagnostic
}

/// Build an entity-scoped script diagnostic for the typed component bridge.
#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn script_component_diagnostic(
    code: &str,
    entity_id: &str,
    message: String,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(code, DiagnosticSeverity::Error, "script", message);
    diagnostic.entity = Some(entity_id.to_owned());
    diagnostic
}

/// Human-readable list of the component type keys scripts may access, used
/// by `SCRIPT_COMPONENT_UNKNOWN` diagnostics.
#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn supported_script_component_description(supported: &[&'static str]) -> String {
    if supported.is_empty() {
        return "no component types are script-accessible in this build".to_string();
    }
    format!("supported component types: {}", supported.join(", "))
}

#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn apply_script_transform_command(
    world_slot: &WorldSlot,
    requested_by: &str,
    target_id: &str,
    transform: ScriptTransform,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Err(reason) = validate_script_transform(&transform) {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_TRANSFORM_INVALID",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' produced an invalid Transform for entity '{target_id}': {reason}"
            ),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
        return;
    }
    let applied = world_slot
        .with_world_mut(|world| {
            let entity = world.entity_by_persistent_id(target_id)?;
            let current = world.get_mut::<engine_scene::components::Transform>(entity)?;
            current.translation = glam::Vec3::from_array(transform.translation);
            // Managed callers may construct a finite but non-unit quaternion.
            current.rotation = glam::Quat::from_array(transform.rotation).normalize();
            current.scale = glam::Vec3::from_array(transform.scale);
            Some(())
        })
        .flatten()
        .is_some();
    if !applied {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_TRANSFORM_TARGET_MISSING",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' targeted entity '{target_id}', which no longer exists or has no Transform"
            ),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn create_script_entity(
    world_slot: &WorldSlot,
    requested_by: &str,
    target_id: &str,
    transform: ScriptTransform,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Err(reason) = validate_script_transform(&transform) {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_ENTITY_CREATE_TRANSFORM_INVALID",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' produced an invalid Transform while creating entity '{target_id}': {reason}"
            ),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
        return;
    }

    let created = world_slot.with_world_mut(|world| {
        let entity = world.create_persistent_entity(target_id.to_owned())?;
        world.add_component(
            entity,
            engine_scene::components::Transform {
                translation: glam::Vec3::from_array(transform.translation),
                rotation: glam::Quat::from_array(transform.rotation).normalize(),
                scale: glam::Vec3::from_array(transform.scale),
                ..Default::default()
            },
        );
        Ok::<_, engine_scene::PersistentEntityCreateError>(())
    });

    match created {
        Some(Ok(())) => {}
        Some(Err(engine_scene::PersistentEntityCreateError::DuplicateId(_))) => {
            let mut diagnostic = Diagnostic::new(
                "SCRIPT_ENTITY_CREATE_CONFLICT",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "script entity '{requested_by}' could not create entity '{target_id}' because that persistent ID already exists; the first creation wins"
                ),
            );
            diagnostic.entity = Some(target_id.to_owned());
            diagnostics.push(diagnostic);
        }
        Some(Err(error)) => {
            let mut diagnostic = Diagnostic::new(
                "SCRIPT_ENTITY_CREATE_FAILED",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "script entity '{requested_by}' could not create entity '{target_id}': {error}"
                ),
            );
            diagnostic.entity = Some(target_id.to_owned());
            diagnostics.push(diagnostic);
        }
        None => diagnostics.push(Diagnostic::new(
            "SCRIPT_WORLD_MISSING",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' could not create entity '{target_id}' because no World is active"
            ),
        )),
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn destroy_script_entity(
    world_slot: &WorldSlot,
    script_engine: &mut ScriptEngine,
    requested_by: &str,
    target_id: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let target = world_slot
        .with_world(|world| world.entity_by_persistent_id(target_id))
        .flatten();
    let Some(target) = target else {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_DESTROY_TARGET_MISSING",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' tried to destroy entity '{target_id}', but that entity does not exist"
            ),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
        return;
    };

    // OnDestroy runs while the entity and World are still valid. The manager
    // removes the instances immediately afterwards, so the destroyed entity
    // cannot be ticked again on the next frame.
    diagnostics.extend(script_engine.destroy_entity_instances(target_id));
    let destroyed = world_slot
        .with_world_mut(|world| world.destroy_entity(target))
        .unwrap_or(false);
    if !destroyed {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_DESTROY_FAILED",
            DiagnosticSeverity::Error,
            "script",
            format!("entity '{target_id}' became stale before it could be destroyed"),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
    }
}
