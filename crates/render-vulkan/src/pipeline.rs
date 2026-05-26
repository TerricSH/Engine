//! Render pass, graphics pipelines, and framebuffers for Gate 2 scenes.

use ash::vk;
use ash::Device as AshDevice;

use crate::error::{VkResult, VulkanError};

pub struct Pipeline {
    pub render_pass: vk::RenderPass,
    pub framebuffers: Vec<vk::Framebuffer>,
    pub pipeline_layout: vk::PipelineLayout,
    pub pipeline: vk::Pipeline,
    pub extent: vk::Extent2D,
    device: AshDevice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineKind {
    Triangle,
    TexturedQuad {
        descriptor_set_layout: vk::DescriptorSetLayout,
    },
}

impl Pipeline {
    /// Build the render pass, graphics pipeline, and per-image
    /// framebuffers for the supplied swapchain image views.
    ///
    /// # Safety
    /// `device`, `image_views`, and the SPIR-V byte slices must remain
    /// valid for the duration of this call.
    pub unsafe fn new(
        device: AshDevice,
        format: vk::Format,
        extent: vk::Extent2D,
        image_views: &[vk::ImageView],
        kind: PipelineKind,
        vert_spv: &[u8],
        frag_spv: &[u8],
        vert_name: &'static str,
        frag_name: &'static str,
    ) -> VkResult<Self> {
        if vert_spv.is_empty() {
            return Err(VulkanError::MissingShader(vert_name));
        }
        if frag_spv.is_empty() {
            return Err(VulkanError::MissingShader(frag_name));
        }

        // SAFETY: shader byte slices are valid; module retains its own copy.
        let vert_module = unsafe { create_shader_module(&device, vert_spv) }?;
        // SAFETY: see above.
        let frag_module = unsafe { create_shader_module(&device, frag_spv) }?;

        let render_pass = unsafe { create_render_pass(&device, format) };
        let render_pass = match render_pass {
            Ok(rp) => rp,
            Err(e) => {
                // SAFETY: modules are valid; cleanup before propagating.
                unsafe {
                    device.destroy_shader_module(vert_module, None);
                    device.destroy_shader_module(frag_module, None);
                }
                return Err(e);
            }
        };

        let descriptor_set_layouts = match kind {
            PipelineKind::Triangle => Vec::new(),
            PipelineKind::TexturedQuad {
                descriptor_set_layout,
            } => vec![descriptor_set_layout],
        };
        let pipeline_layout_info =
            vk::PipelineLayoutCreateInfo::default().set_layouts(&descriptor_set_layouts);
        // SAFETY: layout info has no refs.
        let pipeline_layout =
            match unsafe { device.create_pipeline_layout(&pipeline_layout_info, None) } {
                Ok(l) => l,
                Err(r) => {
                    // SAFETY: modules + render pass are valid.
                    unsafe {
                        device.destroy_render_pass(render_pass, None);
                        device.destroy_shader_module(vert_module, None);
                        device.destroy_shader_module(frag_module, None);
                    }
                    return Err(VulkanError::vk("create_pipeline_layout", r));
                }
            };

        let main = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(main),
        ];

