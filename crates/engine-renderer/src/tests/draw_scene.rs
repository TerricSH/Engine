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
fn weighted_oit_fails_closed_on_an_unsupported_backend() {
    let mut input = valid_frame();
    input.render_options.transparency_mode = crate::TransparencyMode::WeightedBlendedOit;
    let diagnostics = Renderer::new_with_backend(Box::<NullBackend>::default())
        .draw_scene(&input)
        .unwrap_err();
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "RV0144"));
}

struct GraphCountingBackend {
    begin_calls: Arc<AtomicUsize>,
    pass_calls: Arc<AtomicUsize>,
    end_calls: Arc<AtomicUsize>,
}

impl BackendRenderer for GraphCountingBackend {
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        self.begin_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &super::render_graph2::PassNode,
        _barriers: &[super::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn execute_pass(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &super::render_graph2::PassNode,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        self.pass_calls.fetch_add(1, Ordering::SeqCst);
        stats.draw_calls = 7;
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        self.end_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn backend_always_uses_the_render_graph_lifecycle() {
    let begin_calls = Arc::new(AtomicUsize::new(0));
    let pass_calls = Arc::new(AtomicUsize::new(0));
    let end_calls = Arc::new(AtomicUsize::new(0));
    let mut renderer = Renderer::new_with_backend(Box::new(GraphCountingBackend {
        begin_calls: Arc::clone(&begin_calls),
        pass_calls: Arc::clone(&pass_calls),
        end_calls: Arc::clone(&end_calls),
    }));

    let stats = renderer.draw_scene(&valid_frame()).unwrap();

    assert_eq!(begin_calls.load(Ordering::SeqCst), 1);
    assert_eq!(pass_calls.load(Ordering::SeqCst), 3);
    assert_eq!(end_calls.load(Ordering::SeqCst), 1);
    assert_eq!(stats.draw_calls, 7);
}

#[test]
fn draw_scene_with_error_diagnostics_fails() {
    let input = RenderFrameInput::empty(0); // empty views → RV0013 error
    let result = Renderer::new().draw_scene(&input);
    assert!(result.is_err());
    let diagnostics = result.unwrap_err();
    assert!(diagnostics.iter().any(|d| d.code == "RV0013"));
}

#[test]
fn non_finite_exposure_override_is_rejected_before_backend_execution() {
    let mut input = valid_frame();
    input.render_options.exposure_ev100 = Some(f32::NAN);

    let diagnostics = validate_frame_input(&input);

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RV0022"
            && diagnostic.path.as_deref() == Some("render_options.exposure_ev100")
    }));
}

#[test]
fn invalid_post_process_parameters_are_rejected_before_backend_execution() {
    let mut input = valid_frame();
    input.render_options.post_process.bloom.radius = f32::NAN;

    let diagnostics = validate_frame_input(&input);

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RV0025"
            && diagnostic.path.as_deref() == Some("render_options.post_process")
    }));
}

#[test]
fn malformed_environment_cubemap_is_rejected_before_backend_execution() {
    let upload = EnvironmentMapUpload {
        environment_id: AssetId::new("environment.test"),
        format: EnvironmentMapFormat::Rgba16Float,
        mip_levels: vec![EnvironmentCubeMip {
            face_size: 2,
            faces: vec![vec![0; 2 * 2 * 8]; 5],
        }],
        content_hash: [0; 32],
    };

    let diagnostics = Renderer::new().upload_environment_map(upload).unwrap_err();

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_ENVIRONMENT_MAP));
}

#[test]
fn normalized_sub_viewports_are_valid_but_empty_or_out_of_surface_rects_are_rejected() {
    let mut input = valid_frame();
    let embedded = super::Rect {
        min: [0.2, 0.1],
        max: [0.8, 0.9],
    };
    input.views[0].viewport = embedded;
    input.views[0].viewport_rect_normalized = embedded;
    assert!(!validate_frame_input(&input)
        .iter()
        .any(|diagnostic| diagnostic.code == "RV0023"));

    input.views[0].viewport.max[0] = input.views[0].viewport.min[0];
    input.views[0].viewport_rect_normalized.min[1] = -0.01;
    let diagnostics = validate_frame_input(&input);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "RV0023")
            .count(),
        2
    );
}

#[test]
fn frame_material_mask_cutoff_must_be_finite_and_bounded() {
    let mut input = valid_frame();
    input.materials.push(super::MaterialBinding {
        material_id: AssetId::new("material.invalid-mask"),
        pipeline: AssetId::new("pipeline.forward"),
        variant_key: 0,
        textures: Vec::new(),
        uniforms: super::ParamBlock {
            bytes: Vec::new(),
            layout_hash: [0; 32],
        },
        pass_mask: 1,
        transparency: Transparency::Masked { cutoff: f32::NAN },
        double_sided: true,
    });

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RV0024"
            && diagnostic.path.as_deref() == Some("materials[0].transparency.cutoff")
    }));
}
