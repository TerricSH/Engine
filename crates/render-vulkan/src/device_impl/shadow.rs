//! Shadow mapping for VulkanDevice (directional light CSM, 2048 x 2048, 3 cascades).

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{mk_sm, VulkanDevice};

/// Number of CSM cascades.
pub(crate) const CSM_CASCADE_COUNT: usize = 3;

/// Validation failures produced while deriving camera or directional-light
/// data for cascaded shadow maps.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CascadeDataError {
    #[error("projection matrix contains non-finite values")]
    NonFiniteProjection,
    #[error("projection matrix is not a supported right-handed Vulkan zero-to-one projection")]
    UnsupportedProjection,
    #[error("projection matrix does not encode finite positive near/far planes")]
    InvalidClipPlanes,
    #[error("view matrix contains non-finite values")]
    NonFiniteView,
    #[error("view matrix is not invertible")]
    NonInvertibleView,
    #[error("directional shadow light direction must be finite and non-zero")]
    InvalidLightDirection,
    #[error("camera frustum cannot be converted into finite cascade bounds")]
    DegenerateFrustum,
}

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

            let fence = unsafe { d.create_fence(&vk::FenceCreateInfo::default(), None) }
                .map_err(|result| VulkanError::vk("create_shadow_init_fence", result))?;
            let command_buffers = [command_buffer];
            let submit = [vk::SubmitInfo::default().command_buffers(&command_buffers)];
            let completion =
                match unsafe { d.queue_submit(self.logical_device.queue, &submit, fence) } {
                    Ok(()) => unsafe { d.wait_for_fences(&[fence], true, u64::MAX) }
                        .map_err(|result| VulkanError::vk("wait_shadow_init_fence", result)),
                    Err(result) => Err(VulkanError::vk("submit_shadow_init", result)),
                };
            if completion.is_err() {
                let _ = unsafe { d.queue_wait_idle(self.logical_device.queue) };
            }
            unsafe { d.destroy_fence(fence, None) };
            completion
        })();

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
        // Subpass dependencies: external 鈫?shadow (write), shadow 鈫?external (read)
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

        // ---- 6. Pipeline layout (push constant: mat4 = 64 bytes) ----
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

        // ---- 7. Depth-only pipeline (no color attachments) ----
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
                descriptor_count: 1, // binding 2 (light SSBO)
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

    /// Derive finite clip distances from a canonical right-handed Vulkan
    /// zero-to-one projection matrix.
    ///
    /// Both perspective and orthographic matrices are supported, including
    /// reversed-Z variants. Infinite projections are rejected because finite
    /// CSM partitions require a real far plane.
    pub(crate) fn derive_rh_zo_clip_planes(
        projection: &glam::Mat4,
    ) -> Result<(f32, f32), CascadeDataError> {
        const MATRIX_EPSILON: f32 = 1.0e-5;
        const DENOMINATOR_EPSILON: f32 = 1.0e-8;

        if !projection
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(CascadeDataError::NonFiniteProjection);
        }

        // Canonical RH projections make clip.w depend only on view-space z
        // (perspective) or remain one (orthographic). Reject arbitrary/oblique
        // matrices whose clip distances cannot be recovered by these formulas.
        if projection.x_axis.w.abs() > MATRIX_EPSILON || projection.y_axis.w.abs() > MATRIX_EPSILON
        {
            return Err(CascadeDataError::UnsupportedProjection);
        }

        let a = projection.z_axis.z;
        let b = projection.w_axis.z;
        let (depth_at_zero, depth_at_one) = if (projection.z_axis.w + 1.0).abs() <= MATRIX_EPSILON
            && projection.w_axis.w.abs() <= MATRIX_EPSILON
        {
            // Perspective: ndc_z(d) = -a + b / d, where d = -view_z.
            if a.abs() <= DENOMINATOR_EPSILON || (a + 1.0).abs() <= DENOMINATOR_EPSILON {
                return Err(CascadeDataError::InvalidClipPlanes);
            }
            (b / a, b / (a + 1.0))
        } else if projection.z_axis.w.abs() <= MATRIX_EPSILON
            && (projection.w_axis.w - 1.0).abs() <= MATRIX_EPSILON
        {
            // Orthographic: ndc_z(d) = -a*d + b.
            if a.abs() <= DENOMINATOR_EPSILON {
                return Err(CascadeDataError::InvalidClipPlanes);
            }
            (b / a, (b - 1.0) / a)
        } else {
            return Err(CascadeDataError::UnsupportedProjection);
        };

        let near = depth_at_zero.min(depth_at_one);
        let far = depth_at_zero.max(depth_at_one);
        if !near.is_finite()
            || !far.is_finite()
            || near <= DENOMINATOR_EPSILON
            || far <= near + DENOMINATOR_EPSILON
        {
            return Err(CascadeDataError::InvalidClipPlanes);
        }

        Ok((near, far))
    }

    /// Validate and normalize a directional shadow light vector.
    pub(crate) fn normalize_shadow_light_direction(
        direction: glam::Vec3,
    ) -> Result<glam::Vec3, CascadeDataError> {
        let length_squared = direction.length_squared();
        if !direction.is_finite() || !length_squared.is_finite() || length_squared <= 1.0e-12 {
            return Err(CascadeDataError::InvalidLightDirection);
        }
        Ok(direction / length_squared.sqrt())
    }

    /// Compute PSSM cascade split distances in view-space z.
    ///
    /// Returns `[split0, split1, split2]` where `split_i` is the far plane
    /// of cascade `i` (i.e. the distance from the camera in view-space
    /// negative-z direction). Cascade 0 covers `[near..split0]`,
    /// cascade 1 covers `[split0..split1]`, cascade 2 covers `[split1..far]`.
    ///
    /// Uses a practical lambda-blend of logarithmic and uniform partitioning.
    pub(crate) fn compute_cascade_splits(near: f32, far: f32) -> [f32; 3] {
        let lambda = 0.95f32; // bias toward logarithmic
        let mut splits = [0.0f32; 3];
        for (i, split) in splits.iter_mut().enumerate() {
            let t = (i + 1) as f32 / 3.0;
            let log_split = near * (far / near).powf(t);
            let uniform_split = near + (far - near) * t;
            *split = lambda * log_split + (1.0 - lambda) * uniform_split;
        }
        splits
    }

    /// Compute CSM cascade light view-projection matrices.
    ///
    /// Given the camera's view and projection matrices, and the near/far
    /// plane distances, returns:
    /// - `cascade_splits`: `[split0, split1, split2, far]` split distances
    ///   in view-space z
    /// - `light_vps`: 3 light view-projection matrices, one per cascade
    ///
    /// Each cascade's light VP is an orthographic projection that tightly
    /// bounds the corresponding frustum slice when viewed from the (fixed)
    /// light direction.
    pub(crate) fn compute_cascade_data(
        view_matrix: &glam::Mat4,
        proj_matrix: &glam::Mat4,
        near: f32,
        far: f32,
        light_direction: glam::Vec3,
    ) -> Result<([f32; 4], [glam::Mat4; 3]), CascadeDataError> {
        use glam::Vec3;

        if !view_matrix
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(CascadeDataError::NonFiniteView);
        }
        let view_determinant = view_matrix.determinant();
        if !view_determinant.is_finite() || view_determinant == 0.0 {
            return Err(CascadeDataError::NonInvertibleView);
        }
        if !proj_matrix
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(CascadeDataError::NonFiniteProjection);
        }
        let projection_determinant = proj_matrix.determinant();
        if !projection_determinant.is_finite() || projection_determinant == 0.0 {
            return Err(CascadeDataError::UnsupportedProjection);
        }
        if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
            return Err(CascadeDataError::InvalidClipPlanes);
        }
        let light_dir = Self::normalize_shadow_light_direction(light_direction)?;

        let splits = Self::compute_cascade_splits(near, far);
        let splits4: [f32; 4] = [splits[0], splits[1], splits[2], far];

        let inv_view = view_matrix.inverse();
        let inv_proj = proj_matrix.inverse();
        if !inv_view
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite())
            || !inv_proj
                .to_cols_array()
                .iter()
                .all(|value| value.is_finite())
        {
            return Err(CascadeDataError::DegenerateFrustum);
        }

        // Unproject both Vulkan depth endpoints. Sorting by positive view-space
        // distance makes the same code work for perspective, orthographic and
        // reversed-Z projections.
        let ndc_xy = [
            glam::vec2(-1.0, -1.0),
            glam::vec2(1.0, -1.0),
            glam::vec2(1.0, 1.0),
            glam::vec2(-1.0, 1.0),
        ];
        let mut frustum_edges = [(Vec3::ZERO, Vec3::ZERO); 4];
        for (index, xy) in ndc_xy.iter().copied().enumerate() {
            let endpoint_zero = inv_proj * glam::vec4(xy.x, xy.y, 0.0, 1.0);
            let endpoint_one = inv_proj * glam::vec4(xy.x, xy.y, 1.0, 1.0);
            if !endpoint_zero.is_finite()
                || !endpoint_one.is_finite()
                || endpoint_zero.w.abs() <= 1.0e-8
                || endpoint_one.w.abs() <= 1.0e-8
            {
                return Err(CascadeDataError::DegenerateFrustum);
            }
            let point_zero = endpoint_zero.truncate() / endpoint_zero.w;
            let point_one = endpoint_one.truncate() / endpoint_one.w;
            let distance_zero = -point_zero.z;
            let distance_one = -point_one.z;
            if !point_zero.is_finite()
                || !point_one.is_finite()
                || !distance_zero.is_finite()
                || !distance_one.is_finite()
                || distance_zero <= 0.0
                || distance_one <= 0.0
            {
                return Err(CascadeDataError::DegenerateFrustum);
            }
            frustum_edges[index] = if distance_zero <= distance_one {
                (point_zero, point_one)
            } else {
                (point_one, point_zero)
            };
        }

        let mut light_vps = [glam::Mat4::IDENTITY; 3];
        let mut prev_split_z = near;

        for cascade in 0..3 {
            let split_z = splits[cascade];
            let near_t = (prev_split_z - near) / (far - near);
            let far_t = (split_z - near) / (far - near);

            // Compute world-space AABB of the cascade frustum slice.
            let mut min_ws = Vec3::splat(f32::MAX);
            let mut max_ws = Vec3::splat(f32::MIN);
            let mut world_corners = [Vec3::ZERO; 8];
            for (edge_index, (near_corner, far_corner)) in frustum_edges.iter().copied().enumerate()
            {
                let slice_near = near_corner.lerp(far_corner, near_t);
                let slice_far = near_corner.lerp(far_corner, far_t);
                let p_near = inv_view * slice_near.extend(1.0);
                let p_far = inv_view * slice_far.extend(1.0);
                if !p_near.is_finite()
                    || !p_far.is_finite()
                    || p_near.w.abs() <= 1.0e-8
                    || p_far.w.abs() <= 1.0e-8
                {
                    return Err(CascadeDataError::DegenerateFrustum);
                }
                let ws_near = p_near.truncate() / p_near.w;
                let ws_far = p_far.truncate() / p_far.w;
                world_corners[edge_index] = ws_near;
                world_corners[edge_index + 4] = ws_far;

                min_ws = min_ws.min(ws_near).min(ws_far);
                max_ws = max_ws.max(ws_near).max(ws_far);
            }

            // Compute light view at the center of the frustum AABB.
            let center = (min_ws + max_ws) * 0.5;
            let radius = (max_ws - min_ws).length() * 0.5;
            if !center.is_finite() || !radius.is_finite() || radius <= 1.0e-6 {
                return Err(CascadeDataError::DegenerateFrustum);
            }
            let light_pos = center - light_dir * (radius + 1.0);
            let up = if light_dir.dot(Vec3::Y).abs() > 0.99 {
                Vec3::Z
            } else {
                Vec3::Y
            };
            let light_view = glam::Mat4::look_at_rh(light_pos, center, up);

            // Compute tight orthographic bounds in light space.
            let mut ls_min = Vec3::splat(f32::MAX);
            let mut ls_max = Vec3::splat(f32::MIN);
            for corner in world_corners {
                let light_space = (light_view * corner.extend(1.0)).truncate();
                if !light_space.is_finite() {
                    return Err(CascadeDataError::DegenerateFrustum);
                }
                ls_min = ls_min.min(light_space);
                ls_max = ls_max.max(light_space);
            }

            // Add a proportional guard band. Light-space Z is negative in
            // front of the RH light camera, hence the sign conversion below.
            let width = ls_max.x - ls_min.x;
            let height = ls_max.y - ls_min.y;
            let depth = ls_max.z - ls_min.z;
            if !width.is_finite()
                || !height.is_finite()
                || !depth.is_finite()
                || width <= 1.0e-6
                || height <= 1.0e-6
                || depth <= 1.0e-6
            {
                return Err(CascadeDataError::DegenerateFrustum);
            }
            let pad_x = (width * 0.025).max(1.0e-3);
            let pad_y = (height * 0.025).max(1.0e-3);
            let pad_z = (depth * 0.025).max(1.0e-3);
            let light_near = (-ls_max.z - pad_z).max(1.0e-4);
            let light_far = (-ls_min.z + pad_z).max(light_near + 1.0e-3);

            let ortho = glam::Mat4::orthographic_rh(
                ls_min.x - pad_x,
                ls_max.x + pad_x,
                ls_min.y - pad_y,
                ls_max.y + pad_y,
                light_near,
                light_far,
            );

            light_vps[cascade] = ortho * light_view;
            prev_split_z = split_z;
        }

        if light_vps.iter().any(|vp| {
            let determinant = vp.determinant();
            !determinant.is_finite()
                || determinant == 0.0
                || !vp.to_cols_array().iter().all(|value| value.is_finite())
        }) {
            return Err(CascadeDataError::DegenerateFrustum);
        }

        Ok((splits4, light_vps))
    }
}

