use super::*;

pub fn parse_run_request(args: &[String]) -> Result<ProjectRunRequest, String> {
    let mut project = None;
    let mut headless = false;
    let mut frames = None;
    let mut report = None;
    let mut scripts_already_built = false;
    let mut stream_cells = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--headless" => headless = true,
            "--scripts-already-built" => scripts_already_built = true,
            "--stream-cells" => stream_cells = true,
            "--frames" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--frames requires a positive integer".to_string())?;
                frames = Some(parse_frame_count(value)?);
            }
            "--report" => {
                index += 1;
                report = Some(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--report requires a path".to_string())?,
                ));
            }
            "--help" | "-h" => {
                return Err(
                    "usage: sandbox project run <project> [--headless] [--frames N] [--stream-cells]"
                        .into(),
                );
            }
            _ if argument.starts_with("--frames=") => {
                frames = Some(parse_frame_count(
                    argument
                        .strip_prefix("--frames=")
                        .expect("prefix was checked"),
                )?);
            }
            _ if argument.starts_with("--report=") => {
                report = Some(PathBuf::from(
                    argument
                        .strip_prefix("--report=")
                        .expect("prefix was checked"),
                ));
            }
            _ if argument.starts_with('-') => {
                return Err(format!("unknown run option '{argument}'"));
            }
            _ if project.is_none() => project = Some(PathBuf::from(argument)),
            _ => return Err(format!("unexpected run argument '{argument}'")),
        }
        index += 1;
    }

    Ok(ProjectRunRequest {
        project: project.ok_or_else(|| "project run requires a project path".to_string())?,
        headless,
        frames,
        report,
        scripts_already_built,
        stream_cells,
    })
}

pub(super) fn dispatch_scene_command(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(scene_usage());
    };
    match command {
        "list" => {
            let project_path = parse_single_project_path("scene list", &args[1..])?;
            let project = GameProject::load(&project_path).map_err(|error| error.to_string())?;
            let scenes = project
                .scenes()
                .into_iter()
                .map(|(id, path)| {
                    serde_json::json!({
                        "id": id,
                        "path": absolute_for_report(&path),
                        "startup": id == project.startup_scene_id(),
                    })
                })
                .collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema": "ProjectSceneListReport-v0",
                    "project": project.manifest.name,
                    "startup_scene_id": project.startup_scene_id(),
                    "scenes": scenes,
                }))
                .expect("JSON value serialization cannot fail")
            );
            Ok(())
        }
        "new" => {
            let (project, scene_id, name) = parse_scene_new_args(&args[1..])?;
            let path = create_project_scene(&project, &scene_id, name.as_deref())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema": "ProjectSceneCreateReport-v0",
                    "scene_id": scene_id,
                    "path": absolute_for_report(&path),
                    "created": true,
                }))
                .expect("JSON value serialization cannot fail")
            );
            Ok(())
        }
        "rename" => {
            let [project, old_id, new_id] = &args[1..] else {
                return Err(scene_usage());
            };
            let path = rename_project_scene(Path::new(project), old_id, new_id)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema": "ProjectSceneRenameReport-v0",
                    "old_scene_id": old_id,
                    "scene_id": new_id,
                    "path": absolute_for_report(&path),
                    "renamed": true,
                }))
                .expect("JSON value serialization cannot fail")
            );
            Ok(())
        }
        "delete" => {
            let (project, scene_id, replacement_startup) = parse_scene_delete_args(&args[1..])?;
            let deleted =
                delete_project_scene(&project, &scene_id, replacement_startup.as_deref())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema": "ProjectSceneDeleteReport-v0",
                    "scene_id": deleted.scene_id,
                    "trash_directory": absolute_for_report(&deleted.trash_directory),
                    "metadata": absolute_for_report(&deleted.metadata_path),
                    "replacement_startup": deleted.replacement_startup,
                    "deleted": true,
                    "recoverable": true,
                }))
                .expect("JSON value serialization cannot fail")
            );
            Ok(())
        }
        "set-startup" => {
            let [project, scene_id] = &args[1..] else {
                return Err(scene_usage());
            };
            let path = set_project_startup_scene(Path::new(project), scene_id)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema": "ProjectSceneStartupReport-v0",
                    "scene_id": scene_id,
                    "path": absolute_for_report(&path),
                    "updated": true,
                }))
                .expect("JSON value serialization cannot fail")
            );
            Ok(())
        }
        "help" | "--help" | "-h" => {
            println!("{}", scene_usage());
            Ok(())
        }
        other => Err(format!(
            "unknown project scene command '{other}'\n{}",
            scene_usage()
        )),
    }
}

