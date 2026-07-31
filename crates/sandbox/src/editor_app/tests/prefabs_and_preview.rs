#[test]
fn loaded_prefab_instantiation_and_unpack_use_editor_history() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    let asset_id = AssetId::new("prefab-cube-test");
    let prefab = engine_editor::prefab_from_scene_subtree(
        &app.editor_scene.as_ref().unwrap().scene,
        &"cube-01".into(),
        asset_id.clone(),
    )
    .unwrap();
    app.game_loop
        .as_mut()
        .unwrap()
        .runtime
        .asset_registry_mut()
        .insert_typed(asset_id.clone(), prefab);
    write_json(
        &app.project.asset_source.join("prefabs.manifest"),
        &SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: vec![SourceAssetEntry {
                id: asset_id.clone(),
                asset_type: AssetType::Prefab,
                source_path: "prefabs/cube.prefab.ron".to_string(),
                cook_rules: engine_asset::cook::CookRules::default(),
            }],
        },
    );
    app.refresh_asset_catalog().unwrap();
    assert!(app.asset_browser.select_asset(Some(asset_id.clone())));

    app.instantiate_prefab_asset(asset_id, None).unwrap();

    let root = app
        .editor_scene
        .as_ref()
        .unwrap()
        .selected_entity
        .clone()
        .expect("instantiated root is selected");
    assert_ne!(root, "cube-01");
    assert!(app
        .editor_scene
        .as_ref()
        .unwrap()
        .scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == root)
        .unwrap()
        .components
        .contains_key("engine.prefab_instance_ref"));

    app.unpack_prefab_instance(root.clone(), PrefabUnpackMode::Instance)
        .unwrap();
    assert!(!app
        .editor_scene
        .as_ref()
        .unwrap()
        .scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == root)
        .unwrap()
        .components
        .contains_key("engine.prefab_instance_ref"));
    app.editor_scene.as_mut().unwrap().undo().unwrap();
    assert!(app
        .editor_scene
        .as_ref()
        .unwrap()
        .scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == root)
        .unwrap()
        .components
        .contains_key("engine.prefab_instance_ref"));
}

struct MaterialProjectFixture {
    _temp: tempfile::TempDir,
    project: GameProject,
    source_path: PathBuf,
    manifest_entry: SourceAssetEntry,
}

fn test_material_source(roughness: f32, base_color: [f32; 4]) -> MaterialSource {
    MaterialSource {
        schema: MATERIAL_SOURCE_SCHEMA.to_string(),
        base_color,
        metallic: 0.2,
        roughness,
        ambient_occlusion: 0.9,
        emissive: [0.0; 3],
        base_color_texture: None,
        normal_texture: None,
        metallic_roughness_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        transparency: "Opaque".to_string(),
        alpha_cutoff: 0.5,
        double_sided: false,
        advanced: Default::default(),
    }
}

fn write_json(path: &Path, value: &impl serde::Serialize) {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    std::fs::write(path, bytes).unwrap();
}

fn material_project_fixture() -> MaterialProjectFixture {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let source_root = root.join("assets/source");
    let material_dir = source_root.join("materials");
    let cooked_assets = root.join("assets/cooked");
    std::fs::create_dir_all(&material_dir).unwrap();
    std::fs::create_dir_all(&cooked_assets).unwrap();

    let source_path = material_dir.join("project.material.json");
    write_json(
        &source_path,
        &test_material_source(0.7, [0.2, 0.3, 0.4, 1.0]),
    );
    let manifest_entry = SourceAssetEntry {
        id: AssetId::new("mat-project"),
        asset_type: AssetType::Material,
        source_path: "materials/project.material.json".to_string(),
        cook_rules: engine_asset::cook::CookRules::default(),
    };
    write_json(
        &source_root.join("assets.manifest"),
        &SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: vec![manifest_entry.clone()],
        },
    );

    let manifest = ProjectManifest::new("Material Save Test");
    let project = GameProject {
        manifest,
        manifest_path: root.join("game.project.json"),
        root: root.clone(),
        startup_scene: root.join("assets/scenes/main.scene.ron"),
        asset_source: std::fs::canonicalize(&source_root).unwrap(),
        cooked_assets,
        script_project: None,
        script_assembly: None,
        input_actions: None,
    };
    MaterialProjectFixture {
        _temp: temp,
        project,
        source_path,
        manifest_entry,
    }
}

fn cook_fixture(fixture: &MaterialProjectFixture) {
    let mut graph = DependencyGraph::new();
    let runtime_builder = EngineRuntime::builder(EngineConfig::default());
    let report = cook_orchestrate_checked_with_registry(
        &fixture.project.asset_source,
        &fixture.project.cooked_assets,
        &mut graph,
        runtime_builder.asset_type_registry(),
    );
    assert!(report.is_success(), "{:?}", report.diagnostics);
}

