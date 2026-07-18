use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{
    cook_orchestrate_checked_with_registry, read_cooked_artifact, registered_asset_type_id,
    AssetType, CookRules, DependencyGraph, SourceAssetEntry, SourceManifest,
};
use engine_asset::project::{GameProject, ProjectManifest};
use engine_scene::{validate_scene, Scene};
use engine_serialize::{AssetId, DiagnosticSeverity};
use serde::{Deserialize, Serialize};

const SCENE_TRASH_SCHEMA: &str = "EditorSceneTrash-v0";
static SCENE_OPERATION_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRunRequest {
    pub project: PathBuf,
    pub headless: bool,
    pub frames: Option<u64>,
    pub report: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectAction {
    Complete,
    Run(ProjectRunRequest),
    Edit(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProjectImportRequest {
    project: PathBuf,
    source_file: PathBuf,
    asset_id: String,
    asset_type: Option<AssetType>,
    folder: PathBuf,
}

pub fn dispatch(args: &[String]) -> Result<ProjectAction, String> {
    let Some(command) = args.first().map(String::as_str) else {
        print_project_help();
        return Ok(ProjectAction::Complete);
    };

    match command {
        "new" => {
            let (root, name, with_csharp) = parse_new_args(&args[1..])?;
            create_project(&root, name.as_deref(), with_csharp)?;
            Ok(ProjectAction::Complete)
        }
        "check" => {
            let (project, report) = parse_project_report_args("check", &args[1..])?;
            check_project(&project, report.as_deref())?;
            Ok(ProjectAction::Complete)
        }
        "cook" => {
            let project = parse_single_project_path("cook", &args[1..])?;
            cook_project(&project)?;
            Ok(ProjectAction::Complete)
        }
        "import" => {
            let request = parse_import_args(&args[1..])?;
            import_project_asset(&request)?;
            Ok(ProjectAction::Complete)
        }
        "scene" => {
            dispatch_scene_command(&args[1..])?;
            Ok(ProjectAction::Complete)
        }
        "build-scripts" => {
            let project = parse_single_project_path("build-scripts", &args[1..])?;
            build_project_scripts(&project, true)?;
            Ok(ProjectAction::Complete)
        }
        "sync-script-api" => {
            let project = parse_single_project_path("sync-script-api", &args[1..])?;
            sync_project_script_api(&project)?;
            Ok(ProjectAction::Complete)
        }
        "build" => {
            let project_path = parse_single_project_path("build", &args[1..])?;
            build_project(&project_path)?;
            Ok(ProjectAction::Complete)
        }
        "run" => Ok(ProjectAction::Run(parse_run_request(&args[1..])?)),
        "edit" | "editor" => Ok(ProjectAction::Edit(parse_single_project_path(
            "editor",
            &args[1..],
        )?)),
        "help" | "--help" | "-h" => {
            print_project_help();
            Ok(ProjectAction::Complete)
        }
        other => Err(format!(
            "unknown project command '{other}'; run `sandbox project --help`"
        )),
    }
}

pub(crate) fn build_project(path: &Path) -> Result<(), String> {
    cook_project(path)?;
    build_project_scripts(path, false)
}

pub fn parse_run_request(args: &[String]) -> Result<ProjectRunRequest, String> {
    let mut project = None;
    let mut headless = false;
    let mut frames = None;
    let mut report = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--headless" => headless = true,
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
                    "usage: sandbox project run <project> [--headless] [--frames N]".into(),
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
    })
}

pub fn print_global_help() {
    println!(
        "Engine sandbox\n\n\
         Game project workflow:\n\
           sandbox project new <directory> [--name NAME] [--with-csharp]\n\
           sandbox project check <project> [--report PATH]\n\
           sandbox project import <project> <source-file> --id ID [--type TYPE]\n\
           sandbox project scene list <project>\n\
           sandbox project scene new <project> <scene-id> [--name NAME]\n\
           sandbox project scene rename <project> <old-id> <new-id>\n\
           sandbox project scene delete <project> <scene-id> [--replacement-startup ID]\n\
           sandbox project scene set-startup <project> <scene-id>\n\
           sandbox project cook <project>\n\
           sandbox project sync-script-api <project>\n\
           sandbox project build-scripts <project>\n\
           sandbox project build <project>\n\
           sandbox project run <project> [--headless] [--frames N] [--report PATH]\n\
           sandbox project editor <project>\n\n\
         Short aliases:\n\
           sandbox game <project> [--headless] [--frames N]\n\
           sandbox editor <project>"
    );
}

fn print_project_help() {
    println!(
        "Game project commands:\n\
           new      create a portable project and starter scene\n\
           check    validate the manifest, scene, and source asset references\n\
           import   copy, register, and cook a mesh, texture, or material source\n\
           scene    list, create, and choose the startup scene\n\
           cook     cook the project's source assets\n\
           sync-script-api  refresh the engine-owned versioned C# gameplay contract\n\
           build-scripts  compile C# scripts and publish the script host\n\
           build    cook assets and compile configured scripts\n\
           run      run the startup scene\n\
           editor   open the project in the editor"
    );
}

fn dispatch_scene_command(args: &[String]) -> Result<(), String> {
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

fn parse_scene_new_args(args: &[String]) -> Result<(PathBuf, String, Option<String>), String> {
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

fn parse_scene_delete_args(args: &[String]) -> Result<(PathBuf, String, Option<String>), String> {
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

fn scene_usage() -> String {
    "usage:\n  sandbox project scene list <project>\n  sandbox project scene new <project> <scene-id> [--name NAME]\n  sandbox project scene rename <project> <old-id> <new-id>\n  sandbox project scene delete <project> <scene-id> [--replacement-startup ID]\n  sandbox project scene set-startup <project> <scene-id>".into()
}

fn parse_new_args(args: &[String]) -> Result<(PathBuf, Option<String>, bool), String> {
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

fn parse_import_args(args: &[String]) -> Result<ProjectImportRequest, String> {
    let mut positional = Vec::new();
    let mut asset_id = None;
    let mut asset_type = None;
    let mut folder = None;
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
    })
}

fn parse_import_asset_type(value: &str) -> Result<AssetType, String> {
    match value.to_ascii_lowercase().as_str() {
        "mesh" => Ok(AssetType::Mesh),
        "texture" => Ok(AssetType::Texture),
        "material" => Ok(AssetType::Material),
        "audio" => Ok(AssetType::Audio),
        "animation" => Ok(AssetType::Animation),
        "skeleton" => Ok(AssetType::Skeleton),
        "navmesh" | "nav" => Ok(AssetType::NavMesh),
        "prefab" => Ok(AssetType::Prefab),
        _ => Err(format!(
            "unsupported import type '{value}'; expected mesh, texture, material, audio, animation, skeleton, navmesh, or prefab"
        )),
    }
}

fn import_usage() -> String {
    "usage: sandbox project import <project> <source-file> --id <asset-id> [--type mesh|texture|material|audio|animation|skeleton|navmesh|prefab] [--folder <path-below-assets/source>]".into()
}

fn parse_single_project_path(command: &str, args: &[String]) -> Result<PathBuf, String> {
    match args {
        [path] if !path.starts_with('-') => Ok(PathBuf::from(path)),
        [] => Err(format!("project {command} requires a project path")),
        _ => Err(format!(
            "usage: sandbox project {command} <project-directory-or-manifest>"
        )),
    }
}

fn parse_project_report_args(
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

fn parse_frame_count(value: &str) -> Result<u64, String> {
    let frames = value
        .parse::<u64>()
        .map_err(|_| format!("invalid frame count '{value}'"))?;
    if frames == 0 {
        return Err("frame count must be greater than zero".into());
    }
    Ok(frames)
}

pub(crate) fn create_project(
    root: &Path,
    requested_name: Option<&str>,
    with_csharp: bool,
) -> Result<(), String> {
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
            &super::project_input::starter_input_json(),
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
            super::project_scripts::STARTER_SCRIPT_PROJECT,
        )?;
        super::project_scripts::write_generated_script_api(root, &script_project)?;
    }
    write_text(&root.join(".gitignore"), "/build/\n")?;
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

pub(crate) fn create_project_scene(
    project_path: &Path,
    scene_id: &str,
    requested_name: Option<&str>,
) -> Result<PathBuf, String> {
    create_project_scene_from(project_path, scene_id, requested_name, None, None)
}

#[cfg(any(feature = "tooling-editor", test))]
pub(crate) fn create_project_scene_in_folder(
    project_path: &Path,
    scene_id: &str,
    requested_name: Option<&str>,
    relative_folder: &Path,
) -> Result<PathBuf, String> {
    create_project_scene_from(
        project_path,
        scene_id,
        requested_name,
        None,
        Some(relative_folder),
    )
}

#[cfg(any(feature = "tooling-editor", test))]
pub(crate) fn duplicate_project_scene(
    project_path: &Path,
    scene_id: &str,
    source: &Scene,
) -> Result<PathBuf, String> {
    create_project_scene_from(
        project_path,
        scene_id,
        Some(source.name.as_str()),
        Some(source),
        None,
    )
}

fn create_project_scene_from(
    project_path: &Path,
    scene_id: &str,
    requested_name: Option<&str>,
    source: Option<&Scene>,
    relative_folder: Option<&Path>,
) -> Result<PathBuf, String> {
    let (_operation_guard, project) = lock_project_scene_operations(project_path)?;
    validate_portable_scene_id(scene_id)?;
    if project
        .scenes()
        .iter()
        .any(|(existing, _)| existing.eq_ignore_ascii_case(scene_id))
    {
        return Err(format!("project scene ID already exists: '{scene_id}'"));
    }

    let relative_folder = portable_scene_subdirectory(relative_folder.unwrap_or(Path::new("")))?;
    let relative_path = if relative_folder.as_os_str().is_empty() {
        PathBuf::from(format!("assets/scenes/{scene_id}.scene.ron"))
    } else {
        PathBuf::from(format!(
            "assets/scenes/{}/{scene_id}.scene.ron",
            portable_path_string(&relative_folder)?
        ))
    };
    let mut manifest = project.manifest.clone();
    // Mutating a legacy project upgrades it to an explicit catalog without
    // changing the stable `main` ID synthesized by the loader.
    manifest.scenes = manifest.scene_catalog();
    manifest
        .scenes
        .insert(scene_id.to_string(), relative_path.clone());
    manifest
        .validate()
        .map_err(|error| format!("invalid scene catalog update: {error}"))?;

    let target = project.root.join(&relative_path);
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("scene path has no portable file name: {}", target.display()))?;
    let parent = target
        .parent()
        .ok_or_else(|| format!("scene path has no parent: {}", target.display()))?;
    ensure_scene_directory_chain(&project.root, parent, true)?;
    if let Some(conflict) = find_case_insensitive_entry(parent, file_name)? {
        return Err(format!(
            "scene file already exists and will not be overwritten: {}",
            conflict.display()
        ));
    }

    let display_name = requested_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(scene_id);
    let mut scene = source
        .cloned()
        .unwrap_or_else(|| engine_scene::starter_scene(scene_id, display_name));
    scene.scene_id = scene_id.to_string();
    scene.name = display_name.to_string();
    commit_scene_transaction(
        &project.root,
        vec![
            SceneTransactionWrite::create(target.clone(), serialize_scene(&scene)?),
            SceneTransactionWrite::replace(
                project.manifest_path.clone(),
                serialize_project_manifest(&manifest)?,
            ),
        ],
        Vec::new(),
        None,
    )?;
    Ok(target)
}

/// Result of moving a project scene into the recoverable editor trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeletedProjectScene {
    pub scene_id: String,
    pub trash_directory: PathBuf,
    pub metadata_path: PathBuf,
    pub replacement_startup: Option<String>,
}

/// Rename a cataloged project scene and its authoring file as one transaction.
///
/// The serialized scene ID always follows the catalog ID. A display name that
/// was generated from the previous catalog/serialized ID follows the rename;
/// an explicitly-authored display name remains unchanged.
pub(crate) fn rename_project_scene(
    project_path: &Path,
    old_id: &str,
    new_id: &str,
) -> Result<PathBuf, String> {
    rename_project_scene_impl(project_path, old_id, new_id, None)
}

fn rename_project_scene_impl(
    project_path: &Path,
    old_id: &str,
    new_id: &str,
    fail_after_mutation: Option<usize>,
) -> Result<PathBuf, String> {
    let (_operation_guard, project) = lock_project_scene_operations(project_path)?;
    if old_id == new_id {
        return Err(format!("project scene already has ID '{old_id}'"));
    }
    if old_id.eq_ignore_ascii_case(new_id) {
        return Err(format!(
            "project scene IDs cannot be renamed only by case: '{old_id}' -> '{new_id}'"
        ));
    }
    validate_portable_scene_id(new_id)?;

    let catalog = project.manifest.scene_catalog();
    let old_relative = exact_scene_catalog_path(&catalog, old_id)?.clone();
    if let Some((conflicting_id, _)) = catalog
        .iter()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(new_id))
    {
        return Err(format!(
            "project scene ID '{new_id}' collides with existing scene '{conflicting_id}'"
        ));
    }
    let old_path = project.root.join(&old_relative);
    ensure_scene_file_is_regular(&project.root, &old_path)?;

    let scene_directory = old_relative.parent().ok_or_else(|| {
        format!(
            "cataloged scene path has no parent directory: {}",
            old_relative.display()
        )
    })?;
    let desired_relative = scene_directory.join(format!("{new_id}.scene.ron"));
    let same_portable_path =
        portable_scene_path_key(&old_relative) == portable_scene_path_key(&desired_relative);
    let new_relative = if same_portable_path {
        old_relative.clone()
    } else {
        desired_relative
    };
    let target = project.root.join(&new_relative);
    let target_parent = target
        .parent()
        .ok_or_else(|| format!("scene path has no parent: {}", target.display()))?;
    ensure_scene_directory_chain(&project.root, target_parent, true)?;
    if !same_portable_path {
        let file_name = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("scene path has no portable file name: {}", target.display()))?;
        if let Some(conflict) = find_case_insensitive_entry(target_parent, file_name)? {
            return Err(format!(
                "renamed scene file already exists or differs only by case: {}",
                conflict.display()
            ));
        }
    }

    let mut scene = Scene::load_from_file(&old_path).map_err(|error| {
        format!(
            "could not load project scene '{old_id}' from {}: {error}",
            old_path.display()
        )
    })?;
    let display_name_follows_identity =
        scene.name.trim().is_empty() || scene.name == old_id || scene.name == scene.scene_id;
    scene.scene_id = new_id.to_string();
    if display_name_follows_identity {
        scene.name = new_id.to_string();
    }

    let mut manifest = project.manifest.clone();
    manifest.scenes = catalog;
    manifest
        .scenes
        .remove(old_id)
        .expect("the exact scene catalog entry was located above");
    manifest
        .scenes
        .insert(new_id.to_string(), new_relative.clone());
    if project.startup_scene_id() == old_id {
        manifest.startup_scene = PathBuf::from(new_id);
    }
    let manifest_bytes = serialize_project_manifest(&manifest)?;
    let scene_bytes = serialize_scene(&scene)?;

    let (writes, deletes) = if same_portable_path {
        (
            vec![
                SceneTransactionWrite::replace(old_path.clone(), scene_bytes),
                SceneTransactionWrite::replace(project.manifest_path.clone(), manifest_bytes),
            ],
            Vec::new(),
        )
    } else {
        (
            vec![
                SceneTransactionWrite::create(target.clone(), scene_bytes),
                SceneTransactionWrite::replace(project.manifest_path.clone(), manifest_bytes),
            ],
            vec![old_path],
        )
    };
    commit_scene_transaction(&project.root, writes, deletes, fail_after_mutation)?;
    Ok(target)
}

