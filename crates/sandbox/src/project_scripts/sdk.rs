use super::*;

pub(crate) fn script_api_sha256() -> String {
    let mut digest = Sha256::new();
    for (name, source) in [
        (
            engine_script_api::GENERATED_CSHARP_API_FILE,
            STARTER_SCRIPT_API_SOURCE,
        ),
        (
            engine_script_api::GENERATED_CSHARP_RULES_FILE,
            RULES_SCRIPT_API_SOURCE,
        ),
        (
            engine_script_api::GENERATED_CSHARP_TACTICS_FILE,
            TACTICS_SCRIPT_API_SOURCE,
        ),
        (
            engine_script_api::GENERATED_CSHARP_JRPG_FILE,
            JRPG_SCRIPT_API_SOURCE,
        ),
        (
            engine_script_api::GENERATED_CSHARP_RENDERING_FILE,
            RENDERING_SCRIPT_API_SOURCE,
        ),
        (
            engine_script_api::GENERATED_CSHARP_RUNTIME_ASSETS_FILE,
            RUNTIME_ASSETS_SCRIPT_API_SOURCE,
        ),
        (
            engine_script_api::GENERATED_CSHARP_ONLINE_XR_FILE,
            ONLINE_XR_SCRIPT_API_SOURCE,
        ),
    ] {
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(source.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

pub(crate) fn generated_script_api_manifest_json() -> Result<String, String> {
    let mut json = serde_json::to_string_pretty(&GeneratedScriptApiManifest {
        schema: "EngineGameplaySdkContract-v1",
        script_api: engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA,
        version: engine_script_api::GAMEPLAY_SCRIPT_API_VERSION,
        owner: "engine",
        managed_sdk_assembly: engine_script_api::MANAGED_SDK_ASSEMBLY_NAME,
        generated_sources: [
            engine_script_api::GENERATED_CSHARP_API_FILE,
            engine_script_api::GENERATED_CSHARP_RULES_FILE,
            engine_script_api::GENERATED_CSHARP_TACTICS_FILE,
            engine_script_api::GENERATED_CSHARP_JRPG_FILE,
            engine_script_api::GENERATED_CSHARP_RENDERING_FILE,
            engine_script_api::GENERATED_CSHARP_RUNTIME_ASSETS_FILE,
            engine_script_api::GENERATED_CSHARP_ONLINE_XR_FILE,
        ],
        msbuild_targets: engine_script_api::GENERATED_MSBUILD_TARGETS_FILE,
        sha256: script_api_sha256(),
    })
    .map_err(|error| format!("could not serialize gameplay script API contract: {error}"))?;
    json.push('\n');
    Ok(json)
}

pub(crate) fn materialize_script_sdk_source(
    project_root: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    let sdk_source_dir = project_root.join("build/script-sdk-source");
    ensure_inside_project(project_root, &sdk_source_dir, "script SDK source")?;
    std::fs::create_dir_all(&sdk_source_dir).map_err(|error| {
        format!(
            "could not create script SDK source directory {}: {error}",
            sdk_source_dir.display()
        )
    })?;
    let source = sdk_source_dir.join(engine_script_api::GENERATED_CSHARP_API_FILE);
    let rules_source = sdk_source_dir.join(engine_script_api::GENERATED_CSHARP_RULES_FILE);
    let tactics_source = sdk_source_dir.join(engine_script_api::GENERATED_CSHARP_TACTICS_FILE);
    let jrpg_source = sdk_source_dir.join(engine_script_api::GENERATED_CSHARP_JRPG_FILE);
    let rendering_source = sdk_source_dir.join(engine_script_api::GENERATED_CSHARP_RENDERING_FILE);
    let runtime_assets_source =
        sdk_source_dir.join(engine_script_api::GENERATED_CSHARP_RUNTIME_ASSETS_FILE);
    let online_xr_source = sdk_source_dir.join(engine_script_api::GENERATED_CSHARP_ONLINE_XR_FILE);
    let project = sdk_source_dir.join("EngineGameplay.csproj");
    write_file(&source, STARTER_SCRIPT_API_SOURCE)?;
    write_file(&rules_source, RULES_SCRIPT_API_SOURCE)?;
    write_file(&tactics_source, TACTICS_SCRIPT_API_SOURCE)?;
    write_file(&jrpg_source, JRPG_SCRIPT_API_SOURCE)?;
    write_file(&rendering_source, RENDERING_SCRIPT_API_SOURCE)?;
    write_file(&runtime_assets_source, RUNTIME_ASSETS_SCRIPT_API_SOURCE)?;
    write_file(&online_xr_source, ONLINE_XR_SCRIPT_API_SOURCE)?;
    write_file(&project, SCRIPT_SDK_PROJECT)?;
    Ok((source, project))
}

pub(crate) fn ensure_script_sdk_import(script_project: &Path) -> Result<(), String> {
    const IMPORT: &str = "  <Import Project=\"EngineGameplay.targets\" />";
    let contents = std::fs::read_to_string(script_project).map_err(|error| {
        format!(
            "could not read game script project {}: {error}",
            script_project.display()
        )
    })?;
    if contents.contains(IMPORT.trim()) {
        return Ok(());
    }
    let closing = contents.rfind("</Project>").ok_or_else(|| {
        format!(
            "cannot migrate game script project without a closing </Project>: {}",
            script_project.display()
        )
    })?;
    let mut migrated = String::with_capacity(contents.len() + IMPORT.len() + 2);
    migrated.push_str(&contents[..closing]);
    if !migrated.ends_with('\n') {
        migrated.push('\n');
    }
    migrated.push_str(IMPORT);
    migrated.push('\n');
    migrated.push_str(&contents[closing..]);
    write_file(script_project, &migrated)
}

/// Write the engine-owned Script SDK integration and deterministic manifest.
///
/// Game projects own their explicitly created gameplay sources. The managed
/// API implementation stays outside the game source directory and is
/// referenced through an engine-owned MSBuild target. Installed engines deploy
/// a prebuilt `EngineGameplay.dll`; source-tree development materializes the
/// SDK source only when it is explicitly built.
pub(crate) fn write_generated_script_api(
    project_root: &Path,
    script_project: &Path,
) -> Result<PathBuf, String> {
    let source_dir = script_project.parent().ok_or_else(|| {
        format!(
            "script_project has no source directory: {}",
            script_project.display()
        )
    })?;
    ensure_inside_project(project_root, source_dir, "script_project source")?;
    std::fs::create_dir_all(source_dir).map_err(|error| {
        format!(
            "could not create script source directory {}: {error}",
            source_dir.display()
        )
    })?;
    ensure_script_sdk_import(script_project)?;

    let contract = source_dir.join(engine_script_api::GENERATED_CONTRACT_FILE);
    let msbuild_targets = source_dir.join(engine_script_api::GENERATED_MSBUILD_TARGETS_FILE);
    for generated_file in [
        engine_script_api::GENERATED_CSHARP_API_FILE,
        engine_script_api::GENERATED_CSHARP_RULES_FILE,
        engine_script_api::GENERATED_CSHARP_TACTICS_FILE,
        engine_script_api::GENERATED_CSHARP_JRPG_FILE,
        engine_script_api::GENERATED_CSHARP_RENDERING_FILE,
        engine_script_api::GENERATED_CSHARP_RUNTIME_ASSETS_FILE,
        engine_script_api::GENERATED_CSHARP_ONLINE_XR_FILE,
    ] {
        let legacy_source = source_dir.join(generated_file);
        if legacy_source.is_file() {
            std::fs::remove_file(&legacy_source).map_err(|error| {
                format!(
                    "could not remove legacy generated Script API {}: {error}",
                    legacy_source.display()
                )
            })?;
        }
    }
    write_file(&msbuild_targets, SCRIPT_SDK_TARGETS)?;
    write_file(&contract, &generated_script_api_manifest_json()?)?;
    Ok(contract)
}

pub(crate) fn sync_project_script_api(
    project: &GameProject,
) -> Result<ScriptApiSyncReport, String> {
    let script_project = project.script_project.as_deref().ok_or_else(|| {
        "project has no authoring script_project; create it with --with-csharp".to_string()
    })?;
    if project.script_assembly.is_none() {
        return Err(
            "script_project is configured but script_assembly is missing from game.project.json"
                .into(),
        );
    }
    let contract = write_generated_script_api(&project.root, script_project)?;
    let script_api_hash = validate_generated_script_api(&project.root, script_project)?;
    let installation =
        crate::engine_installation::EngineInstallation::discover_from_current_executable()?;
    let source = if let Some(installation) = installation.as_ref() {
        installation.validate_managed_sdk_contract(
            engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA,
            engine_script_api::GAMEPLAY_SCRIPT_API_VERSION,
            &script_api_hash,
        )?;
        deploy_installed_managed_tools(project, installation)?;
        None
    } else {
        crate::engine_installation::development_source_root()?;
        let (source, _) = materialize_script_sdk_source(&project.root)?;
        Some(report_path(&source))
    };
    let source_dir = script_project
        .parent()
        .ok_or_else(|| "script_project has no source directory".to_string())?;
    Ok(ScriptApiSyncReport {
        schema: "ProjectScriptApiSyncReport-v1",
        project: project.manifest.name.clone(),
        script_api: engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA,
        version: engine_script_api::GAMEPLAY_SCRIPT_API_VERSION,
        source,
        contract: report_path(&contract),
        msbuild_targets: report_path(
            &source_dir.join(engine_script_api::GENERATED_MSBUILD_TARGETS_FILE),
        ),
        sdk_assembly: report_path(&project.root.join("build/script-sdk").join(format!(
            "{}.dll",
            engine_script_api::MANAGED_SDK_ASSEMBLY_NAME
        ))),
        sha256: script_api_sha256(),
        passed: true,
    })
}

pub(crate) fn validate_generated_script_api(
    project_root: &Path,
    script_project: &Path,
) -> Result<String, String> {
    let source_dir = script_project.parent().ok_or_else(|| {
        format!(
            "script_project has no source directory: {}",
            script_project.display()
        )
    })?;
    let contract = source_dir.join(engine_script_api::GENERATED_CONTRACT_FILE);
    let msbuild_targets = source_dir.join(engine_script_api::GENERATED_MSBUILD_TARGETS_FILE);
    let has_legacy_source = [
        engine_script_api::GENERATED_CSHARP_API_FILE,
        engine_script_api::GENERATED_CSHARP_RULES_FILE,
        engine_script_api::GENERATED_CSHARP_TACTICS_FILE,
        engine_script_api::GENERATED_CSHARP_JRPG_FILE,
        engine_script_api::GENERATED_CSHARP_RENDERING_FILE,
        engine_script_api::GENERATED_CSHARP_RUNTIME_ASSETS_FILE,
        engine_script_api::GENERATED_CSHARP_ONLINE_XR_FILE,
    ]
    .into_iter()
    .any(|generated_file| source_dir.join(generated_file).exists());
    let expected_manifest = generated_script_api_manifest_json()?;
    let project_import_is_current = std::fs::read_to_string(script_project)
        .is_ok_and(|contents| contents.contains("<Import Project=\"EngineGameplay.targets\" />"));
    if has_legacy_source
        || !project_import_is_current
        || !file_contents_equal(&contract, &expected_manifest)
        || !file_contents_equal(&msbuild_targets, SCRIPT_SDK_TARGETS)
    {
        return Err(format!(
            "the engine-owned gameplay SDK integration beside {} is missing or stale; run `sandbox project sync-script-api {}` before building game scripts",
            script_project.display(),
            project_root.display()
        ));
    }
    Ok(script_api_sha256())
}

/// Deploy the immutable managed SDK and process host carried by an installed
/// editor. This operation never evaluates project code and is therefore safe
/// to run while opening a workspace.
///
/// Source-tree development returns `Ok(false)` and keeps its explicit
/// build-from-source path. An invalid explicit installation fails closed.
pub(crate) fn deploy_installed_project_script_runtime(
    project: &GameProject,
) -> Result<bool, String> {
    if project.script_project.is_none() && project.script_assembly.is_none() {
        return Ok(false);
    }
    let Some(installation) =
        crate::engine_installation::EngineInstallation::discover_from_current_executable()?
    else {
        return Ok(false);
    };
    let script_project = project
        .script_project
        .as_deref()
        .ok_or_else(|| "installed script runtime deployment requires script_project".to_string())?;
    let script_api_hash = validate_generated_script_api(&project.root, script_project)?;
    installation.validate_managed_sdk_contract(
        engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA,
        engine_script_api::GAMEPLAY_SCRIPT_API_VERSION,
        &script_api_hash,
    )?;
    deploy_installed_managed_tools(project, &installation)?;
    Ok(true)
}

pub(crate) fn deploy_installed_managed_tools(
    project: &GameProject,
    installation: &crate::engine_installation::EngineInstallation,
) -> Result<(), String> {
    deploy_installed_managed_tools_from(
        project,
        &installation.managed_sdk,
        &installation.script_host,
        self_test_script_host,
    )
}

pub(crate) fn deploy_installed_managed_tools_from<F>(
    project: &GameProject,
    installed_sdk: &Path,
    installed_host: &Path,
    validate_host: F,
) -> Result<(), String>
where
    F: Fn(&Path, &Path) -> Result<(), String>,
{
    let sdk_output_dir = project.root.join("build/script-sdk");
    ensure_inside_project(&project.root, &sdk_output_dir, "script SDK output")?;
    let sdk_output_next = sibling_with_suffix(&sdk_output_dir, ".next")?;
    let installed_sdk_name = format!("{}.dll", engine_script_api::MANAGED_SDK_ASSEMBLY_NAME);
    if !directory_contains_only_regular_file_equal(
        &sdk_output_dir,
        &installed_sdk_name,
        installed_sdk,
    )? {
        reset_owned_directory(&project.root, &sdk_output_next)?;
        copy_installed_file(
            installed_sdk,
            &sdk_output_next.join(&installed_sdk_name),
            "managed gameplay SDK",
        )?;
        replace_owned_directory(&project.root, &sdk_output_next, &sdk_output_dir)?;
    }

    let host_dir = project.root.join("build/script-host");
    let host_executable = host_dir.join(host_executable_name());
    if directory_files_equal(installed_host, &host_dir)? {
        validate_host(&host_executable, &host_dir)?;
        return Ok(());
    }

    let host_next = project.root.join("build/script-host.next");
    reset_owned_directory(&project.root, &host_next)?;
    copy_installed_directory_files(installed_host, &host_next, "managed script host")?;
    let next_host_executable = host_next.join(host_executable_name());
    if !next_host_executable.is_file() {
        return Err(format!(
            "installed managed script host does not contain {}: {}",
            host_executable_name(),
            installed_host.display()
        ));
    }
    validate_host(&next_host_executable, &host_next)?;
    replace_owned_directory(&project.root, &host_next, &host_dir)
}
