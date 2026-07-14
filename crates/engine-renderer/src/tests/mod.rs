use super::{
    validate_frame_input, AssetId, AxisAlignedBox, BackendRenderer, BlendMode, ClearFlags,
    Diagnostic, DiagnosticSeverity, FrameStats, IndexFormat, LightItem, LightKind, MaterialUpload,
    MeshUpload, MeshVertexFormat, RenderFrameInput, RenderView, Renderer, ResourceKind,
    ResourceRemoval, SamplerDescriptor, ShadowMode, TextureMipLevel, TextureUpload,
    TextureUploadFormat, Transparency, UploadReceipt, ViewCompose, DIAG_ABORT_UNSUPPORTED,
    DIAG_BACKEND_MISSING, DIAG_INVALID_MATERIAL_VALUES, DIAG_INVALID_MESH_VERTICES,
    DIAG_INVALID_TEXTURE_MIPS, DIAG_MATERIAL_UPLOAD_UNSUPPORTED, DIAG_MESH_UPLOAD_UNSUPPORTED,
    DIAG_TEXTURE_UPLOAD_UNSUPPORTED, IDENTITY_MAT4,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Default)]
struct NullBackend;

impl BackendRenderer for NullBackend {
    fn render_frame(&mut self, _input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        Ok(FrameStats::default())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn upload_mesh(&mut self, _upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }

    fn upload_texture(&mut self, _upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        _upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }

    fn remove_resource(&mut self, _removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn resize(&mut self, _width: u32, _height: u32) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }
}

// ============================================================================
// Renderer::draw_scene tests
// ============================================================================

#[test]
fn empty_frame_is_rejected() {
    let input = RenderFrameInput::empty(0);
    assert!(Renderer::new().draw_scene(&input).is_err());
}

#[test]
fn valid_frame_with_view_succeeds() {
    let mut input = RenderFrameInput::empty(0);
    input.views.push(RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    });
    // Contract tests use an explicit backend; missing backends fail closed.
    let mut renderer = Renderer::new_with_backend(Box::<NullBackend>::default());
    assert!(renderer.draw_scene(&input).is_ok());
}

#[test]
fn draw_scene_with_error_diagnostics_fails() {
    let input = RenderFrameInput::empty(0); // empty views → RV0013 error
    let result = Renderer::new().draw_scene(&input);
    assert!(result.is_err());
    let diagnostics = result.unwrap_err();
    assert!(diagnostics.iter().any(|d| d.code == "RV0013"));
}

// ============================================================================
// validate_frame_input tests
// ============================================================================

#[test]
fn validate_empty_views_produces_rv0013() {
    let input = RenderFrameInput::empty(0);
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0013"),
        "expected RV0013 for empty views, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_duplicate_view_ids_produces_rv0014() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![
        RenderView {
            view_id: 0,
            camera_entity: None,
            viewport: super::Rect::FULL,
            viewport_rect_normalized: super::Rect::FULL,
            view_matrix: IDENTITY_MAT4,
            projection_matrix: IDENTITY_MAT4,
            clear_flags: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: ViewCompose::Base {
                clear: ClearFlags::ColorAndDepth,
                clear_color: [0.0, 0.0, 0.0, 1.0],
            },
            stack_order: 0,
            frustum: None,
        },
        RenderView {
            view_id: 0, // duplicate
            camera_entity: None,
            viewport: super::Rect::FULL,
            viewport_rect_normalized: super::Rect::FULL,
            view_matrix: IDENTITY_MAT4,
            projection_matrix: IDENTITY_MAT4,
            clear_flags: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: ViewCompose::Base {
                clear: ClearFlags::ColorAndDepth,
                clear_color: [0.0, 0.0, 0.0, 1.0],
            },
            stack_order: 1,
            frustum: None,
        },
    ];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0014"),
        "expected RV0014 for duplicate views, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_missing_base_view_produces_rv0007() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 1,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Overlay {
            base_view_id: 99, // non-existent base view
            blend_mode: BlendMode::AlphaBlend,
        },
        stack_order: 0,
        frustum: None,
    }];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0007"),
        "expected RV0007 for overlay with missing base view, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_unsupported_shadow_mode_for_point_light_produces_rv0015() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Point,
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
        range: 10.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Hard, // not supported for Point lights
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0015"),
        "expected RV0015 for unsupported shadow mode on point light, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_unsupported_shadow_mode_for_spot_light_produces_rv0015() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Spot,
        color: [1.0, 1.0, 1.0],
        intensity: 5.0,
        range: 20.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Soft, // not supported for Spot lights
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0015"),
        "expected RV0015 for unsupported shadow mode on spot light, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_directional_shadow_mode_is_accepted() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    // Directional lights support Hard/Soft shadow modes — no RV0015 expected
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Directional,
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
        range: 100.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Hard,
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        !diagnostics.iter().any(|d| d.code == "RV0015"),
        "directional light with Hard shadow should NOT produce RV0015, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_zero_light_intensity_produces_rv0016() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Directional,
        color: [1.0, 1.0, 1.0],
        intensity: 0.0, // zero — should warn
        range: 100.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Off,
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0016"),
        "expected RV0016 for zero intensity light, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_negative_light_intensity_produces_rv0016() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Point,
        color: [1.0, 1.0, 1.0],
        intensity: -1.0, // negative → should warn
        range: 10.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Off,
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0016"),
        "expected RV0016 for negative intensity light, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_invalid_contract_version_produces_rv0012() {
    let mut input = RenderFrameInput::empty(0);
    input.contract_version = "bad-version".to_string();
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0012"),
        "expected RV0012 for invalid contract, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_valid_input_produces_no_diagnostics() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.is_empty(),
        "valid input should produce no diagnostics, got: {:?}",
        diagnostics
    );
}

