#[test]
fn failed_project_cook_preserves_previous_directory_until_full_success() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("transactional-cook");
    create_project(&root, Some("Transactional Cook"), false).unwrap();
    let cooked = root.join("build/cooked");
    std::fs::create_dir_all(&cooked).unwrap();
    let previous = cooked.join("previous.cooked");
    std::fs::write(&previous, b"previous successful batch").unwrap();
    let broken_manifest = root.join("assets/source/broken.manifest");
    std::fs::write(&broken_manifest, b"{").unwrap();

    assert!(cook_project(&root).is_err());
    assert_eq!(
        std::fs::read(&previous).unwrap(),
        b"previous successful batch"
    );

    std::fs::remove_file(broken_manifest).unwrap();
    cook_project(&root).unwrap();
    assert!(cooked.is_dir());
    assert!(!previous.exists());
}

#[test]
fn project_asset_type_validation_matches_enabled_runtime_extensions() {
    let builder = engine_core::EngineRuntime::builder(engine_core::EngineConfig::default());
    assert!(validate_project_asset_type(&AssetType::Mesh, builder.asset_type_registry()).is_ok());
    assert!(validate_project_asset_type(&AssetType::Font, builder.asset_type_registry()).is_err());
    #[cfg(feature = "runtime-subsystems")]
    assert!(validate_project_asset_type(&AssetType::Audio, builder.asset_type_registry()).is_ok());
    #[cfg(not(feature = "runtime-subsystems"))]
    assert!(validate_project_asset_type(&AssetType::Audio, builder.asset_type_registry()).is_err());
}

#[cfg(feature = "runtime-subsystems")]
fn minimal_pcm_wav() -> Vec<u8> {
    let samples = [0i16; 80];
    let data_size = u32::try_from(samples.len() * 2).unwrap();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8_000u32.to_le_bytes());
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[cfg(feature = "runtime-subsystems")]
#[test]
fn project_workflow_cooks_checks_and_loads_all_runtime_extension_assets() {
    use engine_animation::{AnimationClip, Joint, JointTransform, Skeleton};
    use engine_nav::NavMesh;
    use glam::Vec3;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("extension-project");
    create_project(&root, Some("Extension Project"), false).unwrap();
    let source = root.join("assets/source");
    std::fs::write(source.join("ambient.wav"), minimal_pcm_wav()).unwrap();

    let skeleton = Skeleton {
        joints: vec![Joint {
            name: "root".into(),
            parent_index: None,
            local_transform: JointTransform::IDENTITY,
        }],
        inverse_bind_matrices: vec![[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]],
    };
    std::fs::write(
        source.join("hero.skel"),
        bincode::serialize(&skeleton).unwrap(),
    )
    .unwrap();
    let animation = AnimationClip {
        name: "idle".into(),
        duration: 1.0,
        channels: vec![],
        joint_indices: vec![],
    };
    std::fs::write(
        source.join("idle.anim"),
        bincode::serialize(&animation).unwrap(),
    )
    .unwrap();
    let mut navmesh = NavMesh::new();
    let a = navmesh.add_vertex(Vec3::new(0.0, 0.0, 0.0));
    let b = navmesh.add_vertex(Vec3::new(1.0, 0.0, 0.0));
    let c = navmesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
    navmesh.add_polygon(&[a, b, c], 1.0);
    navmesh.rebuild_bvh();
    std::fs::write(
        source.join("level.navmesh"),
        bincode::serialize(&navmesh).unwrap(),
    )
    .unwrap();

    let entry = |id: &str, asset_type: AssetType, source_path: &str| SourceAssetEntry {
        id: AssetId::new(id),
        asset_type,
        source_path: source_path.into(),
        cook_rules: CookRules::default(),
    };
    let manifest = SourceManifest {
        schema_version: CURRENT_MANIFEST_VERSION,
        assets: vec![
            entry("audio.ambient", AssetType::Audio, "ambient.wav"),
            entry("skeleton.hero", AssetType::Skeleton, "hero.skel"),
            entry("animation.idle", AssetType::Animation, "idle.anim"),
            entry("navmesh.level", AssetType::NavMesh, "level.navmesh"),
        ],
    };
    let mut manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
    manifest_json.push('\n');
    std::fs::write(source.join("game.manifest"), manifest_json).unwrap();

    check_project(&root, None).unwrap();
    cook_project(&root).unwrap();
    check_project(&root, None).unwrap();

    let project = GameProject::load(&root).unwrap();
    let mut runtime = engine_core::EngineRuntime::new(engine_core::EngineConfig::default());
    let report = crate::project_app::load_project_assets(&mut runtime, &project).unwrap();
    assert_eq!(report.loaded_extension_assets(), 4);
    assert!(runtime
        .extension_asset::<engine_audio::AudioClip>("audio_clip", &AssetId::new("audio.ambient"),)
        .is_some());
    assert!(runtime
        .extension_asset::<Skeleton>("skeleton", &AssetId::new("skeleton.hero"))
        .is_some());
    assert!(runtime
        .extension_asset::<AnimationClip>("animation_clip", &AssetId::new("animation.idle"),)
        .is_some());
    assert!(runtime
        .extension_asset::<NavMesh>("navmesh", &AssetId::new("navmesh.level"))
        .is_some());
}

fn check_test_prefab_source(asset_id: &str, mesh_id: &str, child_prefab: Option<&str>) -> String {
    let mut transform_fields = BTreeMap::new();
    transform_fields.insert(
        "translation".to_string(),
        engine_serialize::Value::Vec3([0.0, 0.0, 0.0]),
    );
    transform_fields.insert(
        "rotation".to_string(),
        engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
    );
    transform_fields.insert(
        "scale".to_string(),
        engine_serialize::Value::Vec3([1.0, 1.0, 1.0]),
    );
    let mut renderable_fields = BTreeMap::new();
    renderable_fields.insert(
        "mesh".to_string(),
        engine_serialize::Value::Asset(AssetId::new(mesh_id)),
    );
    renderable_fields.insert(
        "material".to_string(),
        engine_serialize::Value::Asset(AssetId::new("mat-default")),
    );
    let mut components = BTreeMap::new();
    components.insert(
        "engine.transform".to_string(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: transform_fields,
        },
    );
    components.insert(
        "engine.renderable".to_string(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: renderable_fields,
        },
    );
    let mut prefab = engine_scene::Prefab::new(AssetId::new(asset_id));
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "root".to_string(),
        parent: None,
        name: Some("Root".to_string()),
        enabled: true,
        components,
    });
    if let Some(child) = child_prefab {
        prefab
            .child_prefab_refs
            .push(engine_scene::prefab::PrefabChildRef {
                entity_persistent_id: "root".to_string(),
                prefab_asset: AssetId::new(child),
            });
    }
    engine_scene::serialize_prefab_source(&prefab).unwrap()
}

fn write_check_test_manifest(root: &Path, entries: Vec<SourceAssetEntry>) {
    let manifest = SourceManifest {
        schema_version: CURRENT_MANIFEST_VERSION,
        assets: entries,
    };
    let mut manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
    manifest_json.push('\n');
    std::fs::write(root.join("assets/source/game.manifest"), manifest_json).unwrap();
}

fn check_test_entry(id: &str, asset_type: AssetType, source_path: &str) -> SourceAssetEntry {
    SourceAssetEntry {
        id: AssetId::new(id),
        asset_type,
        source_path: source_path.into(),
        cook_rules: CookRules::default(),
    }
}
