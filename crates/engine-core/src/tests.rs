//! Tests for the engine-core composition root.
//!
//! This module remains a child of the crate root, so private implementation
//! details stay testable without burying the public facade.

use super::*;

struct RecordingBackend {
    uploads: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    rendered_ui_batch_counts: Option<std::sync::Arc<std::sync::Mutex<Vec<usize>>>>,
}

struct CountingRenderExtension {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl engine_renderer::RenderExtensionProducer for CountingRenderExtension {
    fn name(&self) -> &str {
        "test-counting-extension"
    }

    fn produce(&self, _input: &mut engine_renderer::RenderFrameInput, _frame_index: u64) {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl engine_renderer::BackendRenderer for RecordingBackend {
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
        _pass: &engine_renderer::render_graph2::PassNode,
        _frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if let Some(counts) = &self.rendered_ui_batch_counts {
            counts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(input.ui_batches.len());
        }
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn upload_mesh(
        &mut self,
        upload: MeshUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        self.uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("mesh:{}", upload.mesh_id.id));
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_texture(
        &mut self,
        upload: TextureUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        self.uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("texture:{}", upload.texture_id.id));
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        upload: MaterialUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        self.uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("material:{}", upload.material_id.id));
        Ok(engine_renderer::UploadReceipt::new(1))
    }
}

static FFI_WORLD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn serial_ffi_world_test() -> std::sync::MutexGuard<'static, ()> {
    FFI_WORLD_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn insert_empty_component(scene: &mut Scene, type_id: &str) -> String {
    let entity = scene.entities.first_mut().expect("sample scene entity");
    entity.components.insert(
        type_id.to_string(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::new(),
        },
    );
    entity.persistent_id.clone()
}

struct AOnlyComponent;

struct BOnlyComponent;

impl engine_scene::Component for AOnlyComponent {
    const TYPE_ID: &'static str = "test.a_only";
}

impl engine_scene::Component for BOnlyComponent {
    const TYPE_ID: &'static str = "test.b_only";
}

fn a_only_storage() -> Box<dyn engine_scene::ComponentStorageDyn> {
    Box::new(engine_scene::SparseSet::<AOnlyComponent>::new())
}

fn b_only_storage() -> Box<dyn engine_scene::ComponentStorageDyn> {
    Box::new(engine_scene::SparseSet::<BOnlyComponent>::new())
}

fn serialize_a_only(
    _component: &dyn std::any::Any,
) -> std::collections::BTreeMap<String, engine_serialize::Value> {
    std::collections::BTreeMap::new()
}

fn deserialize_a_only(
    _fields: &std::collections::BTreeMap<String, engine_serialize::Value>,
) -> Box<dyn std::any::Any> {
    Box::new(AOnlyComponent)
}

fn deserialize_b_only(
    _fields: &std::collections::BTreeMap<String, engine_serialize::Value>,
) -> Box<dyn std::any::Any> {
    Box::new(BOnlyComponent)
}

fn register_a_only(registry: &mut ComponentRegistry) {
    registry
        .register(engine_scene::ComponentExtension {
            meta: engine_scene::ComponentMeta {
                type_id: <AOnlyComponent as engine_scene::Component>::TYPE_ID,
                display_name: "A Only",
                schema_version: (0, 1, 0),
                has_editor: false,
                script_access: engine_scene::ScriptAccess::ReadWrite,
            },
            storage_factory: a_only_storage,
            serialize: Some(serialize_a_only),
            deserialize: Some(deserialize_a_only),
        })
        .expect("register A-only test extension");
}

fn register_b_only(registry: &mut ComponentRegistry) {
    registry
        .register(engine_scene::ComponentExtension {
            meta: engine_scene::ComponentMeta {
                type_id: <BOnlyComponent as engine_scene::Component>::TYPE_ID,
                display_name: "B Only",
                schema_version: (0, 1, 0),
                has_editor: false,
                script_access: engine_scene::ScriptAccess::ReadWrite,
            },
            storage_factory: b_only_storage,
            serialize: Some(serialize_a_only),
            deserialize: Some(deserialize_b_only),
        })
        .expect("register B-only test extension");
}

// ── EngineConfig tests ───────────────────────────────────────────────

#[test]
fn engine_config_defaults() {
    let config = EngineConfig::default();
    assert_eq!(config.application_name, "engine");
}

#[test]
fn engine_config_debug() {
    let config = EngineConfig::default();
    let debug = format!("{:?}", config);
    assert!(debug.contains("EngineConfig"));
}

#[test]
fn engine_config_partial_eq() {
    let a = EngineConfig::default();
    let b = EngineConfig::default();
    let c = EngineConfig {
        application_name: "custom".to_string(),
        gpu_timestamps: true,
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn engine_config_clone() {
    let config = EngineConfig::default();
    let cloned = config.clone();
    assert_eq!(config, cloned);
}

// ── EngineRuntime tests ──────────────────────────────────────────────

#[test]
fn engine_runtime_creation() {
    let config = EngineConfig::default();
    let runtime = EngineRuntime::new(config.clone());
    assert_eq!(*runtime.config(), config);
}

#[test]
fn runtime_builder_registers_character_extensions_by_default() {
    let builder = EngineRuntimeBuilder::default();
    assert!(builder
        .component_registry()
        .is_registered("engine.character_controller"));
}

#[test]
fn runtime_builder_registers_vfx_extensions_by_default() {
    let builder = EngineRuntimeBuilder::default();
    assert!(builder
        .component_registry()
        .is_registered("engine.vfx.particle_emitter"));
    assert!(builder
        .component_registry()
        .is_registered("engine.vfx.decal"));
}

#[test]
fn runtime_extracts_vfx_and_syncs_builtin_surface_assets() {
    let _guard = serial_ffi_world_test();
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::clone(&uploads),
        rendered_ui_batch_counts: None,
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene loads");
    runtime
        .with_world_mut(|world| {
            let cube = world.entity_by_persistent_id("cube-01").unwrap();
            world.add_component(cube, engine_scene::components::Transform::default());
            world.add_component(cube, engine_vfx::Decal::default());
        })
        .unwrap();

    runtime.render_frame(0).expect("VFX frame renders");

    let uploads = uploads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(uploads.iter().any(|entry| entry == "mesh:mesh-vfx-quad"));
    assert!(uploads
        .iter()
        .any(|entry| entry == "material:mat-vfx-default"));
}

#[cfg(feature = "subsystem-physics")]
#[test]
fn runtime_builder_registers_physics_extensions_with_physics_leaf() {
    let builder = EngineRuntimeBuilder::default();
    assert!(builder
        .component_registry()
        .is_registered("engine.physics.rigid_body"));
    assert!(builder
        .component_registry()
        .is_registered("engine.physics.collider"));
    assert!(builder
        .component_registry()
        .is_registered("engine.physics.physics_material"));
}

#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
))]
#[test]
fn runtime_builder_registers_runtime_subsystem_extensions() {
    let builder = EngineRuntimeBuilder::default();
    for component in [
        "engine.canvas",
        "engine.audio_source",
        "engine.audio_listener",
        "engine.animation_player",
        "engine.skeleton",
        "engine.ik_target",
        "engine.nav_agent",
    ] {
        assert!(
            builder.component_registry().is_registered(component),
            "missing component extension {component}"
        );
    }
    for asset_type in [
        "audio_clip",
        "skeleton",
        "animation_clip",
        "navmesh",
        "behavior",
    ] {
        assert!(
            builder.asset_type_registry().get(asset_type).is_some(),
            "missing asset type extension {asset_type}"
        );
    }
    assert_eq!(builder.render_extension_registry().producer_count(), 1);
    assert!(builder.debug_draw_registry().provider_count() >= 3);
    assert_eq!(
        builder
            .animation_extension_handles()
            .skinned_extract
            .pending_count(),
        0
    );
}

