use super::*;

/// Build the project's game assembly and publish the engine script host.
///
/// Both outputs are produced in sibling `.next` directories and only replace
/// the last good outputs after the corresponding dotnet command succeeds.
pub(crate) fn build_project_scripts(
    project: &GameProject,
) -> Result<Option<ScriptBuildReport>, String> {
    let (script_project, script_assembly) = match (
        project.script_project.as_deref(),
        project.script_assembly.as_deref(),
    ) {
        (None, None) => return Ok(None),
        (Some(source), Some(assembly)) => (source, assembly),
        (Some(_), None) => return Err(
            "script_project is configured but script_assembly is missing from game.project.json"
                .into(),
        ),
        (None, Some(_)) => return Err("script build requires an authoring script_project".into()),
    };
    if !script_project.is_file() {
        return Err(format!(
            "C# script project is missing: {}",
            script_project.display()
        ));
    }
    let script_api_hash = validate_generated_script_api(&project.root, script_project)?;
    let installation =
        crate::engine_installation::EngineInstallation::discover_from_current_executable()?;
    if let Some(installation) = installation.as_ref() {
        installation.validate_managed_sdk_contract(
            engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA,
            engine_script_api::GAMEPLAY_SCRIPT_API_VERSION,
            &script_api_hash,
        )?;
        deploy_installed_managed_tools(project, installation)?;
    } else {
        crate::engine_installation::development_source_root()?;
    }
    let dotnet = resolve_dotnet_executable()?;

    let sdk_output_dir = project.root.join("build/script-sdk");
    ensure_inside_project(&project.root, &sdk_output_dir, "script SDK output")?;
    let sdk_output_next = sibling_with_suffix(&sdk_output_dir, ".next")?;
    let sdk_assembly_name = format!("{}.dll", engine_script_api::MANAGED_SDK_ASSEMBLY_NAME);
    let sdk_assembly = if installation.is_some() {
        sdk_output_dir.join(&sdk_assembly_name)
    } else {
        // Source-tree development keeps a deterministic fallback so engine API
        // work can be tested before an installed distribution is assembled.
        reset_owned_directory(&project.root, &sdk_output_next)?;
        let (_, sdk_project) = materialize_script_sdk_source(&project.root)?;
        let sdk_output = Command::new(&dotnet)
            .arg("build")
            .arg(&sdk_project)
            .arg("--configuration")
            .arg("Release")
            .arg("--nologo")
            .arg("--output")
            .arg(&sdk_output_next)
            .current_dir(sdk_project.parent().unwrap_or(&project.root))
            .output()
            .map_err(|error| format!("could not launch Engine Gameplay SDK build: {error}"))?;
        ensure_command_success("Engine Gameplay SDK build", sdk_output)?;
        sdk_output_next.join(&sdk_assembly_name)
    };
    if !sdk_assembly.is_file() {
        return Err(format!(
            "managed gameplay SDK was not deployed to {}",
            sdk_assembly.display()
        ));
    }

    let assembly_id = assembly_id_from_path(script_assembly)?;
    let output_dir = script_assembly.parent().ok_or_else(|| {
        format!(
            "script assembly has no output directory: {}",
            script_assembly.display()
        )
    })?;
    ensure_inside_project(&project.root, output_dir, "script_assembly output")?;
    let output_next = sibling_with_suffix(output_dir, ".next")?;
    reset_owned_directory(&project.root, &output_next)?;

    let game_output = Command::new(&dotnet)
        .arg("build")
        .arg(script_project)
        .arg("--configuration")
        .arg("Release")
        .arg("--nologo")
        .arg("--output")
        .arg(&output_next)
        .arg(format!(
            "-p:EngineGameplaySdkPath={}",
            sdk_assembly.display()
        ))
        .current_dir(script_project.parent().unwrap_or(&project.root))
        .output()
        .map_err(|error| format!("could not launch dotnet build: {error}"))?;
    ensure_command_success("C# game script build", game_output)?;

    let expected_assembly = output_next.join(
        script_assembly
            .file_name()
            .ok_or_else(|| "script_assembly must name a DLL".to_string())?,
    );
    if !expected_assembly.is_file() {
        return Err(format!(
            "dotnet build succeeded but did not produce the declared script assembly {}; set <AssemblyName>{}</AssemblyName> in {}",
            expected_assembly.display(),
            assembly_id,
            script_project.display()
        ));
    }
    let copied_sdk_assembly = output_next.join(format!(
        "{}.dll",
        engine_script_api::MANAGED_SDK_ASSEMBLY_NAME
    ));
    if !copied_sdk_assembly.is_file() {
        return Err(format!(
            "game script build did not copy the Engine Gameplay SDK dependency to {}",
            copied_sdk_assembly.display()
        ));
    }

    let host_dir = project.root.join("build/script-host");
    let host_executable = host_dir.join(host_executable_name());
    if installation.is_none() {
        let host_source_dir = project.root.join("build/script-host-source");
        ensure_inside_project(&project.root, &host_source_dir, "script host source")?;
        // Reusing a current host matters on Windows: an earlier Play session
        // may still have its executable open while the next game assembly is
        // prepared transactionally.
        let host_is_current = host_executable.is_file()
            && file_contents_equal(
                &host_source_dir.join("EngineScriptHost.csproj"),
                SCRIPT_HOST_PROJECT,
            )
            && file_contents_equal(&host_source_dir.join("Program.cs"), SCRIPT_HOST_SOURCE);

        if host_is_current {
            self_test_script_host(&host_executable, &host_dir)?;
        } else {
            std::fs::create_dir_all(&host_source_dir).map_err(|error| {
                format!(
                    "could not create script host source directory {}: {error}",
                    host_source_dir.display()
                )
            })?;
            write_file(
                &host_source_dir.join("EngineScriptHost.csproj"),
                SCRIPT_HOST_PROJECT,
            )?;
            write_file(&host_source_dir.join("Program.cs"), SCRIPT_HOST_SOURCE)?;

            let host_next = project.root.join("build/script-host.next");
            reset_owned_directory(&project.root, &host_next)?;
            let host_output = Command::new(&dotnet)
                .arg("publish")
                .arg(host_source_dir.join("EngineScriptHost.csproj"))
                .arg("--configuration")
                .arg("Release")
                .arg("--nologo")
                .arg("--self-contained")
                .arg("false")
                .arg("--output")
                .arg(&host_next)
                .current_dir(&host_source_dir)
                .output()
                .map_err(|error| format!("could not launch dotnet publish: {error}"))?;
            ensure_command_success("C# script host publish", host_output)?;

            let next_host_executable = host_next.join(host_executable_name());
            if !next_host_executable.is_file() {
                return Err(format!(
                    "dotnet publish succeeded but did not produce the script host {}",
                    next_host_executable.display()
                ));
            }
            self_test_script_host(&next_host_executable, &host_next)?;
            replace_owned_directory(&project.root, &host_next, &host_dir)?;
        }
    } else if !host_executable.is_file() {
        return Err(format!(
            "installed managed script host was not deployed to {}",
            host_executable.display()
        ));
    }

    if installation.is_none() {
        replace_owned_directory(&project.root, &sdk_output_next, &sdk_output_dir)?;
    }
    replace_owned_directory(&project.root, &output_next, output_dir)?;

    let dependency_assemblies = managed_dependencies(output_dir, script_assembly)?.len();
    Ok(Some(ScriptBuildReport {
        schema: "ProjectScriptBuildReport-v0",
        project: project.manifest.name.clone(),
        script_api: engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA,
        script_api_version: engine_script_api::GAMEPLAY_SCRIPT_API_VERSION,
        script_api_sha256: script_api_hash,
        sdk_assembly: report_path(&sdk_output_dir.join(format!(
            "{}.dll",
            engine_script_api::MANAGED_SDK_ASSEMBLY_NAME
        ))),
        assembly_id,
        assembly: report_path(script_assembly),
        host: report_path(&host_dir.join(host_executable_name())),
        dependency_assemblies,
        passed: true,
    }))
}

