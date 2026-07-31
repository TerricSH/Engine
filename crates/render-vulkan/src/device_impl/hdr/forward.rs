use super::*;

/// Owns every Vulkan handle created while assembling the HDR forward graph.
///
/// The device fields are only populated after the complete graph exists.
/// Until then this guard is the sole owner, so any `?`, explicit error return,
/// or panic unwinds all successfully-created local handles in reverse order.
struct HdrForwardBuildGuard {
    device: ash::Device,
    render_pass: Option<vk::RenderPass>,
    pipeline_layout: Option<vk::PipelineLayout>,
    shader_modules: Vec<vk::ShaderModule>,
    pipelines: Vec<vk::Pipeline>,
    framebuffer: Option<vk::Framebuffer>,
    armed: bool,
}

impl HdrForwardBuildGuard {
    fn new(device: &ash::Device) -> Self {
        Self {
            device: device.clone(),
            render_pass: None,
            pipeline_layout: None,
            shader_modules: Vec::new(),
            pipelines: Vec::new(),
            framebuffer: None,
            armed: true,
        }
    }

    fn track_shader_module(&mut self, module: vk::ShaderModule) {
        self.shader_modules.push(module);
    }

    fn destroy_shader_module(&mut self, module: vk::ShaderModule) {
        if let Some(index) = self
            .shader_modules
            .iter()
            .position(|candidate| *candidate == module)
        {
            self.shader_modules.swap_remove(index);
        }
        unsafe {
            self.device.destroy_shader_module(module, None);
        }
    }

    fn track_pipelines(&mut self, pipelines: &[vk::Pipeline]) {
        self.pipelines.extend_from_slice(pipelines);
    }

    fn commit(mut self) {
        self.armed = false;
    }
}

impl Drop for HdrForwardBuildGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        unsafe {
            if let Some(framebuffer) = self.framebuffer.take() {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            for pipeline in self.pipelines.drain(..).rev() {
                self.device.destroy_pipeline(pipeline, None);
            }
            for shader_module in self.shader_modules.drain(..).rev() {
                self.device.destroy_shader_module(shader_module, None);
            }
            if let Some(pipeline_layout) = self.pipeline_layout.take() {
                self.device.destroy_pipeline_layout(pipeline_layout, None);
            }
            if let Some(render_pass) = self.render_pass.take() {
                self.device.destroy_render_pass(render_pass, None);
            }
        }
    }
}

impl VulkanDevice {
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
        if self.oit_accum_image.is_none() {
            let (image, view, allocation) = self.create_oit_color_target("oit-accumulation")?;
            self.oit_accum_image = Some(image);
            self.oit_accum_view = Some(view);
            self.oit_accum_allocation = Some(allocation);
        }
        if self.oit_optical_depth_image.is_none() {
            let (image, view, allocation) = self.create_oit_color_target("oit-optical-depth")?;
            self.oit_optical_depth_image = Some(image);
            self.oit_optical_depth_view = Some(view);
            self.oit_optical_depth_allocation = Some(allocation);
        }
        if self.hdr_msaa_samples != vk::SampleCountFlags::TYPE_1 {
            if self.hdr_msaa_color_image.is_none() {
                let (image, view, allocation) = self.create_msaa_color_target("hdr-msaa-color")?;
                self.hdr_msaa_color_image = Some(image);
                self.hdr_msaa_color_view = Some(view);
                self.hdr_msaa_color_allocation = Some(allocation);
            }
            if self.oit_msaa_accum_image.is_none() {
                let (image, view, allocation) =
                    self.create_msaa_color_target("oit-msaa-accumulation")?;
                self.oit_msaa_accum_image = Some(image);
                self.oit_msaa_accum_view = Some(view);
                self.oit_msaa_accum_allocation = Some(allocation);
            }
            if self.oit_msaa_optical_depth_image.is_none() {
                let (image, view, allocation) =
                    self.create_msaa_color_target("oit-msaa-optical-depth")?;
                self.oit_msaa_optical_depth_image = Some(image);
                self.oit_msaa_optical_depth_view = Some(view);
                self.oit_msaa_optical_depth_allocation = Some(allocation);
            }
            if self.hdr_msaa_depth_image.is_none() {
                let (image, view, allocation) = self.create_hdr_msaa_depth_target()?;
                self.hdr_msaa_depth_image = Some(image);
                self.hdr_msaa_depth_view = Some(view);
                self.hdr_msaa_depth_allocation = Some(allocation);
            }
        }

