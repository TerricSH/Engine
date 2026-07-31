use super::super::*;
use super::common::*;

#[test]
fn registry_concrete_types_drive_classification_not_id_prefixes() {
    let fixture = AssetCatalogFixture::new("registry_types");
    let registry = registry_with_typed_assets();
    let mut panel = AssetBrowserPanel::new();
    empty_catalog_refresh(&mut panel, &registry, &fixture);

    assert_eq!(panel.catalog_assets().len(), 4);
    assert_eq!(
        panel
            .catalog_assets()
            .iter()
            .find(|entry| entry.id.id == "plain-name")
            .map(|entry| entry.kind),
        Some(AssetKind::Mesh)
    );
    assert_eq!(
        panel
            .catalog_assets()
            .iter()
            .find(|entry| entry.id.id == "not-a-prefix")
            .map(|entry| entry.kind),
        Some(AssetKind::Texture)
    );
    let unknown = panel
        .catalog_assets()
        .iter()
        .find(|entry| entry.id.id == "mesh-lie")
        .expect("raw/extension cache entry remains visible");
    assert_eq!(unknown.kind, AssetKind::Unknown);
    assert!(unknown.loaded);
    assert!(!unknown.manifest_declared);
}

#[test]
fn authoritative_manifests_expose_every_asset_type_and_filter() {
    let fixture = AssetCatalogFixture::new("all_manifest_types");
    let cases = vec![
        (AssetType::Mesh, AssetKind::Mesh, AssetKindFilter::Mesh),
        (
            AssetType::Texture,
            AssetKind::Texture,
            AssetKindFilter::Texture,
        ),
        (
            AssetType::Shader,
            AssetKind::Shader,
            AssetKindFilter::Shader,
        ),
        (AssetType::Scene, AssetKind::Scene, AssetKindFilter::Scene),
        (
            AssetType::Material,
            AssetKind::Material,
            AssetKindFilter::Material,
        ),
        (
            AssetType::Pipeline,
            AssetKind::Pipeline,
            AssetKindFilter::Pipeline,
        ),
        (
            AssetType::Script,
            AssetKind::Script,
            AssetKindFilter::Script,
        ),
        (AssetType::Audio, AssetKind::Audio, AssetKindFilter::Audio),
        (AssetType::Font, AssetKind::Font, AssetKindFilter::Font),
        (
            AssetType::Animation,
            AssetKind::Animation,
            AssetKindFilter::Animation,
        ),
        (
            AssetType::Skeleton,
            AssetKind::Skeleton,
            AssetKindFilter::Skeleton,
        ),
        (
            AssetType::NavMesh,
            AssetKind::NavMesh,
            AssetKindFilter::NavMesh,
        ),
        (AssetType::Logic, AssetKind::Logic, AssetKindFilter::Logic),
        (
            AssetType::Prefab,
            AssetKind::Prefab,
            AssetKindFilter::Prefab,
        ),
        (
            AssetType::EnvironmentMap,
            AssetKind::EnvironmentMap,
            AssetKindFilter::EnvironmentMap,
        ),
        (
            AssetType::MorphTargetSet,
            AssetKind::MorphTargetSet,
            AssetKindFilter::MorphTargetSet,
        ),
        (
            AssetType::Unknown,
            AssetKind::Unknown,
            AssetKindFilter::Unknown,
        ),
    ];
    let assets = cases
        .iter()
        .enumerate()
        .map(|(index, (asset_type, _, _))| {
            source_entry(
                &format!("asset-{index:02}"),
                asset_type.clone(),
                &format!("types/{index:02}.source"),
            )
        })
        .collect();
    fixture.write_manifest(
        "catalog.MANIFEST",
        &SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets,
        },
    );
    fixture.write_cooked_marker("asset-01");

    let mut registry = AssetRegistry::new();
    let loaded_mesh = AssetId::new("asset-00");
    registry.insert_typed(loaded_mesh.clone(), mesh(loaded_mesh));
    let mut panel = AssetBrowserPanel::new();
    let summary = refresh_project_asset_list(&mut panel, &registry, &fixture.source_root).unwrap();

    assert_eq!(summary.manifest_count, 1);
    assert_eq!(summary.declared_asset_count, cases.len());
    assert_eq!(summary.registry_only_asset_count, 0);
    panel.set_current_folder("/types");
    assert_eq!(panel.assets().len(), cases.len());
    for (index, (_, expected_kind, filter)) in cases.iter().enumerate() {
        panel.set_kind_filter(*filter);
        assert_eq!(panel.assets().len(), 1, "{} filter", filter.label());
        let id = format!("asset-{index:02}");
        let entry = panel
            .assets()
            .iter()
            .find(|entry| entry.id.id == id)
            .expect("manifest asset appears in catalog");
        assert_eq!(entry.kind, *expected_kind);
        assert_eq!(
            entry.source_path.as_deref(),
            Some(format!("types/{index:02}.source").as_str())
        );
        assert!(entry.manifest_declared);
        assert_eq!(entry.loaded, index == 0);
        assert_eq!(entry.cooked, index == 1);
        assert_eq!(panel.assets()[0].kind, *expected_kind);
    }

    assert_eq!(AssetKindFilter::ALL_KINDS.len(), cases.len());
    panel.set_kind_filter(AssetKindFilter::All);
    panel.set_search_query("07.source");
    assert_eq!(panel.assets().len(), 1);
    assert_eq!(panel.assets()[0].kind, AssetKind::Audio);
    panel.set_current_folder("/");
    panel.set_search_query("16.SOURCE");
    assert_eq!(panel.assets().len(), 1);
    assert_eq!(panel.assets()[0].kind, AssetKind::Unknown);
}