/// Move a scene into `.engine/trash/scenes` and remove its catalog entry.
///
/// The final project scene cannot be deleted. Deleting the startup scene also
/// requires an explicit, existing replacement ID; no implicit selection is
/// made from catalog order.
pub(crate) fn delete_project_scene(
    project_path: &Path,
    scene_id: &str,
    replacement_startup: Option<&str>,
) -> Result<DeletedProjectScene, String> {
    delete_project_scene_impl(project_path, scene_id, replacement_startup, None)
}

fn delete_project_scene_impl(
    project_path: &Path,
    scene_id: &str,
    replacement_startup: Option<&str>,
    fail_after_mutation: Option<usize>,
) -> Result<DeletedProjectScene, String> {
    let (_operation_guard, project) = lock_project_scene_operations(project_path)?;
    let catalog = project.manifest.scene_catalog();
    if catalog.len() <= 1 {
        return Err("a project must retain at least one scene".into());
    }
    let old_relative = exact_scene_catalog_path(&catalog, scene_id)?.clone();
    let old_path = project.root.join(&old_relative);
    ensure_scene_file_is_regular(&project.root, &old_path)?;
    let scene = Scene::load_from_file(&old_path).map_err(|error| {
        format!(
            "could not load project scene '{scene_id}' from {}: {error}",
            old_path.display()
        )
    })?;

    let deleting_startup = project.startup_scene_id() == scene_id;
    let replacement_startup = if deleting_startup {
        let replacement = replacement_startup.ok_or_else(|| {
            format!(
                "deleting startup scene '{scene_id}' requires an explicit replacement startup scene"
            )
        })?;
        if replacement == scene_id || replacement.eq_ignore_ascii_case(scene_id) {
            return Err("the deleted scene cannot replace itself as the startup scene".into());
        }
        exact_scene_catalog_path(&catalog, replacement)?;
        Some(replacement.to_string())
    } else {
        None
    };

    let mut manifest = project.manifest.clone();
    manifest.scenes = catalog;
    manifest
        .scenes
        .remove(scene_id)
        .expect("the exact scene catalog entry was located above");
    if let Some(replacement) = &replacement_startup {
        manifest.startup_scene = PathBuf::from(replacement);
    }
    let manifest_bytes = serialize_project_manifest(&manifest)?;

    let deleted_unix_nanos = unix_nanos()?;
    let trash_directory = allocate_scene_trash_directory(&project, scene_id, deleted_unix_nanos)?;
    let trash_scene_path = trash_directory.join("scene.scene.ron");
    let metadata_path = trash_directory.join("metadata.json");
    let metadata = SceneTrashMetadata {
        schema: SCENE_TRASH_SCHEMA.to_string(),
        deleted_unix_nanos,
        scene_id: scene_id.to_string(),
        scene_name: scene.name,
        original_scene_path: portable_project_relative_path(&project.root, &old_path)?,
        original_startup_scene: portable_path_string(&project.manifest.startup_scene)?,
        was_startup: deleting_startup,
        replacement_startup: replacement_startup.clone(),
    };
    let mut metadata_bytes = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("could not serialize scene trash metadata: {error}"))?;
    metadata_bytes.push(b'\n');
    let scene_bytes = std::fs::read(&old_path)
        .map_err(|error| format!("could not read {}: {error}", old_path.display()))?;
    std::fs::create_dir(&trash_directory).map_err(|error| {
        format!(
            "could not create scene trash directory {}: {error}",
            trash_directory.display()
        )
    })?;

    let result = commit_scene_transaction(
        &project.root,
        vec![
            SceneTransactionWrite::create(trash_scene_path, scene_bytes),
            SceneTransactionWrite::create(metadata_path.clone(), metadata_bytes),
            SceneTransactionWrite::replace(project.manifest_path.clone(), manifest_bytes),
        ],
        vec![old_path],
        fail_after_mutation,
    );
    if let Err(error) = result {
        let cleanup_error = match std::fs::remove_dir(&trash_directory) {
            Ok(()) => None,
            Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => None,
            Err(cleanup_error) => Some(cleanup_error),
        };
        return match cleanup_error {
            None => Err(error),
            Some(cleanup_error) => Err(format!(
                "{error}\nscene trash rollback could not remove {}: {cleanup_error}",
                trash_directory.display()
            )),
        };
    }

    Ok(DeletedProjectScene {
        scene_id: scene_id.to_string(),
        trash_directory,
        metadata_path,
        replacement_startup,
    })
}

