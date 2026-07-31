    #[test]
    fn duplicate_rewrites_scene_identity_and_preserves_asset_dependencies() {
        let fixture = Fixture::new();
        let mut scene = engine_scene::Scene::load_from_file(
            &fixture.root().join("assets/scenes/main.scene.ron"),
        )
        .expect("load fixture scene");
        scene.scene_id = "scene-original".into();
        scene.dependencies = vec![AssetId::new("external-dependency")];
        scene.scene_settings.environment_map = Some(AssetId::new("external-environment"));
        let source = fixture
            .root()
            .join("assets/source/scene-original.scene.ron");
        scene.save_to_file(&source).expect("write scene source");
        let original = fixture.declare_asset(
            "scene-original",
            AssetType::Scene,
            "scene-original.scene.ron",
            &std::fs::read(&source).expect("read scene source"),
        );

        let duplicate = duplicate_project_asset(fixture.root(), &original.id).expect("duplicate");
        let duplicated =
            engine_scene::Scene::load_from_file(&duplicate.source_path).expect("load duplicate");

        assert_eq!(duplicated.scene_id, duplicate.asset_id.id);
        assert_eq!(duplicated.dependencies, scene.dependencies);
        assert_eq!(
            duplicated.scene_settings.environment_map,
            scene.scene_settings.environment_map
        );
    }

    #[test]
    fn duplicate_rejects_assets_without_an_explicit_identity_policy() {
        let fixture = Fixture::new();
        let original = fixture.declare_asset(
            "unknown-original",
            AssetType::Unknown,
            "unknown-original.data",
            b"opaque identity-bearing payload",
        );
        let manifest_before = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");

        let error = duplicate_project_asset(fixture.root(), &original.id)
            .expect_err("unknown identity policy must fail closed");

        assert!(error.contains("no declared duplicate identity policy"));
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("unchanged manifest"),
            manifest_before
        );
        assert!(!fixture
            .root()
            .join("assets/source/unknown-original-copy.data")
            .exists());
    }

    #[test]
    fn move_preserves_asset_id_and_updates_source_manifest() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Materials")).expect("folder");
        let original = fixture.create_material("Movable");
        let old_source = original.source_path.clone();
        let moved = move_project_asset(
            fixture.root(),
            &original.asset_id,
            Path::new("Materials/Renamed.material.json"),
        )
        .expect("move asset");

        assert_eq!(moved.asset_id, original.asset_id);
        assert!(!old_source.exists());
        assert!(moved.source_path.is_file());
        assert!(moved.cooked_path.is_file());
        let entry = fixture
            .manifest()
            .assets
            .into_iter()
            .find(|entry| entry.id == original.asset_id)
            .expect("moved manifest entry");
        assert_eq!(entry.source_path, "Materials/Renamed.material.json");
    }

    #[test]
    fn move_commit_failure_restores_manifest_source_and_cooked_bytes() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Materials")).expect("folder");
        let original = fixture.create_material("Rollback Move");
        let manifest_before = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");
        let cooked_before = std::fs::read(&original.cooked_path).expect("cooked snapshot");

        let error = move_project_asset_impl(
            fixture.root(),
            &original.asset_id,
            Path::new("Materials/Rollback.material.json"),
            Some(2),
        )
        .expect_err("injected move commit failure");
        assert!(error.contains("simulated asset transaction failure"));
        assert!(original.source_path.is_file());
        assert!(!fixture
            .root()
            .join("assets/source/Materials/Rollback.material.json")
            .exists());
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("restored manifest"),
            manifest_before
        );
        assert_eq!(
            std::fs::read(&original.cooked_path).expect("restored cooked"),
            cooked_before
        );
    }

    #[test]
    fn delete_moves_asset_to_project_trash_with_recovery_metadata() {
        let fixture = Fixture::new();
        let material = fixture.create_material("Disposable");
        let deleted =
            delete_project_asset(fixture.root(), &material.asset_id).expect("delete asset");

        assert!(!material.source_path.exists());
        assert!(!material.cooked_path.exists());
        assert!(deleted.trash_directory.starts_with(fixture.root()));
        assert!(deleted.metadata_path.is_file());
        assert!(deleted
            .trash_directory
            .join("source/disposable.material.json")
            .is_file());
        assert!(deleted
            .trash_directory
            .join("cooked/disposable.cooked")
            .is_file());
        let metadata: TrashMetadata =
            serde_json::from_slice(&std::fs::read(deleted.metadata_path).expect("read metadata"))
                .expect("parse metadata");
        assert_eq!(metadata.schema, TRASH_SCHEMA);
        assert_eq!(metadata.entry.id, material.asset_id);
        assert!(fixture.manifest().assets.is_empty());
    }

    #[test]
    fn delete_commit_failure_restores_live_state_and_removes_trash_payloads() {
        let fixture = Fixture::new();
        let material = fixture.create_material("Keep Me");
        let manifest_before = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");
        let source_before = std::fs::read(&material.source_path).expect("source snapshot");
        let cooked_before = std::fs::read(&material.cooked_path).expect("cooked snapshot");

        // Four writes install the trash payloads and updated manifest; fail
        // after removing the live source so rollback must restore real project
        // state, not merely clean up an uncommitted staging area.
        let error = delete_project_asset_impl(fixture.root(), &material.asset_id, Some(5))
            .expect_err("injected delete failure");
        assert!(error.contains("simulated asset transaction failure"));
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("restored manifest"),
            manifest_before
        );
        assert_eq!(
            std::fs::read(&material.source_path).expect("restored source"),
            source_before
        );
        assert_eq!(
            std::fs::read(&material.cooked_path).expect("restored cooked"),
            cooked_before
        );
        let trash_root = fixture.root().join(".engine/trash/assets");
        let remaining_files = if trash_root.exists() {
            walk_files(&trash_root)
        } else {
            Vec::new()
        };
        assert!(
            remaining_files.is_empty(),
            "trash payloads: {remaining_files:?}"
        );
    }

    #[test]
    fn delete_refuses_scene_references_without_mutating_asset() {
        let fixture = Fixture::new();
        let material = fixture.create_material("Referenced");
        let scene_path = fixture.root().join("assets/scenes/main.scene.ron");
        let mut scene = engine_scene::Scene::load_from_file(&scene_path).expect("load scene");
        scene.dependencies.push(material.asset_id.clone());
        scene
            .save_to_file(&scene_path)
            .expect("save referenced scene");
        let manifest_before = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");

        let error = delete_project_asset(fixture.root(), &material.asset_id)
            .expect_err("referenced asset must be retained");
        assert!(error.contains("scene:main"));
        assert!(material.source_path.is_file());
        assert!(material.cooked_path.is_file());
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("unchanged manifest"),
            manifest_before
        );
    }
