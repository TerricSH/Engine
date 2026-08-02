#[test]
fn check_project_validates_prefab_sources_and_reports_count() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("prefab-project");
    create_project(&root, Some("Prefab Project"), false).unwrap();
    let prefab_dir = root.join("assets/source/Prefabs");
    std::fs::create_dir_all(&prefab_dir).unwrap();
    std::fs::write(
        prefab_dir.join("enemy.prefab.ron"),
        check_test_prefab_source("prefab-enemy", "mesh-cube", None),
    )
    .unwrap();
    write_check_test_manifest(
        &root,
        vec![check_test_entry(
            "prefab-enemy",
            AssetType::Prefab,
            "Prefabs/enemy.prefab.ron",
        )],
    );

    let report_path = root.join("build/check.json");
    check_project(&root, Some(&report_path)).unwrap();

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(report["prefabs"], 1);
}

fn write_world_partition(root: &Path, cells_json: &str) {
    let partition = format!(
        "{{ \"schema\": \"{}\", \"cells\": {{ {cells_json} }} }}\n",
        engine_asset::partition::WORLD_PARTITION_SCHEMA
    );
    std::fs::write(
        root.join(engine_asset::partition::WORLD_PARTITION_FILE_NAME),
        partition,
    )
    .unwrap();
}

fn configure_world_streaming(root: &Path, seamless_planetary: bool) {
    let manifest_path = root.join(engine_asset::project::GAME_PROJECT_FILE_NAME);
    let mut manifest: engine_asset::project::ProjectManifest = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).expect("read project manifest"),
    )
    .expect("parse project manifest");
    manifest.world_streaming.enabled = true;
    manifest.world_streaming.seamless_planetary = seamless_planetary;
    manifest
        .write_to_root(root)
        .expect("write streaming project manifest");
}

#[test]
fn check_project_validates_world_partition_and_reports_cell_count() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("partition-project");
    create_project(&root, Some("Partition Project"), false).unwrap();
    let level_path = create_project_scene(&root, "level_two", None).unwrap();
    // Cell scene entity IDs must be unique across cells and must not
    // overlap the startup scene (unless the cell references the startup
    // scene itself), so re-namespace the starter content of level_two.
    let mut level_two = Scene::load_from_file(&level_path).unwrap();
    for entity in &mut level_two.entities {
        entity.persistent_id = format!("{}-two", entity.persistent_id);
    }
    level_two.scene_settings.active_camera = level_two
        .scene_settings
        .active_camera
        .map(|camera| format!("{camera}-two"));
    level_two.save_to_file(&level_path).unwrap();
    write_world_partition(
            &root,
            "\"cell_main\": { \"scene\": \"main\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [64.0, 16.0, 64.0] } },\n\
             \"cell_two\": { \"scene\": \"level_two\", \"bounds\": { \"center\": [128.0, 0.0, 0.0], \"half_extents\": [32.0, 8.0, 32.0] } }",
        );

    let report_path = root.join("build/check.json");
    check_project(&root, Some(&report_path)).unwrap();

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(report["partition_cells"], 2);
    assert_eq!(report["world_streaming"]["enabled"], false);
    assert_eq!(
        report["world_streaming"]["partition_manifest_present"],
        true
    );
    assert_eq!(report["world_streaming"]["partition_cells"], 2);
}

#[test]
fn check_project_rejects_partition_cells_sharing_entity_ids() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("partition-duplicate-ids");
    create_project(&root, Some("Partition Duplicate IDs"), false).unwrap();
    // level_two keeps the starter entity IDs, so it shares them with main.
    create_project_scene(&root, "level_two", None).unwrap();
    write_world_partition(
            &root,
            "\"cell_main\": { \"scene\": \"main\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [64.0, 16.0, 64.0] } },\n\
             \"cell_two\": { \"scene\": \"level_two\", \"bounds\": { \"center\": [128.0, 0.0, 0.0], \"half_extents\": [32.0, 8.0, 32.0] } }",
        );

    let error = check_project(&root, None).unwrap_err();
    assert!(
        error.contains("cell scene entity ids must be unique across cells"),
        "{error}"
    );
}

