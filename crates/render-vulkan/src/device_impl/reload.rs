//! VulkanDevice methods used by the GPU resource reload coordinator.
//!
//! These are separated from the pipeline/texture creation code to keep
//! concerns distinct — the reload path creates new resources, atomically
//! swaps them into the device state, and returns the old handles so the
//! coordinator can keep them alive for the required number of frames.

use ash::vk;

use crate::allocator::Allocation;
use crate::error::{VkResult, VulkanError};

use super::VulkanDevice;

impl VulkanDevice {
    // ------------------------------------------------------------------
    // Texture helpers
    // ------------------------------------------------------------------

    /// Create a sampled 2D texture from CPU pixel data.
    ///
    /// 1. Creates a `VkImage` with `TRANSFER_DST | SAMPLED` usage.
    /// 2. Allocates and binds GPU memory.
    /// 3. Uploads pixel data via a one-shot staging buffer + command buffer.
    /// 4. Creates a `VkImageView` for shader sampling.
    ///
    /// Returns `(image, image_view, allocation)`.
    pub(crate) fn create_sampled_texture(
        &self,
        width: u32,
        height: u32,
        mip_count: u8,
        data: &[u8],
    ) -> VkResult<(vk::Image, vk::ImageView, Allocation)> {
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();

        // ── 1. Create image ────────────────────────────────────────────
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(mip_count as u32)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is a valid AshDevice; `image_info` describes a valid
        // 2D image; `None` means no custom allocator.
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|r| VulkanError::vk("create_image (reload)", r))?;

        // ── 2. Allocate and bind memory ────────────────────────────────
        // SAFETY: `image` was just created by this device.
        let req = unsafe { d.get_image_memory_requirements(image) };
        let allocation = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "reload-texture",
                requirements: req,
                location: crate::allocator::MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: `image` created by this device; `allocation` was created
        // for this image's memory requirements.
        unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_image (reload)", r))?;

        // ── 3. Stage pixel data via staging buffer ─────────────────────
        // Create a temporary staging buffer (CpuToGpu).
        let staging_size = data.len() as vk::DeviceSize;
        let staging_bi = vk::BufferCreateInfo::default()
            .size(staging_size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is valid; `staging_bi` describes a valid buffer.
        let staging_buf = unsafe { d.create_buffer(&staging_bi, None) }
            .map_err(|r| VulkanError::vk("create_staging_buf (reload)", r))?;
        let staging_req = unsafe { d.get_buffer_memory_requirements(staging_buf) };
        let mut staging_alloc = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "reload-staging",
                requirements: staging_req,
                location: crate::allocator::MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: `staging_buf` created by this device; allocation valid.
        unsafe {
            d.bind_buffer_memory(staging_buf, staging_alloc.memory(), staging_alloc.offset())
        }
        .map_err(|r| VulkanError::vk("bind_staging (reload)", r))?;

        // Copy pixel data into staging buffer.
        if let Some(slice) = staging_alloc.mapped_slice_mut() {
            let copy_len = data.len().min(slice.len());
            slice[..copy_len].copy_from_slice(&data[..copy_len]);
        }

        // ── 4. One-shot command buffer: layout transition + copy ───────
        // SAFETY: we create a temporary command pool + buffer for a single
        // submit.  All handles are valid.
        let cp = unsafe {
            d.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(self.logical_device.queue_family_index)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )
        }
        .map_err(|r| VulkanError::vk("create_cp (reload)", r))?;
        let cbs = unsafe {
            d.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(cp)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(|r| VulkanError::vk("alloc_cb (reload)", r))?;
        let cb = cbs[0];

        // SAFETY: transient pool gives us a fresh command buffer.
        unsafe {
            d.begin_command_buffer(
                cb,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(|r| VulkanError::vk("begin_cb (reload)", r))?;

            // Transition image: UNDEFINED → TRANSFER_DST_OPTIMAL
            let barrier_pre = vk::ImageMemoryBarrier::default()
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: mip_count as u32,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL);
            d.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_pre],
            );

