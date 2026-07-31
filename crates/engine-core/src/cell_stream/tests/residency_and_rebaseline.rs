// ── Residency ───────────────────────────────────────────────────────────

#[test]
fn runtime_created_entity_becomes_resident_and_survives_unload() {
    let fixture = stream_fixture(
        "resident-runtime",
        vec![
            startup_scene(),
            cell_scene(
                "level-a",
                vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
            ),
        ],
        vec![("cell-a", "level-a", origin_bounds())],
    );
    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
    driver.tick(&mut runtime);
    assert!(has_entity(&runtime, "cube-a"));

    // A script-style runtime creation: a persistent entity the driver
    // never merged.
    runtime.with_world_mut(|world| {
        world
            .create_persistent_entity("runtime-probe")
            .expect("create runtime entity");
    });

    set_camera_position(&runtime, Vec3::new(100.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert!(report
        .resident_ids_added
        .contains(&"runtime-probe".to_string()));
    assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
    assert!(!has_entity(&runtime, "cube-a"));
    assert!(has_entity(&runtime, "runtime-probe"));
    assert!(driver.resident_ids().contains("runtime-probe"));
}

#[test]
fn entity_moved_out_of_cell_becomes_resident_detached_and_is_not_remerged() {
    let fixture = stream_fixture(
        "resident-moved",
        vec![
            startup_scene(),
            cell_scene(
                "level-a",
                vec![
                    cube_record("cell-parent", None, [0.0, 0.0, 0.0], "mat-default"),
                    cube_record(
                        "cell-child",
                        Some("cell-parent"),
                        [4.0, 0.0, 0.0],
                        "mat-default",
                    ),
                ],
            ),
        ],
        vec![("cell-a", "level-a", origin_bounds())],
    );
    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
    driver.tick(&mut runtime);
    assert!(has_entity(&runtime, "cell-parent"));
    assert!(has_entity(&runtime, "cell-child"));

    // Move the child far outside the cell's exit bounds (its world
    // position is parent(0) + local(500)).
    runtime.with_world_mut(|world| {
        let child = world
            .entity_by_persistent_id("cell-child")
            .expect("child entity");
        world
            .get_mut::<Transform>(child)
            .expect("child transform")
            .translation = Vec3::new(500.0, 0.0, 0.0);
    });

    set_camera_position(&runtime, Vec3::new(100.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert!(report
        .resident_ids_added
        .contains(&"cell-child".to_string()));
    assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
    assert!(!has_entity(&runtime, "cell-parent"));
    assert!(has_entity(&runtime, "cell-child"));
    runtime.with_world(|world| {
        let child = world
            .entity_by_persistent_id("cell-child")
            .expect("child survives");
        let transform = world.get::<Transform>(child).expect("child transform");
        assert!(transform.parent.is_none(), "resident child detached");
        assert_eq!(transform.translation, Vec3::new(500.0, 0.0, 0.0));
        assert!(world.parent_persistent_id(child).is_none());
    });

    // Re-entering re-merges the cell without duplicating the resident
    // child: the parent returns, the child keeps its runtime state.
    set_camera_position(&runtime, Vec3::ZERO);
    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
    assert!(has_entity(&runtime, "cell-parent"));
    assert!(has_entity(&runtime, "cell-child"));
    runtime.with_world(|world| {
        let child = world
            .entity_by_persistent_id("cell-child")
            .expect("child still present");
        assert_eq!(
            world
                .get::<Transform>(child)
                .expect("child transform")
                .translation,
            Vec3::new(500.0, 0.0, 0.0)
        );
    });
}

// ── Rebaseline ──────────────────────────────────────────────────────────

#[test]
fn rebaseline_adopts_live_cells_and_resets_after_scene_replacement() {
    let fixture = stream_fixture(
        "rebaseline",
        vec![
            startup_scene(),
            cell_scene(
                "level-a",
                vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
            ),
        ],
        vec![
            (
                "cell-main",
                "main",
                bounds([1000.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
            ),
            ("cell-a", "level-a", origin_bounds()),
        ],
    );
    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

    // cell-main references the startup scene, so every one of its
    // entities is already live: it is adopted as Loaded regardless of
    // where its bounds sit.
    assert_eq!(driver.cell_state("cell-main"), Some(&CellState::Loaded));
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));
    assert_eq!(driver.loaded_cells(), vec!["cell-main".to_string()]);
    assert!(driver.base_ids().is_empty());

    // A runtime-created entity joins the resident set...
    runtime.with_world_mut(|world| {
        world
            .create_persistent_entity("runtime-probe")
            .expect("create runtime entity");
    });
    driver.tick(&mut runtime);
    assert!(driver.resident_ids().contains("runtime-probe"));

    // ...and a scene replacement wipes the baseline: the old cell content
    // is gone with the world, the resident set clears, and the cell whose
    // scene was just loaded wholesale is adopted instead.
    runtime
        .load_scene(fixture.scenes["level-a"].clone())
        .expect("replacement scene loads");
    driver.rebaseline(&runtime);
    assert_eq!(driver.cell_state("cell-main"), Some(&CellState::Unloaded));
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
    assert_eq!(driver.loaded_cells(), vec!["cell-a".to_string()]);
    assert!(driver.resident_ids().is_empty());
}

// ── Cameraless worlds ───────────────────────────────────────────────────

#[test]
fn tick_without_active_camera_is_a_noop() {
    let mut no_camera = sample_scene();
    no_camera.scene_id = "main".to_string();
    no_camera.entities = vec![cube_record("cube-01", None, [0.0, 0.0, 0.0], "mat-default")];
    no_camera.scene_settings.active_camera = None;
    no_camera.dependencies = vec![];
    let fixture = stream_fixture(
        "no-camera",
        vec![
            no_camera,
            cell_scene(
                "level-a",
                vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
            ),
        ],
        vec![("cell-a", "level-a", origin_bounds())],
    );
    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

    let report = driver.tick(&mut runtime);
    assert_eq!(report.camera, None);
    assert!(report.merged_cells.is_empty());
    assert!(!report.world_changed());
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));
    assert!(!has_entity(&runtime, "cube-a"));
}