// ============================================================================
// Strict backend and upload contract tests
// ============================================================================

fn valid_frame() -> RenderFrameInput {
    let mut input = RenderFrameInput::empty(1);
    input.views.push(RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    });
    input
}

#[test]
fn render_graph_requires_all_terminal_passes() {
    let mut input = valid_frame();
    input
        .render_options
        .pass_graph_config
        .passes
        .iter_mut()
        .find(|pass| pass.kind == "Present")
        .expect("default graph contains Present")
        .enabled = false;

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RV0017" && diagnostic.message.contains("Present")
    }));
}

#[test]
fn render_graph_rejects_terminal_pass_reordering() {
    let mut input = valid_frame();
    let passes = &mut input.render_options.pass_graph_config.passes;
    let tone_map = passes
        .iter()
        .position(|pass| pass.kind == "ToneMap")
        .expect("default graph contains ToneMap");
    let present = passes
        .iter()
        .position(|pass| pass.kind == "Present")
        .expect("default graph contains Present");
    passes.swap(tone_map, present);

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "RV0019"));
}

fn valid_mesh_upload() -> MeshUpload {
    MeshUpload {
        mesh_id: AssetId::new("mesh.triangle"),
        vertex_format: MeshVertexFormat::Pbr32,
        vertex_count: 3,
        vertex_bytes: vec![0; 3 * 32],
        index_format: IndexFormat::U16,
        index_count: 3,
        index_bytes: vec![0, 0, 1, 0, 2, 0],
        bounds: AxisAlignedBox::UNIT,
        content_hash: [1; 32],
    }
}

fn valid_texture_upload() -> TextureUpload {
    TextureUpload {
        texture_id: AssetId::new("texture.checker"),
        width: 2,
        height: 2,
        format: TextureUploadFormat::Rgba8,
        color_space: super::ColorSpace::Srgb,
        mip_levels: vec![
            TextureMipLevel {
                width: 2,
                height: 2,
                bytes: vec![255; 16],
            },
            TextureMipLevel {
                width: 1,
                height: 1,
                bytes: vec![255; 4],
            },
        ],
        sampler: SamplerDescriptor::default(),
        content_hash: [2; 32],
    }
}

fn valid_material_upload() -> MaterialUpload {
    MaterialUpload {
        material_id: AssetId::new("material.default"),
        base_color: [1.0, 1.0, 1.0, 1.0],
        metallic: 0.0,
        roughness: 1.0,
        ambient_occlusion: 1.0,
        base_color_texture: Some(AssetId::new("texture.checker")),
        transparency: Transparency::Opaque,
        double_sided: false,
        content_hash: [3; 32],
    }
}

#[test]
fn no_backend_fails_closed_for_draw_resize_uploads_and_removal() {
    let mut renderer = Renderer::new();
    let draw = renderer.draw_scene(&valid_frame()).unwrap_err();
    assert!(draw.iter().any(|d| d.code == DIAG_BACKEND_MISSING));

    for diagnostics in [
        renderer.resize(1280, 720).unwrap_err(),
        renderer.upload_mesh(valid_mesh_upload()).unwrap_err(),
        renderer.upload_texture(valid_texture_upload()).unwrap_err(),
        renderer
            .upload_material(valid_material_upload())
            .unwrap_err(),
        renderer
            .remove_resource(ResourceRemoval {
                kind: ResourceKind::Mesh,
                resource_id: AssetId::new("mesh.triangle"),
            })
            .unwrap_err(),
    ] {
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DIAG_BACKEND_MISSING));
    }
}

#[derive(Default)]
struct UnsupportedBackend;

impl BackendRenderer for UnsupportedBackend {
    fn render_frame(&mut self, _input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        Ok(FrameStats::default())
    }
}