#[test]
fn manifest_kind_is_authoritative_and_registry_only_assets_are_merged() {
    let fixture = AssetCatalogFixture::new("authority_and_merge");
    fixture.write_manifest(
        "game.manifest",
        &SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: vec![source_entry(
                "declared-audio",
                AssetType::Audio,
                "audio/theme.ogg",
            )],
        },
    );
    fixture.write_cooked_marker("declared-audio");

    let mut registry = AssetRegistry::new();
    let declared = AssetId::new("declared-audio");
    registry.insert_typed(declared.clone(), mesh(declared));
    let declared_alias = AssetId::with_path("declared-audio", "runtime/duplicate-cache-key.mesh");
    registry.insert_typed(declared_alias.clone(), mesh(declared_alias));
    let builtin = AssetId::with_path("builtin-cube", "builtin/cube.mesh");
    registry.insert_typed(builtin.clone(), mesh(builtin.clone()));
    registry.insert_typed(AssetId::new("extension-data"), 7_u32);

    let mut panel = AssetBrowserPanel::new();
    let summary = refresh_project_asset_list(&mut panel, &registry, &fixture.source_root).unwrap();

    assert_eq!(summary.declared_asset_count, 1);
    assert_eq!(summary.registry_only_asset_count, 2);
    let declared = panel
        .catalog_assets()
        .iter()
        .find(|entry| entry.id.id == "declared-audio")
        .unwrap();
    assert_eq!(declared.kind, AssetKind::Audio);
    assert_eq!(declared.source_path.as_deref(), Some("audio/theme.ogg"));
    assert!(declared.loaded);
    assert!(declared.cooked);
    assert!(declared.manifest_declared);

    let builtin = panel
        .catalog_assets()
        .iter()
        .find(|entry| entry.id.id == "builtin-cube")
        .unwrap();
    assert_eq!(builtin.kind, AssetKind::Mesh);
    assert!(builtin.loaded);
    assert!(!builtin.manifest_declared);
    assert!(builtin.source_path.is_none());

    let extension = panel
        .catalog_assets()
        .iter()
        .find(|entry| entry.id.id == "extension-data")
        .unwrap();
    assert_eq!(extension.kind, AssetKind::Unknown);
}

#[test]
fn invalid_manifest_is_reported_without_replacing_previous_snapshot() {
    let fixture = AssetCatalogFixture::new("invalid_manifest_transaction");
    let registry = registry_with_typed_assets();
    let mut panel = AssetBrowserPanel::new();
    empty_catalog_refresh(&mut panel, &registry, &fixture);
    let previous = panel.catalog_assets().to_vec();
    std::fs::write(fixture.source_root.join("broken.manifest"), b"{")
        .expect("write invalid manifest");

    let error = refresh_project_asset_list(&mut panel, &registry, &fixture.source_root)
        .expect_err("invalid manifest must fail refresh");
    assert!(matches!(
        error,
        AssetBrowserRefreshError::ManifestParse { .. }
    ));
    assert_eq!(panel.catalog_assets(), previous);
}

#[test]
fn unsupported_manifest_schema_is_rejected() {
    let fixture = AssetCatalogFixture::new("unsupported_schema");
    fixture.write_manifest(
        "future.manifest",
        &SourceManifest {
            schema_version: SchemaVersion::new(99, 0, 0),
            assets: Vec::new(),
        },
    );
    let error = refresh_project_asset_list(
        &mut AssetBrowserPanel::new(),
        &AssetRegistry::new(),
        &fixture.source_root,
    )
    .expect_err("unsupported schema must fail refresh");
    assert!(matches!(
        error,
        AssetBrowserRefreshError::UnsupportedSchema { .. }
    ));
}

#[test]
fn duplicate_manifest_ids_are_rejected_case_insensitively() {
    let fixture = AssetCatalogFixture::new("duplicate_ids");
    for (manifest_name, id) in [
        ("a.manifest", "Shared.Asset"),
        ("b.manifest", "shared.asset"),
    ] {
        fixture.write_manifest(
            manifest_name,
            &SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: vec![source_entry(
                    id,
                    AssetType::Scene,
                    "scenes/shared.scene.ron",
                )],
            },
        );
    }
    let error = refresh_project_asset_list(
        &mut AssetBrowserPanel::new(),
        &AssetRegistry::new(),
        &fixture.source_root,
    )
    .expect_err("duplicate IDs must fail refresh");
    assert!(matches!(
        error,
        AssetBrowserRefreshError::DuplicateAssetId { .. }
    ));
}

#[test]
fn manifest_asset_ids_use_cook_validation_rules() {
    let fixture = AssetCatalogFixture::new("invalid_asset_id");
    fixture.write_manifest(
        "bad-id.manifest",
        &SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: vec![source_entry("not/portable", AssetType::Mesh, "mesh.gltf")],
        },
    );
    let error = refresh_project_asset_list(
        &mut AssetBrowserPanel::new(),
        &AssetRegistry::new(),
        &fixture.source_root,
    )
    .expect_err("invalid ID must fail refresh");
    assert!(matches!(
        error,
        AssetBrowserRefreshError::InvalidAssetId { .. }
    ));
}

#[test]
fn tool_owned_temporary_textures_do_not_appear_as_project_assets() {
    let fixture = AssetCatalogFixture::new("private_editor_assets");
    let mut registry = registry_with_typed_assets();
    let id = AssetId::new("editor/preview/temporary/0");
    registry.insert_typed(id.clone(), texture(id));
    let mut panel = AssetBrowserPanel::new();
    empty_catalog_refresh(&mut panel, &registry, &fixture);

    assert!(!panel
        .catalog_assets()
        .iter()
        .any(|entry| entry.id.id.starts_with("editor/")));
}
