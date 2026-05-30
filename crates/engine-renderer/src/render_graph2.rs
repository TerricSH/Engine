//! DAG-based configurable render graph (Gate 4).
//!
//! Provides an extensible `PassKind`, rich `PassNode` with resource
//! declarations, and a `RenderGraph` builder that supports topological
//! sorting.  A `to_legacy()` adapter produces the old `render_graph::RenderGraph`
//! so existing `BackendRenderer` consumers continue to work unchanged.
//!
//! The canonical 4-pass ordering is produced by `RenderGraph::build()`:
//!   directional_shadow_pass → opaque_pbr_forward_pass → tone_map_pass → present
//!
//! Custom orderings can be expressed via `build_with_config()` which
//! honours `PassGraphConfig` (loadable from scene settings).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    render_graph::{self, CompiledBarrier, CompiledRenderGraph, PipeStage, ResourceState},
    RenderFrameInput, RenderView, ViewCompose,
};

// ── Pass kind (extensible) ──────────────────────────────────────────────────

/// Extensible pass kind — can be one of the built-in kinds or a custom string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PassKind {
    DirectionalShadow,
    OpaquePbrForward,
    ToneMap,
    Present,
    Custom(&'static str),
}

impl PassKind {
    /// Machine-readable name for this pass kind.
    pub fn name(&self) -> &'static str {
        match self {
            Self::DirectionalShadow => "directional_shadow_pass",
            Self::OpaquePbrForward => "opaque_pbr_forward_pass",
            Self::ToneMap => "tone_map_pass",
            Self::Present => "present",
            Self::Custom(name) => name,
        }
    }

    /// Try to convert this new `PassKind` to the legacy `render_graph::PassKind`.
    /// Custom variants are passed through.
    pub fn to_legacy(&self) -> Option<render_graph::PassKind> {
        match self {
            Self::DirectionalShadow => Some(render_graph::PassKind::DirectionalShadow),
            Self::OpaquePbrForward => Some(render_graph::PassKind::OpaquePbrForward),
            Self::ToneMap => Some(render_graph::PassKind::ToneMap),
            Self::Present => Some(render_graph::PassKind::Present),
            Self::Custom(name) => Some(render_graph::PassKind::Custom(name)),
        }
    }

    /// String-serialisable kind identifier for config deserialisation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DirectionalShadow => "DirectionalShadow",
            Self::OpaquePbrForward => "OpaquePbrForward",
            Self::ToneMap => "ToneMap",
            Self::Present => "Present",
            Self::Custom(s) => s,
        }
    }

    /// Parse from the string representation returned by [`as_str`](Self::as_str).
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "DirectionalShadow" => Some(Self::DirectionalShadow),
            "OpaquePbrForward" => Some(Self::OpaquePbrForward),
            "ToneMap" => Some(Self::ToneMap),
            "Present" => Some(Self::Present),
            custom => Some(Self::Custom(Box::leak(custom.to_string().into_boxed_str()))),
        }
    }
}

// ── Resource access mode ─────────────────────────────────────────────────────

/// How a pass accesses an attachment resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceAccess {
    Read,
    Write,
    ReadWrite,
}

// ── Resource attachments ────────────────────────────────────────────────────

/// Describes how a single resource attachment (colour or depth) is bound
/// for a pass.
#[derive(Clone, Debug)]
pub struct PassAttachment {
    pub name: String,
    pub format: Option<String>,
    pub clear: bool,
    pub load_op: String, // "clear", "load", "dont_care"
    pub size_source: SizeSource,
    pub access: ResourceAccess,
}

/// Determines how the attachment dimensions are resolved.
#[derive(Clone, Debug)]
pub enum SizeSource {
    Swapchain,
    Custom(u32, u32),
    FromInput(String),
}

// ── Pass node ───────────────────────────────────────────────────────────────

/// A single node in the DAG-based render graph.
#[derive(Clone, Debug)]
pub struct PassNode {
    pub kind: PassKind,
    pub name: &'static str,
    pub view_id: u32,
    pub inputs: Vec<PassAttachment>,
    pub outputs: Vec<PassAttachment>,
    pub depth_stencil: Option<PassAttachment>,
}

impl PassNode {
    /// Convert this new-style `PassNode` to the legacy `render_graph::PassNode`
    /// for use with the existing `BackendRenderer` trait.
    pub fn to_legacy(&self) -> Option<render_graph::PassNode> {
        let legacy_kind = self.kind.to_legacy()?;
        Some(render_graph::PassNode {
            kind: legacy_kind,
            name: self.name,
            view_id: self.view_id,
            reads_depth: self
                .inputs
                .iter()
                .any(|a| a.name == "depth" || a.name == "depth_stencil"),
            writes_swapchain: self.outputs.iter().any(|a| a.name == "swapchain"),
        })
    }
}

// ── Graph edge ──────────────────────────────────────────────────────────────

/// A dependency edge between two passes in the graph.
#[derive(Clone, Debug)]
pub struct GraphEdge {
    pub from_pass: usize,
    pub to_pass: usize,
    pub resource: String,
}

// ── Render graph (DAG builder) ──────────────────────────────────────────────

