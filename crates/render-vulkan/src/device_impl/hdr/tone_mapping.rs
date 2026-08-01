use super::*;

struct ToneMappingResources {
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    descriptor_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set: vk::DescriptorSet,
}

/// Owns every Vulkan object created while the tone-mapping pipeline is being
/// assembled. Until `finish` disarms it, dropping this value rolls back the
/// partial construction in dependency-safe order.
struct PendingToneMappingResources<'a> {
    device: &'a ash::Device,
    render_pass: Option<vk::RenderPass>,
    pipeline_layout: Option<vk::PipelineLayout>,
    pipeline: Option<vk::Pipeline>,
    descriptor_layout: Option<vk::DescriptorSetLayout>,
    descriptor_pool: Option<vk::DescriptorPool>,
    descriptor_set: Option<vk::DescriptorSet>,
    vertex_module: Option<vk::ShaderModule>,
    fragment_module: Option<vk::ShaderModule>,
}

impl<'a> PendingToneMappingResources<'a> {
    fn new(device: &'a ash::Device) -> Self {
        Self {
            device,
            render_pass: None,
            pipeline_layout: None,
            pipeline: None,
            descriptor_layout: None,
            descriptor_pool: None,
            descriptor_set: None,
            vertex_module: None,
            fragment_module: None,
        }
    }

    fn finish(mut self) -> ToneMappingResources {
        // Shader modules are no longer needed after successful pipeline
        // creation. Remove them before disarming the persistent resources.
        // SAFETY: both optional modules were created by `self.device`, remain
        // exclusively guard-owned, and compiled pipelines no longer need them.
        unsafe {
            if let Some(module) = self.vertex_module.take() {
                self.device.destroy_shader_module(module, None);
            }
            if let Some(module) = self.fragment_module.take() {
                self.device.destroy_shader_module(module, None);
            }
        }

        ToneMappingResources {
            render_pass: self
                .render_pass
                .take()
                .expect("successful tone pipeline has a render pass"),
            pipeline_layout: self
                .pipeline_layout
                .take()
                .expect("successful tone pipeline has a pipeline layout"),
            pipeline: self
                .pipeline
                .take()
                .expect("successful tone pipeline has a pipeline"),
            descriptor_layout: self
                .descriptor_layout
                .take()
                .expect("successful tone pipeline has a descriptor layout"),
            descriptor_pool: self
                .descriptor_pool
                .take()
                .expect("successful tone pipeline has a descriptor pool"),
            descriptor_set: self
                .descriptor_set
                .take()
                .expect("successful tone pipeline has a descriptor set"),
        }
    }
}

impl Drop for PendingToneMappingResources<'_> {
    fn drop(&mut self) {
        // SAFETY: every non-empty handle below was created by `device`, has
        // not been published to VulkanDevice, and is destroyed exactly once.
        unsafe {
            if let Some(pipeline) = self.pipeline.take() {
                self.device.destroy_pipeline(pipeline, None);
            }
            if let Some(module) = self.vertex_module.take() {
                self.device.destroy_shader_module(module, None);
            }
            if let Some(module) = self.fragment_module.take() {
                self.device.destroy_shader_module(module, None);
            }
            if let Some(layout) = self.pipeline_layout.take() {
                self.device.destroy_pipeline_layout(layout, None);
            }
            // Destroying the pool releases its descriptor sets.
            if let Some(pool) = self.descriptor_pool.take() {
                self.device.destroy_descriptor_pool(pool, None);
            }
            if let Some(layout) = self.descriptor_layout.take() {
                self.device.destroy_descriptor_set_layout(layout, None);
            }
            if let Some(render_pass) = self.render_pass.take() {
                self.device.destroy_render_pass(render_pass, None);
            }
        }
    }
}

impl VulkanDevice {
    // ======================================================================
    // Tone-mapping render pass + pipeline
    // ======================================================================

    /// Create the tone-mapping render pass, pipeline, and descriptor set for
    /// reading the HDR image.
    pub(crate) fn create_tone_mapping_resources(&mut self) -> VkResult<()> {
        if self.tone_rp.is_some() {
            return Ok(());
        }
        let swapchain_format = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("no swapchain".into()))?
            .format;
        let d = &self.logical_device.device;
        let mut pending = PendingToneMappingResources::new(d);

