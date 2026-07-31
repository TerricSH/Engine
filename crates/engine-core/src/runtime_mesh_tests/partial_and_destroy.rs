// ── Partial vertex update ───────────────────────────────────────────

fn vertex(x: f32, y: f32, z: f32) -> Pbr32Vertex {
    Pbr32Vertex {
        position: [x, y, z],
        normal: [0.0, 0.0, 1.0],
        uv0: [0.0, 0.0],
    }
}

fn read_position(upload: &MeshUpload, vertex_index: usize) -> [f32; 3] {
    let stride = MeshVertexFormat::Pbr32.stride_bytes() as usize;
    let offset = vertex_index * stride;
    let mut position = [0.0_f32; 3];
    for (axis, slot) in position.iter_mut().enumerate() {
        let start = offset + axis * 4;
        *slot = f32::from_ne_bytes(
            upload.vertex_bytes[start..start + 4]
                .try_into()
                .expect("four bytes per float"),
        );
    }
    position
}

#[test]
fn partial_vertex_update_rewrites_only_the_target_range() {
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("deform", quad())
        .expect("create");
    let before = registered_upload(&runtime, handle);

    runtime
        .update_runtime_mesh_vertices(handle, 1, &[vertex(5.0, 0.0, 0.0), vertex(6.0, 1.0, 0.0)])
        .expect("partial update");
    let after = registered_upload(&runtime, handle);

    assert_eq!(read_position(&after, 0), read_position(&before, 0));
    assert_eq!(read_position(&after, 1), [5.0, 0.0, 0.0]);
    assert_eq!(read_position(&after, 2), [6.0, 1.0, 0.0]);
    assert_eq!(read_position(&after, 3), read_position(&before, 3));
    assert_eq!(after.index_bytes, before.index_bytes);
    assert_eq!(after.bounds, before.bounds, "partial edits keep bounds");
    assert_ne!(after.content_hash, before.content_hash);
    // Counts and memory are unchanged by in-place edits.
    assert_eq!(runtime.runtime_mesh_memory().vertex_count, 4);
}

#[test]
fn partial_vertex_update_rejects_invalid_ranges() {
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("deform", quad())
        .expect("create");

    assert!(matches!(
        runtime.update_runtime_mesh_vertices(handle, 0, &[]),
        Err(RuntimeMeshError::InvalidUpdateRange(_))
    ));
    assert!(matches!(
        runtime.update_runtime_mesh_vertices(handle, 3, &[vertex(0.0, 0.0, 0.0); 2]),
        Err(RuntimeMeshError::InvalidUpdateRange(_))
    ));
    assert!(matches!(
        runtime.update_runtime_mesh_vertices(handle, 4, &[vertex(0.0, 0.0, 0.0)]),
        Err(RuntimeMeshError::InvalidUpdateRange(_))
    ));
}

// ── Destroy / handle lifecycle ──────────────────────────────────────

#[test]
fn destroy_removes_mesh_and_zeroes_memory() {
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("temp", triangle())
        .expect("create");
    let id = runtime.runtime_mesh_asset_id(handle).unwrap();

    runtime.destroy_runtime_mesh(handle).expect("destroy");

    assert!(runtime.asset_registry().get::<MeshUpload>(&id).is_none());
    assert!(!runtime.asset_registry().contains(&id));
    assert!(runtime.runtime_mesh_asset_id(handle).is_none());
    let memory = runtime.runtime_mesh_memory();
    assert_eq!(memory.mesh_count, 0);
    assert_eq!(memory.total_bytes(), 0);
}

#[test]
fn stale_and_unknown_handles_are_errors_not_panics() {
    let mut runtime = runtime();
    let handle = runtime
        .create_runtime_mesh("temp", triangle())
        .expect("create");
    runtime.destroy_runtime_mesh(handle).expect("destroy");

    assert_eq!(
        runtime.update_runtime_mesh(handle, triangle()),
        Err(RuntimeMeshError::StaleHandle {
            slot: handle.slot()
        })
    );
    assert_eq!(
        runtime.destroy_runtime_mesh(handle),
        Err(RuntimeMeshError::StaleHandle {
            slot: handle.slot()
        })
    );
    assert_eq!(
        runtime.update_runtime_mesh_vertices(handle, 0, &[vertex(0.0, 0.0, 0.0)]),
        Err(RuntimeMeshError::StaleHandle {
            slot: handle.slot()
        })
    );

    let bogus = RuntimeMeshHandle {
        slot: 99,
        generation: 1,
    };
    assert_eq!(
        runtime.destroy_runtime_mesh(bogus),
        Err(RuntimeMeshError::UnknownHandle { slot: 99 })
    );
    assert!(runtime.runtime_mesh_asset_id(bogus).is_none());
}

#[test]
fn recreate_after_destroy_issues_a_fresh_generation() {
    let mut runtime = runtime();
    let old = runtime
        .create_runtime_mesh("cycle", triangle())
        .expect("create");
    runtime.destroy_runtime_mesh(old).expect("destroy");

    let new = runtime
        .create_runtime_mesh("cycle", quad())
        .expect("re-create under the same name");
    assert_eq!(new.slot(), old.slot(), "slots are reused");
    assert_ne!(new.generation(), old.generation());
    assert_eq!(
        runtime.update_runtime_mesh(old, triangle()),
        Err(RuntimeMeshError::StaleHandle { slot: old.slot() })
    );
    assert_eq!(registered_upload(&runtime, new).vertex_count, 4);
    // Registry reconciliation sees one live entry, so the next upload
    // replaces the buffers without issuing a stale removal first.
}

#[test]
fn multiple_meshes_accumulate_memory() {
    let mut runtime = runtime();
    let a = runtime
        .create_runtime_mesh("a", triangle())
        .expect("create");
    let _b = runtime.create_runtime_mesh("b", quad()).expect("create");
    let memory = runtime.runtime_mesh_memory();
    assert_eq!(memory.mesh_count, 2);
    assert_eq!(memory.vertex_count, 3 + 4);
    assert_eq!(memory.index_count, 3 + 6);
    assert_eq!(memory.vertex_bytes, (3 + 4) * 32);
    assert_eq!(memory.index_bytes, (3 + 6) * 4);

    runtime.destroy_runtime_mesh(a).expect("destroy");
    let memory = runtime.runtime_mesh_memory();
    assert_eq!(memory.mesh_count, 1);
    assert_eq!(memory.vertex_count, 4);
}
