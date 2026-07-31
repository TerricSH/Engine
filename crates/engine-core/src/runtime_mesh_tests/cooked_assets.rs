// ── Cooked-batch interaction ────────────────────────────────────────

fn cook_test_mesh(dir: &std::path::Path, id: &str) {
    let mesh = MeshData {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
        normals: vec![Vec3::Z; 3],
        uvs: vec![],
        indices: vec![0, 1, 2],
        bounds: (Vec3::ZERO, Vec3::ONE),
        joints: vec![],
        weights: vec![],
    };
    let payload = bincode::serialize(&mesh).expect("serialize mesh");
    engine_asset::cook::write_cooked_artifact(
        &dir.join(format!("{id}.cooked")),
        engine_asset::cook::AssetType::Mesh.kind_code(),
        &payload,
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )
    .expect("write cooked mesh");
}

fn cooked_case(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "engine_core_runtime_mesh_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cooked_replace_load_preserves_runtime_meshes() {
    let dir = cooked_case("replace_preserves");
    cook_test_mesh(&dir, "mesh-cooked-one");
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("terrain-chunk-0", triangle())
        .expect("create");
    let before = registered_upload(&runtime, handle);

    let report = runtime
        .load_cooked_assets(&dir)
        .expect("cooked load succeeds");
    assert_eq!(report.loaded_meshes, 1);

    assert_eq!(
        registered_upload(&runtime, handle),
        before,
        "cooked replace must not touch runtime meshes"
    );
    assert!(runtime
        .asset_registry()
        .get::<MeshUpload>(&AssetId::new("mesh-cooked-one"))
        .is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cooked_batch_naming_a_runtime_mesh_id_is_rejected() {
    let dir = cooked_case("replace_conflict");
    cook_test_mesh(&dir, "runtime-mesh-terrain-chunk-0");
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("terrain-chunk-0", triangle())
        .expect("create");
    let before = registered_upload(&runtime, handle);

    let diagnostics = runtime
        .load_cooked_assets(&dir)
        .expect_err("cooked asset colliding with a runtime mesh ID is rejected");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "AS0003"
                && diagnostic.message.contains("runtime-mesh-terrain-chunk-0")),
        "expected an AS0003 runtime-mesh conflict, got {diagnostics:?}"
    );
    assert_eq!(registered_upload(&runtime, handle), before);
    assert_eq!(runtime.runtime_mesh_memory().mesh_count, 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn additive_install_coexists_and_conflicts_cleanly() {
    let dir = cooked_case("additive_coexists");
    cook_test_mesh(&dir, "mesh-streamed-one");
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("chunk", triangle())
        .expect("create");

    let report = runtime
        .install_cooked_assets_additive(&[dir.join("mesh-streamed-one.cooked")])
        .expect("additive install of an unrelated mesh succeeds");
    assert_eq!(report.loaded_meshes, 1);
    assert_eq!(runtime.runtime_mesh_memory().mesh_count, 1);

    // An additive install whose ID matches the live runtime mesh is a
    // conflict, not an overwrite.
    cook_test_mesh(&dir, "runtime-mesh-chunk");
    let diagnostics = runtime
        .install_cooked_assets_additive(&[dir.join("runtime-mesh-chunk.cooked")])
        .expect_err("additive conflict");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("runtime-mesh-chunk")),
        "got {diagnostics:?}"
    );
    assert_eq!(registered_upload(&runtime, handle).vertex_count, 3);
    let _ = std::fs::remove_dir_all(dir);
}
