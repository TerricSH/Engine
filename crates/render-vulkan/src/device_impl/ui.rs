//! Vulkan resources for the editor UI overlay.
//!
//! The overlay is a second render pass over the swapchain image.  Its color
//! attachment uses `LOAD` and starts/ends in `PRESENT_SRC_KHR`, preserving the
//! tone-mapped scene while allowing alpha-blended UI draws immediately before
//! presentation.

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{mk_sm, texture::FALLBACK_MATERIAL_TEXTURE_ID, VulkanDevice};

pub(crate) const MAX_UI_TEXTURE_DESCRIPTORS: usize = 1024;

/// Resolve the descriptor-cache key used by a UI batch.
///
/// Texture-less widgets deliberately sample the engine-owned opaque white
/// texture, so the fragment shader can always execute the same
/// `sampled_texture * vertex_color` path.
pub(crate) fn ui_texture_key(texture_id: Option<&str>) -> &str {
    texture_id.unwrap_or(FALLBACK_MATERIAL_TEXTURE_ID)
}

impl VulkanDevice {
    /// Ensure the load-op UI render pass, pipeline, descriptor pool and
    /// per-swapchain-image framebuffers exist.
    pub(crate) fn ensure_ui_overlay_resources(&mut self) -> VkResult<()> {
        let expected_framebuffers = self
            .swapchain
            .as_ref()
            .ok_or_else(|| VulkanError::Loader("UI overlay requires a swapchain".into()))?
            .image_views
            .len();
        let complete = self.ui_overlay_rp.is_some()
            && self.ui_overlay_pipeline_layout.is_some()
            && self.ui_overlay_pipeline.is_some()
            && self.ui_overlay_desc_layout.is_some()
            && self.ui_overlay_desc_pool.is_some()
            && self.ui_overlay_framebuffers.len() == expected_framebuffers;
        if complete {
            return Ok(());
        }

        self.destroy_ui_overlay_resources();
        let result = self.create_ui_overlay_resources_inner();
        if result.is_err() {
            self.destroy_ui_overlay_resources();
        }
        result
    }

