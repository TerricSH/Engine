fn write_skinned_gltf_fixture(directory: &Path) -> PathBuf {
    std::fs::create_dir_all(directory).unwrap();
    let gltf_path = directory.join("skinned.gltf");
    let mut bytes = Vec::new();
    for position in [[-1.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for value in position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for _ in 0..3 {
        bytes.extend_from_slice(&[0, 1, 1, 1]);
    }
    for _ in 0..3 {
        for value in [0.75f32, 0.25, 0.0, 0.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for index in [0u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes.extend_from_slice(&[0, 0]);
    for _ in 0..2 {
        for column in 0..4 {
            for row in 0..4 {
                let value = if column == row { 1.0f32 } else { 0.0 };
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    for time in [0.0f32, 1.0] {
        bytes.extend_from_slice(&time.to_le_bytes());
    }
    for translation in [[0.0f32, 1.0, 0.0], [0.0, 2.0, 0.0]] {
        for value in translation {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(directory.join("skinned.bin"), bytes).unwrap();
    std::fs::write(
            &gltf_path,
            r#"{
                "asset": { "version": "2.0" },
                "buffers": [{ "uri": "skinned.bin", "byteLength": 264 }],
                "bufferViews": [
                    { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
                    { "buffer": 0, "byteOffset": 36, "byteLength": 12 },
                    { "buffer": 0, "byteOffset": 48, "byteLength": 48 },
                    { "buffer": 0, "byteOffset": 96, "byteLength": 6 },
                    { "buffer": 0, "byteOffset": 104, "byteLength": 128 },
                    { "buffer": 0, "byteOffset": 232, "byteLength": 8 },
                    { "buffer": 0, "byteOffset": 240, "byteLength": 24 }
                ],
                "accessors": [
                    { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1, 0, 0], "max": [1, 1, 0] },
                    { "bufferView": 1, "componentType": 5121, "count": 3, "type": "VEC4" },
                    { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4" },
                    { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" },
                    { "bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4" },
                    { "bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1] },
                    { "bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC3" }
                ],
                "meshes": [{
                    "name": "SkinnedTriangle",
                    "primitives": [
                        {
                            "attributes": { "POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2 },
                            "indices": 3
                        },
                        {
                            "attributes": { "POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2 },
                            "indices": 3
                        }
                    ]
                }],
                "nodes": [
                    { "name": "Mesh", "mesh": 0, "skin": 0 },
                    { "name": "RootJoint", "children": [2] },
                    { "name": "ChildJoint" }
                ],
                "skins": [{ "name": "Rig", "joints": [2, 1], "skeleton": 1, "inverseBindMatrices": 4 }],
                "animations": [{
                    "name": "Raise",
                    "samplers": [{ "input": 5, "output": 6, "interpolation": "LINEAR" }],
                    "channels": [{ "sampler": 0, "target": { "node": 2, "path": "translation" } }]
                }],
                "scenes": [{ "nodes": [0, 1] }],
                "scene": 0
            }"#,
        )
        .unwrap();
    gltf_path
}

fn write_static_material_gltf_fixture(directory: &Path) -> PathBuf {
    std::fs::create_dir_all(directory).unwrap();
    let gltf_path = directory.join("drill.gltf");
    let mut bytes = Vec::new();
    for position in [
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [-0.1, 0.0, 0.0],
        [0.1, 0.0, 0.0],
        [0.0, 1.0, 0.0],
    ] {
        for value in position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(directory.join("drill.bin"), bytes).unwrap();
    image::RgbaImage::from_pixel(2, 1, image::Rgba([32, 96, 192, 224]))
        .save(directory.join("albedo.png"))
        .unwrap();
    std::fs::write(
        &gltf_path,
        r#"{
            "asset": { "version": "2.0" },
            "buffers": [{ "uri": "drill.bin", "byteLength": 72 }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
                { "buffer": 0, "byteOffset": 36, "byteLength": 36 }
            ],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] },
                { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-0.1, 0, 0], "max": [0.1, 1, 0] }
            ],
            "images": [{ "uri": "albedo.png" }],
            "textures": [{ "source": 0 }],
            "materials": [{
                "name": "Paint",
                "pbrMetallicRoughness": {
                    "baseColorFactor": [0.2, 0.4, 0.6, 0.8],
                    "baseColorTexture": { "index": 0 },
                    "metallicFactor": 0.7,
                    "roughnessFactor": 0.3
                },
                "normalTexture": { "index": 0 },
                "emissiveFactor": [0.1, 0.2, 0.3],
                "alphaMode": "MASK",
                "alphaCutoff": 0.25,
                "doubleSided": true
            }],
            "meshes": [
                { "name": "Base", "primitives": [{ "attributes": { "POSITION": 0 }, "material": 0 }] },
                { "name": "Drill", "primitives": [{ "attributes": { "POSITION": 1 }, "material": 0 }] }
            ],
            "nodes": [
                { "name": "Base", "mesh": 0 },
                { "name": "Drill", "mesh": 1, "translation": [0, 2, 0] }
            ],
            "scenes": [{ "nodes": [0, 1] }],
            "scene": 0
        }"#,
    )
    .unwrap();
    gltf_path
}

#[test]
fn parses_headless_project_run() {
    let request =
        parse_run_request(&["my-game".into(), "--headless".into(), "--frames=4".into()]).unwrap();
    assert_eq!(request.project, PathBuf::from("my-game"));
    assert!(request.headless);
    assert_eq!(request.frames, Some(4));
    assert_eq!(request.report, None);
    assert!(!request.scripts_already_built);
    assert!(!request.stream_cells);
}

#[test]
fn parses_editor_run_handoff_without_changing_normal_run_defaults() {
    let request = parse_run_request(&["my-game".into(), "--scripts-already-built".into()]).unwrap();
    assert!(request.scripts_already_built);
}

#[test]
fn gltf_project_import_generates_mesh_skeleton_animation_and_copies_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path().join("gltf-project");
    create_project(&project_root, Some("glTF Project"), false).unwrap();
    let gltf_path = write_skinned_gltf_fixture(&temp.path().join("external"));

    import_project_asset(&ProjectImportRequest {
        project: project_root.clone(),
        source_file: gltf_path,
        asset_id: "hero".into(),
        asset_type: None,
        folder: PathBuf::new(),
        merge_primitives: true,
        bake_node_transforms: Some(false),
    })
    .unwrap();

    let source_root = project_root.join("assets/source");
    assert!(source_root.join("skinned.gltf").is_file());
    assert!(source_root.join("skinned.bin").is_file());
    assert!(source_root.join("hero.skin0.skel").is_file());
    assert!(source_root.join("hero.skin0.animation0.anim").is_file());

    let manifest: SourceManifest =
        serde_json::from_slice(&std::fs::read(source_root.join("game.manifest")).unwrap()).unwrap();
    let ids = manifest
        .assets
        .iter()
        .map(|entry| entry.id.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        ids,
        BTreeSet::from([
            "hero",
            "hero.skeleton.0",
            "hero.animation.0.0",
        ])
    );

    let cooked_root = project_root.join("build/cooked");
    let mesh = read_cooked_artifact(&cooked_root.join("hero.cooked")).unwrap();
    assert_eq!(mesh.header.asset_kind, AssetType::Mesh.kind_code());
    let merged_mesh = engine_asset::cook::decode_cooked_mesh(&mesh).unwrap();
    assert_eq!(merged_mesh.positions.len(), 6);
    assert_eq!(merged_mesh.indices.len(), 6);
    let skeleton = read_cooked_artifact(&cooked_root.join("hero.skeleton.0.cooked")).unwrap();
    assert_eq!(skeleton.header.asset_kind, AssetType::Skeleton.kind_code());
    assert_eq!(
        engine_animation::load_skeleton(&skeleton.payload)
            .unwrap()
            .joint_count(),
        2
    );
    let animation = read_cooked_artifact(&cooked_root.join("hero.animation.0.0.cooked")).unwrap();
    assert_eq!(
        animation.header.asset_kind,
        AssetType::Animation.kind_code()
    );
    assert_eq!(
        engine_animation::load_animation_clip(&animation.payload)
            .unwrap()
            .name(),
        "Raise"
    );
    check_project(&project_root, None).unwrap();
    cook_project(&project_root).unwrap();
    check_project(&project_root, None).unwrap();
}

#[test]
fn gltf_project_import_merges_primitives_bakes_nodes_and_generates_material_assets() {
    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path().join("drill-project");
    create_project(&project_root, Some("Drill Project"), false).unwrap();
    let gltf_path = write_static_material_gltf_fixture(&temp.path().join("external-drill"));

    import_project_asset(&ProjectImportRequest {
        project: project_root.clone(),
        source_file: gltf_path,
        asset_id: "drill-structure".into(),
        asset_type: None,
        folder: PathBuf::new(),
        merge_primitives: true,
        bake_node_transforms: Some(true),
    })
    .unwrap();

    let source_root = project_root.join("assets/source");
    for source in [
        "drill.gltf",
        "drill.bin",
        "albedo.png",
        "drill-structure.texture.0.png",
        "drill-structure.material.0.material.json",
    ] {
        assert!(source_root.join(source).is_file(), "missing {source}");
    }
    let manifest: SourceManifest =
        serde_json::from_slice(&std::fs::read(source_root.join("game.manifest")).unwrap()).unwrap();
    let ids = manifest
        .assets
        .iter()
        .map(|entry| entry.id.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        ids,
        BTreeSet::from([
            "drill-structure",
            "drill-structure.material.0",
            "drill-structure.texture.0",
        ])
    );
    let mesh_entry = manifest
        .assets
        .iter()
        .find(|entry| entry.id.id == "drill-structure")
        .unwrap();
    assert!(mesh_entry.cook_rules.gltf_merge_primitives);
    assert!(mesh_entry.cook_rules.gltf_bake_node_transforms);

    let cooked_root = project_root.join("build/cooked");
    let mesh_artifact =
        read_cooked_artifact(&cooked_root.join("drill-structure.cooked")).unwrap();
    let mesh = engine_asset::cook::decode_cooked_mesh(&mesh_artifact).unwrap();
    assert_eq!(mesh.positions.len(), 6);
    assert_eq!(mesh.indices.len(), 6);
    assert_eq!(mesh.bounds.0.y, 0.0);
    assert_eq!(mesh.bounds.1.y, 3.0);
    assert!(mesh
        .positions
        .iter()
        .any(|position| (position.y - 2.0).abs() < 1.0e-6));

    let material_source: engine_asset::cook::MaterialSource = serde_json::from_slice(
        &std::fs::read(source_root.join("drill-structure.material.0.material.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(material_source.base_color, [0.2, 0.4, 0.6, 0.8]);
    assert_eq!(material_source.metallic, 0.7);
    assert_eq!(material_source.roughness, 0.3);
    assert_eq!(material_source.emissive, [0.1, 0.2, 0.3]);
    assert_eq!(material_source.transparency, "Masked");
    assert_eq!(material_source.alpha_cutoff, 0.25);
    assert!(material_source.double_sided);
    assert_eq!(
        material_source.base_color_texture.as_deref(),
        Some("drill-structure.texture.0")
    );
    assert_eq!(
        material_source.normal_texture.as_deref(),
        Some("drill-structure.texture.0")
    );

    let material_artifact =
        read_cooked_artifact(&cooked_root.join("drill-structure.material.0.cooked")).unwrap();
    let material = engine_asset::cook::decode_cooked_material(&material_artifact).unwrap();
    assert_eq!(
        material.base_color_texture.as_ref().map(|id| id.id.as_str()),
        Some("drill-structure.texture.0")
    );
    let texture_artifact =
        read_cooked_artifact(&cooked_root.join("drill-structure.texture.0.cooked")).unwrap();
    let texture = engine_asset::cook::decode_cooked_texture(&texture_artifact).unwrap();
    assert_eq!((texture.width, texture.height), (2, 1));

    check_project(&project_root, None).unwrap();
    cook_project(&project_root).unwrap();
    check_project(&project_root, None).unwrap();
}

#[test]
fn parses_stream_cells_project_run() {
    let request = parse_run_request(&[
        "my-game".into(),
        "--headless".into(),
        "--stream-cells".into(),
    ])
    .unwrap();
    assert!(request.stream_cells);
}

#[test]
fn rejects_zero_frames_and_extra_projects() {
    assert!(parse_run_request(&["game".into(), "--frames=0".into()]).is_err());
    assert!(parse_run_request(&["one".into(), "two".into()]).is_err());
}

#[test]
fn parses_csharp_project_creation_option() {
    let (root, name, with_csharp) = parse_new_args(&[
        "managed-game".into(),
        "--name".into(),
        "Managed Game".into(),
        "--with-csharp".into(),
    ])
    .unwrap();
    assert_eq!(root, PathBuf::from("managed-game"));
    assert_eq!(name.as_deref(), Some("Managed Game"));
    assert!(with_csharp);
}

#[test]
fn project_creation_installs_a_cataloged_basic_scene() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("basic-scene-project");

    create_project(&root, Some("Basic Scene Project"), false).unwrap();

    let project = GameProject::load(&root).unwrap();
    assert_eq!(project.startup_scene_id(), "main");
    assert_eq!(project.scenes().len(), 1);
    let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
    assert_eq!(scene.scene_id, "main");
    assert_eq!(scene.name, "Main");
    assert_eq!(scene.entities.len(), 3);
    assert!(scene.entities.iter().any(|entity| {
        entity.name.as_deref() == Some("Main Camera")
            && entity.components.contains_key("engine.camera")
    }));
    assert!(scene.entities.iter().any(|entity| {
        entity.name.as_deref() == Some("Cube")
            && entity.components.contains_key("engine.renderable")
    }));
    assert!(scene.entities.iter().any(|entity| {
        entity.name.as_deref() == Some("Directional Light")
            && entity.components.contains_key("engine.light")
    }));
}

#[test]
fn scripted_project_creation_normalizes_a_parent_segment_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("holder")).unwrap();
    let requested = temp
        .path()
        .join("holder")
        .join("..")
        .join("normalized-script-project");

    create_project(&requested, Some("Normalized Script Project"), true).unwrap();

    let normalized = temp.path().join("normalized-script-project");
    let project = GameProject::load(&normalized).unwrap();
    assert!(project.script_project.unwrap().is_file());
    assert!(normalized
        .join("scripts/GameScripts/EngineGameplay.contract.json")
        .is_file());
    assert!(!normalized.join("build/script-sdk-source").exists());
}

#[test]
fn parses_project_import_options() {
    let request = parse_import_args(&[
        "game".into(),
        "checker.ppm".into(),
        "--id=checker-main".into(),
        "--type".into(),
        "TeXtUrE".into(),
        "--folder".into(),
        "Textures/UI".into(),
    ])
    .unwrap();
    assert_eq!(request.project, PathBuf::from("game"));
    assert_eq!(request.source_file, PathBuf::from("checker.ppm"));
    assert_eq!(request.asset_id, "checker-main");
    assert_eq!(request.asset_type, Some(AssetType::Texture));
    assert_eq!(request.folder, PathBuf::from("Textures/UI"));
    assert!(request.merge_primitives);
    assert_eq!(request.bake_node_transforms, None);

    let audio = parse_import_args(&[
        "game".into(),
        "ambient.wav".into(),
        "--id=ambient".into(),
        "--type=audio".into(),
    ])
    .unwrap();
    assert_eq!(audio.asset_type, Some(AssetType::Audio));
    assert!(audio.folder.as_os_str().is_empty());

    let separate = parse_import_args(&[
        "game".into(),
        "model.glb".into(),
        "--id=model".into(),
        "--separate-primitives".into(),
        "--no-bake-node-transforms".into(),
    ])
    .unwrap();
    assert!(!separate.merge_primitives);
    assert_eq!(separate.bake_node_transforms, Some(false));
}

#[test]
fn rejects_invalid_project_import_arguments() {
    assert!(parse_import_args(&["game".into(), "asset.ppm".into()]).is_err());
    assert!(parse_import_args(&[
        "game".into(),
        "asset.ppm".into(),
        "--id".into(),
        "asset".into(),
        "--type=font".into(),
    ])
    .is_err());
    assert!(validate_import_asset_id("../escape").is_err());
    assert!(validate_import_asset_id("CON").is_err());
}
