use super::*;

#[test]
fn cooked_asset_header_creation() {
    let hash = [0xABu8; 32];
    let header = CookedAssetHeader::new(1, SchemaVersion::new(0, 1, 0), hash, 4096);
    assert_eq!(&header.magic, COOKED_MAGIC);
    assert_eq!(header.header_version, 1);
    assert_eq!(header.asset_kind, 1);
    assert_eq!(header.uncompressed_size, 4096);
    assert!(header.is_valid());
}

#[test]
fn cooked_asset_header_invalid_magic() {
    let mut header = CookedAssetHeader::new(1, SchemaVersion::new(0, 1, 0), [0u8; 32], 100);
    header.magic = [0; 8];
    assert!(!header.is_valid());
}

#[test]
fn cooked_asset_header_serde_roundtrip() {
    let hash = [0x42u8; 32];
    let header = CookedAssetHeader::new(3, SchemaVersion::new(1, 0, 0), hash, 8192);
    let bytes = bincode::serialize(&header).unwrap();
    let restored: CookedAssetHeader = bincode::deserialize(&bytes).unwrap();
    assert!(restored.is_valid());
    assert_eq!(restored.asset_kind, 3);
    assert_eq!(restored.schema_version, SchemaVersion::new(1, 0, 0));
    assert_eq!(restored.content_hash, hash);
    assert_eq!(restored.uncompressed_size, 8192);
}