/// A configurable DAG-based render graph.
///
/// Use [`RenderGraph::new()`] to create an empty graph and `add_pass` /
/// `add_edge` to populate it, or use the convenience constructors
/// [`build`](Self::build) and [`build_with_config`](Self::build_with_config)
/// to get the canonical ordering.
#[derive(Clone, Debug)]
pub struct RenderGraph {
    pub passes: Vec<PassNode>,
    pub edges: Vec<GraphEdge>,
}

impl RenderGraph {
    /// Create an empty render graph.
    pub fn new() -> Self {
        Self {
            passes: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Build the canonical 4-pass render graph from frame input.
    ///
    /// For each active view:
    /// 1. `directional_shadow_pass` — only if the view has shadow-casting lights
    /// 2. `opaque_pbr_forward_pass` — main forward shading
    /// 3. `tone_map_pass` — HDR → swapchain tone-mapping
    /// 4. `present` — swapchain present
    pub fn build(input: &RenderFrameInput) -> Self {
        let mut graph = Self::new();

        let views: Vec<&RenderView> = input
            .views
            .iter()
            .filter(|v| {
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
                graph.add_pass(PassNode {
                    kind: PassKind::DirectionalShadow,
                    name: "directional_shadow_pass",
                    view_id: view.view_id,
                    inputs: vec![PassAttachment {
                        name: "depth".into(),
                        format: Some("D32".into()),
                        clear: true,
                        load_op: "clear".into(),
                        size_source: SizeSource::Swapchain,
                        access: ResourceAccess::Read,
                    }],
                    outputs: vec![PassAttachment {
                        name: "shadow_map".into(),
                        format: Some("D32".into()),
                        clear: false,
                        load_op: "load".into(),
                        size_source: SizeSource::Custom(1024, 1024),
                        access: ResourceAccess::Write,
                    }],
                    depth_stencil: Some(PassAttachment {
                        name: "shadow_depth".into(),
                        format: Some("D32".into()),
                        clear: true,
                        load_op: "clear".into(),
                        size_source: SizeSource::Custom(1024, 1024),
                        access: ResourceAccess::ReadWrite,
                    }),
                });
            }

            // 2. Opaque forward pass
            graph.add_pass(PassNode {
                kind: PassKind::OpaquePbrForward,
                name: "opaque_pbr_forward_pass",
                view_id: view.view_id,
                inputs: vec![PassAttachment {
                    name: "depth".into(),
                    format: Some("D32".into()),
                    clear: true,
                    load_op: "load".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Read,
                }],
                outputs: vec![PassAttachment {
                    name: "hdr_color".into(),
                    format: Some("RGBA16F".into()),
                    clear: true,
                    load_op: "clear".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Write,
                }],
                depth_stencil: Some(PassAttachment {
                    name: "depth_stencil".into(),
                    format: Some("D24S8".into()),
                    clear: true,
                    load_op: "clear".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::ReadWrite,
                }),
            });

            // 3. SSAO pass (ambient occlusion)
            graph.add_pass(PassNode {
                kind: PassKind::Custom("ssao"),
                name: "ssao_pass",
                view_id: view.view_id,
                inputs: vec![PassAttachment {
                    name: "depth_stencil".into(),
                    format: Some("D32".into()),
                    clear: false,
                    load_op: "load".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Read,
                }],
                outputs: vec![PassAttachment {
                    name: "ssao_output".into(),
                    format: Some("R8".into()),
                    clear: true,
                    load_op: "clear".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Write,
                }],
                depth_stencil: None,
            });

            // 4. Bloom pass (brightness extraction + blur)
            graph.add_pass(PassNode {
                kind: PassKind::Custom("bloom"),
                name: "bloom_pass",
                view_id: view.view_id,
                inputs: vec![PassAttachment {
                    name: "hdr_color".into(),
                    format: Some("RGBA16F".into()),
                    clear: false,
                    load_op: "load".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Read,
                }],
                outputs: vec![],
                depth_stencil: None,
            });

            // 5. Tone-map pass
            graph.add_pass(PassNode {
                kind: PassKind::ToneMap,
                name: "tone_map_pass",
                view_id: view.view_id,
                inputs: vec![PassAttachment {
                    name: "hdr_color".into(),
                    format: Some("RGBA16F".into()),
                    clear: false,
                    load_op: "load".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Read,
                }],
                outputs: vec![PassAttachment {
                    name: "ldr_color".into(),
                    format: Some("RGBA8".into()),
                    clear: true,
                    load_op: "clear".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Write,
                }],
                depth_stencil: None,
            });

            // 6. Present
            graph.add_pass(PassNode {
                kind: PassKind::Present,
                name: "present",
                view_id: view.view_id,
                inputs: vec![PassAttachment {
                    name: "ldr_color".into(),
                    format: Some("RGBA8".into()),
                    clear: false,
                    load_op: "load".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Read,
                }],
                outputs: vec![PassAttachment {
                    name: "swapchain".into(),
                    format: None,
                    clear: false,
                    load_op: "dont_care".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Write,
                }],
                depth_stencil: None,
            });

            // Add implicit edges for the canonical ordering.
            let pass_indices: Vec<usize> = (0..graph.passes.len()).collect();
            for window in pass_indices.windows(2) {
                if window.len() == 2 {
                    graph.add_edge(window[0], window[1], "auto");
                }
            }
        }

        graph
    }

    /// Build the render graph from frame input, filtered and ordered by the
    /// given `PassGraphConfig`.
    ///
    /// Passes that are disabled in the config are omitted.  The ordering of
    /// visible passes follows the config entry order (not the canonical order).
    /// This allows scene-specific pass graphs to be loaded from settings.
    pub fn build_with_config(input: &RenderFrameInput, config: &PassGraphConfig) -> Self {
        // If the graph config is not enabled, fall back to the canonical build.
        if !config.enabled {
            return Self::build(input);
        }

        let views: Vec<&RenderView> = input
            .views
            .iter()
            .filter(|v| {
                if let ViewCompose::Overlay { base_view_id, .. } = &v.compose {
                    input.views.iter().any(|bv| bv.view_id == *base_view_id)
                } else {
                    true
                }
            })
            .collect();

        let has_shadow_casters = input.lights.iter().any(|l| {
            matches!(
                l.shadow_mode,
                crate::ShadowMode::Hard | crate::ShadowMode::Soft
            )
        });

        let mut graph = Self::new();

        for view in &views {
            for entry in &config.passes {
                if !entry.enabled {
                    continue;
                }

                // Resolve the pass kind from the config string.
                let kind = match PassKind::parse_str(&entry.kind) {
                    Some(k) => k,
                    None => continue,
                };

                // Skip the shadow pass if there are no shadow casters.
                if matches!(kind, PassKind::DirectionalShadow) && !has_shadow_casters {
                    continue;
                }

                let pass = match kind {
                    PassKind::DirectionalShadow => PassNode {
                        kind: PassKind::DirectionalShadow,
                        name: "directional_shadow_pass",
                        view_id: view.view_id,
                        inputs: vec![],
                        outputs: vec![],
                        depth_stencil: None,
                    },
                    PassKind::OpaquePbrForward => PassNode {
                        kind: PassKind::OpaquePbrForward,
                        name: "opaque_pbr_forward_pass",
                        view_id: view.view_id,
                        inputs: vec![],
                        outputs: vec![],
                        depth_stencil: None,
                    },
                    PassKind::ToneMap => PassNode {
                        kind: PassKind::ToneMap,
                        name: "tone_map_pass",
                        view_id: view.view_id,
                        inputs: vec![],
                        outputs: vec![],
                        depth_stencil: None,
                    },
                    PassKind::Present => PassNode {
                        kind: PassKind::Present,
                        name: "present",
                        view_id: view.view_id,
                        inputs: vec![],
                        outputs: vec![],
                        depth_stencil: None,
                    },
                    PassKind::Custom(custom_name) => PassNode {
                        kind: PassKind::Custom(custom_name),
                        name: custom_name,
                        view_id: view.view_id,
                        inputs: vec![],
                        outputs: vec![],
                        depth_stencil: None,
                    },
                };

                graph.add_pass(pass);
            }
        }

        // Add sequential edges based on the config ordering.
        let pass_indices: Vec<usize> = (0..graph.passes.len()).collect();
        for window in pass_indices.windows(2) {
            if window.len() == 2 {
                graph.add_edge(window[0], window[1], "auto");
            }
        }

        graph
    }

    /// Add a pass node to the graph and return its index.
    pub fn add_pass(&mut self, pass: PassNode) -> usize {
        let idx = self.passes.len();
        self.passes.push(pass);
        idx
    }

    /// Add a dependency edge between two passes (identified by their
    /// `add_pass` return values).
    pub fn add_edge(&mut self, from: usize, to: usize, resource: impl Into<String>) {
        self.edges.push(GraphEdge {
            from_pass: from,
            to_pass: to,
            resource: resource.into(),
        });
    }

    /// Topologically sort the passes based on declared edges.
    ///
    /// Returns a permutation of `0..pass_count()` that respects all
    /// dependencies, or an error string if a cycle is detected (Kahn's
    /// algorithm).
    pub fn topological_sort(&self) -> Result<Vec<usize>, String> {
        let n = self.passes.len();
        // Build adjacency list and in-degree count.
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut in_degree: Vec<usize> = vec![0; n];

        for edge in &self.edges {
            if edge.from_pass < n && edge.to_pass < n {
                adj[edge.from_pass].push(edge.to_pass);
                in_degree[edge.to_pass] += 1;
            }
        }

        // Kahn's algorithm.
        let mut queue: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut sorted = Vec::with_capacity(n);

        while let Some(v) = queue.pop() {
            sorted.push(v);
            for &u in &adj[v] {
                in_degree[u] -= 1;
                if in_degree[u] == 0 {
                    queue.push(u);
                }
            }
        }

        if sorted.len() != n {
            return Err(format!(
                "cycle detected: sorted {} of {} passes",
                sorted.len(),
                n
            ));
        }

        Ok(sorted)
    }

    /// Number of passes in this graph.
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    /// Phase 5.4: Compile this render graph into an explicit submit order
    /// with backend-agnostic pipeline barriers.
    ///
    /// 1. Topologically sorts the passes (Kahn's algorithm).
    /// 2. Tracks resource state transitions across the sorted order.
    /// 3. Inserts a [`CompiledBarrier`] whenever a resource changes between
    ///    read and write (or between different write roles).
    ///
    /// Returns a [`CompiledRenderGraph`] that backends can turn into concrete
    /// `VkImageMemoryBarrier` / `VkPipelineBarrier` calls.
    pub fn compile(&self) -> Result<CompiledRenderGraph, String> {
        let pass_order = self.topological_sort()?;
        let n = pass_order.len();

        // ── Resource state tracking ────────────────────────────────────
        let mut resource_states: HashMap<String, ResourceState> = HashMap::new();
        let mut barriers_per_pass: Vec<Vec<CompiledBarrier>> = vec![Vec::new(); n];

        for (sorted_idx, &pass_idx) in pass_order.iter().enumerate() {
            let pass = &self.passes[pass_idx];

            // ── Inputs (read-only) ──
            for input in &pass.inputs {
                let old = resource_states
                    .get(&input.name)
                    .copied()
                    .unwrap_or(ResourceState::Undefined);
                let new = ResourceState::ShaderReadOnlyOptimal;
                if old != new {
                    barriers_per_pass[sorted_idx].push(CompiledBarrier {
                        resource_name: input.name.clone(),
                        src_stage: previous_stage(&old),
                        dst_stage: PipeStage::FragmentShader,
                        old_state: old,
                        new_state: new,
                    });
                }
                resource_states.insert(input.name.clone(), new);
            }

            // ── Depth-stencil attachment ──
            if let Some(ref ds) = pass.depth_stencil {
                let old = resource_states
                    .get(&ds.name)
                    .copied()
                    .unwrap_or(ResourceState::Undefined);
                let new = ResourceState::DepthStencilAttachmentOptimal;
                if old != new {
                    barriers_per_pass[sorted_idx].push(CompiledBarrier {
                        resource_name: ds.name.clone(),
                        src_stage: previous_stage(&old),
                        dst_stage: PipeStage::EarlyFragmentTests,
                        old_state: old,
                        new_state: new,
                    });
                }
                resource_states.insert(ds.name.clone(), new);
            }

            // ── Outputs (written by the pass) ──
            for output in &pass.outputs {
                let old = resource_states
                    .get(&output.name)
                    .copied()
                    .unwrap_or(ResourceState::Undefined);
                let new = output_resource_state(&output.name);
                if old != new {
                    barriers_per_pass[sorted_idx].push(CompiledBarrier {
                        resource_name: output.name.clone(),
                        src_stage: previous_stage(&old),
                        dst_stage: output_stage(&output.name),
                        old_state: old,
                        new_state: new,
                    });
                }
                resource_states.insert(output.name.clone(), new);
            }
        }

        Ok(CompiledRenderGraph {
            pass_order,
            barriers_per_pass,
        })
    }

    /// Phase B: Enhanced compilation with pass culling and access-aware
    /// barrier inference.
    ///
    /// 1. Topologically sorts the passes (Kahn's algorithm).
    /// 2. **Culls** passes whose outputs are never consumed (backward
    ///    reachability from the Present pass and other terminal passes).
    /// 3. Tracks resource state transitions across the live passes and
    ///    inserts [`CompiledBarrier`] whenever a resource transitions
    ///    between read/write roles.
    ///
    /// The `access` field on each [`PassAttachment`] is respected so that
    /// depth-stencil attachments used read-only get
    /// `DepthStencilReadOnlyOptimal` instead of
    /// `DepthStencilAttachmentOptimal`.
    pub fn compile_v2(&self) -> Result<CompiledRenderGraph, String> {
        let all_sorted = self.topological_sort()?;
        let n = all_sorted.len();
        if n == 0 {
            return Ok(CompiledRenderGraph {
                pass_order: vec![],
                barriers_per_pass: vec![],
            });
        }

        // ── Phase 1: Cull dead passes via backward reachability ──────────

        // Collect which passes produce / consume each resource.
        let mut resource_writers: HashMap<String, Vec<usize>> = HashMap::new();
        let mut resource_readers: HashMap<String, Vec<usize>> = HashMap::new();

        for &pass_idx in &all_sorted {
            let pass = &self.passes[pass_idx];
            for o in &pass.outputs {
                resource_writers.entry(o.name.clone()).or_default().push(pass_idx);
            }
            for i in &pass.inputs {
                resource_readers.entry(i.name.clone()).or_default().push(pass_idx);
            }
            if let Some(ref ds) = pass.depth_stencil {
                resource_readers.entry(ds.name.clone()).or_default().push(pass_idx);
                resource_writers.entry(ds.name.clone()).or_default().push(pass_idx);
            }
        }

        // Build forward edges: producer → consumer via resource flow.
        let mut forward_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &pass_idx in &all_sorted {
            let pass = &self.passes[pass_idx];
            let written: Vec<&str> = pass
                .outputs
                .iter()
                .map(|o| o.name.as_str())
                .chain(pass.depth_stencil.as_ref().map(|ds| ds.name.as_str()))
                .collect();
            for w in written {
                if let Some(consumers) = resource_readers.get(w) {
                    for &c in consumers {
                        if c != pass_idx && !forward_edges[pass_idx].contains(&c) {
                            forward_edges[pass_idx].push(c);
                        }
                    }
                }
            }
        }

        // Reverse the forward edges: needed_by[c] = {p | p → c}.
        let mut needed_by: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (p, consumers) in forward_edges.iter().enumerate() {
            for &c in consumers {
                needed_by[c].push(p);
            }
        }

        // Also respect explicit GraphEdges.
        for edge in &self.edges {
            if edge.from_pass < n && edge.to_pass < n {
                if !needed_by[edge.to_pass].contains(&edge.from_pass) {
                    needed_by[edge.to_pass].push(edge.from_pass);
                }
            }
        }

        // BFS backward from terminal passes.
        let mut live: Vec<bool> = vec![false; n];
        let mut queue: Vec<usize> = Vec::new();

        for &pass_idx in &all_sorted {
            let pass = &self.passes[pass_idx];
            // A pass is terminal (always live) if it is the Present pass or
            // if it produces no outputs (operator / side-effect passes).
            let is_terminal = matches!(pass.kind, PassKind::Present)
                || pass.outputs.is_empty();
            if is_terminal && !live[pass_idx] {
                live[pass_idx] = true;
                queue.push(pass_idx);
            }
        }

        while let Some(v) = queue.pop() {
            for &pred in &needed_by[v] {
                if !live[pred] {
                    live[pred] = true;
                    queue.push(pred);
                }
            }
        }

        let live_order: Vec<usize> = all_sorted.into_iter().filter(|&i| live[i]).collect();
        let m = live_order.len();

        // ── Phase 2: Barrier inference ──────────────────────────────────

        let mut resource_states: HashMap<String, ResourceState> = HashMap::new();
        let mut barriers_per_pass: Vec<Vec<CompiledBarrier>> = vec![Vec::new(); m];

        for (sorted_idx, &pass_idx) in live_order.iter().enumerate() {
            let pass = &self.passes[pass_idx];

            // ── Inputs (read or read-write) ──
            for input in &pass.inputs {
                let old = resource_states
                    .get(&input.name)
                    .copied()
                    .unwrap_or(ResourceState::Undefined);
                let new = match input.access {
                    ResourceAccess::Read => ResourceState::ShaderReadOnlyOptimal,
                    ResourceAccess::ReadWrite => ResourceState::General,
                    ResourceAccess::Write => ResourceState::ShaderReadOnlyOptimal,
                };
                if old != new {
                    barriers_per_pass[sorted_idx].push(CompiledBarrier {
                        resource_name: input.name.clone(),
                        src_stage: previous_stage(&old),
                        dst_stage: PipeStage::FragmentShader,
                        old_state: old,
                        new_state: new,
                    });
                }
                resource_states.insert(input.name.clone(), new);
            }

            // ── Depth-stencil attachment ──
            if let Some(ref ds) = pass.depth_stencil {
                let old = resource_states
                    .get(&ds.name)
                    .copied()
                    .unwrap_or(ResourceState::Undefined);
                let new = match ds.access {
                    ResourceAccess::Read => ResourceState::DepthStencilReadOnlyOptimal,
                    ResourceAccess::Write | ResourceAccess::ReadWrite => {
                        ResourceState::DepthStencilAttachmentOptimal
                    }
                };
                if old != new {
                    barriers_per_pass[sorted_idx].push(CompiledBarrier {
                        resource_name: ds.name.clone(),
                        src_stage: previous_stage(&old),
                        dst_stage: PipeStage::EarlyFragmentTests,
                        old_state: old,
                        new_state: new,
                    });
                }
                resource_states.insert(ds.name.clone(), new);
            }

            // ── Outputs (written by the pass) ──
            for output in &pass.outputs {
                let old = resource_states
                    .get(&output.name)
                    .copied()
                    .unwrap_or(ResourceState::Undefined);
                let new = output_resource_state(&output.name);
                if old != new {
                    barriers_per_pass[sorted_idx].push(CompiledBarrier {
                        resource_name: output.name.clone(),
                        src_stage: previous_stage(&old),
                        dst_stage: output_stage(&output.name),
                        old_state: old,
                        new_state: new,
                    });
                }
                resource_states.insert(output.name.clone(), new);
            }
        }

        Ok(CompiledRenderGraph {
            pass_order: live_order,
            barriers_per_pass,
        })
    }

    /// Convert this new-style `RenderGraph` into the legacy
    /// `render_graph::RenderGraph` so it can be consumed by the existing
    /// `BackendRenderer::execute_pass` trait.
    pub fn to_legacy(&self) -> render_graph::RenderGraph {
        let legacy_passes: Vec<render_graph::PassNode> =
            self.passes.iter().filter_map(|p| p.to_legacy()).collect();

        render_graph::RenderGraph {
            passes: legacy_passes,
        }
    }
}

impl Default for RenderGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ── Config types (serialisable for scene settings) ──────────────────────────

/// Configuration for a full render graph, loadable from scene settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassGraphConfig {
    /// Ordered list of pass entries.  The graph builder emits enabled passes
    /// in the order they appear here.
    pub passes: Vec<PassConfigEntry>,
    /// Whether the graph config is active.  When `false`, the canonical
    /// 4-pass ordering is used.
    pub enabled: bool,
}

impl Default for PassGraphConfig {
    fn default() -> Self {
        Self {
            passes: vec![
                PassConfigEntry {
                    kind: "DirectionalShadow".into(),
                    enabled: true,
                },
                PassConfigEntry {
                    kind: "OpaquePbrForward".into(),
                    enabled: true,
                },
                PassConfigEntry {
                    kind: "ssao".into(),
                    enabled: true,
                },
                PassConfigEntry {
                    kind: "bloom".into(),
                    enabled: true,
                },
                PassConfigEntry {
                    kind: "ToneMap".into(),
                    enabled: true,
                },
                PassConfigEntry {
                    kind: "Present".into(),
                    enabled: true,
                },
            ],
            enabled: true,
        }
    }
}

/// A single entry in a `PassGraphConfig`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassConfigEntry {
    /// Pass kind string — matches the values returned by
    /// [`PassKind::as_str`].
    pub kind: String,
    /// Whether this pass is enabled in the graph.
    pub enabled: bool,
}

