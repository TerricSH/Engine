#![forbid(unsafe_code)]

use engine_asset::ReloadCoordinator;
use engine_core::{EngineConfig, EngineRuntime};

mod diagnostics;

fn main() {
    tracing_subscriber::fmt::init();
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "workspace".to_string());
    match command.as_str() {
        "workspace" => tracing::info!("engine workspace initialized"),
        "gate04-scene" => run_gate04_scene(),
        "engine-character-demo" => run_engine_character_demo(),
        "contract-triangle" => run_contract_triangle(),
        "static-lit-scene" => run_static_lit_scene(),
        "triangle" => run_triangle(),
        "model-viewer" => run_model_viewer(),
        "engine-model-viewer" => run_engine_model_viewer(),
        "textured-object" => run_textured_object(),
        "resize-smoke" => run_resize_smoke(),
        "editor" => run_editor(),
        other => {
            tracing::error!(command = other, "unknown sandbox command");
            std::process::exit(2);
        }
    }
}

#[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
mod editor_app;
#[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
use editor_app::run_editor;

#[cfg(not(all(feature = "tooling-editor", feature = "backend-vulkan")))]
fn run_editor() {
    tracing::error!("editor requires `tooling-editor` and `backend-vulkan` features");
    std::process::exit(2);
}

fn run_gate04_scene() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.load_scene(engine_scene::sample_scene());

    let dir = std::env::temp_dir().join("sandbox_reload");
    let _ = std::fs::create_dir_all(&dir);
    let reload_coordinator = ReloadCoordinator::new(&dir, &dir, &dir)
        .expect("reload coordinator creation should succeed");
    let mut sandbox_diags = diagnostics::SandboxDiagnostics::new();

    match runtime.render_frame(0) {
        Ok(stats) => {
            tracing::info!(
                draw_calls = stats.draw_calls,
                "gate04 scene rendered through contract path"
            );

            // The runtime's DiagnosticsCollector already recorded frame stats
            // inside render_frame().  Build a RuntimeDiagnostics snapshot and
            // feed it to the sandbox aggregator along with reload coordinator state.
            let runtime_diags = runtime.runtime_diagnostics();
            sandbox_diags.update(&runtime_diags, &reload_coordinator);

            // Log aggregated diagnostics
            let all = sandbox_diags.all_diagnostics();
            tracing::info!(count = all.len(), "sandbox diagnostics collected");
            for diagnostic in &all {
                tracing::debug!(
                    code = diagnostic.code,
                    severity = ?diagnostic.severity,
                    message = diagnostic.message,
                    "aggregated diagnostic"
                );
            }

            // Also log the raw render stats for immediate feedback.
            tracing::info!(
                draw_calls = stats.draw_calls,
                triangles = stats.triangles,
                gpu_ms = stats.gpu_frame_ms,
                visible = stats.visible_drawables,
                culled = stats.culled_drawables,
                "gate04 frame stats"
            );
        }
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
                Some(std::path::Path::new("./pso_cache")),
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
                _ => EventFlow::Continue,
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
                topology: Some("triangle_list".into()),
                polygon_mode: Some("fill".into()),
                sample_count: Some(1),
                render_pass: None,
                specialization: Vec::new(),
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
                engine_renderer::render_graph::PassKind::Custom(_) => {
                    // Custom passes are no-ops until explicitly wired.
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

            let mut device = match VulkanDevice::new(
                dh,
                wh,
                size.width.max(1),
                size.height.max(1),
                val,
                Some(std::path::Path::new("./pso_cache")),
            ) {
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
                                triangles = stats.triangles,
                                "model-viewer frame"
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
                _ => EventFlow::Continue,
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
// engine-character-demo: WASD-controlled capsule using the engine pipeline
// (GameLoop → EngineRuntime → Renderer → BackendRenderer).
// ============================================================================

#[cfg(feature = "backend-vulkan")]
fn run_engine_character_demo() {
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::Instant;

    use engine_character::{CharacterController, CharacterMovement};
    use engine_core::game_loop::GameLoop;
    use engine_core::EngineConfig;
    use engine_gameplay::input::{
        self as gameplay_input, InputAction, InputActionMap, InputValue, InputValueType, KeyCode,
    };
    use engine_physics::{
        BodyType, Collider, ColliderShape, PhysicsWorld, RigidBody,
    };
    use engine_scene::components::Transform;
    use engine_scene::Entity;
    use glam::Quat;
    use glam::Vec3;
    use platform::winit::window::Window;
    use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

    struct EngineCharacterApp {
        game_loop: Option<GameLoop>,
        controller: Option<CharacterController>,
        physics: Option<PhysicsWorld>,
        held_keys: HashSet<u32>,
        input_map: InputActionMap,
        frames: u64,
        max_frames: Option<u64>,
        last_frame_time: Instant,
        player_entity: Entity,
        camera_entity: Entity,
    }

    // ── Map winit PhysicalKey scancodes → engine-gameplay KeyCodes ──
    fn scancode_to_keycode(scancode: u32) -> Option<KeyCode> {
        match scancode {
            26 => Some(KeyCode::W), // HID Keyboard W
            4  => Some(KeyCode::A), // HID Keyboard A
            22 => Some(KeyCode::S), // HID Keyboard S
            7  => Some(KeyCode::D), // HID Keyboard D
            44 => Some(KeyCode::Space), // HID Keyboard Space
            _ => None,
        }
    }

    // ── Build the InputActionMap for WASD + Space ──────────────────
    fn build_player_input_map() -> InputActionMap {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(InputAction::new("move_forward", InputValueType::Digital));
        map.add_action(InputAction::new("move_back", InputValueType::Digital));
        map.add_action(InputAction::new("move_left", InputValueType::Digital));
        map.add_action(InputAction::new("move_right", InputValueType::Digital));
        map.add_action(InputAction::new("jump", InputValueType::Digital));
        map
    }

    fn action_name_for(key: KeyCode) -> &'static str {
        match key {
            KeyCode::W => "move_forward",
            KeyCode::S => "move_back",
            KeyCode::A => "move_left",
            KeyCode::D => "move_right",
            KeyCode::Space => "jump",
            _ => "unknown",
        }
    }

    fn current_bool(map: &InputActionMap, name: &str) -> bool {
        matches!(
            gameplay_input::query_current_value(map, name),
            Some(InputValue::Bool(true))
        )
    }

    // ── Mesh builders using engine_asset ─────────────────────────

    fn build_ground_mesh() -> (Vec<u8>, Vec<u8>, u32, bool) {
        use engine_asset::mesh::{mesh_data_to_color_bytes, MeshData};
        use glam::Vec3;
        let mesh = MeshData {
            positions: vec![
                Vec3::new(-10.0, -0.5, -10.0),
                Vec3::new(10.0, -0.5, -10.0),
                Vec3::new(10.0, -0.5, 10.0),
                Vec3::new(-10.0, -0.5, 10.0),
            ],
            normals: vec![Vec3::Y; 4],
            uvs: vec![],
            indices: vec![0, 1, 2, 0, 2, 3],
            bounds: (Vec3::new(-10.0, -0.5, -10.0), Vec3::new(10.0, -0.5, 10.0)),
            joints: vec![],
            weights: vec![],
        };
        mesh_data_to_color_bytes(&mesh)
    }

    fn build_capsule_mesh() -> (Vec<u8>, Vec<u8>, u32, bool) {
        use engine_asset::mesh::{mesh_data_to_color_bytes, MeshData};
        use glam::Vec3;
        let segs: u32 = 12;
        let rings_top: u32 = 3;
        let rings_mid: u32 = 2;
        let rings_bot: u32 = 3;
        let total_rings = rings_top + rings_mid + rings_bot;
        let half_h = 0.75;
        let radius = 0.3;
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut indices = Vec::new();
        for ring in 0..=total_rings {
            let ring_f = ring as f32;
            let (cy, cr, ny, nr) = if ring <= rings_top {
                // Top hemisphere: θ=0 (top pole) → θ=π/2 (equator)
                let theta = ring_f / rings_top as f32 * std::f32::consts::FRAC_PI_2;
                let y = half_h + radius * theta.cos();
                let r = radius * theta.sin().max(0.001);
                (y, r, theta.cos(), theta.sin().max(0.001))
            } else if ring <= rings_top + rings_mid {
                // Cylinder body: y = half_h → -half_h, r = radius
                let t = (ring_f - rings_top as f32) / rings_mid as f32;
                let y = half_h - t * 2.0 * half_h;
                (y, radius, 0.0, 1.0)
            } else {
                // Bottom hemisphere: θ=0 (equator) → θ=π/2 (bottom pole)
                let t = (ring_f - rings_top as f32 - rings_mid as f32) / rings_bot as f32;
                let theta = t * std::f32::consts::FRAC_PI_2;
                let y = -half_h - radius * (1.0 - theta.cos());
                let r = radius * theta.cos().max(0.001);
                (y, r, -theta.sin(), theta.cos().max(0.001))
            };
            for seg in 0..segs {
                let a = seg as f32 / segs as f32 * std::f32::consts::TAU;
                let nx = a.cos();
                let nz = a.sin();
                positions.push(Vec3::new(nx * cr, cy, nz * cr));
                normals.push(Vec3::new(nx * nr, ny, nz * nr));
            }
        }
        for ring in 0..total_rings {
            for seg in 0..segs {
                let ns = (seg + 1) % segs;
                let a = ring * segs + seg;
                let b = ring * segs + ns;
                let c = (ring + 1) * segs + seg;
                let d = (ring + 1) * segs + ns;
                indices.push(a); indices.push(b); indices.push(c);
                indices.push(b); indices.push(d); indices.push(c);
            }
        }
        let mesh = MeshData {
            positions, normals,
            uvs: vec![], indices,
            bounds: (Vec3::new(-radius, -half_h - radius, -radius), Vec3::new(radius, half_h + radius, radius)),
            joints: vec![], weights: vec![],
        };
        mesh_data_to_color_bytes(&mesh)
    }

    impl WindowApp for EngineCharacterApp {
        fn on_create(&mut self, window: Arc<Window>) {
            let size = window.inner_size();
            let w = size.width;
            let h = size.height;

            // ── Create rendering backend through engine API ──────────
            let backend = match engine_core::create_vulkan_backend_renderer(
                window.display_handle().unwrap().as_raw(),
                window.window_handle().unwrap().as_raw(),
                w,
                h,
                cfg!(debug_assertions),
                None,
            ) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("backend creation failed: {e}");
                    return;
                }
            };

            // ── Build engine runtime with backend ──────────────────────
            let mut game_loop = GameLoop::new(EngineConfig {
                application_name: "engine-character-demo".into(),
            });
            game_loop.runtime.renderer_mut().set_backend(backend);

            // ── Build the ECS world ────────────────────────────────────
            use engine_scene::World;
            let mut world = World::new();

            let ground = world.create_entity();
            world.add_component(
                ground,
                RigidBody {
                    body_type: BodyType::Static,
                    ..RigidBody::default()
                },
            );
            world.add_component(
                ground,
                Collider {
                    shape: ColliderShape::Cuboid {
                        hx: 10.0,
                        hy: 0.5,
                        hz: 10.0,
                    },
                    ..Collider::default()
                },
            );
            world.add_component(
                ground,
                Transform {
                    translation: Vec3::new(0.0, -0.5, 0.0),
                    ..Transform::default()
                },
            );

            let player = world.create_entity();
            world.add_component(
                player,
                Transform {
                    translation: Vec3::new(0.0, 3.0, 0.0),
                    ..Transform::default()
                },
            );

            // ── Camera entity (third-person, behind+above player) ─────
            let camera = world.create_entity();
            world.add_component(
                camera,
                Transform {
                    translation: Vec3::new(0.0, 5.0, 8.0),
                    rotation: glam::Quat::from_rotation_x(-0.45),
                    ..Transform::default()
                },
            );
            world.add_component(camera, engine_scene::components::Camera::default());

            // ── Directional light ──
            let light_entity = world.create_entity();
            world.add_component(
                light_entity,
                engine_scene::components::Light {
                    kind: engine_scene::components::LightKind::Directional,
                    color: [1.0, 1.0, 1.0],
                    intensity: 100000.0,
                    range: 100.0,
                    spot_angles: None,
                    shadow_mode: 1,
                    direction: [0.5, -0.8, 0.3],
                },
            );

            // ── Renderable components ──────────────────────────────────
            world.add_component(
                ground,
                engine_scene::components::Renderable {
                    mesh_asset: "mesh-ground".into(),
                    material_asset: "default".into(),
                    visible: true,
                    cast_shadows: false,
                    render_layer: "default".into(),
                },
            );
            world.add_component(
                player,
                engine_scene::components::Renderable {
                    mesh_asset: "mesh-hero".into(),
                    material_asset: "default".into(),
                    visible: true,
                    cast_shadows: true,
                    render_layer: "default".into(),
                },
            );

            // ── Upload meshes through engine renderer API ──────────────
            let (ground_vb, ground_ib, ground_ic, ground_u16) = build_ground_mesh();
            let (cube_vb, cube_ib, cube_ic, cube_u16) = build_capsule_mesh();

            let _ = game_loop.runtime.renderer_mut().upload_mesh(
                "mesh-ground",
                &ground_vb,
                &ground_ib,
                ground_ic,
                ground_u16,
            );
            let _ = game_loop.runtime.renderer_mut().upload_mesh(
                "mesh-hero",
                &cube_vb,
                &cube_ib,
                cube_ic,
                cube_u16,
            );

            // ── Place the World in EngineRuntime ───────────────────────
            // After this, use game_loop.runtime.world_mut() exclusively.
            game_loop.runtime.set_world(world);

            // ── Init physics ───────────────────────────────────────────
            let mut physics = PhysicsWorld::new(Vec3::new(0.0, -9.81, 0.0));
            if let Some(w) = game_loop.runtime.world() {
                physics.sync_from_ecs(w);
            }

            // ── Character controller ───────────────────────────────────
            let mut controller = CharacterController::new();
            controller.set_position(Vec3::new(0.0, 3.0, 0.0));

            self.game_loop = Some(game_loop);
            self.physics = Some(physics);
            self.controller = Some(controller);
            self.input_map = build_player_input_map();
            self.player_entity = player;
            self.camera_entity = camera;
        }

        fn on_event(&mut self, window: &Window, event: PlatformEvent) -> EventFlow {
            match event {
                PlatformEvent::KeyPressed { key, .. } => {
                    self.held_keys.insert(key);
                    if let Some(gk) = scancode_to_keycode(key) {
                        gameplay_input::set_current_value(
                            &mut self.input_map,
                            &action_name_for(gk),
                            InputValue::Bool(true),
                        );
                    }
                }
                PlatformEvent::KeyReleased { key, .. } => {
                    self.held_keys.remove(&key);
                    if let Some(gk) = scancode_to_keycode(key) {
                        gameplay_input::set_current_value(
                            &mut self.input_map,
                            &action_name_for(gk),
                            InputValue::Bool(false),
                        );
                    }
                }
                PlatformEvent::Resized { width, height } => {
                    if let Some(ref mut gl) = self.game_loop {
                        let _ = gl.runtime.renderer_mut().resize(width, height);
                    }
                }
                PlatformEvent::Redraw => {
                    let dt = self.last_frame_time.elapsed().as_secs_f32();
                    self.last_frame_time = Instant::now();

                        // ── Read movement from InputActionMap ──────────
                        let fwd = current_bool(&self.input_map, "move_forward");
                        let back = current_bool(&self.input_map, "move_back");
                        let left = current_bool(&self.input_map, "move_left");
                        let right = current_bool(&self.input_map, "move_right");
                        let jump = current_bool(&self.input_map, "jump");

                        let (dx, dz) = (
                            (right as i8 - left as i8) as f32,
                            (fwd as i8 - back as i8) as f32,
                        );
                    let dir = Vec3::new(dx, 0.0, dz);
                    let dir = if dir.length_squared() > 0.001 {
                        dir.normalize()
                    } else {
                        dir
                    };

                    // ── Character + physics + render in one borrow ─────
                    if let (Some(ref mut gl), Some(ref mut ctrl), Some(ref mut physics)) =
                        (&mut self.game_loop, &mut self.controller, &mut self.physics)
                    {
                        // Character movement
                        let input = CharacterMovement {
                            direction: dir,
                            wish_jump: jump,
                            delta_time: dt.min(0.1),
                        };
                        ctrl.update(&input, Some(physics));

                        // Write character position to runtime's world
                        if let Some(rw) = gl.runtime.world_mut() {
                            let pos = ctrl.position();
                            if let Some(t) = rw.get_mut::<Transform>(self.player_entity) {
                                t.translation = pos;
                            }
                        }

                        // ── Orbit camera follows the player ────────────
                        if let Some(rw) = gl.runtime.world_mut() {
                            let pos = ctrl.position();
                            if let Some(t) = rw.get_mut::<Transform>(self.camera_entity) {
                                let eye = pos + Vec3::new(0.0, 5.0, 8.0);
                                let dir = (pos - eye).normalize();
                                t.translation = eye;
                                t.rotation = Quat::from_rotation_arc(-Vec3::Z, dir);
                            }
                        }

                        // Step physics on runtime's world
                        if let Some(rw) = gl.runtime.world_mut() {
                            physics.step(dt.min(0.1), rw);
                        }

                        // Render
                        if let Err(errs) = gl.render(self.frames) {
                            for e in &errs {
                                tracing::warn!(code = e.code, "render error: {}", e.message);
                            }
                        }
                    }
                    window.request_redraw();
                    self.frames += 1;
                    if self.max_frames.is_some_and(|l| self.frames >= l) {
                        return EventFlow::Exit;
                    }
                }
                PlatformEvent::CloseRequested => return EventFlow::Exit,
                _ => {}
            }
            EventFlow::Continue
        }
    }

    fn parse_frame_limit() -> Option<u64> {
        std::env::args()
            .skip(1)
            .find(|a| a.starts_with("--frames="))
            .and_then(|s| s.split('=').nth(1).and_then(|v| v.parse().ok()))
    }

    let max_frames = parse_frame_limit();
    let app = EngineCharacterApp {
        game_loop: None,
        controller: None,
        physics: None,
        held_keys: HashSet::new(),
        input_map: build_player_input_map(),
        frames: 0,
        max_frames,
        last_frame_time: Instant::now(),
        player_entity: Entity::new(0, 0),
        camera_entity: Entity::new(0, 0),
    };

    if let Err(e) = platform::run(
        WindowDescriptor {
            title: "Engine Character Demo".into(),
            width: 1280,
            height: 720,
        },
        app,
    ) {
        tracing::error!("{e}");
    }
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_engine_character_demo() {
    tracing::error!("engine-character-demo requires `backend-vulkan` feature");
    std::process::exit(2);
}

// ============================================================================
// model-viewer: loads a glTF mesh and renders it with orbit camera + FORWARD
// shaders through the VulkanDevice model rendering path.
// ============================================================================

#[cfg(feature = "backend-vulkan")]
fn run_model_viewer() {
    use engine_asset::mesh::load_mesh_from_gltf;
    use engine_renderer::{
        BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, RenderFrameInput, Renderer,
    };
    use glam::{Mat4, Vec2, Vec3};
    use platform::winit::window::Window;
    use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use render_core::{BufferDescriptor, BufferHandle, Device, MemoryHint};
    use render_vulkan::device_impl::VulkanDevice;
    use std::sync::Arc;

    // ── CLI ───────────────────────────────────────────────────────────────
    // Parse model path (skip --frames / --frames=N flags).
    let model_path = std::env::args().skip(2).find(|a| !a.starts_with("--"));
    let mesh = match model_path.as_deref() {
        Some(path) if !path.is_empty() => match load_mesh_from_gltf(std::path::Path::new(path)) {
            Ok(m) => m,
            Err(err) => {
                tracing::warn!(path, error = %err, "glTF load failed, using test cube");
                engine_asset::mesh::create_test_cube()
            }
        },
        _ => {
            tracing::info!("no model path provided, using test cube");
            engine_asset::mesh::create_test_cube()
        }
    };
    tracing::info!(
        vertices = mesh.positions.len(),
        indices = mesh.indices.len(),
        "mesh loaded"
    );

    // ── BackendRenderer implementation ────────────────────────────────────
    struct ModelViewerBackend {
        device: VulkanDevice,
        vertex_buf: BufferHandle,
        index_buf: BufferHandle,
        index_count: u32,
        camera_angle: f32,
        width: f32,
        height: f32,
        saved_screenshot: bool,
    }

    impl BackendRenderer for ModelViewerBackend {
        fn execute_pass(
            &mut self,
            input: &RenderFrameInput,
            pass: &engine_renderer::render_graph::PassNode,
            stats: &mut FrameStats,
        ) -> Result<(), Vec<Diagnostic>> {
            if pass.kind != engine_renderer::render_graph::PassKind::OpaquePbrForward {
                return Ok(()); // only render on the forward pass
            }
            let frame_stats = self.render_frame(input)?;
            stats.draw_calls += frame_stats.draw_calls;
            stats.triangles += frame_stats.triangles;
            stats.visible_drawables += frame_stats.visible_drawables;
            Ok(())
        }

        fn render_frame(
            &mut self,
            _input: &RenderFrameInput,
        ) -> Result<FrameStats, Vec<Diagnostic>> {
            let width = self.width;
            let height = self.height;

            // ── Orbit camera ──────────────────────────────────────────
            self.camera_angle += 0.015;
            let radius = 3.0;
            let eye = Vec3::new(
                radius * self.camera_angle.sin(),
                0.6,
                radius * self.camera_angle.cos(),
            );
            let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
            let aspect = width / height;
            let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);

            // OpenGL NDC [-1,1] → Vulkan NDC [0,1] (flip Y + remap Z)
            let vulkan_correction = Mat4::from_cols_array_2d(&[
                [1.0, 0.0, 0.0, 0.0],
                [0.0, -1.0, 0.0, 0.0],
                [0.0, 0.0, 0.5, 0.0],
                [0.0, 0.0, 0.5, 1.0],
            ]);
            let view_proj = vulkan_correction * proj * view;

            // Model matrix = identity (mesh is at origin).
            let model = Mat4::IDENTITY;

            // ── Pack UBO (176 bytes matching forward shader layout) ───
            let mut ubo = Vec::with_capacity(176);
            // model (mat4, offset 0)
            for v in model.to_cols_array_2d().iter().flatten() {
                ubo.extend_from_slice(&v.to_ne_bytes());
            }
            // view_proj (mat4, offset 64)
            for v in view_proj.to_cols_array_2d().iter().flatten() {
                ubo.extend_from_slice(&v.to_ne_bytes());
            }
            // light_dir (vec4, offset 128) — normalized, pointing down-right
            let light_dir = Vec3::new(0.5, -0.707, 0.5).normalize();
            for v in &[light_dir.x, light_dir.y, light_dir.z, 0.0f32] {
                ubo.extend_from_slice(&v.to_ne_bytes());
            }
            // light_color (vec4, offset 144) — bright white, intensity 1.5
            for v in &[1.5f32, 1.5f32, 1.5f32, 1.5f32] {
                ubo.extend_from_slice(&v.to_ne_bytes());
            }
            // camera_pos (vec4, offset 160)
            for v in &[eye.x, eye.y, eye.z, 1.0f32] {
                ubo.extend_from_slice(&v.to_ne_bytes());
            }

            self.device.write_ubo_current(&ubo, 0);

            // ── Render ────────────────────────────────────────────────
            let stats = match self.device.render_model_frame(
                self.vertex_buf,
                self.index_buf,
                self.index_count,
            ) {
                Ok(s) => s,
                Err(err) => {
                    return Err(vec![Diagnostic::new(
                        "RV0099",
                        DiagnosticSeverity::Error,
                        "sandbox",
                        &format!("model frame failed: {err}"),
                    )]);
                }
            };
            // Screenshot after first successful render (swapchain exists)
            if !self.saved_screenshot {
                self.saved_screenshot = true;
                use render_core::Device;
                if let Err(e) = engine_renderer::screenshot::save_framebuffer(
                    &mut self.device,
                    std::path::Path::new("screenshot.png"),
                    0,
                    0,
                    self.width as u32,
                    self.height as u32,
                ) {
                    tracing::warn!("screenshot failed: {e}");
                }
            }
            Ok(FrameStats {
                draw_calls: stats.draw_calls,
                triangles: stats.triangles,
                visible_drawables: 1,
                ..FrameStats::default()
            })
        }
    }

    // ── WindowApp ─────────────────────────────────────────────────────────
    struct ModelViewerApp {
        renderer: Option<Renderer>,
        frames: u64,
        max_frames: Option<u64>,
        mesh: Option<engine_asset::mesh::MeshData>,
        last_frame_time: std::time::Instant,
    }

    impl WindowApp for ModelViewerApp {
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

            let mut device: VulkanDevice = match VulkanDevice::new(
                display_handle,
                window_handle,
                size.width.max(1),
                size.height.max(1),
                enable_validation,
                Some(std::path::Path::new("./pso_cache")),
            ) {
                Ok(d) => d,
                Err(err) => {
                    tracing::error!(error = %err, "VulkanDevice creation failed");
                    std::process::exit(1);
                }
            };

            // Set FORWARD shaders.
            device.set_mvp_shaders(
                render_vulkan::shaders_embedded::FORWARD_VERT_SPV,
                render_vulkan::shaders_embedded::FORWARD_FRAG_SPV,
            );

            // ── Build interleaved vertex buffer (cube + ground plane) ─
            let mesh = self.mesh.take().expect("mesh loaded earlier");
            let stride = 32u64; // position(12) + normal(12) + uv(8)

            // Ground plane: a 6×6 quad at y=−1.0, normal +Y
            let plane_verts: [(f32, f32, f32); 4] = [
                (-3.0, -1.0, -3.0),
                (3.0, -1.0, -3.0),
                (3.0, -1.0, 3.0),
                (-3.0, -1.0, 3.0),
            ];
            let plane_uvs: [(f32, f32); 4] = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
            let plane_indices: [u32; 6] = [0, 1, 2, 0, 2, 3];

            let cube_vert_count = mesh.positions.len();
            let plane_vert_offset = cube_vert_count as u32;
            let total_verts = cube_vert_count + plane_verts.len();
            let total_indices = mesh.indices.len() + plane_indices.len();

            let mut vert_bytes: Vec<u8> = Vec::with_capacity(total_verts * stride as usize);
            // Cube vertices
            for i in 0..cube_vert_count {
                let p = mesh.positions[i];
                let n = mesh.normals[i];
                let uv = mesh.uvs.get(i).copied().unwrap_or(Vec2::ZERO);
                vert_bytes.extend_from_slice(&p.x.to_ne_bytes());
                vert_bytes.extend_from_slice(&p.y.to_ne_bytes());
                vert_bytes.extend_from_slice(&p.z.to_ne_bytes());
                vert_bytes.extend_from_slice(&n.x.to_ne_bytes());
                vert_bytes.extend_from_slice(&n.y.to_ne_bytes());
                vert_bytes.extend_from_slice(&n.z.to_ne_bytes());
                vert_bytes.extend_from_slice(&uv.x.to_ne_bytes());
                vert_bytes.extend_from_slice(&uv.y.to_ne_bytes());
            }
            // Plane vertices (normal = 0, 1, 0)
            for (i, &(px, py, pz)) in plane_verts.iter().enumerate() {
                let uv = plane_uvs[i];
                vert_bytes.extend_from_slice(&px.to_ne_bytes());
                vert_bytes.extend_from_slice(&py.to_ne_bytes());
                vert_bytes.extend_from_slice(&pz.to_ne_bytes());
                let one: f32 = 1.0;
                let zero: f32 = 0.0;
                vert_bytes.extend_from_slice(&zero.to_ne_bytes());
                vert_bytes.extend_from_slice(&one.to_ne_bytes());
                vert_bytes.extend_from_slice(&zero.to_ne_bytes());
                vert_bytes.extend_from_slice(&uv.0.to_ne_bytes());
                vert_bytes.extend_from_slice(&uv.1.to_ne_bytes());
            }

            let vb_desc = BufferDescriptor {
                size_bytes: vert_bytes.len() as u64,
                usage_flags: render_core::BufferUsage(0),
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some("model-vertices".into()),
            };
            let vertex_buf = match device.create_buffer(&vb_desc) {
                Ok(b) => b,
                Err(err) => {
                    tracing::error!(error = ?err, "failed to create vertex buffer");
                    std::process::exit(1);
                }
            };
            if let Err(err) = device.write_buffer(vertex_buf, &vert_bytes, 0) {
                tracing::error!(error = ?err, "failed to write vertex buffer");
                std::process::exit(1);
            }

            // ── Build index buffer (cube + plane, plane indices offset) ─
            let mut idx_bytes: Vec<u8> = Vec::with_capacity(total_indices * 4);
            for i in &mesh.indices {
                idx_bytes.extend_from_slice(&i.to_ne_bytes());
            }
            for i in plane_indices {
                idx_bytes.extend_from_slice(&(i + plane_vert_offset).to_ne_bytes());
            }
            let ib_desc = BufferDescriptor {
                size_bytes: idx_bytes.len() as u64,
                usage_flags: render_core::BufferUsage(0),
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some("model-indices".into()),
            };
            let index_buf = match device.create_buffer(&ib_desc) {
                Ok(b) => b,
                Err(err) => {
                    tracing::error!(error = ?err, "failed to create index buffer");
                    std::process::exit(1);
                }
            };
            if let Err(err) = device.write_buffer(index_buf, &idx_bytes, 0) {
                tracing::error!(error = ?err, "failed to write index buffer");
                std::process::exit(1);
            }

            let index_count = total_indices as u32;

            let backend = ModelViewerBackend {
                device,
                vertex_buf,
                index_buf,
                index_count,
                camera_angle: 0.0,
                width: size.width.max(1) as f32,
                height: size.height.max(1) as f32,
                saved_screenshot: false,
            };

            let mut renderer = Renderer::new();
            renderer.set_backend(Box::new(backend));
            self.renderer = Some(renderer);
            tracing::info!("model-viewer renderer initialized");
        }

        fn on_event(&mut self, _window: &Window, event: PlatformEvent) -> EventFlow {
            match event {
                PlatformEvent::Resized { .. } => EventFlow::Continue,
                PlatformEvent::Redraw => {
                    // FPS limiter: target ~60 FPS
                    let elapsed = self.last_frame_time.elapsed();
                    let target_frame_time = std::time::Duration::from_secs_f64(1.0 / 60.0);
                    if elapsed < target_frame_time {
                        std::thread::sleep(target_frame_time - elapsed);
                    }
                    self.last_frame_time = std::time::Instant::now();

                    if let Some(ref mut renderer) = self.renderer {
                        let mut input = RenderFrameInput::empty(self.frames);
                        input.views.push(engine_renderer::RenderView {
                            view_id: 0,
                            camera_entity: None,
                            viewport: engine_renderer::Rect::FULL,
                            viewport_rect_normalized: engine_renderer::Rect::FULL,
                            view_matrix: engine_renderer::IDENTITY_MAT4,
                            projection_matrix: engine_renderer::IDENTITY_MAT4,
                            clear_flags: engine_renderer::ClearFlags::ColorAndDepth,
                            clear_color: [0.02, 0.02, 0.06, 1.0],
                            render_layer_mask: u32::MAX,
                            msaa_samples: 1,
                            compose: engine_renderer::ViewCompose::Base {
                                clear: engine_renderer::ClearFlags::ColorAndDepth,
                                clear_color: [0.02, 0.02, 0.06, 1.0],
                            },
                            stack_order: 0,
                            frustum: None,
                        });
                        match renderer.draw_scene(&input) {
                            Ok(stats) => {
                                tracing::info!(
                                    draw_calls = stats.draw_calls,
                                    triangles = stats.triangles,
                                    "model-viewer frame"
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
                _ => EventFlow::Continue,
            }
        }
    }

    let max_frames = parse_frame_limit();

    let app = ModelViewerApp {
        renderer: None,
        frames: 0,
        max_frames,
        mesh: Some(mesh),
        last_frame_time: std::time::Instant::now(),
    };
    if let Err(err) = platform::run(
        WindowDescriptor {
            title: "Engine Sandbox - Model Viewer".to_string(),
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
fn run_model_viewer() {
    tracing::error!("model-viewer requires `backend-vulkan` feature");
    std::process::exit(2);
}

// ============================================================================
// engine-model-viewer: glTF model viewer using the standard engine pipeline
// (GameLoop → EngineRuntime → SceneRenderer → VulkanDevice).
// ============================================================================

#[cfg(feature = "backend-vulkan")]
fn run_engine_model_viewer() {
    use std::sync::Arc;
    use std::time::Instant;

    use engine_asset::gltf::load_gltf_scene;
    use engine_asset::mesh::mesh_data_to_color_bytes;
    use engine_core::game_loop::GameLoop;
    use engine_core::EngineConfig;
    use engine_scene::components::{Camera, Renderable, Transform};
    use engine_scene::Entity;
    use engine_scene::World;
    use glam::Vec3;
    use platform::winit::window::Window;
    use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use render_vulkan::device_impl::VulkanDevice;
    use render_vulkan::scene_renderer::SceneRenderer;

    struct EngineModelViewerApp {
        game_loop: Option<GameLoop>,
        frames: u64,
        last_frame_time: Instant,
        camera_angle: f32,
    }

    impl WindowApp for EngineModelViewerApp {
        fn on_create(&mut self, window: Arc<Window>) {
            let size = window.inner_size();
            let w = size.width.max(1);
            let h = size.height.max(1);

            // ── CLI argument: model path ──────────────────────────────
            let model_path = std::env::args().skip(2).find(|a| !a.starts_with("--"));
            let model_path = model_path.unwrap_or_else(|| "assets/models/DamagedHelmet.gltf".into());

            // ── Load glTF ─────────────────────────────────────────────
            let gltf_scene = match load_gltf_scene(std::path::Path::new(&model_path)) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("glTF load failed: {e}");
                    return;
                }
            };
            tracing::info!(
                meshes = gltf_scene.meshes.len(),
                materials = gltf_scene.materials.len(),
                textures = gltf_scene.textures.len(),
                nodes = gltf_scene.nodes.len(),
                "glTF scene loaded"
            );

            // ── Create device + renderer ──────────────────────────────
            let device = match VulkanDevice::new(
                window.display_handle().unwrap().as_raw(),
                window.window_handle().unwrap().as_raw(),
                w, h,
                cfg!(debug_assertions),
                None,
            ) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("VulkanDevice: {e}");
                    return;
                }
            };
            let scene_renderer = SceneRenderer::new(device, w, h);

            let mut game_loop = GameLoop::new(EngineConfig {
                application_name: "engine-model-viewer".into(),
            });
            game_loop.runtime.renderer_mut().set_backend(Box::new(scene_renderer));

            // ── Build ECS world ───────────────────────────────────────
            let mut world = World::new();

            // Camera entity
            let camera = world.create_entity();
            world.add_component(camera, Transform {
                translation: Vec3::new(0.0, 2.0, 5.0),
                ..Transform::default()
            });
            world.add_component(camera, Camera::default());

            // Upload each mesh and create a renderable entity for each node.
            for (i, mesh_data) in gltf_scene.meshes.iter().enumerate() {
                let mesh_id = format!("mesh-{i}");
                let (vb, ib, ic, iu16) = mesh_data_to_color_bytes(mesh_data);
                let _ = game_loop.runtime.renderer_mut().upload_mesh(&mesh_id, &vb, &ib, ic, iu16);

                let entity = world.create_entity();
                // Use the node's world transform if available.
                let pos = gltf_scene.nodes.get(i).map(|n| {
                    let (_, _, t) = n.transform.to_scale_rotation_translation();
                    t
                }).unwrap_or(Vec3::ZERO);
                world.add_component(entity, Transform {
                    translation: pos,
                    ..Transform::default()
                });
                world.add_component(entity, Renderable {
                    mesh_asset: mesh_id,
                    material_asset: "default".into(),
                    visible: true,
                    cast_shadows: true,
                    render_layer: "default".into(),
                });
            }

            game_loop.runtime.set_world(world);
            self.game_loop = Some(game_loop);
        }

        fn on_event(&mut self, _window: &Window, event: PlatformEvent) -> EventFlow {
            match event {
                PlatformEvent::Redraw => {
                    let dt = self.last_frame_time.elapsed().as_secs_f32();
                    self.last_frame_time = Instant::now();
                    self.camera_angle += dt * 0.3;

                    if let Some(ref mut gl) = self.game_loop {
                        // Rotate orbit camera around origin.
                        let dist = 5.0f32;
                        let eye = Vec3::new(
                            self.camera_angle.sin() * dist,
                            2.0,
                            self.camera_angle.cos() * dist,
                        );
                        if let Some(w) = gl.runtime.world_mut() {
                            // Find the first (camera) entity by iterating
                            let cameras: Vec<Entity> = w.query::<Camera>()
                                .map(|(e, _)| e)
                                .collect();
                            for ce in cameras {
                                if let Some(t) = w.get_mut::<Transform>(ce) {
                                    let dir = (-eye).normalize();
                                    t.translation = eye;
                                    t.rotation = glam::Quat::from_rotation_arc(-Vec3::Z, dir);
                                }
                            }
                        }
                        if let Err(errs) = gl.render(self.frames) {
                            for d in &errs {
                                tracing::warn!(code = d.code, "render: {}", d.message);
                            }
                        }
                    }
                    _window.request_redraw();
                    self.frames += 1;
                }
                PlatformEvent::Resized { width, height } => {
                    if let Some(ref mut gl) = self.game_loop {
                        let _ = gl.runtime.renderer_mut().resize(width, height);
                    }
                }
                PlatformEvent::CloseRequested => return EventFlow::Exit,
                _ => {}
            }
            EventFlow::Continue
        }
    }

    let app = EngineModelViewerApp {
        game_loop: None,
        frames: 0,
        last_frame_time: Instant::now(),
        camera_angle: 0.0,
    };
    if let Err(e) = platform::run(
        WindowDescriptor {
            title: "Engine Model Viewer".into(),
            width: 1280,
            height: 720,
        },
        app,
    ) {
        tracing::error!("{e}");
    }
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_engine_model_viewer() {
    tracing::error!("engine-model-viewer requires `backend-vulkan` feature");
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
                _ => EventFlow::Continue,
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