pub(crate) fn set_project_startup_scene(
    project_path: &Path,
    scene_id: &str,
) -> Result<PathBuf, String> {
    let (_operation_guard, project) = lock_project_scene_operations(project_path)?;
    let catalog = project.manifest.scene_catalog();
    let relative = exact_scene_catalog_path(&catalog, scene_id)?;
    let scene_path = project.root.join(relative);
    let mut manifest = project.manifest.clone();
    manifest.scenes = catalog;
    manifest.startup_scene = PathBuf::from(scene_id);
    atomic_write_project_manifest(&manifest, &project.manifest_path)?;
    Ok(scene_path)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SceneTrashMetadata {
    schema: String,
    deleted_unix_nanos: u128,
    scene_id: String,
    scene_name: String,
    original_scene_path: String,
    original_startup_scene: String,
    was_startup: bool,
    replacement_startup: Option<String>,
}

struct ProjectSceneOperationGuard {
    _mutex_guard: MutexGuard<'static, ()>,
    lock_file: Option<File>,
    lock_path: PathBuf,
}

impl Drop for ProjectSceneOperationGuard {
    fn drop(&mut self) {
        self.lock_file.take();
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

fn lock_project_scene_operations(
    project_path: &Path,
) -> Result<(ProjectSceneOperationGuard, GameProject), String> {
    let mutex_guard = SCENE_OPERATION_MUTEX
        .lock()
        .map_err(|_| "project scene operation lock was poisoned by a prior panic".to_string())?;
    let initial = GameProject::load(project_path).map_err(|error| error.to_string())?;
    let lock_directory = initial.root.join(".engine/locks");
    ensure_scene_directory_chain(&initial.root, &lock_directory, true)?;
    let lock_path = lock_directory.join("scene-operations.lock");
    let created_unix_nanos = unix_nanos()?;
    let mut lock_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "another project scene operation is active (or left a stale lock): {}",
                    lock_path.display()
                )
            } else {
                format!(
                    "could not acquire project scene operation lock {}: {error}",
                    lock_path.display()
                )
            }
        })?;
    let owner = format!(
        "pid={}\ncreated_unix_nanos={}\n",
        std::process::id(),
        created_unix_nanos
    );
    if let Err(error) = lock_file
        .write_all(owner.as_bytes())
        .and_then(|()| lock_file.sync_all())
    {
        drop(lock_file);
        let _ = std::fs::remove_file(&lock_path);
        return Err(format!(
            "could not initialize project scene operation lock {}: {error}",
            lock_path.display()
        ));
    }
    let guard = ProjectSceneOperationGuard {
        _mutex_guard: mutex_guard,
        lock_file: Some(lock_file),
        lock_path,
    };
    // Reload after acquiring the cross-process lock so no catalog snapshot
    // taken before another process committed is used for a mutation.
    let project = GameProject::load(&initial.root).map_err(|error| error.to_string())?;
    Ok((guard, project))
}

fn exact_scene_catalog_path<'a>(
    catalog: &'a BTreeMap<String, PathBuf>,
    scene_id: &str,
) -> Result<&'a PathBuf, String> {
    if let Some(path) = catalog.get(scene_id) {
        return Ok(path);
    }
    if let Some((actual, _)) = catalog
        .iter()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(scene_id))
    {
        return Err(format!(
            "project scene ID is case-sensitive; requested '{scene_id}', catalog contains '{actual}'"
        ));
    }
    Err(format!(
        "unknown project scene '{scene_id}'; available scenes: {}",
        catalog.keys().cloned().collect::<Vec<_>>().join(", ")
    ))
}

fn validate_portable_scene_id(scene_id: &str) -> Result<(), String> {
    let valid = !scene_id.is_empty()
        && scene_id.len() <= 128
        && scene_id != "."
        && scene_id != ".."
        && scene_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if !valid {
        return Err(format!(
            "scene ID '{scene_id}' must contain 1..=128 ASCII letters, digits, hyphens, underscores, or dots"
        ));
    }
    let portable_stem = scene_id
        .split('.')
        .next()
        .unwrap_or(scene_id)
        .to_ascii_uppercase();
    if matches!(portable_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (portable_stem.len() == 4
            && (portable_stem.starts_with("COM") || portable_stem.starts_with("LPT"))
            && portable_stem.as_bytes()[3].is_ascii_digit()
            && portable_stem.as_bytes()[3] != b'0')
    {
        return Err(format!(
            "scene ID '{scene_id}' is not portable because it uses a reserved file name"
        ));
    }
    Ok(())
}

fn portable_scene_subdirectory(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(format!(
                "scene folder must be a safe project-relative path: {}",
                path.display()
            ));
        };
        let component = component
            .to_str()
            .ok_or_else(|| "scene folder contains non-UTF-8 text".to_string())?;
        if component.is_empty()
            || component.ends_with([' ', '.'])
            || component.chars().any(|character| {
                character.is_control()
                    || matches!(
                        character,
                        '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                    )
            })
        {
            return Err(format!(
                "scene folder contains a non-portable component '{component}'"
            ));
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or(component)
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit()
                && stem.as_bytes()[3] != b'0')
        {
            return Err(format!(
                "scene folder uses reserved portable name '{component}'"
            ));
        }
        normalized.push(component);
    }
    Ok(normalized)
}

fn ensure_scene_file_is_regular(project_root: &Path, path: &Path) -> Result<(), String> {
    ensure_no_scene_symlink_ancestors(project_root, path)?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect scene file {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "project scene path is not a regular file: {}",
            path.display()
        ));
    }
    Ok(())
}

fn ensure_no_scene_symlink_ancestors(project_root: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(project_root).map_err(|_| {
        format!(
            "project scene operation path escapes project root: {}",
            path.display()
        )
    })?;
    let mut current = project_root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(component) => current.push(component),
            Component::CurDir => continue,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "project scene operation path is not normalized: {}",
                    path.display()
                ));
            }
        }
        if current.exists() {
            let metadata = std::fs::symlink_metadata(&current)
                .map_err(|error| format!("could not inspect {}: {error}", current.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "project scene operations do not follow symbolic links: {}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn ensure_scene_directory_chain(
    project_root: &Path,
    directory: &Path,
    create_missing: bool,
) -> Result<(), String> {
    let relative = directory.strip_prefix(project_root).map_err(|_| {
        format!(
            "project scene directory escapes project root: {}",
            directory.display()
        )
    })?;
    let mut current = project_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            if matches!(component, Component::CurDir) {
                continue;
            }
            return Err(format!(
                "project scene directory is not normalized: {}",
                directory.display()
            ));
        };
        let requested = component
            .to_str()
            .ok_or_else(|| "project scene directory contains non-UTF-8 text".to_string())?;
        if let Some(existing) = find_case_insensitive_entry(&current, requested)? {
            if existing.file_name() != Some(component) {
                return Err(format!(
                    "project scene directory differs only by case from existing path: {}",
                    existing.display()
                ));
            }
            let metadata = std::fs::symlink_metadata(&existing)
                .map_err(|error| format!("could not inspect {}: {error}", existing.display()))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "project scene directory is not a real directory: {}",
                    existing.display()
                ));
            }
            current = existing;
            continue;
        }
        if !create_missing {
            return Err(format!(
                "project scene directory does not exist: {}",
                current.join(component).display()
            ));
        }
        let created = current.join(component);
        match std::fs::create_dir(&created) {
            Ok(()) => current = created,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(&created).map_err(|inspect_error| {
                    format!("could not inspect {}: {inspect_error}", created.display())
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "project scene directory is not a real directory: {}",
                        created.display()
                    ));
                }
                current = created;
            }
            Err(error) => {
                return Err(format!(
                    "could not create project scene directory {}: {error}",
                    created.display()
                ));
            }
        }
    }
    Ok(())
}

fn portable_scene_path_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

fn portable_path_string(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| "project scene path contains non-UTF-8 text".to_string())?
                    .to_string(),
            ),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "project scene path is not project-relative: {}",
                    path.display()
                ));
            }
        }
    }
    if parts.is_empty() {
        Err("project scene path may not be empty".into())
    } else {
        Ok(parts.join("/"))
    }
}

fn portable_project_relative_path(project_root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(project_root).map_err(|_| {
        format!(
            "scene path is outside the project root and cannot be recorded: {}",
            path.display()
        )
    })?;
    portable_path_string(relative)
}

fn unix_nanos() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))
}

fn allocate_scene_trash_directory(
    project: &GameProject,
    scene_id: &str,
    deleted_unix_nanos: u128,
) -> Result<PathBuf, String> {
    let trash_root = project.root.join(".engine/trash/scenes");
    ensure_scene_directory_chain(&project.root, &trash_root, true)?;
    for attempt in 0..100usize {
        let candidate = trash_root.join(format!(
            "{deleted_unix_nanos}-{scene_id}-{}-{attempt}",
            std::process::id()
        ));
        if find_case_insensitive_entry(
            &trash_root,
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .expect("generated scene trash name is portable UTF-8"),
        )?
        .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not allocate a unique scene trash directory below {}",
        trash_root.display()
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SceneWriteMode {
    Create,
    Replace,
}

#[derive(Clone, Debug)]
struct SceneTransactionWrite {
    path: PathBuf,
    bytes: Vec<u8>,
    mode: SceneWriteMode,
}

impl SceneTransactionWrite {
    fn create(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: SceneWriteMode::Create,
        }
    }

    fn replace(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: SceneWriteMode::Replace,
        }
    }
}

#[derive(Clone, Debug)]
struct SceneFileSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

