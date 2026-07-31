// ── Frame timing (ENG-04) ───────────────────────────────────────────

/// Backend that reports canned GPU pass timings, mimicking the async
/// read-back shape of the Vulkan backend.
struct GpuTimingBackend {
    gpu_enabled: std::sync::Arc<std::sync::Mutex<Option<bool>>>,
}

impl engine_renderer::BackendRenderer for GpuTimingBackend {
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
        _input: &engine_renderer::RenderFrameInput,
        _pass: &engine_renderer::render_graph2::PassNode,
        _frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn end_frame(&mut self, stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        stats.gpu_timing = engine_renderer::GpuTimingStatus::Available;
        // The samples belong to the frame recorded two frames ago.
        stats.gpu_pass_frame_index = Some(0);
        stats.gpu_pass_times = vec![
            engine_renderer::GpuPassTime {
                name: "directional_shadow_pass".to_string(),
                ms: 0.5,
            },
            engine_renderer::GpuPassTime {
                name: "opaque_pbr_forward_pass".to_string(),
                ms: 1.5,
            },
        ];
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn upload_mesh(
        &mut self,
        _upload: MeshUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_texture(
        &mut self,
        _upload: TextureUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        _upload: MaterialUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn set_gpu_timing_enabled(&mut self, enabled: bool) {
        *self
            .gpu_enabled
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(enabled);
    }
}

fn recording_backend() -> RecordingBackend {
    RecordingBackend {
        uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        rendered_ui_batch_counts: None,
    }
}

fn shared_gpu_flag() -> std::sync::Arc<std::sync::Mutex<Option<bool>>> {
    std::sync::Arc::new(std::sync::Mutex::new(None))
}

#[test]
fn render_frame_records_cpu_stage_timings() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(recording_backend()));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    for frame in 0..3 {
        runtime.render_frame(frame).expect("frame should render");
    }

    let timings = runtime.last_frame_timings().expect("frame timings");
    assert_eq!(timings.frame_index, 2);
    assert_eq!(
        timings.gpu_status,
        engine_renderer::GpuTimingStatus::Unavailable
    );
    assert!(timings.gpu_frame_index.is_none());
    for stage in ["extraction", "sync_render_assets", "render_submit"] {
        let pass = timings
            .passes
            .iter()
            .find(|pass| pass.name == stage)
            .unwrap_or_else(|| panic!("missing CPU stage '{stage}'"));
        assert!(pass.cpu_ms.is_some(), "stage '{stage}' needs cpu_ms");
        assert!(pass.gpu_ms.is_none());
    }
    let stage_sum: f32 = timings.passes.iter().filter_map(|pass| pass.cpu_ms).sum();
    assert!(
        (stage_sum - timings.total_cpu_ms).abs() < f32::EPSILON,
        "stage sum must equal total_cpu_ms"
    );

    let summary = runtime.frame_timing_summary();
    assert_eq!(summary.window_frames, 3);
    assert_eq!(
        summary.gpu_status,
        engine_renderer::GpuTimingStatus::Unavailable
    );
    let submit = summary
        .passes
        .iter()
        .find(|pass| pass.name == "render_submit")
        .expect("render_submit stats");
    let cpu = submit.cpu.expect("cpu aggregate");
    assert_eq!(cpu.samples, 3);
    assert!(cpu.avg_ms <= cpu.p95_ms && cpu.p95_ms <= cpu.max_ms);
    assert!(submit.gpu.is_none());
    assert!(summary.total_cpu.is_some());
    assert!(summary.total_gpu.is_none());
}

#[test]
fn backend_gpu_pass_times_flow_into_frame_timings() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(GpuTimingBackend {
        gpu_enabled: shared_gpu_flag(),
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    runtime.render_frame(2).expect("frame should render");

    let timings = runtime.last_frame_timings().expect("frame timings");
    assert_eq!(
        timings.gpu_status,
        engine_renderer::GpuTimingStatus::Available
    );
    assert_eq!(timings.gpu_frame_index, Some(0));
    assert_eq!(timings.total_gpu_ms, Some(2.0));
    let forward = timings
        .passes
        .iter()
        .find(|pass| pass.name == "opaque_pbr_forward_pass")
        .expect("GPU pass sample");
    assert_eq!(forward.gpu_ms, Some(1.5));
    assert_eq!(forward.cpu_ms, None);

    let summary = runtime.frame_timing_summary();
    assert_eq!(
        summary.gpu_status,
        engine_renderer::GpuTimingStatus::Available
    );
    let shadow = summary
        .passes
        .iter()
        .find(|pass| pass.name == "directional_shadow_pass")
        .expect("shadow stats");
    assert_eq!(shadow.gpu.unwrap().samples, 1);
    assert_eq!(summary.total_gpu.unwrap().samples, 1);

    let diagnostics = runtime.runtime_diagnostics();
    assert_eq!(
        diagnostics.frame_timing.gpu_status,
        engine_renderer::GpuTimingStatus::Available
    );
}

#[test]
fn gpu_timing_config_is_forwarded_to_the_backend() {
    let _guard = serial_ffi_world_test();
    let disabled_flag = shared_gpu_flag();
    let mut runtime = EngineRuntime::new(EngineConfig {
        application_name: "timing-test".to_string(),
        gpu_timestamps: false,
    });
    runtime.set_renderer_backend(Box::new(GpuTimingBackend {
        gpu_enabled: std::sync::Arc::clone(&disabled_flag),
    }));
    assert_eq!(
        *disabled_flag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        Some(false)
    );

    let enabled_flag = shared_gpu_flag();
    let mut enabled_runtime = EngineRuntime::new(EngineConfig::default());
    enabled_runtime.set_renderer_backend(Box::new(GpuTimingBackend {
        gpu_enabled: std::sync::Arc::clone(&enabled_flag),
    }));
    assert_eq!(
        *enabled_flag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        Some(true)
    );
}

#[test]
fn failed_render_discards_partial_frame_timing() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    // No backend installed: draw_scene fails after extraction/sync stages.
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");
    assert!(runtime.render_frame(0).is_err());
    assert!(runtime.last_frame_timings().is_none());
    assert_eq!(runtime.frame_timing_summary().window_frames, 0);

    // A subsequent healthy frame starts from a clean recorder.
    runtime.set_renderer_backend(Box::new(recording_backend()));
    runtime.render_frame(1).expect("frame should render");
    let timings = runtime.last_frame_timings().expect("frame timings");
    assert_eq!(timings.frame_index, 1);
    assert_eq!(
        timings
            .passes
            .iter()
            .filter(|pass| pass.cpu_ms.is_some())
            .count(),
        3
    );
}

#[test]
fn streamed_cooked_assets_install_additively_between_rendered_frames() {
    let _guard = serial_ffi_world_test();
    let dir = crate::cooked_assets::tests::cooked_case("mid_run_stream");
    crate::cooked_assets::tests::cook_test_material(&dir, "material.streamed", None);
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        rendered_ui_batch_counts: None,
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");
    runtime.render_frame(0).expect("frame before streaming");
    let streamed_id = AssetId::new("material.streamed");
    assert!(runtime
        .asset_registry()
        .get::<MaterialUpload>(&streamed_id)
        .is_none());

    // Mid-run: stream an extra cooked asset in the background and drain
    // at the frame boundary while frames keep rendering.
    runtime.enqueue_cooked_asset_stream(vec![dir.join("material.streamed.cooked")]);
    let mut installed_frame = None;
    for frame in 1..=600 {
        let report = runtime.drain_cooked_asset_stream();
        assert!(report.is_ok(), "diagnostics: {:?}", report.diagnostics);
        runtime.render_frame(frame).expect("frame during streaming");
        if runtime
            .asset_registry()
            .get::<MaterialUpload>(&streamed_id)
            .is_some()
        {
            installed_frame = Some(frame);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    assert!(installed_frame.is_some(), "streamed asset never installed");
    assert_eq!(runtime.cooked_asset_stream_pending(), 0);
    assert_eq!(runtime.asset_registry().pending_loads(), 0);
    // The runtime keeps rendering normally after the additive install.
    runtime.render_frame(601).expect("frame after streaming");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn runtime_builder_exposes_registry_before_build() {
    let mut builder = EngineRuntimeBuilder::default();
    register_a_only(builder.component_registry_mut());
    assert!(builder.component_registry().is_registered("test.a_only"));
}

#[test]
fn ffi_component_table_changes_only_when_a_runtime_activates() {
    let _guard = serial_ffi_world_test();
    let mut builder = EngineRuntimeBuilder::default();
    register_a_only(builder.component_registry_mut());
    let mut runtime_a = builder.build();
    runtime_a.set_world(World::new());

    let a_only = engine_ffi::component::lookup_component_type("A Only")
        .expect("A-only extension should be exposed while A is active");
    assert_eq!(
        engine_ffi::component::lookup_engine_type_id(a_only),
        Some("test.a_only")
    );
    let character = engine_ffi::component::lookup_component_type("Character Controller")
        .expect("character extension should be exposed to FFI");
    assert_eq!(
        engine_ffi::component::lookup_engine_type_id(character),
        Some("engine.character_controller")
    );

    // Merely constructing B must not mutate A's active type table.
    let mut runtime_b = EngineRuntime::new(EngineConfig::default());
    assert!(engine_ffi::component::lookup_component_type("A Only").is_some());

    // Activating B atomically replaces both the slot and type table.
    runtime_b.set_world(World::new());
    assert!(engine_ffi::component::lookup_component_type("A Only").is_none());
    assert!(engine_ffi::component::lookup_component_type("Character Controller").is_some());

    // Core metadata currently has no serialise/deserialise hooks, so it
    // must not be advertised as an FFI-readable component.
    assert!(engine_ffi::component::lookup_component_type("Transform").is_none());
}

#[test]
fn ffi_component_ids_are_stable_across_active_registry_order_and_membership() {
    let _guard = serial_ffi_world_test();
    let mut first_builder = EngineRuntimeBuilder::default();
    register_a_only(first_builder.component_registry_mut());
    register_b_only(first_builder.component_registry_mut());
    let mut first = first_builder.build();
    first.set_world(World::new());
    let a_id = engine_ffi::component::lookup_component_type("A Only").expect("A ID");
    let b_id = engine_ffi::component::lookup_component_type("B Only").expect("B ID");

    let mut reordered_builder = EngineRuntimeBuilder::default();
    register_b_only(reordered_builder.component_registry_mut());
    register_a_only(reordered_builder.component_registry_mut());
    let mut reordered = reordered_builder.build();
    reordered.set_world(World::new());
    assert_eq!(
        engine_ffi::component::lookup_component_type("A Only"),
        Some(a_id)
    );
    assert_eq!(
        engine_ffi::component::lookup_component_type("B Only"),
        Some(b_id)
    );

    let mut b_only_builder = EngineRuntimeBuilder::default();
    register_b_only(b_only_builder.component_registry_mut());
    let mut b_only = b_only_builder.build();
    b_only.set_world(World::new());
    assert!(engine_ffi::component::lookup_component_type("A Only").is_none());
    assert!(engine_ffi::component::lookup_engine_type_id(a_id).is_none());
    assert_eq!(
        engine_ffi::component::lookup_engine_type_id(b_id),
        Some("test.b_only")
    );
}

#[cfg(feature = "subsystem-physics")]
#[test]
fn gameplay_physics_extensions_are_exposed_to_ffi() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::new());
    let rigid_body = engine_ffi::component::lookup_component_type("RigidBody")
        .expect("physics extension should be exposed to FFI");
    assert_eq!(
        engine_ffi::component::lookup_engine_type_id(rigid_body),
        Some("engine.physics.rigid_body")
    );
}