pub(super) fn parse_scene_new_args(
    args: &[String],
) -> Result<(PathBuf, String, Option<String>), String> {
    let mut positional = Vec::new();
    let mut name = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--name" => {
                index += 1;
                name = Some(
                    args.get(index)
                        .ok_or_else(|| "--name requires a scene display name".to_string())?
                        .clone(),
                );
            }
            argument if argument.starts_with("--name=") => {
                name = Some(
                    argument
                        .strip_prefix("--name=")
                        .expect("prefix was checked")
                        .to_string(),
                );
            }
            argument if argument.starts_with('-') => {
                return Err(format!("unknown project scene new option '{argument}'"));
            }
            argument => positional.push(argument.to_string()),
        }
        index += 1;
    }
    if positional.len() != 2 {
        return Err(scene_usage());
    }
    Ok((
        PathBuf::from(positional.remove(0)),
        positional.remove(0),
        name,
    ))
}

pub(super) fn parse_scene_delete_args(
    args: &[String],
) -> Result<(PathBuf, String, Option<String>), String> {
    let mut positional = Vec::new();
    let mut replacement_startup = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--replacement-startup" => {
                if replacement_startup.is_some() {
                    return Err(
                        "project scene delete received --replacement-startup more than once".into(),
                    );
                }
                index += 1;
                replacement_startup = Some(
                    args.get(index)
                        .ok_or_else(|| "--replacement-startup requires a scene ID".to_string())?
                        .clone(),
                );
            }
            argument if argument.starts_with("--replacement-startup=") => {
                if replacement_startup.is_some() {
                    return Err(
                        "project scene delete received --replacement-startup more than once".into(),
                    );
                }
                replacement_startup = Some(
                    argument
                        .strip_prefix("--replacement-startup=")
                        .expect("prefix was checked")
                        .to_string(),
                );
            }
            "--help" | "-h" => return Err(scene_usage()),
            argument if argument.starts_with('-') => {
                return Err(format!("unknown project scene delete option '{argument}'"));
            }
            argument => positional.push(argument.to_string()),
        }
        index += 1;
    }
    if positional.len() != 2 {
        return Err(scene_usage());
    }
    Ok((
        PathBuf::from(positional.remove(0)),
        positional.remove(0),
        replacement_startup,
    ))
}

pub(super) fn scene_usage() -> String {
    "usage:\n  sandbox project scene list <project>\n  sandbox project scene new <project> <scene-id> [--name NAME]\n  sandbox project scene rename <project> <old-id> <new-id>\n  sandbox project scene delete <project> <scene-id> [--replacement-startup ID]\n  sandbox project scene set-startup <project> <scene-id>".into()
}

pub(super) fn parse_new_args(args: &[String]) -> Result<(PathBuf, Option<String>, bool), String> {
    let mut root = None;
    let mut name = None;
    let mut with_csharp = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--with-csharp" => with_csharp = true,
            "--name" => {
                index += 1;
                name = Some(
                    args.get(index)
                        .ok_or_else(|| "--name requires a value".to_string())?
                        .clone(),
                );
            }
            _ if argument.starts_with("--name=") => {
                name = Some(
                    argument
                        .strip_prefix("--name=")
                        .expect("prefix was checked")
                        .to_string(),
                );
            }
            _ if argument.starts_with('-') => {
                return Err(format!("unknown new-project option '{argument}'"));
            }
            _ if root.is_none() => root = Some(PathBuf::from(argument)),
            _ => return Err(format!("unexpected new-project argument '{argument}'")),
        }
        index += 1;
    }
    Ok((
        root.ok_or_else(|| "project new requires a destination directory".to_string())?,
        name,
        with_csharp,
    ))
}

