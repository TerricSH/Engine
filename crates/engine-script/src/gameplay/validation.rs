use super::snapshots::ScriptTransform;

/// Validate a persistent entity id received from untrusted script code.
///
/// Entity ids are identifiers, never paths. Keeping the wire-safe subset
/// deliberately small prevents traversal-like strings and control characters
/// from crossing into diagnostics, lookup tables, or future persistence APIs.
pub fn validate_entity_id(entity_id: &str) -> Result<(), String> {
    let valid = !entity_id.is_empty()
        && entity_id.len() <= 128
        && entity_id != "."
        && entity_id != ".."
        && entity_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid entity id {entity_id:?}: expected 1 to 128 ASCII letters, digits, hyphens, underscores, or dots (but not '.' or '..'); entity ids are not file paths"
        ))
    }
}

/// Validate a Transform received from an untrusted script host.
pub fn validate_script_transform(transform: &ScriptTransform) -> Result<(), String> {
    if !transform
        .translation
        .iter()
        .chain(transform.rotation.iter())
        .chain(transform.scale.iter())
        .all(|value| value.is_finite())
    {
        return Err("translation, rotation, and scale must contain only finite values".into());
    }
    let rotation_length_squared = transform
        .rotation
        .iter()
        .map(|value| value * value)
        .sum::<f32>();
    if rotation_length_squared <= f32::EPSILON {
        return Err("rotation quaternion must not be zero length".into());
    }
    Ok(())
}

/// Validate a project scene catalog identifier.
///
/// This intentionally mirrors the portable scene-id contract used by project
/// manifests without making the script protocol depend on `engine-asset`.
pub fn validate_scene_id(scene_id: &str) -> Result<(), String> {
    let valid = !scene_id.is_empty()
        && scene_id.len() <= 128
        && scene_id != "."
        && scene_id != ".."
        && scene_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid scene id {scene_id:?}: use a key from game.project.json `scenes` containing 1 to 128 ASCII letters, digits, hyphens, underscores, or dots (but not '.' or '..')"
        ))
    }
}

/// Validate a cooked prefab asset identifier used by `Scene.Spawn`.
///
/// The spawned instance root takes this id as the base of its persistent
/// entity id, so prefab ids share the wire-safe identifier contract of
/// persistent entity ids. Authors choose asset ids, so keeping the two
/// contracts aligned is always possible.
pub fn validate_prefab_id(prefab_id: &str) -> Result<(), String> {
    validate_entity_id(prefab_id).map_err(|_| {
        format!(
            "invalid prefab id {prefab_id:?}: expected a cooked prefab asset id containing 1 to 128 ASCII letters, digits, hyphens, underscores, or dots (but not '.' or '..'); prefab ids are not file paths"
        )
    })
}