#[test]
fn backend_default_uploads_report_stable_unsupported_diagnostics() {
    let mut renderer = Renderer::new_with_backend(Box::<UnsupportedBackend>::default());
    assert!(renderer
        .upload_mesh(valid_mesh_upload())
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_MESH_UPLOAD_UNSUPPORTED));
    assert!(renderer
        .upload_texture(valid_texture_upload())
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_TEXTURE_UPLOAD_UNSUPPORTED));
    assert!(renderer
        .upload_material(valid_material_upload())
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_MATERIAL_UPLOAD_UNSUPPORTED));
}

struct CountingBackend {
    upload_calls: Arc<AtomicUsize>,
}

impl BackendRenderer for CountingBackend {
    fn render_frame(&mut self, _input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        Ok(FrameStats::default())
    }

    fn upload_mesh(&mut self, _upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        self.upload_calls.fetch_add(1, Ordering::SeqCst);
        Ok(UploadReceipt::new(1))
    }

    fn upload_texture(&mut self, _upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        self.upload_calls.fetch_add(1, Ordering::SeqCst);
        Ok(UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        _upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        self.upload_calls.fetch_add(1, Ordering::SeqCst);
        Ok(UploadReceipt::new(1))
    }
}

#[test]
fn invalid_uploads_never_enter_the_backend() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut renderer = Renderer::new_with_backend(Box::new(CountingBackend {
        upload_calls: Arc::clone(&calls),
    }));

    let mut mesh = valid_mesh_upload();
    mesh.vertex_bytes.pop();
    let mesh_errors = renderer.upload_mesh(mesh).unwrap_err();
    assert!(mesh_errors
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_MESH_VERTICES));

    let mut texture = valid_texture_upload();
    texture.mip_levels[1].bytes.pop();
    let texture_errors = renderer.upload_texture(texture).unwrap_err();
    assert!(texture_errors
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_TEXTURE_MIPS));

    let mut material = valid_material_upload();
    material.roughness = f32::NAN;
    let material_errors = renderer.upload_material(material).unwrap_err();
    assert!(material_errors
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_MATERIAL_VALUES));

    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[derive(Clone, Copy)]
enum FailureStage {
    Barrier,
    Pass,
    End,
}

struct FailingFrameBackend {
    stage: FailureStage,
    abort_calls: Arc<AtomicUsize>,
    abort_fails: bool,
}

impl FailingFrameBackend {
    fn stage_error(&self) -> Vec<Diagnostic> {
        vec![Diagnostic::new(
            "TEST_FRAME_FAILURE",
            DiagnosticSeverity::Error,
            "renderer.test",
            "injected frame failure",
        )]
    }
}

impl BackendRenderer for FailingFrameBackend {
    fn render_frame(&mut self, _input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        Ok(FrameStats::default())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &super::render_graph::PassNode,
        _barriers: &[super::render_graph::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        if matches!(self.stage, FailureStage::Barrier) {
            Err(self.stage_error())
        } else {
            Ok(())
        }
    }

    fn execute_pass(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &super::render_graph::PassNode,
        _frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if matches!(self.stage, FailureStage::Pass) {
            Err(self.stage_error())
        } else {
            Ok(())
        }
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        if matches!(self.stage, FailureStage::End) {
            Err(self.stage_error())
        } else {
            Ok(())
        }
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        self.abort_calls.fetch_add(1, Ordering::SeqCst);
        if self.abort_fails {
            Err(vec![Diagnostic::new(
                "TEST_ABORT_FAILURE",
                DiagnosticSeverity::Error,
                "renderer.test",
                "injected abort failure",
            )])
        } else {
            Ok(())
        }
    }
}

#[test]
fn barrier_pass_and_end_failures_abort_the_frame_once() {
    for stage in [FailureStage::Barrier, FailureStage::Pass, FailureStage::End] {
        let abort_calls = Arc::new(AtomicUsize::new(0));
        let mut renderer = Renderer::new_with_backend(Box::new(FailingFrameBackend {
            stage,
            abort_calls: Arc::clone(&abort_calls),
            abort_fails: false,
        }));
        let diagnostics = renderer.draw_scene(&valid_frame()).unwrap_err();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "TEST_FRAME_FAILURE"));
        assert_eq!(abort_calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn abort_error_is_appended_without_losing_the_original_failure() {
    let abort_calls = Arc::new(AtomicUsize::new(0));
    let mut renderer = Renderer::new_with_backend(Box::new(FailingFrameBackend {
        stage: FailureStage::Pass,
        abort_calls: Arc::clone(&abort_calls),
        abort_fails: true,
    }));
    let diagnostics = renderer.draw_scene(&valid_frame()).unwrap_err();
    assert_eq!(diagnostics[0].code, "TEST_FRAME_FAILURE");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "TEST_ABORT_FAILURE"));
    assert_eq!(abort_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn default_abort_reports_that_the_backend_cannot_reset_recording_state() {
    let mut backend = UnsupportedBackend;
    assert!(backend
        .abort_frame()
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_ABORT_UNSUPPORTED));
}