        let vertex_bindings = vertex_bindings(kind);
        let vertex_attributes = vertex_attributes(kind);
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_bindings)
            .vertex_attribute_descriptions(&vertex_attributes);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);
        let viewports = [vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }];
        let scissors = [vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        }];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let color_blend_attachments = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false)];
        let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&color_blend_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .color_blend_state(&color_blend)
            .dynamic_state(&dynamic_state)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);

        // SAFETY: all referenced slices outlive this call.
        let pipeline = match unsafe {
            device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
        } {
            Ok(v) => v[0],
            Err((_, r)) => {
                // SAFETY: layouts/passes/modules valid; clean up.
                unsafe {
                    device.destroy_pipeline_layout(pipeline_layout, None);
                    device.destroy_render_pass(render_pass, None);
                    device.destroy_shader_module(vert_module, None);
                    device.destroy_shader_module(frag_module, None);
                }
                return Err(VulkanError::vk("create_graphics_pipelines", r));
            }
        };

        // Shader modules are no longer needed once the pipeline references them.
        // SAFETY: modules are no longer referenced by anything except the pipeline.
        unsafe {
            device.destroy_shader_module(vert_module, None);
            device.destroy_shader_module(frag_module, None);
        }

        let mut framebuffers = Vec::with_capacity(image_views.len());
        for &view in image_views {
            let attachments = [view];
            let fb_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(&attachments)
                .width(extent.width)
                .height(extent.height)
                .layers(1);
            // SAFETY: fb_info outlives this call.
            let fb = match unsafe { device.create_framebuffer(&fb_info, None) } {
                Ok(fb) => fb,
                Err(r) => {
                    // SAFETY: clean up the partial state.
                    unsafe {
                        for &existing in &framebuffers {
                            device.destroy_framebuffer(existing, None);
                        }
                        device.destroy_pipeline(pipeline, None);
                        device.destroy_pipeline_layout(pipeline_layout, None);
                        device.destroy_render_pass(render_pass, None);
                    }
                    return Err(VulkanError::vk("create_framebuffer", r));
                }
            };
            framebuffers.push(fb);
        }

        Ok(Self {
            render_pass,
            framebuffers,
            pipeline_layout,
            pipeline,
            extent,
            device,
        })
    }
}

fn vertex_bindings(kind: PipelineKind) -> Vec<vk::VertexInputBindingDescription> {
    match kind {
        PipelineKind::Triangle => Vec::new(),
        PipelineKind::TexturedQuad { .. } => vec![vk::VertexInputBindingDescription {
            binding: 0,
            stride: 16,
            input_rate: vk::VertexInputRate::VERTEX,
        }],
    }
}

fn vertex_attributes(kind: PipelineKind) -> Vec<vk::VertexInputAttributeDescription> {
    match kind {
        PipelineKind::Triangle => Vec::new(),
        PipelineKind::TexturedQuad { .. } => vec![
            vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vk::Format::R32G32_SFLOAT,
                offset: 0,
            },
            vk::VertexInputAttributeDescription {
                location: 1,
                binding: 0,
                format: vk::Format::R32G32_SFLOAT,
                offset: 8,
            },
        ],
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        // SAFETY: VulkanRenderer waits for the device to be idle before dropping.
        unsafe {
            for &fb in &self.framebuffers {
                self.device.destroy_framebuffer(fb, None);
            }
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.destroy_render_pass(self.render_pass, None);
        }
    }
}

unsafe fn create_shader_module(device: &AshDevice, spv: &[u8]) -> VkResult<vk::ShaderModule> {
    // SPIR-V words are 32-bit; ensure alignment by copying.
    if spv.len() % 4 != 0 {
        return Err(VulkanError::Loader(format!(
            "shader byte length {} is not a multiple of 4",
            spv.len()
        )));
    }
    let mut code = vec![0u32; spv.len() / 4];
    for (i, chunk) in spv.chunks_exact(4).enumerate() {
        code[i] = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    let info = vk::ShaderModuleCreateInfo::default().code(&code);
    // SAFETY: info outlives the call.
    let module = unsafe { device.create_shader_module(&info, None) }
        .map_err(|r| VulkanError::vk("create_shader_module", r))?;
    Ok(module)
}

unsafe fn create_render_pass(device: &AshDevice, format: vk::Format) -> VkResult<vk::RenderPass> {
    let attachments = [vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
    let color_refs = [vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
    let subpasses = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_refs)];
    let dependencies = [vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];
    let info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpasses)
        .dependencies(&dependencies);
    // SAFETY: info outlives this call.
    let pass = unsafe { device.create_render_pass(&info, None) }
        .map_err(|r| VulkanError::vk("create_render_pass", r))?;
    Ok(pass)
}
