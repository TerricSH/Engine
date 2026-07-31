    #[test]
    fn folder_creation_is_single_step_and_refuses_conflicts() {
        let fixture = Fixture::new();
        let materials =
            create_asset_folder(fixture.root(), Path::new("Materials")).expect("create folder");
        assert!(materials.is_dir());
        let original_entries = std::fs::read_dir(&materials)
            .expect("read materials")
            .count();
        let error = create_asset_folder(fixture.root(), Path::new("materials"))
            .expect_err("case collision must fail");
        assert!(error.contains("differs only by case") || error.contains("already exists"));
        assert_eq!(
            std::fs::read_dir(materials)
                .expect("read unchanged materials")
                .count(),
            original_entries
        );
    }

    #[test]
    fn folder_rename_updates_declared_source_paths_and_preserves_ids() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Materials")).expect("source folder");
        let material = create_material_asset(
            fixture.root(),
            Path::new("Materials"),
            "Ground",
            &MaterialTemplate::default(),
        )
        .expect("material");

        let renamed = rename_asset_folder(
            fixture.root(),
            Path::new("Materials"),
            Path::new("Environment"),
        )
        .expect("rename folder");

        assert!(renamed.ends_with("assets/source/Environment"));
        assert!(!fixture.root().join("assets/source/Materials").exists());
        assert!(fixture
            .root()
            .join("assets/source/Environment/ground.material.json")
            .is_file());
        let entry = fixture
            .manifest()
            .assets
            .into_iter()
            .find(|entry| entry.id == material.asset_id)
            .expect("renamed manifest entry");
        assert_eq!(entry.source_path, "Environment/ground.material.json");
        assert!(material.cooked_path.is_file());
    }

    #[test]
    fn folder_rename_rejects_escape_self_and_case_collisions() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Models")).expect("models folder");
        create_asset_folder(fixture.root(), Path::new("Other")).expect("other folder");

        assert!(
            rename_asset_folder(fixture.root(), Path::new("Models"), Path::new("../outside"),)
                .expect_err("escape must fail")
                .contains("project-relative")
        );
        assert!(rename_asset_folder(
            fixture.root(),
            Path::new("Models"),
            Path::new("mOdElS/Nested"),
        )
        .expect_err("self move must fail")
        .contains("current parent"));
        assert!(
            rename_asset_folder(fixture.root(), Path::new("Models"), Path::new("other"),)
                .expect_err("case collision must fail")
                .contains("already exists")
        );
        assert!(fixture.root().join("assets/source/Models").is_dir());
    }

    #[test]
    fn folder_rename_rejects_cross_parent_moves_to_protect_relative_sidecars() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Models")).expect("models folder");
        create_asset_folder(fixture.root(), Path::new("Packages")).expect("packages folder");

        let error = rename_asset_folder(
            fixture.root(),
            Path::new("Models"),
            Path::new("Packages/RenamedModels"),
        )
        .expect_err("cross-parent move must fail");

        assert!(error.contains("current parent"));
        assert!(fixture.root().join("assets/source/Models").is_dir());
        assert!(!fixture
            .root()
            .join("assets/source/Packages/RenamedModels")
            .exists());
    }

    #[test]
    fn folder_rename_moves_nested_manifest_files_without_recreating_old_folder() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Packages")).expect("package folder");
        let nested_manifest = fixture.root().join("assets/source/Packages/local.manifest");
        std::fs::write(
            &nested_manifest,
            serde_json::to_vec_pretty(&SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: Vec::new(),
            })
            .expect("serialize nested manifest"),
        )
        .expect("write nested manifest");

        rename_asset_folder(fixture.root(), Path::new("Packages"), Path::new("Vendor"))
            .expect("rename folder with nested manifest file");

        assert!(!fixture.root().join("assets/source/Packages").exists());
        assert!(fixture
            .root()
            .join("assets/source/Vendor/local.manifest")
            .is_file());
    }

    #[test]
    fn folder_delete_only_removes_empty_non_root_folders() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Empty")).expect("empty folder");
        delete_asset_folder(fixture.root(), Path::new("Empty")).expect("delete empty folder");
        assert!(!fixture.root().join("assets/source/Empty").exists());

        create_asset_folder(fixture.root(), Path::new("Occupied")).expect("occupied folder");
        std::fs::write(
            fixture.root().join("assets/source/Occupied/readme.txt"),
            b"keep",
        )
        .expect("write occupant");
        let error = delete_asset_folder(fixture.root(), Path::new("Occupied"))
            .expect_err("non-empty folder must not be recursively deleted");
        assert!(error.contains("not empty"));
        assert!(fixture
            .root()
            .join("assets/source/Occupied/readme.txt")
            .is_file());
        assert!(delete_asset_folder(fixture.root(), Path::new(""))
            .expect_err("source root is protected")
            .contains("may not be empty"));
    }

    #[test]
    fn material_create_writes_manifest_source_and_valid_cooked_artifact() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Materials")).expect("material folder");
        let created = create_material_asset(
            fixture.root(),
            Path::new("Materials"),
            "Hero Surface",
            &MaterialTemplate::default(),
        )
        .expect("create material");

        assert_eq!(created.asset_id.id, "hero-surface");
        assert!(created
            .source_path
            .ends_with("Materials/hero-surface.material.json"));
        assert!(created.source_path.is_file());
        let artifact = read_cooked_artifact(&created.cooked_path).expect("valid cooked material");
        assert_eq!(artifact.header.asset_kind, AssetType::Material.kind_code());
        let manifest = fixture.manifest();
        let entry = manifest
            .assets
            .iter()
            .find(|entry| entry.id == created.asset_id)
            .expect("manifest entry");
        assert_eq!(entry.source_path, "Materials/hero-surface.material.json");
    }

    #[test]
    fn failed_material_cook_rolls_back_every_live_file() {
        let fixture = Fixture::new();
        let original_manifest = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");
        let invalid = MaterialTemplate {
            metallic: 2.0,
            ..MaterialTemplate::default()
        };
        let error =
            create_material_asset(fixture.root(), Path::new(""), "Broken Material", &invalid)
                .expect_err("invalid material must not commit");
        assert!(error.contains("cooking failed"));
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("manifest after failure"),
            original_manifest
        );
        assert!(!fixture
            .root()
            .join("assets/source/broken-material.material.json")
            .exists());
        assert!(!fixture
            .root()
            .join("build/cooked/broken-material.cooked")
            .exists());
    }

    #[test]
    fn duplicate_generates_unique_stable_ids_and_cooked_assets() {
        let fixture = Fixture::new();
        let original = fixture.create_material("Ground");
        let first =
            duplicate_project_asset(fixture.root(), &original.asset_id).expect("first duplicate");
        let second =
            duplicate_project_asset(fixture.root(), &original.asset_id).expect("second duplicate");

        assert_eq!(first.asset_id.id, "ground-copy");
        assert_eq!(second.asset_id.id, "ground-copy-2");
        assert_ne!(first.source_path, second.source_path);
        assert!(first.source_path.is_file());
        assert!(second.source_path.is_file());
        read_cooked_artifact(&first.cooked_path).expect("first cooked duplicate");
        read_cooked_artifact(&second.cooked_path).expect("second cooked duplicate");
        let ids = fixture
            .manifest()
            .assets
            .into_iter()
            .map(|entry| entry.id.id)
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("ground"));
        assert!(ids.contains("ground-copy"));
        assert!(ids.contains("ground-copy-2"));
    }

    #[test]
    fn duplicate_rewrites_prefab_self_identity_and_preserves_external_references() {
        let fixture = Fixture::new();
        let source = prefab_source(
            "prefab-original",
            "mesh-external",
            "material-external",
            "prefab-child",
        );
        let original = fixture.declare_asset(
            "prefab-original",
            AssetType::Prefab,
            "prefab-original.prefab.ron",
            source.as_bytes(),
        );

        let duplicate = duplicate_project_asset(fixture.root(), &original.id).expect("duplicate");
        let duplicated = engine_scene::parse_prefab_source(
            &std::fs::read(&duplicate.source_path).expect("read duplicated prefab"),
        )
        .expect("parse duplicated prefab");

        assert_eq!(duplicated.source_asset, duplicate.asset_id);
        let component = &duplicated.hierarchy[0].components["test.component"];
        let Value::List(values) = &component.fields["asset"] else {
            panic!("nested prefab dependency must remain a list");
        };
        let Value::Map(values) = &values[0] else {
            panic!("nested prefab dependency must remain a map");
        };
        assert_eq!(
            values.get("nested"),
            Some(&Value::Asset(AssetId::new("mesh-external")))
        );
        assert_eq!(
            duplicated.component_defaults["test.component"]["default_asset"],
            Value::Asset(AssetId::new("material-external"))
        );
        assert_eq!(
            duplicated.child_prefab_refs[0].prefab_asset,
            AssetId::new("prefab-child")
        );
    }

    #[test]
    fn duplicate_rewrites_logic_self_identity_and_preserves_asset_references() {
        let fixture = Fixture::new();
        let original = fixture.declare_asset(
            "logic-original",
            AssetType::Logic,
            "logic-original.logic.json",
            &logic_source(
                "logic-original",
                "property-external",
                "default-external",
                "condition-external",
            ),
        );

        let duplicate = duplicate_project_asset(fixture.root(), &original.id).expect("duplicate");
        let duplicated: LogicAsset = serde_json::from_slice(
            &std::fs::read(&duplicate.source_path).expect("read duplicated logic"),
        )
        .expect("parse duplicated logic");

        assert_eq!(duplicated.asset_id, duplicate.asset_id.id);
        assert!(matches!(
            duplicated.nodes[0].properties.get("asset"),
            Some(LogicValue::AssetRef(asset)) if asset.id == "property-external"
        ));
        assert!(matches!(
            duplicated.parameters["asset"].default.as_ref(),
            Some(LogicValue::AssetRef(asset)) if asset.id == "default-external"
        ));
        let Some(LogicCondition::Not(condition)) = &duplicated.nodes[0].transitions[0].condition
        else {
            panic!("logic condition structure must be preserved");
        };
        let LogicCondition::And(conditions) = condition.as_ref() else {
            panic!("logic and condition must be preserved");
        };
        assert!(matches!(
            &conditions[0],
            LogicCondition::Comparison { value: LogicValue::AssetRef(asset), .. }
                if asset.id == "condition-external"
        ));
    }
