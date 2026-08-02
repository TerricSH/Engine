// ── Validation ──────────────────────────────────────────────────────────

#[test]
fn validation_rejects_duplicate_persistent_ids_across_cells() {
    let startup = startup_scene();
    let level_a = cell_scene(
        "level-a",
        vec![cube_record("shared-cube", None, [0.0; 3], "mat-default")],
    );
    let level_b = cell_scene(
        "level-b",
        vec![cube_record("shared-cube", None, [0.0; 3], "mat-default")],
    );
    let scenes = BTreeMap::from([
        ("main".to_string(), &startup),
        ("level-a".to_string(), &level_a),
        ("level-b".to_string(), &level_b),
    ]);
    let partition = partition_of(&[("cell-a", "level-a"), ("cell-b", "level-b")]);
    let error = validate_partition_cell_scenes(&partition, "main", &scenes)
        .expect_err("duplicate ids across cells must fail");
    assert!(matches!(
        error,
        CellStreamError::DuplicatePersistentIdAcrossCells { .. }
    ));
}

#[test]
fn validation_accepts_script_components_in_cells() {
    let startup = startup_scene();
    let scripted = entity_record(
        "scripted",
        None,
        BTreeMap::from([(
            "engine.script".to_string(),
            component(BTreeMap::from([(
                "script".to_string(),
                Value::Str("Game.Player".to_string()),
            )])),
        )]),
    );
    let level_a = cell_scene("level-a", vec![scripted]);
    let scenes = BTreeMap::from([
        ("main".to_string(), &startup),
        ("level-a".to_string(), &level_a),
    ]);
    let partition = partition_of(&[("cell-a", "level-a")]);
    validate_partition_cell_scenes(&partition, "main", &scenes)
        .expect("engine.script metadata must be supported in streamed cells");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn streamed_cell_scripts_attach_and_destroy_with_the_cell() {
    let _guard = crate::tests::serial_ffi_world_test();
    use engine_script::MockHost;

    let scripted = entity_record(
        "scripted",
        None,
        BTreeMap::from([
            (
                "engine.transform".to_string(),
                transform_component([0.0; 3]),
            ),
            (
                "engine.script".to_string(),
                component(BTreeMap::from([
                    (
                        "assembly_id".to_string(),
                        Value::Str("game".to_string()),
                    ),
                    (
                        "class_name".to_string(),
                        Value::Str("Game.Streamed".to_string()),
                    ),
                ])),
            ),
        ]),
    );
    let fixture = stream_fixture(
        "script-lifecycle",
        vec![startup_scene(), cell_scene("level-a", vec![scripted])],
        vec![("cell-a", "level-a", origin_bounds())],
    );

    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    runtime.register_script_host(Box::new(MockHost::new()));
    runtime.set_script_host_name("mock");
    runtime
        .load_script_assembly("game", "mock", b"managed")
        .expect("load mock assembly");
    runtime
        .load_scene(fixture.scenes["main"].clone())
        .expect("load startup scene");
    let mut driver = CellStreamingDriver::new(
        &fixture.partition,
        &fixture.project,
        CellStreamingConfig::default(),
    )
    .expect("construct streaming driver");
    driver.rebaseline(&runtime);

    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
    assert_eq!(runtime.script_engine().managers()[0].instance_count(), 1);

    set_camera_position(&runtime, Vec3::new(20.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
    assert_eq!(runtime.script_engine().managers()[0].instance_count(), 0);

    set_camera_position(&runtime, Vec3::ZERO);
    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
    assert_eq!(
        runtime.script_engine().managers()[0].instance_count(),
        1,
        "re-entering a cell must attach exactly one fresh instance"
    );

    runtime.with_world_mut(|world| {
        let entity = world
            .entity_by_persistent_id("scripted")
            .expect("streamed script entity");
        world
            .get_mut::<Transform>(entity)
            .expect("streamed transform")
            .translation = Vec3::new(20.0, 0.0, 0.0);
    });
    set_camera_position(&runtime, Vec3::new(20.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
    assert!(driver.resident_ids().contains("scripted"));
    assert!(has_entity(&runtime, "scripted"));
    assert_eq!(
        runtime.script_engine().managers()[0].instance_count(),
        1,
        "resident entities must retain their script instance"
    );
}

#[test]
fn validation_rejects_cell_ids_overlapping_the_startup_scene() {
    let startup = startup_scene();
    let level_a = cell_scene(
        "level-a",
        vec![cube_record("cube-01", None, [0.0; 3], "mat-default")],
    );
    let scenes = BTreeMap::from([
        ("main".to_string(), &startup),
        ("level-a".to_string(), &level_a),
    ]);
    let partition = partition_of(&[("cell-a", "level-a")]);
    let error = validate_partition_cell_scenes(&partition, "main", &scenes)
        .expect_err("startup id overlap must fail");
    assert_eq!(
        error,
        CellStreamError::StartupSceneIdConflict {
            cell_id: "cell-a".to_string(),
            persistent_id: "cube-01".to_string(),
            startup_scene_id: "main".to_string(),
        }
    );

    // A cell that references the startup scene itself may share its ids:
    // the driver adopts the already-live entities at rebaseline.
    let scenes = BTreeMap::from([("main".to_string(), &startup)]);
    let partition = partition_of(&[("cell-main", "main")]);
    validate_partition_cell_scenes(&partition, "main", &scenes)
        .expect("startup-referencing cell is valid");
}

#[test]
fn validation_rejects_unknown_cell_scenes() {
    let startup = startup_scene();
    let scenes = BTreeMap::from([("main".to_string(), &startup)]);
    let partition = partition_of(&[("cell-ghost", "ghost")]);
    let error = validate_partition_cell_scenes(&partition, "main", &scenes)
        .expect_err("unknown cell scene must fail");
    assert_eq!(
        error,
        CellStreamError::UnknownCellScene {
            cell_id: "cell-ghost".to_string(),
            scene_id: "ghost".to_string(),
        }
    );
}

#[test]
fn driver_new_rejects_cells_referencing_unknown_scenes() {
    let fixture = stream_fixture("unknown-cell-scene", vec![startup_scene()], vec![]);
    let partition = partition_of(&[("cell-ghost", "ghost")]);
    let error =
        CellStreamingDriver::new(&partition, &fixture.project, CellStreamingConfig::default())
            .err()
            .expect("driver construction must fail");
    assert_eq!(
        error,
        CellStreamError::UnknownCellScene {
            cell_id: "cell-ghost".to_string(),
            scene_id: "ghost".to_string(),
        }
    );
}

#[test]
fn driver_new_reports_cell_scene_load_failures() {
    let mut fixture = stream_fixture("missing-scene-file", vec![startup_scene()], vec![]);
    // Catalog entry without a file on disk.
    fixture.project.manifest.scenes.insert(
        "ghost".to_string(),
        PathBuf::from("assets/scenes/ghost.scene.ron"),
    );
    let partition = partition_of(&[("cell-ghost", "ghost")]);
    let error =
        CellStreamingDriver::new(&partition, &fixture.project, CellStreamingConfig::default())
            .err()
            .expect("driver construction must fail");
    assert!(matches!(error, CellStreamError::CellSceneLoad { .. }));
}
