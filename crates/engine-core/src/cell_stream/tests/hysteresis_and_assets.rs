// ── Hysteresis and budgets ──────────────────────────────────────────────

#[test]
fn cell_streams_in_and_out_with_hysteresis() {
    let fixture = stream_fixture(
        "hysteresis",
        vec![
            startup_scene(),
            cell_scene(
                "level-a",
                vec![cube_record("cube-a", None, [2.0, 0.0, 0.0], "mat-default")],
            ),
        ],
        vec![("cell-a", "level-a", origin_bounds())],
    );
    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

    // Camera at the origin: a zero-asset cell merges in a single tick.
    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
    assert!(report.world_changed());
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
    assert!(has_entity(&runtime, "cube-a"));

    // Hysteresis band: outside enter (10.0) but inside exit (11.5).
    set_camera_position(&runtime, Vec3::new(11.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert!(!report.world_changed());
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
    assert!(has_entity(&runtime, "cube-a"));

    // Outside exit: the cell unloads.
    set_camera_position(&runtime, Vec3::new(20.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));
    assert!(!has_entity(&runtime, "cube-a"));

    // Re-entering streams the cell back in.
    set_camera_position(&runtime, Vec3::ZERO);
    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
    assert!(has_entity(&runtime, "cube-a"));
    assert_eq!(driver.total_merges(), 2);
    assert_eq!(driver.total_unloads(), 1);
}

#[test]
fn custom_hysteresis_factors_are_honored() {
    let fixture = stream_fixture(
        "custom-factors",
        vec![
            startup_scene(),
            cell_scene(
                "level-a",
                vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
            ),
        ],
        vec![("cell-a", "level-a", origin_bounds())],
    );
    let config = CellStreamingConfig {
        enter_factor: 0.5,
        exit_factor: 0.6,
        ..CellStreamingConfig::default()
    };
    let (mut runtime, mut driver) = running_driver(&fixture, config);

    // Enter band is |x| <= 5.0: the camera at 7.0 stays out.
    set_camera_position(&runtime, Vec3::new(7.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert!(!report.world_changed());
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));

    // Inside enter: merges.
    set_camera_position(&runtime, Vec3::new(4.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);

    // Outside enter but inside exit (6.0): stays loaded.
    set_camera_position(&runtime, Vec3::new(5.5, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert!(!report.world_changed());
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));

    // Outside exit: unloads.
    set_camera_position(&runtime, Vec3::new(7.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
    assert!(!has_entity(&runtime, "cube-a"));
}

#[test]
fn invalid_hysteresis_config_is_rejected() {
    let fixture = stream_fixture("invalid-config", vec![startup_scene()], vec![]);
    for config in [
        CellStreamingConfig {
            enter_factor: 0.0,
            ..CellStreamingConfig::default()
        },
        CellStreamingConfig {
            enter_factor: f32::NAN,
            ..CellStreamingConfig::default()
        },
        CellStreamingConfig {
            enter_factor: 2.0,
            exit_factor: 1.5,
            ..CellStreamingConfig::default()
        },
    ] {
        let result = CellStreamingDriver::new(&fixture.partition, &fixture.project, config);
        assert!(
            matches!(result, Err(CellStreamError::InvalidConfig(_))),
            "config {config:?} must be rejected"
        );
    }

    // Zero budgets are clamped to one, not rejected.
    let clamped = CellStreamingConfig {
        max_merges_per_commit: 0,
        max_unloads_per_commit: 0,
        ..CellStreamingConfig::default()
    };
    assert!(
        CellStreamingDriver::new(&fixture.partition, &fixture.project, clamped)
            .err()
            .is_none()
    );
}

#[test]
fn merge_budget_commits_one_cell_per_tick() {
    let fixture = two_cell_fixture("merge-budget");
    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

    // Both cells contain the camera at the origin; the budget is one.
    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
    assert_eq!(driver.cell_state("cell-b"), Some(&CellState::Merging));
    assert!(has_entity(&runtime, "cube-a"));
    assert!(!has_entity(&runtime, "cube-b"));

    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-b".to_string()]);
    assert_eq!(driver.cell_state("cell-b"), Some(&CellState::Loaded));
    assert!(has_entity(&runtime, "cube-b"));
}

#[test]
fn unload_budget_and_camera_return_cancels_pending_unload() {
    let fixture = two_cell_fixture("unload-budget");
    let config = CellStreamingConfig {
        max_merges_per_commit: 2,
        max_unloads_per_commit: 1,
        ..CellStreamingConfig::default()
    };
    let (mut runtime, mut driver) = running_driver(&fixture, config);

    let report = driver.tick(&mut runtime);
    assert_eq!(
        report.merged_cells,
        vec!["cell-a".to_string(), "cell-b".to_string()]
    );

    // Both cells fall out of range; only one unload commits per tick.
    set_camera_position(&runtime, Vec3::new(100.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));
    assert_eq!(driver.cell_state("cell-b"), Some(&CellState::Unloading));
    assert!(!has_entity(&runtime, "cube-a"));
    assert!(has_entity(&runtime, "cube-b"));

    // The camera returns before cell-b's unload commits: the unload is
    // cancelled and cell-b was never destroyed; cell-a streams back in.
    set_camera_position(&runtime, Vec3::ZERO);
    let report = driver.tick(&mut runtime);
    assert!(report.unloaded_cells.is_empty());
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
    assert_eq!(driver.cell_state("cell-b"), Some(&CellState::Loaded));
    assert!(has_entity(&runtime, "cube-a"));
    assert!(has_entity(&runtime, "cube-b"));
    assert_eq!(driver.total_unloads(), 1);
}

// ── Asset streaming ─────────────────────────────────────────────────────

#[test]
fn merge_waits_for_background_asset_stream() {
    let fixture = stream_fixture(
        "asset-stream",
        vec![
            startup_scene(),
            cell_scene(
                "level-a",
                vec![
                    cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat.cell.a"),
                    cube_record("cube-b", None, [3.0, 0.0, 0.0], "mat.cell.b"),
                ],
            ),
        ],
        vec![("cell-a", "level-a", origin_bounds())],
    );
    std::fs::create_dir_all(&fixture.project.cooked_assets).expect("cooked dir");
    cook_test_material(&fixture.project.cooked_assets, "mat.cell.a", None);
    cook_test_material(&fixture.project.cooked_assets, "mat.cell.b", None);

    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
    runtime.set_cooked_asset_stream_budget(1);

    // First tick: assets are enqueued but cannot be committed yet.
    let report = driver.tick(&mut runtime);
    assert_eq!(report.enqueued_assets, 2);
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::LoadingAssets));
    assert!(!has_entity(&runtime, "cube-a"));

    // Pump ticks until the background stream commits both materials and
    // the merge lands (budget 1 per drain, so this takes a few ticks).
    // The sleep mirrors drain_until_idle: without it the spin loop can
    // exhaust all iterations before the decoder thread is scheduled.
    let mut merged = false;
    for _ in 0..200 {
        let report = driver.tick(&mut runtime);
        if report.merged_cells == vec!["cell-a".to_string()] {
            merged = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(merged, "cell merged once its assets streamed in");
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
    assert!(has_entity(&runtime, "cube-a"));
    assert!(has_entity(&runtime, "cube-b"));
    assert!(runtime
        .asset_registry()
        .contains(&AssetId::new("mat.cell.a")));
    assert!(runtime
        .asset_registry()
        .contains(&AssetId::new("mat.cell.b")));
}

#[test]
fn missing_cooked_asset_fails_the_cell_without_retry() {
    let fixture = stream_fixture(
        "missing-asset",
        vec![
            startup_scene(),
            cell_scene(
                "level-a",
                vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat.missing")],
            ),
        ],
        vec![("cell-a", "level-a", origin_bounds())],
    );
    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

    let report = driver.tick(&mut runtime);
    assert_eq!(report.failed_cells, vec!["cell-a".to_string()]);
    assert!(matches!(
        driver.cell_state("cell-a"),
        Some(CellState::Failed(_))
    ));
    assert!(!has_entity(&runtime, "cube-a"));
    assert!(runtime
        .diagnostics_collector()
        .all()
        .iter()
        .any(|diagnostic| diagnostic.code == "CELL_STREAM"));

    // A failed cell never retries on its own.
    let report = driver.tick(&mut runtime);
    assert!(report.failed_cells.is_empty());
    assert!(report.merged_cells.is_empty());
    assert_eq!(report.enqueued_assets, 0);
    assert!(matches!(
        driver.cell_state("cell-a"),
        Some(CellState::Failed(_))
    ));
}
