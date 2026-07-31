use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use engine_asset::cook::{read_cooked_artifact, AssetType, SourceManifest};
use engine_asset::project::GameProject;
use engine_scene::Scene;

mod common;

fn sandbox() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sandbox"))
}

fn run(arguments: &[&str]) -> Output {
    Command::new(sandbox())
        .args(arguments)
        .env("ENGINE_LOG_DIR", "off")
        .output()
        .expect("run sandbox")
}

fn unique_project_root() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sandbox-project-workflow-{}-{unique}",
        std::process::id()
    ))
}

fn path_text(path: &Path) -> &str {
    path.to_str().expect("test path is UTF-8")
}

fn assert_success(output: &Output, operation: &str) {
    assert!(
        output.status.success(),
        "{operation} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: &Output, operation: &str) {
    assert!(
        !output.status.success(),
        "{operation} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout_json(output: &Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_start = stdout
        .lines()
        .position(|line| line.trim_start().starts_with('{'))
        .expect("command stdout contains a JSON object");
    let json = stdout
        .lines()
        .skip(json_start)
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&json).expect("parse command JSON report")
}

#[cfg(feature = "subsystem-scripting-csharp")]
const MANAGED_WORKFLOW_BEHAVIOUR: &str = r#"using Engine;

namespace GameScripts;

public sealed class WorkflowBehaviour : EngineBehaviour
{
    public float Speed = 3.0f;
    public int UpdateCount = 0;
    public float ElapsedSeconds = 0.0f;
    public bool LastJump = false;

    public void OnCreate()
    {
        UpdateCount = 0;
        ElapsedSeconds = 0.0f;
    }

    public void OnStart()
    {
    }

    public void OnUpdate(float deltaTime)
    {
        UpdateCount += 1;
        ElapsedSeconds += deltaTime;
        LastJump = Input.GetBool("jump");
        var translation = Transform.Translation;
        Transform.Translation = new Vector3(
            translation.X + Speed * deltaTime,
            translation.Y,
            translation.Z);
    }
}
"#;

#[cfg(feature = "subsystem-scripting-csharp")]
fn install_managed_workflow_behaviour(root: &Path) -> PathBuf {
    let source = root.join("scripts/GameScripts/WorkflowBehaviour.cs");
    std::fs::write(&source, MANAGED_WORKFLOW_BEHAVIOUR).expect("write managed test behaviour");

    let scene_path = root.join("assets/scenes/main.scene.ron");
    let mut scene = Scene::load_from_file(&scene_path).expect("load managed fixture scene");
    let entity = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("managed fixture entity");
    entity
        .components
        .entry("engine.transform".into())
        .or_insert_with(|| engine_scene::ComponentRecord {
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
        });
    entity.components.insert(
        "engine.script".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                (
                    "assembly_id".into(),
                    engine_serialize::Value::Str("GameScripts".into()),
                ),
                (
                    "class_name".into(),
                    engine_serialize::Value::Str("GameScripts.WorkflowBehaviour".into()),
                ),
                ("Speed".into(), engine_serialize::Value::Float32(3.0)),
                ("UpdateCount".into(), engine_serialize::Value::Int(0)),
                (
                    "ElapsedSeconds".into(),
                    engine_serialize::Value::Float32(0.0),
                ),
            ]),
        },
    );
    scene
        .save_to_file(&scene_path)
        .expect("save managed fixture scene");
    source
}

include!("project_workflow/core_workflow.rs");
include!("project_workflow/scene_and_api.rs");
include!("project_workflow/managed_lifecycle.rs");
include!("project_workflow/prefab_spawn.rs");
include!("project_workflow/physics_queries.rs");
include!("project_workflow/component_access.rs");
