    #[test]
    fn release_versions_match_the_authoritative_script_contract() {
        let maximum_version = "a".repeat(64);
        for valid in ["1", "v1.2.3", "nightly_2026-07-18", &maximum_version] {
            assert!(validate_release_version(valid).is_ok(), "{valid}");
        }
        let oversized_version = "a".repeat(65);
        for invalid in [
            "",
            ".hidden",
            "two words",
            "a/b",
            "版本",
            &oversized_version,
        ] {
            assert!(validate_release_version(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn package_plan_calls_the_official_script_without_allow_dirty_by_default() {
        let (directory, manifest_path) = project_fixture();
        let output = directory.path().join("release-output");
        let plan = service()
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(PackageWindowsOptions::new("v1.2.3", &output)),
            )
            .unwrap();
        let arguments = argument_strings(&plan);

        assert_eq!(plan.kind, EditorBuildOperationKind::PackageWindows);
        assert!(arguments.iter().any(|argument| {
            argument
                .replace('\\', "/")
                .ends_with("/.github/scripts/package-windows.ps1")
        }));
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-ProjectPath"
                && Path::new(&arguments[1]).file_name() == Some(OsStr::new("game.project.json"))
        }));
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-OutputRoot" && Path::new(&arguments[1]) == output
        }));
        assert!(!arguments.iter().any(|argument| argument == "-AllowDirty"));
        assert!(!arguments.iter().any(|argument| argument == "-SkipSmoke"));
    }

    #[test]
    fn installed_package_plan_uses_project_local_output_and_prebuilt_toolchain() {
        let (_installation, service) = installed_service();
        let (_project, manifest_path) = project_fixture();
        let project_root = manifest_path.parent().unwrap();
        let plan = service
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                    "v-installed",
                    "Dist",
                )),
            )
            .unwrap();
        let arguments = argument_strings(&plan);
        assert_eq!(plan.working_directory, project_root);
        assert!(arguments.iter().any(|argument| {
            argument
                .replace('\\', "/")
                .ends_with("/tools/package-windows.ps1")
        }));
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-EngineInstallRoot"
                && Path::new(&arguments[1])
                    .join("engine.installation.json")
                    .is_file()
        }));
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-OutputRoot" && Path::new(&arguments[1]) == project_root.join("Dist")
        }));
        assert!(!arguments.iter().any(|argument| {
            matches!(
                argument.as_str(),
                "-CargoTargetDir" | "-SkipBuild" | "-AllowDirty"
            )
        }));

        let outside_output = project_root.parent().unwrap().join("outside-dist");
        let error = service
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                    "v-outside",
                    outside_output,
                )),
            )
            .unwrap_err();
        assert_eq!(error.kind, EditorBuildFailureKind::InvalidRequest);
        assert!(error.message.contains("inside the project workspace"));
    }

    #[test]
    fn installed_package_plan_rejects_project_owned_build_directories() {
        let (_installation, service) = installed_service();
        let (_project, manifest_path) = scripted_project_fixture();
        let project_root = manifest_path.parent().unwrap();

        for (label, output) in [
            ("cooked_assets", "build/cooked/package-output"),
            ("script_assembly output", "build/scripts/package-output"),
            ("managed script SDK", "build/script-sdk/package-output"),
            ("managed script host", "build/script-host/package-output"),
        ] {
            let error = service
                .plan(
                    &manifest_path,
                    EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                        "v-conflict",
                        output,
                    )),
                )
                .unwrap_err();
            assert_eq!(error.kind, EditorBuildFailureKind::InvalidRequest);
            assert!(
                error.message.contains(label),
                "expected {label:?} in error for {output:?}: {}",
                error.message
            );
        }

        let safe = service
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                    "v-safe",
                    "build/releases",
                )),
            )
            .unwrap();
        let arguments = argument_strings(&safe);
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-OutputRoot"
                && Path::new(&arguments[1]) == project_root.join("build/releases")
        }));
    }

    #[test]
    fn dirty_and_skip_switches_require_explicit_options() {
        let (directory, manifest_path) = project_fixture();
        let mut options =
            PackageWindowsOptions::new("local-dry-run", directory.path().join("release-output"));
        options.allow_dirty = true;
        options.skip_build = true;
        options.skip_smoke = true;
        let plan = service()
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(options),
            )
            .unwrap();
        let arguments = argument_strings(&plan);

        assert!(arguments.iter().any(|argument| argument == "-AllowDirty"));
        assert!(arguments.iter().any(|argument| argument == "-SkipBuild"));
        assert!(arguments.iter().any(|argument| argument == "-SkipSmoke"));
    }

    #[test]
    fn validate_and_cook_plans_use_distinct_formal_cli_commands() {
        let (_directory, manifest_path) = project_fixture();
        let project_root = manifest_path.parent().unwrap();
        let validate = service()
            .plan(&manifest_path, EditorBuildOperation::Validate)
            .unwrap();
        let cook = service()
            .plan(&manifest_path, EditorBuildOperation::CookAndCompile)
            .unwrap();
        let validate_arguments = argument_strings(&validate);
        let cook_arguments = argument_strings(&cook);

        assert_eq!(&validate_arguments[..2], &["project", "check"]);
        assert!(validate_arguments
            .iter()
            .any(|argument| argument == "--report"));
        assert_eq!(&cook_arguments[..2], &["project", "build"]);
        assert!(!cook_arguments.iter().any(|argument| argument == "--report"));
        assert_eq!(validate.working_directory, project_root);
        assert_eq!(cook.working_directory, project_root);
    }

    #[test]
    fn unsafe_package_roots_and_versions_are_rejected_before_spawn() {
        let (_directory, manifest_path) = project_fixture();
        let service = service();
        let bad_version = service.plan(
            &manifest_path,
            EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                "../escape",
                "artifacts/release",
            )),
        );
        assert_eq!(
            bad_version.unwrap_err().kind,
            EditorBuildFailureKind::InvalidRequest
        );

        let project_root = manifest_path.parent().unwrap();
        let root_output = service.plan(
            &manifest_path,
            EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                "safe-version",
                project_root,
            )),
        );
        assert_eq!(
            root_output.unwrap_err().kind,
            EditorBuildFailureKind::InvalidRequest
        );
    }
