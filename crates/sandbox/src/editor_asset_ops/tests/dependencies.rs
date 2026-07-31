    #[test]
    fn source_dependency_extractor_covers_every_identity_bearing_source_field() {
        let fixture = Fixture::new();
        let material_source = MaterialSource {
            schema: MATERIAL_SOURCE_SCHEMA.into(),
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.5,
            ambient_occlusion: 1.0,
            emissive: [0.0; 3],
            base_color_texture: Some("material-texture".into()),
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: Default::default(),
            transparency: "Opaque".into(),
            alpha_cutoff: 0.5,
            double_sided: false,
        };
        let material = fixture.declare_asset(
            "material-reference-source",
            AssetType::Material,
            "material-reference-source.material.json",
            &serde_json::to_vec_pretty(&material_source).expect("serialize material"),
        );

        let mut scene = engine_scene::Scene::load_from_file(
            &fixture.root().join("assets/scenes/main.scene.ron"),
        )
        .expect("load fixture scene");
        scene.dependencies = vec![AssetId::new("scene-explicit")];
        scene.scene_settings.environment_map = Some(AssetId::new("scene-environment"));
        scene.entities[0].components.insert(
            "test.asset".into(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([(
                    "asset".into(),
                    Value::Asset(AssetId::new("scene-component")),
                )]),
            },
        );
        let scene_path = fixture.root().join("assets/source/reference.scene.ron");
        scene.save_to_file(&scene_path).expect("save source scene");
        let scene = fixture.declare_asset(
            "scene-reference-source",
            AssetType::Scene,
            "reference.scene.ron",
            &std::fs::read(scene_path).expect("read source scene"),
        );

        let prefab_source = prefab_source(
            "prefab-reference-source",
            "prefab-hierarchy",
            "prefab-default",
            "prefab-child",
        );
        let prefab = fixture.declare_asset(
            "prefab-reference-source",
            AssetType::Prefab,
            "prefab-reference-source.prefab.ron",
            prefab_source.as_bytes(),
        );
        let logic = fixture.declare_asset(
            "logic-reference-source",
            AssetType::Logic,
            "logic-reference-source.logic.json",
            &logic_source(
                "logic-reference-source",
                "logic-property",
                "logic-default",
                "logic-condition",
            ),
        );
        let project = load_project(fixture.root()).expect("load project");

        assert_eq!(
            source_asset_dependencies(&project, &material).expect("material dependencies"),
            BTreeSet::from([AssetId::new("material-texture")])
        );
        let scene_dependencies =
            source_asset_dependencies(&project, &scene).expect("scene dependencies");
        for id in ["scene-explicit", "scene-environment", "scene-component"] {
            assert!(
                scene_dependencies.contains(&AssetId::new(id)),
                "missing {id}"
            );
        }
        assert_eq!(
            source_asset_dependencies(&project, &prefab).expect("prefab dependencies"),
            BTreeSet::from([
                AssetId::new("prefab-hierarchy"),
                AssetId::new("prefab-default"),
                AssetId::new("prefab-child"),
            ])
        );
        assert_eq!(
            source_asset_dependencies(&project, &logic).expect("logic dependencies"),
            BTreeSet::from([
                AssetId::new("logic-property"),
                AssetId::new("logic-default"),
                AssetId::new("logic-condition"),
            ])
        );
    }

    #[test]
    fn delete_refuses_manifest_material_references() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Referenced");
        let source = MaterialSource {
            schema: MATERIAL_SOURCE_SCHEMA.into(),
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.5,
            ambient_occlusion: 1.0,
            emissive: [0.0; 3],
            base_color_texture: Some(target.asset_id.id.clone()),
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: Default::default(),
            transparency: "Opaque".into(),
            alpha_cutoff: 0.5,
            double_sided: false,
        };
        fixture.declare_asset(
            "referencing-material",
            AssetType::Material,
            "referencing-material.material.json",
            &serde_json::to_vec_pretty(&source).expect("serialize material"),
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("material dependency must block delete");

        assert!(error.contains("material:referencing-material"));
        assert!(target.source_path.is_file());
    }

    #[test]
    fn delete_refuses_manifest_scene_references() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Referenced");
        let mut scene = engine_scene::Scene::load_from_file(
            &fixture.root().join("assets/scenes/main.scene.ron"),
        )
        .expect("load scene");
        scene.scene_settings.environment_map = Some(target.asset_id.clone());
        let source = fixture.root().join("assets/source/referencing.scene.ron");
        scene.save_to_file(&source).expect("save source scene");
        fixture.declare_asset(
            "referencing-scene",
            AssetType::Scene,
            "referencing.scene.ron",
            &std::fs::read(source).expect("read source scene"),
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("source scene dependency must block delete");

        assert!(error.contains("source-scene:referencing-scene"));
        assert!(target.source_path.is_file());
    }

    #[test]
    fn delete_refuses_prefab_hierarchy_default_and_child_references() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Referenced");
        let source = prefab_source(
            "referencing-prefab",
            &target.asset_id.id,
            &target.asset_id.id,
            &target.asset_id.id,
        );
        fixture.declare_asset(
            "referencing-prefab",
            AssetType::Prefab,
            "referencing-prefab.prefab.ron",
            source.as_bytes(),
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("prefab dependency must block delete");

        assert!(error.contains("prefab:referencing-prefab"));
        assert!(target.source_path.is_file());
    }

    #[test]
    fn delete_refuses_logic_property_default_and_condition_references() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Referenced");
        fixture.declare_asset(
            "referencing-logic",
            AssetType::Logic,
            "referencing-logic.logic.json",
            &logic_source(
                "referencing-logic",
                &target.asset_id.id,
                &target.asset_id.id,
                &target.asset_id.id,
            ),
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("logic dependency must block delete");

        assert!(error.contains("logic:referencing-logic"));
        assert!(target.source_path.is_file());
    }

    #[test]
    fn delete_fails_closed_when_a_dependency_source_cannot_be_parsed() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Keep Safe");
        fixture.declare_asset(
            "broken-prefab",
            AssetType::Prefab,
            "broken.prefab.ron",
            b"(not valid prefab source)",
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("uninspectable dependency source must block delete");

        assert!(error.contains("could not inspect prefab 'broken-prefab'"));
        assert!(target.source_path.is_file());
        assert!(target.cooked_path.is_file());
    }

    fn walk_files(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory).expect("enumerate trash") {
                let path = entry.expect("trash entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files
    }
