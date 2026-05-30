//! Shadow mapping for VulkanDevice (directional light, 2048×2048).

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{mk_sm, VulkanDevice};

impl VulkanDevice {
    /// Ensure shadow mapping resources exist (idempotent).
    pub(crate) fn ensure_shadow(&mut self) -> VkResult<()> {
        if self.shadow_map.is_some() {
            return Ok(());
        }
        self.create_shadow_resources()
    }

    /// Create 2048×2048 directional-light shadow mapping resources.
    fn create_shadow_resources(&mut self) -> VkResult<()> {
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();
        const SHADOW_SIZE: u32 = 2048;

        // ---- 1. Shadow map image (D32_SFLOAT, GPU-only) ----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .extent(vk::Extent3D {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is a valid AshDevice; `image_info` describes a valid
        // 2D depth image; `None` means no custom allocator.
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
                linear: false,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: `image` was created by this device; `allocation` was created
        // for this image's memory requirements; the memory and offset are valid.
        unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_shadow_image", r))?;

        // ---- 2. Image view ----
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: `d` is a valid AshDevice; `view_info` references a valid
        // image and subresource range; `None` means no custom allocator.
        let image_view = unsafe { d.create_image_view(&view_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_image_view", r))?;

        // ---- 3. Sampler (PCF: COMPARE_MODE + LINEAR + CLAMP_TO_EDGE) ----
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

        // ---- 4. Render pass (depth-only, CLEAR load op) ----
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
        // Subpass dependencies: external → shadow (write), shadow → external (read)
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

        // ---- 5. Pipeline layout (push constant: mat4 = 64 bytes) ----
        let pc_range = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX,
            offset: 0,
            size: 64,
        }];
        let pll_info = vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&pc_range);
        // SAFETY: `d` is a valid AshDevice; `pll_info` describes a valid
        // pipeline layout with push constants; `None` means no custom allocator.
        let pll = unsafe { d.create_pipeline_layout(&pll_info, None) }
            .map_err(|r| VulkanError::vk("cpl_shadow", r))?;

        // ---- 6. Depth-only pipeline (no color attachments) ----
        let vm = unsafe { mk_sm(d, crate::shaders_embedded::SHADOW_VERT_SPV)? };
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
        let va = [vk::VertexInputAttributeDescription {
            location: 0,
            binding: 0,
            format: vk::Format::R32G32B32_SFLOAT,
            offset: 0,
        }];
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
        // No color attachments – empty blend state
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

