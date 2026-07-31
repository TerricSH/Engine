#[test]
fn staged_pipeline_matches_legacy_whole_directory_load() {
    let dir = cooked_case("staged_equivalence");
    cook_test_material(&dir, "material.alpha", None);
    cook_test_material(&dir, "material.beta", None);

    let mut legacy = EngineRuntime::new(crate::EngineConfig::default());
    let legacy_report = legacy.load_cooked_assets(&dir).unwrap();

    let mut staged = EngineRuntime::new(crate::EngineConfig::default());
    let paths = vec![
        dir.join("material.alpha.cooked"),
        dir.join("material.beta.cooked"),
    ];
    let decoded = decode_cooked_batch(&paths, staged.asset_type_registry()).expect("decode stage");
    assert_eq!(decoded.discovered_assets(), 2);
    assert_eq!(decoded.decoded_assets(), 2);
    let validated = staged
        .validate_cooked_batch(decoded, CookedCommitMode::Replace)
        .expect("validate stage");
    assert_eq!(validated.mode(), CookedCommitMode::Replace);
    let staged_report = staged.commit_cooked_batch(validated);

    assert_eq!(legacy_report, staged_report);
    for id in ["material.alpha", "material.beta"] {
        let id = AssetId::new(id);
        assert_eq!(
            legacy
                .asset_registry()
                .get::<MaterialUpload>(&id)
                .map(|h| h.get().clone()),
            staged
                .asset_registry()
                .get::<MaterialUpload>(&id)
                .map(|h| h.get().clone()),
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn additive_install_merges_without_unloading_and_tracks_for_later_replace() {
    let dir_a = cooked_case("additive_base");
    cook_test_material(&dir_a, "material.base", None);
    let dir_b = cooked_case("additive_extra");
    cook_test_material(&dir_b, "material.extra", None);
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

    runtime.load_cooked_assets(&dir_a).unwrap();
    let report = runtime
        .install_cooked_assets_additive(&[dir_b.join("material.extra.cooked")])
        .unwrap();

    assert_eq!(report.loaded_materials, 1);
    assert_eq!(report.identical_assets, 0);
    for id in ["material.base", "material.extra"] {
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new(id))
            .is_some());
    }

    // A later whole-directory replace unloads additively installed assets too.
    let empty = cooked_case("additive_empty");
    runtime.load_cooked_assets(&empty).unwrap();
    for id in ["material.base", "material.extra"] {
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new(id))
            .is_none());
    }
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
    let _ = std::fs::remove_dir_all(empty);
}

#[test]
fn additive_identical_payload_is_a_noop_success() {
    let dir = cooked_case("additive_identical");
    cook_test_material(&dir, "material.same", None);
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    let paths = [dir.join("material.same.cooked")];

    let first = runtime.install_cooked_assets_additive(&paths).unwrap();
    assert_eq!(first.loaded_materials, 1);
    assert_eq!(first.identical_assets, 0);

    let second = runtime.install_cooked_assets_additive(&paths).unwrap();
    assert_eq!(second.loaded_materials, 0);
    assert_eq!(second.loaded_assets(), 0);
    assert_eq!(second.identical_assets, 1);
    assert!(runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.same"))
        .is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn additive_differing_payload_is_a_validation_error_naming_the_id() {
    let dir_a = cooked_case("additive_conflict_a");
    cook_test_material_with_color(&dir_a, "material.dup", None, [0.8, 0.7, 0.6, 1.0]);
    let dir_b = cooked_case("additive_conflict_b");
    cook_test_material_with_color(&dir_b, "material.dup", None, [0.1, 0.2, 0.3, 1.0]);
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

    runtime
        .install_cooked_assets_additive(&[dir_a.join("material.dup.cooked")])
        .unwrap();
    let diagnostics = runtime
        .install_cooked_assets_additive(&[dir_b.join("material.dup.cooked")])
        .unwrap_err();

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "AS0003");
    assert!(diagnostics[0].message.contains("material.dup"));
    // The original payload survives the rejected install.
    let installed = runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.dup"))
        .expect("original material remains");
    assert_eq!(installed.get().base_color, [0.8, 0.7, 0.6, 1.0]);
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
}

#[test]
fn additive_validation_failure_leaves_prior_batch_active() {
    let dir = cooked_case("additive_prior_batch");
    cook_test_material(&dir, "material.prior", None);
    let broken = cooked_case("additive_broken");
    cook_test_material(&broken, "material.broken", Some("texture.missing"));
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    runtime.load_cooked_assets(&dir).unwrap();

    let diagnostics = runtime
        .install_cooked_assets_additive(&[broken.join("material.broken.cooked")])
        .unwrap_err();

    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message.contains("texture.missing"));
    assert!(runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.prior"))
        .is_some());
    assert!(runtime
        .asset_registry()
        .get::<MaterialUpload>(&AssetId::new("material.broken"))
        .is_none());
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(broken);
}
