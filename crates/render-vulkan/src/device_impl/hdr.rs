//! HDR offscreen rendering + tone-mapping resources for VulkanDevice.
//!
//! Phase 2.1: Creates an RGBA16F color texture, a forward HDR render pass /
//! pipeline, a tone-mapping render pass / pipeline, and per-swapchain tone
//! framebuffers.

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{mk_sm, VulkanDevice};

impl VulkanDevice {
    // ======================================================================
    // HDR color texture (RGBA16F, matches swapchain extent)
    // ======================================================================

    /// Create (or recreate) the HDR color attachment image + view.
    ///
    /// Idempotent: if the image already exists, does nothing.
    pub(crate) fn create_hdr_color_texture(&mut self) -> VkResult<()> {
        if self.hdr_color_image.is_some() {
            return Ok(());
        }
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();
        let extent = self.swapchain_extent;
        if extent.width == 0 || extent.height == 0 {
            return Ok(());
        }

        // ---- 1. Image ----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R16G16B16A16_SFLOAT)
            .extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_DST,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is a valid AshDevice; `image_info` describes a valid 2D
        // color image; `None` means no custom allocator.
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|r| VulkanError::vk("create_hdr_image", r))?;
        // SAFETY: `image` was just created by this device.
        let req = unsafe { d.get_image_memory_requirements(image) };
        let allocation = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "hdr-color",
                requirements: req,
                location: crate::allocator::MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: `image` was created by this device; `allocation` was created
        // for this image's memory requirements.
        unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_hdr_image", r))?;

