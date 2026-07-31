#[test]
fn rgba8_mip_split_rejects_truncated_and_trailing_data() {
    assert!(split_rgba8_mips(2, 2, 2, &[0; 19]).is_err());
    assert!(split_rgba8_mips(2, 2, 2, &[0; 21]).is_err());
    let levels = split_rgba8_mips(2, 2, 2, &[0; 20]).unwrap();
    assert_eq!(levels.len(), 2);
    assert_eq!((levels[1].width, levels[1].height), (1, 1));
}

#[test]
fn cooked_skinned_mesh_reaches_the_runtime_as_skinned64() {
    let dir = cooked_case("skinned_mesh");
    let mesh = engine_asset::mesh::MeshData {
        positions: vec![glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y],
        normals: vec![glam::Vec3::Z; 3],
        uvs: vec![glam::Vec2::ZERO; 3],
        indices: vec![0, 1, 2],
        bounds: (glam::Vec3::ZERO, glam::Vec3::ONE),
        joints: vec![[0, 1, 0, 0]; 3],
        weights: vec![[0.75, 0.25, 0.0, 0.0]; 3],
    };
    engine_asset::cook::write_cooked_artifact(
        &dir.join("mesh.skinned.cooked"),
        AssetType::Mesh.kind_code(),
        &bincode::serialize(&mesh).unwrap(),
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )
    .unwrap();

    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    runtime.load_cooked_assets(&dir).unwrap();
    let upload = runtime
        .asset_registry()
        .get::<MeshUpload>(&AssetId::new("mesh.skinned"))
        .expect("skinned mesh upload");
    assert_eq!(upload.get().vertex_format, MeshVertexFormat::Skinned64);
    assert_eq!(upload.get().vertex_bytes.len(), 3 * 64);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_cooked_directory_is_an_empty_load() {
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    let missing = std::path::PathBuf::from("definitely-missing-cooked-assets");
    let report = runtime.load_cooked_assets(&missing).unwrap();
    assert_eq!(report, CookedAssetLoadReport::default());
}

#[test]
fn material_texture_dependency_accepts_batch_or_typed_registry_texture() {
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    runtime.register_texture_asset(texture_upload("texture.registry"));
    let materials = vec![
        (
            PathBuf::from("batch.cooked"),
            material_upload("material.batch", Some("texture.batch")),
        ),
        (
            PathBuf::from("registry.cooked"),
            material_upload("material.registry", Some("texture.registry")),
        ),
    ];

    assert!(validate_material_texture_dependencies(
        &runtime,
        &[texture_upload("texture.batch")],
        &materials,
        &BTreeSet::new(),
    )
    .is_empty());
}

#[test]
fn material_texture_dependency_requires_a_typed_texture() {
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    runtime.register_material_asset(material_upload("texture.wrong-type", None));
    let path = PathBuf::from("missing-dependency.cooked");
    let materials = vec![(
        path.clone(),
        material_upload("material.invalid", Some("texture.wrong-type")),
    )];

    let diagnostics =
        validate_material_texture_dependencies(&runtime, &[], &materials, &BTreeSet::new());

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].path.as_deref(), path.to_str());
    assert!(diagnostics[0].message.contains("texture.wrong-type"));
}

#[test]
fn auxiliary_material_texture_dependencies_are_validated() {
    let runtime = EngineRuntime::new(crate::EngineConfig::default());
    let path = PathBuf::from("missing-normal-dependency.cooked");
    let mut upload = material_upload("material.invalid-normal", None);
    upload.normal_texture = Some(AssetId::new("texture.normal-missing"));

    let diagnostics = validate_material_texture_dependencies(
        &runtime,
        &[],
        &[(path.clone(), upload)],
        &BTreeSet::new(),
    );

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].path.as_deref(), path.to_str());
    assert!(diagnostics[0].message.contains("texture.normal-missing"));
}

#[test]
fn cooked_material_is_registered_and_counted() {
    let dir = cooked_case("load");
    cook_test_material(&dir, "material.plain", None);
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

    let report = runtime.load_cooked_assets(&dir).unwrap();

    assert_eq!(report.discovered_assets, 1);
    assert_eq!(report.loaded_materials, 1);
    assert_eq!(report.loaded_render_assets(), 1);
    assert!(runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.plain"))
        .is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cooked_surface_materials_preserve_alpha_and_culling_state() {
    let dir = cooked_case("surface_states");
    cook_test_surface_material(&dir, "material.masked", "Masked", 0.37, true);
    cook_test_surface_material(&dir, "material.blended", "Blend", 0.5, false);
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

    runtime.load_cooked_assets(&dir).unwrap();

    let masked = runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.masked"))
        .unwrap();
    assert_eq!(
        masked.get().transparency,
        Transparency::Masked { cutoff: 0.37 }
    );
    assert!(masked.get().double_sided);
    let blended = runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.blended"))
        .unwrap();
    assert_eq!(blended.get().transparency, Transparency::Blend);
    assert!(!blended.get().double_sided);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_material_texture_prevents_partial_batch_registration() {
    let dir = cooked_case("atomic_dependency_failure");
    cook_test_material(&dir, "material.valid", None);
    cook_test_material(&dir, "material.invalid", Some("texture.missing"));
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

    let diagnostics = runtime.load_cooked_assets(&dir).unwrap_err();

    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message.contains("texture.missing"));
    assert!(runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.valid"))
        .is_none());
    assert!(runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.invalid"))
        .is_none());
    let _ = std::fs::remove_dir_all(dir);
}
