    #[test]
    fn validation_completion_requires_the_expected_report_contract() {
        let directory = tempfile::tempdir().unwrap();
        let report = directory.path().join("report.json");
        std::fs::write(
            &report,
            serde_json::to_vec(&serde_json::json!({
                "schema": PROJECT_CHECK_SCHEMA,
                "passed": true,
                "project": "Fixture",
                "startup_scene_id": "main",
                "scenes": 1,
                "entities": 2,
                "declared_assets": 3,
                "cooked_assets": 3
            }))
            .unwrap(),
        )
        .unwrap();

        let result = finish_validation(
            &report,
            Duration::from_millis(10),
            EditorBuildOutput::default(),
        )
        .unwrap();
        let EditorBuildResult::Validated(result) = result else {
            panic!("wrong result kind");
        };
        assert_eq!(result.project, "Fixture");
        assert_eq!(result.declared_assets, 3);

        std::fs::write(&report, br#"{"schema":"wrong","passed":true}"#).unwrap();
        let error =
            finish_validation(&report, Duration::ZERO, EditorBuildOutput::default()).unwrap_err();
        assert_eq!(error.kind, EditorBuildFailureKind::InvalidResult);
    }

    #[test]
    fn package_completion_verifies_metadata_and_both_checksums() {
        let directory = tempfile::tempdir().unwrap();
        let release_root = directory.path().join("v9");
        let manifest_directory = release_root.join(WINDOWS_PLATFORM).join("manifests");
        std::fs::create_dir_all(&manifest_directory).unwrap();
        std::fs::write(
            manifest_directory.join("release.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": RELEASE_METADATA_SCHEMA,
                "release_id": "v9",
                "platform": WINDOWS_PLATFORM,
                "backend": "vulkan",
                "dirty": false
            }))
            .unwrap(),
        )
        .unwrap();
        for name in [
            format!("{WINDOWS_PLATFORM}.zip"),
            format!("{WINDOWS_PLATFORM}-symbols.zip"),
        ] {
            let path = release_root.join(&name);
            std::fs::write(&path, name.as_bytes()).unwrap();
            let hash = format!("{:x}", Sha256::digest(name.as_bytes()));
            let mut sidecar = File::create(format!("{}.sha256", path.display())).unwrap();
            writeln!(sidecar, "{hash}  {name}").unwrap();
        }

        let result = finish_windows_package(
            "v9".into(),
            false,
            release_root,
            Duration::from_secs(1),
            EditorBuildOutput::default(),
        )
        .unwrap();
        let EditorBuildResult::PackagedWindows(result) = result else {
            panic!("wrong result kind");
        };
        assert_eq!(result.version, "v9");
        assert!(!result.dirty);
        assert_eq!(result.archive_sha256.len(), 64);
        assert_eq!(result.symbols_sha256.len(), 64);
    }

    #[test]
    fn bounded_capture_keeps_the_diagnostic_tail() {
        let mut capture = CapturedBytes::default();
        capture.append(&vec![b'a'; CAPTURE_LIMIT_BYTES]);
        capture.append(b"final-error");
        let (text, truncated) = capture.snapshot();
        assert!(truncated);
        assert_eq!(text.len(), CAPTURE_LIMIT_BYTES);
        assert!(text.ends_with("final-error"));
    }

    #[test]
    fn displayed_process_error_contains_stderr_tail() {
        let error = EditorBuildError::process(
            EditorBuildOperationKind::PackageWindows,
            EditorBuildFailureKind::ProcessFailed,
            "authoritative build process failed",
            Some(7),
            EditorBuildOutput {
                stderr: "release packaging requires a clean worktree".into(),
                ..EditorBuildOutput::default()
            },
        );
        let display = error.to_string();
        assert!(display.contains("exit code 7"));
        assert!(display.contains("clean worktree"));
    }

    /// Marker line the child-process helper prints so the parent test can
    /// confirm stdout capture flows through the background-task machinery.
    const CHILD_HELPER_STDOUT_MARKER: &str = "editor-build-task-test";

    /// Child-process entry point spawned by
    /// `background_task_exposes_output_completion_and_finished_cancellation`.
    ///
    /// Ignored during normal test runs: the parent test re-invokes this test
    /// binary with `--exact`/`--ignored` so the child prints a marker line
    /// and exits successfully. Spawning the current test executable keeps the
    /// background-task test hermetic — it no longer depends on PowerShell,
    /// `/bin/sh`, or any other system tool being installed (ENG-71).
    #[test]
    #[ignore = "child-process helper for the background-task test"]
    fn child_process_helper_prints_marker_and_exits() {
        println!("{CHILD_HELPER_STDOUT_MARKER}");
    }

    #[test]
    fn background_task_exposes_output_completion_and_finished_cancellation() {
        let (_directory, manifest_path) = project_fixture();
        // Spawn the current test executable in its helper mode: the binary
        // always exists on every platform and needs no system shell, so the
        // test exercises the spawn/output/cancellation machinery without
        // conflating code correctness with environment contents.
        let executable = std::env::current_exe().expect("current test executable");
        let arguments = vec![
            OsString::from("--exact"),
            OsString::from("editor_build_ops::tests::child_process_helper_prints_marker_and_exits"),
            OsString::from("--ignored"),
            OsString::from("--nocapture"),
        ];
        let plan = ProcessPlan {
            kind: EditorBuildOperationKind::CookAndCompile,
            executable,
            arguments,
            working_directory: workspace_root(),
            completion: CompletionPlan::CookAndCompile { manifest_path },
        };
        let mut task = EditorBuildTask::spawn(plan).unwrap();
        assert_eq!(task.operation(), EditorBuildOperationKind::CookAndCompile);
        let deadline = Instant::now() + Duration::from_secs(30);
        let result = loop {
            if let Some(result) = task.try_complete() {
                break result.unwrap();
            }
            assert!(Instant::now() < deadline, "background task timed out");
            thread::sleep(Duration::from_millis(10));
        };
        let EditorBuildResult::CookedAndCompiled(result) = result else {
            panic!("wrong result kind");
        };
        assert_eq!(result.project, "Build Service Test");
        assert!(task
            .output_snapshot()
            .stdout
            .contains(CHILD_HELPER_STDOUT_MARKER));
        assert!(!task.cancel().unwrap());
    }

    #[test]
    fn current_editor_service_rejects_a_missing_project_before_spawn() {
        let service = EditorBuildService::for_current_editor().unwrap();
        let missing = tempfile::tempdir().unwrap().path().join("missing-project");
        let error = match service.start(&missing, EditorBuildOperation::Validate) {
            Ok(_) => panic!("missing project unexpectedly started"),
            Err(error) => error,
        };
        assert_eq!(error.kind, EditorBuildFailureKind::InvalidRequest);
    }
