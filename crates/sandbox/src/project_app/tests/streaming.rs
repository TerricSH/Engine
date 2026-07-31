    #[test]
    fn cell_streaming_is_opt_in_and_requires_a_partition_manifest() {
        let (_temp, project) = cell_streaming_project_fixture();
        // Without the flag no driver is constructed even when a partition
        // manifest exists; with the flag the partition builds a driver.
        assert!(create_cell_streaming_driver(&project, false)
            .unwrap()
            .is_none());
        assert!(create_cell_streaming_driver(&project, true)
            .unwrap()
            .is_some());

        // The flag without a partition manifest is an explicit error.
        let (_temp2, no_partition) = scene_project_fixture();
        let error = create_cell_streaming_driver(&no_partition, true)
            .err()
            .expect("streaming without a partition manifest must fail");
        assert!(error.contains("world.partition.json"), "{error}");
    }

    #[test]
    fn headless_cell_streaming_loads_and_unloads_cells_around_the_camera() {
        let (_temp, project) = cell_streaming_project_fixture();
        let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
        let (mut game_loop, _) = create_game_loop(&project, scene).unwrap();
        game_loop
            .runtime
            .set_renderer_backend(Box::<crate::qa::QaBackend>::default());
        let mut driver = create_cell_streaming_driver(&project, true).unwrap();
        driver.as_mut().unwrap().rebaseline(&game_loop.runtime);
        let mut current_scene_id = project.startup_scene_id().to_string();

        // Frame boundary with the camera at the origin: the cell streams in.
        let transitions =
            process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id)
                .unwrap();
        tick_cell_streaming(&mut game_loop, &mut driver, transitions);
        assert!(has_persistent_entity(&game_loop, "cube-two"));
        assert_eq!(
            driver.as_ref().unwrap().loaded_cells(),
            vec!["cell_two".to_string()]
        );

        // The camera leaves the cell bounds: the cell unloads at the next
        // frame boundary.
        game_loop.update(1.0 / 60.0);
        set_main_camera_position(&mut game_loop, [100.0, 0.0, 0.0]);
        let transitions =
            process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id)
                .unwrap();
        tick_cell_streaming(&mut game_loop, &mut driver, transitions);
        assert!(!has_persistent_entity(&game_loop, "cube-two"));
        assert!(driver.as_ref().unwrap().loaded_cells().is_empty());

        // The camera returns: the cell streams back in.
        game_loop.update(1.0 / 60.0);
        set_main_camera_position(&mut game_loop, [0.0, 0.0, 0.0]);
        let transitions =
            process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id)
                .unwrap();
        tick_cell_streaming(&mut game_loop, &mut driver, transitions);
        assert!(has_persistent_entity(&game_loop, "cube-two"));
        let driver = driver.unwrap();
        assert_eq!(driver.total_merges(), 2);
        assert_eq!(driver.total_unloads(), 1);
    }

    #[test]
    fn headless_run_report_includes_cell_streaming_state() {
        let (_temp, project) = cell_streaming_project_fixture();
        let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
        let report_path = project.root.join("build/run-report.json");
        run_headless(project, scene, 3, Some(&report_path), true).unwrap();

        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
        assert_eq!(report["passed"], true);
        assert_eq!(report["cell_streaming"]["enabled"], true);
        assert_eq!(
            report["cell_streaming"]["loaded_cells"],
            serde_json::json!(["cell_two"])
        );
        assert_eq!(report["cell_streaming"]["total_merges"], 1);
        assert_eq!(report["cell_streaming"]["total_unloads"], 0);
        assert_eq!(report["cell_streaming"]["resident_entities"], 0);
        assert_eq!(
            report["cell_streaming"]["cell_states"]["cell_two"],
            "Loaded"
        );
        // No origin shifting configured: the report shows the zero origin.
        assert_eq!(report["world_origin"], serde_json::json!([0.0, 0.0, 0.0]));
        assert_eq!(report["world_origin_shifts"], 0);
    }

    #[test]
    fn headless_run_report_includes_frame_timing_section() {
        let (_temp, project) = scene_project_fixture();
        let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
        let report_path = project.root.join("build/run-report.json");
        run_headless(project, scene, 3, Some(&report_path), false).unwrap();

        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
        assert_eq!(report["passed"], true);

        let frame_timing = &report["frame_timing"];
        assert_eq!(frame_timing["window_frames"], 3);
        // The headless QA backend cannot provide GPU timestamps: the section
        // reports "unavailable" and carries CPU-only aggregates.
        assert_eq!(frame_timing["gpu_status"], "unavailable");
        assert!(frame_timing.get("total_gpu").is_none());
        assert!(frame_timing["total_cpu"]["avg_ms"].is_number());

        let passes = frame_timing["passes"].as_array().unwrap();
        for stage in [
            "update",
            "extraction",
            "sync_render_assets",
            "render_submit",
        ] {
            let pass = passes
                .iter()
                .find(|pass| pass["name"] == stage)
                .unwrap_or_else(|| panic!("missing stage '{stage}' in {passes:?}"));
            assert_eq!(pass["cpu"]["samples"], 3);
            assert!(pass["cpu"]["avg_ms"].is_number());
            assert!(pass["cpu"]["p95_ms"].is_number());
            assert!(pass["cpu"]["max_ms"].is_number());
            assert!(pass.get("gpu").is_none());
        }
    }

    #[test]
    fn headless_run_shifts_world_origin_past_threshold() {
        let (_temp, project) = origin_shift_project_fixture();
        let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
        let report_path = project.root.join("build/run-report.json");
        run_headless(project, scene, 3, Some(&report_path), false).unwrap();

        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
        assert_eq!(report["passed"], true);
        assert!(report["total_draw_calls"].as_u64().unwrap() > 0);
        // The camera starts at x = 150 with threshold 100: exactly one shift
        // runs at the first frame boundary and the camera lands back on the
        // relative origin, so no further shift triggers.
        assert_eq!(report["world_origin_shifts"], 1);
        let origin = report["world_origin"].as_array().unwrap();
        assert_eq!(origin[0].as_f64().unwrap(), 150.0);
        assert_eq!(origin[1].as_f64().unwrap(), 0.0);
        assert_eq!(origin[2].as_f64().unwrap(), 0.0);
    }
