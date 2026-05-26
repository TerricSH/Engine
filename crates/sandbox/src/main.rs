#![forbid(unsafe_code)]

use engine_core::{EngineConfig, EngineRuntime};

fn main() {
    tracing_subscriber::fmt::init();
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "workspace".to_string());
    match command.as_str() {
        "workspace" => tracing::info!("engine workspace initialized"),
        "gate04-scene" => run_gate04_scene(),
        "contract-triangle" => run_contract_triangle(),
        "static-lit-scene" => run_static_lit_scene(),
        "triangle" => run_triangle(),
        "textured-object" => run_textured_object(),
        "resize-smoke" => run_resize_smoke(),
        other => {
            tracing::error!(command = other, "unknown sandbox command");
            std::process::exit(2);
        }
    }
}

fn run_gate04_scene() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.load_scene(engine_scene::sample_scene());
    match runtime.render_frame(0) {
        Ok(stats) => tracing::info!(
            draw_calls = stats.draw_calls,
            "gate04 scene rendered through contract path"
        ),
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                tracing::error!(code = diagnostic.code, message = diagnostic.message);
            }
            std::process::exit(1);
        }
    }
}

// ============================================================================
// contract-triangle: renders a triangle through Renderer → BackendRenderer
// ============================================================================

#[cfg(feature = "backend-vulkan")]
fn run_contract_triangle() {
    use engine_renderer::{
        BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, RenderFrameInput, Renderer,
    };
    use platform::winit::window::Window;
    use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use render_vulkan::device_impl::VulkanDevice;
    use std::sync::Arc;

    struct ContractBackend {
        device: VulkanDevice,
    }

    impl BackendRenderer for ContractBackend {
        fn render_frame(
            &mut self,
            _input: &RenderFrameInput,
        ) -> Result<FrameStats, Vec<Diagnostic>> {
            match self.device.render_triangle_frame() {
                Ok(stats) => Ok(FrameStats {
                    draw_calls: stats.draw_calls,
                    triangles: stats.triangles,
                    visible_drawables: 1,
                    ..FrameStats::default()
                }),
                Err(e) => Err(vec![Diagnostic::new(
                    "RV0099",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("triangle frame failed: {e}"),
                )]),
            }
        }
    }

    struct ContractTriangleApp {
        renderer: Option<Renderer>,
        frames: u64,
        max_frames: Option<u64>,
        backend: Option<ContractBackend>,
    }

    impl WindowApp for ContractTriangleApp {
        fn on_create(&mut self, window: Arc<Window>) {
            let size = window.inner_size();
            let display_handle = match window.display_handle() {
                Ok(h) => h.as_raw(),
                Err(err) => {
                    tracing::error!(error = %err, "failed to acquire raw display handle");
                    return;
                }
            };
            let window_handle = match window.window_handle() {
                Ok(h) => h.as_raw(),
                Err(err) => {
                    tracing::error!(error = %err, "failed to acquire raw window handle");
                    return;
                }
            };
            let enable_validation = std::env::var("ENGINE_VK_VALIDATION").is_ok();

            let mut vk_device: VulkanDevice = match VulkanDevice::new(
                display_handle,
                window_handle,
                size.width.max(1),
                size.height.max(1),
                enable_validation,
            ) {
                Ok(d) => d,
                Err(err) => {
                    tracing::error!(error = %err, "VulkanDevice creation failed");
                    std::process::exit(1);
                }
            };

            // Set the embedded triangle shaders.
            vk_device.set_mvp_shaders(
                render_vulkan::shaders_embedded::TRIANGLE_VERT_SPV,
                render_vulkan::shaders_embedded::TRIANGLE_FRAG_SPV,
            );

            let backend = ContractBackend { device: vk_device };
            let mut renderer = Renderer::new();
            renderer.set_backend(Box::new(backend));

            self.renderer = Some(renderer);
            tracing::info!("contract-triangle renderer initialized");
        }

        fn on_event(&mut self, _window: &Window, event: PlatformEvent) -> EventFlow {
            match event {
                PlatformEvent::Resized { .. } => EventFlow::Continue,
                PlatformEvent::Redraw => {
                    if let Some(ref mut renderer) = self.renderer {
                        let input = RenderFrameInput::empty(self.frames);
                        match renderer.draw_scene(&input) {
                            Ok(stats) => {
                                tracing::info!(
                                    draw_calls = stats.draw_calls,
                                    triangles = stats.triangles,
                                    "contract-triangle frame rendered"
                                );
                            }
                            Err(diags) => {
                                for d in &diags {
                                    tracing::error!(code = d.code, message = d.message);
                                }
                                return EventFlow::Exit;
                            }
                        }
                        self.frames += 1;
                        if let Some(limit) = self.max_frames {
                            if self.frames >= limit {
                                tracing::info!(
                                    frames = self.frames,
                                    "frame limit reached; exiting"
                                );
                                return EventFlow::Exit;
                            }
                        }
                    }
                    EventFlow::Continue
                }
                PlatformEvent::CloseRequested => EventFlow::Exit,
                PlatformEvent::Resumed | PlatformEvent::Suspended => EventFlow::Continue,
            }
        }
    }

    let max_frames = parse_frame_limit();

    let app = ContractTriangleApp {
        renderer: None,
        frames: 0,
        max_frames,
        backend: None,
    };
    if let Err(err) = platform::run(
        WindowDescriptor {
            title: "Engine Sandbox - Contract Triangle".to_string(),
            width: 1280,
            height: 720,
        },
        app,
    ) {
        tracing::error!(error = %err, "platform run failed");
        std::process::exit(1);
    }
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_contract_triangle() {
    tracing::error!("contract-triangle requires `backend-vulkan` feature");
    std::process::exit(2);
}

// ============================================================================
// static-lit-scene: renders a colored quad through Device trait methods
// (create_buffer, write_buffer, create_render_pass, create_framebuffer,
//  create_pipeline, begin_frame → CommandEncoder → end_frame)
// ============================================================================

#[cfg(feature = "backend-vulkan")]
fn run_static_lit_scene() {
    use engine_renderer::{
        BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, RenderFrameInput, Renderer,
    };
    use platform::winit::window::Window;
    use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use render_core::CommandEncoder;
    use render_core::{
        self, BufferDescriptor, BufferHandle, Device, MemoryHint, PipelineDescriptor,
        PipelineLayoutDescriptor, PushConstantRange, RenderPassDescriptor, SwapchainDescriptor,
        TextureFormat, VertexAttribute, VertexLayout,
    };
    use render_vulkan::device_impl::VulkanDevice;
    use std::sync::Arc;

    // Colored quad for FORWARD shaders: position (float32x3) + color (float32x4) = 28 bytes/vertex
    const VERTEX_DATA: &[u8] = &[
        0, 0, 0, 0xBF, 0, 0, 0xBF, 0, 0, 0, 0, 0, 0, 0x80, 0x3F, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0x80, 0x3F, // -0.5,-0.5,0, 1,0,0,1
        0, 0, 0x3F, 0, 0, 0xBF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3F, 0, 0, 0, 0, 0, 0, 0x80,
        0x3F, // 0.5,-0.5,0, 0,1,0,1
        0, 0, 0x3F, 0, 0, 0x3F, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3F, 0, 0, 0x80,
        0x3F, // 0.5,0.5,0, 0,0,1,1
        0, 0, 0xBF, 0, 0, 0x3F, 0, 0, 0, 0, 0, 0, 0x80, 0x3F, 0, 0, 0x80, 0x3F, 0, 0, 0, 0, 0, 0,
        0x80, 0x3F, // -0.5,0.5,0, 1,1,0,1
    ];

    struct SceneBackend {
        device: VulkanDevice,
        initialized: bool,
        vertex_buf: Option<BufferHandle>,
        rp: Option<render_core::RenderPassHandle>,
        fb: Option<render_core::FramebufferHandle>,
        pl: Option<render_core::PipelineHandle>,
        pll: Option<render_core::PipelineLayoutHandle>,
        // Frame lifecycle state (for multi-pass dispatch)
        cur_sc: Option<render_core::SwapchainHandle>,
        cur_ii: Option<u32>,
        cur_enc: Option<Box<dyn CommandEncoder>>,
    }

    impl SceneBackend {
        fn init_once(&mut self) -> Result<(), Vec<Diagnostic>> {
            if self.initialized {
                return Ok(());
            }
            self.device.render_triangle_frame().map_err(|e| {
                vec![Diagnostic::new(
                    "RV0099",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("init: {e}"),
                )]
            })?;
            let vb_desc = BufferDescriptor {
                size_bytes: VERTEX_DATA.len() as u64,
                usage_flags: render_core::BufferUsage(0),
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some("quad-vertices".into()),
            };
            let vb = self.device.create_buffer(&vb_desc).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0100",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("{e:?}"),
                )]
            })?;
            self.device.write_buffer(vb, VERTEX_DATA, 0).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0101",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("{e:?}"),
                )]
            })?;
            let rp_desc = RenderPassDescriptor {
                color_attachments: vec![TextureFormat::Bgra8Unorm],
                depth_stencil_format: Some(TextureFormat::Depth32Float),
                sample_count: 1,
                debug_label: Some("scene-rp".into()),
            };
            let rp = self.device.create_render_pass(&rp_desc).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0102",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("{e:?}"),
                )]
            })?;
            let pll_desc = PipelineLayoutDescriptor {
                bind_group_layouts: vec![],
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 3,
                    offset: 0,
                    size: 128,
                }],
                debug_label: Some("scene-pll".into()),
            };
            let pll = self.device.create_pipeline_layout(&pll_desc).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0107",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("{e:?}"),
                )]
            })?;
            let pl_desc = PipelineDescriptor {
                shader_modules: vec![],
                vertex_layout: VertexLayout {
                    stride_bytes: 28,
                    attributes: vec![
                        VertexAttribute {
                            semantic: "position".into(),
                            format: "float32x3".into(),
                            offset_bytes: 0,
                        },
                        VertexAttribute {
                            semantic: "color".into(),
                            format: "float32x4".into(),
                            offset_bytes: 12,
                        },
                    ],
                },
                bind_layouts: vec![],
                pipeline_layout: Some(pll),
                raster_state: render_core::RasterState {
                    cull_mode: Some("none".into()),
                    front_face: None,
                },
                depth_state: render_core::DepthState {
                    format: Some(TextureFormat::Depth32Float),
                    write_enabled: true,
                    compare: Some("less".into()),
                },
                blend_state: render_core::BlendState { mode: None },
                render_targets: vec![TextureFormat::Bgra8Unorm],
                debug_label: Some("scene-pl".into()),
            };
            let pl = self.device.create_pipeline(&pl_desc).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0103",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("{e:?}"),
                )]
            })?;
            self.vertex_buf = Some(vb);
            self.rp = Some(rp);
            self.fb = Some(render_core::FramebufferHandle::new(0, 0));
            self.pll = Some(pll);
            self.pl = Some(pl);
            self.initialized = true;
            tracing::info!("static-lit-scene resources initialized");
            Ok(())
        }
    }

    impl BackendRenderer for SceneBackend {
        fn render_frame(
            &mut self,
            _input: &RenderFrameInput,
        ) -> Result<FrameStats, Vec<Diagnostic>> {
            // Legacy path: do initialization + full frame
            self.init_once()?;
            self.device.write_default_ubo();
            let sc = SwapchainDescriptor {
                surface: render_core::SurfaceHandle::new(0, 1),
                width: 1280,
                height: 720,
                vsync: false,
                debug_label: None,
            };
            let sc_h = self.device.create_swapchain(&sc).unwrap();
            let (ii, mut encoder) = self.device.begin_frame(sc_h).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0105",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("{e:?}"),
                )]
            })?;
            if let (Some(rp), Some(fb)) = (self.rp, self.fb) {
                encoder.begin_render_pass(rp, fb, (0, 0, 1280, 720), [0.02, 0.02, 0.06, 1.0], None);
            }
            encoder.set_viewport(0.0, 0.0, 1280.0, 720.0, 0.0, 1.0);
            encoder.set_scissor(0, 0, 1280, 720);
            if let Some(pl) = self.pl {
                encoder.bind_pipeline(pl);
            }
            if let Some(pll) = self.pll {
                encoder.bind_descriptor_sets(pll, 0, &[], &[]);
            }
            if let Some(pll) = self.pll {
                let mut pc = Vec::with_capacity(128);
                for i in 0..16 {
                    let v = if i % 5 == 0 { 1.0f32 } else { 0.0f32 };
                    pc.extend_from_slice(&v.to_ne_bytes());
                }
                for v in &[0.5f32, -1.0, 0.5, 0.0] {
                    pc.extend_from_slice(&v.to_ne_bytes());
                }
                for v in &[1.5f32, 1.5, 1.5, 1.5] {
                    pc.extend_from_slice(&v.to_ne_bytes());
                }
                for v in &[0.15f32, 0.15, 0.15, 0.15] {
                    pc.extend_from_slice(&v.to_ne_bytes());
                }
                encoder.push_constants(pll, 3, 0, &pc);
            }
            if let Some(vb) = self.vertex_buf {
                encoder.bind_vertex_buffers(&[vb], &[0]);
            }
            encoder.draw(4, 1, 0, 0);
            encoder.end_render_pass();
            let stats = self.device.end_frame(sc_h, encoder, ii).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0106",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("{e:?}"),
                )]
            })?;
            Ok(FrameStats {
                draw_calls: stats.draw_calls,
                triangles: 2,
                visible_drawables: 1,
                ..FrameStats::default()
            })
        }

        fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
            self.init_once()?;
            self.device.write_default_ubo();
            let sc = SwapchainDescriptor {
                surface: render_core::SurfaceHandle::new(0, 1),
                width: 1280,
                height: 720,
                vsync: false,
                debug_label: None,
            };
            let sc_h = self.device.create_swapchain(&sc).unwrap();
            let (ii, enc) = self.device.begin_frame(sc_h).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0105",
                    DiagnosticSeverity::Error,
                    "sandbox",
                    &format!("{e:?}"),
                )]
            })?;
            self.cur_sc = Some(sc_h);
            self.cur_ii = Some(ii);
            self.cur_enc = Some(enc);
            Ok(())
        }

        fn execute_pass(
            &mut self,
            _input: &RenderFrameInput,
            pass: &engine_renderer::render_graph::PassNode,
            _stats: &mut FrameStats,
        ) -> Result<(), Vec<Diagnostic>> {
            let Some(ref mut encoder) = self.cur_enc else {
                return Ok(());
            };

            match pass.kind {
                engine_renderer::render_graph::PassKind::DirectionalShadow => {
                    // Shadow pass: no-op for MVP (no shadow-casting objects)
                }
                engine_renderer::render_graph::PassKind::OpaquePbrForward => {
                    if let (Some(rp), Some(fb)) = (self.rp, self.fb) {
                        encoder.begin_render_pass(
                            rp,
                            fb,
                            (0, 0, 1280, 720),
                            [0.02, 0.02, 0.06, 1.0],
                            None,
                        );
                    }
                    encoder.set_viewport(0.0, 0.0, 1280.0, 720.0, 0.0, 1.0);
                    encoder.set_scissor(0, 0, 1280, 720);
                    if let Some(pl) = self.pl {
                        encoder.bind_pipeline(pl);
                    }
                    if let Some(pll) = self.pll {
                        encoder.bind_descriptor_sets(pll, 0, &[], &[]);
                    }
                    if let Some(pll) = self.pll {
                        let mut pc = Vec::with_capacity(128);
                        for i in 0..16 {
                            let v = if i % 5 == 0 { 1.0f32 } else { 0.0f32 };
                            pc.extend_from_slice(&v.to_ne_bytes());
                        }
                        for v in &[0.5f32, -1.0, 0.5, 0.0] {
                            pc.extend_from_slice(&v.to_ne_bytes());
                        }
                        for v in &[1.5f32, 1.5, 1.5, 1.5] {
                            pc.extend_from_slice(&v.to_ne_bytes());
                        }
                        for v in &[0.15f32, 0.15, 0.15, 0.15] {
                            pc.extend_from_slice(&v.to_ne_bytes());
                        }
                        encoder.push_constants(pll, 3, 0, &pc);
                    }
                    if let Some(vb) = self.vertex_buf {
                        encoder.bind_vertex_buffers(&[vb], &[0]);
                    }
                    encoder.draw(4, 1, 0, 0);
                    encoder.end_render_pass();
                }
                engine_renderer::render_graph::PassKind::ToneMap => {
                    // Tone-mapping: no-op for MVP (forward pass renders directly to swapchain)
                }
                engine_renderer::render_graph::PassKind::Present => {
                    // Present is handled by end_frame
                }
            }
            Ok(())
        }

        fn end_frame(&mut self, stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
            if let (Some(sc_h), Some(ii)) = (self.cur_sc.take(), self.cur_ii.take()) {
                let enc = self.cur_enc.take().unwrap();
                let s = self.device.end_frame(sc_h, enc, ii).map_err(|e| {
                    vec![Diagnostic::new(
                        "RV0106",
                        DiagnosticSeverity::Error,
                        "sandbox",
                        &format!("{e:?}"),
                    )]
                })?;
                stats.draw_calls = s.draw_calls;
                stats.triangles = s.triangles;
            }
            Ok(())
        }
    }

    struct StaticLitSceneApp {
        renderer: Option<Renderer>,
        frames: u64,
        max_frames: Option<u64>,
    }

    impl WindowApp for StaticLitSceneApp {
        fn on_create(&mut self, window: Arc<Window>) {
            let size = window.inner_size();
            let dh = match window.display_handle() {
                Ok(h) => h.as_raw(),
                Err(e) => {
                    tracing::error!("dh: {e}");
                    return;
                }
            };
            let wh = match window.window_handle() {
                Ok(h) => h.as_raw(),
                Err(e) => {
                    tracing::error!("wh: {e}");
                    return;
                }
            };
            let val = std::env::var("ENGINE_VK_VALIDATION").is_ok();

            let mut device =
                match VulkanDevice::new(dh, wh, size.width.max(1), size.height.max(1), val) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!("VulkanDevice: {e}");
                        std::process::exit(1);
                    }
                };
            device.set_mvp_shaders(
                render_vulkan::shaders_embedded::FORWARD_VERT_SPV,
                render_vulkan::shaders_embedded::FORWARD_FRAG_SPV,
            );

            let backend = SceneBackend {
                device,
                initialized: false,
                vertex_buf: None,
                rp: None,
                fb: None,
                pl: None,
                pll: None,
                cur_sc: None,
                cur_ii: None,
                cur_enc: None,
            };
            let mut renderer = Renderer::new();
            renderer.set_backend(Box::new(backend));
            self.renderer = Some(renderer);
            tracing::info!("static-lit-scene renderer initialized");
        }

        fn on_event(&mut self, _window: &Window, event: PlatformEvent) -> EventFlow {
            match event {
                PlatformEvent::Resized { .. } => EventFlow::Continue,
                PlatformEvent::Redraw => {
                    if let Some(ref mut renderer) = self.renderer {
                        let input = RenderFrameInput::empty(self.frames);
                        match renderer.draw_scene(&input) {
                            Ok(stats) => tracing::info!(
                                draw_calls = stats.draw_calls,
                                "static-lit-scene frame"
                            ),
                            Err(diags) => {
                                for d in &diags {
                                    tracing::error!(code = d.code, msg = d.message);
                                }
                                return EventFlow::Exit;
                            }
                        }
                        self.frames += 1;
                        if let Some(limit) = self.max_frames {
                            if self.frames >= limit {
                                return EventFlow::Exit;
                            }
                        }
                    }
                    EventFlow::Continue
                }
                PlatformEvent::CloseRequested => EventFlow::Exit,
                PlatformEvent::Resumed | PlatformEvent::Suspended => EventFlow::Continue,
            }
        }
    }

    let max_frames = parse_frame_limit();
    let app = StaticLitSceneApp {
        renderer: None,
        frames: 0,
        max_frames,
    };
    if let Err(e) = platform::run(
        WindowDescriptor {
            title: "Static Lit Scene".into(),
            width: 1280,
            height: 720,
        },
        app,
    ) {
        tracing::error!("platform: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_static_lit_scene() {
    tracing::error!("static-lit-scene requires backend-vulkan");
    std::process::exit(2);
}

// ============================================================================
// Legacy Vulkan demos (Gate 2 — unchanged)
// ============================================================================

#[cfg(feature = "backend-vulkan")]
fn run_triangle() {
    run_vulkan_scene(
        "Engine Sandbox - Triangle",
        render_vulkan::VulkanSceneKind::Triangle,
        false,
    );
}

#[cfg(feature = "backend-vulkan")]
fn run_textured_object() {
    run_vulkan_scene(
        "Engine Sandbox - Textured Object",
        render_vulkan::VulkanSceneKind::TexturedQuad,
        false,
    );
}

#[cfg(feature = "backend-vulkan")]
fn run_resize_smoke() {
    run_vulkan_scene(
        "Engine Sandbox - Resize Smoke",
        render_vulkan::VulkanSceneKind::TexturedQuad,
        true,
    );
}

#[cfg(feature = "backend-vulkan")]
fn run_vulkan_scene(title: &str, scene: render_vulkan::VulkanSceneKind, auto_resize: bool) {
    use std::sync::Arc;

    use platform::winit::window::Window;
    use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use render_vulkan::{VulkanRenderer, VulkanRendererDescriptor, VulkanSceneKind};

    struct VulkanSampleApp {
        renderer: Option<VulkanRenderer>,
        frames: u64,
        max_frames: Option<u64>,
        scene: VulkanSceneKind,
        auto_resize: bool,
    }

    impl WindowApp for VulkanSampleApp {
        fn on_create(&mut self, window: Arc<Window>) {
            let size = window.inner_size();
            let display_handle = match window.display_handle() {
                Ok(h) => h.as_raw(),
                Err(err) => {
                    tracing::error!(error = %err, "failed to acquire raw display handle");
                    return;
                }
            };
            let window_handle = match window.window_handle() {
                Ok(h) => h.as_raw(),
                Err(err) => {
                    tracing::error!(error = %err, "failed to acquire raw window handle");
                    return;
                }
            };
            let enable_validation = std::env::var("ENGINE_VK_VALIDATION").is_ok();
            match VulkanRenderer::new(VulkanRendererDescriptor {
                display_handle,
                window_handle,
                width: size.width.max(1),
                height: size.height.max(1),
                enable_validation,
                scene: self.scene,
            }) {
                Ok(renderer) => {
                    tracing::info!("vulkan renderer initialized");
                    self.renderer = Some(renderer);
                }
                Err(err) => {
                    tracing::error!(error = %err, "vulkan renderer initialization failed");
                    std::process::exit(1);
                }
            }
        }

        fn on_event(&mut self, window: &Window, event: PlatformEvent) -> EventFlow {
            match event {
                PlatformEvent::Resized { width, height } => {
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.resize(width, height);
                    }
                    EventFlow::Continue
                }
                PlatformEvent::Redraw => {
                    if let Some(renderer) = self.renderer.as_mut() {
                        if self.auto_resize {
                            request_resize_step(window, self.frames);
                        }
                        if let Err(err) = renderer.render() {
                            tracing::error!(error = %err, "frame render failed");
                            return EventFlow::Exit;
                        }
                        self.frames += 1;
                        if let Some(limit) = self.max_frames {
                            if self.frames >= limit {
                                tracing::info!(
                                    frames = self.frames,
                                    "frame limit reached; exiting"
                                );
                                renderer.wait_idle();
                                return EventFlow::Exit;
                            }
                        }
                    }
                    EventFlow::Continue
                }
                PlatformEvent::CloseRequested => {
                    if let Some(renderer) = self.renderer.as_ref() {
                        renderer.wait_idle();
                    }
                    EventFlow::Exit
                }
                PlatformEvent::Resumed | PlatformEvent::Suspended => EventFlow::Continue,
            }
        }
    }

    let max_frames = parse_frame_limit();

    let app = VulkanSampleApp {
        renderer: None,
        frames: 0,
        max_frames,
        scene,
        auto_resize,
    };
    if let Err(err) = platform::run(
        WindowDescriptor {
            title: title.to_string(),
            width: 1280,
            height: 720,
        },
        app,
    ) {
        tracing::error!(error = %err, "platform run failed");
        std::process::exit(1);
    }
}