            // Copy staging buffer → image (base mip level).
            let region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                });
            d.cmd_copy_buffer_to_image(
                cb,
                staging_buf,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );

            // Transition image: TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
            let barrier_post = vk::ImageMemoryBarrier::default()
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: mip_count as u32,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            d.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_post],
            );

            d.end_command_buffer(cb)
                .map_err(|r| VulkanError::vk("end_cb (reload)", r))?;
        }

        // Submit and wait (one-shot).
        let fence_info = vk::FenceCreateInfo::default();
        // SAFETY: `d` is valid; `fence_info` describes a default fence.
        let fence = unsafe { d.create_fence(&fence_info, None) }
            .map_err(|r| VulkanError::vk("create_fence (reload)", r))?;
        let cmd_bufs = [cb];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
        // SAFETY: queue is valid; submit info is correctly structured.
        unsafe {
            d.queue_submit(self.logical_device.queue, &[submit_info], fence)
                .map_err(|r| VulkanError::vk("queue_submit (reload)", r))?;
            d.wait_for_fences(&[fence], true, u64::MAX)
                .map_err(|r| VulkanError::vk("wait_fences (reload)", r))?;
            d.destroy_fence(fence, None);
        }

        // ── 5. Clean up staging resources ──────────────────────────────
        // SAFETY: upload has completed; staging resources are no longer needed.
        unsafe {
            d.destroy_buffer(staging_buf, None);
            d.destroy_command_pool(cp, None);
        }
        // Free staging allocation.
        allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .free(&mut staging_alloc);

        // ── 6. Create image view ───────────────────────────────────────
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: mip_count as u32,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: `d` is valid; `view_info` references a valid image.
        let image_view = unsafe { d.create_image_view(&view_info, None) }
            .map_err(|r| VulkanError::vk("create_image_view (reload)", r))?;

        Ok((image, image_view, allocation))
    }

    /// Destroy a sampled texture (image, view, allocation).
    pub(crate) fn destroy_sampled_texture(
        &self,
        image: vk::Image,
        image_view: vk::ImageView,
        mut allocation: Allocation,
    ) {
        let d = &self.logical_device.device;
        // SAFETY: handles were created by this device.
        unsafe {
            d.destroy_image_view(image_view, None);
            d.destroy_image(image, None);
        }
        if let Ok(mut guard) = self.logical_device.allocator().lock() {
            guard.free(&mut allocation);
        }
    }

    // ------------------------------------------------------------------
    // Shadow-map swap
    // ------------------------------------------------------------------

    /// Atomically replace the CSM shadow-map texture (3-layer array).
    ///
    /// Takes the new 3-layer image, layered view (for descriptor), per-layer views
    /// (for framebuffers), and allocation. Recreates cascade framebuffers internally.
    ///
    /// Returns the **old** resources `(image, layered_view, [layer_views], allocation,
    /// [framebuffers])` so the caller can queue them for deferred destruction.
    pub(crate) fn replace_shadow_map(
        &mut self,
        new_image: vk::Image,
        new_layered_view: vk::ImageView,
        new_layer_views: Vec<vk::ImageView>,
        new_allocation: Allocation,
    ) -> Option<(
        vk::Image,
        vk::ImageView,
        Vec<vk::ImageView>,
        Option<Allocation>,
        Vec<vk::Framebuffer>,
    )> {
        let old_image = self.shadow_map.take();
        let old_layered_view = self.shadow_map_view.take();
        let old_layer_views = std::mem::take(&mut self.shadow_layer_views);
        let old_alloc = self.shadow_allocation.take();
        let old_fbs = std::mem::take(&mut self.shadow_fbs);

        self.shadow_map = Some(new_image);
        self.shadow_map_view = Some(new_layered_view);
        self.shadow_layer_views = new_layer_views;
        self.shadow_allocation = Some(new_allocation);

        // Recreate cascade framebuffers from the new layer views.
        if let Some(rp) = self.shadow_rp {
            let mut new_fbs = Vec::with_capacity(self.shadow_layer_views.len());
            for &lv in &self.shadow_layer_views {
                // SAFETY: device is valid; framebuffer info references valid
                // render pass and layer image view.
                let fb = unsafe {
                    self.logical_device.device.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(rp)
                            .attachments(&[lv])
                            .width(2048)
                            .height(2048)
                            .layers(1),
                        None,
                    )
                }
                .ok()?;
                new_fbs.push(fb);
            }
            self.shadow_fbs = new_fbs;
        }

        // Update the shadow descriptor set to point at the new layered view.
        if let (Some(ds), Some(sampler)) = (self.shadow_desc_set, self.shadow_sampler) {
            let image_info = [vk::DescriptorImageInfo::default()
                .sampler(sampler)
                .image_view(self.shadow_map_view.unwrap_or(vk::ImageView::null()))
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
            let writes = [vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info)];
            // SAFETY: device is valid; descriptor set and sampler are valid.
            unsafe {
                self.logical_device
                    .device
                    .update_descriptor_sets(&writes, &[]);
            }
        }

        if let (Some(img), Some(vw)) = (old_image, old_layered_view) {
            Some((img, vw, old_layer_views, old_alloc, old_fbs))
        } else {
            None
        }
    }

    // ------------------------------------------------------------------
    // Pipeline recreation
    // ------------------------------------------------------------------

    /// Recreate the MVP triangle pipeline using new SPIR-V.
    ///
    /// Returns the **old** `(pipeline, pipeline_layout)` for deferred
    /// destruction.  On failure the old pipeline is kept.
    ///
    /// The SPIR-V byte slices are leaked into `'static` so they can be
    /// stored on the device (which expects `&'static [u8]`).  This is
    /// acceptable for hot-reload (small, infrequent allocations).
    pub(crate) fn recreate_mvp_pipeline(
        &mut self,
        vert_spirv: &[u8],
        frag_spirv: &[u8],
    ) -> VkResult<(vk::Pipeline, vk::PipelineLayout)> {
        let old_pipeline = self
            .mvp_pipeline
            .ok_or(VulkanError::Loader("MVP pipeline not created yet".into()))?;
        let old_layout = self.mvp_pipeline_layout.ok_or(VulkanError::Loader(
            "MVP pipeline layout not created".into(),
        ))?;

        let saved_vert = self.mvp_vert_spv.replace(vert_spirv.to_vec());
        let saved_frag = self.mvp_frag_spv.replace(frag_spirv.to_vec());

        // Rebuild (this reads self.mvp_vert_spv / mvp_frag_spv).
        match self.build_mvp() {
            Ok(()) => Ok((old_pipeline, old_layout)),
            Err(e) => {
                self.mvp_vert_spv = saved_vert;
                self.mvp_frag_spv = saved_frag;
                self.mvp_pipeline = Some(old_pipeline);
                self.mvp_pipeline_layout = Some(old_layout);
                Err(e)
            }
        }
    }

    /// Recreate the model forward pipeline using new SPIR-V.
    ///
    /// Returns the **old** `(pipeline, pipeline_layout)` for deferred
    /// destruction.  On failure the old pipeline is kept.
    pub(crate) fn recreate_model_pipeline(
        &mut self,
        vert_spirv: &[u8],
        frag_spirv: &[u8],
    ) -> VkResult<(vk::Pipeline, vk::PipelineLayout)> {
        let old_pipeline = self
            .model_pipeline
            .ok_or(VulkanError::Loader("model pipeline not created yet".into()))?;
        let old_layout = self.model_pipeline_layout.ok_or(VulkanError::Loader(
            "model pipeline layout not created".into(),
        ))?;

        let saved_vert = self.mvp_vert_spv.replace(vert_spirv.to_vec());
        let saved_frag = self.mvp_frag_spv.replace(frag_spirv.to_vec());

        match self.build_model_pipeline() {
            Ok(()) => Ok((old_pipeline, old_layout)),
            Err(e) => {
                self.mvp_vert_spv = saved_vert;
                self.mvp_frag_spv = saved_frag;
                self.model_pipeline = Some(old_pipeline);
                self.model_pipeline_layout = Some(old_layout);
                Err(e)
            }
        }
    }

    /// Recreate the shadow-mapping pipeline using new SPIR-V.
    ///
    /// Returns the **old** `(pipeline, pipeline_layout)` for deferred
    /// destruction.  On failure the old pipeline is kept.
    pub(crate) fn recreate_shadow_pipeline(
        &mut self,
        vert_spirv: &[u8],
        frag_spirv: &[u8],
    ) -> VkResult<(vk::Pipeline, vk::PipelineLayout)> {
        let old_pipeline = self.shadow_pipeline.ok_or(VulkanError::Loader(
            "shadow pipeline not created yet".into(),
        ))?;
        let old_layout = self.shadow_pipeline_layout.ok_or(VulkanError::Loader(
            "shadow pipeline layout not created".into(),
        ))?;

        // ── Create new shader modules ──────────────────────────────────
        let d = &self.logical_device.device;
        // SAFETY: `d` is a valid AshDevice; bytecode is valid SPIR-V.
        let vm = unsafe { super::mk_sm(d, vert_spirv)? };
        let fm = unsafe { super::mk_sm(d, frag_spirv)? };

        // ── Reuse the existing render pass for the shadow pipeline ─────
        let rp = self
            .shadow_rp
            .ok_or(VulkanError::Loader("shadow render pass not found".into()))?;

        // ── Create new pipeline layout (same as old) ───────────────────
        let pc_range = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX,
            offset: 0,
            size: 64,
        }];
        let pll_info = vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&pc_range);
        // SAFETY: `d` is a valid AshDevice.
        let new_layout = unsafe { d.create_pipeline_layout(&pll_info, None) }
            .map_err(|r| VulkanError::vk("cpl_shadow_reload", r))?;

        // ── Build the new pipeline ─────────────────────────────────────
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
            .layout(new_layout)
            .render_pass(rp)
            .subpass(0);

        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid
        // graphics pipeline; `vk::PipelineCache::null()` is allowed.
        let new_pipeline =
            unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
                .map_err(|(_, r)| {
                    // Destroy the new layout since we failed.
                    unsafe {
                        d.destroy_pipeline_layout(new_layout, None);
                    }
                    // SAFETY: shader modules were created above.
                    unsafe {
                        d.destroy_shader_module(vm, None);
                        d.destroy_shader_module(fm, None);
                    }
                    VulkanError::vk("cgp_shadow_reload", r)
                })?[0];

        // SAFETY: shader modules are no longer needed after pipeline creation.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }

        // Store the new pipeline/layout on the device.
        self.shadow_pipeline = Some(new_pipeline);
        self.shadow_pipeline_layout = Some(new_layout);

        Ok((old_pipeline, old_layout))
    }

    // ------------------------------------------------------------------
    // Frame-boundary reload processing
    // ------------------------------------------------------------------

    /// Process pending reloads at the start of a frame (before acquire).
    ///
    /// Calls [`GpuReloadCoordinator::apply_next`] for one pending reload, if
    /// any.
    pub fn process_reloads(
        &mut self,
        coordinator: &mut crate::reload::GpuReloadCoordinator,
    ) -> Result<bool, VulkanError> {
        coordinator.apply_next(self)
    }
}
