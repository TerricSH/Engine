#[test]
fn generated_script_api_is_engine_owned_versioned_and_drift_checked() {
    let temporary = tempfile::tempdir().expect("temporary project");
    let root = temporary.path();
    let script_project = root.join("scripts/GameScripts/GameScripts.csproj");
    std::fs::create_dir_all(script_project.parent().unwrap()).unwrap();
    std::fs::write(
        &script_project,
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n</Project>\n",
    )
    .unwrap();

    let contract = write_generated_script_api(root, &script_project)
        .expect("write generated Script API contract");
    assert!(
        !root.join("build/script-sdk-source").exists(),
        "writing the project integration must not materialize the engine SDK source"
    );
    let (source, _) =
        materialize_script_sdk_source(root).expect("materialize development SDK source");
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        STARTER_SCRIPT_API_SOURCE
    );
    assert_eq!(
        std::fs::read_to_string(
            root.join("build/script-sdk-source")
                .join(engine_script_api::GENERATED_CSHARP_RULES_FILE)
        )
        .unwrap(),
        RULES_SCRIPT_API_SOURCE
    );
    assert_eq!(
        std::fs::read_to_string(
            root.join("build/script-sdk-source")
                .join(engine_script_api::GENERATED_CSHARP_TACTICS_FILE)
        )
        .unwrap(),
        TACTICS_SCRIPT_API_SOURCE
    );
    assert_eq!(
        std::fs::read_to_string(
            root.join("build/script-sdk-source")
                .join(engine_script_api::GENERATED_CSHARP_JRPG_FILE)
        )
        .unwrap(),
        JRPG_SCRIPT_API_SOURCE
    );
    assert_eq!(
        std::fs::read_to_string(
            root.join("build/script-sdk-source")
                .join(engine_script_api::GENERATED_CSHARP_RENDERING_FILE)
        )
        .unwrap(),
        RENDERING_SCRIPT_API_SOURCE
    );
    assert_eq!(
        std::fs::read_to_string(
            root.join("build/script-sdk-source")
                .join(engine_script_api::GENERATED_CSHARP_RUNTIME_ASSETS_FILE)
        )
        .unwrap(),
        RUNTIME_ASSETS_SCRIPT_API_SOURCE
    );
    assert_eq!(
        std::fs::read_to_string(
            root.join("build/script-sdk-source")
                .join(engine_script_api::GENERATED_CSHARP_ONLINE_XR_FILE)
        )
        .unwrap(),
        ONLINE_XR_SCRIPT_API_SOURCE
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&contract).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "EngineGameplaySdkContract-v1");
    assert_eq!(
        manifest["generated_sources"][1],
        engine_script_api::GENERATED_CSHARP_RULES_FILE
    );
    assert_eq!(
        manifest["generated_sources"][2],
        engine_script_api::GENERATED_CSHARP_TACTICS_FILE
    );
    assert_eq!(
        manifest["generated_sources"][3],
        engine_script_api::GENERATED_CSHARP_JRPG_FILE
    );
    assert_eq!(
        manifest["generated_sources"][4],
        engine_script_api::GENERATED_CSHARP_RENDERING_FILE
    );
    assert_eq!(
        manifest["generated_sources"][5],
        engine_script_api::GENERATED_CSHARP_RUNTIME_ASSETS_FILE
    );
    assert_eq!(
        manifest["generated_sources"][6],
        engine_script_api::GENERATED_CSHARP_ONLINE_XR_FILE
    );
    assert_eq!(
        manifest["script_api"],
        engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA
    );
    assert_eq!(
        manifest["version"],
        engine_script_api::GAMEPLAY_SCRIPT_API_VERSION
    );
    assert_eq!(manifest["owner"], "engine");
    assert_eq!(manifest["managed_sdk_assembly"], "EngineGameplay");
    assert_eq!(manifest["sha256"], script_api_sha256());
    assert!(root
        .join("build/script-sdk-source/EngineGameplay.csproj")
        .is_file());
    assert!(std::fs::read_to_string(&script_project)
        .unwrap()
        .contains("<Import Project=\"EngineGameplay.targets\" />"));
    let targets = root.join("scripts/GameScripts/EngineGameplay.targets");
    assert_eq!(
        std::fs::read_to_string(&targets).unwrap(),
        SCRIPT_SDK_TARGETS
    );
    assert!(!root.join("scripts/GameScripts/EngineGameplay.cs").exists());
    assert!(validate_generated_script_api(root, &script_project).is_ok());

    std::fs::write(
        &targets,
        "<!-- game-specific edit in engine-owned integration -->",
    )
    .expect("mutate generated Script SDK integration");
    let error = validate_generated_script_api(root, &script_project)
        .expect_err("generated contract drift must stop the script build");
    assert!(error.contains("sync-script-api"));
}

