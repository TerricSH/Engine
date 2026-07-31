use super::passes::{
    add_sequential_view_edges, directional_shadow_pass, input_resource_state, input_stage,
    opaque_pbr_forward_pass, output_resource_state, output_stage, present_pass, previous_stage,
    tone_map_pass,
};
use super::*;

/// Backend-agnostic state used by compiled resource barriers.
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

/// Backend-agnostic pipeline stage used by compiled resource barriers.
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

/// A resource transition emitted before a compiled pass.
#[derive(Clone, Debug)]
pub struct CompiledBarrier {
    pub resource_name: String,
    pub src_stage: PipeStage,
    pub dst_stage: PipeStage,
    pub old_state: ResourceState,
    pub new_state: ResourceState,
}

/// The compiled pass order and barriers for a render graph.
#[derive(Clone, Debug)]
pub struct CompiledRenderGraph {
    pub pass_order: Vec<usize>,
    pub barriers_per_pass: Vec<Vec<CompiledBarrier>>,
}

impl CompiledRenderGraph {
    pub fn pass_count(&self) -> usize {
        self.pass_order.len()
    }
}

// ── Pass kind (extensible) ──────────────────────────────────────────────────

/// Extensible pass kind — can be one of the built-in kinds or a custom string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PassKind {
    DirectionalShadow,
    OpaquePbrForward,
    ToneMap,
    Present,
    Custom(&'static str),
}

impl PassKind {
    /// Built-in pass kinds in their canonical execution order.
    ///
    /// Custom kinds are deliberately absent: a custom pass is executable only
    /// after a backend has registered a matching [`crate::RenderPass`]. UI
    /// code must not present arbitrary strings as available render features.
    pub const BUILTIN_KINDS: [Self; 4] = [
        Self::DirectionalShadow,
        Self::OpaquePbrForward,
        Self::ToneMap,
        Self::Present,
    ];

    /// Whether this kind is implemented without a registered custom pass.
    pub const fn is_builtin(self) -> bool {
        !matches!(self, Self::Custom(_))
    }

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
            custom => Some(Self::Custom(intern_custom_pass_kind(custom))),
        }
    }
}

fn intern_custom_pass_kind(name: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(existing) = names.get(name) {
        return existing;
    }

    // PassKind's public v0 contract uses &'static str. Interning once per
    // distinct configured identifier keeps that API while avoiding the old
    // per-frame Box::leak growth in build_with_config().
    let interned = Box::leak(name.to_owned().into_boxed_str());
    names.insert(interned);
    interned
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassAttachment {
    pub name: String,
    pub format: Option<String>,
    pub clear: bool,
    pub load_op: String, // "clear", "load", "dont_care"
    pub size_source: SizeSource,
    pub access: ResourceAccess,
}

/// Determines how the attachment dimensions are resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SizeSource {
    Swapchain,
    Custom(u32, u32),
    FromInput(String),
}

// ── Pass node ───────────────────────────────────────────────────────────────

/// A single node in the DAG-based render graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassNode {
    pub kind: PassKind,
    pub name: &'static str,
    pub view_id: u32,
    pub inputs: Vec<PassAttachment>,
    pub outputs: Vec<PassAttachment>,
    pub depth_stencil: Option<PassAttachment>,
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
            let first_pass = graph.passes.len();
            let has_shadow_casters = input.lights.iter().any(|l| {
                l.kind == crate::LightKind::Directional
                    && matches!(
                        l.shadow_mode,
                        crate::ShadowMode::Hard | crate::ShadowMode::Soft
                    )
            });

            // 1. Directional shadow pass (only if shadows are needed)
            if has_shadow_casters {
                graph.add_pass(directional_shadow_pass(view.view_id));
            }

            // 2. Opaque forward pass
            graph.add_pass(opaque_pbr_forward_pass(
                view.view_id,
                PassGraphOutputMode::HdrThenToneMap,
            ));

            // 3. Tone-map pass
            graph.add_pass(tone_map_pass(view.view_id));

            // 4. Present
            graph.add_pass(present_pass(
                view.view_id,
                PassGraphOutputMode::HdrThenToneMap,
            ));

            add_sequential_view_edges(&mut graph, first_pass);
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
            l.kind == crate::LightKind::Directional
                && matches!(
                    l.shadow_mode,
                    crate::ShadowMode::Hard | crate::ShadowMode::Soft
                )
        });

        let mut graph = Self::new();

        for view in &views {
            let first_pass = graph.passes.len();
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

                // Direct-to-swapchain graphs never contain a tone-map pass.
                // Validation reports a configured ToneMap as an error before
                // rendering; keeping the builder fail-closed also prevents
                // callers that use it directly from constructing an invalid
                // implicit HDR path.
                if matches!(kind, PassKind::ToneMap)
                    && config.output_mode == PassGraphOutputMode::DirectToSwapchain
                {
                    continue;
                }

                let pass = match kind {
                    PassKind::DirectionalShadow => directional_shadow_pass(view.view_id),
                    PassKind::OpaquePbrForward => {
                        opaque_pbr_forward_pass(view.view_id, config.output_mode)
                    }
                    PassKind::ToneMap => tone_map_pass(view.view_id),
                    PassKind::Present => present_pass(view.view_id, config.output_mode),
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

            add_sequential_view_edges(&mut graph, first_pass);
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

    /// Compile this render graph with pass culling and access-aware barrier inference.
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
    pub fn compile(&self) -> Result<CompiledRenderGraph, String> {
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
                resource_writers
                    .entry(o.name.clone())
                    .or_default()
                    .push(pass_idx);
            }
            for i in &pass.inputs {
                resource_readers
                    .entry(i.name.clone())
                    .or_default()
                    .push(pass_idx);
            }
            if let Some(ref ds) = pass.depth_stencil {
                resource_readers
                    .entry(ds.name.clone())
                    .or_default()
                    .push(pass_idx);
                resource_writers
                    .entry(ds.name.clone())
                    .or_default()
                    .push(pass_idx);
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
            if edge.from_pass < n
                && edge.to_pass < n
                && !needed_by[edge.to_pass].contains(&edge.from_pass)
            {
                needed_by[edge.to_pass].push(edge.from_pass);
            }
        }

        // BFS backward from terminal passes.
        let mut live: Vec<bool> = vec![false; n];
        let mut queue: Vec<usize> = Vec::new();

        for &pass_idx in &all_sorted {
            let pass = &self.passes[pass_idx];
            // A pass is terminal (always live) if it is the Present pass or
            // if it produces no outputs (operator / side-effect passes).
            let is_terminal = matches!(pass.kind, PassKind::Present) || pass.outputs.is_empty();
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
                let new = input_resource_state(pass, input);
                if old != new {
                    barriers_per_pass[sorted_idx].push(CompiledBarrier {
                        resource_name: input.name.clone(),
                        src_stage: previous_stage(&old),
                        dst_stage: input_stage(pass, input),
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
                let new = output_resource_state(pass, &output.name);
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
}
