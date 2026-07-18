use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use engine_asset::cook::{read_cooked_artifact, AssetType, SourceManifest};
use engine_asset::project::GameProject;
use engine_scene::Scene;

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

#[test]
fn creates_checks_runs_and_cooks_a_game_project() {
    let root = unique_project_root();
    let check_report = root.join("build/project-check.json");
    let run_report = root.join("build/project-run.json");

    let output = run(&["project", "new", path_text(&root), "--name", "Test Game"]);
    assert_success(&output, "project new");
    assert!(root.join("config/input.actions.json").is_file());
    let output = run(&[
        "project",
        "check",
        path_text(&root),
        "--report",
        path_text(&check_report),
    ]);
    assert_success(&output, "project check");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&run_report),
    ]);
    assert_success(&output, "game");
    let output = run(&["project", "cook", path_text(&root)]);
    assert_success(&output, "project cook");

    let check: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&check_report).expect("read project check report"))
            .expect("parse project check report");
    let run: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&run_report).expect("read project run report"))
            .expect("parse project run report");
    assert_eq!(check["schema"], "ProjectCheckReport-v0");
    assert_eq!(check["passed"], true);
    assert_eq!(check["input_actions"], 6);
    assert_eq!(check["input_bindings"], 6);
    assert_eq!(run["schema"], "ProjectRunReport-v0");
    assert_eq!(run["passed"], true);
    assert_eq!(run["frames"], 3);
    assert!(run["total_draw_calls"].as_u64().unwrap_or(0) >= 3);
    assert!(root.join("build/cooked").is_dir());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn creates_lists_validates_and_runs_multiple_project_scenes() {
    let root = unique_project_root();
    let check_report = root.join("build/multi-scene-check.json");

    let output = run(&["project", "new", path_text(&root), "--name", "Scene Game"]);
    assert_success(&output, "project new for scene catalog");
    let output = run(&[
        "project",
        "scene",
        "new",
        path_text(&root),
        "level_two",
        "--name",
        "Level Two",
    ]);
    assert_success(&output, "project scene new");
    assert!(root.join("assets/scenes/level_two.scene.ron").is_file());

    let output = run(&["project", "scene", "list", path_text(&root)]);
    assert_success(&output, "project scene list");
    let list = stdout_json(&output);
    assert_eq!(list["startup_scene_id"], "main");
    assert_eq!(list["scenes"].as_array().unwrap().len(), 2);

    let output = run(&[
        "project",
        "check",
        path_text(&root),
        "--report",
        path_text(&check_report),
    ]);
    assert_success(&output, "multi-scene project check");
    let check: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&check_report).unwrap()).unwrap();
    assert_eq!(check["scenes"], 2);
    assert_eq!(check["scene_entities"]["main"], 3);
    assert_eq!(check["scene_entities"]["level_two"], 3);

    let output = run(&[
        "project",
        "scene",
        "set-startup",
        path_text(&root),
        "level_two",
    ]);
    assert_success(&output, "project scene set-startup");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("game.project.json")).expect("read scene catalog manifest"),
    )
    .unwrap();
    assert_eq!(manifest["startup_scene"], "level_two");
    assert_eq!(manifest["scenes"]["main"], "assets/scenes/main.scene.ron");
    assert_eq!(
        manifest["scenes"]["level_two"],
        "assets/scenes/level_two.scene.ron"
    );
    let output = run(&["game", path_text(&root), "--headless", "--frames", "1"]);
    assert_success(&output, "run catalog startup scene");

    // Check must cover every catalog entry, not only the active startup.
    std::fs::write(root.join("assets/scenes/main.scene.ron"), b"not ron")
        .expect("corrupt non-startup scene");
    let output = run(&["project", "check", path_text(&root)]);
    assert_failure(&output, "non-startup scene validation");
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(messages.contains("main"), "missing scene ID: {messages}");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn renames_and_recoverably_deletes_project_scenes_from_the_formal_workflow() {
    let root = unique_project_root();
    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Scene Mutation Game",
    ]);
    assert_success(&output, "project new for scene mutation");
    let output = run(&["project", "scene", "new", path_text(&root), "level_old"]);
    assert_success(&output, "project scene new for rename");
    let output = run(&[
        "project",
        "scene",
        "set-startup",
        path_text(&root),
        "level_old",
    ]);
    assert_success(&output, "project scene startup before rename");

    let output = run(&[
        "project",
        "scene",
        "rename",
        path_text(&root),
        "level_old",
        "level_new",
    ]);
    assert_success(&output, "project scene rename");
    let rename_report = stdout_json(&output);
    assert_eq!(rename_report["schema"], "ProjectSceneRenameReport-v0");
    assert_eq!(rename_report["old_scene_id"], "level_old");
    assert_eq!(rename_report["scene_id"], "level_new");
    assert_eq!(rename_report["renamed"], true);
    assert!(!root.join("assets/scenes/level_old.scene.ron").exists());
    let renamed_path = root.join("assets/scenes/level_new.scene.ron");
    let renamed = Scene::load_from_file(&renamed_path).expect("load renamed scene");
    assert_eq!(renamed.scene_id, "level_new");
    assert_eq!(renamed.name, "level_new");
    let project = GameProject::load(&root).expect("load project after scene rename");
    assert_eq!(project.startup_scene_id(), "level_new");
    assert!(project.scene_path("level_old").is_none());

    let output = run(&["project", "scene", "delete", path_text(&root), "level_new"]);
    assert_failure(&output, "startup scene delete without replacement");
    assert!(renamed_path.is_file());

    let output = run(&[
        "project",
        "scene",
        "delete",
        path_text(&root),
        "level_new",
        "--replacement-startup",
        "main",
    ]);
    assert_success(&output, "recoverable project scene delete");
    let delete_report = stdout_json(&output);
    assert_eq!(delete_report["schema"], "ProjectSceneDeleteReport-v0");
    assert_eq!(delete_report["scene_id"], "level_new");
    assert_eq!(delete_report["replacement_startup"], "main");
    assert_eq!(delete_report["recoverable"], true);
    let trash_directory = PathBuf::from(
        delete_report["trash_directory"]
            .as_str()
            .expect("scene trash directory path"),
    );
    let metadata_path = PathBuf::from(
        delete_report["metadata"]
            .as_str()
            .expect("scene trash metadata path"),
    );
    assert!(!renamed_path.exists());
    assert!(trash_directory.join("scene.scene.ron").is_file());
    assert!(metadata_path.is_file());
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(metadata_path).expect("read scene trash metadata"))
            .expect("parse scene trash metadata");
    assert_eq!(metadata["schema"], "EditorSceneTrash-v0");
    assert_eq!(metadata["scene_id"], "level_new");
    assert_eq!(
        metadata["original_scene_path"],
        "assets/scenes/level_new.scene.ron"
    );
    assert_eq!(metadata["was_startup"], true);
    assert_eq!(metadata["replacement_startup"], "main");
    let project = GameProject::load(&root).expect("load project after scene delete");
    assert_eq!(project.startup_scene_id(), "main");
    assert_eq!(project.scenes().len(), 1);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn imports_texture_and_rolls_back_every_rejected_or_failed_import() {
    let root = unique_project_root();
    let external = root.with_extension("import-sources");
    std::fs::create_dir_all(&external).expect("create external source directory");

    let output = run(&["project", "new", path_text(&root), "--name", "Import Game"]);
    assert_success(&output, "project new for import");

    let texture_source = external.join("checker.ppm");
    std::fs::create_dir_all(root.join("assets/source/Textures"))
        .expect("create texture import folder");
    std::fs::write(
        &texture_source,
        b"P3\n2 2\n255\n255 0 0  0 255 0\n0 0 255  255 255 255\n",
    )
    .expect("write PPM texture");
    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&texture_source),
        "--id",
        "imported-checker",
        "--folder",
        "Textures",
    ]);
    assert_success(&output, "project texture import");
    let import_report = String::from_utf8_lossy(&output.stdout);
    assert!(import_report.contains("\"schema\": \"ProjectImportReport-v0\""));
    assert!(import_report.contains("\"asset_type\": \"texture\""));
    assert!(import_report.contains("\"imported\": true"));

    let copied_source = root.join("assets/source/Textures/checker.ppm");
    let manifest_path = root.join("assets/source/game.manifest");
    let cooked_path = root.join("build/cooked/imported-checker.cooked");
    assert_eq!(
        std::fs::read(&copied_source).expect("read copied source"),
        std::fs::read(&texture_source).expect("read external source")
    );
    let manifest: SourceManifest = serde_json::from_slice(
        &std::fs::read(&manifest_path).expect("read updated source manifest"),
    )
    .expect("parse updated source manifest");
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(manifest.assets[0].id.id, "imported-checker");
    assert_eq!(manifest.assets[0].asset_type, AssetType::Texture);
    assert_eq!(manifest.assets[0].source_path, "Textures/checker.ppm");
    let cooked = read_cooked_artifact(&cooked_path).expect("validate imported cooked texture");
    assert_eq!(cooked.header.asset_kind, AssetType::Texture.kind_code());

    let output = run(&["project", "check", path_text(&root)]);
    assert_success(&output, "project check after import");
    let output = run(&["project", "cook", path_text(&root)]);
    assert_success(&output, "project cook after import");

    let stable_manifest = std::fs::read(&manifest_path).expect("snapshot source manifest");
    let stable_cooked = std::fs::read(&cooked_path).expect("snapshot cooked texture");

    let duplicate_source = external.join("duplicate.ppm");
    std::fs::copy(&texture_source, &duplicate_source).expect("write duplicate PPM source");
    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&duplicate_source),
        "--id",
        "IMPORTED-CHECKER",
    ]);
    assert_failure(&output, "case-insensitive duplicate ID import");
    assert!(!root.join("assets/source/duplicate.ppm").exists());
    assert_eq!(std::fs::read(&manifest_path).unwrap(), stable_manifest);
    assert_eq!(std::fs::read(&cooked_path).unwrap(), stable_cooked);

    let unsupported_source = external.join("unsupported.txt");
    std::fs::write(&unsupported_source, b"not an asset").expect("write unsupported source");
    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&unsupported_source),
        "--id",
        "unsupported-asset",
    ]);
    assert_failure(&output, "unsupported extension import");
    assert!(!root.join("assets/source/unsupported.txt").exists());
    assert_eq!(std::fs::read(&manifest_path).unwrap(), stable_manifest);

    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&texture_source),
        "--id",
        "second-checker",
        "--folder",
        "Textures",
    ]);
    assert_failure(&output, "source target conflict import");
    assert!(!root.join("build/cooked/second-checker.cooked").exists());
    assert_eq!(std::fs::read(&manifest_path).unwrap(), stable_manifest);

    let broken_source = external.join("broken.ppm");
    std::fs::write(&broken_source, b"this is not a PPM image").expect("write broken PPM");
    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&broken_source),
        "--id",
        "broken-texture",
        "--type",
        "texture",
    ]);
    assert_failure(&output, "failed cook import rollback");
    assert!(!root.join("assets/source/broken.ppm").exists());
    assert!(!root.join("build/cooked/broken-texture.cooked").exists());
    assert_eq!(std::fs::read(&manifest_path).unwrap(), stable_manifest);
    assert_eq!(std::fs::read(&cooked_path).unwrap(), stable_cooked);

    let output = run(&["project", "check", path_text(&root)]);
    assert_success(&output, "project check after failed imports");

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(external);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_scene_load_transitions_to_catalog_scene() {
    let root = unique_project_root();
    let run_report = root.join("build/csharp-scene-load-run.json");

    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Managed Scene Game",
        "--with-csharp",
    ]);
    assert_success(&output, "managed scene project new");
    let output = run(&[
        "project",
        "scene",
        "new",
        path_text(&root),
        "level_two",
        "--name",
        "Level Two",
    ]);
    assert_success(&output, "managed project scene new");

    let script_source = install_managed_workflow_behaviour(&root);
    let starter_source =
        std::fs::read_to_string(&script_source).expect("read managed workflow behaviour");
    let scene_loading_source = starter_source.replace(
        "    public void OnStart()\n    {\n    }",
        "    public void OnStart()\n    {\n        Scene.Load(\"level_two\");\n    }",
    );
    assert_ne!(
        scene_loading_source, starter_source,
        "workflow fixture OnStart method changed unexpectedly"
    );
    std::fs::write(&script_source, scene_loading_source).expect("write scene-loading script");

    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_success(&output, "managed scene script build");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&run_report),
    ]);
    assert_success(&output, "managed scene transition runtime");

    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&run_report).expect("read managed scene transition report"),
    )
    .expect("parse managed scene transition report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["final_scene_id"], "level_two");
    assert_eq!(report["scene_transitions"], 1);

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_script_api_sync_restores_engine_owned_contract() {
    let root = unique_project_root();
    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Managed Contract",
        "--with-csharp",
    ]);
    assert_success(&output, "managed contract project new");

    let legacy_source = root.join("scripts/GameScripts/EngineGameplay.cs");
    let source = root.join("build/script-sdk-source/EngineGameplay.cs");
    let contract = root.join("scripts/GameScripts/EngineGameplay.contract.json");
    let targets = root.join("scripts/GameScripts/EngineGameplay.targets");
    assert!(source.is_file());
    assert!(contract.is_file());
    assert!(targets.is_file());
    assert!(!legacy_source.exists());

    std::fs::write(&legacy_source, "// legacy generated gameplay API\n")
        .expect("restore legacy generated gameplay API");
    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_failure(&output, "stale managed contract build");
    assert!(String::from_utf8_lossy(&output.stderr).contains("sync-script-api"));

    let output = run(&["project", "sync-script-api", path_text(&root)]);
    assert_success(&output, "managed contract sync");
    let report = stdout_json(&output);
    assert_eq!(report["schema"], "ProjectScriptApiSyncReport-v0");
    assert_eq!(
        report["script_api"],
        engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA
    );
    assert_eq!(
        report["version"],
        engine_script_api::GAMEPLAY_SCRIPT_API_VERSION
    );
    assert_eq!(report["passed"], true);
    assert!(!legacy_source.exists());

    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&contract).expect("read generated gameplay contract"),
    )
    .expect("parse generated gameplay contract");
    assert_eq!(manifest["owner"], "engine");
    assert_eq!(manifest["sha256"], report["sha256"]);
    assert!(std::fs::read_to_string(&source)
        .expect("read synchronized gameplay API")
        .contains("public static class ScriptApiContract"));

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_project_builds_and_runs_managed_lifecycle() {
    let root = unique_project_root();
    let run_report = root.join("build/csharp-run.json");

    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Managed Game",
        "--with-csharp",
    ]);
    assert_success(&output, "managed project new");
    let script_source = install_managed_workflow_behaviour(&root);
    let output = run(&["project", "check", path_text(&root)]);
    assert_success(&output, "managed project check");
    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_success(&output, "managed project script build");
    let build_report = stdout_json(&output);
    assert_eq!(build_report["schema"], "ProjectScriptBuildReport-v0");
    assert_eq!(build_report["dependency_assemblies"], 1);
    assert!(build_report["sdk_assembly"]
        .as_str()
        .is_some_and(|path| path.ends_with("build/script-sdk/EngineGameplay.dll")));

    assert!(root.join("build/scripts/GameScripts.dll").is_file());
    assert!(root.join("build/scripts/EngineGameplay.dll").is_file());
    assert!(root.join("build/script-sdk/EngineGameplay.dll").is_file());
    assert!(!root.join("scripts/GameScripts/EngineGameplay.cs").exists());
    let host_name = if cfg!(windows) {
        "EngineScriptHost.exe"
    } else {
        "EngineScriptHost"
    };
    assert!(root.join("build/script-host").join(host_name).is_file());

    // A gameplay API contract error must fail closed and preserve the useful
    // managed exception instead of surfacing only TargetInvocationException.
    let valid_source =
        std::fs::read_to_string(&script_source).expect("read managed workflow behaviour");

    // Exercise the generated managed Entity API through the real process
    // host: query another World entity's snapshot, then use the explicit
    // target command to move the script entity.
    let entity_api_report = root.join("build/csharp-entity-api-run.json");
    let entity_api_source = valid_source.replace(
        "    public void OnStart()\n    {\n    }",
        "    public void OnStart()\n    {\n        var camera = Scene.GetEntity(\"camera-main\");\n        if (!Scene.Exists(camera.Id))\n            throw new InvalidOperationException(\"camera snapshot missing\");\n        if (Physics.Events.Count != 0)\n            throw new InvalidOperationException(\"unexpected startup physics event\");\n        var self = Scene.GetEntity(EntityId);\n        if (!self.HasTransform)\n            throw new InvalidOperationException(\"self Transform snapshot missing\");\n        var snapshot = self.Transform.Translation;\n        self.Transform.Translation = new Vector3(0.25f, snapshot.Y, snapshot.Z);\n    }",
    );
    assert_ne!(entity_api_source, valid_source);
    std::fs::write(&script_source, entity_api_source).expect("write entity API script");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "1",
        "--report",
        path_text(&entity_api_report),
    ]);
    assert_success(&output, "managed Entity snapshot and target Transform API");
    let entity_api_run: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&entity_api_report).expect("read managed Entity API report"),
    )
    .expect("parse managed Entity API report");
    assert!(
        entity_api_run["script_entity_translations"]["cube-01"][0]
            .as_f64()
            .expect("explicit target translation x")
            >= 0.24
    );

    // Exercise a real Rapier contact through GameLoop -> gameplay context ->
    // generated C# Physics.Events. The peer has no renderable, so the normal
    // visible-scene requirement remains unchanged.
    let scene_path = root.join("assets/scenes/main.scene.ron");
    let mut physics_scene = engine_scene::Scene::load_from_file(&scene_path)
        .expect("load managed scene for physics event fixture");
    let scene_without_physics_fixture = physics_scene.clone();
    let cube = physics_scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("script cube");
    let transform = cube
        .components
        .get("engine.transform")
        .cloned()
        .unwrap_or_else(|| engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::new(),
        });
    cube.components.insert(
        "engine.physics.rigid_body".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::new(),
        },
    );
    cube.components.insert(
        "engine.physics.collider".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::new(),
        },
    );
    physics_scene.entities.push(engine_scene::EntityRecord {
        persistent_id: "physics-peer".into(),
        parent: None,
        name: Some("Physics Peer".into()),
        enabled: true,
        components: std::collections::BTreeMap::from([
            ("engine.transform".into(), transform),
            (
                "engine.physics.rigid_body".into(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: std::collections::BTreeMap::from([(
                        "body_type".into(),
                        engine_serialize::Value::Enum("Static".into()),
                    )]),
                },
            ),
            (
                "engine.physics.collider".into(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: std::collections::BTreeMap::new(),
                },
            ),
        ]),
    });
    physics_scene
        .save_to_file(&scene_path)
        .expect("save physics event fixture");

    let physics_report = root.join("build/csharp-physics-event-run.json");
    let physics_source = valid_source.replace(
        "    public void OnStart()\n    {\n    }",
        "    public void OnStart()\n    {\n        var contact = Physics.Events.FirstOrDefault(evt =>\n            evt.Kind == \"collision_entered\" && evt.OtherEntityId == \"physics-peer\");\n        if (contact == null || contact.Other?.Id != \"physics-peer\")\n            throw new InvalidOperationException(\"physics contact snapshot missing\");\n        Transform.Translation = new Vector3(0.25f, Transform.Translation.Y, Transform.Translation.Z);\n    }",
    );
    assert_ne!(physics_source, valid_source);
    std::fs::write(&script_source, physics_source).expect("write physics event script");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "1",
        "--report",
        path_text(&physics_report),
    ]);
    assert_success(&output, "managed Physics.Events contact API");
    let physics_run: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&physics_report).expect("read managed physics event report"),
    )
    .expect("parse managed physics event report");
    assert!(
        physics_run["script_entity_translations"]["cube-01"][0]
            .as_f64()
            .expect("physics event translation x")
            >= 0.24
    );
    scene_without_physics_fixture
        .save_to_file(&scene_path)
        .expect("restore managed scene after physics event fixture");

    // Missing explicit targets cross the host successfully and are rejected
    // by the runtime with a stable, actionable diagnostic.
    let missing_destroy_source = valid_source.replace(
        "    public void OnStart()\n    {\n    }",
        "    public void OnStart()\n    {\n        Scene.Destroy(\"missing-entity\");\n    }",
    );
    assert_ne!(missing_destroy_source, valid_source);
    std::fs::write(&script_source, missing_destroy_source)
        .expect("write missing destroy target script");
    let output = run(&["game", path_text(&root), "--headless", "--frames", "1"]);
    assert_failure(&output, "managed missing destroy target diagnostic");
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        messages.contains("SCRIPT_DESTROY_TARGET_MISSING") && messages.contains("missing-entity"),
        "missing destroy target lost its diagnostic: {messages}"
    );

    let invalid_source = valid_source.replace(
        "Input.GetBool(\"jump\")",
        "Input.GetBool(\"missing-action\")",
    );
    assert_ne!(invalid_source, valid_source);
    std::fs::write(&script_source, invalid_source).expect("write invalid input action script");
    let output = run(&["game", path_text(&root), "--headless", "--frames", "1"]);
    assert_failure(&output, "managed gameplay API contract error");
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        messages.contains("Input action 'missing-action' is not configured"),
        "managed error lost its actionable cause: {messages}"
    );
    std::fs::write(&script_source, valid_source).expect("restore valid workflow script");
    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_success(&output, "restore managed project script build");

    // Simulate a runtime package: source assets are absent, so `game` must
    // consume the built DLL instead of entering the authoring auto-build path.
    std::fs::remove_dir_all(root.join("assets/source")).expect("remove authoring assets");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&run_report),
    ]);
    assert_success(&output, "managed game runtime");

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&run_report).expect("read managed run report"))
            .expect("parse managed run report");
    assert_eq!(report["passed"], true);
    // The runtime loads the engine SDK dependency before the game assembly.
    assert_eq!(report["script_assemblies"], 2);
    assert_eq!(report["script_instances"], 1);
    assert_eq!(report["script_started_instances"], 1);
    assert_eq!(report["script_update_count"], 3);
    let translation = report["script_entity_translations"]["cube-01"]
        .as_array()
        .expect("workflow script entity translation");
    assert!(
        translation[0].as_f64().expect("translation x") > 0.14,
        "workflow C# script must move its owning ECS Transform: {translation:?}"
    );
    assert_eq!(translation[1], 0.0);
    assert_eq!(translation[2], 0.0);
    assert_eq!(report["script_errors"], 0);

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(feature = "subsystem-scripting-csharp")]
const MANAGED_PREFAB_SPAWNER_BEHAVIOUR: &str = r#"using Engine;