        // ---- 7. Framebuffer ----
        // SAFETY: `d` is a valid AshDevice; framebuffer info references a valid
        // render pass and image view; `None` means no custom allocator.
        let fb = unsafe {
            d.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(rp)
                    .attachments(&[image_view])
                    .width(SHADOW_SIZE)
                    .height(SHADOW_SIZE)
                    .layers(1),
                None,
            )
        }
        .map_err(|r| VulkanError::vk("cfb_shadow", r))?;

        // ---- 8. Descriptor set layout (set=1) ----
        // binding=0: COMBINED_IMAGE_SAMPLER, VERTEX+FRAGMENT (shadow map)
        // binding=1: COMBINED_IMAGE_SAMPLER, FRAGMENT (env cubemap)
        let ds_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(
                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                ),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let ds_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&ds_bindings);
        // SAFETY: `d` is a valid AshDevice; `ds_layout_info` describes a valid
        // layout with two combined image sampler bindings; `None` means no custom
        // allocator.
        let ds_layout = unsafe { d.create_descriptor_set_layout(&ds_layout_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_ds_layout", r))?;

        // ---- 9. Descriptor pool + set ----
        let pool_sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: 2, // binding 0 (shadow) + binding 1 (env cubemap)
        }];
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

        // Write descriptor binding=0: shadow map (depth image + sampler)
        let shadow_image_info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(image_view)
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
        self.shadow_map_view = Some(image_view);
        self.shadow_allocation = Some(allocation);
        self.shadow_sampler = Some(sampler);
        self.shadow_rp = Some(rp);
        self.shadow_pipeline_layout = Some(pll);
        self.shadow_pipeline = Some(pipeline);
        self.shadow_fb = Some(fb);
        self.shadow_desc_layout = Some(ds_layout);
        self.shadow_desc_pool = Some(pool);
        self.shadow_desc_set = Some(desc_set);

        // ---- 10. Bind-only pipeline layout (set=1 only, for early binding in begin_frame) ----
        let bind_set_layouts = [ds_layout];
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
        if let Some(fb) = self.shadow_fb.take() {
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

    /// Compute a light view-projection matrix for directional shadow mapping.
    pub(crate) fn compute_light_mvp(&self) -> [[f32; 4]; 4] {
        let light_dir = glam::Vec3::new(0.5, -0.707, 0.5).normalize();
        // Position the light far away, looking at the origin
        let light_pos = -light_dir * 10.0;
        let view = glam::Mat4::look_at_rh(light_pos, glam::Vec3::ZERO, glam::Vec3::Y);
        let ortho = glam::Mat4::orthographic_rh(-5.0, 5.0, -5.0, 5.0, 0.1, 20.0);
        let light_mvp = ortho * view;
        light_mvp.to_cols_array_2d()
    }

    /// Record a shadow-mapping render pass into the already-begun command buffer.
    ///
    /// The command buffer MUST have been started via [`begin_cb`] before calling
    /// this method. The shadow map is bound as a depth attachment and the scene
    /// is rendered from the light's point of view using the given `light_mvp`
    /// push constant.
    pub(crate) fn record_shadow_pass(
        &self,
        fi: usize,
        light_mvp: &[[f32; 4]; 4],
        vertex_buf: render_core::BufferHandle,
        index_buf: render_core::BufferHandle,
        index_count: u32,
    ) -> VkResult<()> {
        let d = &self.logical_device.device;
        let f = &self.frame_sync[fi];
        let rp = self.shadow_rp.ok_or(VulkanError::Loader(
            "shadow render pass not initialized".into(),
        ))?;
        let pl = self.shadow_pipeline.ok_or(VulkanError::Loader(
            "shadow pipeline not initialized".into(),
        ))?;
        let pll = self.shadow_pipeline_layout.ok_or(VulkanError::Loader(
            "shadow pipeline layout not initialized".into(),
        ))?;
        const SHADOW_SIZE: u32 = 2048;

        // Look up Vulkan buffer handles
        let vk_vb = self
            .buffers
            .get(vertex_buf.index, vertex_buf.generation)
            .map(|e| e.buffer)
            .ok_or(VulkanError::Loader("vertex buffer not found".into()))?;
        let vk_ib = self
            .buffers
            .get(index_buf.index, index_buf.generation)
            .map(|e| e.buffer)
            .ok_or(VulkanError::Loader("index buffer not found".into()))?;

        // Begin shadow render pass
        let clear_depth = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };
        let clear_values = [clear_depth];
        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(rp)
            .framebuffer(self.shadow_fb.ok_or(VulkanError::Loader(
                "shadow framebuffer not initialized".into(),
            ))?)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: SHADOW_SIZE,
                    height: SHADOW_SIZE,
                },
            })
            .clear_values(&clear_values);

        // SAFETY: command buffer is in recording state; render pass and
        // framebuffer are valid; `SubpassContents::INLINE` is correct.
        unsafe {
            d.cmd_begin_render_pass(f.command_buffer, &rpbi, vk::SubpassContents::INLINE);
        }

        // Viewport + scissor
        let vp = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: SHADOW_SIZE as f32,
            height: SHADOW_SIZE as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        // SAFETY: command buffer is inside a render pass instance; all handles
        // (pipeline, push constants, buffers) are valid; `light_mvp` is a
        // stack-local array valid for the duration of the unsafe block.
        unsafe {
            d.cmd_set_viewport(f.command_buffer, 0, &[vp]);
            d.cmd_set_scissor(
                f.command_buffer,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D {
                        width: SHADOW_SIZE,
                        height: SHADOW_SIZE,
                    },
                }],
            );
            d.cmd_bind_pipeline(f.command_buffer, vk::PipelineBindPoint::GRAPHICS, pl);

            // Push constant: mat4 light MVP (64 bytes)
            // SAFETY: `light_mvp` pointer is valid for the size of the matrix;
            // push constant range (64 bytes at offset 0) matches the pipeline
            // layout declaration.
            let mvp_bytes: &[u8] = std::slice::from_raw_parts(
                light_mvp.as_ptr() as *const u8,
                std::mem::size_of::<[[f32; 4]; 4]>(),
            );
            d.cmd_push_constants(
                f.command_buffer,
                pll,
                vk::ShaderStageFlags::VERTEX,
                0,
                mvp_bytes,
            );

            // Vertex + index buffers
            let vbs = [vk_vb];
            let offsets = [0u64];
            d.cmd_bind_vertex_buffers(f.command_buffer, 0, &vbs, &offsets);
            d.cmd_bind_index_buffer(f.command_buffer, vk_ib, 0, vk::IndexType::UINT32);
            d.cmd_draw_indexed(f.command_buffer, index_count, 1, 0, 0, 0);
            d.cmd_end_render_pass(f.command_buffer);
        }

        // Pipeline barrier: make shadow depth writes visible to fragment shader reads
        // in the subsequent forward render pass.
        let barrier = vk::ImageMemoryBarrier::default()
            .image(self.shadow_map.ok_or(VulkanError::Loader(
                "shadow map image not initialized".into(),
            ))?)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
        // SAFETY: command buffer is still in recording state (not yet ended);
        // the barrier references a valid image; stage masks are correct for
        // depth write → shader read transition.
        unsafe {
            d.cmd_pipeline_barrier(
                f.command_buffer,
                vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }

        Ok(())
    }
}