#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
))]
#[test]
fn runtime_subsystem_components_survive_strict_scene_loading() {
    let _guard = serial_ffi_world_test();
    let mut scene = engine_scene::sample_scene();
    for component in [
        "engine.canvas",
        "engine.audio_source",
        "engine.audio_listener",
        "engine.animation_player",
        "engine.skeleton",
        "engine.ik_target",
        "engine.nav_agent",
    ] {
        insert_empty_component(&mut scene, component);
    }

    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime
        .load_scene(scene)
        .expect("registered runtime subsystem components should load strictly");

    runtime
        .with_world(|world| {
            assert_eq!(world.query::<engine_ui::Canvas>().count(), 1);
            assert_eq!(
                world
                    .query::<engine_audio::components::AudioSourceComponent>()
                    .count(),
                1
            );
            assert_eq!(
                world.query::<engine_animation::AnimationPlayer>().count(),
                1
            );
            assert_eq!(world.query::<engine_nav::AiAgent>().count(), 1);
        })
        .expect("strict load should install a World");
}

#[cfg(not(any(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
)))]
#[test]
fn minimal_runtime_does_not_install_optional_subsystems() {
    let builder = EngineRuntimeBuilder::default();
    assert!(!builder.component_registry().is_registered("engine.canvas"));
    assert!(!builder
        .component_registry()
        .is_registered("engine.audio_source"));
    assert!(!builder
        .component_registry()
        .is_registered("engine.animation_player"));
    assert!(!builder
        .component_registry()
        .is_registered("engine.nav_agent"));
    assert!(builder.asset_type_registry().get("audio_clip").is_none());
    assert_eq!(builder.render_extension_registry().producer_count(), 0);
}

#[test]
fn runtime_invokes_registered_render_extensions_before_drawing() {
    let _guard = serial_ffi_world_test();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut builder = EngineRuntimeBuilder::default();
    builder
        .render_extension_registry_mut()
        .register(Box::new(CountingRenderExtension {
            calls: std::sync::Arc::clone(&calls),
        }));
    let mut runtime = builder.build();
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        rendered_ui_batch_counts: None,
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    runtime.render_frame(17).expect("frame should render");

    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
}

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

#[test]
fn engine_runtime_config_accessor() {
    let config = EngineConfig::default();
    let runtime = EngineRuntime::new(config);
    let retrieved = runtime.config();
    assert_eq!(retrieved.application_name, "engine");
}

#[test]
fn engine_runtime_render_frame_without_scene_fails() {
    let config = EngineConfig::default();
    let mut runtime = EngineRuntime::new(config);
    let result = runtime.render_frame(0);
    assert!(result.is_err());
}

