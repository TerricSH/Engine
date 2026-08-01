macro_rules! vulkan_device_render_target_methods {
    () => {
        fn create_render_pass(
            &mut self,
            desc: &RenderPassDescriptor,
        ) -> Result<RenderPassHandle, render_core::RhiError> {
            if desc.color_attachments.len() > 1 {
                return Err(render_core::RhiError::UnsupportedFeature {
                    feature: "Vulkan generic render passes with multiple color attachments".into(),
                });
            }
            if desc.color_attachments.is_empty() && desc.depth_stencil_format.is_none() {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "render_pass.attachments".into(),
                    reason: "at least one color or depth attachment is required".into(),
                });
            }
            if desc.present_after && desc.color_attachments.is_empty() {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "render_pass.present_after".into(),
                    reason: "a present pass requires a color attachment".into(),
                });
            }
            if let Some(format) = desc.depth_stencil_format {
                if format != TextureFormat::Depth32Float {
                    return Err(render_core::RhiError::UnsupportedFeature {
                        feature: format!("Vulkan depth format {format:?}"),
                    });
                }
            }
            if !matches!(desc.sample_count, 1 | 2 | 4 | 8) {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "render_pass.sample_count".into(),
                    reason: format!("unsupported sample count {}", desc.sample_count),
                });
            }
            let d = &self.logical_device.device;
            let samples = parse_sample_count(Some(desc.sample_count));
            let vk_fmt = color_attachment_format(
                desc.color_attachments.first(),
                self.swapchain.as_ref().map(|swapchain| swapchain.format),
            );
            let has_depth = desc.depth_stencil_format.is_some();

            // Build render pass using a flat approach to avoid ash lifetime issues
            let (rp, has_depth) = if has_depth && !desc.color_attachments.is_empty() {
                let atts = [
                    vk::AttachmentDescription::default()
                        .format(vk_fmt)
                        .samples(samples)
                        .load_op(vk::AttachmentLoadOp::CLEAR)
                        .store_op(vk::AttachmentStoreOp::STORE)
                        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                        .initial_layout(vk::ImageLayout::UNDEFINED)
                        .final_layout(if desc.present_after {
                            vk::ImageLayout::PRESENT_SRC_KHR
                        } else {
                            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
                        }),
                    vk::AttachmentDescription::default()
                        .format(vk::Format::D32_SFLOAT)
                        .samples(samples)
                        .load_op(vk::AttachmentLoadOp::CLEAR)
                        .store_op(vk::AttachmentStoreOp::DONT_CARE)
                        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                        .initial_layout(vk::ImageLayout::UNDEFINED)
                        .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
                ];
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
                let subpasses = [subpass];
                let deps = [dep];
                let rp_info = vk::RenderPassCreateInfo::default()
                    .attachments(&atts)
                    .subpasses(&subpasses)
                    .dependencies(&deps);
                // SAFETY: `d` is a valid AshDevice; `rp_info` describes a valid
                // render pass with color + depth attachments; `None` means no
                // custom allocator.
                (
                    // SAFETY: the validated create-info and referenced local
                    // arrays remain alive for this device call.
                    unsafe { d.create_render_pass(&rp_info, None) }.map_err(|r| {
                        render_core::RhiError::Backend {
                            detail: format!("{r:?}"),
                        }
                    })?,
                    true,
                )
            } else if has_depth {
                let attachments = [vk::AttachmentDescription::default()
                    .format(vk::Format::D32_SFLOAT)
                    .samples(samples)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)];
                let depth_ref = vk::AttachmentReference::default()
                    .attachment(0)
                    .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
                let subpass = vk::SubpassDescription::default()
                    .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                    .depth_stencil_attachment(&depth_ref);
                let dependency = vk::SubpassDependency::default()
                    .src_subpass(vk::SUBPASS_EXTERNAL)
                    .dst_subpass(0)
                    .src_stage_mask(
                        vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                            | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                    )
                    .dst_stage_mask(
                        vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                            | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                    )
                    .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE);
                let subpasses = [subpass];
                let dependencies = [dependency];
                let info = vk::RenderPassCreateInfo::default()
                    .attachments(&attachments)
                    .subpasses(&subpasses)
                    .dependencies(&dependencies);
                // SAFETY: `d` is live; every attachment/subpass reference is
                // within the local arrays and those arrays outlive the call.
                (
                    // SAFETY: the validated create-info and referenced local
                    // arrays remain alive for this device call.
                    unsafe { d.create_render_pass(&info, None) }.map_err(|result| {
                        render_core::RhiError::Backend {
                            detail: format!("create depth-only render pass: {result:?}"),
                        }
                    })?,
                    true,
                )
            } else {
                let atts = [vk::AttachmentDescription::default()
                    .format(vk_fmt)
                    .samples(samples)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(if desc.present_after {
                        vk::ImageLayout::PRESENT_SRC_KHR
                    } else {
                        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
                    })];
                let color_ref = [vk::AttachmentReference::default()
                    .attachment(0)
                    .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
                let subpass = vk::SubpassDescription::default()
                    .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                    .color_attachments(&color_ref);
                let dep = default_dep();
                let subpasses = [subpass];
                let deps = [dep];
                let rp_info = vk::RenderPassCreateInfo::default()
                    .attachments(&atts)
                    .subpasses(&subpasses)
                    .dependencies(&deps);
                // SAFETY: `d` is a valid AshDevice; `rp_info` describes a valid
                // render pass with color attachment only; `None` means no custom
                // allocator.
                (
                    // SAFETY: the validated create-info and referenced local
                    // arrays remain alive for this device call.
                    unsafe { d.create_render_pass(&rp_info, None) }.map_err(|r| {
                        render_core::RhiError::Backend {
                            detail: format!("{r:?}"),
                        }
                    })?,
                    false,
                )
            };
            let (idx, gen) = self.render_passes.insert(rp);
            self.rp_has_depth.insert(idx, has_depth);
            self.rp_color_formats.insert(
                idx,
                desc.color_attachments
                    .iter()
                    .copied()
                    .map(texture_format)
                    .collect(),
            );
            if let Some(format) = desc.depth_stencil_format {
                self.rp_depth_formats.insert(idx, texture_format(format));
            }
            self.rp_sample_counts.insert(idx, desc.sample_count);
            Ok(RenderPassHandle::new(idx, gen))
        }

        fn destroy_render_pass(&mut self, pass: RenderPassHandle) {
            if let Some(render_pass) = self.render_passes.remove(pass.index, pass.generation) {
                self.rp_has_depth.remove(&pass.index);
                self.rp_color_formats.remove(&pass.index);
                self.rp_depth_formats.remove(&pass.index);
                self.rp_sample_counts.remove(&pass.index);
                // SAFETY: slab removal transfers exclusive ownership of a pass
                // created by this device; dependent framebuffers/pipelines are
                // required by the RHI lifecycle to be destroyed first.
                unsafe {
                    self.logical_device
                        .device
                        .destroy_render_pass(render_pass, None);
                }
            }
        }

        fn create_framebuffer(
            &mut self,
            desc: &FramebufferDescriptor,
        ) -> Result<FramebufferHandle, render_core::RhiError> {
            if desc.width == 0 || desc.height == 0 {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "framebuffer".into(),
                    reason: "width and height must be non-zero".into(),
                });
            }
            let d = &self.logical_device.device;
            let rp = self
                .render_passes
                .get(desc.render_pass.index, desc.render_pass.generation)
                .copied()
                .ok_or(render_core::RhiError::InvalidHandle)?;
            let has_depth = self
                .rp_has_depth
                .get(&desc.render_pass.index)
                .copied()
                .unwrap_or(false);
            let expected_colors = self
                .rp_color_formats
                .get(&desc.render_pass.index)
                .ok_or(render_core::RhiError::InvalidHandle)?;
            let expected_samples = self
                .rp_sample_counts
                .get(&desc.render_pass.index)
                .copied()
                .ok_or(render_core::RhiError::InvalidHandle)?;
            if desc.color_attachments.len() != expected_colors.len() {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "framebuffer.color_attachments".into(),
                    reason: format!(
                        "render pass expects {} color attachment(s), got {}",
                        expected_colors.len(),
                        desc.color_attachments.len()
                    ),
                });
            }
            if has_depth != desc.depth_stencil_attachment.is_some() {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "framebuffer.depth_stencil_attachment".into(),
                    reason: "framebuffer depth attachment does not match its render pass".into(),
                });
            }
            let mut attachments = Vec::with_capacity(
                desc.color_attachments.len() + usize::from(desc.depth_stencil_attachment.is_some()),
            );
            for (handle, expected_format) in desc.color_attachments.iter().zip(expected_colors) {
                let texture = self
                    .rhi_textures
                    .get(handle.index, handle.generation)
                    .ok_or(render_core::RhiError::InvalidHandle)?;
                if texture.format != *expected_format
                    || texture.width != desc.width
                    || texture.height != desc.height
                    || texture.sample_count != expected_samples
                {
                    return Err(render_core::RhiError::InvalidDescriptor {
                        field: "framebuffer.color_attachments".into(),
                        reason: "color attachment format or extent is incompatible".into(),
                    });
                }
                attachments.push(texture.view);
            }
            if let Some(handle) = desc.depth_stencil_attachment {
                let expected_depth = self
                    .rp_depth_formats
                    .get(&desc.render_pass.index)
                    .copied()
                    .ok_or(render_core::RhiError::InvalidHandle)?;
                let texture = self
                    .rhi_textures
                    .get(handle.index, handle.generation)
                    .ok_or(render_core::RhiError::InvalidHandle)?;
                if texture.format != expected_depth
                    || texture.width != desc.width
                    || texture.height != desc.height
                    || texture.sample_count != expected_samples
                {
                    return Err(render_core::RhiError::InvalidDescriptor {
                        field: "framebuffer.depth_stencil_attachment".into(),
                        reason: "depth attachment must be matching-extent Depth32Float".into(),
                    });
                }
                attachments.push(texture.view);
            }
            if attachments.is_empty() {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "framebuffer.attachments".into(),
                    reason: "at least one attachment is required".into(),
                });
            }
            let info = vk::FramebufferCreateInfo::default()
                .render_pass(rp)
                .attachments(&attachments)
                .width(desc.width)
                .height(desc.height)
                .layers(1);
            // SAFETY: `render_pass` and every attachment view are live handles
            // from `d`; the attachment slice remains valid for this call and
            // dimensions were validated against the RHI descriptor.
            let framebuffer = unsafe { d.create_framebuffer(&info, None) }.map_err(|result| {
                render_core::RhiError::Backend {
                    detail: format!("create framebuffer: {result:?}"),
                }
            })?;
            let (idx, gen) = self.framebuffers.insert(FbEntry {
                framebuffer,
                color_attachment_count: desc.color_attachments.len() as u32,
                has_depth,
            });
            Ok(FramebufferHandle::new(idx, gen))
        }

        fn destroy_framebuffer(&mut self, framebuffer: FramebufferHandle) {
            if let Some(framebuffer) = self
                .framebuffers
                .remove(framebuffer.index, framebuffer.generation)
            {
                // SAFETY: slab removal gives exclusive ownership of this
                // device-created framebuffer after its render work completed.
                unsafe {
                    self.logical_device
                        .device
                        .destroy_framebuffer(framebuffer.framebuffer, None);
                }
            }
        }

        fn create_pipeline_layout(
            &mut self,
            desc: &PipelineLayoutDescriptor,
        ) -> Result<PipelineLayoutHandle, render_core::RhiError> {
            let allowed_stage_flags = (vk::ShaderStageFlags::VERTEX
                | vk::ShaderStageFlags::FRAGMENT
                | vk::ShaderStageFlags::COMPUTE)
                .as_raw();
            let max_push_constants = self.adapter.properties.limits.max_push_constants_size;
            for range in &desc.push_constant_ranges {
                let end = range.offset.checked_add(range.size).ok_or_else(|| {
                    render_core::RhiError::InvalidDescriptor {
                        field: "pipeline_layout.push_constant_ranges".into(),
                        reason: "push constant range overflows u32".into(),
                    }
                })?;
                if range.size == 0
                    || !range.offset.is_multiple_of(4)
                    || !range.size.is_multiple_of(4)
                    || range.stage_flags == 0
                    || range.stage_flags & !allowed_stage_flags != 0
                    || end > max_push_constants
                {
                    return Err(render_core::RhiError::InvalidDescriptor {
                        field: "pipeline_layout.push_constant_ranges".into(),
                        reason: format!(
                            "range offset={} size={} stages={:#x} exceeds Vulkan limits",
                            range.offset, range.size, range.stage_flags
                        ),
                    });
                }
            }
            let d = &self.logical_device.device;
            let pc_ranges: Vec<vk::PushConstantRange> = desc
                .push_constant_ranges
                .iter()
                .map(|pc| vk::PushConstantRange {
                    stage_flags: vk::ShaderStageFlags::from_raw(pc.stage_flags),
                    offset: pc.offset,
                    size: pc.size,
                })
                .collect();

            // ── Gather descriptor set layouts ──────────────────────────────
            // If the descriptor provides explicit bind_group_layouts, create
            // VkDescriptorSetLayout objects from them.  Otherwise fall back to
            // the existing per-frame (set=0) + shadow (set=1) layouts.
            let mut set_layouts: Vec<vk::DescriptorSetLayout>;
            let mut owned_set_layouts: Vec<vk::DescriptorSetLayout> = Vec::new();

            if desc.bind_group_layouts.is_empty() {
                // Fallback: use existing per-frame + shadow + material layouts
                set_layouts = fallback_pipeline_set_layouts(
                    self.desc_set_layout_0,
                    self.shadow_desc_layout,
                    self.material_desc_set_layout,
                )?;
            } else {
                set_layouts = Vec::new();
                let ordered = ordered_bind_group_layouts(&desc.bind_group_layouts)?;
                let binding_sets = ordered
                    .iter()
                    .map(|layout| vulkan_descriptor_bindings(layout))
                    .collect::<Result<Vec<_>, _>>()?;
                for vk_bindings in binding_sets {
                    let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&vk_bindings);
                    // SAFETY: `d` is a valid AshDevice; `info` describes a valid
                    // descriptor set layout; `None` means no custom allocator.
                    let sl = match unsafe { d.create_descriptor_set_layout(&info, None) } {
                        Ok(layout) => layout,
                        Err(result) => {
                            for layout in owned_set_layouts.drain(..) {
                                // SAFETY: each layout was created by `d` in this
                                // transaction and no pipeline layout owns it yet.
                                unsafe { d.destroy_descriptor_set_layout(layout, None) };
                            }
                            return Err(render_core::RhiError::Backend {
                                detail: format!("create descriptor set layout: {result:?}"),
                            });
                        }
                    };
                    owned_set_layouts.push(sl);
                    set_layouts.push(sl);
                }
            }

            let info = vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&set_layouts)
                .push_constant_ranges(&pc_ranges);
            // SAFETY: `d` is a valid AshDevice; `info` describes a valid pipeline
            // layout with descriptor set layouts and push constant ranges; `None`
            // means no custom allocator.
            let layout = match unsafe { d.create_pipeline_layout(&info, None) } {
                Ok(layout) => layout,
                Err(result) => {
                    for layout in owned_set_layouts.drain(..) {
                        // SAFETY: pipeline-layout creation failed, leaving each
                        // just-created descriptor layout exclusively owned here.
                        unsafe { d.destroy_descriptor_set_layout(layout, None) };
                    }
                    return Err(render_core::RhiError::Backend {
                        detail: format!("create pipeline layout: {result:?}"),
                    });
                }
            };
            let (idx, gen) = self.pipeline_layouts.insert(PlEntry {
                layout,
                set_layouts: owned_set_layouts,
                _device: d.clone(),
            });
            Ok(PipelineLayoutHandle::new(idx, gen))
        }

        fn destroy_pipeline_layout(&mut self, layout: PipelineLayoutHandle) {
            if let Some(layout) = self
                .pipeline_layouts
                .remove(layout.index, layout.generation)
            {
                for set_layout in layout.set_layouts {
                    // SAFETY: the slab entry exclusively owns these layouts;
                    // dependent descriptor sets/pipelines have been retired.
                    unsafe {
                        self.logical_device
                            .device
                            .destroy_descriptor_set_layout(set_layout, None);
                    }
                }
                // SAFETY: the pipeline-layout handle was created by this device,
                // removed from the slab, and has no surviving pipeline users.
                unsafe {
                    self.logical_device
                        .device
                        .destroy_pipeline_layout(layout.layout, None);
                }
            }
        }
    };
}

pub(super) use vulkan_device_render_target_methods;
