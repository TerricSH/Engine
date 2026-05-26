//! Minimal render graph for Gate 3.
//!
//! Defines the canonical pass ordering:
//!   directional_shadow_pass → opaque_pbr_forward_pass → tone_map_pass → present
//!
//! Each pass is recorded with a `tracing` span so per-pass timing and
//! diagnostics are visible in the log. The graph is rebuilt every frame
//! from `RenderFrameInput` and executed by the renderer backend.

use crate::{RenderFrameInput, RenderView, ViewCompose};

/// The kinds of passes in the Gate 3 render graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PassKind {
    DirectionalShadow,
    OpaquePbrForward,
    ToneMap,
    Present,
}

impl PassKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::DirectionalShadow => "directional_shadow_pass",
            Self::OpaquePbrForward => "opaque_pbr_forward_pass",
            Self::ToneMap => "tone_map_pass",
            Self::Present => "present",
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

            // 3. Tone-map pass
            passes.push(PassNode {
                kind: PassKind::ToneMap,
                name: "tone_map_pass",
                view_id: view.view_id,
                reads_depth: false,
                writes_swapchain: false,
            });

            // 4. Present
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