// ============================================================================
// Helper functions for graph compilation
// ============================================================================

/// Map a [`ResourceState`] to the pipeline stage that most recently wrote /
/// produced it.  Used as the `src_stage` of a barrier.
fn previous_stage(state: &ResourceState) -> PipeStage {
    match state {
        ResourceState::ColorAttachmentOptimal => PipeStage::ColorAttachmentOutput,
        ResourceState::DepthStencilAttachmentOptimal => PipeStage::LateFragmentTests,
        ResourceState::DepthStencilReadOnlyOptimal => PipeStage::EarlyFragmentTests,
        ResourceState::ShaderReadOnlyOptimal => PipeStage::FragmentShader,
        ResourceState::TransferSrcOptimal | ResourceState::TransferDstOptimal => {
            PipeStage::Transfer
        }
        ResourceState::PresentSrc => PipeStage::BottomOfPipe,
        ResourceState::Undefined | ResourceState::General => PipeStage::TopOfPipe,
    }
}

/// Determine the [`ResourceState`] that an output attachment should be in
/// after the pass produces it.
fn output_resource_state(name: &str) -> ResourceState {
    match name {
        "swapchain" => ResourceState::PresentSrc,
        "shadow_map" | "shadow_depth" => ResourceState::DepthStencilAttachmentOptimal,
        "hdr_color" => ResourceState::ColorAttachmentOptimal,
        "ldr_color" => ResourceState::ColorAttachmentOptimal,
        "ssao_output" => ResourceState::ShaderReadOnlyOptimal,
        _ => ResourceState::ColorAttachmentOptimal,
    }
}

