//! Pluggable render pass trait and registry.
//!
//! Provides an abstract [`RenderPass`] trait that backends implement to
//! add custom rendering passes to the render graph, and a [`PassRegistry`]
//! for registering and looking up passes by their string kind.
//!
//! The canonical pass kinds used by the built-in graph builder are:
//!
//! | `kind()` return value       | Graph node              |
//! |-----------------------------|-------------------------|
//! | `"directional_shadow_pass"` | `PassKind::DirectionalShadow` |
//! | `"opaque_pbr_forward_pass"` | `PassKind::OpaquePbrForward`  |
//! | `"tone_map_pass"`           | `PassKind::ToneMap`           |
//! | `"present"`                 | `PassKind::Present`           |
//! | `"ssao"`                    | `PassKind::Custom("ssao")`    |
//! | `"bloom"`                   | `PassKind::Custom("bloom")`   |
//!
//! Backends that implement pass-level execution override
//! [`BackendRenderer::execute_pass`] and route to the registry.

use crate::render_graph;
use crate::{Diagnostic, FrameStats, RenderFrameInput};
use render_core::{CommandEncoder, Device};

/// A single pluggable render pass.
///
/// Each pass is identified by [`kind`](Self::kind) which must match the
/// `PassKind::name()` value of the corresponding graph node so that
/// [`PassRegistry::find`] can discover it at execution time.
pub trait RenderPass: Send {
    /// Machine-readable identifier — must match [`PassKind::name`].
    fn kind(&self) -> &'static str;

    /// Build a [`PassNode`](render_graph::PassNode) declaration that will be
    /// inserted into the render graph for the given view.
    fn declare(&self, view_id: u32) -> render_graph::PassNode;

    /// Prepare device resources (pipelines, descriptor sets, …).
    ///
    /// Called once during initialisation, *before* any frame recording.
    /// The `device` argument is the backend's abstract device.
    fn prepare(&mut self, device: &mut dyn Device) -> Result<(), Vec<Diagnostic>>;

    /// Execute this pass for the current frame.
    ///
    /// Called once per frame during the render-graph traversal, after the
    /// graph compiler has inserted the necessary resource barriers.
    fn execute(
        &mut self,
        input: &RenderFrameInput,
        encoder: &mut dyn CommandEncoder,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>>;

    /// Whether this pass is enabled for the given frame input.
    ///
    /// Return `false` to skip the pass without producing an error.
    /// The default implementation always returns `true`.
    fn is_enabled(&self, _input: &RenderFrameInput) -> bool {
        true
    }
}

/// A registry of pluggable render passes.
///
/// Passes are registered once during backend initialisation and are looked
/// up by their string kind when the render graph is traversed.
pub struct PassRegistry {
    passes: Vec<Box<dyn RenderPass>>,
}

impl PassRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    /// Register a pass.
    ///
    /// The pass is stored in insertion order.  [`find`](Self::find) and
    /// [`find_mut`](Self::find_mut) search linearly, so the number of
    /// registered passes should remain small (< 20).
    pub fn register(&mut self, pass: Box<dyn RenderPass>) {
        self.passes.push(pass);
    }

    /// Find a registered pass by its [`kind`](RenderPass::kind).
    pub fn find(&self, kind: &str) -> Option<&dyn RenderPass> {
        self.passes.iter().find(|p| p.kind() == kind).map(|p| p.as_ref())
    }

    /// Find a registered pass by its [`kind`](RenderPass::kind) (mutable).
    pub fn find_mut(&mut self, kind: &str) -> Option<&mut dyn RenderPass> {
        // Manual loop with explicit lifetime — the closure-based version
        // struggles with lifetime inference for trait-object returns.
        for p in &mut self.passes {
            if p.kind() == kind {
                return Some(p.as_mut());
            }
        }
        None
    }
}

impl Default for PassRegistry {
    fn default() -> Self {
        Self::new()
    }
}