fn commit_scene_transaction(
    project_root: &Path,
    writes: Vec<SceneTransactionWrite>,
    deletes: Vec<PathBuf>,
    fail_after_mutation: Option<usize>,
) -> Result<(), String> {
    let mut snapshots = Vec::<SceneFileSnapshot>::new();
    let mut touched_paths = BTreeSet::new();
    for path in writes.iter().map(|write| &write.path).chain(deletes.iter()) {
        let relative = path.strip_prefix(project_root).map_err(|_| {
            format!(
                "scene transaction path escapes project root: {}",
                path.display()
            )
        })?;
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(format!(
                "scene transaction path is not normalized: {}",
                path.display()
            ));
        }
        let portable_key = portable_scene_path_key(relative);
        if !touched_paths.insert(portable_key) {
            return Err(format!(
                "scene transaction touches the same portable path more than once: {}",
                path.display()
            ));
        }
        ensure_no_scene_symlink_ancestors(project_root, path)?;
        let bytes = if path.exists() {
            let metadata = std::fs::symlink_metadata(path)
                .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "scene transaction target is not a regular file: {}",
                    path.display()
                ));
            }
            Some(
                std::fs::read(path)
                    .map_err(|error| format!("could not snapshot {}: {error}", path.display()))?,
            )
        } else {
            None
        };
        snapshots.push(SceneFileSnapshot {
            path: path.clone(),
            bytes,
        });
    }

    for write in &writes {
        let existed = snapshots
            .iter()
            .find(|snapshot| snapshot.path == write.path)
            .and_then(|snapshot| snapshot.bytes.as_ref())
            .is_some();
        match write.mode {
            SceneWriteMode::Create if existed => {
                return Err(format!(
                    "scene transaction will not overwrite existing file: {}",
                    write.path.display()
                ));
            }
            SceneWriteMode::Replace if !existed => {
                return Err(format!(
                    "scene transaction expected an existing file: {}",
                    write.path.display()
                ));
            }
            _ => {}
        }
    }
    for delete in &deletes {
        if snapshots
            .iter()
            .find(|snapshot| snapshot.path == *delete)
            .and_then(|snapshot| snapshot.bytes.as_ref())
            .is_none()
        {
            return Err(format!(
                "scene transaction cannot move missing file: {}",
                delete.display()
            ));
        }
    }

    let mut mutations = 0usize;
    let result = (|| {
        for write in &writes {
            match write.mode {
                SceneWriteMode::Create => write_bytes_create_new(&write.path, &write.bytes)?,
                SceneWriteMode::Replace => atomic_write_bytes(&write.path, &write.bytes)?,
            }
            mutations += 1;
            maybe_inject_scene_commit_failure(fail_after_mutation, mutations)?;
        }
        for delete in &deletes {
            std::fs::remove_file(delete)
                .map_err(|error| format!("could not move {}: {error}", delete.display()))?;
            mutations += 1;
            maybe_inject_scene_commit_failure(fail_after_mutation, mutations)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let rollback_errors = restore_scene_snapshots(&snapshots);
        return if rollback_errors.is_empty() {
            Err(error)
        } else {
            Err(format!(
                "{error}\nscene transaction rollback also failed:\n{}",
                rollback_errors.join("\n")
            ))
        };
    }
    Ok(())
}

fn maybe_inject_scene_commit_failure(
    fail_after_mutation: Option<usize>,
    mutations: usize,
) -> Result<(), String> {
    if fail_after_mutation == Some(mutations) {
        Err(format!(
            "injected scene transaction failure after mutation {mutations}"
        ))
    } else {
        Ok(())
    }
}

fn restore_scene_snapshots(snapshots: &[SceneFileSnapshot]) -> Vec<String> {
    let mut errors = Vec::new();
    for snapshot in snapshots.iter().rev() {
        let result = match &snapshot.bytes {
            Some(bytes) => atomic_write_bytes(&snapshot.path, bytes),
            None if snapshot.path.exists() => std::fs::remove_file(&snapshot.path)
                .map_err(|error| format!("could not remove {}: {error}", snapshot.path.display())),
            None => Ok(()),
        };
        if let Err(error) = result {
            errors.push(error);
        }
    }
    errors
}

fn write_bytes_create_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    if !parent.is_dir() {
        return Err(format!(
            "scene transaction parent directory does not exist: {}",
            parent.display()
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "could not create {} without overwriting an existing file: {error}",
                path.display()
            )
        })?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(format!("could not write {}: {error}", path.display()));
    }
    Ok(())
}

fn serialize_scene(scene: &Scene) -> Result<Vec<u8>, String> {
    ron::ser::to_string_pretty(scene, ron::ser::PrettyConfig::default())
        .map(String::into_bytes)
        .map_err(|error| format!("could not serialize scene '{}': {error}", scene.scene_id))
}

fn serialize_project_manifest(manifest: &ProjectManifest) -> Result<Vec<u8>, String> {
    manifest
        .validate()
        .map_err(|error| format!("invalid project manifest update: {error}"))?;
    let mut serialized = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("could not serialize project manifest: {error}"))?;
    serialized.push(b'\n');
    Ok(serialized)
}

fn atomic_write_project_manifest(manifest: &ProjectManifest, path: &Path) -> Result<(), String> {
    atomic_write_bytes(path, &serialize_project_manifest(manifest)?)
}

pub(crate) fn atomic_write_bytes(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "could not create a temporary file beside {}: {error}",
            path.display()
        )
    })?;
    temporary
        .write_all(contents)
        .map_err(|error| format!("could not write temporary {}: {error}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("could not flush temporary {}: {error}", path.display()))?;
    let persisted = temporary.persist(path).map_err(|error| {
        format!(
            "could not atomically replace {}: {}",
            path.display(),
            error.error
        )
    })?;
    persisted
        .sync_all()
        .map_err(|error| format!("could not flush {}: {error}", path.display()))?;
    Ok(())
}

fn check_project(path: &Path, report_path: Option<&Path>) -> Result<(), String> {
    let project = GameProject::load(path).map_err(|error| error.to_string())?;
    let input_map = super::project_input::load_project_input_map(&project)?;
    let input_binding_count = input_map
        .actions
        .iter()
        .map(|action| action.bindings.len())
        .sum::<usize>();
    let mut loaded_scenes = Vec::new();
    let mut scene_entities = BTreeMap::new();
    let mut total_entities = 0usize;
    let mut script_assembly = None;
    let mut script_components = 0usize;
    let mut strict_runtime = engine_core::EngineRuntime::new(engine_core::EngineConfig {
        application_name: format!("{}-project-check", project.manifest.name),
    });
    for (scene_id, scene_path) in project.scenes() {
        let scene = Scene::load_from_file(&scene_path).map_err(|error| {
            format!(
                "could not load project scene '{scene_id}' from {}: {error}",
                scene_path.display()
            )
        })?;
        let inspection = super::project_scripts::inspect_project_scripts(&project, &scene)
            .map_err(|error| format!("scene '{scene_id}' script validation failed: {error}"))?;
        script_assembly = inspection.assembly_id;
        script_components += inspection.component_count;

        let errors = validate_scene(&scene)
            .into_iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.severity,
                    DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
                )
            })
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect::<Vec<_>>();
        if !errors.is_empty() {
            return Err(format!(
                "scene '{scene_id}' validation failed:\n{}",
                errors.join("\n")
            ));
        }

        let mut ecs_scene = scene.clone();
        for entity in &mut ecs_scene.entities {
            entity.components.remove("engine.script");
        }
        strict_runtime
            .load_scene(ecs_scene)
            .map_err(|diagnostics| {
                let messages = diagnostics
                    .into_iter()
                    .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("scene '{scene_id}' could not be restored into an ECS World:\n{messages}")
            })?;
        total_entities += scene.entities.len();
        scene_entities.insert(scene_id.clone(), scene.entities.len());
        loaded_scenes.push((scene_id, scene_path, scene));
    }

    let manifest_paths = source_manifest_paths(&project.asset_source)?;
    if manifest_paths.is_empty() {
        return Err(format!(
            "no .manifest files found in {}",
            project.asset_source.display()
        ));
    }

    let mut asset_ids = BTreeSet::new();
    let mut portable_asset_ids = BTreeSet::new();
    let mut declared_asset_types = BTreeMap::new();
    let mut declared_asset_count = 0usize;
    for manifest_path in &manifest_paths {
        let content = std::fs::read_to_string(manifest_path)
            .map_err(|error| format!("could not read {}: {error}", manifest_path.display()))?;
        let manifest: SourceManifest = serde_json::from_str(&content)
            .map_err(|error| format!("invalid {}: {error}", manifest_path.display()))?;
        if manifest.schema_version != CURRENT_MANIFEST_VERSION {
            return Err(format!(
                "unsupported source manifest schema in {}",
                manifest_path.display()
            ));
        }
        for asset in manifest.assets {
            let portable_id = asset.id.id.to_ascii_lowercase();
            if asset.id.id.trim().is_empty() || !portable_asset_ids.insert(portable_id) {
                return Err(format!(
                    "empty, duplicate, or case-conflicting asset id '{}' in {}",
                    asset.id.id,
                    manifest_path.display()
                ));
            }
            asset_ids.insert(asset.id.id.clone());
            validate_source_path(&project.asset_source, &asset.source_path)?;
            validate_project_asset_type(&asset.asset_type, strict_runtime.asset_type_registry())
                .map_err(|error| {
                    format!(
                        "asset '{}' in {} cannot be cooked and loaded: {error}",
                        asset.id.id,
                        manifest_path.display()
                    )
                })?;
            declared_asset_types.insert(asset.id.id, asset.asset_type);
            declared_asset_count += 1;
        }
    }

    let cooked_report =
        validate_existing_cooked_assets(&project, &declared_asset_types, &mut strict_runtime)?;

    let builtins = BTreeSet::from(["mesh-cube".to_string(), "mat-default".to_string()]);
    let mut all_scene_dependencies = BTreeSet::new();
    for (scene_id, _, scene) in &loaded_scenes {
        let scene_dependencies = scene
            .collect_asset_dependencies()
            .into_iter()
            .chain(scene.dependencies.iter().cloned())
            .collect::<BTreeSet<_>>();
        let missing = scene_dependencies
            .iter()
            .filter(|dependency| {
                !asset_ids.contains(&dependency.id) && !builtins.contains(&dependency.id)
            })
            .map(|dependency| dependency.id.clone())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!(
                "scene '{scene_id}' references undeclared assets: {}",
                missing.join(", ")
            ));
        }
        all_scene_dependencies.extend(scene_dependencies);
    }

    let report = serde_json::to_string_pretty(&serde_json::json!({
        "schema": "ProjectCheckReport-v0",
        "project": project.manifest.name,
        "root": absolute_for_report(&project.root),
        "startup_scene_id": project.startup_scene_id(),
        "startup_scene": absolute_for_report(project.startup_scene_path()),
        "scenes": loaded_scenes.len(),
        "scene_entities": scene_entities,
        "entities": total_entities,
        "source_manifests": manifest_paths.len(),
        "declared_assets": declared_asset_count,
        "cooked_assets": cooked_report.discovered_assets,
        "loaded_render_assets": cooked_report.loaded_render_assets(),
        "loaded_extension_assets": cooked_report.loaded_extension_assets,
        "skipped_cooked_assets": cooked_report.skipped_assets,
        "scene_asset_dependencies": all_scene_dependencies.len(),
        "input_actions": input_map.actions.len(),
        "input_bindings": input_binding_count,
        "script_assembly": script_assembly,
        "script_components": script_components,
        "passed": true
    }))
    .expect("JSON value serialization cannot fail");
    emit_report(&report, report_path)?;
    Ok(())
}

