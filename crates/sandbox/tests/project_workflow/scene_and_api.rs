#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_scene_load_transitions_to_catalog_scene() {
    if !common::require_tool("dotnet") {
        return;
    }
    let root = unique_project_root();
    let run_report = root.join("build/csharp-scene-load-run.json");

    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Managed Scene Game",
        "--with-csharp",
    ]);
    assert_success(&output, "managed scene project new");
    let output = run(&[
        "project",
        "scene",
        "new",
        path_text(&root),
        "level_two",
        "--name",
        "Level Two",
    ]);
    assert_success(&output, "managed project scene new");

    let script_source = install_managed_workflow_behaviour(&root);
    let starter_source =
        std::fs::read_to_string(&script_source).expect("read managed workflow behaviour");
    let scene_loading_source = starter_source.replace(
        "    public void OnStart()\n    {\n    }",
        "    public void OnStart()\n    {\n        Scene.Load(\"level_two\");\n    }",
    );
    assert_ne!(
        scene_loading_source, starter_source,
        "workflow fixture OnStart method changed unexpectedly"
    );
    std::fs::write(&script_source, scene_loading_source).expect("write scene-loading script");

    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_success(&output, "managed scene script build");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&run_report),
    ]);
    assert_success(&output, "managed scene transition runtime");

    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&run_report).expect("read managed scene transition report"),
    )
    .expect("parse managed scene transition report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["final_scene_id"], "level_two");
    assert_eq!(report["scene_transitions"], 1);

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_script_api_sync_restores_engine_owned_contract() {
    if !common::require_tool("dotnet") {
        return;
    }
    let root = unique_project_root();
    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Managed Contract",
        "--with-csharp",
    ]);
    assert_success(&output, "managed contract project new");

    let legacy_source = root.join("scripts/GameScripts/EngineGameplay.cs");
    let source = root.join("build/script-sdk-source/EngineGameplay.cs");
    let contract = root.join("scripts/GameScripts/EngineGameplay.contract.json");
    let targets = root.join("scripts/GameScripts/EngineGameplay.targets");
    assert!(
        !source.exists(),
        "project creation must not materialize the development SDK source"
    );
    assert!(contract.is_file());
    assert!(targets.is_file());
    assert!(!legacy_source.exists());

    std::fs::write(&legacy_source, "// legacy generated gameplay API\n")
        .expect("restore legacy generated gameplay API");
    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_failure(&output, "stale managed contract build");
    assert!(String::from_utf8_lossy(&output.stderr).contains("sync-script-api"));

    let output = run(&["project", "sync-script-api", path_text(&root)]);
    assert_success(&output, "managed contract sync");
    let report = stdout_json(&output);
    assert_eq!(report["schema"], "ProjectScriptApiSyncReport-v1");
    assert_eq!(
        report["script_api"],
        engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA
    );
    assert_eq!(
        report["version"],
        engine_script_api::GAMEPLAY_SCRIPT_API_VERSION
    );
    assert_eq!(report["passed"], true);
    assert_eq!(
        report["source"],
        source.to_string_lossy().replace('\\', "/")
    );
    assert!(!legacy_source.exists());

    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&contract).expect("read generated gameplay contract"),
    )
    .expect("parse generated gameplay contract");
    assert_eq!(manifest["owner"], "engine");
    assert_eq!(manifest["sha256"], report["sha256"]);
    assert!(std::fs::read_to_string(&source)
        .expect("read synchronized gameplay API")
        .contains("public static class ScriptApiContract"));

    let _ = std::fs::remove_dir_all(root);
}