#[test]
fn check_project_accepts_script_components_in_partition_cells() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("partition-script-cell");
    create_project(&root, Some("Partition Script Cell"), true).unwrap();
    let level_path = create_project_scene(&root, "level_two", None).unwrap();
    let mut level_two = Scene::load_from_file(&level_path).unwrap();
    for entity in &mut level_two.entities {
        entity.persistent_id = format!("{}-two", entity.persistent_id);
    }
    level_two.scene_settings.active_camera = level_two
        .scene_settings
        .active_camera
        .map(|camera| format!("{camera}-two"));
    level_two.entities.push(engine_scene::EntityRecord {
        persistent_id: "scripted-two".to_string(),
        parent: None,
        name: Some("Scripted".to_string()),
        enabled: true,
        components: BTreeMap::from([(
            "engine.script".to_string(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    (
                        "assembly_id".to_string(),
                        engine_serialize::Value::Str("GameScripts".to_string()),
                    ),
                    (
                        "class_name".to_string(),
                        engine_serialize::Value::Str("Game.Enemy".to_string()),
                    ),
                ]),
            },
        )]),
    });
    level_two.save_to_file(&level_path).unwrap();
    write_world_partition(
            &root,
            "\"cell_two\": { \"scene\": \"level_two\", \"bounds\": { \"center\": [128.0, 0.0, 0.0], \"half_extents\": [32.0, 8.0, 32.0] } }",
        );

    check_project(&root, None).expect("scripted partition cells are supported");
}

#[test]
fn check_project_reports_zero_partition_cells_without_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("no-partition-project");
    create_project(&root, Some("No Partition"), false).unwrap();

    let report_path = root.join("build/check.json");
    check_project(&root, Some(&report_path)).unwrap();

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(report["partition_cells"], 0);
    assert_eq!(report["world_streaming"]["enabled"], false);
    assert_eq!(
        report["world_streaming"]["partition_manifest_present"],
        false
    );
}

#[test]
fn check_project_rejects_enabled_streaming_without_partition_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("streaming-without-partition");
    create_project(&root, Some("Missing Partition"), false).unwrap();
    configure_world_streaming(&root, true);

    let error = check_project(&root, None).unwrap_err();
    assert!(
        error.contains("world_streaming.enabled requires world.partition.json"),
        "{error}"
    );
}

#[test]
fn check_project_reports_project_owned_seamless_streaming_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("seamless-streaming-report");
    create_project(&root, Some("Seamless Streaming"), false).unwrap();
    write_world_partition(
        &root,
        "\"cell_main\": { \"scene\": \"main\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [64.0, 16.0, 64.0] } }",
    );
    configure_world_streaming(&root, true);

    let report_path = root.join("build/check.json");
    check_project(&root, Some(&report_path)).unwrap();
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(report["world_streaming"]["enabled"], true);
    assert_eq!(report["world_streaming"]["seamless_planetary"], true);
    assert_eq!(
        report["world_streaming"]["partition_manifest_present"],
        true
    );
    assert_eq!(report["world_streaming"]["partition_cells"], 1);
}

#[test]
fn check_project_rejects_partition_with_unknown_scene_reference() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("partition-unknown-scene");
    create_project(&root, Some("Partition Unknown Scene"), false).unwrap();
    write_world_partition(
            &root,
            "\"cell_missing\": { \"scene\": \"missing_scene\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [16.0, 4.0, 16.0] } }",
        );

    let error = check_project(&root, None).unwrap_err();
    assert!(
            error.contains(
                "world partition cell \"cell_missing\" references unknown project scene \"missing_scene\""
            ),
            "{error}"
        );
}

#[test]
fn check_project_rejects_partition_with_invalid_bounds_or_schema() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("partition-invalid-bounds");
    create_project(&root, Some("Partition Invalid Bounds"), false).unwrap();
    write_world_partition(
            &root,
            "\"cell_bad\": { \"scene\": \"main\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [-1.0, 4.0, 16.0] } }",
        );

    let error = check_project(&root, None).unwrap_err();
    assert!(
        error.contains("world partition cell \"cell_bad\" has invalid bounds"),
        "{error}"
    );

    std::fs::write(
        root.join(engine_asset::partition::WORLD_PARTITION_FILE_NAME),
        "{ \"schema\": \"WorldPartition-v9\", \"cells\": {} }",
    )
    .unwrap();
    let error = check_project(&root, None).unwrap_err();
    assert!(
        error.contains("unsupported world partition schema: WorldPartition-v9"),
        "{error}"
    );
}

