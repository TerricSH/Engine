struct MockDevice {
    next_index: u32,
    create_calls: u32,
    destroyed: Vec<PipelineHandle>,
    created_descs: Vec<PipelineDescriptor>,
}

impl MockDevice {
    fn new() -> Self {
        Self {
            next_index: 1,
            create_calls: 0,
            destroyed: Vec::new(),
            created_descs: Vec::new(),
        }
    }
}

impl Device for MockDevice {
    fn adapter_info(&self) -> &render_core::AdapterInfo {
        unimplemented!("not needed in tests")
    }

    fn create_pipeline(
        &mut self,
        desc: &PipelineDescriptor,
    ) -> Result<PipelineHandle, render_core::RhiError> {
        self.create_calls += 1;
        self.created_descs.push(desc.clone());
        let handle = PipelineHandle::new(self.next_index, 1);
        self.next_index += 1;
        Ok(handle)
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) {
        self.destroyed.push(handle);
    }

    fn destroy_buffer(&mut self, _buffer: BufferHandle) {}

    fn destroy_texture(&mut self, _texture: render_core::TextureHandle) {}

    fn destroy_shader_module(&mut self, _module: ShaderModuleHandle) {}

    fn destroy_render_pass(&mut self, _pass: RenderPassHandle) {}

    fn destroy_framebuffer(&mut self, _framebuffer: FramebufferHandle) {}

    fn destroy_pipeline_layout(&mut self, _layout: PipelineLayoutHandle) {}

    fn destroy_swapchain(&mut self, _swapchain: SwapchainHandle) {}

    fn destroy_surface(&mut self, _surface: render_core::SurfaceHandle) {}

    fn wait_idle(&self) {}
}

#[derive(Default)]
struct MockEncoder;

impl CommandEncoder for MockEncoder {
    fn begin_render_pass(
        &mut self,
        _render_pass: RenderPassHandle,
        _framebuffer: FramebufferHandle,
        _area: (u32, u32, u32, u32),
        _clear_color: [f32; 4],
        _clear_depth: Option<f32>,
    ) {
    }

    fn bind_pipeline(&mut self, _pipeline: PipelineHandle) {}

    fn bind_vertex_buffers(&mut self, _buffers: &[BufferHandle], _offsets: &[u64]) {}

    fn bind_index_buffer(
        &mut self,
        _buffer: BufferHandle,
        _offset: u64,
        _index_format: IndexFormat,
    ) {
    }

    fn bind_descriptor_sets(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _first_set: u32,
        _sets: &[render_core::DescriptorSetHandle],
        _dynamic_offsets: &[u32],
    ) -> Result<(), render_core::RhiError> {
        Ok(())
    }

    fn set_viewport(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        _min_depth: f32,
        _max_depth: f32,
    ) {
    }

    fn set_scissor(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) {}

    fn draw(
        &mut self,
        _vertex_count: u32,
        _instance_count: u32,
        _first_vertex: u32,
        _first_instance: u32,
    ) {
    }

    fn draw_indexed(
        &mut self,
        _index_count: u32,
        _instance_count: u32,
        _first_index: u32,
        _vertex_offset: i32,
        _first_instance: u32,
    ) {
    }

    fn end_render_pass(&mut self) {}

    fn push_constants(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _stage_flags: u32,
        _offset: u32,
        _data: &[u8],
    ) {
    }
}

struct CountingPass {
    kind: &'static str,
    prepare_count: Arc<AtomicUsize>,
    execute_count: Arc<AtomicUsize>,
    fail_prepare: bool,
    reads_depth: bool,
    writes_swapchain: bool,
}

impl CountingPass {
    fn new(
        kind: &'static str,
        prepare_count: Arc<AtomicUsize>,
        execute_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            kind,
            prepare_count,
            execute_count,
            fail_prepare: false,
            reads_depth: false,
            writes_swapchain: false,
        }
    }

    fn with_declared_resources(mut self) -> Self {
        self.reads_depth = true;
        self.writes_swapchain = true;
        self
    }

    fn failing(
        kind: &'static str,
        prepare_count: Arc<AtomicUsize>,
        execute_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            kind,
            prepare_count,
            execute_count,
            fail_prepare: true,
            reads_depth: false,
            writes_swapchain: false,
        }
    }
}

