use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{
    cook_orchestrate_checked_with_registry, read_cooked_artifact, registered_asset_type_id,
    AssetType, CookRules, DependencyGraph, SourceAssetEntry, SourceManifest,
};
use engine_asset::project::{GameProject, ProjectManifest};
use engine_scene::{validate_scene, Scene};
use engine_serialize::{AssetId, DiagnosticSeverity};

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
        "build" => {
            let project_path = parse_single_project_path("build", &args[1..])?;
            cook_project(&project_path)?;
            build_project_scripts(&project_path, false)?;
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
           sandbox project scene set-startup <project> <scene-id>\n\
           sandbox project cook <project>\n\
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

fn scene_usage() -> String {
    "usage:\n  sandbox project scene list <project>\n  sandbox project scene new <project> <scene-id> [--name NAME]\n  sandbox project scene set-startup <project> <scene-id>".into()
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
        _ => Err(format!(
            "unsupported import type '{value}'; expected mesh, texture, material, audio, animation, skeleton, or navmesh"
        )),
    }
}

fn import_usage() -> String {
    "usage: sandbox project import <project> <source-file> --id <asset-id> [--type mesh|texture|material|audio|animation|skeleton|navmesh]".into()
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

fn create_project(
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

    let mut scene = engine_scene::sample_scene();
    scene.scene_id = "scene-main".into();
    scene.name = "Main".into();
    if with_csharp {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "assembly_id".into(),
            engine_serialize::Value::Str("GameScripts".into()),
        );
        fields.insert(
            "class_name".into(),
            engine_serialize::Value::Str("GameScripts.Main".into()),
        );
        fields.insert("Speed".into(), engine_serialize::Value::Float32(3.0));
        fields.insert("UpdateCount".into(), engine_serialize::Value::Int(0));
        fields.insert(
            "ElapsedSeconds".into(),
            engine_serialize::Value::Float32(0.0),
        );
        let target = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .ok_or_else(|| "starter scene has no script attachment entity".to_string())?;
        target.components.insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: std::collections::BTreeMap::from([
                    (
                        "translation".into(),
                        engine_serialize::Value::Vec3([0.0; 3]),
                    ),
                    (
                        "rotation".into(),
                        engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
                    ),
                    ("scale".into(), engine_serialize::Value::Vec3([1.0; 3])),
                ]),
            },
        );
        target.components.insert(
            "engine.script".into(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields,
            },
        );
    }
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
        let script_source = root.join("scripts/GameScripts/Main.cs");
        let script_api_source = root.join("scripts/GameScripts/EngineGameplay.cs");
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
        write_text(
            &script_source,
            super::project_scripts::STARTER_SCRIPT_SOURCE,
        )?;
        write_text(
            &script_api_source,
            super::project_scripts::STARTER_SCRIPT_API_SOURCE,
        )?;
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
    let project = GameProject::load(project_path).map_err(|error| error.to_string())?;
    if project
        .scenes()
        .iter()
        .any(|(existing, _)| existing.eq_ignore_ascii_case(scene_id))
    {
        return Err(format!("project scene ID already exists: '{scene_id}'"));
    }

    let relative_path = PathBuf::from(format!("assets/scenes/{scene_id}.scene.ron"));
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
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "could not create scene directory {}: {error}",
            parent.display()
        )
    })?;
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
    let mut scene = engine_scene::sample_scene();
    scene.scene_id = scene_id.to_string();
    scene.name = display_name.to_string();
    atomic_write_scene(&scene, &target)?;
    if let Err(error) = atomic_write_project_manifest(&manifest, &project.manifest_path) {
        return match std::fs::remove_file(&target) {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!(
                "{error}\nscene catalog rollback could not remove {}: {rollback_error}",
                target.display()
            )),
        };
    }
    Ok(target)
}

pub(crate) fn set_project_startup_scene(
    project_path: &Path,
    scene_id: &str,
) -> Result<PathBuf, String> {
    let project = GameProject::load(project_path).map_err(|error| error.to_string())?;
    let scene_path = project.scene_path(scene_id).ok_or_else(|| {
        format!(
            "unknown project scene '{scene_id}'; available scenes: {}",
            project
                .scenes()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let mut manifest = project.manifest.clone();
    manifest.scenes = manifest.scene_catalog();
    manifest.startup_scene = PathBuf::from(scene_id);
    atomic_write_project_manifest(&manifest, &project.manifest_path)?;
    Ok(scene_path)
}

fn atomic_write_scene(scene: &Scene, path: &Path) -> Result<(), String> {
    let serialized = ron::ser::to_string_pretty(scene, ron::ser::PrettyConfig::default())
        .map_err(|error| format!("could not serialize scene '{}': {error}", scene.scene_id))?;
    atomic_write_bytes(path, serialized.as_bytes())
}

fn atomic_write_project_manifest(manifest: &ProjectManifest, path: &Path) -> Result<(), String> {
    manifest
        .validate()
        .map_err(|error| format!("invalid project manifest update: {error}"))?;
    let mut serialized = serde_json::to_string_pretty(manifest)
        .map_err(|error| format!("could not serialize project manifest: {error}"))?;
    serialized.push('\n');
    atomic_write_bytes(path, serialized.as_bytes())
}

fn atomic_write_bytes(path: &Path, contents: &[u8]) -> Result<(), String> {
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
            .load_scene_to_world(ecs_scene)
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

    if let Some(conflict) = find_case_insensitive_entry(&project.asset_source, source_name)? {
        return Err(format!(
            "source asset target already exists and will not be overwritten: {}",
            conflict.display()
        ));
    }
    let copied_source = project.asset_source.join(source_name);
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
        source_path: source_name.to_string(),
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
    let inferred = if file_name.ends_with(".material.json") {
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
    fn parses_project_import_options() {
        let request = parse_import_args(&[
            "game".into(),
            "checker.ppm".into(),
            "--id=checker-main".into(),
            "--type".into(),
            "TeXtUrE".into(),
        ])
        .unwrap();
        assert_eq!(request.project, PathBuf::from("game"));
        assert_eq!(request.source_file, PathBuf::from("checker.ppm"));
        assert_eq!(request.asset_id, "checker-main");
        assert_eq!(request.asset_type, Some(AssetType::Texture));

        let audio = parse_import_args(&[
            "game".into(),
            "ambient.wav".into(),
            "--id=ambient".into(),
            "--type=audio".into(),
        ])
        .unwrap();
        assert_eq!(audio.asset_type, Some(AssetType::Audio));
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