pub(super) fn parse_import_args(args: &[String]) -> Result<ProjectImportRequest, String> {
    let mut positional = Vec::new();
    let mut asset_id = None;
    let mut asset_type = None;
    let mut folder = None;
    let mut merge_primitives = true;
    let mut bake_node_transforms = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--id" => {
                if asset_id.is_some() {
                    return Err("project import received --id more than once".into());
                }
                index += 1;
                asset_id = Some(
                    args.get(index)
                        .ok_or_else(|| "--id requires an asset ID".to_string())?
                        .clone(),
                );
            }
            "--type" => {
                if asset_type.is_some() {
                    return Err("project import received --type more than once".into());
                }
                index += 1;
                asset_type = Some(parse_import_asset_type(
                    args.get(index)
                        .ok_or_else(|| "--type requires a supported asset type".to_string())?,
                )?);
            }
            "--folder" => {
                if folder.is_some() {
                    return Err("project import received --folder more than once".into());
                }
                index += 1;
                folder = Some(PathBuf::from(args.get(index).ok_or_else(|| {
                    "--folder requires a project-relative path".to_string()
                })?));
            }
            "--separate-primitives" => merge_primitives = false,
            "--merge-primitives" => merge_primitives = true,
            "--no-bake-node-transforms" => bake_node_transforms = Some(false),
            "--bake-node-transforms" => bake_node_transforms = Some(true),
            _ if argument.starts_with("--id=") => {
                if asset_id.is_some() {
                    return Err("project import received --id more than once".into());
                }
                asset_id = Some(
                    argument
                        .strip_prefix("--id=")
                        .expect("prefix was checked")
                        .to_string(),
                );
            }
            _ if argument.starts_with("--type=") => {
                if asset_type.is_some() {
                    return Err("project import received --type more than once".into());
                }
                asset_type = Some(parse_import_asset_type(
                    argument
                        .strip_prefix("--type=")
                        .expect("prefix was checked"),
                )?);
            }
            _ if argument.starts_with("--folder=") => {
                if folder.is_some() {
                    return Err("project import received --folder more than once".into());
                }
                folder = Some(PathBuf::from(
                    argument
                        .strip_prefix("--folder=")
                        .expect("prefix was checked"),
                ));
            }
            "--help" | "-h" => return Err(import_usage()),
            _ if argument.starts_with('-') => {
                return Err(format!("unknown project import option '{argument}'"));
            }
            _ => positional.push(PathBuf::from(argument)),
        }
        index += 1;
    }

    if positional.len() != 2 {
        return Err(import_usage());
    }
    Ok(ProjectImportRequest {
        project: positional.remove(0),
        source_file: positional.remove(0),
        asset_id: asset_id.ok_or_else(|| "project import requires --id <asset-id>".to_string())?,
        asset_type,
        folder: folder.unwrap_or_default(),
        merge_primitives,
        bake_node_transforms,
    })
}

pub(super) fn parse_import_asset_type(value: &str) -> Result<AssetType, String> {
    match value.to_ascii_lowercase().as_str() {
        "mesh" => Ok(AssetType::Mesh),
        "texture" => Ok(AssetType::Texture),
        "material" => Ok(AssetType::Material),
        "environment" | "environment-map" | "hdri" => Ok(AssetType::EnvironmentMap),
        "audio" => Ok(AssetType::Audio),
        "animation" => Ok(AssetType::Animation),
        "skeleton" => Ok(AssetType::Skeleton),
        "navmesh" | "nav" => Ok(AssetType::NavMesh),
        "prefab" => Ok(AssetType::Prefab),
        _ => Err(format!(
            "unsupported import type '{value}'; expected mesh, texture, material, environment, audio, animation, skeleton, navmesh, or prefab"
        )),
    }
}

pub(super) fn import_usage() -> String {
    "usage: sandbox project import <project> <source-file> --id <asset-id> [--type mesh|texture|material|environment|audio|animation|skeleton|navmesh|prefab] [--folder <path-below-assets/source>] [--separate-primitives] [--no-bake-node-transforms]".into()
}

pub(super) fn parse_single_project_path(command: &str, args: &[String]) -> Result<PathBuf, String> {
    match args {
        [path] if !path.starts_with('-') => Ok(PathBuf::from(path)),
        [] => Err(format!("project {command} requires a project path")),
        _ => Err(format!(
            "usage: sandbox project {command} <project-directory-or-manifest>"
        )),
    }
}

pub(super) fn parse_project_report_args(
    command: &str,
    args: &[String],
) -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut project = None;
    let mut report = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--report" => {
                index += 1;
                report = Some(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--report requires a path".to_string())?,
                ));
            }
            argument if argument.starts_with("--report=") => {
                report = Some(PathBuf::from(
                    argument
                        .strip_prefix("--report=")
                        .expect("prefix was checked"),
                ));
            }
            argument if argument.starts_with('-') => {
                return Err(format!("unknown project {command} option '{argument}'"));
            }
            argument if project.is_none() => project = Some(PathBuf::from(argument)),
            argument => {
                return Err(format!(
                    "unexpected project {command} argument '{argument}'"
                ))
            }
        }
        index += 1;
    }
    Ok((
        project.ok_or_else(|| format!("project {command} requires a project path"))?,
        report,
    ))
}

pub(super) fn parse_frame_count(value: &str) -> Result<u64, String> {
    let frames = value
        .parse::<u64>()
        .map_err(|_| format!("invalid frame count '{value}'"))?;
    if frames == 0 {
        return Err("frame count must be greater than zero".into());
    }
    Ok(frames)
}
