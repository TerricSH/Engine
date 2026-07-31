// ── Create / lifecycle ──────────────────────────────────────────────

#[test]
fn create_registers_mesh_and_reports_memory() {
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("terrain-chunk-0", triangle())
        .expect("create");

    let id = runtime
        .runtime_mesh_asset_id(handle)
        .expect("handle resolves");
    assert_eq!(id.id, "runtime-mesh-terrain-chunk-0");

    let upload = registered_upload(&runtime, handle);
    assert_eq!(upload.mesh_id, id);
    assert_eq!(upload.vertex_format, MeshVertexFormat::Pbr32);
    assert_eq!(upload.vertex_count, 3);
    assert_eq!(upload.index_count, 3);
    assert_eq!(upload.vertex_bytes.len(), 3 * 32);
    assert_eq!(upload.index_bytes.len(), 3 * 4);
    // Bounds were computed from the positions.
    assert_eq!(upload.bounds.min, [0.0, 0.0, 0.0]);
    assert_eq!(upload.bounds.max, [1.0, 1.0, 0.0]);

    let memory = runtime.runtime_mesh_memory();
    assert_eq!(memory.mesh_count, 1);
    assert_eq!(memory.vertex_count, 3);
    assert_eq!(memory.index_count, 3);
    assert_eq!(memory.vertex_bytes, 3 * 32);
    assert_eq!(memory.index_bytes, 3 * 4);
    assert_eq!(memory.total_bytes(), 3 * 32 + 3 * 4);
}

#[test]
fn explicit_bounds_are_preserved() {
    let mut runtime = runtime();
    let handle = runtime.create_runtime_mesh("quad", quad()).expect("create");
    let upload = registered_upload(&runtime, handle);
    assert_eq!(upload.bounds.min, [0.0, 0.0, 0.0]);
    assert_eq!(upload.bounds.max, [1.0, 1.0, 1.0]);
}

#[test]
fn create_rejects_invalid_names() {
    let mut runtime = runtime();
    for name in ["", "has/slash", "has\\backslash", ":", "..", "trail."] {
        let result = runtime.create_runtime_mesh(name, triangle());
        assert!(
            matches!(result, Err(RuntimeMeshError::InvalidName(_))),
            "name '{name}' must be rejected, got {result:?}"
        );
    }
}

#[test]
fn create_rejects_duplicate_name() {
    let mut runtime = runtime();
    runtime
        .create_runtime_mesh("dup", triangle())
        .expect("create");
    let result = runtime.create_runtime_mesh("dup", triangle());
    assert_eq!(
        result,
        Err(RuntimeMeshError::DuplicateName(
            "a live runtime mesh already uses name 'dup'".to_string()
        ))
    );
}

#[test]
fn create_rejects_id_occupied_by_foreign_asset() {
    let mut runtime = runtime();
    let foreign = triangle()
        .to_upload(AssetId::new("runtime-mesh-taken"))
        .unwrap();
    runtime.register_mesh_asset(foreign);

    let result = runtime.create_runtime_mesh("taken", triangle());
    assert!(
        matches!(result, Err(RuntimeMeshError::AssetIdConflict(_))),
        "got {result:?}"
    );
}

#[test]
fn create_rejects_invalid_geometry() {
    let mut runtime = runtime();

    let mut empty = triangle();
    empty.positions.clear();
    empty.indices.clear();
    assert!(matches!(
        runtime.create_runtime_mesh("empty", empty),
        Err(RuntimeMeshError::InvalidGeometry(_))
    ));

    let mut bad_indices = triangle();
    bad_indices.indices = vec![0, 1];
    assert!(matches!(
        runtime.create_runtime_mesh("bad-indices", bad_indices),
        Err(RuntimeMeshError::InvalidGeometry(_))
    ));

    let mut out_of_range = triangle();
    out_of_range.indices = vec![0, 1, 7];
    assert!(matches!(
        runtime.create_runtime_mesh("oob", out_of_range),
        Err(RuntimeMeshError::InvalidGeometry(_))
    ));

    let mut mismatched_normals = triangle();
    mismatched_normals.normals = vec![Vec3::Z];
    assert!(matches!(
        runtime.create_runtime_mesh("normals", mismatched_normals),
        Err(RuntimeMeshError::InvalidGeometry(_))
    ));

    let mut non_finite = triangle();
    non_finite.positions[1] = Vec3::new(f32::NAN, 0.0, 0.0);
    assert!(matches!(
        runtime.create_runtime_mesh("nan", non_finite),
        Err(RuntimeMeshError::InvalidGeometry(_))
    ));

    let mut bad_bounds = triangle();
    bad_bounds.bounds = Some((Vec3::ONE, Vec3::ZERO));
    assert!(matches!(
        runtime.create_runtime_mesh("bounds", bad_bounds),
        Err(RuntimeMeshError::InvalidGeometry(_))
    ));

    // None of the failed creates registered anything.
    assert_eq!(runtime.runtime_mesh_memory().mesh_count, 0);
}

// ── Full update ─────────────────────────────────────────────────────

#[test]
fn update_replaces_payload_and_memory() {
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("morph", triangle())
        .expect("create");
    let before = registered_upload(&runtime, handle);

    runtime
        .update_runtime_mesh(handle, quad())
        .expect("full update");
    let after = registered_upload(&runtime, handle);

    assert_eq!(after.mesh_id, before.mesh_id, "asset ID is stable");
    assert_eq!(after.vertex_count, 4);
    assert_eq!(after.index_count, 6);
    assert_eq!(after.vertex_bytes.len(), 4 * 32);
    assert_ne!(after.content_hash, before.content_hash);

    let memory = runtime.runtime_mesh_memory();
    assert_eq!(memory.mesh_count, 1);
    assert_eq!(memory.vertex_count, 4);
    assert_eq!(memory.index_count, 6);
    assert_eq!(memory.vertex_bytes, 4 * 32);
}

#[test]
fn update_validates_geometry() {
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("morph", triangle())
        .expect("create");
    let mut invalid = triangle();
    invalid.indices = vec![0, 1, 9];
    assert!(matches!(
        runtime.update_runtime_mesh(handle, invalid),
        Err(RuntimeMeshError::InvalidGeometry(_))
    ));
    // The original payload is untouched by the failed update.
    assert_eq!(registered_upload(&runtime, handle).vertex_count, 3);
}
