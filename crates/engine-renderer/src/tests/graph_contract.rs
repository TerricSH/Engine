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
fn malformed_particle_batch_is_rejected() {
    let mut input = valid_frame();
    input.particle_batches.push(super::ParticleBatch {
        emitter: None,
        mesh: AssetId::new("mesh-vfx-quad"),
        material: AssetId::new("material-vfx"),
        instances: vec![super::ParticleInstance {
            position: [0.0, f32::NAN, 0.0],
            size: 1.0,
            rotation_radians: 0.0,
            normalized_age: 0.5,
            color: [255; 4],
        }],
        gpu_simulation: None,
        bounds: AxisAlignedBox::UNIT,
        render_layer: "Transparent".into(),
        sort_key: 0,
    });

    assert!(validate_frame_input(&input)
        .iter()
        .any(|diagnostic| diagnostic.code == "RV0028"));
}

struct ParticleFallbackCapture {
    observation: Arc<Mutex<Option<(usize, bool)>>>,
}

impl BackendRenderer for ParticleFallbackCapture {
    fn begin_frame(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        let batch = &input.particle_batches[0];
        *self.observation.lock().unwrap() =
            Some((batch.instances.len(), batch.gpu_simulation.is_some()));
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
        _frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }
}

#[test]
fn unsupported_backends_receive_deterministic_cpu_particle_fallback() {
    let observation = Arc::new(Mutex::new(None));
    let mut renderer = Renderer::new_with_backend(Box::new(ParticleFallbackCapture {
        observation: Arc::clone(&observation),
    }));
    let mut input = valid_frame();
    input.particle_batches.push(super::ParticleBatch {
        emitter: None,
        mesh: AssetId::new("mesh-vfx-quad"),
        material: AssetId::new("material-vfx"),
        instances: Vec::new(),
        gpu_simulation: Some(super::GpuParticleSimulation {
            origin: [0.0; 3],
            elapsed: 1.0,
            emission_duration: 0.0,
            emission_rate: 4.0,
            burst_count: 2,
            max_particles: 32,
            lifetime_min: 2.0,
            lifetime_max: 2.0,
            speed_min: 1.0,
            speed_max: 1.0,
            start_size: 1.0,
            end_size: 0.0,
            start_color: [255; 4],
            end_color: [255, 255, 255, 0],
            direction: [0.0, 1.0, 0.0],
            spread_angle_radians: 0.2,
            acceleration: [0.0, -1.0, 0.0],
            drag: 0.1,
            turbulence_strength: 0.0,
            turbulence_frequency: 1.0,
            angular_velocity_min: 0.0,
            angular_velocity_max: 0.0,
            seed: 7,
        }),
        bounds: AxisAlignedBox::UNIT,
        render_layer: "Transparent".into(),
        sort_key: 0,
    });

    renderer.draw_scene(&input).unwrap();
    assert_eq!(*observation.lock().unwrap(), Some((6, false)));
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
fn render_graph_allows_direct_to_swapchain_without_tone_map() {
    let mut input = valid_frame();
    input.render_options.tone_mapping = ToneMapping::None;
    input.render_options.pass_graph_config.output_mode = PassGraphOutputMode::DirectToSwapchain;
    input
        .render_options
        .pass_graph_config
        .passes
        .retain(|pass| pass.kind != "ToneMap");

    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().all(|diagnostic| !matches!(
            diagnostic.severity,
            DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
        )),
        "direct-to-swapchain graph should be valid when tone mapping is disabled: {diagnostics:?}"
    );
}

#[test]
fn direct_to_swapchain_rejects_weighted_oit_without_resolve_pass() {
    let mut input = valid_frame();
    input.render_options.tone_mapping = ToneMapping::None;
    input.render_options.transparency_mode = crate::TransparencyMode::WeightedBlendedOit;
    input.render_options.pass_graph_config.output_mode = PassGraphOutputMode::DirectToSwapchain;
    input
        .render_options
        .pass_graph_config
        .passes
        .retain(|pass| pass.kind != "ToneMap");

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RV0061"
            && diagnostic.path.as_deref() == Some("render_options.transparency_mode")
    }));
}

#[test]
fn direct_to_swapchain_rejects_tone_map_pass() {
    let mut input = valid_frame();
    input.render_options.tone_mapping = ToneMapping::None;
    input.render_options.pass_graph_config.output_mode = PassGraphOutputMode::DirectToSwapchain;

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RV0017"
            && diagnostic.message.contains("DirectToSwapchain")
            && diagnostic.message.contains("found 1")
    }));
}

#[test]
fn hdr_output_keeps_identity_composite_when_tone_mapping_is_none() {
    let mut input = valid_frame();
    input.render_options.tone_mapping = ToneMapping::None;

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics.iter().all(|diagnostic| !matches!(
        diagnostic.severity,
        DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
    )));
}

#[test]
fn direct_to_swapchain_rejects_unapplied_tone_mapping() {
    let mut input = valid_frame();
    input.render_options.pass_graph_config.output_mode = PassGraphOutputMode::DirectToSwapchain;
    input
        .render_options
        .pass_graph_config
        .passes
        .retain(|pass| pass.kind != "ToneMap");

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RV0017"
            && diagnostic.message.contains("DirectToSwapchain")
            && diagnostic.message.contains("ToneMapping::Aces")
    }));
}

#[test]
fn render_graph_requires_tone_map_when_tone_mapping_is_enabled() {
    let mut input = valid_frame();
    input
        .render_options
        .pass_graph_config
        .passes
        .retain(|pass| pass.kind != "ToneMap");

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RV0017"
            && diagnostic.message.contains("HdrThenToneMap")
            && diagnostic.message.contains("found 0")
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