namespace GameScripts;

public sealed class PrefabSpawnerBehaviour : EngineBehaviour
{
    public int UpdateCount = 0;

    public void OnCreate()
    {
    }

    public void OnStart()
    {
    }

    public void OnUpdate(float deltaTime)
    {
        UpdateCount += 1;
        if (UpdateCount == 1)
        {
            Scene.Spawn("prefab-enemy", new Vector3(1.0f, 2.0f, 3.0f));
        }
        if (UpdateCount == 2)
        {
            var spawned = Scene.FindEntity("prefab-enemy");
            if (spawned == null)
                throw new InvalidOperationException("spawned prefab root missing");
            if (Scene.FindEntity("prefab-enemy.hat") == null)
                throw new InvalidOperationException("spawned prefab child missing");
            var translation = spawned.Transform.Translation;
            if (translation.X != 1.0f || translation.Y != 2.0f || translation.Z != 3.0f)
                throw new InvalidOperationException("spawn translation override was not applied");
        }
    }
}

public sealed class SpawnedBehaviour : EngineBehaviour
{
    public void OnCreate()
    {
    }

    public void OnStart()
    {
    }

    public void OnUpdate(float deltaTime)
    {
    }
}
"#;

/// Install a complete prefab-spawn fixture into a `--with-csharp` project:
/// a spawner script on the scene cube, a cooked-prefab source with a scripted
/// root and a child entity, and the manifest declaration that `project check`
/// and `project cook` consume.
#[cfg(feature = "subsystem-scripting-csharp")]
fn install_prefab_spawn_fixture(root: &Path) {
    std::fs::write(
        root.join("scripts/GameScripts/PrefabSpawnerBehaviour.cs"),
        MANAGED_PREFAB_SPAWNER_BEHAVIOUR,
    )
    .expect("write managed prefab spawner behaviour");

    let scene_path = root.join("assets/scenes/main.scene.ron");
    let mut scene = Scene::load_from_file(&scene_path).expect("load spawner fixture scene");
    let entity = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("spawner fixture entity");
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
                    engine_serialize::Value::Str("GameScripts.PrefabSpawnerBehaviour".into()),
                ),
            ]),
        },
    );
    scene
        .save_to_file(&scene_path)
        .expect("save spawner fixture scene");

    let transform_record = |translation: [f32; 3]| engine_scene::ComponentRecord {
        schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields: std::collections::BTreeMap::from([
            (
                "translation".into(),
                engine_serialize::Value::Vec3(translation),
            ),
            (
                "rotation".into(),
                engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
            ),
            ("scale".into(), engine_serialize::Value::Vec3([1.0; 3])),
        ]),
    };
    let mut prefab = engine_scene::Prefab::new(engine_serialize::AssetId::new("prefab-enemy"));
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "enemy".into(),
        parent: None,
        name: Some("Enemy".into()),
        enabled: true,
        components: std::collections::BTreeMap::from([
            ("engine.transform".into(), transform_record([0.0; 3])),
            (
                "engine.renderable".into(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: std::collections::BTreeMap::from([
                        (
                            "mesh".into(),
                            engine_serialize::Value::Asset(engine_serialize::AssetId::new(
                                "mesh-cube",
                            )),
                        ),
                        (
                            "material".into(),
                            engine_serialize::Value::Asset(engine_serialize::AssetId::new(
                                "mat-default",
                            )),
                        ),
                    ]),
                },
            ),
            (
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
                            engine_serialize::Value::Str("GameScripts.SpawnedBehaviour".into()),
                        ),
                    ]),
                },
            ),
        ]),
    });
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "hat".into(),
        parent: Some("enemy".into()),
        name: Some("Hat".into()),
        enabled: true,
        components: std::collections::BTreeMap::from([(
            "engine.transform".into(),
            transform_record([0.0, 1.0, 0.0]),
        )]),
    });
    let prefab_dir = root.join("assets/source/Prefabs");
    std::fs::create_dir_all(&prefab_dir).expect("create prefab source dir");
    std::fs::write(
        prefab_dir.join("enemy.prefab.ron"),
        engine_scene::serialize_prefab_source(&prefab).expect("serialize prefab fixture"),
    )
    .expect("write prefab source");

    let manifest_path = root.join("assets/source/game.manifest");
    let mut manifest: SourceManifest = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).expect("read source manifest"),
    )
    .expect("parse source manifest");
    manifest.assets.push(engine_asset::cook::SourceAssetEntry {
        id: engine_serialize::AssetId::new("prefab-enemy"),
        asset_type: AssetType::Prefab,
        source_path: "Prefabs/enemy.prefab.ron".into(),
        cook_rules: engine_asset::cook::CookRules::default(),
    });
    let mut manifest_json = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
    manifest_json.push('\n');
    std::fs::write(&manifest_path, manifest_json).expect("write source manifest");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_scene_spawn_instantiates_cooked_prefabs_at_frame_boundaries() {
    let root = unique_project_root();
    let run_report = root.join("build/csharp-spawn-run.json");

    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Prefab Game",
        "--with-csharp",
    ]);
    assert_success(&output, "prefab project new");
    install_prefab_spawn_fixture(&root);
    let output = run(&["project", "check", path_text(&root)]);
    assert_success(&output, "prefab project check");
    let output = run(&["project", "cook", path_text(&root)]);
    assert_success(&output, "prefab project cook");
    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_success(&output, "prefab project script build");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&run_report),
    ]);
    assert_success(&output, "prefab spawn run");

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&run_report).expect("read prefab spawn report"))
            .expect("parse prefab spawn report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["script_errors"], 0);
    assert_eq!(
        report["script_instances"], 2,
        "spawner plus spawned prefab script instances"
    );
    let translation = report["script_entity_translations"]["prefab-enemy"]
        .as_array()
        .expect("spawned prefab root translation");
    assert_eq!(translation[0], 1.0);
    assert_eq!(translation[1], 2.0);
    assert_eq!(translation[2], 3.0);

    // Unknown prefab ids fail closed with an actionable diagnostic.
    let unknown_source = MANAGED_PREFAB_SPAWNER_BEHAVIOUR.replace(
        "Scene.Spawn(\"prefab-enemy\", new Vector3(1.0f, 2.0f, 3.0f));",
        "Scene.Spawn(\"prefab-missing\");",
    );
    assert_ne!(unknown_source, MANAGED_PREFAB_SPAWNER_BEHAVIOUR);
    std::fs::write(
        root.join("scripts/GameScripts/PrefabSpawnerBehaviour.cs"),
        unknown_source,
    )
    .expect("write unknown prefab script");
    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_success(&output, "unknown prefab script build");
    let output = run(&["game", path_text(&root), "--headless", "--frames", "1"]);
    assert_failure(&output, "unknown prefab diagnostic");
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        messages.contains("SCRIPT_PREFAB_UNKNOWN") && messages.contains("prefab-missing"),
        "unknown prefab lost its diagnostic: {messages}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(feature = "subsystem-scripting-csharp")]
const MANAGED_PHYSICS_PROBE_BEHAVIOUR: &str = r#"using Engine;

namespace GameScripts;

public sealed class PhysicsProbeBehaviour : EngineBehaviour
{
    private PhysicsQuery _hitQuery;
    private PhysicsQuery _missQuery;
    private PhysicsQuery _overlapQuery;
    public int UpdateCount = 0;

    public void OnCreate()
    {
    }

    public void OnStart()
    {
    }

    public void OnUpdate(float deltaTime)
    {
        UpdateCount += 1;
        if (UpdateCount == 1)
        {
            // Query handles never resolve on the frame that issued them.
            if (Physics.TryGetRaycastHit(_hitQuery, out _))
                throw new InvalidOperationException("query resolved on its issuing frame");
            _hitQuery = Physics.Raycast(
                new Vector3(0.0f, 5.0f, 0.0f),
                new Vector3(0.0f, -1.0f, 0.0f),
                10.0f);
            _missQuery = Physics.Raycast(
                new Vector3(0.0f, 5.0f, 0.0f),
                new Vector3(0.0f, 1.0f, 0.0f),
                10.0f);
            _overlapQuery = Physics.OverlapSphere(new Vector3(0.0f, 0.0f, 0.0f), 1.0f);
            return;
        }
        if (UpdateCount == 2)
        {
            if (!Physics.TryGetRaycastHit(_hitQuery, out var hit))
                throw new InvalidOperationException("raycast hit missing on the next frame");
            if (hit.EntityId != "cube-01" || hit.Entity?.Id != "cube-01")
                throw new InvalidOperationException("raycast hit the wrong entity");
            if (Math.Abs(hit.Distance - 4.5f) > 1e-3f)
                throw new InvalidOperationException($"unexpected raycast distance {hit.Distance}");
            if (Math.Abs(hit.Point.Y - 0.5f) > 1e-3f || Math.Abs(hit.Normal.Y - 1.0f) > 1e-3f)
                throw new InvalidOperationException("raycast hit geometry mismatch");
            if (Physics.TryGetRaycastHit(_missQuery, out _))
                throw new InvalidOperationException("miss raycast resolved as a hit");
            if (!Physics.TryGetOverlapResult(_overlapQuery, out var entityIds) ||
                !entityIds.Contains("cube-01"))
                throw new InvalidOperationException("overlap sphere missed cube-01");
        }
        if (UpdateCount >= 3)
        {
            // Results are frame-local and expire after their delivery frame.
            if (Physics.TryGetRaycastHit(_hitQuery, out _) ||
                Physics.TryGetOverlapResult(_overlapQuery, out _))
                throw new InvalidOperationException("frame-local query results must expire");
        }
    }
}
"#;

// Exercise the deferred physics query pipeline end to end: generated C#
// Physics.Raycast/OverlapSphere -> gameplay command -> process host -> native
// Rapier query -> next frame's gameplay context -> managed result lookup.
#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_physics_queries_round_trip_through_the_process_host() {
    let root = unique_project_root();
    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Physics Probe Game",
        "--with-csharp",
    ]);
    assert_success(&output, "physics query project new");

    let source = root.join("scripts/GameScripts/PhysicsProbeBehaviour.cs");
    std::fs::write(&source, MANAGED_PHYSICS_PROBE_BEHAVIOUR)
        .expect("write physics probe behaviour");

    // Attach the probe to cube-01 with an explicit origin transform, a static
    // rigid body, and the default unit collider so the ray and overlap
    // results are deterministic.
    let scene_path = root.join("assets/scenes/main.scene.ron");
    let mut scene = Scene::load_from_file(&scene_path).expect("load physics probe scene");
    let entity = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("physics probe fixture entity");
    entity.components.insert(
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
    entity.components.insert(
        "engine.physics.rigid_body".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([(
                "body_type".into(),
                engine_serialize::Value::Enum("Static".into()),
            )]),
        },
    );
    entity.components.insert(
        "engine.physics.collider".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::new(),
        },
    );
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
                    engine_serialize::Value::Str("GameScripts.PhysicsProbeBehaviour".into()),
                ),
            ]),
        },
    );
    scene
        .save_to_file(&scene_path)
        .expect("save physics probe scene");

    let report_path = root.join("csharp-physics-query-run.json");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&report_path),
    ]);
    assert_success(&output, "managed physics query round trip");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).expect("read physics query report"))
            .expect("parse physics query report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["script_errors"], 0);
    assert_eq!(report["script_update_count"], 3);

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(feature = "subsystem-scripting-csharp")]
const MANAGED_COMPONENT_PROBE_BEHAVIOUR: &str = r#"using Engine;

