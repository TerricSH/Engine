use super::*;

pub(crate) fn cook_project(path: &Path) -> Result<(), String> {
    let project = GameProject::load(path).map_err(|error| error.to_string())?;
    let cooked_parent = project.cooked_assets.parent().ok_or_else(|| {
        format!(
            "cooked asset path has no parent: {}",
            project.cooked_assets.display()
        )
    })?;
    std::fs::create_dir_all(cooked_parent).map_err(|error| {
        format!(
            "could not create cooked asset parent {}: {error}",
            cooked_parent.display()
        )
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".project-cook-staging-")
        .tempdir_in(cooked_parent)
        .map_err(|error| {
            format!(
                "could not create cook staging directory in {}: {error}",
                cooked_parent.display()
            )
        })?;
    let mut graph = DependencyGraph::new();
    let mut runtime_builder = engine_core::EngineRuntime::builder(engine_core::EngineConfig {
        application_name: format!("{}-asset-cook", project.manifest.name),
        gpu_timestamps: true,
    });
    engine_animation::loader::register_asset_types(runtime_builder.asset_type_registry_mut());
    let report = cook_orchestrate_checked_with_registry(
        &project.asset_source,
        staging.path(),
        &mut graph,
        runtime_builder.asset_type_registry(),
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| format!("could not serialize cook report: {error}"))?
    );
    if !report.is_success() {
        return Err("project asset cooking failed".into());
    }
    replace_cooked_directory(&project, &staging)?;
    Ok(())
}

pub(crate) fn replace_cooked_directory(
    project: &GameProject,
    staging: &tempfile::TempDir,
) -> Result<(), String> {
    let target = &project.cooked_assets;
    if !target.starts_with(&project.root) || !staging.path().starts_with(&project.root) {
        return Err("refusing to replace a cooked directory outside the project root".into());
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("cooked asset path has no parent: {}", target.display()))?;
    let backup = tempfile::Builder::new()
        .prefix(".project-cook-backup-")
        .tempdir_in(parent)
        .map_err(|error| {
            format!(
                "could not create cook backup directory in {}: {error}",
                parent.display()
            )
        })?;
    let previous = backup.path().join("previous");
    let had_previous = target.exists();
    if had_previous {
        std::fs::rename(target, &previous).map_err(|error| {
            format!(
                "could not move previous cooked directory {} aside: {error}",
                target.display()
            )
        })?;
    }

    if let Err(install_error) = std::fs::rename(staging.path(), target) {
        let restore_error = if had_previous {
            std::fs::rename(&previous, target).err()
        } else {
            None
        };
        return match restore_error {
            Some(restore_error) => {
                let preserved_backup = backup.keep();
                Err(format!(
                    "could not install cooked directory {}: {install_error}; restoring the previous directory also failed: {restore_error}; the previous batch was preserved at {}",
                    target.display(),
                    preserved_backup.display()
                ))
            }
            None => Err(format!(
                "could not install cooked directory {}: {install_error}",
                target.display()
            )),
        };
    }
    Ok(())
}

pub(crate) fn sync_project_script_api(path: &Path) -> Result<(), String> {
    let project = GameProject::load(path).map_err(|error| error.to_string())?;
    let report = crate::project_scripts::sync_project_script_api(&project)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| format!("could not serialize script API sync report: {error}"))?
    );
    Ok(())
}

pub(crate) fn build_project_scripts(path: &Path, require_configured: bool) -> Result<(), String> {
    let project = GameProject::load(path).map_err(|error| error.to_string())?;
    match crate::project_scripts::build_project_scripts(&project)? {
        Some(report) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report)
                    .map_err(|error| format!("could not serialize script build report: {error}"))?
            );
            Ok(())
        }
        None if require_configured => {
            Err("project has no script_project/script_assembly configuration".into())
        }
        None => Ok(()),
    }
}

pub(crate) fn source_manifest_paths(source: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(source)
        .map_err(|error| format!("could not enumerate {}: {error}", source.display()))?
    {
        let path = entry
            .map_err(|error| format!("could not enumerate {}: {error}", source.display()))?
            .path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("manifest"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

pub(crate) fn validate_source_path(source_root: &Path, relative: &str) -> Result<(), String> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe source asset path '{relative}'"));
    }
    let resolved = source_root.join(path);
    if !resolved.is_file() {
        return Err(format!("source asset is missing: {}", resolved.display()));
    }
    Ok(())
}

pub(crate) fn write_text(path: &Path, content: &str) -> Result<(), String> {
    std::fs::write(path, content)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

pub(crate) fn emit_report(report: &str, path: Option<&Path>) -> Result<(), String> {
    println!("{report}");
    if let Some(path) = path {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "could not create report directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        write_text(path, &format!("{report}\n"))?;
    }
    Ok(())
}

pub(crate) fn absolute_for_report(path: &Path) -> String {
    if path.is_absolute() {
        path.to_string_lossy().into_owned()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path).to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.to_string_lossy().into_owned())
    }
}