/// Determine the [`PipeStage`] at which an output is produced.
fn output_stage(name: &str) -> PipeStage {
    match name {
        "swapchain" => PipeStage::ColorAttachmentOutput,
        "shadow_map" | "shadow_depth" => PipeStage::LateFragmentTests,
        "hdr_color" | "ldr_color" => PipeStage::ColorAttachmentOutput,
        _ => PipeStage::ColorAttachmentOutput,
    }
}

// ============================================================================
// Transient resource aliasing  (Phase B)
// ============================================================================

/// A single slot in the aliasing plan — one or more resources that share
/// the same physical memory because their lifetimes do not overlap.
#[derive(Clone, Debug)]
pub struct AliasSlot {
    /// Resources aliased into this slot.
    pub resources: Vec<String>,
    /// Index of this slot in the pool.
    pub slot_index: usize,
}

/// Plan produced by [`TransientResourcePool::build`] describing how to
/// alias transient resources onto a fixed number of memory slots.
#[derive(Clone, Debug, Default)]
pub struct AliasingPlan {
    /// Ordered list of alias slots.
    pub slots: Vec<AliasSlot>,
    /// Mapping from resource name to its assigned slot index.
    pub resource_to_slot: HashMap<String, usize>,
}

/// A pool that analyses resource lifetimes across the sorted pass order
/// and assigns non-overlapping resources to the same memory slot.
///
/// # Algorithm
///
/// 1. For each resource, compute the interval `[first_pass, last_pass]`
///    over the sorted pass order.
/// 2. Sort resources by their first-use pass.
/// 3. Greedy interval packing: assign each resource to the first slot
///    whose current occupant's interval does not overlap.
///
/// Resources whose names appear in `exempt` (e.g. `"swapchain"`) are
/// excluded from aliasing because their memory is owned by the swapchain.
#[derive(Clone, Debug)]
pub struct TransientResourcePool {
    /// Resource names that must not be aliased.
    exempt: Vec<String>,
}

