#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_project_builds_and_runs_managed_lifecycle() {
    if !common::require_tool("dotnet") {
        return;
    }
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