#[test]
fn installed_deployment_exactly_mirrors_sdk_and_host_without_sources() {
    let temporary = tempfile::tempdir().expect("temporary project");
    let root = temporary.path().join("game");
    crate::project_cli::create_project(&root, Some("Installed Tools"), true)
        .expect("scripted project");
    let project = GameProject::load(&root).expect("load scripted project");

    let installed_sdk = temporary.path().join("install/sdk/EngineGameplay.dll");
    let installed_host = temporary.path().join("install/sdk/script-host");
    std::fs::create_dir_all(installed_sdk.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&installed_host).unwrap();
    let installed_sdk_bytes = [0x00, 0xff, 0x45, 0x53, 0x44, 0x4b];
    std::fs::write(&installed_sdk, installed_sdk_bytes).unwrap();
    for (name, bytes) in [
        (
            host_executable_name(),
            b"verified host executable".as_slice(),
        ),
        ("EngineScriptHost.dll", b"verified host assembly".as_slice()),
        (
            "EngineScriptHost.runtimeconfig.json",
            b"{\"runtimeOptions\":{}}".as_slice(),
        ),
    ] {
        std::fs::write(installed_host.join(name), bytes).unwrap();
    }

    let project_sdk = root.join("build/script-sdk");
    let project_host = root.join("build/script-host");
    std::fs::create_dir_all(&project_sdk).unwrap();
    std::fs::create_dir_all(&project_host).unwrap();
    std::fs::write(project_sdk.join("EngineGameplay.dll"), b"wrong SDK").unwrap();
    std::fs::write(project_sdk.join("stale.dll"), b"stale SDK file").unwrap();
    std::fs::write(
        project_host.join(host_executable_name()),
        b"wrong host executable",
    )
    .unwrap();
    std::fs::write(project_host.join("stale.dll"), b"stale host file").unwrap();

    let validations = std::cell::Cell::new(0usize);
    let validate_host = |executable: &Path, directory: &Path| {
        assert_eq!(executable, directory.join(host_executable_name()));
        assert!(executable.is_file());
        validations.set(validations.get() + 1);
        Ok(())
    };
    deploy_installed_managed_tools_from(&project, &installed_sdk, &installed_host, validate_host)
        .expect("deploy installed managed tools");

    assert!(directory_contains_only_regular_file_equal(
        &project_sdk,
        "EngineGameplay.dll",
        &installed_sdk
    )
    .unwrap());
    assert!(directory_files_equal(&installed_host, &project_host).unwrap());
    assert_eq!(
        std::fs::read(project_sdk.join("EngineGameplay.dll")).unwrap(),
        installed_sdk_bytes
    );
    for entry in std::fs::read_dir(&installed_host).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(
            std::fs::read(project_host.join(entry.file_name())).unwrap(),
            std::fs::read(entry.path()).unwrap()
        );
    }
    assert_eq!(std::fs::read_dir(&project_sdk).unwrap().count(), 1);
    assert_eq!(
        std::fs::read_dir(&project_host).unwrap().count(),
        std::fs::read_dir(&installed_host).unwrap().count()
    );
    assert!(!root.join("build/script-sdk.next").exists());
    assert!(!root.join("build/script-sdk.previous").exists());
    assert!(!root.join("build/script-host.next").exists());
    assert!(!root.join("build/script-host.previous").exists());
    assert!(!root.join("build/script-sdk-source").exists());
    assert!(!root.join("build/script-host-source").exists());

    deploy_installed_managed_tools_from(&project, &installed_sdk, &installed_host, validate_host)
        .expect("reuse identical installed managed tools");
    assert_eq!(validations.get(), 2);
}
