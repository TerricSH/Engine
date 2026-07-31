#[test]
fn creates_checks_runs_and_cooks_a_game_project() {
    let root = unique_project_root();
    let check_report = root.join("build/project-check.json");
    let run_report = root.join("build/project-run.json");

    let output = run(&["project", "new", path_text(&root), "--name", "Test Game"]);
    assert_success(&output, "project new");
    assert!(root.join("config/input.actions.json").is_file());
    let output = run(&[
        "project",
        "check",
        path_text(&root),
        "--report",
        path_text(&check_report),
    ]);
    assert_success(&output, "project check");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&run_report),
    ]);
    assert_success(&output, "game");
    let output = run(&["project", "cook", path_text(&root)]);
    assert_success(&output, "project cook");

    let check: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&check_report).expect("read project check report"))
            .expect("parse project check report");
    let run: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&run_report).expect("read project run report"))
            .expect("parse project run report");
    assert_eq!(check["schema"], "ProjectCheckReport-v0");
    assert_eq!(check["passed"], true);
    assert_eq!(check["input_actions"], 6);
    assert_eq!(check["input_bindings"], 6);
    assert_eq!(run["schema"], "ProjectRunReport-v0");
    assert_eq!(run["passed"], true);
    assert_eq!(run["frames"], 3);
    assert!(run["total_draw_calls"].as_u64().unwrap_or(0) >= 3);
    assert!(root.join("build/cooked").is_dir());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn creates_lists_validates_and_runs_multiple_project_scenes() {
    let root = unique_project_root();
    let check_report = root.join("build/multi-scene-check.json");

    let output = run(&["project", "new", path_text(&root), "--name", "Scene Game"]);
    assert_success(&output, "project new for scene catalog");
    let output = run(&[
        "project",
        "scene",
        "new",
        path_text(&root),
        "level_two",
        "--name",
        "Level Two",
    ]);
    assert_success(&output, "project scene new");
    assert!(root.join("assets/scenes/level_two.scene.ron").is_file());

    let output = run(&["project", "scene", "list", path_text(&root)]);
    assert_success(&output, "project scene list");
    let list = stdout_json(&output);
    assert_eq!(list["startup_scene_id"], "main");
    assert_eq!(list["scenes"].as_array().unwrap().len(), 2);

    let output = run(&[
        "project",
        "check",
        path_text(&root),
        "--report",
        path_text(&check_report),
    ]);
    assert_success(&output, "multi-scene project check");
    let check: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&check_report).unwrap()).unwrap();
    assert_eq!(check["scenes"], 2);
    assert_eq!(check["scene_entities"]["main"], 3);
    assert_eq!(check["scene_entities"]["level_two"], 3);

    let output = run(&[
        "project",
        "scene",
        "set-startup",
        path_text(&root),
        "level_two",
    ]);
    assert_success(&output, "project scene set-startup");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("game.project.json")).expect("read scene catalog manifest"),
    )
    .unwrap();
    assert_eq!(manifest["startup_scene"], "level_two");
    assert_eq!(manifest["scenes"]["main"], "assets/scenes/main.scene.ron");
    assert_eq!(
        manifest["scenes"]["level_two"],
        "assets/scenes/level_two.scene.ron"
    );
    let output = run(&["game", path_text(&root), "--headless", "--frames", "1"]);
    assert_success(&output, "run catalog startup scene");

    // Check must cover every catalog entry, not only the active startup.
    std::fs::write(root.join("assets/scenes/main.scene.ron"), b"not ron")
        .expect("corrupt non-startup scene");
    let output = run(&["project", "check", path_text(&root)]);
    assert_failure(&output, "non-startup scene validation");
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(messages.contains("main"), "missing scene ID: {messages}");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn renames_and_recoverably_deletes_project_scenes_from_the_formal_workflow() {
    let root = unique_project_root();
    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Scene Mutation Game",
    ]);
    assert_success(&output, "project new for scene mutation");
    let output = run(&["project", "scene", "new", path_text(&root), "level_old"]);
    assert_success(&output, "project scene new for rename");
    let output = run(&[
        "project",
        "scene",
        "set-startup",
        path_text(&root),
        "level_old",
    ]);
    assert_success(&output, "project scene startup before rename");

    let output = run(&[
        "project",
        "scene",
        "rename",
        path_text(&root),
        "level_old",
        "level_new",
    ]);
    assert_success(&output, "project scene rename");
    let rename_report = stdout_json(&output);
    assert_eq!(rename_report["schema"], "ProjectSceneRenameReport-v0");
    assert_eq!(rename_report["old_scene_id"], "level_old");
    assert_eq!(rename_report["scene_id"], "level_new");
    assert_eq!(rename_report["renamed"], true);
    assert!(!root.join("assets/scenes/level_old.scene.ron").exists());
    let renamed_path = root.join("assets/scenes/level_new.scene.ron");
    let renamed = Scene::load_from_file(&renamed_path).expect("load renamed scene");
    assert_eq!(renamed.scene_id, "level_new");
    assert_eq!(renamed.name, "level_new");
    let project = GameProject::load(&root).expect("load project after scene rename");
    assert_eq!(project.startup_scene_id(), "level_new");
    assert!(project.scene_path("level_old").is_none());

    let output = run(&["project", "scene", "delete", path_text(&root), "level_new"]);
    assert_failure(&output, "startup scene delete without replacement");
    assert!(renamed_path.is_file());

    let output = run(&[
        "project",
        "scene",
        "delete",
        path_text(&root),
        "level_new",
        "--replacement-startup",
        "main",
    ]);
    assert_success(&output, "recoverable project scene delete");
    let delete_report = stdout_json(&output);
    assert_eq!(delete_report["schema"], "ProjectSceneDeleteReport-v0");
    assert_eq!(delete_report["scene_id"], "level_new");
    assert_eq!(delete_report["replacement_startup"], "main");
    assert_eq!(delete_report["recoverable"], true);
    let trash_directory = PathBuf::from(
        delete_report["trash_directory"]
            .as_str()
            .expect("scene trash directory path"),
    );
    let metadata_path = PathBuf::from(
        delete_report["metadata"]
            .as_str()
            .expect("scene trash metadata path"),
    );
    assert!(!renamed_path.exists());
    assert!(trash_directory.join("scene.scene.ron").is_file());
    assert!(metadata_path.is_file());
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(metadata_path).expect("read scene trash metadata"))
            .expect("parse scene trash metadata");
    assert_eq!(metadata["schema"], "EditorSceneTrash-v0");
    assert_eq!(metadata["scene_id"], "level_new");
    assert_eq!(
        metadata["original_scene_path"],
        "assets/scenes/level_new.scene.ron"
    );
    assert_eq!(metadata["was_startup"], true);
    assert_eq!(metadata["replacement_startup"], "main");
    let project = GameProject::load(&root).expect("load project after scene delete");
    assert_eq!(project.startup_scene_id(), "main");
    assert_eq!(project.scenes().len(), 1);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn imports_texture_and_rolls_back_every_rejected_or_failed_import() {
    let root = unique_project_root();
    let external = root.with_extension("import-sources");
    std::fs::create_dir_all(&external).expect("create external source directory");

    let output = run(&["project", "new", path_text(&root), "--name", "Import Game"]);
    assert_success(&output, "project new for import");

    let texture_source = external.join("checker.ppm");
    std::fs::create_dir_all(root.join("assets/source/Textures"))
        .expect("create texture import folder");
    std::fs::write(
        &texture_source,
        b"P3\n2 2\n255\n255 0 0  0 255 0\n0 0 255  255 255 255\n",
    )
    .expect("write PPM texture");
    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&texture_source),
        "--id",
        "imported-checker",
        "--folder",
        "Textures",
    ]);
    assert_success(&output, "project texture import");
    let import_report = String::from_utf8_lossy(&output.stdout);
    assert!(import_report.contains("\"schema\": \"ProjectImportReport-v0\""));
    assert!(import_report.contains("\"asset_type\": \"texture\""));
    assert!(import_report.contains("\"imported\": true"));

    let copied_source = root.join("assets/source/Textures/checker.ppm");
    let manifest_path = root.join("assets/source/game.manifest");
    let cooked_path = root.join("build/cooked/imported-checker.cooked");
    assert_eq!(
        std::fs::read(&copied_source).expect("read copied source"),
        std::fs::read(&texture_source).expect("read external source")
    );
    let manifest: SourceManifest = serde_json::from_slice(
        &std::fs::read(&manifest_path).expect("read updated source manifest"),
    )
    .expect("parse updated source manifest");
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(manifest.assets[0].id.id, "imported-checker");
    assert_eq!(manifest.assets[0].asset_type, AssetType::Texture);
    assert_eq!(manifest.assets[0].source_path, "Textures/checker.ppm");
    let cooked = read_cooked_artifact(&cooked_path).expect("validate imported cooked texture");
    assert_eq!(cooked.header.asset_kind, AssetType::Texture.kind_code());

    let output = run(&["project", "check", path_text(&root)]);
    assert_success(&output, "project check after import");
    let output = run(&["project", "cook", path_text(&root)]);
    assert_success(&output, "project cook after import");

    let stable_manifest = std::fs::read(&manifest_path).expect("snapshot source manifest");
    let stable_cooked = std::fs::read(&cooked_path).expect("snapshot cooked texture");

    let duplicate_source = external.join("duplicate.ppm");
    std::fs::copy(&texture_source, &duplicate_source).expect("write duplicate PPM source");
    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&duplicate_source),
        "--id",
        "IMPORTED-CHECKER",
    ]);
    assert_failure(&output, "case-insensitive duplicate ID import");
    assert!(!root.join("assets/source/duplicate.ppm").exists());
    assert_eq!(std::fs::read(&manifest_path).unwrap(), stable_manifest);
    assert_eq!(std::fs::read(&cooked_path).unwrap(), stable_cooked);

    let unsupported_source = external.join("unsupported.txt");
    std::fs::write(&unsupported_source, b"not an asset").expect("write unsupported source");
    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&unsupported_source),
        "--id",
        "unsupported-asset",
    ]);
    assert_failure(&output, "unsupported extension import");
    assert!(!root.join("assets/source/unsupported.txt").exists());
    assert_eq!(std::fs::read(&manifest_path).unwrap(), stable_manifest);

    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&texture_source),
        "--id",
        "second-checker",
        "--folder",
        "Textures",
    ]);
    assert_failure(&output, "source target conflict import");
    assert!(!root.join("build/cooked/second-checker.cooked").exists());
    assert_eq!(std::fs::read(&manifest_path).unwrap(), stable_manifest);

    let broken_source = external.join("broken.ppm");
    std::fs::write(&broken_source, b"this is not a PPM image").expect("write broken PPM");
    let output = run(&[
        "project",
        "import",
        path_text(&root),
        path_text(&broken_source),
        "--id",
        "broken-texture",
        "--type",
        "texture",
    ]);
    assert_failure(&output, "failed cook import rollback");
    assert!(!root.join("assets/source/broken.ppm").exists());
    assert!(!root.join("build/cooked/broken-texture.cooked").exists());
    assert_eq!(std::fs::read(&manifest_path).unwrap(), stable_manifest);
    assert_eq!(std::fs::read(&cooked_path).unwrap(), stable_cooked);

    let output = run(&["project", "check", path_text(&root)]);
    assert_success(&output, "project check after failed imports");

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(external);
}