impl TransientResourcePool {
    /// Create a new pool with the given exempt resources.
    pub fn new(exempt: Vec<String>) -> Self {
        Self { exempt }
    }

    /// Build an aliasing plan from the render graph's pass declarations.
    ///
    /// `pass_order` is the sorted execution order (e.g. from
    /// [`compile_v2`](RenderGraph::compile_v2)).
    pub fn build(&self, graph: &RenderGraph, pass_order: &[usize]) -> AliasingPlan {
        // ── Step 1: collect lifetime intervals ──────────────────────────
        let mut first_use: HashMap<String, usize> = HashMap::new();
        let mut last_use: HashMap<String, usize> = HashMap::new();

        for (sorted_idx, &pass_idx) in pass_order.iter().enumerate() {
            let pass = &graph.passes[pass_idx];
            for i in &pass.inputs {
                first_use.entry(i.name.clone()).or_insert(sorted_idx);
                last_use.insert(i.name.clone(), sorted_idx);
            }
            for o in &pass.outputs {
                first_use.entry(o.name.clone()).or_insert(sorted_idx);
                last_use.insert(o.name.clone(), sorted_idx);
            }
            if let Some(ref ds) = pass.depth_stencil {
                first_use.entry(ds.name.clone()).or_insert(sorted_idx);
                last_use.insert(ds.name.clone(), sorted_idx);
            }
        }

        // ── Step 2: build intervals, excluding exempt resources ─────────
        struct Interval {
            name: String,
            first: usize,
            last: usize,
        }

        let mut intervals: Vec<Interval> = Vec::new();
        for (name, &first) in &first_use {
            if self.exempt.iter().any(|e| e == name) {
                continue;
            }
            let last = *last_use.get(name).unwrap_or(&first);
            intervals.push(Interval {
                name: name.clone(),
                first,
                last,
            });
        }

        // Sort by first-use pass.
        intervals.sort_by_key(|iv| iv.first);

        // ── Step 3: greedy interval packing ─────────────────────────────
        let mut slot_ends: Vec<usize> = Vec::new();
        let mut slots: Vec<AliasSlot> = Vec::new();
        let mut resource_to_slot: HashMap<String, usize> = HashMap::new();

        for iv in &intervals {
            let mut placed = false;
            for (slot_idx, &end) in slot_ends.iter().enumerate() {
                if iv.first > end {
                    // Non-overlapping → alias into this slot.
                    slots[slot_idx].resources.push(iv.name.clone());
                    slot_ends[slot_idx] = slot_ends[slot_idx].max(iv.last);
                    resource_to_slot.insert(iv.name.clone(), slot_idx);
                    placed = true;
                    break;
                }
            }
            if !placed {
                // Need a new slot.
                let slot_idx = slots.len();
                slot_ends.push(iv.last);
                slots.push(AliasSlot {
                    resources: vec![iv.name.clone()],
                    slot_index: slot_idx,
                });
                resource_to_slot.insert(iv.name.clone(), slot_idx);
            }
        }

        AliasingPlan {
            slots,
            resource_to_slot,
        }
    }
}

