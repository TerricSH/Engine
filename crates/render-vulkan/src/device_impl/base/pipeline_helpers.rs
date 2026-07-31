use super::super::*;

// ============================================================================
// Helpers
// ============================================================================

/// Build a [`PipelineColorBlendAttachmentState`] from a mode string.
///
/// Supported modes: `"Alpha"`, `"Additive"`, `"Multiply"`, or `None` / `"Opaque"`.
pub(in super::super) fn blend_attachment_from_mode(
    mode: &str,
) -> vk::PipelineColorBlendAttachmentState {
    let (enable, src_color, dst_color, src_alpha, dst_alpha) = match mode {
        "Alpha" => (
            true,
            vk::BlendFactor::SRC_ALPHA,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        ),
        "Additive" => (
            true,
            vk::BlendFactor::SRC_ALPHA,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ONE,
        ),
        "Multiply" => (
            true,
            vk::BlendFactor::ZERO,
            vk::BlendFactor::SRC_COLOR,
            vk::BlendFactor::ZERO,
            vk::BlendFactor::SRC_ALPHA,
        ),
        _ => (
            false,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ZERO,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ZERO,
        ),
    };
    vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(enable)
        .src_color_blend_factor(src_color)
        .dst_color_blend_factor(dst_color)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(src_alpha)
        .dst_alpha_blend_factor(dst_alpha)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA)
}

pub(in super::super) fn mrt_blend_attachments(
    mode: &str,
) -> [vk::PipelineColorBlendAttachmentState; 3] {
    let disabled = vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(false)
        .color_write_mask(vk::ColorComponentFlags::empty());
    if mode == "WeightedOit" {
        let additive = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        [disabled, additive, additive]
    } else {
        [blend_attachment_from_mode(mode), disabled, disabled]
    }
}

pub(in super::super) fn default_dep() -> vk::SubpassDependency {
    vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
}

pub(in super::super) fn vfmt(f: &str) -> vk::Format {
    match f {
        "float32x2" => vk::Format::R32G32_SFLOAT,
        "float32x3" => vk::Format::R32G32B32_SFLOAT,
        "float32x4" => vk::Format::R32G32B32A32_SFLOAT,
        "uint32x4" => vk::Format::R32G32B32A32_UINT,
        _ => vk::Format::R32G32B32_SFLOAT,
    }
}

pub(in super::super) fn compare_op(s: &Option<String>) -> vk::CompareOp {
    match s.as_deref() {
        Some("less") => vk::CompareOp::LESS,
        Some("equal") => vk::CompareOp::EQUAL,
        Some("lequal") => vk::CompareOp::LESS_OR_EQUAL,
        Some("greater") => vk::CompareOp::GREATER,
        Some("always") => vk::CompareOp::ALWAYS,
        _ => vk::CompareOp::ALWAYS,
    }
}

/// Create a Vulkan shader module from SPIR-V bytecode.
///
/// # Safety
///
/// - `d` must be a valid [`AshDevice`] that has not been destroyed.
/// - `spv` must contain valid SPIR-V binary data (word-aligned, correctly
///   sized for the targeted shader stage).
///
/// Map a resource kind string to a `VkDescriptorType`.
pub(in super::super) fn resource_kind_to_descriptor_type(
    kind: &str,
) -> Result<vk::DescriptorType, render_core::RhiError> {
    match kind {
        "uniform_buffer" => Ok(vk::DescriptorType::UNIFORM_BUFFER),
        "storage_buffer" => Ok(vk::DescriptorType::STORAGE_BUFFER),
        "sampler" => Ok(vk::DescriptorType::SAMPLER),
        "combined_image_sampler" => Ok(vk::DescriptorType::COMBINED_IMAGE_SAMPLER),
        "sampled_image" => Ok(vk::DescriptorType::SAMPLED_IMAGE),
        "storage_image" => Ok(vk::DescriptorType::STORAGE_IMAGE),
        "uniform_texel_buffer" => Ok(vk::DescriptorType::UNIFORM_TEXEL_BUFFER),
        "storage_texel_buffer" => Ok(vk::DescriptorType::STORAGE_TEXEL_BUFFER),
        "input_attachment" => Ok(vk::DescriptorType::INPUT_ATTACHMENT),
        _ => Err(render_core::RhiError::InvalidDescriptor {
            field: "bind_group_layouts.bindings.resource_kind".into(),
            reason: format!("unsupported Vulkan descriptor resource kind '{kind}'"),
        }),
    }
}

pub(in super::super) fn parse_topology(s: &Option<String>) -> vk::PrimitiveTopology {
    match s.as_deref() {
        Some("point_list") => vk::PrimitiveTopology::POINT_LIST,
        Some("line_list") => vk::PrimitiveTopology::LINE_LIST,
        Some("line_strip") => vk::PrimitiveTopology::LINE_STRIP,
        Some("triangle_strip") => vk::PrimitiveTopology::TRIANGLE_STRIP,
        Some("triangle_fan") => vk::PrimitiveTopology::TRIANGLE_FAN,
        _ => vk::PrimitiveTopology::TRIANGLE_LIST,
    }
}

pub(in super::super) fn parse_polygon_mode(s: &Option<String>) -> vk::PolygonMode {
    match s.as_deref() {
        Some("line") => vk::PolygonMode::LINE,
        Some("point") => vk::PolygonMode::POINT,
        _ => vk::PolygonMode::FILL,
    }
}

pub(in super::super) fn parse_sample_count(s: Option<u8>) -> vk::SampleCountFlags {
    match s {
        Some(2) => vk::SampleCountFlags::TYPE_2,
        Some(4) => vk::SampleCountFlags::TYPE_4,
        Some(8) => vk::SampleCountFlags::TYPE_8,
        Some(16) => vk::SampleCountFlags::TYPE_16,
        Some(32) => vk::SampleCountFlags::TYPE_32,
        Some(64) => vk::SampleCountFlags::TYPE_64,
        _ => vk::SampleCountFlags::TYPE_1,
    }
}

/// Create a Vulkan shader module from SPIR-V bytecode.
///
/// # Safety
///
/// - `d` must be a valid [`AshDevice`] that has not been destroyed.
/// - `spv` must contain valid SPIR-V binary data (word-aligned, correctly
///   sized for the targeted shader stage).
pub(in super::super) unsafe fn mk_sm(d: &AshDevice, spv: &[u8]) -> VkResult<vk::ShaderModule> {
    if spv.is_empty() {
        return Err(VulkanError::MissingShader(""));
    }
    if !spv.len().is_multiple_of(4) {
        return Err(VulkanError::Loader(format!("len {}", spv.len())));
    }
    let mut code = vec![0u32; spv.len() / 4];
    for (i, c) in spv.chunks_exact(4).enumerate() {
        code[i] = u32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
    }
    unsafe { d.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None) }
        .map_err(|r| VulkanError::vk("sm", r))
}