fn validate_project_asset_type(
    asset_type: &AssetType,
    registry: &engine_scene::registry::AssetTypeRegistry,
) -> Result<(), String> {
    match asset_type {
        AssetType::Mesh
        | AssetType::Texture
        | AssetType::Shader
        | AssetType::Scene
        | AssetType::Material
        | AssetType::Logic => Ok(()),
        _ => {
            let type_id = registered_asset_type_id(asset_type).ok_or_else(|| {
                format!("asset type {asset_type:?} has no supported project pipeline mapping")
            })?;
            let extension = registry.get(type_id).ok_or_else(|| {
                format!("required runtime extension '{type_id}' is not registered")
            })?;
            if extension.cooker.is_none() {
                return Err(format!(
                    "runtime extension '{type_id}' does not provide a cooker"
                ));
            }
            if extension.loader.is_none() {
                return Err(format!(
                    "runtime extension '{type_id}' does not provide a loader"
                ));
            }
            Ok(())
        }
    }
}

fn validate_existing_cooked_assets(
    project: &GameProject,
    declared_asset_types: &BTreeMap<String, AssetType>,
    runtime: &mut engine_core::EngineRuntime,
) -> Result<engine_core::CookedAssetLoadReport, String> {
    if !project.cooked_assets.exists() {
        return Ok(engine_core::CookedAssetLoadReport::default());
    }
    if !project.cooked_assets.is_dir() {
        return Err(format!(
            "configured cooked asset path is not a directory: {}",
            project.cooked_assets.display()
        ));
    }

    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&project.cooked_assets).map_err(|error| {
        format!(
            "could not enumerate {}: {error}",
            project.cooked_assets.display()
        )
    })? {
        let path = entry
            .map_err(|error| {
                format!(
                    "could not enumerate {}: {error}",
                    project.cooked_assets.display()
                )
            })?
            .path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("cooked"))
        {
            paths.push(path);
        }
    }
    paths.sort();

    for path in &paths {
        let id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .ok_or_else(|| format!("cooked asset has no portable UTF-8 ID: {}", path.display()))?;
        let declared_type = declared_asset_types.get(id).ok_or_else(|| {
            format!(
                "cooked artifact '{}' is not declared by any source manifest",
                path.display()
            )
        })?;
        let artifact = read_cooked_artifact(path)
            .map_err(|error| format!("invalid cooked artifact {}: {error}", path.display()))?;
        if artifact.header.asset_kind != declared_type.kind_code() {
            return Err(format!(
                "cooked artifact '{}' has kind {}, but its manifest declares {:?} (kind {})",
                path.display(),
                artifact.header.asset_kind,
                declared_type,
                declared_type.kind_code()
            ));
        }
    }

    runtime
        .load_cooked_assets(&project.cooked_assets)
        .map_err(|diagnostics| {
            let details = diagnostics
                .into_iter()
                .map(|diagnostic| {
                    format!(
                        "{}: {}{}",
                        diagnostic.code,
                        diagnostic.message,
                        diagnostic
                            .path
                            .as_deref()
                            .map(|path| format!(" ({path})"))
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("cooked asset validation failed:\n{details}")
        })
}

fn import_project_asset(request: &ProjectImportRequest) -> Result<(), String> {
    let project = GameProject::load(&request.project).map_err(|error| error.to_string())?;
    validate_import_asset_id(&request.asset_id)?;
    let import_folder = normalize_existing_import_folder(&project.asset_source, &request.folder)?;
    let import_directory = project.asset_source.join(&import_folder);

    let source_file = std::fs::canonicalize(&request.source_file).map_err(|error| {
        format!(
            "could not resolve import source {}: {error}",
            request.source_file.display()
        )
    })?;
    if !source_file.is_file() {
        return Err(format!(
            "import source is not a regular file: {}",
            source_file.display()
        ));
    }
    let asset_type = resolve_import_asset_type(&source_file, request.asset_type.as_ref())?;
    let source_name = source_file
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            format!(
                "import source has no portable UTF-8 file name: {}",
                source_file.display()
            )
        })?;

    if let Some(conflict) = find_case_insensitive_entry(&import_directory, source_name)? {
        return Err(format!(
            "source asset target already exists and will not be overwritten: {}",
            conflict.display()
        ));
    }
    let relative_source = import_folder.join(source_name);
    let copied_source = project.asset_source.join(&relative_source);
    let cooked_name = format!("{}.cooked", request.asset_id);
    if let Some(conflict) = find_case_insensitive_entry(&project.cooked_assets, &cooked_name)? {
        return Err(format!(
            "cooked asset target already exists and will not be overwritten: {}",
            conflict.display()
        ));
    }
    let cooked_target = project.cooked_assets.join(&cooked_name);

    let (manifest_path, mut manifest) = load_import_manifest(&project, &request.asset_id)?;
    let original_manifest =
        if manifest_path.is_file() {
            Some(std::fs::read(&manifest_path).map_err(|error| {
                format!("could not back up {}: {error}", manifest_path.display())
            })?)
        } else if manifest_path.exists() {
            return Err(format!(
                "source manifest target is not a regular file: {}",
                manifest_path.display()
            ));
        } else {
            None
        };

    manifest.assets.push(SourceAssetEntry {
        id: AssetId::new(request.asset_id.clone()),
        asset_type: asset_type.clone(),
        source_path: relative_source
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        cook_rules: CookRules::default(),
    });
    manifest.assets.sort_by(|left, right| {
        left.id
            .id
            .to_ascii_lowercase()
            .cmp(&right.id.id.to_ascii_lowercase())
            .then_with(|| left.id.id.cmp(&right.id.id))
    });
    let mut manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("could not serialize source manifest: {error}"))?;
    manifest_json.push('\n');
    let staging_dir = import_staging_directory(&project)?;

    copy_file_create_new(&source_file, &copied_source)?;
    if let Err(error) = write_text(&manifest_path, &manifest_json) {
        return Err(rollback_import_failure(
            error,
            &manifest_path,
            original_manifest.as_deref(),
            &copied_source,
            None,
        ));
    }

    let mut graph = DependencyGraph::new();
    let runtime_builder = engine_core::EngineRuntime::builder(engine_core::EngineConfig {
        application_name: format!("{}-asset-import", project.manifest.name),
    });
    let cook_report = cook_orchestrate_checked_with_registry(
        &project.asset_source,
        &staging_dir,
        &mut graph,
        runtime_builder.asset_type_registry(),
    );
    let staged_cooked = staging_dir.join(&cooked_name);
    let cook_result = if !cook_report.is_success() {
        Err(cook_report_failure(&cook_report))
    } else if !cook_report
        .results
        .iter()
        .any(|result| result.success && result.asset_id == request.asset_id)
    {
        Err(format!(
            "cook succeeded without reporting imported asset '{}'",
            request.asset_id
        ))
    } else {
        read_cooked_artifact(&staged_cooked)
            .map_err(|error| {
                format!(
                    "imported asset did not produce a valid cooked artifact {}: {error}",
                    staged_cooked.display()
                )
            })
            .and_then(|artifact| {
                if artifact.header.asset_kind == asset_type.kind_code() {
                    Ok(())
                } else {
                    Err(format!(
                        "imported asset cooked as kind {}, expected {}",
                        artifact.header.asset_kind,
                        asset_type.kind_code()
                    ))
                }
            })
    };
    if let Err(error) = cook_result {
        remove_import_staging(&project, &staging_dir);
        return Err(rollback_import_failure(
            error,
            &manifest_path,
            original_manifest.as_deref(),
            &copied_source,
            None,
        ));
    }

    if let Err(error) = std::fs::create_dir_all(&project.cooked_assets) {
        remove_import_staging(&project, &staging_dir);
        return Err(rollback_import_failure(
            format!(
                "could not create cooked asset directory {}: {error}",
                project.cooked_assets.display()
            ),
            &manifest_path,
            original_manifest.as_deref(),
            &copied_source,
            None,
        ));
    }
    if let Err(error) = copy_file_create_new(&staged_cooked, &cooked_target) {
        remove_import_staging(&project, &staging_dir);
        return Err(rollback_import_failure(
            error,
            &manifest_path,
            original_manifest.as_deref(),
            &copied_source,
            None,
        ));
    }
    if let Err(error) = read_cooked_artifact(&cooked_target) {
        remove_import_staging(&project, &staging_dir);
        return Err(rollback_import_failure(
            format!(
                "installed cooked artifact {} failed validation: {error}",
                cooked_target.display()
            ),
            &manifest_path,
            original_manifest.as_deref(),
            &copied_source,
            Some(&cooked_target),
        ));
    }
    remove_import_staging(&project, &staging_dir);

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "ProjectImportReport-v0",
            "project": project.manifest.name,
            "asset_id": request.asset_id,
            "asset_type": import_asset_type_label(&asset_type),
            "source": absolute_for_report(&copied_source),
            "manifest": absolute_for_report(&manifest_path),
            "cooked": absolute_for_report(&cooked_target),
            "cooked_assets_checked": cook_report.succeeded_asset_count,
            "imported": true
        }))
        .expect("JSON value serialization cannot fail")
    );
    Ok(())
}

#[cfg(feature = "tooling-editor")]
pub(crate) fn import_project_asset_from(
    project: PathBuf,
    source_file: PathBuf,
    asset_id: String,
    asset_type: Option<AssetType>,
    folder: PathBuf,
) -> Result<(), String> {
    import_project_asset(&ProjectImportRequest {
        project,
        source_file,
        asset_id,
        asset_type,
        folder,
    })
}

