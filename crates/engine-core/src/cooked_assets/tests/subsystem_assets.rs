#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation"
))]
fn write_registered_extension_source(
    runtime: &EngineRuntime,
    dir: &Path,
    id: &str,
    kind: AssetType,
    source: &[u8],
) {
    let type_id = registered_asset_type_id(&kind).expect("mapped extension kind");
    let extension = runtime
        .asset_type_registry()
        .get(type_id)
        .expect("registered runtime extension");
    let mut payload = Vec::new();
    extension.cooker.expect("registered extension cooker")(source, &mut payload).unwrap();
    engine_asset::cook::write_cooked_artifact(
        &dir.join(format!("{id}.cooked")),
        kind.kind_code(),
        &payload,
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )
    .unwrap();
}

#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation"
))]
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

#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation"
))]
#[test]
fn runtime_subsystem_cookers_and_loaders_roundtrip_all_mapped_asset_kinds() {
    use engine_animation::{AnimationClip, Joint, JointTransform, Skeleton};
    use engine_nav::NavMesh;
    use glam::Vec3;

    let dir = cooked_case("real_runtime_extensions");
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    write_registered_extension_source(
        &runtime,
        &dir,
        "audio.real",
        AssetType::Audio,
        &minimal_pcm_wav(),
    );
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
    write_registered_extension_source(
        &runtime,
        &dir,
        "skeleton.real",
        AssetType::Skeleton,
        &bincode::serialize(&skeleton).unwrap(),
    );
    let clip = AnimationClip {
        name: "idle".into(),
        duration: 1.0,
        channels: vec![],
        joint_indices: vec![],
    };
    write_registered_extension_source(
        &runtime,
        &dir,
        "animation.real",
        AssetType::Animation,
        &bincode::serialize(&clip).unwrap(),
    );
    let mut navmesh = NavMesh::new();
    let a = navmesh.add_vertex(Vec3::new(0.0, 0.0, 0.0));
    let b = navmesh.add_vertex(Vec3::new(1.0, 0.0, 0.0));
    let c = navmesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
    navmesh.add_polygon(&[a, b, c], 1.0);
    navmesh.rebuild_bvh();
    write_registered_extension_source(
        &runtime,
        &dir,
        "navmesh.real",
        AssetType::NavMesh,
        &bincode::serialize(&navmesh).unwrap(),
    );
    write_registered_extension_source(
        &runtime,
        &dir,
        "logic.real",
        AssetType::Logic,
        br#"{
            "schema_version":{"major":0,"minor":1,"patch":0},
            "asset_id":"logic.real",
            "kind":"SkillGraph",
            "nodes":[{"id":"root","node_type":"ability","label":null,"transitions":[],"properties":{},"children":[]}],
            "parameters":{},
            "metadata":{"author":null,"description":null,"tags":["test"],"version":"1.0.0"}
        }"#,
    );

    let report = runtime.load_cooked_assets(&dir).unwrap();

    assert_eq!(report.discovered_assets, 5);
    assert_eq!(report.loaded_extension_assets(), 5);
    assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
    assert_eq!(runtime.extension_asset_count("skeleton"), 1);
    assert_eq!(runtime.extension_asset_count("animation_clip"), 1);
    assert_eq!(runtime.extension_asset_count("navmesh"), 1);
    assert_eq!(runtime.extension_asset_count("logic"), 1);
    assert_eq!(
        runtime
            .extension_asset::<engine_audio::AudioClip>("audio_clip", &AssetId::new("audio.real"),)
            .expect("audio clip")
            .get()
            .sample_rate(),
        8_000
    );
    assert_eq!(
        runtime
            .extension_asset::<Skeleton>("skeleton", &AssetId::new("skeleton.real"))
            .expect("skeleton")
            .get()
            .joint_count(),
        1
    );
    assert_eq!(
        runtime
            .extension_asset::<AnimationClip>("animation_clip", &AssetId::new("animation.real"),)
            .expect("animation clip")
            .get()
            .name(),
        "idle"
    );
    assert!(runtime
        .extension_asset::<NavMesh>("navmesh", &AssetId::new("navmesh.real"))
        .is_some());
    assert!(runtime
        .extension_asset::<engine_asset::cook::LogicAsset>("logic", &AssetId::new("logic.real"))
        .is_some());

    engine_asset::cook::write_cooked_artifact(
        &dir.join("audio.real.cooked"),
        AssetType::Audio.kind_code(),
        b"not a valid cooked audio payload",
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )
    .unwrap();
    let diagnostics = runtime.load_cooked_assets(&dir).unwrap_err();
    assert!(diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.message.contains("extension loader 'audio_clip'") }));
    assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
    assert_eq!(
        runtime
            .extension_asset::<engine_audio::AudioClip>("audio_clip", &AssetId::new("audio.real"),)
            .expect("previous audio remains installed")
            .get()
            .sample_rate(),
        8_000
    );
    let _ = std::fs::remove_dir_all(dir);
}