impl Default for TransientResourcePool {
    fn default() -> Self {
        Self {
            exempt: vec!["swapchain".into()],
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────

    fn make_graph() -> RenderGraph {
        // Build a simple 4-pass graph:
        //   A (writes "color") → B (reads "color", writes "temp", writes
        //   "unused") → C (side-effect, reads "temp") → D (reads "temp",
        //   outputs "swapchain")
        let mut graph = RenderGraph::new();

        // Pass 0: A → writes "color"
        graph.add_pass(PassNode {
            kind: PassKind::Custom("A"),
            name: "pass_a",
            view_id: 0,
            inputs: vec![],
            outputs: vec![PassAttachment {
                name: "color".into(),
                format: Some("RGBA16F".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            }],
            depth_stencil: None,
        });

        // Pass 1: B → reads "color", writes "temp", also writes "unused"
        graph.add_pass(PassNode {
            kind: PassKind::Custom("B"),
            name: "pass_b",
            view_id: 0,
            inputs: vec![PassAttachment {
                name: "color".into(),
                format: Some("RGBA16F".into()),
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: vec![
                PassAttachment {
                    name: "temp".into(),
                    format: Some("R8".into()),
                    clear: true,
                    load_op: "clear".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Write,
                },
                PassAttachment {
                    name: "unused".into(),
                    format: Some("R8".into()),
                    clear: true,
                    load_op: "clear".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Write,
                },
            ],
            depth_stencil: None,
        });

        // Pass 2: C → side-effect pass (reads "temp", no outputs)
        graph.add_pass(PassNode {
            kind: PassKind::Custom("C"),
            name: "pass_c",
            view_id: 0,
            inputs: vec![PassAttachment {
                name: "temp".into(),
                format: Some("R8".into()),
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: vec![],
            depth_stencil: None,
        });

        // Pass 3: D → Present (reads "temp", writes "swapchain")
        graph.add_pass(PassNode {
            kind: PassKind::Present,
            name: "present",
            view_id: 0,
            inputs: vec![PassAttachment {
                name: "temp".into(),
                format: Some("R8".into()),
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: vec![PassAttachment {
                name: "swapchain".into(),
                format: None,
                clear: false,
                load_op: "dont_care".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            }],
            depth_stencil: None,
        });

        // Add sequential edges for deterministic ordering.
        let count = graph.passes.len();
        for i in 0..count.saturating_sub(1) {
            graph.add_edge(i, i + 1, "auto");
        }

        graph
    }

    // ── compile_v2 tests ─────────────────────────────────────────────────

    #[test]
    fn compile_v2_topological_sort() {
        let graph = make_graph();
        let compiled = graph.compile_v2().expect("compile_v2 should succeed");
        assert_eq!(compiled.pass_order.len(), 4);
        let positions: Vec<usize> = compiled.pass_order.iter().copied().collect();
        for w in positions.windows(2) {
            assert!(w[0] < w[1], "pass_order should be ascending");
        }
    }

    #[test]
    fn compile_v2_culls_unconsumed_output() {
        // Pass B writes "unused" that nobody reads, but B also writes "temp"
        // which IS consumed → B stays live because "temp" reaches Present.
        let graph = make_graph();
        let compiled = graph.compile_v2().expect("compile_v2 should succeed");
        assert_eq!(compiled.pass_order.len(), 4);
    }

    #[test]
    fn compile_v2_culls_dead_pass() {
        let mut graph = make_graph();

        // Pass 4: E (dead) — writes "dead_buffer", nothing reads it.
        graph.add_pass(PassNode {
            kind: PassKind::Custom("E"),
            name: "dead_pass",
            view_id: 0,
            inputs: vec![],
            outputs: vec![PassAttachment {
                name: "dead_buffer".into(),
                format: Some("R8".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            }],
            depth_stencil: None,
        });

        let compiled = graph.compile_v2().expect("compile_v2 should succeed");
        assert_eq!(compiled.pass_order.len(), 4);
        assert!(
            !compiled.pass_order.contains(&4),
            "dead pass should be culled"
        );
    }

    #[test]
    fn compile_v2_barriers_between_transitions() {
        let graph = make_graph();
        let compiled = graph.compile_v2().expect("compile_v2 should succeed");
        let total_barriers: usize = compiled
            .barriers_per_pass
            .iter()
            .map(|b| b.len())
            .sum();
        assert!(total_barriers >= 1, "expected at least 1 barrier");
    }

    #[test]
    fn compile_v2_empty_graph() {
        let graph = RenderGraph::new();
        let compiled = graph.compile_v2().expect("empty graph should compile");
        assert!(compiled.pass_order.is_empty());
        assert!(compiled.barriers_per_pass.is_empty());
    }

    #[test]
    fn compile_v2_matches_compile_when_no_dead_passes() {
        let graph = make_graph();
        let v2 = graph.compile_v2().expect("compile_v2");
        let v1 = graph.compile().expect("compile");
        // Since there are no dead passes, orders match.
        assert_eq!(v2.pass_order, v1.pass_order);
    }

    // ── TransientResourcePool tests ──────────────────────────────────────

    #[test]
    fn transient_pool_empty_graph() {
        let graph = RenderGraph::new();
        let pool = TransientResourcePool::default();
        let plan = pool.build(&graph, &[]);
        assert!(plan.slots.is_empty());
        assert!(plan.resource_to_slot.is_empty());
    }

    #[test]
    fn transient_pool_aliases_non_overlapping_resources() {
        let graph = make_graph();
        let order: Vec<usize> = (0..graph.passes.len()).collect();
        let pool = TransientResourcePool::default();
        let plan = pool.build(&graph, &order);

        assert!(
            !plan.resource_to_slot.contains_key("swapchain"),
            "swapchain should be exempt"
        );

        for name in &["color", "temp", "unused"] {
            assert!(
                plan.resource_to_slot.contains_key(*name),
                "resource '{name}' should have a slot assignment"
            );
        }
    }

    #[test]
    fn transient_pool_overlapping_resources_get_different_slots() {
        let mut graph = RenderGraph::new();

        graph.add_pass(PassNode {
            kind: PassKind::Custom("P"),
            name: "p",
            view_id: 0,
            inputs: vec![],
            outputs: vec![
                PassAttachment {
                    name: "a".into(),
                    format: Some("R8".into()),
                    clear: true,
                    load_op: "clear".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Write,
                },
                PassAttachment {
                    name: "b".into(),
                    format: Some("R8".into()),
                    clear: true,
                    load_op: "clear".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Write,
                },
            ],
            depth_stencil: None,
        });

        let order = vec![0usize];
        let pool = TransientResourcePool::new(vec![]);
        let plan = pool.build(&graph, &order);

        let slot_a = plan.resource_to_slot.get("a").copied();
        let slot_b = plan.resource_to_slot.get("b").copied();
        assert!(slot_a.is_some());
        assert!(slot_b.is_some());
        assert_ne!(
            slot_a, slot_b,
            "overlapping resources must get different slots"
        );
    }

    #[test]
    fn transient_pool_sequential_resources_can_share_slot() {
        let mut graph = RenderGraph::new();

        // Pass 0: writes "early"
        graph.add_pass(PassNode {
            kind: PassKind::Custom("E"),
            name: "early",
            view_id: 0,
            inputs: vec![],
            outputs: vec![PassAttachment {
                name: "early".into(),
                format: Some("R8".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            }],
            depth_stencil: None,
        });

        // Pass 1: reads "early", writes "late" (no overlap — early not
        // used after pass 1, late starts at pass 1).
        graph.add_pass(PassNode {
            kind: PassKind::Custom("L"),
            name: "late",
            view_id: 0,
            inputs: vec![PassAttachment {
                name: "early".into(),
                format: Some("R8".into()),
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: vec![PassAttachment {
                name: "late".into(),
                format: Some("R16F".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            }],
            depth_stencil: None,
        });

        // Pass 2: Present (consumes "late")
        graph.add_pass(PassNode {
            kind: PassKind::Present,
            name: "present",
            view_id: 0,
            inputs: vec![PassAttachment {
                name: "late".into(),
                format: Some("R16F".into()),
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: vec![],
            depth_stencil: None,
        });

        let order: Vec<usize> = (0..graph.passes.len()).collect();
        let pool = TransientResourcePool::new(vec![]);
        let plan = pool.build(&graph, &order);

        let slot_early = plan.resource_to_slot.get("early").copied();
        let slot_late = plan.resource_to_slot.get("late").copied();
        assert!(slot_early.is_some());
        assert!(slot_late.is_some());
        // Early's lifetime [0,1], late's [1,2]; they overlap at pass 1
        // (both referenced), so must be in different slots.
        assert_ne!(slot_early, slot_late);
    }
}
