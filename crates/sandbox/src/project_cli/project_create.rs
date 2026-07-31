use super::*;

pub(crate) fn create_project(
    root: &Path,
    requested_name: Option<&str>,
    with_csharp: bool,
) -> Result<(), String> {
    if root.as_os_str().is_empty() {
        return Err("project destination must not be empty".into());
    }
    if root.exists() {
        if !root.is_dir() {
            return Err(format!(
                "project destination is not a directory: {}",
                root.display()
            ));
        }
        if std::fs::read_dir(root)
            .map_err(|error| format!("could not inspect {}: {error}", root.display()))?
            .next()
            .is_some()
        {
            return Err(format!(
                "project destination must be empty: {}",
                root.display()
            ));
        }
    }

    let inferred_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Game");
    let mut manifest = ProjectManifest::new(requested_name.unwrap_or(inferred_name));
    if with_csharp {
        manifest.script_project = Some(PathBuf::from("scripts/GameScripts/GameScripts.csproj"));
        manifest.script_assembly = Some(PathBuf::from("build/scripts/GameScripts.dll"));
    }
    manifest
        .validate()
        .map_err(|error| format!("invalid project settings: {error}"))?;

    std::fs::create_dir_all(root.join("assets/source"))
        .map_err(|error| format!("could not create project directories: {error}"))?;
    std::fs::create_dir_all(root.join("assets/scenes"))
        .map_err(|error| format!("could not create project directories: {error}"))?;
    std::fs::create_dir_all(root.join("config"))
        .map_err(|error| format!("could not create project directories: {error}"))?;
    let root = std::fs::canonicalize(root)
        .map_err(|error| format!("could not resolve created project root: {error}"))?;
    let root = root.as_path();

    let scene = engine_scene::starter_scene("main", "Main");
    scene
        .save_to_file(&root.join(&manifest.startup_scene))
        .map_err(|error| format!("could not create starter scene: {error}"))?;

    let source_manifest = SourceManifest {
        schema_version: CURRENT_MANIFEST_VERSION,
        assets: Vec::new(),
    };
    let mut source_json = serde_json::to_string_pretty(&source_manifest)
        .map_err(|error| format!("could not serialize source manifest: {error}"))?;
    source_json.push('\n');
    write_text(&root.join("assets/source/game.manifest"), &source_json)?;
    if let Some(input_actions) = &manifest.input_actions {
        write_text(
            &root.join(input_actions),
            &crate::project_input::starter_input_json(),
        )?;
    }
    if with_csharp {
        let script_project = root.join("scripts/GameScripts/GameScripts.csproj");
        std::fs::create_dir_all(
            script_project
                .parent()
                .expect("starter script project has a parent"),
        )
        .map_err(|error| format!("could not create script source directory: {error}"))?;
        write_text(
            &script_project,
            crate::project_scripts::STARTER_SCRIPT_PROJECT,
        )?;
        crate::project_scripts::write_generated_script_api(root, &script_project)?;
    }
    write_text(&root.join(".gitignore"), "/build/\n/Dist/\n")?;
    write_text(
        &root.join("README.md"),
        &format!(
            "# {}\n\nCreated with the engine project workflow.\n\n\
             ```text\n\
             sandbox project check .\n\
             sandbox project build .\n\
             sandbox project run .\n\
             sandbox project editor .\n\
             ```\n",
            manifest.name
        ),
    )?;
    let manifest_path = manifest
        .write_to_root(root)
        .map_err(|error| format!("could not write project manifest: {error}"))?;
    if with_csharp {
        let project = GameProject::load(&manifest_path)
            .map_err(|error| format!("could not reopen created project: {error}"))?;
        crate::project_scripts::deploy_installed_project_script_runtime(&project).map_err(
            |error| {
                format!(
                    "project files were created at {}, but the installed script SDK/host could \
                     not be deployed: {error}. Repair the engine installation, then run \
                     `sandbox project sync-script-api {}` to finish setup",
                    root.display(),
                    root.display()
                )
            },
        )?;
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "ProjectCreateReport-v0",
            "project": manifest.name,
            "manifest": absolute_for_report(&manifest_path),
            "startup_scene": absolute_for_report(&root.join(&manifest.startup_scene)),
            "with_csharp": with_csharp,
            "created": true
        }))
        .expect("JSON value serialization cannot fail")
    );
    Ok(())
}
