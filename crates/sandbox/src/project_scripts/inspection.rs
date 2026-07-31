use super::*;

/// Validate the source/runtime script pairing and every scene attachment.
pub(crate) fn inspect_project_scripts(
    project: &GameProject,
    scene: &Scene,
) -> Result<ScriptProjectInspection, String> {
    let configured = match (&project.script_project, &project.script_assembly) {
        (None, None) => None,
        (Some(_), Some(assembly)) => Some(assembly_id_from_path(assembly)?),
        (Some(_), None) => return Err(
            "script_project is configured but script_assembly is missing from game.project.json"
                .into(),
        ),
        (None, Some(_)) => {
            return Err("script_assembly is configured without an authoring script_project".into());
        }
    };

    let mut component_count = 0usize;
    for entity in &scene.entities {
        let Some(component) = entity.components.get(SCRIPT_COMPONENT_TYPE) else {
            continue;
        };
        component_count += 1;
        let assembly_id = match component.fields.get("assembly_id") {
            Some(Value::Str(value)) if !value.trim().is_empty() => value,
            _ => {
                return Err(format!(
                    "entity '{}' has an engine.script component without a non-empty string assembly_id",
                    entity.persistent_id
                ));
            }
        };
        let class_name = match component.fields.get("class_name") {
            Some(Value::Str(value)) if !value.trim().is_empty() => value,
            _ => {
                return Err(format!(
                    "entity '{}' has an engine.script component without a non-empty string class_name",
                    entity.persistent_id
                ));
            }
        };
        let Some(expected) = configured.as_deref() else {
            return Err(format!(
                "entity '{}' attaches script '{}', but the project has no script_project/script_assembly configuration",
                entity.persistent_id, class_name
            ));
        };
        if assembly_id != expected {
            return Err(format!(
                "entity '{}' references script assembly '{}'; expected '{}' from script_assembly",
                entity.persistent_id, assembly_id, expected
            ));
        }
    }

    Ok(ScriptProjectInspection {
        assembly_id: configured,
        component_count,
    })
}

/// Validate runtime scene attachments against the compiled DLL. Packaged
/// projects are allowed to omit the authoring-only `script_project`.
pub(crate) fn validate_runtime_script_references(
    project: &GameProject,
    scene: &Scene,
) -> Result<usize, String> {
    let expected = project
        .script_assembly
        .as_deref()
        .map(assembly_id_from_path)
        .transpose()?;
    let mut count = 0usize;
    for entity in &scene.entities {
        let Some(component) = entity.components.get(SCRIPT_COMPONENT_TYPE) else {
            continue;
        };
        count += 1;
        let assembly_id = match component.fields.get("assembly_id") {
            Some(Value::Str(value)) if !value.trim().is_empty() => value,
            _ => {
                return Err(format!(
                    "entity '{}' has an invalid engine.script assembly_id",
                    entity.persistent_id
                ));
            }
        };
        match component.fields.get("class_name") {
            Some(Value::Str(value)) if !value.trim().is_empty() => {}
            _ => {
                return Err(format!(
                    "entity '{}' has an invalid engine.script class_name",
                    entity.persistent_id
                ));
            }
        }
        let Some(expected) = expected.as_deref() else {
            return Err(format!(
                "entity '{}' contains engine.script but script_assembly is not configured",
                entity.persistent_id
            ));
        };
        if assembly_id != expected {
            return Err(format!(
                "entity '{}' references script assembly '{}'; expected '{}'",
                entity.persistent_id, assembly_id, expected
            ));
        }
    }
    Ok(count)
}