impl RenderPass for CountingPass {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn declare(&self, view_id: u32) -> render_graph2::PassNode {
        let inputs = self
            .reads_depth
            .then(|| render_graph2::PassAttachment {
                name: "depth_stencil".into(),
                format: Some("D32".into()),
                clear: false,
                load_op: "load".into(),
                size_source: render_graph2::SizeSource::Swapchain,
                access: render_graph2::ResourceAccess::Read,
            })
            .into_iter()
            .collect();
        let outputs = self
            .writes_swapchain
            .then(|| render_graph2::PassAttachment {
                name: "swapchain".into(),
                format: None,
                clear: false,
                load_op: "load".into(),
                size_source: render_graph2::SizeSource::Swapchain,
                access: render_graph2::ResourceAccess::Write,
            })
            .into_iter()
            .collect();
        render_graph2::PassNode {
            kind: render_graph2::PassKind::Custom(self.kind),
            name: self.kind,
            view_id,
            inputs,
            outputs,
            depth_stencil: None,
        }
    }

    fn prepare(&mut self, _device: &mut dyn Device) -> Result<(), Vec<Diagnostic>> {
        self.prepare_count.fetch_add(1, Ordering::SeqCst);
        if self.fail_prepare {
            Err(vec![Diagnostic::new(
                "TEST_PREPARE",
                DiagnosticSeverity::Error,
                "test",
                "custom pass preparation failed",
            )])
        } else {
            Ok(())
        }
    }

    fn execute(
        &mut self,
        _input: &RenderFrameInput,
        _encoder: &mut dyn CommandEncoder,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        self.execute_count.fetch_add(1, Ordering::SeqCst);
        stats.draw_calls = stats.draw_calls.saturating_add(1);
        Ok(())
    }
}

fn frame_with_custom_pass(kind: &str) -> RenderFrameInput {
    let mut input = RenderFrameInput::empty(7);
    input.views.push(engine_renderer::RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: engine_renderer::Rect::FULL,
        viewport_rect_normalized: engine_renderer::Rect::FULL,
        view_matrix: engine_renderer::IDENTITY_MAT4,
        projection_matrix: engine_renderer::IDENTITY_MAT4,
        clear_flags: engine_renderer::ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: engine_renderer::ViewCompose::Base {
            clear: engine_renderer::ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    });
    input.render_options.pass_graph_config = engine_renderer::PassGraphConfig {
        passes: vec![
            engine_renderer::PassConfigEntry {
                kind: kind.to_string(),
                enabled: true,
            },
            engine_renderer::PassConfigEntry {
                kind: "Present".to_string(),
                enabled: true,
            },
        ],
        enabled: true,
        output_mode: engine_renderer::PassGraphOutputMode::HdrThenToneMap,
    };
    input
}

#[test]
fn configured_custom_pass_is_prepared_once_and_executed() {
    let prepare_count = Arc::new(AtomicUsize::new(0));
    let execute_count = Arc::new(AtomicUsize::new(0));
    let mut registry = PassRegistry::new();
    let mut device = MockDevice::new();
    prepare_and_register_custom_pass(
        &mut registry,
        &mut device,
        Box::new(CountingPass::new(
            "custom_post",
            Arc::clone(&prepare_count),
            Arc::clone(&execute_count),
        )),
    )
    .expect("custom pass registration");

    let input = frame_with_custom_pass("custom_post");
    let mut graph = engine_renderer::render_graph2::RenderGraph::build_with_config(
        &input,
        &input.render_options.pass_graph_config,
    );
    apply_registered_custom_pass_declarations(&registry, &mut graph)
        .expect("custom pass declaration");
    let compiled = graph.compile().expect("custom render graph compile");
    let mut encoder = MockEncoder;
    let mut stats = FrameStats::default();
    let mut executed_custom_node = false;
    for pass_index in compiled.pass_order {
        let pass = &graph.passes[pass_index];
        if let engine_renderer::render_graph2::PassKind::Custom(name) = pass.kind {
            execute_registered_custom_pass(&mut registry, name, &input, &mut encoder, &mut stats)
                .expect("registered custom pass execution");
            executed_custom_node = true;
        }
    }

    assert!(executed_custom_node);
    assert_eq!(prepare_count.load(Ordering::SeqCst), 1);
    assert_eq!(execute_count.load(Ordering::SeqCst), 1);
    assert_eq!(stats.draw_calls, 1);
}