#[test]
fn runtime_submits_host_ui_batches_with_the_scene() {
    let _guard = serial_ffi_world_test();
    let ui_counts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        rendered_ui_batch_counts: Some(std::sync::Arc::clone(&ui_counts)),
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    let batch = engine_renderer::UiBatch {
        canvas_id: "editor".into(),
        z_order: 0,
        clip_rect: engine_renderer::Rect {
            min: [0.0, 0.0],
            max: [800.0, 600.0],
        },
        texture: None,
        vertices: vec![
            engine_renderer::UiVertex {
                position: [0.0, 0.0],
                uv: [0.0, 0.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [10.0, 0.0],
                uv: [1.0, 0.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [10.0, 10.0],
                uv: [1.0, 1.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [0.0, 10.0],
                uv: [0.0, 1.0],
                color: [255; 4],
            },
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        material: AssetId::new("ui/default"),
    };

    runtime
        .render_frame_with_ui(7, vec![batch])
        .expect("scene and host UI should render together");
    let ui_counts = ui_counts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(!ui_counts.is_empty());
    assert!(ui_counts.iter().all(|count| *count == 1));
}

#[cfg(feature = "subsystem-ui")]
#[test]
fn runtime_refreshes_generated_font_atlas_after_ui_batch_build() {
    if engine_ui::font_atlas_texture_upload().is_none() {
        return;
    }
    let _guard = serial_ffi_world_test();
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::clone(&uploads),
        rendered_ui_batch_counts: None,
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
    canvas.add_element(engine_ui::UiElement::new(
        engine_ui::UiElementKind::Text {
            content: "Editor text".into(),
            font_size: 18.0,
            color: engine_ui::Color::WHITE,
        },
        engine_ui::Layout::FILL,
    ));
    canvas.layout_all();
    let batches = canvas.build_batches();
    assert!(batches.iter().any(|batch| {
        batch
            .texture
            .as_ref()
            .is_some_and(|texture| texture.id == engine_ui::FONT_ATLAS_ASSET)
    }));

    runtime
        .render_frame_with_ui(0, batches)
        .expect("generated font atlas should be registered before rendering");

    let texture_id = AssetId::new(engine_ui::FONT_ATLAS_ASSET);
    let atlas = runtime
        .asset_registry()
        .get::<TextureUpload>(&texture_id)
        .expect("font atlas must be owned by AssetRegistry");
    assert!(atlas.get().mip_levels[0]
        .bytes
        .chunks_exact(4)
        .any(|pixel| pixel[3] != 0));
    assert!(uploads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|upload| upload == "texture:engine/font-atlas"));
}

#[cfg(feature = "subsystem-ui")]
#[test]
fn game_loop_submits_retained_scene_canvas_batches_automatically() {
    let _guard = serial_ffi_world_test();
    let ui_counts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = game_loop::GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .set_renderer_backend(Box::new(RecordingBackend {
            uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            rendered_ui_batch_counts: Some(std::sync::Arc::clone(&ui_counts)),
        }));
    game_loop
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");
    game_loop
        .runtime
        .with_world_mut(|world| {
            let entity = world.entity_by_persistent_id("camera-main").unwrap();
            let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
            canvas.add_element(engine_ui::UiElement::new(
                engine_ui::UiElementKind::Panel {
                    color: engine_ui::Color::new(40, 80, 120, 255),
                },
                engine_ui::Layout::FILL,
            ));
            world.add_component(entity, canvas);
        })
        .expect("runtime world should be available");

    game_loop
        .render(7)
        .expect("retained scene Canvas should render automatically");

    let ui_counts = ui_counts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(!ui_counts.is_empty());
    assert!(ui_counts.iter().all(|count| *count == 1));
}

#[test]
fn runtime_uploads_and_deduplicates_ui_only_textures() {
    let _guard = serial_ffi_world_test();
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::clone(&uploads),
        rendered_ui_batch_counts: None,
    }));

    let texture_id = AssetId::new("texture-ui-atlas");
    runtime.register_texture_asset(TextureUpload {
        texture_id: texture_id.clone(),
        width: 1,
        height: 1,
        format: engine_renderer::TextureUploadFormat::Rgba8,
        color_space: engine_renderer::ColorSpace::Srgb,
        mip_levels: vec![engine_renderer::TextureMipLevel {
            width: 1,
            height: 1,
            bytes: vec![255, 255, 255, 255],
        }],
        sampler: engine_renderer::SamplerDescriptor::default(),
        content_hash: [9; 32],
    });
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");
    let batch = engine_renderer::UiBatch {
        canvas_id: "hud".into(),
        z_order: 0,
        clip_rect: engine_renderer::Rect {
            min: [0.0, 0.0],
            max: [128.0, 128.0],
        },
        texture: Some(texture_id),
        vertices: vec![
            engine_renderer::UiVertex {
                position: [0.0, 0.0],
                uv: [0.0, 0.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [1.0, 0.0],
                uv: [1.0, 0.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [0.0, 1.0],
                uv: [0.0, 1.0],
                color: [255; 4],
            },
        ],
        indices: vec![0, 1, 2],
        material: AssetId::new("ui/default"),
    };

    runtime
        .render_frame_with_ui(8, vec![batch.clone(), batch])
        .expect("UI texture should be synchronised before rendering");

    let uploads = uploads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(
        uploads
            .iter()
            .filter(|upload| upload.as_str() == "texture:texture-ui-atlas")
            .count(),
        1
    );
}

#[test]
fn runtime_uploads_registered_scene_resources_in_dependency_order() {
    let _guard = serial_ffi_world_test();
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::clone(&uploads),
        rendered_ui_batch_counts: None,
    }));

    let texture_id = AssetId::new("texture-auto");
    runtime.register_texture_asset(TextureUpload {
        texture_id: texture_id.clone(),
        width: 1,
        height: 1,
        format: engine_renderer::TextureUploadFormat::Rgba8,
        color_space: engine_renderer::ColorSpace::Srgb,
        mip_levels: vec![engine_renderer::TextureMipLevel {
            width: 1,
            height: 1,
            bytes: vec![255, 255, 255, 255],
        }],
        sampler: engine_renderer::SamplerDescriptor::default(),
        content_hash: [1; 32],
    });
    let material_id = AssetId::new("material-auto");
    runtime.register_material_asset(MaterialUpload {
        material_id: material_id.clone(),
        base_color: [1.0; 4],
        metallic: 0.0,
        roughness: 1.0,
        ambient_occlusion: 1.0,
        emissive: [0.0; 3],
        base_color_texture: Some(texture_id),
        normal_texture: None,
        metallic_roughness_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        advanced: engine_renderer::AdvancedMaterialParameters::default(),
        transparency: engine_renderer::Transparency::Opaque,
        double_sided: false,
        content_hash: [2; 32],
    });

    let mut scene = engine_scene::sample_scene();
    let renderable = scene
        .entities
        .iter_mut()
        .find_map(|entity| entity.components.get_mut("engine.renderable"))
        .expect("sample renderable");
    renderable.fields.insert(
        "material".to_string(),
        engine_serialize::Value::Asset(material_id),
    );
    runtime.load_scene(scene).expect("scene load");

    runtime.render_frame(0).expect("render");

    assert_eq!(
        *uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![
            "texture:texture-auto".to_string(),
            "material:material-auto".to_string(),
            "mesh:mesh-cube".to_string(),
        ]
    );
}

#[test]
fn engine_runtime_diagnostics_collector() {
    let config = EngineConfig::default();
    let runtime = EngineRuntime::new(config);
    let collector = runtime.diagnostics_collector();
    assert!(collector.all().is_empty());
}

#[test]
fn engine_runtime_runtime_diagnostics() {
    let config = EngineConfig::default();
    let runtime = EngineRuntime::new(config);
    let rd = runtime.runtime_diagnostics();
    assert!(
        rd.script_engine_state.contains("coroutines=0"),
        "missing coroutines=0"
    );
    assert!(rd.reload_queue.is_none());
}

