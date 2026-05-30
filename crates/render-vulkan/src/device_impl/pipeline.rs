//! Pipeline building for VulkanDevice (MVP triangle + forward model).

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{blend_attachment_from_mode, default_dep, mk_sm, VulkanDevice};

impl VulkanDevice {
    /// Destroy all MVP and model pipeline resources (used on resize / suboptimal
    /// present).  Also tears down descriptor infrastructure and depth texture so
    /// they can be re-created at the new resolution.
    pub(crate) fn destroy_mvp(&mut self) {
        self.destroy_descriptor_infra();
        self.destroy_depth_texture();
        let d = &self.logical_device.device;
        for fb in self.mvp_framebuffers.drain(..) {
            // SAFETY: `fb` was created by this device and is still alive; not
            // already destroyed; `None` means no custom allocator.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        for fb in self.model_framebuffers.drain(..) {
            // SAFETY: `fb` was created by this device and is still alive.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        if let Some(p) = self.mvp_pipeline.take() {
            // SAFETY: `p` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.mvp_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(p) = self.model_pipeline.take() {
            // SAFETY: `p` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.model_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(rp) = self.mvp_rp.take() {
            // SAFETY: `rp` was created by this device and is still alive.
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        if let Some(rp) = self.model_rp.take() {
            // SAFETY: `rp` was created by this device and is still alive.
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        // Destroy HDR + tone-mapping resources on resize so they get
        // re-created at the new swapchain extent.
        self.destroy_hdr_resources();
        self.swapchain = None;
    }

    /// Build the MVP triangle pipeline (color-only, no depth, no vertex input).
    pub(crate) fn build_mvp(&mut self) -> VkResult<()> {
        let vert = self
            .mvp_vert_spv
            .clone()
            .ok_or(VulkanError::MissingShader("mvp.vert"))?;
        let frag = self
            .mvp_frag_spv
            .clone()
            .ok_or(VulkanError::MissingShader("mvp.frag"))?;
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let fmt = sc.format;
        let ext = self.swapchain_extent;
        let d = &self.logical_device.device;
        // SAFETY: `d` is a valid AshDevice; `vert` contains valid SPIR-V code.
        let vm = unsafe { mk_sm(d, &vert)? };
        // SAFETY: `d` is a valid AshDevice; `frag` contains valid SPIR-V code.
        let fm = unsafe { mk_sm(d, &frag)? };
        let at = vk::AttachmentDescription::default()
            .format(fmt)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
        let cr = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let crs = [cr];
        let atts = [at];
        let sp = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&crs);
        let sps = [sp];
        let dep = default_dep();
        let deps = [dep];
        let rpi = vk::RenderPassCreateInfo::default()
            .attachments(&atts)
            .subpasses(&sps)
            .dependencies(&deps);
        // SAFETY: `d` is a valid AshDevice; `rpi` describes a valid render
        // pass; `None` means no custom allocator.
        let rp =
            unsafe { d.create_render_pass(&rpi, None) }.map_err(|r| VulkanError::vk("crp", r))?;
        let pli = vk::PipelineLayoutCreateInfo::default();
        // SAFETY: `d` is a valid AshDevice; `pli` describes a valid pipeline
        // layout; `None` means no custom allocator.
        let pl = unsafe { d.create_pipeline_layout(&pli, None) }
            .map_err(|r| VulkanError::vk("cpl", r))?;
        let main = c"main";
        let sr = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vm)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fm)
                .name(main),
        ];
        let vi = vk::PipelineVertexInputStateCreateInfo::default();
        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let vs = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let cba = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false)];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);
        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .color_blend_state(&cb)
            .dynamic_state(&ds)
            .layout(pl)
            .render_pass(rp)
            .subpass(0);
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid graphics
        // pipeline; `vk::PipelineCache::null()` is allowed; `None` means no
        // custom allocator.
        let p = unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
            .map_err(|(_, r)| VulkanError::vk("cgp", r))?[0];
        // SAFETY: `vm` and `fm` were created by this device and are no longer
        // needed after pipeline creation; `None` means no custom allocator.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }
        let mut fbs = Vec::new();
        for iv in &sc.image_views {
            let iva = [*iv];
            fbs.push(
                // SAFETY: `d` is a valid AshDevice; framebuffer creation info
                // references a valid render pass and image views; `None` means
                // no custom allocator.
                unsafe {
                    d.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(rp)
                            .attachments(&iva)
                            .width(ext.width)
                            .height(ext.height)
                            .layers(1),
                        None,
                    )
                }
                .map_err(|r| VulkanError::vk("cfb", r))?,
            );
        }
        self.mvp_rp = Some(rp);
        self.mvp_pipeline_layout = Some(pl);
        self.mvp_pipeline = Some(p);
        self.mvp_framebuffers = fbs;
        Ok(())
    }

    /// Build the forward model pipeline (color + depth, vertex input with
    /// position/normal/UV, descriptor sets for UBO + shadow map).
    pub(crate) fn build_model_pipeline(&mut self) -> VkResult<()> {
        let vert = self
            .mvp_vert_spv
            .clone()
            .ok_or(VulkanError::MissingShader("model.vert"))?;
        let frag = self
            .mvp_frag_spv
            .clone()
            .ok_or(VulkanError::MissingShader("model.frag"))?;
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let fmt = sc.format;
        let ext = self.swapchain_extent;
        let d = &self.logical_device.device;
        // SAFETY: `d` is a valid AshDevice; `vert` contains valid SPIR-V code.
        let vm = unsafe { mk_sm(d, &vert)? };
        // SAFETY: `d` is a valid AshDevice; `frag` contains valid SPIR-V code.
        let fm = unsafe { mk_sm(d, &frag)? };

        // --- Pipeline layout: set=0 (UBO) + set=1 (shadow) + set=2 (material) ---
        let mut set_layouts: Vec<vk::DescriptorSetLayout> = Vec::new();
        if let Some(dsl) = self.desc_set_layout_0 {
            set_layouts.push(dsl);
        }
        if let Some(sdl) = self.shadow_desc_layout {
            set_layouts.push(sdl);
        }
        if let Some(mdl) = self.material_desc_set_layout {
            set_layouts.push(mdl);
        }
        let pli = vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
        // SAFETY: `d` is a valid AshDevice; `pli` describes a valid pipeline
        // layout; `None` means no custom allocator.
        let pl = unsafe { d.create_pipeline_layout(&pli, None) }
            .map_err(|r| VulkanError::vk("cpl_model", r))?;

        // --- Render pass: color + depth ---
        let color_at = vk::AttachmentDescription::default()
            .format(fmt)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
        let depth_at = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let color_ref = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let depth_ref = vk::AttachmentReference::default()
            .attachment(1)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_ref)
            .depth_stencil_attachment(&depth_ref);
        let dep = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
            )
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            );
        let atts = [color_at, depth_at];
        let subpasses = [subpass];
        let deps = [dep];
        let rp_info = vk::RenderPassCreateInfo::default()
            .attachments(&atts)
            .subpasses(&subpasses)
            .dependencies(&deps);
        // SAFETY: `d` is a valid AshDevice; `rp_info` describes a valid render
        // pass; `None` means no custom allocator.
        let rp = unsafe { d.create_render_pass(&rp_info, None) }
            .map_err(|r| VulkanError::vk("crp_model", r))?;

        // --- Framebuffers (one per swapchain image, color + depth) ---
        let depth_view = self.depth_image_view.unwrap_or(vk::ImageView::null());
        let mut fbs = Vec::new();
        for iv in &sc.image_views {
            let att_views = [*iv, depth_view];
            fbs.push(
                // SAFETY: `d` is a valid AshDevice; framebuffer creation info
                // references a valid render pass and image views; `None` means
                // no custom allocator.
                unsafe {
                    d.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(rp)
                            .attachments(&att_views)
                            .width(ext.width)
                            .height(ext.height)
                            .layers(1),
                        None,
                    )
                }
                .map_err(|r| VulkanError::vk("cfb_model", r))?,
            );
        }

        // --- Vertex input: position(loc=0), normal(loc=1), UV(loc=2) ---
        let stride = 32u32; // 3 + 3 + 2 floats = 8 * 4 = 32 bytes
        let vb = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(stride)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let va = [
            vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: 0,
            },
            vk::VertexInputAttributeDescription {
                location: 1,
                binding: 0,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: 12,
            },
            vk::VertexInputAttributeDescription {
                location: 2,
                binding: 0,
                format: vk::Format::R32G32_SFLOAT,
                offset: 24,
            },
        ];
        let vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vb)
            .vertex_attribute_descriptions(&va);

        let main = c"main";
        let sr = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vm)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fm)
                .name(main),
        ];
        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let vs2 = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .line_width(1.0);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend_attachment = blend_attachment_from_mode("Alpha");
        let cba = [blend_attachment];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);
        let ds = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds2 = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);
        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs2)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .depth_stencil_state(&ds)
            .color_blend_state(&cb)
            .dynamic_state(&ds2)
            .layout(pl)
            .render_pass(rp)
            .subpass(0);
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid graphics
        // pipeline; `vk::PipelineCache::null()` is allowed; `None` means no
        // custom allocator.
        let p = unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
            .map_err(|(_, r)| VulkanError::vk("cgp_model", r))?[0];
        // SAFETY: `vm` and `fm` were created by this device and are no longer
        // needed after pipeline creation; `None` means no custom allocator.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }
        self.model_pipeline_layout = Some(pl);
        self.model_pipeline = Some(p);
        self.model_rp = Some(rp);
        self.model_framebuffers = fbs;
        Ok(())
    }
}
