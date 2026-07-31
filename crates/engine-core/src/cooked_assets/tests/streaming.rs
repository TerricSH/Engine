#[test]
fn background_stream_installs_assets_at_the_frame_boundary() {
    let dir = cooked_case("stream_roundtrip");
    for index in 0..3 {
        cook_test_material(&dir, &format!("material.stream{index}"), None);
    }
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    let paths = (0..3)
        .map(|index| dir.join(format!("material.stream{index}.cooked")))
        .collect::<Vec<_>>();

    assert_eq!(runtime.enqueue_cooked_asset_stream(paths), 3);
    assert_eq!(runtime.asset_registry().pending_loads(), 3);
    assert_eq!(runtime.cooked_asset_stream_pending(), 3);
    assert_eq!(
        runtime
            .asset_registry()
            .asset_state(&AssetId::new("material.stream1")),
        Some(engine_asset::AssetState::Loading),
    );

    let report = drain_until_idle(&mut runtime, 1_000);
    assert!(report.is_ok(), "diagnostics: {:?}", report.diagnostics);
    assert_eq!(report.committed, 3);
    assert_eq!(runtime.cooked_asset_stream_pending(), 0);
    assert_eq!(runtime.asset_registry().pending_loads(), 0);
    for index in 0..3 {
        let id = AssetId::new(format!("material.stream{index}"));
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&id)
            .is_some());
        assert_eq!(
            runtime.asset_registry().asset_state(&id),
            Some(engine_asset::AssetState::Ready),
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stream_drain_respects_the_commit_budget() {
    let dir = cooked_case("stream_budget");
    for index in 0..5 {
        cook_test_material(&dir, &format!("material.budget{index}"), None);
    }
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    runtime.set_cooked_asset_stream_budget(2);
    assert_eq!(runtime.cooked_asset_stream_budget(), 2);
    let paths = (0..5)
        .map(|index| dir.join(format!("material.budget{index}.cooked")))
        .collect::<Vec<_>>();
    runtime.enqueue_cooked_asset_stream(paths);

    let mut productive_drains = 0;
    let mut total_committed = 0;
    for _ in 0..1_000 {
        let report = runtime.drain_cooked_asset_stream();
        assert!(report.committed <= 2, "budget exceeded: {report:?}");
        if report.committed > 0 {
            productive_drains += 1;
            total_committed += report.committed;
        }
        if report.is_complete() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    assert_eq!(total_committed, 5);
    assert_eq!(productive_drains, 3, "5 assets at budget 2 commit 2+2+1");
    for index in 0..5 {
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new(format!("material.budget{index}")))
            .is_some());
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stream_commit_conflict_discards_the_failed_batch_and_keeps_prior_state() {
    let dir_a = cooked_case("stream_conflict_a");
    cook_test_material_with_color(&dir_a, "material.keep", None, [0.8, 0.7, 0.6, 1.0]);
    let dir_b = cooked_case("stream_conflict_b");
    cook_test_material_with_color(&dir_b, "material.keep", None, [0.1, 0.2, 0.3, 1.0]);
    cook_test_material(&dir_b, "material.sibling", None);
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    runtime.load_cooked_assets(&dir_a).unwrap();

    runtime.enqueue_cooked_asset_stream(vec![
        dir_b.join("material.keep.cooked"),
        dir_b.join("material.sibling.cooked"),
    ]);
    let report = drain_until_idle(&mut runtime, 1_000);

    assert!(!report.is_ok());
    assert_eq!(report.failed_batches, 1);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "AS0003"
            && diagnostic.message.contains("material.keep")));
    // The conflicting batch was discarded entirely; prior state is intact.
    let installed = runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.keep"))
        .expect("prior material remains");
    assert_eq!(installed.get().base_color, [0.8, 0.7, 0.6, 1.0]);
    assert!(runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.sibling"))
        .is_none());
    assert_eq!(runtime.asset_registry().pending_loads(), 0);
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
}

#[test]
fn stream_decode_failure_reports_and_clears_loading_marks() {
    let dir = cooked_case("stream_decode_failure");
    cook_test_material(&dir, "material.good", None);
    engine_asset::cook::write_cooked_artifact(
        &dir.join("broken.cooked"),
        4_242,
        b"valid outer artifact with unknown kind",
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )
    .unwrap();
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

    runtime.enqueue_cooked_asset_stream(vec![
        dir.join("material.good.cooked"),
        dir.join("broken.cooked"),
    ]);
    let report = drain_until_idle(&mut runtime, 1_000);

    assert!(!report.is_ok());
    assert_eq!(report.failed_batches, 1);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("kind code 4242")));
    // Decode is all-or-nothing per batch: the good sibling never installs.
    assert!(runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.good"))
        .is_none());
    assert_eq!(runtime.asset_registry().pending_loads(), 0);
    assert_eq!(runtime.cooked_asset_stream_pending(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn drain_without_enqueue_is_a_noop() {
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    let report = runtime.drain_cooked_asset_stream();
    assert!(report.is_complete());
    assert!(report.is_ok());
    assert_eq!(report.committed, 0);
    assert_eq!(runtime.cooked_asset_stream_pending(), 0);
}