fn load_import_manifest(
    project: &GameProject,
    requested_asset_id: &str,
) -> Result<(PathBuf, SourceManifest), String> {
    let paths = source_manifest_paths(&project.asset_source)?;
    let mut portable_ids = BTreeSet::new();
    let mut game_manifest = None;
    for path in paths {
        let content = std::fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let manifest: SourceManifest = serde_json::from_str(&content)
            .map_err(|error| format!("invalid {}: {error}", path.display()))?;
        if manifest.schema_version != CURRENT_MANIFEST_VERSION {
            return Err(format!(
                "unsupported source manifest schema in {}",
                path.display()
            ));
        }
        for asset in &manifest.assets {
            validate_import_asset_id(&asset.id.id)
                .map_err(|error| format!("invalid asset id in {}: {error}", path.display()))?;
            let portable_id = asset.id.id.to_ascii_lowercase();
            if !portable_ids.insert(portable_id) {
                return Err(format!(
                    "asset id '{}' is duplicated or differs only by case in source manifests",
                    asset.id.id
                ));
            }
            validate_source_path(&project.asset_source, &asset.source_path)?;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("game.manifest"))
        {
            if game_manifest.is_some() {
                return Err(
                    "multiple source manifests differ only by the name 'game.manifest'".into(),
                );
            }
            game_manifest = Some((path, manifest));
        }
    }

    if portable_ids.contains(&requested_asset_id.to_ascii_lowercase()) {
        return Err(format!(
            "asset id '{requested_asset_id}' already exists or differs only by case"
        ));
    }
    Ok(game_manifest.unwrap_or_else(|| {
        (
            project.asset_source.join("game.manifest"),
            SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: Vec::new(),
            },
        )
    }))
}

fn resolve_import_asset_type(
    source: &Path,
    requested: Option<&AssetType>,
) -> Result<AssetType, String> {
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("source file name is not UTF-8: {}", source.display()))?
        .to_ascii_lowercase();
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let inferred = if file_name.ends_with(".prefab.ron") {
        Some(AssetType::Prefab)
    } else if file_name.ends_with(".material.json") {
        Some(AssetType::Material)
    } else if matches!(extension.as_str(), "gltf" | "glb") {
        Some(AssetType::Mesh)
    } else if is_supported_texture_extension(&extension) {
        Some(AssetType::Texture)
    } else if matches!(extension.as_str(), "wav" | "mp3" | "ogg" | "flac") {
        Some(AssetType::Audio)
    } else if extension == "anim" {
        Some(AssetType::Animation)
    } else if extension == "skel" {
        Some(AssetType::Skeleton)
    } else if matches!(extension.as_str(), "navmesh" | "nav") {
        Some(AssetType::NavMesh)
    } else {
        None
    };

    let asset_type = requested.cloned().or(inferred.clone()).ok_or_else(|| {
        format!(
            "could not safely infer an import type for {}; use a supported extension or --type",
            source.display()
        )
    })?;
    let extension_supported = match asset_type {
        AssetType::Mesh => matches!(extension.as_str(), "gltf" | "glb"),
        AssetType::Texture => is_supported_texture_extension(&extension),
        AssetType::Material => extension == "json",
        AssetType::Audio => matches!(extension.as_str(), "wav" | "mp3" | "ogg" | "flac"),
        AssetType::Animation => extension == "anim",
        AssetType::Skeleton => extension == "skel",
        AssetType::NavMesh => matches!(extension.as_str(), "navmesh" | "nav"),
        AssetType::Prefab => file_name.ends_with(".prefab.ron"),
        _ => false,
    };
    if !extension_supported {
        return Err(format!(
            "source extension '.{extension}' is not supported for {} imports",
            import_asset_type_label(&asset_type)
        ));
    }
    if let (Some(requested), Some(inferred)) = (requested, inferred) {
        if requested != &inferred {
            return Err(format!(
                "requested import type {} conflicts with the source format inferred as {}",
                import_asset_type_label(requested),
                import_asset_type_label(&inferred)
            ));
        }
    }
    Ok(asset_type)
}

fn is_supported_texture_extension(extension: &str) -> bool {
    matches!(
        extension,
        "png"
            | "jpg"
            | "jpeg"
            | "bmp"
            | "gif"
            | "ico"
            | "tif"
            | "tiff"
            | "webp"
            | "pnm"
            | "pbm"
            | "pgm"
            | "ppm"
            | "pam"
            | "tga"
            | "hdr"
            | "qoi"
            | "exr"
    )
}

fn import_asset_type_label(asset_type: &AssetType) -> &'static str {
    match asset_type {
        AssetType::Mesh => "mesh",
        AssetType::Texture => "texture",
        AssetType::Material => "material",
        AssetType::Audio => "audio",
        AssetType::Animation => "animation",
        AssetType::Skeleton => "skeleton",
        AssetType::NavMesh => "navmesh",
        AssetType::Prefab => "prefab",
        _ => "unsupported",
    }
}

fn validate_import_asset_id(asset_id: &str) -> Result<(), String> {
    if asset_id.is_empty() || asset_id.len() > 128 {
        return Err("asset id must contain between 1 and 128 ASCII characters".into());
    }
    if !asset_id
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        return Err("asset id must start with an ASCII letter or digit".into());
    }
    if !asset_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(
            "asset id may contain only ASCII letters, digits, hyphens, underscores, and dots"
                .into(),
        );
    }
    let portable_stem = asset_id
        .split('.')
        .next()
        .unwrap_or(asset_id)
        .to_ascii_uppercase();
    if matches!(portable_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (portable_stem.len() == 4
            && (portable_stem.starts_with("COM") || portable_stem.starts_with("LPT"))
            && portable_stem.as_bytes()[3].is_ascii_digit()
            && portable_stem.as_bytes()[3] != b'0')
    {
        return Err(format!(
            "asset id '{asset_id}' is not portable because it uses a reserved file name"
        ));
    }
    Ok(())
}

fn normalize_existing_import_folder(source_root: &Path, folder: &Path) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    let mut current = source_root.to_path_buf();
    for component in folder.components() {
        let Component::Normal(component) = component else {
            return Err(format!(
                "asset import folder must be a portable project-relative path: {}",
                folder.display()
            ));
        };
        let requested = component
            .to_str()
            .ok_or_else(|| "asset import folder contains non-UTF-8 text".to_string())?;
        if requested.is_empty()
            || requested.ends_with([' ', '.'])
            || requested.chars().any(|character| {
                character.is_control()
                    || matches!(
                        character,
                        '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                    )
            })
        {
            return Err(format!(
                "asset import folder contains a non-portable component '{requested}'"
            ));
        }
        let stem = requested
            .split('.')
            .next()
            .unwrap_or(requested)
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit()
                && stem.as_bytes()[3] != b'0')
        {
            return Err(format!(
                "asset import folder uses reserved portable name '{requested}'"
            ));
        }
        let existing = find_case_insensitive_entry(&current, requested)?.ok_or_else(|| {
            format!(
                "asset import folder does not exist: {}",
                current.join(component).display()
            )
        })?;
        if existing.file_name() != Some(component) {
            return Err(format!(
                "asset import folder differs only by case from existing folder: {}",
                existing.display()
            ));
        }
        let metadata = std::fs::symlink_metadata(&existing)
            .map_err(|error| format!("could not inspect {}: {error}", existing.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "asset import folder is not a real directory: {}",
                existing.display()
            ));
        }
        normalized.push(component);
        current = existing;
    }
    Ok(normalized)
}

fn find_case_insensitive_entry(directory: &Path, name: &str) -> Result<Option<PathBuf>, String> {
    if !directory.exists() {
        return Ok(None);
    }
    if !directory.is_dir() {
        return Err(format!("expected a directory: {}", directory.display()));
    }
    for entry in std::fs::read_dir(directory)
        .map_err(|error| format!("could not enumerate {}: {error}", directory.display()))?
    {
        let entry = entry
            .map_err(|error| format!("could not enumerate {}: {error}", directory.display()))?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

fn copy_file_create_new(source: &Path, target: &Path) -> Result<(), String> {
    let mut input = std::fs::File::open(source)
        .map_err(|error| format!("could not open {}: {error}", source.display()))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| {
            format!(
                "could not create {} without overwriting an existing file: {error}",
                target.display()
            )
        })?;
    if let Err(error) = std::io::copy(&mut input, &mut output) {
        drop(output);
        let _ = std::fs::remove_file(target);
        return Err(format!(
            "could not copy {} to {}: {error}",
            source.display(),
            target.display()
        ));
    }
    Ok(())
}

fn import_staging_directory(project: &GameProject) -> Result<PathBuf, String> {
    let parent = project
        .cooked_assets
        .parent()
        .unwrap_or(project.root.as_path());
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    for attempt in 0..100_u32 {
        let candidate = parent.join(format!(
            ".asset-import-cook-{}-{unique}-{attempt}",
            std::process::id()
        ));
        if !candidate.exists() && candidate.starts_with(&project.root) {
            return Ok(candidate);
        }
    }
    Err("could not allocate a unique asset import staging directory".into())
}

fn remove_import_staging(project: &GameProject, staging: &Path) {
    if staging.starts_with(&project.root) && staging.exists() {
        let _ = std::fs::remove_dir_all(staging);
    }
}

fn cook_report_failure(report: &engine_asset::cook::CookReport) -> String {
    let details = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        })
        .take(8)
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("\n");
    if details.is_empty() {
        "project asset cooking failed during import".into()
    } else {
        format!("project asset cooking failed during import:\n{details}")
    }
}

fn rollback_import_failure(
    failure: String,
    manifest_path: &Path,
    original_manifest: Option<&[u8]>,
    copied_source: &Path,
    copied_cooked: Option<&Path>,
) -> String {
    let mut rollback_errors = Vec::new();
    let manifest_restore = match original_manifest {
        Some(content) => std::fs::write(manifest_path, content),
        None if manifest_path.exists() => std::fs::remove_file(manifest_path),
        None => Ok(()),
    };
    if let Err(error) = manifest_restore {
        rollback_errors.push(format!(
            "could not restore {}: {error}",
            manifest_path.display()
        ));
    }
    if copied_source.exists() {
        if let Err(error) = std::fs::remove_file(copied_source) {
            rollback_errors.push(format!(
                "could not remove copied source {}: {error}",
                copied_source.display()
            ));
        }
    }
    if let Some(cooked) = copied_cooked.filter(|path| path.exists()) {
        if let Err(error) = std::fs::remove_file(cooked) {
            rollback_errors.push(format!(
                "could not remove copied cooked artifact {}: {error}",
                cooked.display()
            ));
        }
    }
    if rollback_errors.is_empty() {
        failure
    } else {
        format!(
            "{failure}\nasset import rollback also failed:\n{}",
            rollback_errors.join("\n")
        )
    }
}

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
    let runtime_builder = engine_core::EngineRuntime::builder(engine_core::EngineConfig {
        application_name: format!("{}-asset-cook", project.manifest.name),
    });
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

