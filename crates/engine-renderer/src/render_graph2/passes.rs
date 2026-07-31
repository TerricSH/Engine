use super::*;

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
    /// Describes whether the forward pass writes an intermediate HDR target
    /// or writes the presentation image directly.
    #[serde(default)]
    pub output_mode: PassGraphOutputMode,
}

/// Output contract for a configured pass graph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PassGraphOutputMode {
    /// Forward rendering writes HDR and a final ToneMap/composite pass copies
    /// it to the presentation image. `ToneMapping::None` makes that pass an
    /// identity conversion; it does not remove the pass.
    #[default]
    HdrThenToneMap,
    /// Forward rendering writes the presentation image directly, so a
    /// ToneMap pass must not be declared.
    DirectToSwapchain,
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
                    kind: "ToneMap".into(),
                    enabled: true,
                },
                PassConfigEntry {
                    kind: "Present".into(),
                    enabled: true,
                },
            ],
            enabled: true,
            output_mode: PassGraphOutputMode::HdrThenToneMap,
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
// Built-in pass declarations
// ============================================================================

/// Connect only the passes emitted for the current view. Keeping this next to
/// the shared pass constructors makes both graph-building paths follow the
/// same resource and dependency contract.
pub(super) fn add_sequential_view_edges(graph: &mut RenderGraph, first_pass: usize) {
    let end_pass = graph.passes.len();
    for from_pass in first_pass..end_pass.saturating_sub(1) {
        graph.add_edge(from_pass, from_pass + 1, "auto");
    }
}

pub(super) fn directional_shadow_pass(view_id: u32) -> PassNode {
    PassNode {
        kind: PassKind::DirectionalShadow,
        name: "directional_shadow_pass",
        view_id,
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
    }
}

pub(super) fn opaque_pbr_forward_pass(view_id: u32, output_mode: PassGraphOutputMode) -> PassNode {
    let color_output = match output_mode {
        PassGraphOutputMode::HdrThenToneMap => PassAttachment {
            name: "hdr_color".into(),
            format: Some("RGBA16F".into()),
            clear: true,
            load_op: "clear".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Write,
        },
        PassGraphOutputMode::DirectToSwapchain => PassAttachment {
            name: "swapchain".into(),
            format: None,
            clear: true,
            load_op: "clear".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Write,
        },
    };
    let mut color_outputs = vec![color_output];
    if output_mode == PassGraphOutputMode::HdrThenToneMap {
        color_outputs.extend([
            PassAttachment {
                name: "oit_accumulation".into(),
                format: Some("RGBA16F".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            },
            PassAttachment {
                name: "oit_optical_depth".into(),
                format: Some("RGBA16F".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            },
        ]);
    }

    PassNode {
        kind: PassKind::OpaquePbrForward,
        name: "opaque_pbr_forward_pass",
        view_id,
        inputs: vec![PassAttachment {
            name: "depth".into(),
            format: Some("D32".into()),
            clear: true,
            load_op: "load".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Read,
        }],
        outputs: color_outputs,
        depth_stencil: Some(PassAttachment {
            name: "depth_stencil".into(),
            format: Some("D24S8".into()),
            clear: true,
            load_op: "clear".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::ReadWrite,
        }),
    }
}

pub(super) fn tone_map_pass(view_id: u32) -> PassNode {
    PassNode {
        kind: PassKind::ToneMap,
        name: "tone_map_pass",
        view_id,
        inputs: ["hdr_color", "oit_accumulation", "oit_optical_depth"]
            .into_iter()
            .map(|name| PassAttachment {
                name: name.into(),
                format: Some("RGBA16F".into()),
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            })
            .collect(),
        outputs: vec![PassAttachment {
            name: "swapchain".into(),
            format: None,
            clear: true,
            load_op: "clear".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Write,
        }],
        depth_stencil: None,
    }
}

pub(super) fn present_pass(view_id: u32, output_mode: PassGraphOutputMode) -> PassNode {
    match output_mode {
        PassGraphOutputMode::HdrThenToneMap => PassNode {
            kind: PassKind::Present,
            name: "present",
            view_id,
            inputs: vec![PassAttachment {
                name: "swapchain".into(),
                format: None,
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: Vec::new(),
            depth_stencil: None,
        },
        PassGraphOutputMode::DirectToSwapchain => PassNode {
            kind: PassKind::Present,
            name: "present",
            view_id,
            inputs: vec![PassAttachment {
                name: "swapchain".into(),
                format: None,
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: Vec::new(),
            depth_stencil: None,
        },
    }
}

// ============================================================================
// Helper functions for graph compilation
// ============================================================================

/// Map a [`ResourceState`] to the pipeline stage that most recently wrote /
/// produced it.  Used as the `src_stage` of a barrier.
pub(super) fn previous_stage(state: &ResourceState) -> PipeStage {
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

/// Determine the state required while a pass consumes an input. A direct
/// present consumes the already-rendered swapchain image as a presentation
/// source rather than sampling it as a texture.
pub(super) fn input_resource_state(pass: &PassNode, input: &PassAttachment) -> ResourceState {
    if matches!(pass.kind, PassKind::Present) && input.name == "swapchain" {
        return ResourceState::PresentSrc;
    }

    match input.access {
        ResourceAccess::Read | ResourceAccess::Write => ResourceState::ShaderReadOnlyOptimal,
        ResourceAccess::ReadWrite => ResourceState::General,
    }
}

pub(super) fn input_stage(pass: &PassNode, input: &PassAttachment) -> PipeStage {
    if matches!(pass.kind, PassKind::Present) && input.name == "swapchain" {
        PipeStage::BottomOfPipe
    } else {
        PipeStage::FragmentShader
    }
}

/// Determine the [`ResourceState`] that an output attachment should be in
/// after the pass produces it. In direct mode the opaque pass writes the
/// swapchain as a color attachment; the following Present pass performs the
/// transition to [`ResourceState::PresentSrc`].
pub(super) fn output_resource_state(pass: &PassNode, name: &str) -> ResourceState {
    match name {
        "swapchain" if matches!(pass.kind, PassKind::Present) => ResourceState::PresentSrc,
        "swapchain" => ResourceState::ColorAttachmentOptimal,
        "shadow_map" | "shadow_depth" => ResourceState::DepthStencilAttachmentOptimal,
        "hdr_color" => ResourceState::ColorAttachmentOptimal,
        "ldr_color" => ResourceState::ColorAttachmentOptimal,
        "ssao_output" => ResourceState::ShaderReadOnlyOptimal,
        _ => ResourceState::ColorAttachmentOptimal,
    }
}

/// Determine the [`PipeStage`] at which an output is produced.
pub(super) fn output_stage(name: &str) -> PipeStage {
    match name {
        "swapchain" => PipeStage::ColorAttachmentOutput,
        "shadow_map" | "shadow_depth" => PipeStage::LateFragmentTests,
        "hdr_color" | "ldr_color" => PipeStage::ColorAttachmentOutput,
        _ => PipeStage::ColorAttachmentOutput,
    }
}