    fn create_ui_overlay_resources_inner(&mut self) -> VkResult<()> {
        self.create_fallback_material_texture()?;

        let (swapchain_format, extent, image_views) = {
            let swapchain = self
                .swapchain
                .as_ref()
                .ok_or_else(|| VulkanError::Loader("UI overlay requires a swapchain".into()))?;
            (
                swapchain.format,
                swapchain.extent,
                swapchain.image_views.clone(),
            )
        };
        let d = &self.logical_device.device;

        // LOAD preserves the completed tone-map pass.  The explicit PRESENT
        // layouts let this pass transition the image to COLOR_ATTACHMENT and
        // back without a separate out-of-render-pass image barrier.
        let attachments = [vk::AttachmentDescription::default()
            .format(swapchain_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::LOAD)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
        let color_refs = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_refs)];
        let dependencies = [
            vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_READ
                        | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                )
                .dependency_flags(vk::DependencyFlags::BY_REGION),
            vk::SubpassDependency::default()
                .src_subpass(0)
                .dst_subpass(vk::SUBPASS_EXTERNAL)
                .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags::BOTTOM_OF_PIPE)
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dependency_flags(vk::DependencyFlags::BY_REGION),
        ];
        // SAFETY: `d` is live; attachment/subpass/dependency arrays remain
        // borrowed through creation and form a valid single-color render pass.
        let render_pass = unsafe {
            d.create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(&attachments)
                    .subpasses(&subpasses)
                    .dependencies(&dependencies),
                None,
            )
        }
        .map_err(|result| VulkanError::vk("create_ui_overlay_render_pass", result))?;
        self.ui_overlay_rp = Some(render_pass);

        let descriptor_bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
        // SAFETY: the live device consumes the call-scoped valid binding array.
        let descriptor_layout = unsafe {
            d.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&descriptor_bindings),
                None,
            )
        }
        .map_err(|result| VulkanError::vk("create_ui_overlay_descriptor_layout", result))?;
        self.ui_overlay_desc_layout = Some(descriptor_layout);

        let pool_sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: MAX_UI_TEXTURE_DESCRIPTORS as u32,
        }];
        // SAFETY: pool counts are bounded/non-zero and the referenced local
        // pool-size array remains alive through the device call.
        let descriptor_pool = unsafe {
            d.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                    .max_sets(MAX_UI_TEXTURE_DESCRIPTORS as u32)
                    .pool_sizes(&pool_sizes),
                None,
            )
        }
        .map_err(|result| VulkanError::vk("create_ui_overlay_descriptor_pool", result))?;
        self.ui_overlay_desc_pool = Some(descriptor_pool);

        let set_layouts = [descriptor_layout];
        let push_ranges = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX,
            offset: 0,
            size: 8,
        }];
        // SAFETY: descriptor layout is live; referenced layout/range arrays are
        // valid and remain alive through pipeline-layout creation.
        let pipeline_layout = unsafe {
            d.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&push_ranges),
                None,
            )
        }
        .map_err(|result| VulkanError::vk("create_ui_overlay_pipeline_layout", result))?;
        self.ui_overlay_pipeline_layout = Some(pipeline_layout);

        let vertex_spv = crate::shaders_embedded::UI_OVERLAY_VERT_SPV;
        let fragment_spv = crate::shaders_embedded::UI_OVERLAY_FRAG_SPV;
        if vertex_spv.is_empty() || fragment_spv.is_empty() {
            return Err(VulkanError::MissingShader("ui_overlay"));
        }
        // SAFETY: `d` is live and `mk_sm` validates the static vertex SPIR-V.
        let vertex_module = unsafe { mk_sm(d, vertex_spv)? };
        // SAFETY: `d` is live and `mk_sm` validates the static fragment SPIR-V.
        let fragment_module = match unsafe { mk_sm(d, fragment_spv) } {
            Ok(module) => module,
            Err(error) => {
                // SAFETY: fragment creation failed before publication; the
                // vertex module is exclusively owned and unused.
                unsafe { d.destroy_shader_module(vertex_module, None) };
                return Err(error);
            }
        };

        let entry = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex_module)
                .name(entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment_module)
                .name(entry),
        ];
        let bindings = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: 32,
            input_rate: vk::VertexInputRate::VERTEX,
        }];
        let attributes = [
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
            vk::VertexInputAttributeDescription {
                location: 2,
                binding: 0,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 16,
            },
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let color_blend_attachments = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let color_blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&color_blend_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .color_blend_state(&color_blend)
            .dynamic_state(&dynamic)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);
        // SAFETY: pipeline state references live compatible handles and all
        // local state arrays remain alive through the batch creation call.
        let pipeline_result =
            unsafe { d.create_graphics_pipelines(self.pipeline_cache, &[pipeline_info], None) };
        // SAFETY: pipeline creation has returned; compiled pipelines do not
        // retain shader modules, both of which are exclusively owned here.
        unsafe {
            d.destroy_shader_module(fragment_module, None);
            d.destroy_shader_module(vertex_module, None);
        }
        let pipeline = pipeline_result
            .map_err(|(_, result)| VulkanError::vk("create_ui_overlay_pipeline", result))?[0];
        self.ui_overlay_pipeline = Some(pipeline);

        for image_view in image_views {
            let attachments = [image_view];
            // SAFETY: pass/view are live and compatible; dimensions match the
            // swapchain and the attachment array lives through creation.
            let framebuffer = unsafe {
                d.create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(render_pass)
                        .attachments(&attachments)
                        .width(extent.width)
                        .height(extent.height)
                        .layers(1),
                    None,
                )
            }
            .map_err(|result| VulkanError::vk("create_ui_overlay_framebuffer", result))?;
            self.ui_overlay_framebuffers.push(framebuffer);
        }

        Ok(())
    }

    /// Return a descriptor set sampling `texture_id`, allocating it lazily.
    /// Missing named textures are errors; only `None` resolves to white.
    pub(crate) fn ui_overlay_descriptor_set(
        &mut self,
        texture_id: Option<&str>,
    ) -> VkResult<vk::DescriptorSet> {
        let key = ui_texture_key(texture_id);
        if let Some(descriptor_set) = self.ui_overlay_desc_sets.get(key).copied() {
            return Ok(descriptor_set);
        }
        if self.ui_overlay_desc_sets.len() >= MAX_UI_TEXTURE_DESCRIPTORS {
            return Err(VulkanError::Loader(format!(
                "UI texture descriptor capacity ({MAX_UI_TEXTURE_DESCRIPTORS}) exceeded"
            )));
        }
        let (view, sampler) = self
            .textures
            .get(key)
            .map(|texture| (texture.view, texture.sampler))
            .ok_or_else(|| VulkanError::Loader(format!("UI texture '{key}' is not uploaded")))?;
        let descriptor_layout = self.ui_overlay_desc_layout.ok_or_else(|| {
            VulkanError::Loader("UI overlay descriptor layout is not initialized".into())
        })?;
        let descriptor_pool = self.ui_overlay_desc_pool.ok_or_else(|| {
            VulkanError::Loader("UI overlay descriptor pool is not initialized".into())
        })?;
        let layouts = [descriptor_layout];
        // SAFETY: pool/layout are live device handles with remaining capacity;
        // the layout slice remains alive through this allocation.
        let descriptor_set = unsafe {
            self.logical_device.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(descriptor_pool)
                    .set_layouts(&layouts),
            )
        }
        .map_err(|result| VulkanError::vk("allocate_ui_overlay_descriptor", result))?[0];
        self.write_ui_overlay_descriptor(descriptor_set, view, sampler);
        self.ui_overlay_desc_sets
            .insert(key.to_owned(), descriptor_set);
        Ok(descriptor_set)
    }

    fn write_ui_overlay_descriptor(
        &self,
        descriptor_set: vk::DescriptorSet,
        view: vk::ImageView,
        sampler: vk::Sampler,
    ) {
        let image_info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&image_info)];
        // SAFETY: set/view/sampler are live compatible device handles and the
        // descriptor-info slices remain alive through the update.
        unsafe {
            self.logical_device
                .device
                .update_descriptor_sets(&writes, &[]);
        }
    }

    /// Point a cached UI descriptor at a replacement texture before the old
    /// image is destroyed.  The caller waits for GPU idle first.
    pub(crate) fn refresh_ui_overlay_texture_descriptor(
        &mut self,
        texture_id: &str,
    ) -> VkResult<()> {
        let Some(descriptor_set) = self.ui_overlay_desc_sets.get(texture_id).copied() else {
            return Ok(());
        };
        let (view, sampler) = self
            .textures
            .get(texture_id)
            .map(|texture| (texture.view, texture.sampler))
            .ok_or_else(|| {
                VulkanError::Loader(format!(
                    "replacement UI texture '{texture_id}' disappeared before descriptor update"
                ))
            })?;
        self.write_ui_overlay_descriptor(descriptor_set, view, sampler);
        Ok(())
    }

    /// Free the cached descriptor before its texture is removed.
    pub(crate) fn release_ui_overlay_texture_descriptor(
        &mut self,
        texture_id: &str,
    ) -> VkResult<()> {
        let Some(descriptor_set) = self.ui_overlay_desc_sets.remove(texture_id) else {
            return Ok(());
        };
        let Some(descriptor_pool) = self.ui_overlay_desc_pool else {
            self.ui_overlay_desc_sets
                .insert(texture_id.to_owned(), descriptor_set);
            return Err(VulkanError::Loader(
                "UI descriptor cache outlived its descriptor pool".into(),
            ));
        };
        // SAFETY: the pool was created with FREE_DESCRIPTOR_SET and this cached
        // set was allocated from it; removal gives exclusive host ownership.
        if let Err(result) = unsafe {
            self.logical_device
                .device
                .free_descriptor_sets(descriptor_pool, &[descriptor_set])
        } {
            self.ui_overlay_desc_sets
                .insert(texture_id.to_owned(), descriptor_set);
            return Err(VulkanError::vk("free_ui_overlay_descriptor", result));
        }
        Ok(())
    }

    /// Destroy every UI object that depends on the current swapchain.  The
    /// descriptor pool is included so no cached set can reference a texture
    /// after this call.
    pub(crate) fn destroy_ui_overlay_resources(&mut self) {
        let d = &self.logical_device.device;
        for framebuffer in self.ui_overlay_framebuffers.drain(..) {
            // SAFETY: resize/drop synchronization makes the device idle; drained
            // framebuffers belong exclusively to `d`.
            unsafe { d.destroy_framebuffer(framebuffer, None) };
        }
        if let Some(pipeline) = self.ui_overlay_pipeline.take() {
            // SAFETY: no submission uses this taken device-created pipeline.
            unsafe { d.destroy_pipeline(pipeline, None) };
        }
        if let Some(layout) = self.ui_overlay_pipeline_layout.take() {
            // SAFETY: the dependent pipeline was destroyed above and this
            // device-created layout is exclusively owned.
            unsafe { d.destroy_pipeline_layout(layout, None) };
        }
        self.ui_overlay_desc_sets.clear();
        if let Some(pool) = self.ui_overlay_desc_pool.take() {
            // SAFETY: the idle device owns the pool; destroying it releases all
            // descriptor sets after the host cache was cleared.
            unsafe { d.destroy_descriptor_pool(pool, None) };
        }
        if let Some(layout) = self.ui_overlay_desc_layout.take() {
            // SAFETY: pipeline layout/pool users are gone; this device-created
            // descriptor layout is exclusively owned.
            unsafe { d.destroy_descriptor_set_layout(layout, None) };
        }
        if let Some(render_pass) = self.ui_overlay_rp.take() {
            // SAFETY: framebuffer/pipeline users were destroyed above and the
            // taken pass belongs exclusively to idle `d`.
            unsafe { d.destroy_render_pass(render_pass, None) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textureless_ui_uses_the_engine_white_texture() {
        assert_eq!(ui_texture_key(None), FALLBACK_MATERIAL_TEXTURE_ID);
    }

    #[test]
    fn named_ui_texture_keeps_its_asset_id() {
        assert_eq!(ui_texture_key(Some("editor/icons")), "editor/icons");
    }

    #[test]
    fn descriptor_capacity_is_large_but_bounded() {
        assert_eq!(MAX_UI_TEXTURE_DESCRIPTORS, 1024);
    }
}