        // ---- Tone-mapping render pass (color = BGRA8 only, no depth) ----
        let at = vk::AttachmentDescription::default()
            .format(swapchain_format)
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
        let dep = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::TOP_OF_PIPE)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);
        let deps = [dep];
        let rpi = vk::RenderPassCreateInfo::default()
            .attachments(&atts)
            .subpasses(&sps)
            .dependencies(&deps);
        // SAFETY: `d` is a valid AshDevice; `rpi` describes a valid render
        // pass; `None` means no custom allocator.
        let rp = unsafe { d.create_render_pass(&rpi, None) }
            .map_err(|r| VulkanError::vk("crp_tone", r))?;
        pending.render_pass = Some(rp);

        // ---- Descriptor set layout (set=0, binding=0 = combined image sampler) ----
        let ds_bindings = [0_u32, 1, 2].map(|binding| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(binding)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        });
        let ds_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&ds_bindings);
        // SAFETY: `d` is a valid AshDevice.
        let ds_layout = unsafe { d.create_descriptor_set_layout(&ds_layout_info, None) }
            .map_err(|r| VulkanError::vk("create_tone_ds_layout", r))?;
        pending.descriptor_layout = Some(ds_layout);

        // ---- Descriptor pool + set ----
        let pool_sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: 3,
        }];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        // SAFETY: `d` is a valid AshDevice.
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_tone_ds_pool", r))?;
        pending.descriptor_pool = Some(pool);

        let ds_layouts = [ds_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&ds_layouts);
        // SAFETY: `d` is a valid AshDevice; the pool has enough capacity.
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_tone_ds", r))?;
        let desc_set = desc_sets.first().copied().ok_or_else(|| {
            VulkanError::Loader("tone descriptor allocation returned no set".into())
        })?;
        pending.descriptor_set = Some(desc_set);

        // ---- Pipeline layout: set=0 (HDR sampler) + post-process parameters ----
        // The 128-byte block includes tone mapping, bloom, grading, vignette,
        // and packed planetary-lens parameters.
        let pc_range = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::FRAGMENT,
            offset: 0,
            size: 128,
        }];
        let tone_set_layouts = [ds_layout];
        let pll_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&tone_set_layouts)
            .push_constant_ranges(&pc_range);
        // SAFETY: `d` is a valid AshDevice.
        let pll = unsafe { d.create_pipeline_layout(&pll_info, None) }
            .map_err(|r| VulkanError::vk("cpl_tone", r))?;
        pending.pipeline_layout = Some(pll);

        // ---- Tonemap pipeline ----
        let vert_spv = crate::shaders_embedded::TONEMAP_VERT_SPV;
        let frag_spv = crate::shaders_embedded::TONEMAP_FRAG_SPV;
        if vert_spv.is_empty() || frag_spv.is_empty() {
            return Err(VulkanError::MissingShader("tonemap"));
        }
        // SAFETY: `d` is a valid AshDevice; SPIR-V bytecode is valid.
        let vm = unsafe { mk_sm(d, vert_spv)? };
        pending.vertex_module = Some(vm);
        // SAFETY: `d` is live; the embedded fragment SPIR-V is validated by
        // `mk_sm` and remains borrowed through module creation.
        let fm = unsafe { mk_sm(d, frag_spv)? };
        pending.fragment_module = Some(fm);

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
        // No vertex buffers (fullscreen triangle).
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
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid graphics
        // pipeline; `vk::PipelineCache::null()` is allowed.
        let mut pipelines =
            match unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
            {
                Ok(pipelines) => pipelines,
                Err((partial_pipelines, result)) => {
                    // Ash returns any handles Vulkan created before reporting
                    // the batch failure; those handles are caller-owned.
                    // SAFETY: every non-null partial handle was created by `d`
                    // in this failed transaction and was never published/used.
                    unsafe {
                        for pipeline in partial_pipelines {
                            if pipeline != vk::Pipeline::null() {
                                d.destroy_pipeline(pipeline, None);
                            }
                        }
                    }
                    return Err(VulkanError::vk("cgp_tone", result));
                }
            };
        let pipeline_count = pipelines.len();
        if pipeline_count != 1 {
            // SAFETY: an unexpected result count is rejected before publication;
            // every returned handle is exclusively owned and unused.
            unsafe {
                for pipeline in pipelines.drain(..) {
                    if pipeline != vk::Pipeline::null() {
                        d.destroy_pipeline(pipeline, None);
                    }
                }
            }
            return Err(VulkanError::Loader(format!(
                "tone pipeline creation returned {} pipelines instead of one",
                pipeline_count
            )));
        }
        let pipeline = pipelines
            .pop()
            .expect("pipeline vector length was checked above");
        pending.pipeline = Some(pipeline);

        let resources = pending.finish();
        self.tone_rp = Some(resources.render_pass);
        self.tone_pipeline_layout = Some(resources.pipeline_layout);
        self.tone_pipeline = Some(resources.pipeline);
        self.tone_desc_layout = Some(resources.descriptor_layout);
        self.tone_desc_pool = Some(resources.descriptor_pool);
        self.tone_desc_set = Some(resources.descriptor_set);

        Ok(())
    }

    // ======================================================================
    // Tone-mapping framebuffers (one per swapchain image, BGRA8 only)
    // ======================================================================

    /// Create tone-mapping framebuffers for all swapchain image views.
    pub(crate) fn create_tone_framebuffers(&mut self) -> VkResult<()> {
        // Destroy existing first
        self.destroy_tone_framebuffers();

        let rp = self
            .tone_rp
            .ok_or(VulkanError::Loader("tone RP not initialized".into()))?;
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("no swapchain".into()))?;
        let ext = sc.extent;
        let d = &self.logical_device.device;

        let mut fbs = Vec::with_capacity(sc.image_views.len());
        for &iv in &sc.image_views {
            let iva = [iv];
            // SAFETY: `d` is a valid AshDevice; framebuffer info references a
            // valid render pass and image view; `None` means no custom allocator.
            let fb_result = unsafe {
                d.create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(rp)
                        .attachments(&iva)
                        .width(ext.width)
                        .height(ext.height)
                        .layers(1),
                    None,
                )
            };
            match fb_result {
                Ok(fb) => fbs.push(fb),
                Err(result) => {
                    // The vector is still local, so roll back every
                    // framebuffer created before the failing swapchain view.
                    for fb in fbs.drain(..) {
                        // SAFETY: each framebuffer was created by `d` earlier in
                        // this transaction and no render submission references it.
                        unsafe {
                            d.destroy_framebuffer(fb, None);
                        }
                    }
                    return Err(VulkanError::vk("cfb_tone", result));
                }
            }
        }
        self.tone_framebuffers = fbs;
        Ok(())
    }

    /// Write the HDR image view + sampler into the tone-mapping descriptor set.
    pub(crate) fn update_tone_descriptor_set(&mut self) {
        let Some(ds) = self.tone_desc_set else { return };
        let Some(sampler) = self.hdr_color_sampler else {
            return;
        };
        let Some(image_view) = self.hdr_color_view else {
            return;
        };
        let Some(oit_accum_view) = self.oit_accum_view else {
            return;
        };
        let Some(oit_optical_depth_view) = self.oit_optical_depth_view else {
            return;
        };
        let d = &self.logical_device.device;

        let image_infos = [image_view, oit_accum_view, oit_optical_depth_view].map(|view| {
            vk::DescriptorImageInfo::default()
                .sampler(sampler)
                .image_view(view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        });
        let image_info_0 = [image_infos[0]];
        let image_info_1 = [image_infos[1]];
        let image_info_2 = [image_infos[2]];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info_0),
            vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info_1),
            vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info_2),
        ];
        // SAFETY: `d` is a valid AshDevice; descriptor set, sampler, and image
        // view are valid.
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }
    }

    // ======================================================================
    // Full HDR convenience initializer
    // ======================================================================

    /// Create all HDR + tone-mapping resources (idempotent).
    pub(crate) fn ensure_hdr_resources(&mut self) -> VkResult<()> {
        let multisample_resources_ready = self.hdr_msaa_samples == vk::SampleCountFlags::TYPE_1
            || (self.hdr_msaa_color_image.is_some()
                && self.oit_msaa_accum_image.is_some()
                && self.oit_msaa_optical_depth_image.is_some()
                && self.hdr_msaa_depth_image.is_some());
        if self.hdr_color_image.is_some()
            && self.oit_accum_image.is_some()
            && self.oit_optical_depth_image.is_some()
            && self.hdr_forward_rp.is_some()
            && self.tone_rp.is_some()
            && multisample_resources_ready
        {
            return Ok(());
        }
        let creation_result = (|| -> VkResult<()> {
            self.create_hdr_color_texture()?;
            self.create_hdr_forward_resources()?;
            self.create_tone_mapping_resources()?;
            self.create_tone_framebuffers()?;
            Ok(())
        })();
        if let Err(error) = creation_result {
            // All resource creators publish into `self` incrementally. Treat
            // the complete HDR graph as one transaction so a retry cannot
            // observe or leak a partially initialized graph.
            self.destroy_hdr_resources();
            return Err(error);
        }
        self.update_tone_descriptor_set();
        Ok(())
    }
}
