// ── Physics (gameplay feature) ──────────────────────────────────────────

#[cfg(feature = "subsystem-physics")]
#[test]
fn physics_bodies_follow_cell_merge_and_unload() {
    let rigid_body = component(BTreeMap::from([
        ("body_type".to_string(), Value::Enum("Dynamic".to_string())),
        ("mass".to_string(), Value::Float32(1.0)),
    ]));
    let ball = entity_record(
        "ball",
        None,
        BTreeMap::from([
            (
                "engine.transform".to_string(),
                transform_component([0.0, 5.0, 0.0]),
            ),
            (
                "engine.renderable".to_string(),
                renderable_component("mesh-cube", "mat-default"),
            ),
            ("engine.physics.rigid_body".to_string(), rigid_body),
        ]),
    );
    let fixture = stream_fixture(
        "physics",
        vec![startup_scene(), cell_scene("level-a", vec![ball])],
        vec![("cell-a", "level-a", origin_bounds())],
    );

    let mut game_loop = crate::game_loop::GameLoop::new(crate::EngineConfig::default());
    game_loop
        .load_scene(fixture.scenes["main"].clone())
        .expect("startup scene loads");
    assert_eq!(
        game_loop
            .physics
            .as_ref()
            .expect("physics initialised")
            .body_count(),
        0
    );

    let mut driver = match CellStreamingDriver::new(
        &fixture.partition,
        &fixture.project,
        CellStreamingConfig::default(),
    ) {
        Ok(driver) => driver,
        Err(error) => panic!("driver construction failed: {error}"),
    };
    driver.rebaseline(&game_loop.runtime);

    // Merge: the rigid body gains a physics body after the resync.
    let report = driver.tick(&mut game_loop.runtime);
    assert!(report.world_changed());
    game_loop.resync_physics_from_world();
    let ball_entity = game_loop
        .runtime
        .with_world(|world| world.entity_by_persistent_id("ball"))
        .flatten()
        .expect("ball merged");
    assert!(game_loop
        .physics
        .as_ref()
        .expect("physics")
        .has_body(ball_entity));

    // Unload: the incremental sync removes the body again.
    set_camera_position(&game_loop.runtime, Vec3::new(100.0, 0.0, 0.0));
    let report = driver.tick(&mut game_loop.runtime);
    assert!(report.world_changed());
    game_loop.resync_physics_from_world();
    assert_eq!(game_loop.physics.as_ref().expect("physics").body_count(), 0);
}

// ── World-origin shifts (ENG-01 Phase 2) ───────────────────────────────

/// Fixture with a far-field cell: authored bounds x ∈ [8000, 8020] and a
/// cube authored at logical x = 8005.
fn far_cell_fixture(name: &str) -> StreamFixture {
    stream_fixture(
        name,
        vec![
            startup_scene(),
            cell_scene(
                "level-a",
                vec![cube_record(
                    "cube-a",
                    None,
                    [8005.0, 0.0, 0.0],
                    "mat-default",
                )],
            ),
        ],
        vec![(
            "cell-a",
            "level-a",
            bounds([8010.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
        )],
    )
}

/// Move the camera to logical x = 8010, then shift the world origin by
/// 8 km: the camera lands at relative x = 10 with its logical position
/// unchanged.
fn shift_origin_to_camera(runtime: &EngineRuntime) {
    set_camera_position(runtime, Vec3::new(8010.0, 0.0, 0.0));
    runtime.with_world_mut(|world| {
        world.shift_world_origin([8000.0, 0.0, 0.0]);
    });
}

#[test]
fn streaming_decisions_use_logical_positions_with_a_non_zero_origin() {
    let fixture = far_cell_fixture("origin-logical");
    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
    shift_origin_to_camera(&runtime);

    // Logical camera position 8010 is inside the cell bounds even though
    // the stored (relative) camera position is only 10.
    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
    assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
    assert!(has_entity(&runtime, "cube-a"));

    // Moving the relative camera to 100 lifts the logical position to
    // 8100 — outside the exit band — so the cell unloads.
    set_camera_position(&runtime, Vec3::new(100.0, 0.0, 0.0));
    let report = driver.tick(&mut runtime);
    assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
    assert!(!has_entity(&runtime, "cube-a"));
}

#[test]
fn merged_cell_roots_are_rebased_into_origin_relative_space() {
    let fixture = far_cell_fixture("origin-rebase");
    let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
    shift_origin_to_camera(&runtime);

    let report = driver.tick(&mut runtime);
    assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);

    // The cube was authored at logical x = 8005; after the merge its
    // stored transform must be rebased by -origin, i.e. relative x ≈ 5,
    // so `world_origin + translation` still equals 8005.
    let translation = runtime
        .with_world(|world| {
            let cube = world.entity_by_persistent_id("cube-a")?;
            Some(world.get::<Transform>(cube)?.translation)
        })
        .flatten()
        .expect("merged cube transform");
    assert!(
        (translation.x - 5.0).abs() < 1e-3,
        "expected relative x ≈ 5, got {translation:?}"
    );
    let origin = runtime
        .with_world(|world| world.world_origin())
        .expect("world");
    assert_eq!(origin, [8000.0, 0.0, 0.0]);
}
