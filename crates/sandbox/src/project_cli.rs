use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{
    cook_orchestrate_checked_with_registry, read_cooked_artifact, registered_asset_type_id,
    AssetType, CookRules, DependencyGraph, SourceAssetEntry, SourceManifest,
};
use engine_asset::project::{GameProject, ProjectManifest};
use engine_scene::{validate_scene, Scene};
use engine_serialize::{AssetId, DiagnosticSeverity};

const SCENE_TRASH_SCHEMA: &str = "EditorSceneTrash-v0";
static SCENE_OPERATION_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRunRequest {
    pub project: PathBuf,
    pub headless: bool,
    pub frames: Option<u64>,
    pub report: Option<PathBuf>,
    /// Internal hand-off from the editor after its build task has already
    /// compiled the managed project. Normal CLI runs keep this false.
    pub scripts_already_built: bool,
    /// Legacy CLI override for world-partition cell streaming
    /// (`--stream-cells`). Projects normally enable it in `world_streaming`;
    /// either path requires `world.partition.json` at the project root.
    pub stream_cells: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectAction {
    Complete,
    Run(ProjectRunRequest),
    Edit(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectImportRequest {
    project: PathBuf,
    source_file: PathBuf,
    asset_id: String,
    asset_type: Option<AssetType>,
    folder: PathBuf,
    /// glTF imports produce one complete mesh by default.
    merge_primitives: bool,
    /// Static glTF node transforms are preserved in the cooked vertex data.
    bake_node_transforms: Option<bool>,
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

mod help;
pub(crate) use help::*;
mod parser;
pub(crate) use parser::*;

mod asset_import;
mod cook_scripts;
mod project_check;
mod project_create;
mod scene_ops;

pub(crate) use asset_import::*;
pub(crate) use cook_scripts::*;
pub(crate) use project_check::*;
pub(crate) use project_create::*;
pub(crate) use scene_ops::*;

#[cfg(test)]
mod tests;