namespace GameScripts;

public sealed class ComponentProbeBehaviour : EngineBehaviour
{
    private ComponentQuery _audioQuery;
    private ComponentQuery _lightQuery;
    private ComponentQuery _cameraQuery;
    private ComponentQuery _missingQuery;
    private ComponentQuery _updatedAudioQuery;
    private ComponentQuery _updatedLightQuery;
    public int UpdateCount = 0;

    public void OnCreate()
    {
    }

    public void OnStart()
    {
    }

    public void OnUpdate(float deltaTime)
    {
        UpdateCount += 1;
        if (UpdateCount == 1)
        {
            // Query handles never resolve on the frame that issued them.
            if (Components.TryGet(_audioQuery, out _))
                throw new InvalidOperationException("query resolved on its issuing frame");
            var cube = Scene.GetEntity("cube-01");
            _audioQuery = cube.QueryComponent("engine.audio_source");
            _lightQuery = Components.Query("light-directional", "engine.light");
            _cameraQuery = Components.Query("camera-main", "engine.camera");
            _missingQuery = Components.Query("cube-01", "engine.light");
            return;
        }
        if (UpdateCount == 2)
        {
            if (!Components.TryGet(_audioQuery, out var audio))
                throw new InvalidOperationException("audio snapshot missing on the next frame");
            if (audio.EntityId != "cube-01" || audio.ComponentType != "engine.audio_source")
                throw new InvalidOperationException("audio snapshot identity mismatch");
            if (Math.Abs(audio.GetFloat("volume") - 0.8f) > 1e-6f)
                throw new InvalidOperationException("unexpected audio volume");
            if (audio.GetBool("playing"))
                throw new InvalidOperationException("audio source should not be playing");
            if (!audio.HasField("max_distance") || audio.HasField("clip_asset"))
                throw new InvalidOperationException("audio snapshot field coverage mismatch");
            if (!Components.TryGet(_lightQuery, out var light))
                throw new InvalidOperationException("light snapshot missing on the next frame");
            if (Math.Abs(light.GetFloat("intensity") - 2.5f) > 1e-6f)
                throw new InvalidOperationException("unexpected light intensity");
            if (light.GetEnum("kind") != "Directional")
                throw new InvalidOperationException("unexpected light kind");
            var lightColor = light.GetVector3("color");
            if (Math.Abs(lightColor.X - 1.0f) > 1e-6f || Math.Abs(lightColor.Y - 0.96f) > 1e-3f)
                throw new InvalidOperationException("unexpected light color");
            if (!Components.TryGet(_cameraQuery, out var camera))
                throw new InvalidOperationException("camera snapshot missing on the next frame");
            if (Math.Abs(camera.GetFloat("near") - 0.1f) > 1e-6f)
                throw new InvalidOperationException("unexpected camera near plane");
            if (camera.GetEnum("projection") != "Perspective")
                throw new InvalidOperationException("unexpected camera projection");
            var clearColor = camera.GetColor("clear_color");
            if (Math.Abs(clearColor.B - 0.06f) > 1e-3f || Math.Abs(clearColor.A - 1.0f) > 1e-6f)
                throw new InvalidOperationException("unexpected camera clear color");
            if (Components.TryGet(_missingQuery, out _) || !Components.IsMissing(_missingQuery))
                throw new InvalidOperationException("absent component must report IsMissing");

            // Merge writes: only the provided fields change on the target.
            var cube = Scene.GetEntity("cube-01");
            cube.SetComponentField("engine.audio_source", "volume", ComponentValue.FromFloat(0.25f));
            cube.SetComponentField("engine.audio_source", "playing", true);
            Scene.GetEntity("light-directional")
                .SetComponentField("engine.light", "intensity", 9.0f);
            _updatedAudioQuery = Components.Query("cube-01", "engine.audio_source");
            _updatedLightQuery = Components.Query("light-directional", "engine.light");
            return;
        }
        if (UpdateCount >= 3)
        {
            // Results are frame-local and expire after their delivery frame.
            if (Components.TryGet(_audioQuery, out _))
                throw new InvalidOperationException("frame-local query results must expire");
            if (!Components.TryGet(_updatedAudioQuery, out var audio))
                throw new InvalidOperationException("updated audio snapshot missing");
            if (Math.Abs(audio.GetFloat("volume") - 0.25f) > 1e-6f)
                throw new InvalidOperationException("audio volume write did not apply");
            if (!audio.GetBool("playing"))
                throw new InvalidOperationException("audio playing write did not apply");
            // Fields the write did not mention survive the merge.
            if (Math.Abs(audio.GetFloat("max_distance") - 15.0f) > 1e-6f)
                throw new InvalidOperationException("merge dropped unwritten fields");
            if (!Components.TryGet(_updatedLightQuery, out var light))
                throw new InvalidOperationException("updated light snapshot missing");
            if (Math.Abs(light.GetFloat("intensity") - 9.0f) > 1e-6f)
                throw new InvalidOperationException("light intensity write did not apply");
        }
    }
}
"#;

