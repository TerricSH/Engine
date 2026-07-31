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
    if !common::require_tool("dotnet") {
        return;
    }
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