fn replace_cooked_directory(
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

fn sync_project_script_api(path: &Path) -> Result<(), String> {
    let project = GameProject::load(path).map_err(|error| error.to_string())?;
    let report = super::project_scripts::sync_project_script_api(&project)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| format!("could not serialize script API sync report: {error}"))?
    );
    Ok(())
}

pub(crate) fn build_project_scripts(path: &Path, require_configured: bool) -> Result<(), String> {
    let project = GameProject::load(path).map_err(|error| error.to_string())?;
    match super::project_scripts::build_project_scripts(&project)? {
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

fn source_manifest_paths(source: &Path) -> Result<Vec<PathBuf>, String> {
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

fn validate_source_path(source_root: &Path, relative: &str) -> Result<(), String> {
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

fn write_text(path: &Path, content: &str) -> Result<(), String> {
    std::fs::write(path, content)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

fn emit_report(report: &str, path: Option<&Path>) -> Result<(), String> {
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

fn absolute_for_report(path: &Path) -> String {
    if path.is_absolute() {
        path.to_string_lossy().into_owned()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path).to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headless_project_run() {
        let request =
            parse_run_request(&["my-game".into(), "--headless".into(), "--frames=4".into()])
                .unwrap();
        assert_eq!(request.project, PathBuf::from("my-game"));
        assert!(request.headless);
        assert_eq!(request.frames, Some(4));
        assert_eq!(request.report, None);
    }

    #[test]
    fn rejects_zero_frames_and_extra_projects() {
        assert!(parse_run_request(&["game".into(), "--frames=0".into()]).is_err());
        assert!(parse_run_request(&["one".into(), "two".into()]).is_err());
    }

    #[test]
    fn parses_csharp_project_creation_option() {
        let (root, name, with_csharp) = parse_new_args(&[
            "managed-game".into(),
            "--name".into(),
            "Managed Game".into(),
            "--with-csharp".into(),
        ])
        .unwrap();
        assert_eq!(root, PathBuf::from("managed-game"));
        assert_eq!(name.as_deref(), Some("Managed Game"));
        assert!(with_csharp);
    }

    #[test]
    fn project_creation_installs_a_cataloged_basic_scene() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("basic-scene-project");

        create_project(&root, Some("Basic Scene Project"), false).unwrap();

        let project = GameProject::load(&root).unwrap();
        assert_eq!(project.startup_scene_id(), "main");
        assert_eq!(project.scenes().len(), 1);
        let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
        assert_eq!(scene.scene_id, "main");
        assert_eq!(scene.name, "Main");
        assert_eq!(scene.entities.len(), 3);
        assert!(scene.entities.iter().any(|entity| {
            entity.name.as_deref() == Some("Main Camera")
                && entity.components.contains_key("engine.camera")
        }));
        assert!(scene.entities.iter().any(|entity| {
            entity.name.as_deref() == Some("Cube")
                && entity.components.contains_key("engine.renderable")
        }));
        assert!(scene.entities.iter().any(|entity| {
            entity.name.as_deref() == Some("Directional Light")
                && entity.components.contains_key("engine.light")
        }));
    }

    #[test]
    fn parses_project_import_options() {
        let request = parse_import_args(&[
            "game".into(),
            "checker.ppm".into(),
            "--id=checker-main".into(),
            "--type".into(),
            "TeXtUrE".into(),
            "--folder".into(),
            "Textures/UI".into(),
        ])
        .unwrap();
        assert_eq!(request.project, PathBuf::from("game"));
        assert_eq!(request.source_file, PathBuf::from("checker.ppm"));
        assert_eq!(request.asset_id, "checker-main");
        assert_eq!(request.asset_type, Some(AssetType::Texture));
        assert_eq!(request.folder, PathBuf::from("Textures/UI"));

        let audio = parse_import_args(&[
            "game".into(),
            "ambient.wav".into(),
            "--id=ambient".into(),
            "--type=audio".into(),
        ])
        .unwrap();
        assert_eq!(audio.asset_type, Some(AssetType::Audio));
        assert!(audio.folder.as_os_str().is_empty());
    }

    #[test]
    fn rejects_invalid_project_import_arguments() {
        assert!(parse_import_args(&["game".into(), "asset.ppm".into()]).is_err());
        assert!(parse_import_args(&[
            "game".into(),
            "asset.ppm".into(),
            "--id".into(),
            "asset".into(),
            "--type=font".into(),
        ])
        .is_err());
        assert!(validate_import_asset_id("../escape").is_err());
        assert!(validate_import_asset_id("CON").is_err());
    }

    #[test]
    fn duplicate_project_scene_copies_authoring_data_and_catalogs_new_id() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("duplicate-scene");
        create_project(&root, Some("Duplicate Scene"), false).unwrap();
        let project = GameProject::load(&root).unwrap();
        let mut source = Scene::load_from_file(project.startup_scene_path()).unwrap();
        source.name = "Authored Level".to_string();
        source.entities[0].name = Some("Changed In Memory".to_string());

        let duplicate = duplicate_project_scene(&root, "level_copy", &source).unwrap();
        let copied = Scene::load_from_file(&duplicate).unwrap();
        let reloaded = GameProject::load(&root).unwrap();

        assert_eq!(copied.scene_id, "level_copy");
        assert_eq!(copied.name, "Authored Level");
        assert_eq!(
            copied.entities[0].name.as_deref(),
            Some("Changed In Memory")
        );
        assert_eq!(
            reloaded.scene_path("level_copy").as_deref(),
            Some(duplicate.as_path())
        );
    }

    #[test]
    fn editor_scene_creation_uses_a_safe_prefilled_subfolder() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("scene-subfolder");
        create_project(&root, Some("Scene Subfolder"), false).unwrap();

        let created =
            create_project_scene_in_folder(&root, "level_one", None, Path::new("levels/campaign"))
                .unwrap();
        assert_eq!(
            created,
            root.join("assets/scenes/levels/campaign/level_one.scene.ron")
        );
        assert_eq!(
            GameProject::load(&root)
                .unwrap()
                .scene_path("level_one")
                .as_deref(),
            Some(created.as_path())
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("game.project.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["scenes"]["level_one"],
            "assets/scenes/levels/campaign/level_one.scene.ron"
        );
        assert!(
            create_project_scene_in_folder(&root, "escape", None, Path::new("../outside"),)
                .is_err()
        );
        assert!(
            create_project_scene_in_folder(&root, "reserved", None, Path::new("CON"),).is_err()
        );
    }

    #[test]
    fn renames_project_scene_content_identity_path_and_startup_transactionally() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("rename-scene");
        create_project(&root, Some("Rename Scene"), false).unwrap();
        let old_path = create_project_scene(&root, "level_old", None).unwrap();
        let mut scene = Scene::load_from_file(&old_path).unwrap();
        scene.entities[0].name = Some("Authored Entity".into());
        scene.save_to_file(&old_path).unwrap();
        set_project_startup_scene(&root, "level_old").unwrap();

        let renamed_path = rename_project_scene(&root, "level_old", "level_new").unwrap();
        let renamed = Scene::load_from_file(&renamed_path).unwrap();
        let project = GameProject::load(&root).unwrap();

        assert!(!old_path.exists());
        assert_eq!(renamed_path, root.join("assets/scenes/level_new.scene.ron"));
        assert_eq!(renamed.scene_id, "level_new");
        assert_eq!(renamed.name, "level_new");
        assert_eq!(renamed.entities[0].name.as_deref(), Some("Authored Entity"));
        assert_eq!(project.startup_scene_id(), "level_new");
        assert_eq!(
            project.scene_path("level_new").as_deref(),
            Some(renamed_path.as_path())
        );
        assert!(project.scene_path("level_old").is_none());

        let custom_path =
            create_project_scene(&root, "authored_old", Some("Authored Display Name")).unwrap();
        let custom_renamed = rename_project_scene(&root, "authored_old", "authored_new").unwrap();
        assert!(!custom_path.exists());
        assert_eq!(
            Scene::load_from_file(&custom_renamed).unwrap().name,
            "Authored Display Name"
        );
    }

    #[test]
    fn scene_rename_rejects_portable_id_and_file_collisions_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("rename-collision");
        create_project(&root, Some("Rename Collision"), false).unwrap();
        let alpha = create_project_scene(&root, "alpha", None).unwrap();
        create_project_scene(&root, "beta", None).unwrap();
        let manifest_path = root.join("game.project.json");
        let original_manifest = std::fs::read(&manifest_path).unwrap();
        let original_alpha = std::fs::read(&alpha).unwrap();

        let error = rename_project_scene(&root, "alpha", "BETA").unwrap_err();
        assert!(error.contains("collides"));
        assert_eq!(std::fs::read(&manifest_path).unwrap(), original_manifest);
        assert_eq!(std::fs::read(&alpha).unwrap(), original_alpha);

        let orphan = root.join("assets/scenes/orphan.scene.ron");
        std::fs::copy(&alpha, &orphan).unwrap();
        let error = rename_project_scene(&root, "alpha", "orphan").unwrap_err();
        assert!(error.contains("already exists"));
        assert_eq!(std::fs::read(&manifest_path).unwrap(), original_manifest);
        assert_eq!(std::fs::read(&alpha).unwrap(), original_alpha);

        let error = rename_project_scene(&root, "alpha", "CON").unwrap_err();
        assert!(error.contains("reserved"));
        GameProject::load(&root).unwrap();
    }

    #[test]
    fn deleting_scene_requires_safe_startup_replacement_and_writes_recovery_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("delete-scene");
        create_project(&root, Some("Delete Scene"), false).unwrap();

        let error = delete_project_scene(&root, "main", None).unwrap_err();
        assert!(error.contains("retain at least one"));

        let replacement = create_project_scene(&root, "replacement", None).unwrap();
        let project = GameProject::load(&root).unwrap();
        let original_path = project.scene_path("main").unwrap();
        let original_scene = std::fs::read(&original_path).unwrap();
        let error = delete_project_scene(&root, "main", None).unwrap_err();
        assert!(error.contains("explicit replacement"));
        let error = delete_project_scene(&root, "main", Some("missing")).unwrap_err();
        assert!(error.contains("unknown project scene"));
        assert!(original_path.is_file());

        let deleted = delete_project_scene(&root, "main", Some("replacement")).unwrap();
        let reloaded = GameProject::load(&root).unwrap();
        let metadata: SceneTrashMetadata = serde_json::from_slice(
            &std::fs::read(&deleted.metadata_path).expect("read scene trash metadata"),
        )
        .expect("parse scene trash metadata");

        assert!(!original_path.exists());
        assert!(replacement.is_file());
        assert_eq!(reloaded.startup_scene_id(), "replacement");
        assert_eq!(reloaded.scenes().len(), 1);
        assert_eq!(deleted.scene_id, "main");
        assert_eq!(deleted.replacement_startup.as_deref(), Some("replacement"));
        assert_eq!(
            std::fs::read(deleted.trash_directory.join("scene.scene.ron")).unwrap(),
            original_scene
        );
        assert_eq!(metadata.schema, SCENE_TRASH_SCHEMA);
        assert_eq!(metadata.scene_id, "main");
        assert_eq!(metadata.original_scene_path, "assets/scenes/main.scene.ron");
        assert!(metadata.was_startup);
        assert_eq!(metadata.replacement_startup.as_deref(), Some("replacement"));

        let manifest: ProjectManifest =
            serde_json::from_slice(&std::fs::read(root.join("game.project.json")).unwrap())
                .unwrap();
        manifest.validate().unwrap();
    }

    #[test]
    fn scene_rename_and_delete_roll_back_every_touched_file() {
        let temp = tempfile::tempdir().unwrap();
        let rename_root = temp.path().join("rename-rollback");
        create_project(&rename_root, Some("Rename Rollback"), false).unwrap();
        let old_path = create_project_scene(&rename_root, "old", None).unwrap();
        set_project_startup_scene(&rename_root, "old").unwrap();
        let manifest_path = rename_root.join("game.project.json");
        let original_manifest = std::fs::read(&manifest_path).unwrap();
        let original_scene = std::fs::read(&old_path).unwrap();

        let error = rename_project_scene_impl(&rename_root, "old", "new", Some(3)).unwrap_err();
        assert!(error.contains("injected scene transaction failure"));
        assert_eq!(std::fs::read(&manifest_path).unwrap(), original_manifest);
        assert_eq!(std::fs::read(&old_path).unwrap(), original_scene);
        assert!(!rename_root.join("assets/scenes/new.scene.ron").exists());
        let rename_project = GameProject::load(&rename_root).unwrap();
        assert_eq!(rename_project.startup_scene_id(), "old");
        assert!(rename_project.scene_path("new").is_none());

        let delete_root = temp.path().join("delete-rollback");
        create_project(&delete_root, Some("Delete Rollback"), false).unwrap();
        create_project_scene(&delete_root, "replacement", None).unwrap();
        let delete_project = GameProject::load(&delete_root).unwrap();
        let main_path = delete_project.scene_path("main").unwrap();
        let delete_manifest_path = delete_root.join("game.project.json");
        let original_manifest = std::fs::read(&delete_manifest_path).unwrap();
        let original_scene = std::fs::read(&main_path).unwrap();

        let error = delete_project_scene_impl(&delete_root, "main", Some("replacement"), Some(4))
            .unwrap_err();
        assert!(error.contains("injected scene transaction failure"));
        assert_eq!(
            std::fs::read(&delete_manifest_path).unwrap(),
            original_manifest
        );
        assert_eq!(std::fs::read(&main_path).unwrap(), original_scene);
        let trash_root = delete_root.join(".engine/trash/scenes");
        assert_eq!(std::fs::read_dir(&trash_root).unwrap().count(), 0);
        let delete_project = GameProject::load(&delete_root).unwrap();
        assert_eq!(delete_project.startup_scene_id(), "main");
        assert!(delete_project.scene_path("main").is_some());
    }

    #[test]
    fn scene_mutations_refuse_an_existing_cross_process_lock() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("scene-lock");
        create_project(&root, Some("Scene Lock"), false).unwrap();
        let lock_directory = root.join(".engine/locks");
        std::fs::create_dir_all(&lock_directory).unwrap();
        let lock_path = lock_directory.join("scene-operations.lock");
        std::fs::write(&lock_path, "owned by another process\n").unwrap();

        let error = create_project_scene(&root, "blocked", None).unwrap_err();
        assert!(error.contains("another project scene operation is active"));
        assert!(!root.join("assets/scenes/blocked.scene.ron").exists());
        assert!(GameProject::load(&root)
            .unwrap()
            .scene_path("blocked")
            .is_none());

        std::fs::remove_file(lock_path).unwrap();
    }

    #[test]
    fn failed_project_cook_preserves_previous_directory_until_full_success() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("transactional-cook");
        create_project(&root, Some("Transactional Cook"), false).unwrap();
        let cooked = root.join("build/cooked");
        std::fs::create_dir_all(&cooked).unwrap();
        let previous = cooked.join("previous.cooked");
        std::fs::write(&previous, b"previous successful batch").unwrap();
        let broken_manifest = root.join("assets/source/broken.manifest");
        std::fs::write(&broken_manifest, b"{").unwrap();

        assert!(cook_project(&root).is_err());
        assert_eq!(
            std::fs::read(&previous).unwrap(),
            b"previous successful batch"
        );

        std::fs::remove_file(broken_manifest).unwrap();
        cook_project(&root).unwrap();
        assert!(cooked.is_dir());
        assert!(!previous.exists());
    }

    #[test]
    fn project_asset_type_validation_matches_enabled_runtime_extensions() {
        let builder = engine_core::EngineRuntime::builder(engine_core::EngineConfig::default());
        assert!(
            validate_project_asset_type(&AssetType::Mesh, builder.asset_type_registry()).is_ok()
        );
        assert!(
            validate_project_asset_type(&AssetType::Font, builder.asset_type_registry()).is_err()
        );
        #[cfg(feature = "runtime-subsystems")]
        assert!(
            validate_project_asset_type(&AssetType::Audio, builder.asset_type_registry()).is_ok()
        );
        #[cfg(not(feature = "runtime-subsystems"))]
        assert!(
            validate_project_asset_type(&AssetType::Audio, builder.asset_type_registry()).is_err()
        );
    }

    #[cfg(feature = "runtime-subsystems")]
    fn minimal_pcm_wav() -> Vec<u8> {
        let samples = [0i16; 80];
        let data_size = u32::try_from(samples.len() * 2).unwrap();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8_000u32.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn project_workflow_cooks_checks_and_loads_all_runtime_extension_assets() {
        use engine_animation::{AnimationClip, Joint, JointTransform, Skeleton};
        use engine_nav::NavMesh;
        use glam::Vec3;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extension-project");
        create_project(&root, Some("Extension Project"), false).unwrap();
        let source = root.join("assets/source");
        std::fs::write(source.join("ambient.wav"), minimal_pcm_wav()).unwrap();

        let skeleton = Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent_index: None,
                local_transform: JointTransform::IDENTITY,
            }],
            inverse_bind_matrices: vec![[
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]],
        };
        std::fs::write(
            source.join("hero.skel"),
            bincode::serialize(&skeleton).unwrap(),
        )
        .unwrap();
        let animation = AnimationClip {
            name: "idle".into(),
            duration: 1.0,
            channels: vec![],
            joint_indices: vec![],
        };
        std::fs::write(
            source.join("idle.anim"),
            bincode::serialize(&animation).unwrap(),
        )
        .unwrap();
        let mut navmesh = NavMesh::new();
        let a = navmesh.add_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = navmesh.add_vertex(Vec3::new(1.0, 0.0, 0.0));
        let c = navmesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
        navmesh.add_polygon(&[a, b, c], 1.0);
        navmesh.rebuild_bvh();
        std::fs::write(
            source.join("level.navmesh"),
            bincode::serialize(&navmesh).unwrap(),
        )
        .unwrap();

        let entry = |id: &str, asset_type: AssetType, source_path: &str| SourceAssetEntry {
            id: AssetId::new(id),
            asset_type,
            source_path: source_path.into(),
            cook_rules: CookRules::default(),
        };
        let manifest = SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: vec![
                entry("audio.ambient", AssetType::Audio, "ambient.wav"),
                entry("skeleton.hero", AssetType::Skeleton, "hero.skel"),
                entry("animation.idle", AssetType::Animation, "idle.anim"),
                entry("navmesh.level", AssetType::NavMesh, "level.navmesh"),
            ],
        };
        let mut manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
        manifest_json.push('\n');
        std::fs::write(source.join("game.manifest"), manifest_json).unwrap();

        check_project(&root, None).unwrap();
        cook_project(&root).unwrap();
        check_project(&root, None).unwrap();

        let project = GameProject::load(&root).unwrap();
        let mut runtime = engine_core::EngineRuntime::new(engine_core::EngineConfig::default());
        let report = crate::project_app::load_project_assets(&mut runtime, &project).unwrap();
        assert_eq!(report.loaded_extension_assets(), 4);
        assert!(runtime
            .extension_asset::<engine_audio::AudioClip>(
                "audio_clip",
                &AssetId::new("audio.ambient"),
            )
            .is_some());
        assert!(runtime
            .extension_asset::<Skeleton>("skeleton", &AssetId::new("skeleton.hero"))
            .is_some());
        assert!(runtime
            .extension_asset::<AnimationClip>("animation_clip", &AssetId::new("animation.idle"),)
            .is_some());
        assert!(runtime
            .extension_asset::<NavMesh>("navmesh", &AssetId::new("navmesh.level"))
            .is_some());
    }
}
