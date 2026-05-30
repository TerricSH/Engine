//! Minimal render graph for Gate 3.
//!
//! Defines the canonical pass ordering:
//!   directional_shadow_pass → opaque_pbr_forward_pass → ssao_pass → bloom_pass → tone_map_pass → present
//!
//! Each pass is recorded with a `tracing` span so per-pass timing and
//! diagnostics are visible in the log. The graph is rebuilt every frame
//! from `RenderFrameInput` and executed by the renderer backend.

use crate::{RenderFrameInput, RenderView, ViewCompose};

// Re-export v2 render-graph types so that backend passes written against
// the new-style `PassAttachment` / `ResourceAccess` / `SizeSource` can
// import them from `engine_renderer::render_graph` without change.
#[doc(inline)]
pub use crate::render_graph2::{PassAttachment, ResourceAccess, SizeSource};

// ============================================================================
// CompiledRenderGraph — output of render-graph compilation
// ============================================================================

/// Backend-agnostic state that a resource can be in.  The Vulkan backend maps
/// these to `VkImageLayout` values when inserting pipeline barriers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceState {
    Undefined,
    ColorAttachmentOptimal,
    DepthStencilAttachmentOptimal,
    DepthStencilReadOnlyOptimal,
    ShaderReadOnlyOptimal,
    TransferSrcOptimal,
    TransferDstOptimal,
    PresentSrc,
    General,
}

/// Backend-agnostic pipeline stage flags, used to describe when a barrier's
/// source / destination work executes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeStage {
    TopOfPipe,
    ColorAttachmentOutput,
    EarlyFragmentTests,
    LateFragmentTests,
    FragmentShader,
    ComputeShader,
    Transfer,
    BottomOfPipe,
}

/// A single pipeline-barrier command produced by the graph compiler.
#[derive(Clone, Debug)]
pub struct CompiledBarrier {
    pub resource_name: String,
    pub src_stage: PipeStage,
    pub dst_stage: PipeStage,
    pub old_state: ResourceState,
    pub new_state: ResourceState,
}

/// The result of compiling a [`RenderGraph`](super::render_graph2::RenderGraph).
///
/// Contains the topologically-sorted pass execution order and a list of
/// pipeline barriers that must be inserted *before* each pass to ensure
/// correct resource transitions.
#[derive(Clone, Debug)]
pub struct CompiledRenderGraph {
    /// Indices into the original `passes` array, in submission order.
    pub pass_order: Vec<usize>,
    /// Barriers to apply before each pass (indexed by position in
    /// `pass_order`).
    pub barriers_per_pass: Vec<Vec<CompiledBarrier>>,
}

impl CompiledRenderGraph {
    /// Number of passes in the compiled graph.
    pub fn pass_count(&self) -> usize {
        self.pass_order.len()
    }
}

// ============================================================================
// PassKind (legacy)
// ============================================================================

/// The kinds of passes in the Gate 3 render graph.
///
/// The `Custom` variant allows backend-specific pass types (e.g. bloom,
/// SSAO) to be dispatched by name without modifying the front-end graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PassKind {
    DirectionalShadow,
    OpaquePbrForward,
    ToneMap,
    Present,
    Custom(&'static str),
}

impl PassKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::DirectionalShadow => "directional_shadow_pass",
            Self::OpaquePbrForward => "opaque_pbr_forward_pass",
            Self::ToneMap => "tone_map_pass",
            Self::Present => "present",
            Self::Custom(name) => name,
        }
    }
}

/// A single node in the render graph.
#[derive(Clone, Debug)]
pub struct PassNode {
    pub kind: PassKind,
    /// Human-readable name for debugging and tracing.
    pub name: &'static str,
    /// Which view this pass belongs to (0 for single-view scenes).
    pub view_id: u32,
    /// Whether this pass has a depth pre-pass dependency.
    pub reads_depth: bool,
    /// Whether this pass writes to the swapchain directly.
    pub writes_swapchain: bool,
}

/// The per-frame render graph.
#[derive(Clone, Debug)]
pub struct RenderGraph {
    pub passes: Vec<PassNode>,
}

impl RenderGraph {
    /// Phase 5.4: Compile this legacy render graph.
    ///
    /// Because the legacy graph has no edge or resource-usage metadata,
    /// the compiled order is simply sequential (0, 1, …, n-1) with no
    /// barriers.  Backends that need proper barrier insertion should use
    /// [`render_graph2::RenderGraph::compile`] instead.
    pub fn compile(&self) -> CompiledRenderGraph {
        let n = self.passes.len();
        let pass_order: Vec<usize> = (0..n).collect();
        let barriers_per_pass: Vec<Vec<CompiledBarrier>> = vec![Vec::new(); n];
        CompiledRenderGraph {
            pass_order,
            barriers_per_pass,
        }
    }

    /// Build the canonical Gate 3 render graph from frame input.
    ///
    /// For each active view, the graph contains:
    /// 1. `directional_shadow_pass` — only if the view has shadow-casting lights
    /// 2. `opaque_pbr_forward_pass` — main forward shading
    /// 3. `tone_map_pass` — HDR → swapchain tone-mapping
    /// 4. `present` — swapchain present
    pub fn build(input: &RenderFrameInput) -> Self {
        let mut passes = Vec::new();

        // Collect all active views
        let views: Vec<&RenderView> = input
            .views
            .iter()
            .filter(|v| {
                // Skip overlay views that reference missing base views
                if let ViewCompose::Overlay { base_view_id, .. } = &v.compose {
                    input.views.iter().any(|bv| bv.view_id == *base_view_id)
                } else {
                    true
                }
            })
            .collect();

        for view in &views {
            let has_shadow_casters = input.lights.iter().any(|l| {
                matches!(
                    l.shadow_mode,
                    crate::ShadowMode::Hard | crate::ShadowMode::Soft
                )
            });

            // 1. Directional shadow pass (only if shadows are needed)
            if has_shadow_casters {
                passes.push(PassNode {
                    kind: PassKind::DirectionalShadow,
                    name: "directional_shadow_pass",
                    view_id: view.view_id,
                    reads_depth: false,
                    writes_swapchain: false,
                });
            }

            // 2. Opaque forward pass
            passes.push(PassNode {
                kind: PassKind::OpaquePbrForward,
                name: "opaque_pbr_forward_pass",
                view_id: view.view_id,
                reads_depth: true,
                writes_swapchain: false,
            });

            // 3. SSAO pass (ambient occlusion)
            passes.push(PassNode {
                kind: PassKind::Custom("ssao"),
                name: "ssao_pass",
                view_id: view.view_id,
                reads_depth: true,
                writes_swapchain: false,
            });

            // 4. Bloom pass (brightness extraction + blur)
            passes.push(PassNode {
                kind: PassKind::Custom("bloom"),
                name: "bloom_pass",
                view_id: view.view_id,
                reads_depth: false,
                writes_swapchain: false,
            });

            // 5. Tone-map pass
            passes.push(PassNode {
                kind: PassKind::ToneMap,
                name: "tone_map_pass",
                view_id: view.view_id,
                reads_depth: false,
                writes_swapchain: false,
            });

            // 6. Present
            passes.push(PassNode {
                kind: PassKind::Present,
                name: "present",
                view_id: view.view_id,
                reads_depth: false,
                writes_swapchain: true,
            });
        }

        Self { passes }
    }

    /// Number of passes in this graph.
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }
}