#[test]
fn strict_scene_load_installs_the_runtime_registry() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let runtime_registry = std::sync::Arc::clone(runtime.component_registry());

    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    assert_eq!(
        runtime.with_world(|world| {
            std::sync::Arc::ptr_eq(
                world.component_registry().expect("world registry"),
                &runtime_registry,
            )
        }),
        Some(true)
    );
}

#[test]
fn unknown_component_failure_keeps_active_world_and_scene() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut old_world = World::new();
    old_world.create_entity();
    runtime.set_world(old_world);
    let old_scene = runtime.scene_ref().cloned().expect("old scene snapshot");

    let mut invalid_scene = engine_scene::sample_scene();
    let entity_id = insert_empty_component(&mut invalid_scene, "third.party.missing");
    let diagnostics = runtime
        .load_scene(invalid_scene)
        .expect_err("unknown component must fail strict loading");

    assert_eq!(runtime.with_world(World::alive_count), Some(1));
    assert_eq!(runtime.scene_ref(), Some(&old_scene));
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "SC0030")
        .expect("mapped unknown-component diagnostic");
    assert_eq!(diagnostic.entity.as_deref(), Some(entity_id.as_str()));
    assert_eq!(
        diagnostic
            .fields
            .get("component_type_id")
            .map(String::as_str),
        Some("third.party.missing")
    );
    assert_eq!(
        diagnostic.path.as_deref(),
        Some(format!("entities[{entity_id}].components[third.party.missing]").as_str())
    );

    // The process-wide FFI bridge must still target the previous World.
    let spawned = engine_ffi::world_bridge::entity_spawn();
    assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
    assert_eq!(runtime.with_world(World::alive_count), Some(2));
}

#[test]
fn validation_failures_keep_active_world_and_scene() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut old_world = World::new();
    old_world.create_entity();
    runtime.set_world(old_world);
    let old_scene = runtime.scene_ref().cloned().expect("old scene snapshot");

    let mut duplicate = engine_scene::sample_scene();
    duplicate.entities.push(duplicate.entities[0].clone());
    let duplicate_diagnostics = runtime
        .load_scene(duplicate)
        .expect_err("duplicate entity must fail validation");
    assert!(duplicate_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SC0015"));
    assert_eq!(runtime.with_world(World::alive_count), Some(1));
    assert_eq!(runtime.scene_ref(), Some(&old_scene));

    let mut missing_parent = engine_scene::sample_scene();
    let mut orphan = missing_parent.entities[0].clone();
    orphan.persistent_id = "orphan".to_string();
    orphan.parent = Some("missing-parent".to_string());
    missing_parent.entities.push(orphan);
    let parent_diagnostics = runtime
        .load_scene(missing_parent)
        .expect_err("missing parent must fail validation");
    assert!(parent_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SC0016"));
    assert_eq!(runtime.with_world(World::alive_count), Some(1));
    assert_eq!(runtime.scene_ref(), Some(&old_scene));
}

#[test]
fn set_world_installs_runtime_registry_when_missing() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let runtime_registry = std::sync::Arc::clone(runtime.component_registry());

    runtime.set_world(World::new());

    assert_eq!(
        runtime.with_world(|world| {
            std::sync::Arc::ptr_eq(
                world.component_registry().expect("world registry"),
                &runtime_registry,
            )
        }),
        Some(true)
    );
}

#[test]
fn set_world_preserves_an_existing_registry() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut custom_registry = ComponentRegistry::new();
    register_a_only(&mut custom_registry);
    let custom_registry = std::sync::Arc::new(custom_registry);
    let mut world = World::new();
    world.set_shared_component_registry(std::sync::Arc::clone(&custom_registry));

    runtime.set_world(world);

    assert_eq!(
        runtime.with_world(|world| {
            std::sync::Arc::ptr_eq(
                world.component_registry().expect("world registry"),
                &custom_registry,
            )
        }),
        Some(true)
    );
    assert!(engine_ffi::component::lookup_component_type("A Only").is_some());
    assert!(engine_ffi::component::lookup_component_type("Character Controller").is_none());
}

#[test]
fn engine_runtime_can_replace_the_active_world_repeatedly() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());

    let mut first = World::new();
    first.create_entity();
    runtime.set_world(first);

    runtime.set_world(World::new());
    let spawned = engine_ffi::world_bridge::entity_spawn();
    assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
    assert_eq!(runtime.with_world(World::alive_count), Some(1));
}

#[test]
fn moving_runtime_keeps_ffi_world_binding_valid() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::new());

    let mut runtimes = vec![runtime];
    let moved_runtime = runtimes.pop().expect("moved runtime");

    let spawned = engine_ffi::world_bridge::entity_spawn();
    assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
    assert_eq!(moved_runtime.with_world(World::alive_count), Some(1));
}

#[test]
fn dropping_runtime_makes_its_ffi_world_unavailable() {
    let _guard = serial_ffi_world_test();
    {
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::new());
    }

    assert_eq!(
        engine_ffi::world_bridge::entity_spawn(),
        engine_ffi::types::FfiEntityId::INVALID
    );
}

#[test]
fn dropping_old_runtime_does_not_deactivate_new_runtime() {
    let _guard = serial_ffi_world_test();
    let mut old_runtime = EngineRuntime::new(EngineConfig::default());
    old_runtime.set_world(World::new());
    let mut current_runtime = EngineRuntime::new(EngineConfig::default());
    current_runtime.set_world(World::new());

    drop(old_runtime);
    let spawned = engine_ffi::world_bridge::entity_spawn();
    assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
    assert_eq!(current_runtime.with_world(World::alive_count), Some(1));
}

#[test]
fn canonical_scene_load_replaces_and_activates_the_world() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::new());
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load into a World");

    assert!(runtime.has_world());
    assert_ne!(
        engine_ffi::world_bridge::entity_spawn(),
        engine_ffi::types::FfiEntityId::INVALID
    );
}