#[test]
fn write_and_read_cooked_artifact() {
    use std::io::Read;

    let dir = std::env::temp_dir().join("cook_test_write_read");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let output = dir.join("test_mesh.cooked");
    let payload = vec![0x01, 0x02, 0x03, 0x04];

    let result = write_cooked_artifact(&output, 1, &payload, SchemaVersion::new(0, 1, 0)).unwrap();
    assert!(result.success);
    assert_eq!(result.asset_id, "test_mesh");

    // Read back and verify.
    let mut file = std::fs::File::open(&output).unwrap();
    let mut file_bytes = Vec::new();
    file.read_to_end(&mut file_bytes).unwrap();

    // Header size: bincode serialized size of CookedAssetHeader.
    let header: CookedAssetHeader = bincode::deserialize(&file_bytes[..]).unwrap();
    assert!(header.is_valid());
    assert_eq!(header.asset_kind, 1);

    // Payload after header.
    let header_size = bincode::serialized_size(&header).unwrap() as usize;
    let read_payload = &file_bytes[header_size..];
    assert_eq!(read_payload, &payload);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn public_cooked_reader_validates_payload() {
    let dir = std::env::temp_dir().join(format!("cook_public_reader_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("verified.cooked");
    write_cooked_artifact(
        &output,
        AssetType::Unknown.kind_code(),
        b"verified-payload",
        SchemaVersion::new(0, 1, 0),
    )
    .unwrap();

    let artifact = read_cooked_artifact(&output).unwrap();
    assert_eq!(artifact.payload, b"verified-payload");
    assert_eq!(artifact.header.asset_kind, AssetType::Unknown.kind_code());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn public_cooked_reader_rejects_corruption() {
    let dir =
        std::env::temp_dir().join(format!("cook_public_reader_corrupt_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("corrupt.cooked");
    write_cooked_artifact(
        &output,
        AssetType::Unknown.kind_code(),
        b"verified-payload",
        SchemaVersion::new(0, 1, 0),
    )
    .unwrap();
    let mut bytes = std::fs::read(&output).unwrap();
    *bytes.last_mut().unwrap() ^= 0xFF;
    std::fs::write(&output, bytes).unwrap();

    let error = read_cooked_artifact(&output).unwrap_err();
    assert!(error.to_string().contains("hash mismatch"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn determine_shader_stage_by_extension() {
    assert_eq!(determine_shader_stage(Path::new("shader.vert")), "vertex");
    assert_eq!(determine_shader_stage(Path::new("shader.frag")), "fragment");
    assert_eq!(determine_shader_stage(Path::new("shader.comp")), "compute");
    assert_eq!(
        determine_shader_stage(Path::new("shader.unknown")),
        "vertex"
    );
}

#[test]
fn asset_type_kind_code_mapping() {
    assert_eq!(AssetType::Mesh.kind_code(), 1);
    assert_eq!(AssetType::from_kind_code(1), AssetType::Mesh);
    assert_eq!(AssetType::Unknown.kind_code(), 0xFFFF);
    assert_eq!(AssetType::from_kind_code(0xFFFF), AssetType::Unknown);
}

fn cook_case(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "engine_asset_checked_cook_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let source = root.join("source");
    let cooked = root.join("cooked");
    (root, source, cooked)
}

fn write_manifest(path: &Path, assets: Vec<SourceAssetEntry>) {
    let manifest = SourceManifest {
        schema_version: manifest::CURRENT_MANIFEST_VERSION,
        assets,
    };
    std::fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}

fn entry(id: &str, asset_type: AssetType, source_path: &str) -> SourceAssetEntry {
    SourceAssetEntry {
        id: AssetId::new(id),
        asset_type,
        source_path: source_path.into(),
        cook_rules: CookRules::default(),
    }
}

#[test]
fn checked_cook_allows_an_absent_source_directory_as_an_empty_asset_set() {
    let (root, source, cooked) = cook_case("missing_source");
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked(&source, &cooked, &mut graph);

    assert!(report.is_success());
    assert!(!report.source_directory_present);
    assert_eq!(report.manifest_count, 0);
    assert!(cooked.is_dir());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checked_cook_surfaces_malformed_manifests() {
    let (root, source, cooked) = cook_case("malformed_manifest");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("broken.manifest"), b"{").unwrap();
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked(&source, &cooked, &mut graph);

    assert!(!report.is_success());
    assert_eq!(report.failed_manifest_count, 1);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "COOK_MANIFEST_PARSE_FAILED"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checked_cook_rejects_unsupported_manifest_versions() {
    let (root, source, cooked) = cook_case("manifest_version");
    std::fs::create_dir_all(&source).unwrap();
    let manifest = SourceManifest {
        schema_version: SchemaVersion::new(99, 0, 0),
        assets: vec![],
    };
    std::fs::write(
        source.join("future.manifest"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked(&source, &cooked, &mut graph);

    assert!(!report.is_success());
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "COOK_MANIFEST_VERSION_UNSUPPORTED"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checked_cook_rejects_parent_directory_source_paths() {
    let (root, source, cooked) = cook_case("unsafe_path");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(root.join("outside.logic"), b"{}").unwrap();
    write_manifest(
        &source.join("assets.manifest"),
        vec![entry("logic-main", AssetType::Logic, "../outside.logic")],
    );
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked(&source, &cooked, &mut graph);

    assert!(!report.is_success());
    assert_eq!(report.failed_asset_count, 1);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "COOK_SOURCE_PATH_INVALID"));
    assert!(!cooked.join("logic-main.cooked").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checked_cook_surfaces_cooker_failures_and_graph_state() {
    let (root, source, cooked) = cook_case("cooker_failure");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("broken.logic"), b"not-json").unwrap();
    write_manifest(
        &source.join("assets.manifest"),
        vec![entry("logic-main", AssetType::Logic, "broken.logic")],
    );
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked(&source, &cooked, &mut graph);

    assert!(!report.is_success());
    assert_eq!(report.failed_asset_count, 1);
    assert!(matches!(
        graph.get_state(&AssetId::new("logic-main")),
        Some(CookState::Failed(_))
    ));
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "COOK_ASSET_FAILED"));
    assert!(!cooked.join("logic-main.cooked").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checked_cook_dispatches_material_cooker() {
    let (root, source, cooked) = cook_case("material");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("sample.material.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": MATERIAL_SOURCE_SCHEMA,
            "base_color": [0.9, 0.8, 0.7, 1.0],
            "metallic": 0.1,
            "roughness": 0.6,
            "ambient_occlusion": 1.0,
            "base_color_texture": "sample-texture",
            "transparency": "Opaque",
            "double_sided": false
        }))
        .unwrap(),
    )
    .unwrap();
    write_manifest(
        &source.join("assets.manifest"),
        vec![entry(
            "sample-material",
            AssetType::Material,
            "sample.material.json",
        )],
    );
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked(&source, &cooked, &mut graph);

    assert!(report.is_success(), "{:?}", report.diagnostics);
    assert_eq!(report.succeeded_asset_count, 1);
    let artifact = read_cooked_artifact(&cooked.join("sample-material.cooked")).unwrap();
    let material = decode_cooked_material(&artifact).unwrap();
    assert_eq!(material.metallic, 0.1);
    assert_eq!(
        material.base_color_texture,
        Some(AssetId::new("sample-texture"))
    );
    assert!(matches!(
        graph.get_state(&AssetId::new("sample-material")),
        Some(CookState::Cooked { .. })
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checked_cook_orders_manifests_and_assets_deterministically() {
    let (root, source, cooked) = cook_case("deterministic_order");
    std::fs::create_dir_all(&source).unwrap();
    for name in ["a.data", "m.data", "z.data"] {
        std::fs::write(source.join(name), name.as_bytes()).unwrap();
    }
    write_manifest(
        &source.join("b.manifest"),
        vec![entry("z", AssetType::Material, "z.data")],
    );
    write_manifest(
        &source.join("a.manifest"),
        vec![
            entry("m", AssetType::Material, "m.data"),
            entry("a", AssetType::Material, "a.data"),
        ],
    );
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked(&source, &cooked, &mut graph);
    let ids: Vec<_> = report
        .results
        .iter()
        .map(|result| result.asset_id.as_str())
        .collect();

    assert_eq!(ids, vec!["a", "m", "z"]);
    assert_eq!(report.declared_asset_count, 3);
    assert_eq!(report.failed_asset_count, 3);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checked_cook_rejects_every_case_insensitive_duplicate_before_writing() {
    let (root, source, cooked) = cook_case("duplicate_asset_ids");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("one.data"), b"one").unwrap();
    std::fs::write(source.join("two.data"), b"two").unwrap();
    write_manifest(
        &source.join("assets.manifest"),
        vec![
            entry("Duplicate", AssetType::Material, "one.data"),
            entry("duplicate", AssetType::Material, "two.data"),
        ],
    );
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked(&source, &cooked, &mut graph);

    assert_eq!(report.succeeded_asset_count, 0);
    assert_eq!(report.failed_asset_count, 2);
    assert!(report
        .results
        .iter()
        .all(|result| !result.success && result.diagnostics[0].code == "COOK_ASSET_ID_DUPLICATE"));
    assert!(!cooked.join("Duplicate.cooked").exists());
    assert!(!cooked.join("duplicate.cooked").exists());
    let _ = std::fs::remove_dir_all(root);
}

fn test_extension_cooker(source: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
    if source != b"valid-extension-source" {
        return Err("unexpected source payload".into());
    }
    output.extend_from_slice(b"valid-extension-payload");
    Ok(())
}

fn test_extension_loader(cooked: &[u8]) -> Result<Box<dyn std::any::Any + Send + Sync>, String> {
    if cooked != b"valid-extension-payload" {
        return Err("unexpected cooked payload".into());
    }
    Ok(Box::new(cooked.len()))
}

fn rejecting_extension_loader(_: &[u8]) -> Result<Box<dyn std::any::Any + Send + Sync>, String> {
    Err("runtime rejected payload".into())
}

fn audio_extension_registry(loader: engine_scene::registry::LoaderFn) -> AssetTypeRegistry {
    use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

    let mut registry = AssetTypeRegistry::new();
    registry
        .register(AssetTypeExtension {
            meta: AssetTypeMeta {
                type_id: "audio_clip",
                source_extensions: vec!["test-audio"],
                display_name: "Test Audio",
            },
            cooker: Some(test_extension_cooker),
            loader: Some(loader),
        })
        .unwrap();
    registry
}

#[test]
fn checked_cook_wraps_registered_extension_payload_in_standard_artifact() {
    let (root, source, cooked) = cook_case("registered_extension");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("clip.test-audio"), b"valid-extension-source").unwrap();
    write_manifest(
        &source.join("assets.manifest"),
        vec![entry("test-audio", AssetType::Audio, "clip.test-audio")],
    );
    let registry = audio_extension_registry(test_extension_loader);
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked_with_registry(&source, &cooked, &mut graph, &registry);

    assert!(report.is_success(), "{:?}", report.diagnostics);
    let artifact = read_cooked_artifact(&cooked.join("test-audio.cooked")).unwrap();
    assert_eq!(artifact.header.asset_kind, AssetType::Audio.kind_code());
    assert_eq!(artifact.payload, b"valid-extension-payload");
    let expected_hash: HashDigest = Sha256::digest(b"valid-extension-payload").into();
    assert_eq!(artifact.header.content_hash, expected_hash);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checked_cook_rejects_extension_kind_without_registered_cooker() {
    let (root, source, cooked) = cook_case("unregistered_extension");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("clip.test-audio"), b"valid-extension-source").unwrap();
    write_manifest(
        &source.join("assets.manifest"),
        vec![entry("test-audio", AssetType::Audio, "clip.test-audio")],
    );
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked(&source, &cooked, &mut graph);

    assert!(!report.is_success());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "COOK_ASSET_FAILED"
            && diagnostic.message.contains("requires registered extension")
    }));
    assert!(!cooked.join("test-audio.cooked").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checked_cook_does_not_commit_payload_rejected_by_registered_loader() {
    let (root, source, cooked) = cook_case("extension_loader_rejection");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("clip.test-audio"), b"valid-extension-source").unwrap();
    write_manifest(
        &source.join("assets.manifest"),
        vec![entry("test-audio", AssetType::Audio, "clip.test-audio")],
    );
    let registry = audio_extension_registry(rejecting_extension_loader);
    let mut graph = DependencyGraph::new();

    let report = cook_orchestrate_checked_with_registry(&source, &cooked, &mut graph, &registry);

    assert!(!report.is_success());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "COOK_ASSET_FAILED"
            && diagnostic.message.contains("runtime rejected payload")
    }));
    assert!(!cooked.join("test-audio.cooked").exists());
    let _ = std::fs::remove_dir_all(root);
}
