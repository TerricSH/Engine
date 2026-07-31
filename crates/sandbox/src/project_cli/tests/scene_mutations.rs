#[test]
fn duplicate_project_scene_copies_authoring_data_and_catalogs_new_id() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("duplicate-scene");
    create_project(&root, Some("Duplicate Scene"), false).unwrap();
    let project = GameProject::load(&root).unwrap();
    let mut source = Scene::load_from_file(project.startup_scene_path()).unwrap();
    source.name = "Authored Level".to_string();
    source.entities[0].name = Some("Changed In Memory".to_string());

    let duplicate = duplicate_project_scene(&root, "level_copy", &source).unwrap();
    let copied = Scene::load_from_file(&duplicate).unwrap();
    let reloaded = GameProject::load(&root).unwrap();

    assert_eq!(copied.scene_id, "level_copy");
    assert_eq!(copied.name, "Authored Level");
    assert_eq!(
        copied.entities[0].name.as_deref(),
        Some("Changed In Memory")
    );
    assert_eq!(
        reloaded.scene_path("level_copy").as_deref(),
        Some(duplicate.as_path())
    );
}

#[test]
fn editor_scene_creation_uses_a_safe_prefilled_subfolder() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scene-subfolder");
    create_project(&root, Some("Scene Subfolder"), false).unwrap();

    let created =
        create_project_scene_in_folder(&root, "level_one", None, Path::new("levels/campaign"))
            .unwrap();
    assert_eq!(
        created,
        root.join("assets/scenes/levels/campaign/level_one.scene.ron")
    );
    assert_eq!(
        GameProject::load(&root)
            .unwrap()
            .scene_path("level_one")
            .as_deref(),
        Some(created.as_path())
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("game.project.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["scenes"]["level_one"],
        "assets/scenes/levels/campaign/level_one.scene.ron"
    );
    assert!(
        create_project_scene_in_folder(&root, "escape", None, Path::new("../outside"),).is_err()
    );
    assert!(create_project_scene_in_folder(&root, "reserved", None, Path::new("CON"),).is_err());
}

#[test]
fn renames_project_scene_content_identity_path_and_startup_transactionally() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("rename-scene");
    create_project(&root, Some("Rename Scene"), false).unwrap();
    let old_path = create_project_scene(&root, "level_old", None).unwrap();
    let mut scene = Scene::load_from_file(&old_path).unwrap();
    scene.entities[0].name = Some("Authored Entity".into());
    scene.save_to_file(&old_path).unwrap();
    set_project_startup_scene(&root, "level_old").unwrap();

    let renamed_path = rename_project_scene(&root, "level_old", "level_new").unwrap();
    let renamed = Scene::load_from_file(&renamed_path).unwrap();
    let project = GameProject::load(&root).unwrap();

    assert!(!old_path.exists());
    assert_eq!(renamed_path, root.join("assets/scenes/level_new.scene.ron"));
    assert_eq!(renamed.scene_id, "level_new");
    assert_eq!(renamed.name, "level_new");
    assert_eq!(renamed.entities[0].name.as_deref(), Some("Authored Entity"));
    assert_eq!(project.startup_scene_id(), "level_new");
    assert_eq!(
        project.scene_path("level_new").as_deref(),
        Some(renamed_path.as_path())
    );
    assert!(project.scene_path("level_old").is_none());

    let custom_path =
        create_project_scene(&root, "authored_old", Some("Authored Display Name")).unwrap();
    let custom_renamed = rename_project_scene(&root, "authored_old", "authored_new").unwrap();
    assert!(!custom_path.exists());
    assert_eq!(
        Scene::load_from_file(&custom_renamed).unwrap().name,
        "Authored Display Name"
    );
}

#[test]
fn scene_rename_rejects_portable_id_and_file_collisions_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("rename-collision");
    create_project(&root, Some("Rename Collision"), false).unwrap();
    let alpha = create_project_scene(&root, "alpha", None).unwrap();
    create_project_scene(&root, "beta", None).unwrap();
    let manifest_path = root.join("game.project.json");
    let original_manifest = std::fs::read(&manifest_path).unwrap();
    let original_alpha = std::fs::read(&alpha).unwrap();

    let error = rename_project_scene(&root, "alpha", "BETA").unwrap_err();
    assert!(error.contains("collides"));
    assert_eq!(std::fs::read(&manifest_path).unwrap(), original_manifest);
    assert_eq!(std::fs::read(&alpha).unwrap(), original_alpha);

    let orphan = root.join("assets/scenes/orphan.scene.ron");
    std::fs::copy(&alpha, &orphan).unwrap();
    let error = rename_project_scene(&root, "alpha", "orphan").unwrap_err();
    assert!(error.contains("already exists"));
    assert_eq!(std::fs::read(&manifest_path).unwrap(), original_manifest);
    assert_eq!(std::fs::read(&alpha).unwrap(), original_alpha);

    let error = rename_project_scene(&root, "alpha", "CON").unwrap_err();
    assert!(error.contains("reserved"));
    GameProject::load(&root).unwrap();
}