// ── Script subsystem tests ──────────────────────────────────────────

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn in_process_csharp_bridge_installs_the_native_cdylib() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::new());

    runtime
        .install_in_process_csharp_ffi()
        .expect("matching engine_ffi cdylib should install");

    let path =
        engine_ffi::host_bridge::loaded_cdylib_path().expect("installed native library path");
    assert!(path.exists());
    assert_eq!(
        std::env::var("ENGINE_FFI_HOST_PID").ok(),
        Some(std::process::id().to_string())
    );
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn engine_runtime_script_host_registration() {
    use engine_script::MockHost;

    let config = EngineConfig::default();
    let mut runtime = EngineRuntime::new(config);

    assert_eq!(runtime.script_engine.host_count(), 0);
    runtime.register_script_host(Box::new(MockHost::new()));
    assert_eq!(runtime.script_engine.host_count(), 1);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn engine_runtime_exposes_only_host_verified_script_classes() {
    use engine_script::MockHost;

    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.register_script_host(Box::new(
        MockHost::new().with_verified_classes("game", ["Game.Player"]),
    ));
    runtime
        .load_script_assembly("game", "mock", b"managed")
        .unwrap();

    let classes = runtime.verified_script_classes();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].assembly_id, "game");
    assert_eq!(classes[0].class_name, "Game.Player");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_engine_replacement_is_atomic_and_does_not_accumulate_hosts() {
    use engine_script::{MockHost, ScriptEngine};

    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.register_script_host(Box::new(MockHost::new()));
    runtime
        .load_script_assembly("old", "mock", b"old")
        .expect("old runtime assembly");

    let invalid_candidate = ScriptEngine::new();
    let error = runtime
        .replace_script_engine(invalid_candidate, "mock")
        .expect_err("candidate without the selected host must be rejected");
    assert!(error.to_string().contains("exactly one host"));
    assert_eq!(runtime.script_engine().host_count(), 1);
    assert_eq!(runtime.script_engine().managers()[0].assembly_count(), 1);

    let mut duplicate_candidate = ScriptEngine::new();
    duplicate_candidate.register_host(Box::new(MockHost::new()));
    duplicate_candidate.register_host(Box::new(MockHost::new()));
    runtime
        .replace_script_engine(duplicate_candidate, "mock")
        .expect_err("duplicate selected hosts must be rejected");
    assert_eq!(runtime.script_engine().host_count(), 1);
    assert_eq!(runtime.script_engine().managers()[0].assembly_count(), 1);

    let mut candidate = ScriptEngine::new();
    candidate.register_host(Box::new(MockHost::new()));
    candidate
        .load_script("new-dependency", "mock", b"dependency")
        .expect("candidate dependency");
    candidate
        .load_script("new-game", "mock", b"game")
        .expect("candidate game assembly");

    runtime
        .replace_script_engine(candidate, "mock")
        .expect("valid candidate should replace the runtime");
    assert_eq!(runtime.script_engine().host_count(), 1);
    assert_eq!(runtime.script_engine().managers()[0].assembly_count(), 2);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn engine_runtime_tick_scripts_no_panic() {
    let config = EngineConfig::default();
    let mut runtime = EngineRuntime::new(config);

    // Tick with no hosts registered — should not panic
    runtime.tick_scripts(0.016);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_create_entity_is_transactional_first_wins_and_enters_next_snapshot() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    let first_transform = ScriptTransform {
        translation: [7.0, 8.0, 9.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [2.0, 3.0, 4.0],
    };
    let commands = vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "spawned-01".into(),
                transform: first_transform.clone(),
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "spawned-01".into(),
                transform: ScriptTransform {
                    translation: [100.0, 100.0, 100.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);

    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SCRIPT_ENTITY_CREATE_CONFLICT")
            .count(),
        1
    );
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 3);
            let entity = world
                .entity_by_persistent_id("spawned-01")
                .expect("first creation must persist");
            let transform = world
                .get::<engine_scene::components::Transform>(entity)
                .expect("created entity must have Transform");
            assert_eq!(
                transform.translation.to_array(),
                first_transform.translation
            );
            assert_eq!(transform.rotation.to_array(), first_transform.rotation);
            assert_eq!(transform.scale.to_array(), first_transform.scale);
        })
        .expect("runtime must keep an active World");
    let snapshots = runtime.script_gameplay_entity_snapshots();
    assert_eq!(
        snapshots["spawned-01"].transform,
        Some(first_transform),
        "the next script context must include the newly-created entity"
    );
}

#[cfg(all(
    feature = "subsystem-scripting-csharp",
    feature = "subsystem-animation"
))]
#[test]
fn script_animation_command_uses_dedicated_player_mutation() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    runtime
        .with_world_mut(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world.add_component(entity, engine_animation::AnimationPlayer::new());
        })
        .unwrap();

    let diagnostics =
        runtime.apply_script_gameplay_commands(vec![engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::PlayAnimation {
                entity_id: "cube-01".into(),
                clip_asset: "battle.attack".into(),
                looping: false,
                speed: 1.5,
                restart: true,
            },
        }]);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    runtime
        .with_world(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            let player = world
                .get::<engine_animation::AnimationPlayer>(entity)
                .unwrap();
            assert_eq!(player.clip_asset.as_deref(), Some("battle.attack"));
            assert!(player.playing);
            assert!(!player.looping);
            assert_eq!(player.speed, 1.5);
        })
        .unwrap();
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_create_entity_validation_and_missing_owner_never_partially_create() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    let valid_transform = ScriptTransform {
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };
    let commands = vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "../invalid".into(),
                transform: valid_transform.clone(),
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "invalid-transform".into(),
                transform: ScriptTransform {
                    rotation: [0.0; 4],
                    ..valid_transform.clone()
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "missing-owner".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "orphan".into(),
                transform: valid_transform,
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_ENTITY_CREATE_ID_INVALID"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_ENTITY_CREATE_TRANSFORM_INVALID"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_COMMAND_OWNER_MISSING"));
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 2);
            assert!(world.entity_by_persistent_id("invalid-transform").is_none());
            assert!(world.entity_by_persistent_id("orphan").is_none());
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_create_entity_rechecks_owner_after_prior_same_frame_destroy() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    let commands = vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::DestroySelf,
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "after-destroy".into(),
                transform: ScriptTransform {
                    translation: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_COMMAND_OWNER_MISSING"));
    runtime
        .with_world(|world| {
            assert!(world.entity_by_persistent_id("cube-01").is_none());
            assert!(world.entity_by_persistent_id("after-destroy").is_none());
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn script_spawn_test_transform_record(translation: [f32; 3]) -> engine_scene::ComponentRecord {
    engine_scene::ComponentRecord {
        schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields: std::collections::BTreeMap::from([
            (
                "translation".to_string(),
                engine_serialize::Value::Vec3(translation),
            ),
            (
                "rotation".to_string(),
                engine_serialize::Value::Quat([
                    0.0,
                    0.0,
                    std::f32::consts::FRAC_1_SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2,
                ]),
            ),
            (
                "scale".to_string(),
                engine_serialize::Value::Vec3([2.0, 2.0, 2.0]),
            ),
        ]),
    }
}

/// Two-entity prefab: root `root` with a rotated/scaled Transform and a
/// child `bolt`, so tests can assert deterministic id assignment,
/// hierarchy parenting, and translation overrides.
#[cfg(feature = "subsystem-scripting-csharp")]
fn script_spawn_test_prefab(prefab_id: &str) -> engine_scene::Prefab {
    let mut prefab = engine_scene::Prefab::new(AssetId::new(prefab_id));
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "root".to_string(),
        parent: None,
        name: Some("Root".to_string()),
        enabled: true,
        components: std::collections::BTreeMap::from([(
            "engine.transform".to_string(),
            script_spawn_test_transform_record([1.0, 2.0, 3.0]),
        )]),
    });
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "bolt".to_string(),
        parent: Some("root".to_string()),
        name: Some("Bolt".to_string()),
        enabled: true,
        components: std::collections::BTreeMap::from([(
            "engine.transform".to_string(),
            script_spawn_test_transform_record([0.0, 1.0, 0.0]),
        )]),
    });
    prefab
}