#[test]
fn check_project_rejects_prefab_with_undeclared_asset_reference() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("prefab-missing-dep");
    create_project(&root, Some("Prefab Missing Dependency"), false).unwrap();
    let prefab_dir = root.join("assets/source/Prefabs");
    std::fs::create_dir_all(&prefab_dir).unwrap();
    std::fs::write(
        prefab_dir.join("enemy.prefab.ron"),
        check_test_prefab_source("prefab-enemy", "mesh-missing", None),
    )
    .unwrap();
    write_check_test_manifest(
        &root,
        vec![check_test_entry(
            "prefab-enemy",
            AssetType::Prefab,
            "Prefabs/enemy.prefab.ron",
        )],
    );

    let error = check_project(&root, None).unwrap_err();
    assert!(
        error.contains("prefab 'prefab-enemy' references undeclared assets: mesh-missing"),
        "{error}"
    );
}

#[test]
fn check_project_rejects_undeclared_and_non_prefab_nested_references() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("prefab-nested");
    create_project(&root, Some("Prefab Nested"), false).unwrap();
    let prefab_dir = root.join("assets/source/Prefabs");
    std::fs::create_dir_all(&prefab_dir).unwrap();
    std::fs::write(
        prefab_dir.join("parent.prefab.ron"),
        check_test_prefab_source("prefab-parent", "mesh-cube", Some("prefab-child")),
    )
    .unwrap();
    std::fs::write(
        prefab_dir.join("child.prefab.ron"),
        check_test_prefab_source("prefab-child", "mesh-cube", None),
    )
    .unwrap();

    write_check_test_manifest(
        &root,
        vec![check_test_entry(
            "prefab-parent",
            AssetType::Prefab,
            "Prefabs/parent.prefab.ron",
        )],
    );
    let error = check_project(&root, None).unwrap_err();
    assert!(
        error.contains("prefab 'prefab-parent' references undeclared nested prefab 'prefab-child'"),
        "{error}"
    );

    // Declaring the child with the wrong asset type is also rejected: the
    // runtime resolves nested references against cooked prefabs only.
    std::fs::write(
        root.join("assets/source/checker.ppm"),
        b"P3\n1 1\n255\n0 0 0\n",
    )
    .unwrap();
    write_check_test_manifest(
        &root,
        vec![
            check_test_entry(
                "prefab-parent",
                AssetType::Prefab,
                "Prefabs/parent.prefab.ron",
            ),
            check_test_entry("prefab-child", AssetType::Texture, "checker.ppm"),
        ],
    );
    let error = check_project(&root, None).unwrap_err();
    assert!(
        error.contains("but that asset is declared as Texture"),
        "{error}"
    );
}

#[test]
fn check_project_rejects_nested_prefab_cycles() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("prefab-cycle");
    create_project(&root, Some("Prefab Cycle"), false).unwrap();
    let prefab_dir = root.join("assets/source/Prefabs");
    std::fs::create_dir_all(&prefab_dir).unwrap();
    std::fs::write(
        prefab_dir.join("a.prefab.ron"),
        check_test_prefab_source("prefab-a", "mesh-cube", Some("prefab-b")),
    )
    .unwrap();
    std::fs::write(
        prefab_dir.join("b.prefab.ron"),
        check_test_prefab_source("prefab-b", "mesh-cube", Some("prefab-a")),
    )
    .unwrap();
    write_check_test_manifest(
        &root,
        vec![
            check_test_entry("prefab-a", AssetType::Prefab, "Prefabs/a.prefab.ron"),
            check_test_entry("prefab-b", AssetType::Prefab, "Prefabs/b.prefab.ron"),
        ],
    );

    let error = check_project(&root, None).unwrap_err();
    assert!(error.contains("failed graph validation"), "{error}");
}