#[test]
fn registered_custom_pass_declaration_populates_graph_resources() {
    let mut registry = PassRegistry::new();
    let mut device = MockDevice::new();
    prepare_and_register_custom_pass(
        &mut registry,
        &mut device,
        Box::new(
            CountingPass::new(
                "custom_composite",
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
            )
            .with_declared_resources(),
        ),
    )
    .expect("custom pass registration");

    let input = frame_with_custom_pass("custom_composite");
    let mut graph = engine_renderer::render_graph2::RenderGraph::build_with_config(
        &input,
        &input.render_options.pass_graph_config,
    );
    apply_registered_custom_pass_declarations(&registry, &mut graph)
        .expect("custom pass declaration");
    let custom = graph
        .passes
        .iter()
        .find(|node| {
            matches!(
                &node.kind,
                engine_renderer::render_graph2::PassKind::Custom(name)
                    if *name == "custom_composite"
            )
        })
        .expect("custom graph node");

    assert_eq!(custom.inputs.len(), 1);
    assert_eq!(custom.inputs[0].name, "depth_stencil");
    assert_eq!(
        custom.inputs[0].access,
        engine_renderer::render_graph2::ResourceAccess::Read
    );
    assert_eq!(custom.outputs.len(), 1);
    assert_eq!(custom.outputs[0].name, "swapchain");
    assert_eq!(
        custom.outputs[0].access,
        engine_renderer::render_graph2::ResourceAccess::Write
    );
}

#[test]
fn duplicate_custom_pass_is_rejected_before_prepare() {
    let first_prepares = Arc::new(AtomicUsize::new(0));
    let duplicate_prepares = Arc::new(AtomicUsize::new(0));
    let execute_count = Arc::new(AtomicUsize::new(0));
    let mut registry = PassRegistry::new();
    let mut device = MockDevice::new();
    prepare_and_register_custom_pass(
        &mut registry,
        &mut device,
        Box::new(CountingPass::new(
            "custom_post",
            Arc::clone(&first_prepares),
            Arc::clone(&execute_count),
        )),
    )
    .expect("first registration");

    let diagnostics = prepare_and_register_custom_pass(
        &mut registry,
        &mut device,
        Box::new(CountingPass::new(
            "custom_post",
            Arc::clone(&duplicate_prepares),
            Arc::clone(&execute_count),
        )),
    )
    .expect_err("duplicate registration must fail");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "RV0299"));
    assert_eq!(first_prepares.load(Ordering::SeqCst), 1);
    assert_eq!(duplicate_prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn failed_custom_pass_prepare_does_not_register_the_pass() {
    let prepare_count = Arc::new(AtomicUsize::new(0));
    let execute_count = Arc::new(AtomicUsize::new(0));
    let mut registry = PassRegistry::new();
    let mut device = MockDevice::new();

    let diagnostics = prepare_and_register_custom_pass(
        &mut registry,
        &mut device,
        Box::new(CountingPass::failing(
            "custom_post",
            Arc::clone(&prepare_count),
            execute_count,
        )),
    )
    .expect_err("prepare failure must fail registration");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "TEST_PREPARE"));
    assert_eq!(prepare_count.load(Ordering::SeqCst), 1);
    assert!(registry.find("custom_post").is_none());
}

#[test]
fn configured_unregistered_custom_pass_still_fails_closed() {
    let input = frame_with_custom_pass("missing_post");
    let mut registry = PassRegistry::new();
    let mut encoder = MockEncoder;
    let mut stats = FrameStats::default();

    let diagnostics = execute_registered_custom_pass(
        &mut registry,
        "missing_post",
        &input,
        &mut encoder,
        &mut stats,
    )
    .expect_err("unregistered custom pass must fail");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "RV0291"));
}
