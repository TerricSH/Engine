#[test]
fn editor_creates_non_overwriting_csharp_behaviour_template() {
    let temporary = tempfile::tempdir().expect("temporary project");
    let root = temporary.path().join("script-template");
    crate::project_cli::create_project(&root, Some("Script Template"), true).unwrap();
    let project = GameProject::load(&root).unwrap();
    assert!(!root.join("scripts/GameScripts/Main.cs").exists());
    let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
    assert!(scene
        .entities
        .iter()
        .all(|entity| !entity.components.contains_key(SCRIPT_COMPONENT_TYPE)));

    let path = create_project_script_in(&project, Path::new(""), "PlayerController").unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(source.contains("class PlayerController : EngineBehaviour"));
    assert!(create_project_script_in(&project, Path::new(""), "PlayerController").is_err());
    assert!(create_project_script_in(&project, Path::new(""), "../Escape").is_err());

    let nested =
        create_project_script_in(&project, Path::new("gameplay/characters"), "EnemyAi").unwrap();
    assert_eq!(
        nested,
        root.join("scripts/GameScripts/gameplay/characters/EnemyAi.cs")
    );
    assert!(create_project_script_in(&project, Path::new("../outside"), "Escape").is_err());
    assert!(create_project_script_in(&project, Path::new("NUL"), "Reserved").is_err());
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_refresh_is_a_noop_for_projects_without_managed_scripts() {
    let root = PathBuf::from("unused-no-script-project");
    let project = GameProject {
        manifest: engine_asset::project::ProjectManifest::new("No Scripts"),
        manifest_path: root.join("game.project.json"),
        startup_scene: root.join("assets/scenes/main.scene.ron"),
        asset_source: root.join("assets-src"),
        cooked_assets: root.join("build/cooked"),
        script_project: None,
        script_assembly: None,
        input_actions: None,
        root,
    };
    let mut runtime = EngineRuntime::new(engine_core::EngineConfig::default());

    let refreshed = rebuild_and_reload_project_scripts(&mut runtime, &project)
        .expect("script-free projects should not invoke dotnet or mutate the runtime");

    assert_eq!(refreshed, PreparedScriptRuntime::default());
    assert_eq!(runtime.script_engine().host_count(), 0);
}

#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-ui"))]
#[test]
fn repeated_script_refresh_replaces_once_and_failed_build_keeps_last_good_runtime() {
    use engine_asset::project::ProjectManifest;
    use engine_scene::ComponentRecord;
    use engine_serialize::{SchemaVersion, Value};

    // Environment-coupled test: it drives the real dotnet toolchain.
    // Without the SDK this is a capability skip, not a failure (ENG-71).
    if !crate::test_capability::require_tool("dotnet") {
        return;
    }

    let temporary = tempfile::tempdir().expect("temporary script project");
    let root = temporary.path();
    std::fs::create_dir_all(root.join("assets/source")).expect("asset source directory");
    std::fs::create_dir_all(root.join("assets/scenes")).expect("scene directory");
    std::fs::create_dir_all(root.join("scripts/GameScripts")).expect("script source directory");

    let mut manifest = ProjectManifest::new("Transactional Scripts");
    manifest.input_actions = None;
    manifest.script_project = Some(PathBuf::from("scripts/GameScripts/GameScripts.csproj"));
    manifest.script_assembly = Some(PathBuf::from("build/scripts/GameScripts.dll"));

    let mut scene = engine_scene::sample_scene();
    let script_entity = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("sample script entity");
    script_entity.components.insert(
        SCRIPT_COMPONENT_TYPE.to_string(),
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                ("assembly_id".into(), Value::Str("GameScripts".into())),
                (
                    "class_name".into(),
                    Value::Str("ScriptReloadTests.ReloadFixtureBehaviour".into()),
                ),
            ]),
        },
    );
    scene
        .save_to_file(&root.join(&manifest.startup_scene))
        .expect("starter scene");
    std::fs::write(
        root.join("scripts/GameScripts/GameScripts.csproj"),
        STARTER_SCRIPT_PROJECT,
    )
    .expect("script project");
    write_generated_script_api(root, &root.join("scripts/GameScripts/GameScripts.csproj"))
        .expect("script API contract");
    let script_source = root.join("scripts/GameScripts/ReloadFixtureBehaviour.cs");
    std::fs::write(&script_source, TEST_RELOAD_SCRIPT_SOURCE).expect("script source");
    manifest.write_to_root(root).expect("project manifest");
    let project = GameProject::load(root).expect("load script project");

    let mut runtime = EngineRuntime::new(engine_core::EngineConfig::default());
    let first = rebuild_and_reload_project_scripts(&mut runtime, &project)
        .expect("initial isolated script runtime");
    assert!(first.assemblies >= 1);
    assert_eq!(runtime.script_engine().host_count(), 1);
    let verified = runtime.verified_script_classes();
    assert_eq!(
        verified.len(),
        1,
        "abstract and unrelated types must be filtered"
    );
    assert_eq!(verified[0].assembly_id, "GameScripts");
    assert_eq!(
        verified[0].class_name,
        "ScriptReloadTests.ReloadFixtureBehaviour"
    );

    let changed_source = TEST_RELOAD_SCRIPT_SOURCE
            .replace("public int UpdateCount = 0;", "public int UpdateCount = 7;")
            .replace(
                "        ElapsedSeconds = 0.0f;\n    }",
                "        ElapsedSeconds = 0.0f;\n        Scene.CreateEntity(\"managed-spawn\", new Vector3(4.0f, 5.0f, 6.0f));\n        var hud = UI.CreateCanvas(\"managed-hud\", 1280.0f, 720.0f, UIScaleMode.FitWidth);\n        hud.AddPanel(UILayout.Absolute(24.0f, 24.0f, 320.0f, 32.0f), new UIColor(20, 20, 20, 210), 10);\n        var music = hud.AddToggle(UILayout.Absolute(24.0f, 72.0f, 200.0f, 40.0f), \"Music\", false, new UIColor(0, 200, 80), new UIColor(80, 80, 80));\n        music.IsOn = true;\n        var hints = hud.AddCheckbox(UILayout.Absolute(24.0f, 120.0f, 200.0f, 40.0f), \"Hints\", false, UIColor.White);\n        hints.IsChecked = true;\n        var volume = hud.AddSlider(UILayout.Absolute(24.0f, 168.0f, 240.0f, 40.0f), \"Volume\", 0.25f, 0.0f, 1.0f);\n        volume.Value = 0.8f;\n    }",
            )
            .replace(
                "UpdateCount += 1;",
                "if (!Scene.Exists(\"managed-spawn\"))\n            throw new InvalidOperationException(\"deferred entity was not visible on the next frame\");\n        UpdateCount += 1;",
            )
            .replace(
                "        var translation = Transform.Translation;\n        Transform.Translation = new Vector3(\n            translation.X + Speed * deltaTime,\n            translation.Y,\n            translation.Z);",
                "        // This fixture's sample owner intentionally has no Transform.",
            );
    assert_ne!(changed_source, TEST_RELOAD_SCRIPT_SOURCE);
    std::fs::write(&script_source, changed_source).expect("changed script source");
    let second = rebuild_and_reload_project_scripts(&mut runtime, &project)
        .expect("second isolated script runtime");
    assert_eq!(second.assemblies, first.assemblies);
    assert_eq!(
        runtime.script_engine().host_count(),
        1,
        "reload must replace the old process host rather than register another one"
    );

    std::fs::write(&script_source, "this is not valid C#").expect("invalid script source");
    let error = rebuild_and_reload_project_scripts(&mut runtime, &project)
        .expect_err("invalid source must fail before runtime activation");
    assert!(error.contains("C# game script build failed"));
    assert_eq!(runtime.script_engine().host_count(), 1);
    assert_eq!(
        runtime.script_engine().managers()[0].assembly_count(),
        second.assemblies
    );

    runtime
        .load_scene(scene)
        .expect("last good process host should remain usable after failed refresh");
    fail_on_script_errors(&runtime, "post-refresh attachment")
        .expect("last good managed assembly should still instantiate");
    runtime
        .with_world(|world| {
            let hud = world
                .entity_by_persistent_id("managed-hud")
                .expect("managed UI.CreateCanvas must create a persistent Canvas entity");
            let snapshot = world.to_scene();
            let record = snapshot
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "managed-hud")
                .expect("managed Canvas must enter the scene snapshot");
            assert!(record.components.contains_key("engine.canvas"));
            assert!(world.is_alive(hud));
            let canvas = world
                .get::<engine_ui::Canvas>(hud)
                .expect("managed Canvas component");
            assert_eq!(canvas.scale_mode, engine_ui::ScaleMode::FitWidth);
            assert!(matches!(
                &canvas.get_element(engine_ui::ElementId(2)).unwrap().kind,
                engine_ui::UiElementKind::Toggle { is_on: true, .. }
            ));
            assert!(matches!(
                &canvas.get_element(engine_ui::ElementId(3)).unwrap().kind,
                engine_ui::UiElementKind::Checkbox { checked: true, .. }
            ));
            assert!(matches!(
                &canvas.get_element(engine_ui::ElementId(4)).unwrap().kind,
                engine_ui::UiElementKind::Slider { value, .. }
                    if (*value - 0.8).abs() < f32::EPSILON
            ));
        })
        .expect("managed Canvas must be applied to the active World");
    let input_actions = std::collections::BTreeMap::from([(
        "jump".into(),
        engine_script::GameplayInputValue::Bool(false),
    )]);
    runtime.tick_scripts_with_input(0.016, &input_actions);
    fail_on_script_errors(&runtime, "deferred entity next-frame snapshot")
        .expect("managed script must observe its deferred entity on the next frame");
    runtime
        .with_world(|world| {
            let entity = world
                .entity_by_persistent_id("managed-spawn")
                .expect("managed OnCreate must create a persistent entity");
            let transform = world
                .get::<engine_scene::components::Transform>(entity)
                .expect("managed-created entity must have Transform");
            assert_eq!(transform.translation.to_array(), [4.0, 5.0, 6.0]);
        })
        .expect("script lifecycle must keep an active World");
    drop(runtime);
}