// Exercise deferred typed component access end to end: generated C#
// Components.Query/SetComponent -> gameplay command -> process host -> native
// component snapshot/merge -> next frame's gameplay context -> managed reads.
#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_component_access_round_trips_through_the_process_host() {
    let root = unique_project_root();
    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Component Probe Game",
        "--with-csharp",
    ]);
    assert_success(&output, "component probe project new");

    let source = root.join("scripts/GameScripts/ComponentProbeBehaviour.cs");
    std::fs::write(&source, MANAGED_COMPONENT_PROBE_BEHAVIOUR)
        .expect("write component probe behaviour");

    // Attach the probe to cube-01 with an authored audio source so snapshot
    // reads, missing-component detection, and merge writes are deterministic.
    let scene_path = root.join("assets/scenes/main.scene.ron");
    let mut scene = Scene::load_from_file(&scene_path).expect("load component probe scene");
    let entity = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("component probe fixture entity");
    entity.components.insert(
        "engine.audio_source".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                ("volume".into(), engine_serialize::Value::Float32(0.8)),
                ("playing".into(), engine_serialize::Value::Bool(false)),
                (
                    "max_distance".into(),
                    engine_serialize::Value::Float32(15.0),
                ),
            ]),
        },
    );
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
                    engine_serialize::Value::Str("GameScripts.ComponentProbeBehaviour".into()),
                ),
            ]),
        },
    );
    scene
        .save_to_file(&scene_path)
        .expect("save component probe scene");

    let report_path = root.join("csharp-component-run.json");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&report_path),
    ]);
    assert_success(&output, "managed component access round trip");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).expect("read component report"))
            .expect("parse component report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["script_errors"], 0);
    assert_eq!(report["script_update_count"], 3);

    // Unsupported component type keys fail closed with an actionable diagnostic.
    let unknown_source = MANAGED_COMPONENT_PROBE_BEHAVIOUR.replace(
        "_audioQuery = cube.QueryComponent(\"engine.audio_source\");",
        "_audioQuery = cube.QueryComponent(\"engine.nope\");",
    );
    assert_ne!(unknown_source, MANAGED_COMPONENT_PROBE_BEHAVIOUR);
    std::fs::write(&source, unknown_source).expect("write unknown component script");
    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_success(&output, "unknown component script build");
    let output = run(&["game", path_text(&root), "--headless", "--frames", "1"]);
    assert_failure(&output, "unknown component diagnostic");
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        messages.contains("SCRIPT_COMPONENT_UNKNOWN") && messages.contains("engine.nope"),
        "unknown component type lost its diagnostic: {messages}"
    );

    let _ = std::fs::remove_dir_all(root);
}
