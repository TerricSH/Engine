use super::super::*;
use super::common::*;

#[test]
fn search_and_kind_filters_recompute_immediately_and_clamp_page() {
    let fixture = AssetCatalogFixture::new("search_filters");
    let registry = registry_with_typed_assets();
    let mut panel = AssetBrowserPanel::new();
    empty_catalog_refresh(&mut panel, &registry, &fixture);

    panel.set_search_query("ALBEDO");
    assert_eq!(panel.assets().len(), 1);
    assert_eq!(panel.assets()[0].kind, AssetKind::Texture);

    panel.set_kind_filter(AssetKindFilter::Mesh);
    assert!(panel.assets().is_empty());
    assert_eq!(panel.page(), 0);

    panel.set_current_folder("/models");
    panel.set_search_query("");
    assert_eq!(panel.assets().len(), 1);
    assert_eq!(panel.assets()[0].kind, AssetKind::Mesh);
}

#[test]
fn folders_breadcrumbs_and_direct_contents_come_from_catalog_paths() {
    let fixture = AssetCatalogFixture::new("folders");
    std::fs::create_dir_all(fixture.source_root.join("empty/nested")).unwrap();
    fixture.write_manifest(
        "game.manifest",
        &SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: vec![
                source_entry("root", AssetType::Logic, "root.ron"),
                source_entry("shared", AssetType::Mesh, "models/shared.gltf"),
                source_entry("hero", AssetType::Mesh, "models/hero/body.gltf"),
                source_entry("albedo", AssetType::Texture, "textures/albedo.png"),
            ],
        },
    );
    let mut panel = AssetBrowserPanel::new();
    empty_catalog_refresh(&mut panel, &AssetRegistry::new(), &fixture);

    assert_eq!(
        panel
            .folders()
            .iter()
            .map(|folder| folder.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/",
            "/empty",
            "/empty/nested",
            "/models",
            "/models/hero",
            "/textures"
        ]
    );
    assert_eq!(panel.assets().len(), 1);
    assert_eq!(panel.assets()[0].id.id, "root");

    panel.set_current_folder("MODELS");
    assert_eq!(panel.current_folder(), "/models");
    assert_eq!(panel.assets().len(), 1);
    assert_eq!(panel.assets()[0].id.id, "shared");

    panel.set_current_folder("/models/hero");
    assert_eq!(panel.breadcrumbs(), vec!["/", "/models", "/models/hero"]);
    assert_eq!(panel.assets()[0].id.id, "hero");

    panel.set_current_folder("/empty/nested");
    assert!(panel.assets().is_empty());

    panel.set_current_folder("/models");
    panel.set_search_query("hero");
    assert_eq!(panel.assets().len(), 1);
    assert_eq!(panel.assets()[0].id.id, "hero");

    panel.set_current_folder("/missing");
    assert_eq!(panel.current_folder(), "/");
}

#[test]
fn pagination_uses_fixed_size_and_clamps_after_filtering() {
    let fixture = AssetCatalogFixture::new("pagination");
    let mut registry = AssetRegistry::new();
    for index in 0..(ASSET_BROWSER_PAGE_SIZE + 2) {
        let id = AssetId::with_path(format!("asset-{index:02}"), "models/");
        registry.insert_typed(id.clone(), mesh(id));
    }
    let mut panel = AssetBrowserPanel::new();
    empty_catalog_refresh(&mut panel, &registry, &fixture);
    panel.set_current_folder("/models");

    assert_eq!(panel.page_size(), ASSET_BROWSER_PAGE_SIZE);
    assert_eq!(panel.page_count(), 2);
    assert_eq!(panel.visible_assets().len(), ASSET_BROWSER_PAGE_SIZE);
    assert!(panel.next_page());
    assert_eq!(panel.visible_assets().len(), 2);
    assert!(!panel.next_page());

    panel.set_search_query("asset-00");
    assert_eq!(panel.page(), 0);
    assert_eq!(panel.page_count(), 1);
    assert!(!panel.previous_page());
}

#[test]
fn selection_keeps_complete_asset_id_across_filters() {
    let fixture = AssetCatalogFixture::new("selection");
    let registry = registry_with_typed_assets();
    let mut panel = AssetBrowserPanel::new();
    empty_catalog_refresh(&mut panel, &registry, &fixture);
    let id = AssetId::with_path("plain-name", "models/plain.mesh");

    assert!(panel.select_asset(Some(id.clone())));
    panel.set_kind_filter(AssetKindFilter::Texture);
    assert_eq!(panel.selected_asset(), Some(&id));
    assert_eq!(
        panel.selected_entry().map(|entry| entry.kind),
        Some(AssetKind::Mesh)
    );
}

#[test]
fn reveal_asset_clears_filters_opens_folder_and_selects_exact_identity() {
    let fixture = AssetCatalogFixture::new("reveal_asset");
    let mut registry = AssetRegistry::new();
    for index in 0..(ASSET_BROWSER_PAGE_SIZE + 2) {
        let id = AssetId::with_path(
            format!("asset-{index:02}"),
            format!("models/asset-{index:02}.mesh"),
        );
        registry.insert_typed(id.clone(), mesh(id));
    }
    let mut panel = AssetBrowserPanel::new();
    empty_catalog_refresh(&mut panel, &registry, &fixture);
    panel.set_search_query("does-not-match");
    panel.set_kind_filter(AssetKindFilter::Texture);
    let target_index = ASSET_BROWSER_PAGE_SIZE + 1;
    let target_id = format!("asset-{target_index:02}");
    let target_path = format!("models/asset-{target_index:02}.mesh");

    assert!(panel.reveal_asset(&target_id));
    assert_eq!(panel.search_query(), "");
    assert_eq!(panel.kind_filter(), AssetKindFilter::All);
    assert_eq!(panel.current_folder(), "/models");
    assert_eq!(panel.page(), 1);
    assert_eq!(
        panel.selected_asset(),
        Some(&AssetId::with_path(target_id, target_path))
    );
}

#[test]
fn refresh_removes_selection_only_when_registry_entry_disappears() {
    let fixture = AssetCatalogFixture::new("selection_removal");
    let registry = registry_with_typed_assets();
    let mut panel = AssetBrowserPanel::new();
    empty_catalog_refresh(&mut panel, &registry, &fixture);
    let selected = AssetId::with_path("plain-name", "models/plain.mesh");
    assert!(panel.select_asset(Some(selected)));

    empty_catalog_refresh(&mut panel, &AssetRegistry::new(), &fixture);
    assert!(panel.selected_asset().is_none());
}