#[cfg(test)]
mod tests {
    use super::{CascadeDataError, VulkanDevice};
    use glam::{Mat4, Vec3};

    fn assert_approx(actual: f32, expected: f32) {
        let tolerance = expected.abs().max(1.0) * 1.0e-4;
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, got {actual} (tolerance {tolerance})"
        );
    }

    #[test]
    fn derives_clip_planes_from_rh_zo_perspective_projection() {
        let expected_near = 0.25;
        let expected_far = 750.0;
        let projection = Mat4::perspective_rh(
            60.0f32.to_radians(),
            16.0 / 9.0,
            expected_near,
            expected_far,
        );

        let (near, far) = VulkanDevice::derive_rh_zo_clip_planes(&projection)
            .expect("finite perspective clip planes should be recoverable");

        assert_approx(near, expected_near);
        assert_approx(far, expected_far);
    }

    #[test]
    fn derives_clip_planes_from_rh_zo_orthographic_projection() {
        let expected_near = 2.0;
        let expected_far = 42.0;
        let projection = Mat4::orthographic_rh(-8.0, 12.0, -5.0, 7.0, expected_near, expected_far);

        let (near, far) = VulkanDevice::derive_rh_zo_clip_planes(&projection)
            .expect("finite orthographic clip planes should be recoverable");

        assert_approx(near, expected_near);
        assert_approx(far, expected_far);
    }

    #[test]
    fn different_directional_lights_produce_different_cascade_matrices() {
        let view = Mat4::look_at_rh(Vec3::new(3.0, 4.0, 8.0), Vec3::ZERO, Vec3::Y);
        let projection = Mat4::perspective_rh(55.0f32.to_radians(), 1.5, 0.2, 80.0);
        let (near, far) = VulkanDevice::derive_rh_zo_clip_planes(&projection).unwrap();

        let (_, first) = VulkanDevice::compute_cascade_data(
            &view,
            &projection,
            near,
            far,
            Vec3::new(1.0, -2.0, 0.5),
        )
        .expect("first light direction should produce valid cascades");
        let (_, second) = VulkanDevice::compute_cascade_data(
            &view,
            &projection,
            near,
            far,
            Vec3::new(-0.25, -1.0, -1.5),
        )
        .expect("second light direction should produce valid cascades");

        let maximum_difference = first
            .iter()
            .zip(second.iter())
            .flat_map(|(left, right)| {
                left.to_cols_array()
                    .into_iter()
                    .zip(right.to_cols_array())
                    .map(|(left, right)| (left - right).abs())
            })
            .fold(0.0f32, f32::max);
        assert!(maximum_difference > 1.0e-3);
    }

    #[test]
    fn invalid_shadow_inputs_are_rejected_without_a_fixed_fallback() {
        assert_eq!(
            VulkanDevice::normalize_shadow_light_direction(Vec3::ZERO),
            Err(CascadeDataError::InvalidLightDirection)
        );
        assert_eq!(
            VulkanDevice::normalize_shadow_light_direction(Vec3::new(f32::NAN, 0.0, 1.0)),
            Err(CascadeDataError::InvalidLightDirection)
        );
        assert_eq!(
            VulkanDevice::derive_rh_zo_clip_planes(&Mat4::IDENTITY),
            Err(CascadeDataError::InvalidClipPlanes)
        );

        let projection = Mat4::perspective_rh(60.0f32.to_radians(), 1.0, 0.1, 100.0);
        assert_eq!(
            VulkanDevice::compute_cascade_data(
                &Mat4::IDENTITY,
                &projection,
                0.1,
                100.0,
                Vec3::ZERO,
            ),
            Err(CascadeDataError::InvalidLightDirection)
        );
    }
}
