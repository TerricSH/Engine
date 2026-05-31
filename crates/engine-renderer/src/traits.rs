use crate::render_graph::CompiledBarrier;
use crate::{
    render_graph, validate_frame_input, Diagnostic, DiagnosticSeverity, FrameStats,
    RenderFrameInput,
};

/// Backend renderer trait — implemented by concrete rendering backends
/// (Vulkan, OpenGL, DX12) to bridge scene input to GPU execution.
pub trait BackendRenderer: Send {
    /// Render one frame from the given scene input (legacy single-pass path).
    fn render_frame(&mut self, input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>>;

    /// Begin a new frame. Called once before [`execute_pass`](Self::execute_pass).
    /// Default: no-op, rendering happens in render_frame.
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    /// End the current frame. Called once after all passes.
    /// Default: no-op.
    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    /// Apply graph-compiled resource barriers before a pass executes.
    /// Backends that rely on render-pass subpass layout transitions can
    /// ignore barriers they do not own.
    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &render_graph::PassNode,
        _barriers: &[CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    /// Execute a single render-graph pass. The default implementation
    /// delegates to [`render_frame`](Self::render_frame) for backwards compat.
    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &render_graph::PassNode,
        frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        let _ = pass;
        let _ = frame_stats;
        self.render_frame(input).map(|_| ())
    }
}

pub struct Renderer {
    backend: Option<Box<dyn BackendRenderer>>,
}

impl Renderer {
    pub fn new() -> Self {
        Self { backend: None }
    }

    pub fn new_with_backend(backend: Box<dyn BackendRenderer>) -> Self {
        Self {
            backend: Some(backend),
        }
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn set_backend(&mut self, backend: Box<dyn BackendRenderer>) {
        self.backend = Some(backend);
    }

    /// Render a frame by building the render graph and executing each pass.
    pub fn draw_scene(&mut self, input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        let diagnostics = validate_frame_input(input);
        if diagnostics.iter().any(|d| {
            matches!(
                d.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        }) {
            return Err(diagnostics);
        }

        if let Some(backend) = self.backend.as_mut() {
            // Build the render graph from the frame input (DAG-based builder)
            let graph = crate::render_graph2::RenderGraph::build_with_config(
                input,
                &input.render_options.pass_graph_config,
            );

            // Phase B: compile the graph (topological sort, cull dead passes,
            // infer barriers).
            let compiled = graph.compile_v2().map_err(|err| {
                vec![Diagnostic::new(
                    "RV0020",
                    DiagnosticSeverity::Error,
                    "renderer.render_graph",
                    format!("render graph compile_v2 failed: {err}"),
                )]
            })?;

            let mut stats = FrameStats::default();

            // Begin frame (backend allocates per-frame resources)
            backend.begin_frame(input)?;

            // Execute each live pass with tracing spans.
            for (compiled_idx, &pass_idx) in compiled.pass_order.iter().enumerate() {
                let Some(pass) = graph.passes.get(pass_idx) else {
                    return Err(vec![Diagnostic::new(
                        "RV0021",
                        DiagnosticSeverity::Error,
                        "renderer.render_graph",
                        format!("compiled graph referenced missing pass index {pass_idx}"),
                    )]);
                };
                let barriers = compiled
                    .barriers_per_pass
                    .get(compiled_idx)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let span = tracing::info_span!("frame.view.{}.{}", input.frame_index, pass.name);
                let _guard = span.enter();
                tracing::info!(pass = pass.name, "executing render pass");

                // Convert to legacy PassNode for the backend trait.
                let legacy = pass.to_legacy();
                if let Some(ref legacy_pass) = legacy {
                    backend.apply_pass_barriers(input, legacy_pass, barriers)?;
                    backend.execute_pass(input, legacy_pass, &mut stats)?;
                }
            }

            // End frame (backend submits and presents)
            backend.end_frame(&mut stats)?;

            Ok(stats)
        } else {
            // No backend attached — return mock stats (for contract-only testing)
            Ok(FrameStats {
                visible_drawables: input.drawables.len() as u32 + input.skinned_items.len() as u32,
                visible_lights: input.lights.len() as u32,
                draw_calls: input.drawables.len() as u32 + input.skinned_items.len() as u32,
                ..FrameStats::default()
            })
        }
    }
}