#[cfg(feature = "backend-vulkan")]
fn parse_frame_limit() -> Option<u64> {
    let mut args = std::env::args().skip(2);
    while let Some(arg) = args.next() {
        if arg == "--frames" {
            return args.next().and_then(|value| value.parse::<u64>().ok());
        }
        if let Some(value) = arg.strip_prefix("--frames=") {
            return value.parse::<u64>().ok();
        }
    }
    None
}

#[cfg(feature = "backend-vulkan")]
fn request_resize_step(window: &platform::winit::window::Window, frame: u64) {
    let size = match frame {
        30 => Some((960, 540)),
        60 => Some((320, 240)),
        90 => Some((1280, 720)),
        _ => None,
    };
    if let Some((width, height)) = size {
        let _ = window.request_inner_size(platform::winit::dpi::PhysicalSize::new(width, height));
        tracing::info!(width, height, "resize-smoke requested window size");
    }
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_triangle() {
    tracing::error!("the `triangle` command requires `backend-vulkan`");
    std::process::exit(2);
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_textured_object() {
    tracing::error!("the `textured-object` command requires `backend-vulkan`");
    std::process::exit(2);
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_resize_smoke() {
    tracing::error!("the `resize-smoke` command requires `backend-vulkan`");
    std::process::exit(2);
}