/// Register the process host and load dependencies plus the game assembly.
pub(crate) fn prepare_project_scripts(
    runtime: &mut EngineRuntime,
    project: &GameProject,
) -> Result<PreparedScriptRuntime, String> {
    let Some(script_assembly) = project.script_assembly.as_deref() else {
        return Ok(PreparedScriptRuntime::default());
    };

    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = (runtime, script_assembly);
        Err(
            "this project contains C# scripts; rebuild sandbox with the `subsystem-scripting-csharp` feature"
                .into(),
        )
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        let (candidate, prepared) = prepare_isolated_project_script_engine(
            project,
            script_assembly,
            &resolve_script_host(project)?,
        )?;
        runtime
            .replace_script_engine(candidate, SCRIPT_HOST_NAME)
            .map_err(|error| format!("could not activate prepared C# script runtime: {error}"))?;
        Ok(prepared)
    }
}

/// Rebuild authoring scripts and replace the active managed runtime only after
/// a fresh process host has loaded every dependency and the game assembly.
///
/// Build, host-launch, or assembly-load failures return before
/// [`EngineRuntime::replace_script_engine`] is called, leaving the last good
/// runtime usable. Projects without managed scripts are an intentional no-op.
#[cfg(any(
    all(feature = "tooling-editor", feature = "backend-vulkan"),
    all(test, feature = "subsystem-scripting-csharp")
))]
pub(crate) fn rebuild_and_reload_project_scripts(
    runtime: &mut EngineRuntime,
    project: &GameProject,
) -> Result<PreparedScriptRuntime, String> {
    if project.script_project.is_none() && project.script_assembly.is_none() {
        return Ok(PreparedScriptRuntime::default());
    }

    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = runtime;
        Err("cannot reload C# scripts without the `subsystem-scripting-csharp` feature".into())
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        build_project_scripts(project)?;
        let script_assembly = project.script_assembly.as_deref().ok_or_else(|| {
            "script_project is configured but script_assembly is missing from game.project.json"
                .to_string()
        })?;
        let host_path = resolve_script_host(project)?;
        let (candidate, prepared) =
            prepare_isolated_project_script_engine(project, script_assembly, &host_path)?;
        runtime
            .replace_script_engine(candidate, SCRIPT_HOST_NAME)
            .map_err(|error| format!("could not activate rebuilt C# script runtime: {error}"))?;
        Ok(prepared)
    }
}
