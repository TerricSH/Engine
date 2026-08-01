use super::*;

impl VulkanDevice {
    /// Ensure shadow mapping resources exist (idempotent).
    pub(crate) fn ensure_shadow(&mut self) -> VkResult<()> {
        if self.shadow_map.is_some() {
            return Ok(());
        }
        self.create_shadow_resources()
    }

    /// Establish the descriptor-declared layout even when the current frame
    /// has no shadow-casting light and therefore omits the shadow render pass.
    /// The forward pipeline keeps a shadow descriptor bound unconditionally,
    /// so leaving a newly allocated image in `UNDEFINED` makes the entire
    /// command buffer invalid before the shader can branch on the light count.
    fn initialize_shadow_image_layout(&self, image: vk::Image, layer_count: u32) -> VkResult<()> {
        let d = &self.logical_device.device;
        // SAFETY: `d` is live and the selected graphics queue-family index is
        // valid; the transient pool is owned locally until final cleanup.
        let pool = unsafe {
            d.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT)
                    .queue_family_index(self.logical_device.queue_family_index),
                None,
            )
        }
        .map_err(|result| VulkanError::vk("create_shadow_init_command_pool", result))?;

        let result = (|| {
            // SAFETY: `pool` was just created by `d`; the create-info requests
            // one primary buffer and its temporary storage lives through call.
            let command_buffer = unsafe {
                d.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
            }
            .map_err(|result| VulkanError::vk("allocate_shadow_init_command_buffer", result))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                VulkanError::Loader(
                    "Vulkan returned no command buffer for shadow initialization".into(),
                )
            })?;

            // SAFETY: the newly allocated command buffer is in initial state;
            // the one-shot begin info is valid and call-scoped.
            unsafe {
                d.begin_command_buffer(
                    command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
            }
            .map_err(|result| VulkanError::vk("begin_shadow_init_command_buffer", result))?;
            let barrier = vk::ImageMemoryBarrier::default()
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count,
                })
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
            // SAFETY: the command buffer is recording; `image` is a live D32
            // array with at least `layer_count` layers, and barrier ranges/stages
            // match the declared layout transition. It is ended exactly once.
            unsafe {
                d.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
                d.end_command_buffer(command_buffer)
            }
            .map_err(|result| VulkanError::vk("end_shadow_init_command_buffer", result))?;

            // SAFETY: `d` is live and the default create-info produces a fresh
            // unsignaled fence owned by this local transaction.
            let fence = unsafe { d.create_fence(&vk::FenceCreateInfo::default(), None) }
                .map_err(|result| VulkanError::vk("create_shadow_init_fence", result))?;
            let command_buffers = [command_buffer];
            let submit = [vk::SubmitInfo::default().command_buffers(&command_buffers)];
            let completion =
                // SAFETY: the queue belongs to `d`; the command buffer ended,
                // submit arrays live through the call, and `fence` is unsignaled.
                match unsafe { d.queue_submit(self.logical_device.queue, &submit, fence) } {
                    Ok(()) => {
                        // SAFETY: `fence` tracks the submission above and remains
                        // live; an infinite wait observes completion before cleanup.
                        unsafe { d.wait_for_fences(&[fence], true, u64::MAX) }
                    }
                        .map_err(|result| VulkanError::vk("wait_shadow_init_fence", result)),
                    Err(result) => Err(VulkanError::vk("submit_shadow_init", result)),
                };
            if completion.is_err() {
                // SAFETY: the queue is a live queue from `d`; waiting idle is the
                // conservative rollback before destroying transaction resources.
                let _ = unsafe { d.queue_wait_idle(self.logical_device.queue) };
            }
            // SAFETY: successful completion or the idle fallback above ensures
            // the transaction's device-created fence has no pending owner.
            unsafe { d.destroy_fence(fence, None) };
            completion
        })();

        // SAFETY: all work from command buffers allocated by `pool` completed or
        // the queue-idle fallback ran; the local pool is exclusively owned.
        unsafe { d.destroy_command_pool(pool, None) };
        result
    }

    /// Create 2048 x 2048 directional-light CSM shadow resources (3 cascades).
    fn create_shadow_resources(&mut self) -> VkResult<()> {
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();
        const SHADOW_SIZE: u32 = 2048;
        const CASCADE_COUNT: u32 = CSM_CASCADE_COUNT as u32;

        // ---- 1. Shadow map image (2D array, D32_SFLOAT, GPU-only) ----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .extent(vk::Extent3D {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(CASCADE_COUNT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is a valid AshDevice; `image_info` describes a valid
        // 2D depth image array; `None` means no custom allocator.
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_image", r))?;
        // SAFETY: `image` was just created by this device; querying memory
        // requirements for a valid image is safe.
        let req = unsafe { d.get_image_memory_requirements(image) };
        let allocation = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "shadow-map",
                requirements: req,
                location: crate::allocator::MemoryLocation::GpuOnly,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: `image` was created by this device; `allocation` was created
        // for this image's memory requirements; the memory and offset are valid.
        unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_shadow_image", r))?;
        self.initialize_shadow_image_layout(image, CASCADE_COUNT)?;

        // ---- 2. Layered image view (for descriptor / shader sampling) ----
        let array_view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D_ARRAY)
            .format(vk::Format::D32_SFLOAT)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: CASCADE_COUNT,
            });
        // SAFETY: `d` is a valid AshDevice; `array_view_info` references a valid
        // image and subresource range covering all layers; `None` means no custom allocator.
        let array_image_view = unsafe { d.create_image_view(&array_view_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_array_view", r))?;

        // ---- 3. Per-layer image views (one per cascade, for framebuffer attachment) ----
        let mut layer_views = Vec::with_capacity(CSM_CASCADE_COUNT);
        for i in 0..CSM_CASCADE_COUNT {
            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(vk::Format::D32_SFLOAT)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: i as u32,
                    layer_count: 1,
                });
            // SAFETY: `d` is a valid AshDevice; each layer view references a
            // valid sub-resource of the shadow image; `None` means no custom allocator.
            let iv = unsafe { d.create_image_view(&view_info, None) }
                .map_err(|r| VulkanError::vk("create_shadow_layer_view", r))?;
            layer_views.push(iv);
        }

        // ---- 4. Sampler (PCF: COMPARE_MODE + LINEAR + CLAMP_TO_EDGE) ----
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .compare_enable(true)
            .compare_op(vk::CompareOp::LESS)
            .min_lod(0.0)
            .max_lod(1.0)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false);
        // SAFETY: `d` is a valid AshDevice; `sampler_info` describes a valid
        // sampler; `None` means no custom allocator.
        let sampler = unsafe { d.create_sampler(&sampler_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_sampler", r))?;

        // ---- 5. Render pass (depth-only, CLEAR load op) ----
        let depth_at = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
        let depth_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .depth_stencil_attachment(&depth_ref);
        // Subpass dependencies: external -> shadow (write), shadow -> external (read).
        let deps = [
            vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(vk::PipelineStageFlags::TOP_OF_PIPE)
                .dst_stage_mask(vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS)
                .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE),
            vk::SubpassDependency::default()
                .src_subpass(0)
                .dst_subpass(vk::SUBPASS_EXTERNAL)
                .src_stage_mask(vk::PipelineStageFlags::LATE_FRAGMENT_TESTS)
                .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ),
        ];
        let atts = [depth_at];
        let subpasses = [subpass];
        let rp_info = vk::RenderPassCreateInfo::default()
            .attachments(&atts)
            .subpasses(&subpasses)
            .dependencies(&deps);
        // SAFETY: `d` is a valid AshDevice; `rp_info` describes a valid render
        // pass; `None` means no custom allocator.
        let rp = unsafe { d.create_render_pass(&rp_info, None) }
            .map_err(|r| VulkanError::vk("crp_shadow", r))?;

        // ---- 6. Pipeline layout (MVP + radial geomorph parameters) ----
        let pc_range = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX,
            offset: 0,
            size: 128,
        }];
        let pll_info = vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&pc_range);
        // SAFETY: `d` is a valid AshDevice; `pll_info` describes a valid
        // pipeline layout with push constants; `None` means no custom allocator.
        let pll = unsafe { d.create_pipeline_layout(&pll_info, None) }
            .map_err(|r| VulkanError::vk("cpl_shadow", r))?;

        // ---- 7. Depth-only pipeline (no color attachments) ----
        // SAFETY: `d` is live and `mk_sm` validates the checked-in vertex SPIR-V;
        // the embedded byte slice is static.
        let vm = unsafe { mk_sm(d, crate::shaders_embedded::SHADOW_VERT_SPV)? };
        // SAFETY: `d` is live and `mk_sm` validates the checked-in fragment
        // SPIR-V; the embedded byte slice is static.
        let fm = unsafe { mk_sm(d, crate::shaders_embedded::SHADOW_FRAG_SPV)? };
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
        let stride = 32u32;
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
        ];
        let vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vb)
            .vertex_attribute_descriptions(&va);

        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let vs = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0)
            .depth_bias_enable(true)
            .depth_bias_constant_factor(1.5)
            .depth_bias_slope_factor(1.5);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // No color attachments, so use an empty blend state.
        let cba: [vk::PipelineColorBlendAttachmentState; 0] = [];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);
        let ds_state = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);

        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .depth_stencil_state(&ds_state)
            .color_blend_state(&cb)
            .dynamic_state(&ds)
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid graphics
        // pipeline (depth-only, no color attachments); `vk::PipelineCache::null()`
        // is allowed; `None` means no custom allocator.
        let pipeline =
            unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
                .map_err(|(_, r)| VulkanError::vk("cgp_shadow", r))?[0];

        // SAFETY: `vm` and `fm` were created by this device and are no longer
        // needed after pipeline creation; `None` means no custom allocator.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }

        // ---- 8. Per-cascade framebuffers ----
        let mut fbs = Vec::with_capacity(CSM_CASCADE_COUNT);
        for &layer_view in &layer_views {
            // SAFETY: `d` is a valid AshDevice; framebuffer info references a valid
            // render pass and layer image view; `None` means no custom allocator.
            let fb = unsafe {
                d.create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(rp)
                        .attachments(&[layer_view])
                        .width(SHADOW_SIZE)
                        .height(SHADOW_SIZE)
                        .layers(1),
                    None,
                )
            }
            .map_err(|r| VulkanError::vk("cfb_shadow", r))?;
            fbs.push(fb);
        }

        // ---- 9. Descriptor set layout (set=1) ----
        // binding=0: COMBINED_IMAGE_SAMPLER, VERTEX+FRAGMENT (shadow map array)
        // binding=1: COMBINED_IMAGE_SAMPLER, FRAGMENT (env cubemap)
        // binding=2: STORAGE_BUFFER, FRAGMENT (light SSBO, phase 4.3)
        let ds_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(3)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(4)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let ds_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&ds_bindings);
        // SAFETY: `d` is a valid AshDevice; `ds_layout_info` describes a valid
        // layout with two CIS bindings and one storage buffer binding; `None`
        // means no custom allocator.
        let ds_layout = unsafe { d.create_descriptor_set_layout(&ds_layout_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_ds_layout", r))?;

        // ---- 10. Descriptor pool + set ----
        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: 2, // binding 0 (shadow) + binding 1 (env cubemap)
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_BUFFER,
                descriptor_count: 3,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        // SAFETY: `d` is a valid AshDevice; `pool_info` describes a valid pool;
        // `None` means no custom allocator.
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_ds_pool", r))?;

        let ds_layouts = [ds_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&ds_layouts);
        // SAFETY: `d` is a valid AshDevice; `alloc_info` references a valid
        // pool and layout; the pool has enough capacity.
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_shadow_ds", r))?;
        let desc_set = desc_sets[0];

        // Write descriptor binding=0: shadow map array (depth image + sampler)
        let shadow_image_info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(array_image_view)
            .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(desc_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&shadow_image_info)];
        // SAFETY: `d` is a valid AshDevice; write descriptor references valid
        // descriptor set, sampler, and image view; no zero handles.
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }

        // ---- Store ----
        self.shadow_map = Some(image);
        self.shadow_map_view = Some(array_image_view);
        self.shadow_layer_views = layer_views;
        self.shadow_allocation = Some(allocation);
        self.shadow_sampler = Some(sampler);
        self.shadow_rp = Some(rp);
        self.shadow_pipeline_layout = Some(pll);
        self.shadow_pipeline = Some(pipeline);
        self.shadow_fbs = fbs;
        self.shadow_desc_layout = Some(ds_layout);
        self.shadow_desc_pool = Some(pool);
        self.shadow_desc_set = Some(desc_set);

        // ---- 11. Bind-only layout compatible through set=1 for early binding ----
        let frame_layout = self.desc_set_layout_0.ok_or(VulkanError::Loader(
            "frame descriptor layout not initialized".into(),
        ))?;
        let bind_set_layouts = [frame_layout, ds_layout];
        let bind_pli = vk::PipelineLayoutCreateInfo::default().set_layouts(&bind_set_layouts);
        // SAFETY: `d` is a valid AshDevice; `bind_pli` describes a valid layout;
        // `None` means no custom allocator.
        let bind_pll = unsafe { d.create_pipeline_layout(&bind_pli, None) }
            .map_err(|r| VulkanError::vk("cpl_shadow_bind", r))?;
        self.shadow_bind_layout = Some(bind_pll);

        Ok(())
    }

    /// Destroy all shadow mapping resources (reverse order of creation).
    pub(crate) fn destroy_shadow_resources(&mut self) {
        let d = &self.logical_device.device;

        // Descriptor pool automatically frees its descriptor sets
        if let Some(pool) = self.shadow_desc_pool.take() {
            // SAFETY: `pool` was created by this device and is still alive.
            unsafe {
                d.destroy_descriptor_pool(pool, None);
            }
        }
        if let Some(layout) = self.shadow_desc_layout.take() {
            // SAFETY: `layout` was created by this device and is still alive.
            unsafe {
                d.destroy_descriptor_set_layout(layout, None);
            }
        }
        if let Some(layout) = self.shadow_bind_layout.take() {
            // SAFETY: `layout` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline_layout(layout, None);
            }
        }
        for fb in self.shadow_fbs.drain(..) {
            // SAFETY: `fb` was created by this device and is still alive.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        if let Some(p) = self.shadow_pipeline.take() {
            // SAFETY: `p` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.shadow_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(rp) = self.shadow_rp.take() {
            // SAFETY: `rp` was created by this device and is still alive.
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        if let Some(s) = self.shadow_sampler.take() {
            // SAFETY: `s` was created by this device and is still alive.
            unsafe {
                d.destroy_sampler(s, None);
            }
        }
        for iv in self.shadow_layer_views.drain(..) {
            // SAFETY: `iv` was created by this device and is still alive.
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(iv) = self.shadow_map_view.take() {
            // SAFETY: `iv` was created by this device and is still alive.
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(img) = self.shadow_map.take() {
            // SAFETY: `img` was created by this device and is still alive.
            unsafe {
                d.destroy_image(img, None);
            }
        }
        if let Some(mut a) = self.shadow_allocation.take() {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                guard.free(&mut a);
            }
        }
    }
}