/// Install a cooked prefab into the runtime exactly like the cooked-batch
/// loader does: typed payload in the asset registry plus the extension
/// type-id registration that `extension_asset::<Prefab>("prefab", ..)`
/// consults.
#[cfg(feature = "subsystem-scripting-csharp")]
fn register_script_prefab(
    runtime: &mut EngineRuntime,
    prefab_id: &str,
    prefab: engine_scene::Prefab,
) {
    let asset_id = AssetId::new(prefab_id);
    runtime
        .asset_registry_mut()
        .insert_typed(asset_id.clone(), prefab);
    runtime
        .loaded_extension_asset_ids
        .entry("prefab".to_string())
        .or_default()
        .insert(asset_id);
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn spawn_prefab_command(
    owner: &str,
    prefab_id: &str,
    translation: Option<[f32; 3]>,
) -> engine_script::OwnedGameplayCommand {
    engine_script::OwnedGameplayCommand {
        entity_id: owner.to_string(),
        command: GameplayCommand::SpawnPrefab {
            prefab_id: prefab_id.to_string(),
            translation,
        },
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_assigns_deterministic_ids_and_enters_next_snapshot() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    register_script_prefab(
        &mut runtime,
        "prefab-x",
        script_spawn_test_prefab("prefab-x"),
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![
        spawn_prefab_command("cube-01", "prefab-x", None),
        spawn_prefab_command("cube-01", "prefab-x", None),
    ]);

    assert!(
        diagnostics.is_empty(),
        "unexpected spawn diagnostics: {diagnostics:?}"
    );
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 6);
            for id in ["prefab-x", "prefab-x.bolt", "prefab-x-2", "prefab-x-2.bolt"] {
                assert!(
                    world.entity_by_persistent_id(id).is_some(),
                    "missing spawned entity '{id}'"
                );
            }
            let root = world
                .entity_by_persistent_id("prefab-x")
                .expect("first spawn keeps the bare prefab id");
            let root_transform = world
                .get::<engine_scene::components::Transform>(root)
                .expect("spawned root must keep its Transform");
            assert_eq!(root_transform.translation.to_array(), [1.0, 2.0, 3.0]);
            let child = world
                .entity_by_persistent_id("prefab-x.bolt")
                .expect("child id derives from the prefab-local id");
            let child_transform = world
                .get::<engine_scene::components::Transform>(child)
                .expect("spawned child must keep its Transform");
            assert_eq!(child_transform.parent, Some(root));
        })
        .expect("runtime must keep an active World");
    let snapshots = runtime.script_gameplay_entity_snapshots();
    for id in ["prefab-x", "prefab-x.bolt", "prefab-x-2", "prefab-x-2.bolt"] {
        assert!(
            snapshots.contains_key(id),
            "next script context must include spawned entity '{id}'"
        );
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_unknown_id_reports_actionable_diagnostic() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    register_script_prefab(
        &mut runtime,
        "prefab-x",
        script_spawn_test_prefab("prefab-x"),
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
        "cube-01",
        "prefab-missing",
        None,
    )]);

    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "SCRIPT_PREFAB_UNKNOWN");
    assert_eq!(diagnostic.entity.as_deref(), Some("cube-01"));
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 2);
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_invalid_requests_never_partially_spawn() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    register_script_prefab(
        &mut runtime,
        "prefab-x",
        script_spawn_test_prefab("prefab-x"),
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![
        spawn_prefab_command("cube-01", "../invalid", None),
        spawn_prefab_command("cube-01", "prefab-x", Some([f32::NAN, 0.0, 0.0])),
        spawn_prefab_command("missing-owner", "prefab-x", None),
    ]);

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_PREFAB_ID_INVALID"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_PREFAB_TRANSFORM_INVALID"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_COMMAND_OWNER_MISSING"));
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 2);
            assert!(world.entity_by_persistent_id("prefab-x").is_none());
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_translation_override_preserves_prefab_rotation_and_scale() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    register_script_prefab(
        &mut runtime,
        "prefab-x",
        script_spawn_test_prefab("prefab-x"),
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
        "cube-01",
        "prefab-x",
        Some([7.0, 8.0, 9.0]),
    )]);

    assert!(
        diagnostics.is_empty(),
        "unexpected spawn diagnostics: {diagnostics:?}"
    );
    runtime
        .with_world(|world| {
            let root = world
                .entity_by_persistent_id("prefab-x")
                .expect("spawned root must exist");
            let transform = world
                .get::<engine_scene::components::Transform>(root)
                .expect("spawned root must keep its Transform");
            assert_eq!(transform.translation.to_array(), [7.0, 8.0, 9.0]);
            assert_eq!(
                transform.rotation.to_array(),
                [
                    0.0,
                    0.0,
                    std::f32::consts::FRAC_1_SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2
                ],
                "the override must not reset the prefab rotation"
            );
            assert_eq!(
                transform.scale.to_array(),
                [2.0, 2.0, 2.0],
                "the override must not reset the prefab scale"
            );
            let child = world
                .entity_by_persistent_id("prefab-x.bolt")
                .expect("spawned child must exist");
            let child_transform = world
                .get::<engine_scene::components::Transform>(child)
                .expect("spawned child must keep its Transform");
            assert_eq!(
                child_transform.translation.to_array(),
                [0.0, 1.0, 0.0],
                "the override only applies to the root"
            );
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_attaches_scene_only_scripts_and_creates_instances() {
    let _guard = serial_ffi_world_test();
    use engine_script::MockHost;

    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.register_script_host(Box::new(MockHost::new()));
    runtime.set_script_host_name("mock");
    runtime
        .load_script_assembly("game", "mock", b"managed")
        .expect("mock assembly should load");
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));

    let mut prefab = engine_scene::Prefab::new(AssetId::new("prefab-scripted"));
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "root".to_string(),
        parent: None,
        name: Some("Root".to_string()),
        enabled: true,
        components: std::collections::BTreeMap::from([
            (
                "engine.transform".to_string(),
                script_spawn_test_transform_record([0.0; 3]),
            ),
            (
                "engine.script".to_string(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: std::collections::BTreeMap::from([
                        (
                            "assembly_id".to_string(),
                            engine_serialize::Value::Str("game".to_string()),
                        ),
                        (
                            "class_name".to_string(),
                            engine_serialize::Value::Str("Game.Spawned".to_string()),
                        ),
                    ]),
                },
            ),
        ]),
    });
    register_script_prefab(&mut runtime, "prefab-scripted", prefab);

    assert_eq!(runtime.script_engine.managers()[0].instance_count(), 0);
    let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
        "cube-01",
        "prefab-scripted",
        None,
    )]);

    assert!(
        diagnostics.is_empty(),
        "unexpected spawn diagnostics: {diagnostics:?}"
    );
    assert_eq!(
        runtime.script_engine.managers()[0].instance_count(),
        1,
        "the scene-only engine.script record must attach to the spawned entity"
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
        "cube-01",
        "prefab-scripted",
        None,
    )]);
    assert!(
        diagnostics.is_empty(),
        "unexpected spawn diagnostics: {diagnostics:?}"
    );
    assert_eq!(runtime.script_engine.managers()[0].instance_count(), 2);
    runtime
        .with_world(|world| {
            assert!(world.entity_by_persistent_id("prefab-scripted").is_some());
            assert!(world.entity_by_persistent_id("prefab-scripted-2").is_some());
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_character_control_queues_intent_on_the_target_controller() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    runtime.with_world_mut(|world| {
        let entity = world.entity_by_persistent_id("cube-01").unwrap();
        world.add_component(entity, engine_character::CharacterController::new());
    });

    let diagnostics =
        runtime.apply_script_gameplay_commands(vec![engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CharacterControl {
                entity_id: "cube-01".into(),
                direction: [0.6, 0.0, -0.8],
                jump: true,
                speed: Some(7.5),
            },
        }]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    runtime.with_world(|world| {
        let entity = world.entity_by_persistent_id("cube-01").unwrap();
        let controller = world
            .get::<engine_character::CharacterController>(entity)
            .unwrap();
        assert_eq!(controller.pending_commands.len(), 1);
        let command = controller.pending_commands[0];
        assert_eq!(command.direction.to_array(), [0.6, 0.0, -0.8]);
        assert_eq!(command.desired_speed, 7.5);
        assert!(command.jump_requested);
    });
}

#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-ui"))]
#[test]
fn managed_ui_commands_create_and_mutate_retained_canvas_components() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    let layout = engine_script::GameplayUiLayout {
        anchor_min: [0.0, 0.0],
        anchor_max: [0.0, 0.0],
        offset_min: [24.0, 24.0],
        offset_max: [344.0, 56.0],
    };
    let commands = vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::CreateCanvas {
                    canvas_id: "hud".into(),
                    width: 1280.0,
                    height: 720.0,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetCanvasScaleMode {
                    canvas_id: "hud".into(),
                    scale_mode: engine_script::GameplayUiScaleMode::FitWidth,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 1,
                    element: engine_script::GameplayUiElement::Panel {
                        layout,
                        color: engine_script::GameplayUiColor {
                            r: 20,
                            g: 20,
                            b: 20,
                            a: 210,
                        },
                        z_order: 10,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 2,
                    element: engine_script::GameplayUiElement::Text {
                        layout,
                        text: "Score: 0".into(),
                        font_size: 24.0,
                        color: engine_script::GameplayUiColor {
                            r: 255,
                            g: 255,
                            b: 255,
                            a: 255,
                        },
                        z_order: 11,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 3,
                    element: engine_script::GameplayUiElement::Toggle {
                        layout,
                        label: "Music".into(),
                        is_on: false,
                        color_on: engine_script::GameplayUiColor {
                            r: 0,
                            g: 200,
                            b: 80,
                            a: 255,
                        },
                        color_off: engine_script::GameplayUiColor {
                            r: 80,
                            g: 80,
                            b: 80,
                            a: 255,
                        },
                        callback_id: Some("music".into()),
                        z_order: 12,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 4,
                    element: engine_script::GameplayUiElement::Checkbox {
                        layout,
                        label: "Hints".into(),
                        checked: false,
                        color: engine_script::GameplayUiColor {
                            r: 200,
                            g: 200,
                            b: 200,
                            a: 255,
                        },
                        callback_id: Some("hints".into()),
                        z_order: 12,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 5,
                    element: engine_script::GameplayUiElement::Slider {
                        layout,
                        label: "Volume".into(),
                        value: 0.2,
                        min: 0.0,
                        max: 1.0,
                        callback_id: Some("volume".into()),
                        z_order: 12,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetText {
                    canvas_id: "hud".into(),
                    element_id: 2,
                    text: "Score: 10".into(),
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetElementEnabled {
                    canvas_id: "hud".into(),
                    element_id: 1,
                    enabled: false,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetToggleValue {
                    canvas_id: "hud".into(),
                    element_id: 3,
                    is_on: true,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetCheckboxValue {
                    canvas_id: "hud".into(),
                    element_id: 4,
                    checked: true,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetSliderValue {
                    canvas_id: "hud".into(),
                    element_id: 5,
                    value: 0.8,
                },
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    runtime
            .with_world(|world| {
                let hud = world.entity_by_persistent_id("hud").expect("HUD entity");
                let canvas = world
                    .get::<engine_ui::Canvas>(hud)
                    .expect("Canvas component");
                assert_eq!((canvas.width, canvas.height), (1280.0, 720.0));
                assert_eq!(canvas.scale_mode, engine_ui::ScaleMode::FitWidth);
                assert!(!canvas.get_element(engine_ui::ElementId(1)).unwrap().enabled);
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(2)).unwrap().kind,
                    engine_ui::UiElementKind::Text { content, .. } if content == "Score: 10"
                ));
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(3)).unwrap().kind,
                    engine_ui::UiElementKind::Toggle { is_on: true, .. }
                ));
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(4)).unwrap().kind,
                    engine_ui::UiElementKind::Checkbox { checked: true, .. }
                ));
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(5)).unwrap().kind,
                    engine_ui::UiElementKind::Slider { value, .. } if (*value - 0.8).abs() < f32::EPSILON
                ));
            })
            .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn engine_runtime_load_scene_with_scripts() {
    let _guard = serial_ffi_world_test();
    use engine_scene::ComponentRecord;
    use engine_script::MockHost;
    use engine_serialize::SchemaVersion;
    use std::collections::BTreeMap;

    let config = EngineConfig::default();
    let mut runtime = EngineRuntime::new(config);
    runtime.register_script_host(Box::new(MockHost::new()));
    // Match the host name used by MockHost
    runtime.set_script_host_name("mock");

    // Create a minimal scene with a script component
    let mut script_fields = BTreeMap::new();
    script_fields.insert(
        "assembly_id".into(),
        engine_serialize::Value::Str("asm".into()),
    );
    script_fields.insert(
        "class_name".into(),
        engine_serialize::Value::Str("MyScript".into()),
    );

    let mut components = BTreeMap::new();
    components.insert(
        "engine.script".to_string(),
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: script_fields,
        },
    );

    let scene = engine_scene::Scene {
        schema_version: SchemaVersion::new(0, 1, 0),
        engine_version: "0.1.0".to_string(),
        scene_id: "test".to_string(),
        name: "test".to_string(),
        entities: vec![engine_scene::EntityRecord {
            persistent_id: "ent-1".to_string(),
            parent: None,
            name: Some("Entity".to_string()),
            enabled: true,
            components,
        }],
        scene_settings: engine_scene::SceneSettings::default(),
        dependencies: vec![],
        diagnostics_policy: engine_scene::DiagnosticsPolicy::Strict,
    };

    // Pre-load the assembly that the script references
    runtime
        .load_script_assembly("asm", "mock", b"mock_data")
        .unwrap();

    // Load scene — should attach scripts
    runtime
        .load_scene(scene.clone())
        .expect("engine.script metadata should be allowed");

    // After load_scene, the script engine should have an instance
    assert_eq!(runtime.script_engine.host_count(), 1);
    let after = runtime.script_engine.managers()[0].instance_count();
    assert_eq!(after, 1, "script instance should have been created");

    runtime
        .load_scene(scene)
        .expect("reloading a scripted scene should replace its instances");
    assert_eq!(
        runtime.script_engine.managers()[0].instance_count(),
        1,
        "scene reload must not accumulate duplicate script instances"
    );

    // Tick should not produce errors
    runtime.tick_scripts(0.016);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_feature_does_not_ignore_other_unknown_component_types() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut scene = engine_scene::sample_scene();
    insert_empty_component(&mut scene, "engine.script::assembly");

    let diagnostics = runtime
        .load_scene(scene)
        .expect_err("only the exact engine.script type is scene-only");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "SC0030"
            && diagnostic
                .fields
                .get("component_type_id")
                .is_some_and(|type_id| type_id == "engine.script::assembly")
    }));
    assert!(!runtime.has_world());
    assert!(runtime.scene_ref().is_none());
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_component_access_levels_drive_query_and_write_diagnostics() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));

    let commands = vec![
        // ReadOnly component: the write is rejected with the distinct
        // read-only diagnostic, never applied.
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::SetComponent {
                entity_id: "cube-01".into(),
                component_type: "engine.character_controller".into(),
                fields: std::collections::BTreeMap::from([(
                    "move_speed".to_string(),
                    engine_script::GameplayComponentValue::Float(9.0),
                )]),
            },
        },
        // DedicatedApi component: same stable unknown-component
        // diagnostic as unregistered keys.
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::SetComponent {
                entity_id: "cube-01".into(),
                component_type: "engine.transform".into(),
                fields: std::collections::BTreeMap::from([(
                    "translation".to_string(),
                    engine_script::GameplayComponentValue::Vec3([0.0; 3]),
                )]),
            },
        },
        // ReadOnly component queries are accepted: no diagnostic.
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::ComponentQuery {
                query: engine_script::GameplayComponentQuery {
                    query_id: 7,
                    entity_id: "cube-01".into(),
                    component_type: "engine.character_controller".into(),
                },
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);

    let read_only = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "SCRIPT_COMPONENT_READ_ONLY")
        .collect::<Vec<_>>();
    assert_eq!(read_only.len(), 1, "{diagnostics:?}");
    assert!(read_only[0].message.contains("engine.character_controller"));

    let unknown = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "SCRIPT_COMPONENT_UNKNOWN")
        .collect::<Vec<_>>();
    assert_eq!(unknown.len(), 1, "{diagnostics:?}");
    assert!(unknown[0].message.contains("engine.transform"));
    // The supported list in the diagnostic is registry-driven and now
    // includes the read-only character controller.
    assert!(unknown[0].message.contains("engine.character_controller"));
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "SCRIPT_COMPONENT_PAYLOAD_INVALID"),
        "{diagnostics:?}"
    );
}
