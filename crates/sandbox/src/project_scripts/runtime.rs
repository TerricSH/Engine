use super::*;

#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn prepare_isolated_project_script_engine(
    project: &GameProject,
    script_assembly: &Path,
    host_path: &Path,
) -> Result<(engine_script::ScriptEngine, PreparedScriptRuntime), String> {
    use engine_script::{ProcessHost, ScriptEngine};

    if !script_assembly.is_file() {
        return Err(format!(
            "compiled script assembly is missing: {}; run `sandbox project build-scripts {}`",
            script_assembly.display(),
            project.root.display()
        ));
    }

    let mut host = ProcessHost::new(SCRIPT_HOST_NAME);
    host.launch(host_path)
        .map_err(|error| format!("could not start C# script host: {error}"))?;
    let mut candidate = ScriptEngine::new();
    candidate.register_host(Box::new(host));

    let mut loaded_ids = BTreeSet::new();
    let mut loaded = 0usize;
    let mut dependencies = managed_dependencies(
        script_assembly.parent().unwrap_or(&project.root),
        script_assembly,
    )?;
    let sdk_assembly_name = format!("{}.dll", engine_script_api::MANAGED_SDK_ASSEMBLY_NAME);
    if !dependencies.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(&sdk_assembly_name))
    }) {
        return Err(format!(
            "compiled game scripts are missing the engine SDK dependency {sdk_assembly_name}; run `sandbox project build-scripts {}`",
            project.root.display()
        ));
    }
    dependencies.sort_by_key(|path| {
        !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(&sdk_assembly_name))
    });
    for dependency in dependencies {
        let id = assembly_id_from_path(&dependency)?;
        if !loaded_ids.insert(id.clone()) {
            return Err(format!("duplicate managed assembly id '{id}'"));
        }
        let bytes = std::fs::read(&dependency).map_err(|error| {
            format!(
                "could not read script dependency {}: {error}",
                dependency.display()
            )
        })?;
        candidate
            .load_script(&id, SCRIPT_HOST_NAME, &bytes)
            .map_err(|error| {
                format!(
                    "could not load script dependency {}: {error}",
                    dependency.display()
                )
            })?;
        loaded += 1;
    }

    let assembly_id = assembly_id_from_path(script_assembly)?;
    if !loaded_ids.insert(assembly_id.clone()) {
        return Err(format!("duplicate game script assembly id '{assembly_id}'"));
    }
    let bytes = std::fs::read(script_assembly).map_err(|error| {
        format!(
            "could not read game script assembly {}: {error}",
            script_assembly.display()
        )
    })?;
    candidate
        .load_script(&assembly_id, SCRIPT_HOST_NAME, &bytes)
        .map_err(|error| format!("could not load game script assembly: {error}"))?;
    loaded += 1;

    Ok((candidate, PreparedScriptRuntime { assemblies: loaded }))
}

pub(crate) fn script_runtime_counts(runtime: &EngineRuntime) -> (usize, usize, usize) {
    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        let assemblies = runtime
            .script_engine()
            .managers()
            .iter()
            .map(|manager| manager.assembly_count())
            .sum();
        let instances = runtime
            .script_engine()
            .managers()
            .iter()
            .map(|manager| manager.instance_count())
            .sum();
        let started = runtime
            .script_engine()
            .managers()
            .iter()
            .flat_map(|manager| manager.iter_instances())
            .filter(|(_, _, state)| state.started)
            .count();
        (assemblies, instances, started)
    }
    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = runtime;
        (0, 0, 0)
    }
}

/// Sum an integer field across attached scripts. This is primarily useful for
/// headless smoke reports (the starter template exposes `UpdateCount`).
pub(crate) fn script_int_field_sum(runtime: &EngineRuntime, field: &str) -> Option<i64> {
    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        let values = runtime
            .script_engine()
            .managers()
            .iter()
            .flat_map(|manager| manager.iter_instances())
            .filter_map(|(_, _, state)| match state.instance.get_field(field) {
                Some(engine_script::ScriptValue::Int(value)) => Some(value),
                _ => None,
            })
            .collect::<Vec<_>>();
        if values.is_empty() {
            None
        } else {
            Some(values.into_iter().sum())
        }
    }
    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = (runtime, field);
        None
    }
}

/// Snapshot the ECS translations of entities that own attached scripts.
pub(crate) fn script_entity_translations(
    runtime: &EngineRuntime,
) -> std::collections::BTreeMap<String, [f32; 3]> {
    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        let scripted_entities = runtime
            .script_engine()
            .managers()
            .iter()
            .flat_map(|manager| manager.iter_instances().map(|(entity_id, _, _)| entity_id))
            .collect::<std::collections::BTreeSet<_>>();
        runtime
            .with_world(|world| {
                world
                    .query_all::<engine_scene::components::Transform>()
                    .filter_map(|(entity, transform)| {
                        let entity_id = world.persistent_id(entity)?;
                        scripted_entities
                            .contains(entity_id)
                            .then(|| (entity_id.to_owned(), transform.translation.to_array()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = runtime;
        std::collections::BTreeMap::new()
    }
}

pub(crate) fn fail_on_script_errors(runtime: &EngineRuntime, phase: &str) -> Result<(), String> {
    let errors = runtime
        .diagnostics_collector()
        .script_diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        })
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("C# script {phase} failed:\n{}", errors.join("\n")))
    }
}