        // ---- 2. Image view ----
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R16G16B16A16_SFLOAT)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: `d` is a valid AshDevice; `view_info` references a valid
        // image; `None` means no custom allocator.
        let image_view = unsafe { d.create_image_view(&view_info, None) }
            .map_err(|r| VulkanError::vk("create_hdr_image_view", r))?;

        // ---- 3. Sampler (linear, clamp-to-edge, no compare) ----
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .min_lod(0.0)
            .max_lod(1.0)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false);
        // SAFETY: `d` is a valid AshDevice; `sampler_info` describes a valid
        // sampler; `None` means no custom allocator.
        let sampler = unsafe { d.create_sampler(&sampler_info, None) }
            .map_err(|r| VulkanError::vk("create_hdr_sampler", r))?;

        self.hdr_color_image = Some(image);
        self.hdr_color_view = Some(image_view);
        self.hdr_color_allocation = Some(allocation);
        self.hdr_color_sampler = Some(sampler);

        Ok(())
    }

    // ======================================================================
    // Forward HDR render pass + pipeline (RGBA16F color + D32 depth)
    // ======================================================================

    /// Create (or recreate) the forward HDR render pass, pipeline, and
    /// framebuffer.
    pub(crate) fn create_hdr_forward_resources(&mut self) -> VkResult<()> {
        // Skip if already created and the image exists.
        if self.hdr_forward_rp.is_some() {
            return Ok(());
        }
        // Ensure the HDR color texture exists.
        self.create_hdr_color_texture()?;

        let d = &self.logical_device.device;
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("no swapchain".into()))?;
        let _ext = sc.extent;
        let depth_view = self
            .depth_image_view
            .ok_or(VulkanError::Loader("no depth texture".into()))?;
        let hdr_view = self
            .hdr_color_view
            .ok_or(VulkanError::Loader("no HDR texture".into()))?;
        let vert = self
            .mvp_vert_spv
            .clone()
            .ok_or(VulkanError::MissingShader("hdr_forward.vert"))?;
        let frag = self
            .mvp_frag_spv
            .clone()
            .ok_or(VulkanError::MissingShader("hdr_forward.frag"))?;

        // ---- Render pass: color(RGBA16F) + depth(D32) ----
        let color_at = vk::AttachmentDescription::default()
            .format(vk::Format::R16G16B16A16_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
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
            .map_err(|r| VulkanError::vk("crp_hdr_forward", r))?;

        // ---- Pipeline layout (set=0 per-frame UBO, set=1 shadow/env, set=2 material) ----
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
        // SAFETY: `d` is a valid AshDevice; `pli` describes a valid layout.
        let pll = unsafe { d.create_pipeline_layout(&pli, None) }
            .map_err(|r| VulkanError::vk("cpl_hdr_forward", r))?;

        // ---- Shader modules ----
        // SAFETY: `d` is a valid AshDevice; `vert`/`frag` are valid SPIR-V.
        let vm = unsafe { mk_sm(d, &vert)? };
        let fm = unsafe { mk_sm(d, &frag)? };

        // ---- Graphics pipeline ----
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
        let vs = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .line_width(1.0);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend_attachment = super::blend_attachment_from_mode("Alpha");
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
            .viewport_state(&vs)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .depth_stencil_state(&ds)
            .color_blend_state(&cb)
            .dynamic_state(&ds2)
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid graphics
        // pipeline; `vk::PipelineCache::null()` is allowed.
        let pipeline =
            unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
                .map_err(|(_, r)| VulkanError::vk("cgp_hdr_forward", r))?[0];

        // SAFETY: shader modules are no longer needed after pipeline creation.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }

        // ---- Framebuffer (HDR color view + depth view) ----
        let att_views = [hdr_view, depth_view];
        // SAFETY: `d` is a valid AshDevice; framebuffer info references valid
        // image views and render pass; `None` means no custom allocator.
        let fb = unsafe {
            d.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(rp)
                    .attachments(&att_views)
                    .width(_ext.width)
                    .height(_ext.height)
                    .layers(1),
                None,
            )
        }
        .map_err(|r| VulkanError::vk("cfb_hdr_forward", r))?;

        self.hdr_forward_rp = Some(rp);
        self.hdr_forward_pipeline_layout = Some(pll);
        self.hdr_forward_pipeline = Some(pipeline);
        self.hdr_forward_fb = Some(fb);

        Ok(())
    }

    // ======================================================================
    // Tone-mapping render pass + pipeline
    // ======================================================================

    /// Create the tone-mapping render pass, pipeline, and descriptor set for
    /// reading the HDR image.
    pub(crate) fn create_tone_mapping_resources(&mut self) -> VkResult<()> {
        if self.tone_rp.is_some() {
            return Ok(());
        }
        let d = &self.logical_device.device;

        // ---- Tone-mapping render pass (color = BGRA8 only, no depth) ----
        let at = vk::AttachmentDescription::default()
            .format(vk::Format::B8G8R8A8_UNORM)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::LOAD)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
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
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
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

        // ---- Descriptor set layout (set=0, binding=0 = combined image sampler) ----
        let ds_bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
        let ds_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&ds_bindings);
        // SAFETY: `d` is a valid AshDevice.
        let ds_layout = unsafe { d.create_descriptor_set_layout(&ds_layout_info, None) }
            .map_err(|r| VulkanError::vk("create_tone_ds_layout", r))?;

        // ---- Descriptor pool + set ----
        let pool_sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: 1,
        }];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        // SAFETY: `d` is a valid AshDevice.
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_tone_ds_pool", r))?;

        let ds_layouts = [ds_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&ds_layouts);
        // SAFETY: `d` is a valid AshDevice; the pool has enough capacity.
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_tone_ds", r))?;
        let desc_set = desc_sets[0];

        // ---- Pipeline layout: set=0 (HDR sampler) + push constants 128 B ----
        let pc_range = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX,
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

        // ---- Tonemap pipeline ----
        let vert_spv = crate::shaders_embedded::TONEMAP_VERT_SPV;
        let frag_spv = crate::shaders_embedded::TONEMAP_FRAG_SPV;
        if vert_spv.is_empty() || frag_spv.is_empty() {
            return Err(VulkanError::MissingShader("tonemap"));
        }
        // SAFETY: `d` is a valid AshDevice; SPIR-V bytecode is valid.
        let vm = unsafe { mk_sm(d, vert_spv)? };
        let fm = unsafe { mk_sm(d, frag_spv)? };

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
        let pipeline =
            unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
                .map_err(|(_, r)| VulkanError::vk("cgp_tone", r))?[0];

        // SAFETY: shader modules no longer needed after pipeline creation.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }

        self.tone_rp = Some(rp);
        self.tone_pipeline_layout = Some(pll);
        self.tone_pipeline = Some(pipeline);
        self.tone_desc_layout = Some(ds_layout);
        self.tone_desc_pool = Some(pool);
        self.tone_desc_set = Some(desc_set);

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
            let fb = unsafe {
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
            .map_err(|r| VulkanError::vk("cfb_tone", r))?;
            fbs.push(fb);
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
        let d = &self.logical_device.device;

        let image_info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(image_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(ds)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&image_info)];
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
        if self.hdr_color_image.is_some() && self.tone_rp.is_some() {
            return Ok(());
        }
        self.create_hdr_color_texture()?;
        self.create_hdr_forward_resources()?;
        self.create_tone_mapping_resources()?;
        self.create_tone_framebuffers()?;
        self.update_tone_descriptor_set();
        Ok(())
    }

    // ======================================================================
    // Destruction
    // ======================================================================

    /// Destroy tone-mapping framebuffers only (called on resize).
    fn destroy_tone_framebuffers(&mut self) {
        let d = &self.logical_device.device;
        for fb in self.tone_framebuffers.drain(..) {
            // SAFETY: `fb` was created by this device and is still alive.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
    }

    /// Destroy all HDR + tone-mapping resources (reverse order of creation).
    pub(crate) fn destroy_hdr_resources(&mut self) {
        // Destroy tone-mapping framebuffers first (no device borrow conflict).
        for fb in self.tone_framebuffers.drain(..) {
            let d = &self.logical_device.device;
            // SAFETY: `fb` was created by this device and is still alive.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }

        let d = &self.logical_device.device;

        // Forward HDR framebuffer
        if let Some(fb) = self.hdr_forward_fb.take() {
            // SAFETY: `fb` was created by this device.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }

        // Tone pipeline + layout
        if let Some(p) = self.tone_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.tone_pipeline_layout.take() {
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }

        // Forward HDR pipeline + layout
        if let Some(p) = self.hdr_forward_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.hdr_forward_pipeline_layout.take() {
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }

        // Render passes
        if let Some(rp) = self.tone_rp.take() {
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        if let Some(rp) = self.hdr_forward_rp.take() {
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }

        // Tone descriptor set infrastructure
        if let Some(pool) = self.tone_desc_pool.take() {
            // Pool frees its descriptor sets automatically.
            unsafe {
                d.destroy_descriptor_pool(pool, None);
            }
        }
        if let Some(layout) = self.tone_desc_layout.take() {
            unsafe {
                d.destroy_descriptor_set_layout(layout, None);
            }
        }

        // HDR sampler
        if let Some(s) = self.hdr_color_sampler.take() {
            unsafe {
                d.destroy_sampler(s, None);
            }
        }

        // HDR color image view + image + allocation
        if let Some(iv) = self.hdr_color_view.take() {
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(img) = self.hdr_color_image.take() {
            unsafe {
                d.destroy_image(img, None);
            }
        }
        if let Some(mut a) = self.hdr_color_allocation.take() {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                guard.free(&mut a);
            }
        }
    }
}
