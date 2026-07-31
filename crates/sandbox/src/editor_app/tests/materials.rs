#[test]
fn browser_assignment_uses_editor_history_for_undo_and_redo() {
    let mut runtime = engine_core::EngineRuntime::new(EngineConfig::default());
    let builtin_id = AssetId::new("mesh-cube");
    let mut alternate_mesh: MeshUpload = runtime
        .asset_registry()
        .get::<MeshUpload>(&builtin_id)
        .expect("builtin mesh should exist")
        .get()
        .clone();
    let alternate_id = AssetId::with_path("mesh-alternate", "models/alternate.mesh");
    alternate_mesh.mesh_id = alternate_id.clone();
    alternate_mesh.content_hash = [42; 32];
    runtime.register_mesh_asset(alternate_mesh);

    let mut browser = ProjectAssetBrowserPanel::new();
    let source_root = tempfile::tempdir().unwrap();
    refresh_project_asset_list(&mut browser, runtime.asset_registry(), source_root.path()).unwrap();
    assert!(browser.select_asset(Some(alternate_id.clone())));

    let mut editor_scene = EditorScene::new(engine_scene::sample_scene());
    editor_scene.selected_entity = Some("cube-01".to_string());
    assert!(execute_selected_asset_assignment(&browser, &mut editor_scene).unwrap());
    assert!(editor_scene.is_dirty());

    let mesh_value = |scene: &Scene| {
        scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .and_then(|entity| entity.components.get("engine.renderable"))
            .and_then(|component| component.fields.get("mesh"))
            .cloned()
    };
    assert_eq!(
        mesh_value(&editor_scene.scene),
        Some(Value::Asset(alternate_id.clone()))
    );

    editor_scene.undo().unwrap();
    assert_eq!(
        mesh_value(&editor_scene.scene),
        Some(Value::Asset(builtin_id))
    );

    editor_scene.redo().unwrap();
    assert_eq!(
        mesh_value(&editor_scene.scene),
        Some(Value::Asset(alternate_id))
    );
}

#[test]
fn material_save_updates_source_cooked_payload_and_runtime_registry() {
    let fixture = material_project_fixture();
    cook_fixture(&fixture);
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    super::super::project_app::load_project_assets(&mut runtime, &fixture.project).unwrap();
    assert_eq!(
        runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("mat-project"))
            .unwrap()
            .get()
            .roughness,
        0.7
    );

    let request = MaterialSaveRequest {
        material_asset: "mat-project".to_string(),
        source: test_material_source(0.31, [0.8, 0.6, 0.4, 1.0]),
    };
    let outcome = save_project_material(&mut runtime, &fixture.project, &request).unwrap();

    let saved_source: MaterialSource =
        serde_json::from_slice(&std::fs::read(&fixture.source_path).unwrap()).unwrap();
    assert_eq!(saved_source.roughness, 0.31);
    assert_eq!(saved_source.base_color, [0.8, 0.6, 0.4, 1.0]);

    let artifact = read_cooked_artifact(&outcome.cooked_path).unwrap();
    let cooked = decode_cooked_material(&artifact).unwrap();
    assert_eq!(cooked.roughness, 0.31);
    assert_eq!(cooked.base_color, [0.8, 0.6, 0.4, 1.0]);

    let registered = runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("mat-project"))
        .unwrap();
    assert_eq!(registered.get().roughness, 0.31);
    assert_eq!(registered.get().base_color, [0.8, 0.6, 0.4, 1.0]);
    assert_eq!(outcome.source_path, fixture.source_path);
}

#[test]
fn material_save_rejects_builtin_unknown_and_ambiguous_ids() {
    let fixture = material_project_fixture();
    let original_source = std::fs::read(&fixture.source_path).unwrap();
    let mut runtime = EngineRuntime::new(EngineConfig::default());

    let builtin_error = save_project_material(
        &mut runtime,
        &fixture.project,
        &MaterialSaveRequest {
            material_asset: BUILTIN_DEFAULT_MATERIAL_ID.to_string(),
            source: test_material_source(0.2, [1.0; 4]),
        },
    )
    .unwrap_err();
    assert!(builtin_error.contains("Built-in"));

    let unknown_error = save_project_material(
        &mut runtime,
        &fixture.project,
        &MaterialSaveRequest {
            material_asset: "mat-unknown".to_string(),
            source: test_material_source(0.2, [1.0; 4]),
        },
    )
    .unwrap_err();
    assert!(unknown_error.contains("not declared"));

    write_json(
        &fixture.project.asset_source.join("duplicate.manifest"),
        &SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: vec![fixture.manifest_entry.clone()],
        },
    );
    let ambiguous_error = save_project_material(
        &mut runtime,
        &fixture.project,
        &MaterialSaveRequest {
            material_asset: "mat-project".to_string(),
            source: test_material_source(0.2, [1.0; 4]),
        },
    )
    .unwrap_err();
    assert!(ambiguous_error.contains("ambiguous"));
    assert_eq!(
        std::fs::read(&fixture.source_path).unwrap(),
        original_source
    );
}

#[test]
fn failed_material_cook_restores_original_source_and_cooked_asset() {
    let fixture = material_project_fixture();
    cook_fixture(&fixture);
    let source_before = std::fs::read(&fixture.source_path).unwrap();
    let cooked_path = fixture.project.cooked_assets.join("mat-project.cooked");
    let cooked_before = std::fs::read(&cooked_path).unwrap();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    super::super::project_app::load_project_assets(&mut runtime, &fixture.project).unwrap();

    let mut invalid_source = test_material_source(0.1, [0.9, 0.1, 0.2, 1.0]);
    invalid_source.base_color_texture = Some("../unsafe-texture".to_string());
    let error = save_project_material(
        &mut runtime,
        &fixture.project,
        &MaterialSaveRequest {
            material_asset: "mat-project".to_string(),
            source: invalid_source,
        },
    )
    .unwrap_err();

    assert!(error.contains("original material source was restored"));
    assert_eq!(std::fs::read(&fixture.source_path).unwrap(), source_before);
    assert_eq!(std::fs::read(&cooked_path).unwrap(), cooked_before);
    assert_eq!(
        runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("mat-project"))
            .unwrap()
            .get()
            .roughness,
        0.7
    );
}
