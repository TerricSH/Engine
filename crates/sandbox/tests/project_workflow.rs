use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use engine_asset::cook::{read_cooked_artifact, AssetType, SourceManifest};

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
    assert_eq!(check["scene_entities"]["main"], 2);
    assert_eq!(check["scene_entities"]["level_two"], 2);

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
fn imports_texture_and_rolls_back_every_rejected_or_failed_import() {
    let root = unique_project_root();
    let external = root.with_extension("import-sources");
    std::fs::create_dir_all(&external).expect("create external source directory");

    let output = run(&["project", "new", path_text(&root), "--name", "Import Game"]);
    assert_success(&output, "project new for import");

    let texture_source = external.join("checker.ppm");
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
    ]);
    assert_success(&output, "project texture import");
    let import_report = String::from_utf8_lossy(&output.stdout);
    assert!(import_report.contains("\"schema\": \"ProjectImportReport-v0\""));
    assert!(import_report.contains("\"asset_type\": \"texture\""));
    assert!(import_report.contains("\"imported\": true"));

    let copied_source = root.join("assets/source/checker.ppm");
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
    assert_eq!(manifest.assets[0].source_path, "checker.ppm");
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

#[test]
fn placeholder_workspace_command_is_not_a_successful_game_launch() {
    let output = run(&["workspace"]);
    assert_eq!(output.status.code(), Some(2));
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(messages.contains("placeholder"));
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

    let script_source = root.join("scripts/GameScripts/Main.cs");
    let starter_source = std::fs::read_to_string(&script_source).expect("read starter script");
    let scene_loading_source = starter_source.replace(
        "    public void OnStart()\n    {\n    }",
        "    public void OnStart()\n    {\n        Scene.Load(\"level_two\");\n    }",
    );
    assert_ne!(
        scene_loading_source, starter_source,
        "starter OnStart method changed unexpectedly"
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
    let output = run(&["project", "check", path_text(&root)]);
    assert_success(&output, "managed project check");
    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_success(&output, "managed project script build");

    assert!(root.join("build/scripts/GameScripts.dll").is_file());
    let host_name = if cfg!(windows) {
        "EngineScriptHost.exe"
    } else {
        "EngineScriptHost"
    };
    assert!(root.join("build/script-host").join(host_name).is_file());

    // A gameplay API contract error must fail closed and preserve the useful
    // managed exception instead of surfacing only TargetInvocationException.
    let script_source = root.join("scripts/GameScripts/Main.cs");
    let valid_source = std::fs::read_to_string(&script_source).expect("read starter script");

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
    std::fs::write(&script_source, valid_source).expect("restore valid starter script");
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
    assert_eq!(report["script_assemblies"], 1);
    assert_eq!(report["script_instances"], 1);
    assert_eq!(report["script_started_instances"], 1);
    assert_eq!(report["script_update_count"], 3);
    let translation = report["script_entity_translations"]["cube-01"]
        .as_array()
        .expect("starter script entity translation");
    assert!(
        translation[0].as_f64().expect("translation x") > 0.14,
        "starter C# script must move its owning ECS Transform: {translation:?}"
    );
    assert_eq!(translation[1], 0.0);
    assert_eq!(translation[2], 0.0);
    assert_eq!(report["script_errors"], 0);

    let _ = std::fs::remove_dir_all(root);
}