        let d = &self.logical_device.device;
        let mut build = HdrForwardBuildGuard::new(d);
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("no swapchain".into()))?;
        let _ext = sc.extent;
        let resolved_hdr_view = self
            .hdr_color_view
            .ok_or(VulkanError::Loader("no HDR texture".into()))?;
        let resolved_oit_accum_view = self
            .oit_accum_view
            .ok_or(VulkanError::Loader("no OIT accumulation texture".into()))?;
        let resolved_oit_optical_depth_view = self
            .oit_optical_depth_view
            .ok_or(VulkanError::Loader("no OIT optical-depth texture".into()))?;
        let multisampled = self.hdr_msaa_samples != vk::SampleCountFlags::TYPE_1;
        let hdr_view = if multisampled {
            self.hdr_msaa_color_view
                .ok_or(VulkanError::Loader("no multisampled HDR texture".into()))?
        } else {
            resolved_hdr_view
        };
        let oit_accum_view = if multisampled {
            self.oit_msaa_accum_view.ok_or(VulkanError::Loader(
                "no multisampled OIT accumulation texture".into(),
            ))?
        } else {
            resolved_oit_accum_view
        };
        let oit_optical_depth_view = if multisampled {
            self.oit_msaa_optical_depth_view.ok_or(VulkanError::Loader(
                "no multisampled OIT optical-depth texture".into(),
            ))?
        } else {
            resolved_oit_optical_depth_view
        };
        let depth_view = if multisampled {
            self.hdr_msaa_depth_view.ok_or(VulkanError::Loader(
                "no multisampled HDR depth texture".into(),
            ))?
        } else {
            self.depth_image_view
                .ok_or(VulkanError::Loader("no depth texture".into()))?
        };
        let vert = self
            .forward_vert_spv
            .clone()
            .ok_or(VulkanError::MissingShader("hdr_forward.vert"))?;
        let frag = self
            .forward_frag_spv
            .clone()
            .ok_or(VulkanError::MissingShader("hdr_forward.frag"))?;
        let skybox_vert = self
            .skybox_vert_spv
            .clone()
            .ok_or(VulkanError::MissingShader("skybox.vert"))?;
        let skybox_frag = self
            .skybox_frag_spv
            .clone()
            .ok_or(VulkanError::MissingShader("skybox.frag"))?;
        let vfx_billboard_vert = self
            .vfx_billboard_vert_spv
            .clone()
            .ok_or(VulkanError::MissingShader("vfx_billboard.vert"))?;
        let gpu_vfx_billboard_vert = self
            .gpu_vfx_billboard_vert_spv
            .clone()
            .ok_or(VulkanError::MissingShader("gpu_vfx_billboard.vert"))?;
        let vfx_billboard_frag = self
            .vfx_billboard_frag_spv
            .clone()
            .ok_or(VulkanError::MissingShader("vfx_billboard.frag"))?;
        let instanced_vert = self
            .instanced_vert_spv
            .clone()
            .ok_or(VulkanError::MissingShader("instanced.vert"))?;

        // ---- Render pass: color(RGBA16F) + depth(D32) ----
        let color_at = vk::AttachmentDescription::default()
            .format(vk::Format::R16G16B16A16_SFLOAT)
            .samples(self.hdr_msaa_samples)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(if multisampled {
                vk::AttachmentStoreOp::DONT_CARE
            } else {
                vk::AttachmentStoreOp::STORE
            })
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let depth_at = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(self.hdr_msaa_samples)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let color_ref = [
            vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            vk::AttachmentReference::default()
                .attachment(1)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            vk::AttachmentReference::default()
                .attachment(2)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
        ];
        let depth_ref = vk::AttachmentReference::default()
            .attachment(3)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let resolve_refs = [
            vk::AttachmentReference::default()
                .attachment(4)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            vk::AttachmentReference::default()
                .attachment(5)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            vk::AttachmentReference::default()
                .attachment(6)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
        ];
        let mut subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_ref)
            .depth_stencil_attachment(&depth_ref);
        if multisampled {
            subpass = subpass.resolve_attachments(&resolve_refs);
        }
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
        let resolve_at = vk::AttachmentDescription::default()
            .format(vk::Format::R16G16B16A16_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let mut atts = vec![color_at, color_at, color_at, depth_at];
        if multisampled {
            atts.extend([resolve_at, resolve_at, resolve_at]);
        }
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
        build.render_pass = Some(rp);

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
        let push_constant_ranges = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX,
            offset: 0,
            size: 128,
        }];
        let pli = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&push_constant_ranges);
        // SAFETY: `d` is a valid AshDevice; `pli` describes a valid layout.
        let pll = unsafe { d.create_pipeline_layout(&pli, None) }
            .map_err(|r| VulkanError::vk("cpl_hdr_forward", r))?;
        build.pipeline_layout = Some(pll);

        // ---- Shader modules ----
        // SAFETY: `d` is a valid AshDevice; `vert`/`frag` are valid SPIR-V.
        let vm = unsafe { mk_sm(d, &vert)? };
        build.track_shader_module(vm);
        let fm = unsafe { mk_sm(d, &frag)? };
        build.track_shader_module(fm);

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
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(self.hdr_msaa_samples);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds2 = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);
        let mut material_pipelines = Vec::with_capacity(8);
        for (cull_mode, blend_mode, depth_write) in [
            (vk::CullModeFlags::BACK, "Opaque", true),
            (vk::CullModeFlags::NONE, "Opaque", true),
            (vk::CullModeFlags::BACK, "Alpha", false),
            (vk::CullModeFlags::NONE, "Alpha", false),
            (vk::CullModeFlags::BACK, "Additive", false),
            (vk::CullModeFlags::NONE, "Additive", false),
            (vk::CullModeFlags::BACK, "WeightedOit", false),
            (vk::CullModeFlags::NONE, "WeightedOit", false),
        ] {
            let raster = vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(cull_mode)
                .front_face(vk::FrontFace::CLOCKWISE)
                .line_width(1.0);
            let blend_attachments = super::mrt_blend_attachments(blend_mode);
            let blend = vk::PipelineColorBlendStateCreateInfo::default()
                .logic_op_enable(false)
                .attachments(&blend_attachments);
            let depth = vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(true)
                .depth_write_enable(depth_write)
                .depth_compare_op(vk::CompareOp::LESS);
            let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
                .stages(&sr)
                .vertex_input_state(&vi)
                .input_assembly_state(&ia)
                .viewport_state(&vs)
                .rasterization_state(&raster)
                .multisample_state(&ms)
                .depth_stencil_state(&depth)
                .color_blend_state(&blend)
                .dynamic_state(&ds2)
                .layout(pll)
                .render_pass(rp)
                .subpass(0);
            let pipeline_result = unsafe {
                d.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            };
            let pipeline = match pipeline_result {
                Ok(pipelines) => pipelines[0],
                Err((partial_pipelines, result)) => {
                    build.track_pipelines(&partial_pipelines);
                    return Err(VulkanError::vk("cgp_hdr_forward_variant", result));
                }
            };
            build.track_pipelines(&[pipeline]);
            material_pipelines.push(pipeline);
        }
        let pipeline = material_pipelines[0];
        let double_sided_pipeline = material_pipelines[1];
        let blend_pipeline = material_pipelines[2];
        let blend_double_sided_pipeline = material_pipelines[3];
        let additive_pipeline = material_pipelines[4];
        let additive_double_sided_pipeline = material_pipelines[5];
        let oit_pipeline = material_pipelines[6];
        let oit_double_sided_pipeline = material_pipelines[7];

        // SAFETY: shader modules are no longer needed after pipeline creation.
        build.destroy_shader_module(vm);
        build.destroy_shader_module(fm);

        // ---- Skybox pipeline ----
        // It shares the forward pipeline layout and render pass. The vertex
        // shader generates a cube from gl_VertexIndex, so no vertex input is
        // required. Rendering happens before opaque geometry without depth
        // writes, allowing scene geometry to replace the background normally.
        let sky_vm = unsafe { mk_sm(d, &skybox_vert)? };
        build.track_shader_module(sky_vm);
        let sky_fm = unsafe { mk_sm(d, &skybox_frag)? };
        build.track_shader_module(sky_fm);
        let sky_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(sky_vm)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(sky_fm)
                .name(main),
        ];
        let sky_vi = vk::PipelineVertexInputStateCreateInfo::default();
        let sky_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false);
        let sky_blend_attachments = super::mrt_blend_attachments("Opaque");
        let sky_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&sky_blend_attachments);
        let sky_raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .line_width(1.0);
        let sky_pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sky_stages)
            .vertex_input_state(&sky_vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs)
            .rasterization_state(&sky_raster)
            .multisample_state(&ms)
            .depth_stencil_state(&sky_depth)
            .color_blend_state(&sky_blend)
            .dynamic_state(&ds2)
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        let sky_pipeline_result = unsafe {
            d.create_graphics_pipelines(vk::PipelineCache::null(), &[sky_pipeline_info], None)
        };
        let skybox_pipeline = match sky_pipeline_result {
            Ok(pipelines) => {
                let pipeline = pipelines[0];
                build.track_pipelines(&[pipeline]);
                pipeline
            }
            Err((partial_pipelines, result)) => {
                build.track_pipelines(&partial_pipelines);
                return Err(VulkanError::vk("cgp_hdr_skybox", result));
            }
        };
        build.destroy_shader_module(sky_vm);
        build.destroy_shader_module(sky_fm);

        // ---- Instanced particle billboard pipeline ----
        // Binding zero retains the authored quad mesh; binding one advances
        // once per particle and carries position/size plus rotation/age.
        let vfx_vm = unsafe { mk_sm(d, &vfx_billboard_vert)? };
        build.track_shader_module(vfx_vm);
        let vfx_fm = unsafe { mk_sm(d, &vfx_billboard_frag)? };
        build.track_shader_module(vfx_fm);
        let gpu_vfx_vm = unsafe { mk_sm(d, &gpu_vfx_billboard_vert)? };
        build.track_shader_module(gpu_vfx_vm);
        let vfx_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vfx_vm)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(vfx_fm)
                .name(main),
        ];
        let vfx_bindings = [
            vk::VertexInputBindingDescription::default()
                .binding(0)
                .stride(32)
                .input_rate(vk::VertexInputRate::VERTEX),
            vk::VertexInputBindingDescription::default()
                .binding(1)
                .stride(32)
                .input_rate(vk::VertexInputRate::INSTANCE),
        ];
        let vfx_attributes = [
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
            vk::VertexInputAttributeDescription {
                location: 3,
                binding: 1,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 0,
            },
            vk::VertexInputAttributeDescription {
                location: 4,
                binding: 1,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 16,
            },
        ];
        let vfx_vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vfx_bindings)
            .vertex_attribute_descriptions(&vfx_attributes);
        let vfx_raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .line_width(1.0);
        let vfx_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::LESS);
        let vfx_alpha_attachments = super::mrt_blend_attachments("Alpha");
        let vfx_alpha_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&vfx_alpha_attachments);
        let vfx_additive_attachments = super::mrt_blend_attachments("Additive");
        let vfx_additive_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&vfx_additive_attachments);
        let vfx_oit_attachments = super::mrt_blend_attachments("WeightedOit");
        let vfx_oit_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&vfx_oit_attachments);
        let vfx_pipeline_base = vk::GraphicsPipelineCreateInfo::default()
            .stages(&vfx_stages)
            .vertex_input_state(&vfx_vertex_input)
            .input_assembly_state(&ia)
            .viewport_state(&vs)
            .rasterization_state(&vfx_raster)
            .multisample_state(&ms)
            .depth_stencil_state(&vfx_depth)
            .dynamic_state(&ds2)
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        let vfx_pipeline_infos = [
            vfx_pipeline_base.color_blend_state(&vfx_alpha_blend),
            vfx_pipeline_base.color_blend_state(&vfx_additive_blend),
            vfx_pipeline_base.color_blend_state(&vfx_oit_blend),
        ];
        let vfx_pipeline_result = unsafe {
            d.create_graphics_pipelines(vk::PipelineCache::null(), &vfx_pipeline_infos, None)
        };
        let vfx_billboard_pipelines = match vfx_pipeline_result {
            Ok(pipelines) => {
                build.track_pipelines(&pipelines);
                pipelines
            }
            Err((partial_pipelines, result)) => {
                build.track_pipelines(&partial_pipelines);
                return Err(VulkanError::vk("cgp_hdr_vfx_billboard", result));
            }
        };
        let vfx_billboard_pipeline = vfx_billboard_pipelines[0];
        let vfx_billboard_additive_pipeline = vfx_billboard_pipelines[1];
        let vfx_billboard_oit_pipeline = vfx_billboard_pipelines[2];

        let gpu_vfx_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(gpu_vfx_vm)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(vfx_fm)
                .name(main),
        ];
        let gpu_vfx_bindings = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(32)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let gpu_vfx_attributes = [
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
        let gpu_vfx_vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&gpu_vfx_bindings)
            .vertex_attribute_descriptions(&gpu_vfx_attributes);
        let gpu_vfx_pipeline_base = vk::GraphicsPipelineCreateInfo::default()
            .stages(&gpu_vfx_stages)
            .vertex_input_state(&gpu_vfx_vertex_input)
            .input_assembly_state(&ia)
            .viewport_state(&vs)
            .rasterization_state(&vfx_raster)
            .multisample_state(&ms)
            .depth_stencil_state(&vfx_depth)
            .dynamic_state(&ds2)
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        let gpu_vfx_pipeline_infos = [
            gpu_vfx_pipeline_base.color_blend_state(&vfx_alpha_blend),
            gpu_vfx_pipeline_base.color_blend_state(&vfx_additive_blend),
            gpu_vfx_pipeline_base.color_blend_state(&vfx_oit_blend),
        ];
        let gpu_vfx_pipeline_result = unsafe {
            d.create_graphics_pipelines(vk::PipelineCache::null(), &gpu_vfx_pipeline_infos, None)
        };
        let gpu_vfx_billboard_pipelines = match gpu_vfx_pipeline_result {
            Ok(pipelines) => {
                build.track_pipelines(&pipelines);
                pipelines
            }
            Err((partial_pipelines, result)) => {
                build.track_pipelines(&partial_pipelines);
                return Err(VulkanError::vk("cgp_hdr_gpu_vfx_billboard", result));
            }
        };
        build.destroy_shader_module(vfx_vm);
        build.destroy_shader_module(gpu_vfx_vm);
        build.destroy_shader_module(vfx_fm);
        let gpu_vfx_billboard_pipeline = gpu_vfx_billboard_pipelines[0];
        let gpu_vfx_billboard_additive_pipeline = gpu_vfx_billboard_pipelines[1];
        let gpu_vfx_billboard_oit_pipeline = gpu_vfx_billboard_pipelines[2];

        // ---- Instanced opaque/masked surface pipelines ----
        let instanced_vm = unsafe { mk_sm(d, &instanced_vert)? };
        build.track_shader_module(instanced_vm);
        let instanced_fm = unsafe { mk_sm(d, &frag)? };
        build.track_shader_module(instanced_fm);
        let instanced_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(instanced_vm)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(instanced_fm)
                .name(main),
        ];
        let instanced_bindings = [
            vk::VertexInputBindingDescription::default()
                .binding(0)
                .stride(32)
                .input_rate(vk::VertexInputRate::VERTEX),
            vk::VertexInputBindingDescription::default()
                .binding(1)
                .stride(64)
                .input_rate(vk::VertexInputRate::INSTANCE),
        ];
        let instanced_attributes = [
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
            vk::VertexInputAttributeDescription {
                location: 3,
                binding: 1,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 0,
            },
            vk::VertexInputAttributeDescription {
                location: 4,
                binding: 1,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 16,
            },
            vk::VertexInputAttributeDescription {
                location: 5,
                binding: 1,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 32,
            },
            vk::VertexInputAttributeDescription {
                location: 6,
                binding: 1,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 48,
            },
        ];
        let instanced_vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&instanced_bindings)
            .vertex_attribute_descriptions(&instanced_attributes);
        let instanced_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS);
        let instanced_blend_attachments = super::mrt_blend_attachments("Opaque");
        let instanced_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&instanced_blend_attachments);
        let mut instanced_pipelines = Vec::with_capacity(2);
        for cull_mode in [vk::CullModeFlags::BACK, vk::CullModeFlags::NONE] {
            let instanced_raster = vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(cull_mode)
                .front_face(vk::FrontFace::CLOCKWISE)
                .line_width(1.0);
            let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
                .stages(&instanced_stages)
                .vertex_input_state(&instanced_vertex_input)
                .input_assembly_state(&ia)
                .viewport_state(&vs)
                .rasterization_state(&instanced_raster)
                .multisample_state(&ms)
                .depth_stencil_state(&instanced_depth)
                .color_blend_state(&instanced_blend)
                .dynamic_state(&ds2)
                .layout(pll)
                .render_pass(rp)
                .subpass(0);
            let result = unsafe {
                d.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            };
            match result {
                Ok(pipelines) => {
                    build.track_pipelines(&[pipelines[0]]);
                    instanced_pipelines.push(pipelines[0]);
                }
                Err((partial_pipelines, result)) => {
                    build.track_pipelines(&partial_pipelines);
                    return Err(VulkanError::vk("cgp_hdr_instanced", result));
                }
            }
        }
        build.destroy_shader_module(instanced_vm);
        build.destroy_shader_module(instanced_fm);
        let instanced_pipeline = instanced_pipelines[0];
        let instanced_double_sided_pipeline = instanced_pipelines[1];

        // ---- Framebuffer (HDR color view + depth view) ----
        let mut att_views = vec![hdr_view, oit_accum_view, oit_optical_depth_view, depth_view];
        if multisampled {
            att_views.extend([
                resolved_hdr_view,
                resolved_oit_accum_view,
                resolved_oit_optical_depth_view,
            ]);
        }
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
        build.framebuffer = Some(fb);

        self.hdr_forward_rp = Some(rp);
        self.hdr_forward_pipeline_layout = Some(pll);
        self.hdr_forward_pipeline = Some(pipeline);
        self.hdr_forward_double_sided_pipeline = Some(double_sided_pipeline);
        self.hdr_forward_blend_pipeline = Some(blend_pipeline);
        self.hdr_forward_blend_double_sided_pipeline = Some(blend_double_sided_pipeline);
        self.hdr_forward_oit_pipeline = Some(oit_pipeline);
        self.hdr_forward_oit_double_sided_pipeline = Some(oit_double_sided_pipeline);
        self.hdr_forward_additive_pipeline = Some(additive_pipeline);
        self.hdr_forward_additive_double_sided_pipeline = Some(additive_double_sided_pipeline);
        self.hdr_skybox_pipeline = Some(skybox_pipeline);
        self.hdr_vfx_billboard_pipeline = Some(vfx_billboard_pipeline);
        self.hdr_vfx_billboard_additive_pipeline = Some(vfx_billboard_additive_pipeline);
        self.hdr_vfx_billboard_oit_pipeline = Some(vfx_billboard_oit_pipeline);
        self.hdr_gpu_vfx_billboard_pipeline = Some(gpu_vfx_billboard_pipeline);
        self.hdr_gpu_vfx_billboard_additive_pipeline = Some(gpu_vfx_billboard_additive_pipeline);
        self.hdr_gpu_vfx_billboard_oit_pipeline = Some(gpu_vfx_billboard_oit_pipeline);
        self.hdr_instanced_pipeline = Some(instanced_pipeline);
        self.hdr_instanced_double_sided_pipeline = Some(instanced_double_sided_pipeline);
        self.hdr_forward_fb = Some(fb);
        build.commit();

        Ok(())
    }
}