#[test]
fn deleting_scene_requires_safe_startup_replacement_and_writes_recovery_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("delete-scene");
    create_project(&root, Some("Delete Scene"), false).unwrap();

    let error = delete_project_scene(&root, "main", None).unwrap_err();
    assert!(error.contains("retain at least one"));

    let replacement = create_project_scene(&root, "replacement", None).unwrap();
    let project = GameProject::load(&root).unwrap();
    let original_path = project.scene_path("main").unwrap();
    let original_scene = std::fs::read(&original_path).unwrap();
    let error = delete_project_scene(&root, "main", None).unwrap_err();
    assert!(error.contains("explicit replacement"));
    let error = delete_project_scene(&root, "main", Some("missing")).unwrap_err();
    assert!(error.contains("unknown project scene"));
    assert!(original_path.is_file());

    let deleted = delete_project_scene(&root, "main", Some("replacement")).unwrap();
    let reloaded = GameProject::load(&root).unwrap();
    let metadata: SceneTrashMetadata = serde_json::from_slice(
        &std::fs::read(&deleted.metadata_path).expect("read scene trash metadata"),
    )
    .expect("parse scene trash metadata");

    assert!(!original_path.exists());
    assert!(replacement.is_file());
    assert_eq!(reloaded.startup_scene_id(), "replacement");
    assert_eq!(reloaded.scenes().len(), 1);
    assert_eq!(deleted.scene_id, "main");
    assert_eq!(deleted.replacement_startup.as_deref(), Some("replacement"));
    assert_eq!(
        std::fs::read(deleted.trash_directory.join("scene.scene.ron")).unwrap(),
        original_scene
    );
    assert_eq!(metadata.schema, SCENE_TRASH_SCHEMA);
    assert_eq!(metadata.scene_id, "main");
    assert_eq!(metadata.original_scene_path, "assets/scenes/main.scene.ron");
    assert!(metadata.was_startup);
    assert_eq!(metadata.replacement_startup.as_deref(), Some("replacement"));

    let manifest: ProjectManifest =
        serde_json::from_slice(&std::fs::read(root.join("game.project.json")).unwrap()).unwrap();
    manifest.validate().unwrap();
}

#[test]
fn scene_rename_and_delete_roll_back_every_touched_file() {
    let temp = tempfile::tempdir().unwrap();
    let rename_root = temp.path().join("rename-rollback");
    create_project(&rename_root, Some("Rename Rollback"), false).unwrap();
    let old_path = create_project_scene(&rename_root, "old", None).unwrap();
    set_project_startup_scene(&rename_root, "old").unwrap();
    let manifest_path = rename_root.join("game.project.json");
    let original_manifest = std::fs::read(&manifest_path).unwrap();
    let original_scene = std::fs::read(&old_path).unwrap();

    let error = rename_project_scene_impl(&rename_root, "old", "new", Some(3)).unwrap_err();
    assert!(error.contains("injected scene transaction failure"));
    assert_eq!(std::fs::read(&manifest_path).unwrap(), original_manifest);
    assert_eq!(std::fs::read(&old_path).unwrap(), original_scene);
    assert!(!rename_root.join("assets/scenes/new.scene.ron").exists());
    let rename_project = GameProject::load(&rename_root).unwrap();
    assert_eq!(rename_project.startup_scene_id(), "old");
    assert!(rename_project.scene_path("new").is_none());

    let delete_root = temp.path().join("delete-rollback");
    create_project(&delete_root, Some("Delete Rollback"), false).unwrap();
    create_project_scene(&delete_root, "replacement", None).unwrap();
    let delete_project = GameProject::load(&delete_root).unwrap();
    let main_path = delete_project.scene_path("main").unwrap();
    let delete_manifest_path = delete_root.join("game.project.json");
    let original_manifest = std::fs::read(&delete_manifest_path).unwrap();
    let original_scene = std::fs::read(&main_path).unwrap();

    let error =
        delete_project_scene_impl(&delete_root, "main", Some("replacement"), Some(4)).unwrap_err();
    assert!(error.contains("injected scene transaction failure"));
    assert_eq!(
        std::fs::read(&delete_manifest_path).unwrap(),
        original_manifest
    );
    assert_eq!(std::fs::read(&main_path).unwrap(), original_scene);
    let trash_root = delete_root.join(".engine/trash/scenes");
    assert_eq!(std::fs::read_dir(&trash_root).unwrap().count(), 0);
    let delete_project = GameProject::load(&delete_root).unwrap();
    assert_eq!(delete_project.startup_scene_id(), "main");
    assert!(delete_project.scene_path("main").is_some());
}

#[test]
fn scene_mutations_refuse_an_existing_cross_process_lock() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scene-lock");
    create_project(&root, Some("Scene Lock"), false).unwrap();
    let lock_directory = root.join(".engine/locks");
    std::fs::create_dir_all(&lock_directory).unwrap();
    let lock_path = lock_directory.join("scene-operations.lock");
    std::fs::write(&lock_path, "owned by another process\n").unwrap();

    let error = create_project_scene(&root, "blocked", None).unwrap_err();
    assert!(error.contains("another project scene operation is active"));
    assert!(!root.join("assets/scenes/blocked.scene.ron").exists());
    assert!(GameProject::load(&root)
        .unwrap()
        .scene_path("blocked")
        .is_none());

    std::fs::remove_file(lock_path).unwrap();
}
