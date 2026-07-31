#[cfg(not(feature = "subsystem-audio"))]
fn test_extension_loader(cooked: &[u8]) -> Result<Box<dyn Any + Send + Sync>, String> {
    String::from_utf8(cooked.to_vec())
        .map(|value| Box::new(value) as Box<dyn Any + Send + Sync>)
        .map_err(|error| error.to_string())
}

#[cfg(not(feature = "subsystem-audio"))]
#[test]
fn registered_extension_assets_share_the_typed_cache_and_reload_atomically() {
    use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

    let dir = cooked_case("extension_transaction");
    let id = AssetId::new("audio.custom");
    engine_asset::cook::write_cooked_artifact(
        &dir.join("audio.custom.cooked"),
        AssetType::Audio.kind_code(),
        b"first payload",
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )
    .unwrap();
    let mut builder = crate::EngineRuntime::builder(crate::EngineConfig::default());
    builder
        .asset_type_registry_mut()
        .register(AssetTypeExtension {
            meta: AssetTypeMeta {
                type_id: "audio_clip",
                source_extensions: vec!["custom"],
                display_name: "Custom Audio",
            },
            cooker: None,
            loader: Some(test_extension_loader),
        })
        .unwrap();
    let mut runtime = builder.build();

    let report = runtime.load_cooked_assets(&dir).unwrap();

    assert_eq!(report.loaded_extension_assets(), 1);
    assert_eq!(report.loaded_assets(), 1);
    assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
    assert_eq!(
        runtime
            .extension_asset::<String>("audio_clip", &id)
            .expect("extension asset")
            .get(),
        "first payload"
    );
    assert_eq!(
        runtime
            .asset_registry_mut()
            .load(&id)
            .expect("raw payload")
            .get(),
        b"first payload"
    );

    engine_asset::cook::write_cooked_artifact(
        &dir.join("broken.cooked"),
        4_242,
        b"valid outer artifact with unknown kind",
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )
    .unwrap();
    let diagnostics = runtime.load_cooked_assets(&dir).unwrap_err();
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("kind code 4242")));
    assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
    assert_eq!(
        runtime
            .extension_asset::<String>("audio_clip", &id)
            .expect("previous batch remains installed")
            .get(),
        "first payload"
    );

    std::fs::remove_file(dir.join("broken.cooked")).unwrap();
    std::fs::remove_file(dir.join("audio.custom.cooked")).unwrap();
    let empty_report = runtime.load_cooked_assets(&dir).unwrap();
    assert_eq!(empty_report.loaded_assets(), 0);
    assert_eq!(runtime.extension_asset_count("audio_clip"), 0);
    assert!(runtime.asset_registry().get::<String>(&id).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(not(feature = "subsystem-audio"))]
#[test]
fn additive_extension_assets_noop_on_identical_payload_and_reject_conflicts() {
    use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

    let dir = cooked_case("additive_extension");
    let id = AssetId::new("audio.custom");
    let mut builder = crate::EngineRuntime::builder(crate::EngineConfig::default());
    builder
        .asset_type_registry_mut()
        .register(AssetTypeExtension {
            meta: AssetTypeMeta {
                type_id: "audio_clip",
                source_extensions: vec!["custom"],
                display_name: "Custom Audio",
            },
            cooker: None,
            loader: Some(test_extension_loader),
        })
        .unwrap();
    let mut runtime = builder.build();

    let paths = [dir.join("audio.custom.cooked")];
    engine_asset::cook::write_cooked_artifact(
        &paths[0],
        AssetType::Audio.kind_code(),
        b"first payload",
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )
    .unwrap();
    let first = runtime.install_cooked_assets_additive(&paths).unwrap();
    assert_eq!(first.loaded_extension_assets(), 1);

    // Identical cooked payload: no-op success.
    let second = runtime.install_cooked_assets_additive(&paths).unwrap();
    assert_eq!(second.loaded_extension_assets(), 0);
    assert_eq!(second.identical_assets, 1);
    assert_eq!(
        runtime
            .extension_asset::<String>("audio_clip", &id)
            .expect("extension asset")
            .get(),
        "first payload"
    );

    // Differing cooked payload under the same ID: validation error.
    engine_asset::cook::write_cooked_artifact(
        &paths[0],
        AssetType::Audio.kind_code(),
        b"second payload",
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )
    .unwrap();
    let diagnostics = runtime.install_cooked_assets_additive(&paths).unwrap_err();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "AS0003");
    assert!(diagnostics[0].message.contains("audio.custom"));
    assert_eq!(
        runtime
            .extension_asset::<String>("audio_clip", &id)
            .expect("original payload remains")
            .get(),
        "first payload"
    );
    let _ = std::fs::remove_dir_all(dir);
}
