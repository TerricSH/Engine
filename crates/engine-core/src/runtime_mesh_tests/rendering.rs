// ── Rendering integration / GPU deferral ────────────────────────────

#[derive(Default)]
struct MeshTrace {
    uploads: Vec<(String, [u8; 32])>,
    removals: Vec<String>,
    removal_failures_remaining: usize,
}

/// Headless backend that records mesh uploads/removals and reports one
/// draw call per extracted drawable, mirroring the sandbox contract
/// backend.
struct TraceBackend {
    trace: Arc<Mutex<MeshTrace>>,
}

impl BackendRenderer for TraceBackend {
    fn begin_frame(
        &mut self,
        _input: &engine_renderer::RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &engine_renderer::RenderFrameInput,
        _pass: &engine_renderer::render_graph2::PassNode,
        _barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn execute_pass(
        &mut self,
        input: &engine_renderer::RenderFrameInput,
        pass: &engine_renderer::render_graph2::PassNode,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if pass.kind == engine_renderer::render_graph2::PassKind::OpaquePbrForward {
            stats.draw_calls += input.drawables.len() as u32;
            stats.visible_drawables = input.drawables.len() as u32;
        }
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn upload_mesh(
        &mut self,
        upload: MeshUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        self.trace
            .lock()
            .unwrap()
            .uploads
            .push((upload.mesh_id.id.clone(), upload.content_hash));
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_texture(
        &mut self,
        _upload: engine_renderer::TextureUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        _upload: engine_renderer::MaterialUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn remove_resource(&mut self, removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        let mut trace = self.trace.lock().unwrap();
        if trace.removal_failures_remaining > 0 {
            trace.removal_failures_remaining -= 1;
            return Err(vec![Diagnostic::new(
                "TEST_RESOURCE_REMOVAL_FAILED",
                DiagnosticSeverity::Error,
                "runtime-mesh-test",
                "injected backend removal failure",
            )]);
        }
        trace.removals.push(removal.resource_id.id.clone());
        Ok(())
    }
}

fn sample_scene_with_mesh(mesh_id: &str) -> engine_scene::Scene {
    let mut scene = engine_scene::sample_scene();
    let entity = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("sample scene cube entity");
    let renderable = entity
        .components
        .get_mut("engine.renderable")
        .expect("renderable component");
    renderable.fields.insert(
        "mesh".to_string(),
        engine_serialize::Value::Asset(AssetId::new(mesh_id)),
    );
    scene
}

#[test]
fn renderable_referencing_runtime_mesh_produces_draw_calls() {
    let _guard = crate::tests::serial_ffi_world_test();
    let trace = Arc::new(Mutex::new(MeshTrace::default()));
    let mut runtime = runtime();
    runtime.set_renderer_backend(Box::new(TraceBackend {
        trace: Arc::clone(&trace),
    }));
    let handle = runtime
        .create_runtime_mesh("terrain-chunk-0", quad())
        .expect("create");
    let id = runtime.runtime_mesh_asset_id(handle).unwrap();
    runtime
        .load_scene(sample_scene_with_mesh(&id.id))
        .expect("scene loads");

    let stats = runtime.render_frame(0).expect("frame renders");

    assert_eq!(stats.visible_drawables, 1);
    assert_eq!(stats.draw_calls, 1);
    let trace = trace.lock().unwrap();
    assert!(
        trace
            .uploads
            .iter()
            .any(|(id, _)| id == "runtime-mesh-terrain-chunk-0"),
        "runtime mesh must be uploaded through the standard sync path, got {:?}",
        trace.uploads
    );
}

#[test]
fn updated_runtime_mesh_is_reuploaded_with_new_content() {
    let _guard = crate::tests::serial_ffi_world_test();
    let trace = Arc::new(Mutex::new(MeshTrace::default()));
    let mut runtime = runtime();
    runtime.set_renderer_backend(Box::new(TraceBackend {
        trace: Arc::clone(&trace),
    }));
    let handle = runtime
        .create_runtime_mesh("morph", triangle())
        .expect("create");
    let id = runtime.runtime_mesh_asset_id(handle).unwrap();
    runtime
        .load_scene(sample_scene_with_mesh(&id.id))
        .expect("scene loads");

    runtime.render_frame(0).expect("first frame");
    runtime
        .update_runtime_mesh(handle, quad())
        .expect("full update");
    runtime.render_frame(1).expect("second frame");

    let trace = trace.lock().unwrap();
    let hashes: Vec<[u8; 32]> = trace
        .uploads
        .iter()
        .filter(|(id, _)| id == "runtime-mesh-morph")
        .map(|(_, hash)| *hash)
        .collect();
    assert_eq!(hashes.len(), 2, "one upload per frame, got {hashes:?}");
    assert_ne!(hashes[0], hashes[1], "update changed the uploaded content");
}

#[test]
fn destroy_defers_gpu_removal_to_the_next_frame_boundary() {
    let _guard = crate::tests::serial_ffi_world_test();
    let trace = Arc::new(Mutex::new(MeshTrace::default()));
    let mut runtime = runtime();
    runtime.set_renderer_backend(Box::new(TraceBackend {
        trace: Arc::clone(&trace),
    }));
    let handle = runtime
        .create_runtime_mesh("temp", triangle())
        .expect("create");
    let id = runtime.runtime_mesh_asset_id(handle).unwrap();
    runtime
        .load_scene(sample_scene_with_mesh(&id.id))
        .expect("scene loads");
    runtime.render_frame(0).expect("frame renders");

    runtime.destroy_runtime_mesh(handle).expect("destroy");
    assert!(
        trace.lock().unwrap().removals.is_empty(),
        "GPU destruction must not happen mid-frame"
    );

    // The renderable still references the destroyed mesh; extraction no
    // longer resolves it, so the frame fails asset sync — but registry
    // reconciliation removes the stale backend resource first.
    let _ = runtime.render_frame(1);
    {
        let trace = trace.lock().unwrap();
        assert_eq!(
            trace.removals,
            vec!["runtime-mesh-temp".to_string()],
            "registry reconciliation removes the resource exactly once"
        );
    }
    // Once the renderable points at the built-in cube again, rendering
    // recovers; no further removals are queued.
    runtime
        .load_scene(sample_scene_with_mesh("mesh-cube"))
        .expect("scene reloads");
    runtime.render_frame(2).expect("frame renders");
    assert_eq!(trace.lock().unwrap().removals.len(), 1);
}

#[test]
fn failed_registry_removal_is_reported_and_retried() {
    let _guard = crate::tests::serial_ffi_world_test();
    let trace = Arc::new(Mutex::new(MeshTrace::default()));
    let mut runtime = runtime();
    runtime.set_renderer_backend(Box::new(TraceBackend {
        trace: Arc::clone(&trace),
    }));
    let handle = runtime
        .create_runtime_mesh("retry", triangle())
        .expect("create");
    let id = runtime.runtime_mesh_asset_id(handle).unwrap();
    runtime
        .load_scene(sample_scene_with_mesh(&id.id))
        .expect("scene loads");
    runtime.render_frame(0).expect("frame renders");

    runtime.destroy_runtime_mesh(handle).expect("destroy");
    runtime
        .load_scene(sample_scene_with_mesh("mesh-cube"))
        .expect("scene reloads");
    trace.lock().unwrap().removal_failures_remaining = 1;

    let diagnostics = runtime.render_frame(1).expect_err("failure is reported");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "TEST_RESOURCE_REMOVAL_FAILED"));
    assert!(trace.lock().unwrap().removals.is_empty());

    runtime.render_frame(2).expect("next frame retries removal");
    assert_eq!(
        trace.lock().unwrap().removals,
        vec!["runtime-mesh-retry".to_string()]
    );
}

#[test]
fn runtime_diagnostics_reports_runtime_mesh_memory() {
    let mut runtime = runtime();
    assert_eq!(runtime.runtime_diagnostics().runtime_meshes.mesh_count, 0);
    let _handle = runtime.create_runtime_mesh("diag", quad()).expect("create");
    let snapshot = runtime.runtime_diagnostics();
    assert_eq!(snapshot.runtime_meshes.mesh_count, 1);
    assert_eq!(snapshot.runtime_meshes.vertex_bytes, 4 * 32);
    assert_eq!(snapshot.runtime_meshes.total_bytes(), 4 * 32 + 6 * 4);
}
