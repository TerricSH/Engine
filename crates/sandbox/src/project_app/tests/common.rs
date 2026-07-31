    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use engine_asset::project::ProjectManifest;

    use super::*;

    fn scene_project_fixture() -> (tempfile::TempDir, GameProject) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let scene_dir = root.join("assets/scenes");
        let source = root.join("assets/source");
        let cooked = root.join("build/cooked");
        std::fs::create_dir_all(&scene_dir).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cooked).unwrap();

        let main_path = scene_dir.join("main.scene.ron");
        let level_path = scene_dir.join("level_two.scene.ron");
        let mut main = engine_scene::sample_scene();
        main.scene_id = "main".into();
        main.save_to_file(&main_path).unwrap();
        let mut level = engine_scene::sample_scene();
        level.scene_id = "level_two".into();
        level.name = "Level Two".into();
        level.save_to_file(&level_path).unwrap();

        let mut manifest = ProjectManifest::new("Scene Transition Test");
        manifest.startup_scene = PathBuf::from("main");
        manifest.input_actions = None;
        manifest.scenes = BTreeMap::from([
            ("main".into(), PathBuf::from("assets/scenes/main.scene.ron")),
            (
                "level_two".into(),
                PathBuf::from("assets/scenes/level_two.scene.ron"),
            ),
        ]);
        let manifest_path = manifest.write_to_root(&root).unwrap();
        assert!(manifest_path.is_file());
        let project = GameProject::load(&root).unwrap();
        (temp, project)
    }

    fn transform_record(translation: [f32; 3]) -> engine_scene::ComponentRecord {
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                (
                    "translation".into(),
                    engine_serialize::Value::Vec3(translation),
                ),
                (
                    "rotation".into(),
                    engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
                ),
                ("scale".into(), engine_serialize::Value::Vec3([1.0; 3])),
            ]),
        }
    }

    fn cube_renderable_record() -> engine_scene::ComponentRecord {
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                (
                    "mesh".into(),
                    engine_serialize::Value::Asset(engine_serialize::AssetId::new("mesh-cube")),
                ),
                (
                    "material".into(),
                    engine_serialize::Value::Asset(engine_serialize::AssetId::new("mat-default")),
                ),
                ("visible".into(), engine_serialize::Value::Bool(true)),
                (
                    "render_layer".into(),
                    engine_serialize::Value::Str("Default".into()),
                ),
                ("cast_shadows".into(), engine_serialize::Value::Bool(true)),
            ]),
        }
    }

    /// Project with a world partition: `cell_two` covers the origin and
    /// streams `level_two` (one cube with unique IDs) around the camera.
    fn cell_streaming_project_fixture() -> (tempfile::TempDir, GameProject) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let scene_dir = root.join("assets/scenes");
        let source = root.join("assets/source");
        let cooked = root.join("build/cooked");
        std::fs::create_dir_all(&scene_dir).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cooked).unwrap();

        // Startup scene: the sample scene plus a mutable camera transform.
        let mut main = engine_scene::sample_scene();
        main.scene_id = "main".into();
        main.entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "camera-main")
            .expect("sample scene camera")
            .components
            .insert("engine.transform".into(), transform_record([0.0; 3]));
        main.save_to_file(&scene_dir.join("main.scene.ron"))
            .unwrap();

        // Cell scene: unique entity IDs, no camera of its own.
        let mut level_two = engine_scene::sample_scene();
        level_two.scene_id = "level_two".into();
        level_two.name = "Streamed Cell".into();
        level_two.scene_settings.active_camera = None;
        level_two.entities = vec![engine_scene::EntityRecord {
            persistent_id: "cube-two".into(),
            parent: None,
            name: Some("Streamed Cube".into()),
            enabled: true,
            components: BTreeMap::from([
                ("engine.transform".into(), transform_record([1.0, 0.0, 0.0])),
                ("engine.renderable".into(), cube_renderable_record()),
            ]),
        }];
        level_two
            .save_to_file(&scene_dir.join("level_two.scene.ron"))
            .unwrap();

        std::fs::write(
            root.join(engine_asset::partition::WORLD_PARTITION_FILE_NAME),
            format!(
                "{{ \"schema\": \"{}\", \"cells\": {{ \"cell_two\": {{ \"scene\": \"level_two\", \"bounds\": {{ \"center\": [0.0, 0.0, 0.0], \"half_extents\": [10.0, 10.0, 10.0] }} }} }} }}\n",
                engine_asset::partition::WORLD_PARTITION_SCHEMA
            ),
        )
        .unwrap();

        let mut manifest = ProjectManifest::new("Cell Streaming Test");
        manifest.startup_scene = PathBuf::from("main");
        manifest.input_actions = None;
        manifest.scenes = BTreeMap::from([
            ("main".into(), PathBuf::from("assets/scenes/main.scene.ron")),
            (
                "level_two".into(),
                PathBuf::from("assets/scenes/level_two.scene.ron"),
            ),
        ]);
        manifest.write_to_root(&root).unwrap();
        let project = GameProject::load(&root).unwrap();
        (temp, project)
    }

    fn set_main_camera_position(game_loop: &mut GameLoop, position: [f32; 3]) {
        game_loop.runtime.with_world_mut(|world| {
            let camera = world.entity_by_persistent_id("camera-main").unwrap();
            world
                .get_mut::<engine_scene::components::Transform>(camera)
                .unwrap()
                .translation = glam::Vec3::from(position);
        });
    }

    /// Project whose startup scene opts into origin shifting: threshold 100,
    /// camera at x = 150 (past the threshold), and a visible cube five metres
    /// in front of the camera so draw calls keep flowing after the shift.
    fn origin_shift_project_fixture() -> (tempfile::TempDir, GameProject) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let scene_dir = root.join("assets/scenes");
        let source = root.join("assets/source");
        let cooked = root.join("build/cooked");
        std::fs::create_dir_all(&scene_dir).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cooked).unwrap();

        let mut main = engine_scene::sample_scene();
        main.scene_id = "main".into();
        main.entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "camera-main")
            .expect("sample scene camera")
            .components
            .insert(
                "engine.transform".into(),
                transform_record([150.0, 0.0, 0.0]),
            );
        main.entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .expect("sample scene cube")
            .components
            .insert(
                "engine.transform".into(),
                transform_record([150.0, 0.0, -5.0]),
            );
        main.scene_settings.origin_shift.enabled = true;
        main.scene_settings.origin_shift.threshold = 100.0;
        main.save_to_file(&scene_dir.join("main.scene.ron"))
            .unwrap();

        let mut manifest = ProjectManifest::new("Origin Shift Test");
        manifest.startup_scene = PathBuf::from("main");
        manifest.input_actions = None;
        manifest.scenes =
            BTreeMap::from([("main".into(), PathBuf::from("assets/scenes/main.scene.ron"))]);
        manifest.write_to_root(&root).unwrap();
        let project = GameProject::load(&root).unwrap();
        (temp, project)
    }

    fn has_persistent_entity(game_loop: &GameLoop, id: &str) -> bool {
        game_loop
            .runtime
            .with_world(|world| world.entity_by_persistent_id(id).is_some())
            .unwrap_or(false)
    }