#[test]
fn missing_render_reference_uses_preview_fallback_without_mutating_authoring_scene() {
    let runtime = engine_core::EngineRuntime::new(EngineConfig::default());
    let mut authoring = engine_scene::sample_scene();
    let renderable = authoring
        .entities
        .iter_mut()
        .find_map(|entity| entity.components.get_mut("engine.renderable"))
        .unwrap();
    renderable.fields.insert(
        "material".into(),
        Value::Asset(AssetId::new("missing-material")),
    );

    let (preview, diagnostics) = editor_preview_scene(&runtime, &authoring);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "EDASSET_MISSING");
    let authoring_material = authoring.entities.iter().find_map(|entity| {
        entity
            .components
            .get("engine.renderable")
            .and_then(|component| component.fields.get("material"))
    });
    let preview_material = preview.entities.iter().find_map(|entity| {
        entity
            .components
            .get("engine.renderable")
            .and_then(|component| component.fields.get("material"))
    });
    assert_eq!(
        authoring_material,
        Some(&Value::Asset(AssetId::new("missing-material")))
    );
    assert_eq!(
        preview_material,
        Some(&Value::Asset(AssetId::new("mat-default")))
    );
}

#[test]
fn editor_preview_never_instantiates_authoring_game_scripts() {
    let runtime = engine_core::EngineRuntime::new(EngineConfig::default());
    let mut authoring = engine_scene::sample_scene();
    let scripted = authoring
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap();
    scripted.components.insert(
        "engine.script".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::new(),
        },
    );

    let (preview, _) = editor_preview_scene(&runtime, &authoring);
    assert!(authoring
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .components
        .contains_key("engine.script"));
    assert!(!preview
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .components
        .contains_key("engine.script"));
}

#[test]
fn editor_preview_uses_dedicated_camera_without_mutating_game_camera() {
    let runtime = engine_core::EngineRuntime::new(EngineConfig::default());
    let authoring = engine_scene::sample_scene();
    let (preview, diagnostics) = editor_preview_scene(&runtime, &authoring);
    assert!(diagnostics.is_empty());
    assert_eq!(
        authoring.scene_settings.active_camera.as_deref(),
        Some("camera-main")
    );
    assert!(authoring.entities[0].components["engine.camera"].enabled);

    let editor_camera_id = preview.scene_settings.active_camera.as_deref().unwrap();
    assert!(editor_camera_id.starts_with(EDITOR_CAMERA_ID_PREFIX));
    assert_ne!(editor_camera_id, "camera-main");
    assert!(
        !preview
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "camera-main")
            .unwrap()
            .components["engine.camera"]
            .enabled
    );
    let editor_camera = preview
        .entities
        .iter()
        .find(|entity| entity.persistent_id == editor_camera_id)
        .unwrap();
    assert!(editor_camera.components["engine.camera"].enabled);
    assert!(editor_camera.components.contains_key("engine.transform"));
}

#[test]
fn game_preview_uses_authoring_camera_without_instantiating_scripts() {
    let runtime = engine_core::EngineRuntime::new(EngineConfig::default());
    let mut authoring = engine_scene::sample_scene();
    authoring.entities[1].components.insert(
        "engine.script".into(),
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: Default::default(),
        },
    );

    let (preview, diagnostics) = game_preview_scene(&runtime, &authoring);

    assert!(diagnostics.is_empty());
    assert_eq!(
        preview.scene_settings.active_camera.as_deref(),
        Some("camera-main")
    );
    assert!(
        preview
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "camera-main")
            .unwrap()
            .components["engine.camera"]
            .enabled
    );
    assert!(!preview
        .entities
        .iter()
        .any(|entity| entity.persistent_id.starts_with(EDITOR_CAMERA_ID_PREFIX)));
    assert!(preview
        .entities
        .iter()
        .all(|entity| !entity.components.contains_key("engine.script")));
}

#[test]
fn failed_play_load_rolls_back_to_script_free_editor_preview() {
    let mut authoring = engine_scene::sample_scene();
    authoring.entities[1].components.insert(
        "engine.script".into(),
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: Default::default(),
        },
    );
    let mut game_loop = GameLoop::new(EngineConfig::default());
    let mut play_session = EditorPlaySession::default();
    let failure = Diagnostic::new(
        "TEST_PLAY_FAILURE",
        DiagnosticSeverity::Error,
        "test",
        "OnCreate failed",
    );

    let result = play_session.start(&authoring, |scene| {
        game_loop.load_scene(scene).unwrap();
        Err::<(), _>(vec![failure])
    });
    assert!(result.is_err());
    assert!(play_session.is_editing());
    assert!(game_loop
        .runtime
        .scene_ref()
        .unwrap()
        .entities
        .iter()
        .any(|entity| entity.components.contains_key("engine.script")));

    restore_editor_preview(&mut game_loop, &authoring).unwrap();
    let restored = game_loop.runtime.scene_ref().unwrap();
    assert!(restored
        .scene_settings
        .active_camera
        .as_deref()
        .unwrap()
        .starts_with(EDITOR_CAMERA_ID_PREFIX));
    assert!(restored
        .entities
        .iter()
        .all(|entity| !entity.components.contains_key("engine.script")));
}
