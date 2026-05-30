//! `render_core::Device` trait implementation for VulkanDevice.
//!
//! This file implements the render-core abstraction layer, covering buffer
//! creation, render-pass / framebuffer / pipeline creation, frame lifecycle
//! (begin/end), and pixel readback.

use ash::vk;

use render_core::{
    self, AdapterInfo, BufferDescriptor, BufferHandle, CommandEncoder as CmdEncoderTrait,
    FramebufferDescriptor, FramebufferHandle, PipelineDescriptor, PipelineHandle,
    PipelineLayoutDescriptor, PipelineLayoutHandle, RenderPassDescriptor, RenderPassHandle,
    RendererStatistics, ShaderModuleDescriptor, ShaderModuleHandle, SurfaceDescriptor,
    SurfaceHandle, SwapchainDescriptor, SwapchainHandle, TextureDescriptor, TextureFormat,
    TextureHandle,
};

use crate::allocator::{AllocationCreateDesc, AllocationScheme, MemoryLocation};

use super::{
    blend_attachment_from_mode, compare_op, default_dep,
    encoder::VkCmdEncoder,
    mk_sm, parse_polygon_mode, parse_sample_count, parse_topology,
    resource_kind_to_descriptor_type,
    slab::{BufEntry, PipeEntry, PlEntry},
    vfmt, VulkanDevice,
};

impl render_core::Device for VulkanDevice {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.cached_adapter_info
    }
    fn create_surface(
        &mut self,
        _: &SurfaceDescriptor,
    ) -> Result<SurfaceHandle, render_core::RhiError> {
        Ok(SurfaceHandle::new(0, 1))
    }
    fn create_swapchain(
        &mut self,
        _: &SwapchainDescriptor,
    ) -> Result<SwapchainHandle, render_core::RhiError> {
        Ok(SwapchainHandle::new(1, 1))
    }

    fn create_buffer(
        &mut self,
        desc: &BufferDescriptor,
    ) -> Result<BufferHandle, render_core::RhiError> {
        let d = &self.logical_device;
        let size = desc.size_bytes.max(1);
        let usage = vk::BufferUsageFlags::TRANSFER_DST
            | vk::BufferUsageFlags::VERTEX_BUFFER
            | vk::BufferUsageFlags::INDEX_BUFFER;
        let bi = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d.device` is a valid AshDevice; `bi` describes a valid
        // buffer; `None` means no custom allocator.
        let buffer = unsafe { d.device.create_buffer(&bi, None) }.map_err(|r| {
            render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            }
        })?;
        // SAFETY: `buffer` was just created by this device; querying memory
        // requirements for a valid buffer is safe.
        let req = unsafe { d.device.get_buffer_memory_requirements(buffer) };
        let alloc_handle = d.allocator();
        let location = MemoryLocation::CpuToGpu;
        let mut allocation = alloc_handle
            .lock()
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("allocator lock: {e}"),
            })?
            .allocate(&AllocationCreateDesc {
                name: "device-buffer",
                requirements: req,
                location,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| render_core::RhiError::Backend {
                detail: e.to_string(),
            })?;
        // SAFETY: `buffer` was created by this device; `allocation` was created
        // for this buffer's memory requirements; the memory and offset are valid.
        if let Err(r) = unsafe {
            d.device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
        } {
            if let Ok(mut alloc_guard) = alloc_handle.lock() {
                alloc_guard.free(&mut allocation);
            }
            // SAFETY: `buffer` was just created; not bound to memory; destroying
            // a freshly-created buffer is safe even on failed bind.
            unsafe {
                d.device.destroy_buffer(buffer, None);
            }
            return Err(render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            });
        }
        let (idx, gen) = self.buffers.insert(BufEntry {
            buffer,
            allocator: alloc_handle,
            allocation: Some(allocation),
        });
        Ok(BufferHandle::new(idx, gen))
    }

    fn write_buffer(
        &mut self,
        buf: BufferHandle,
        data: &[u8],
        offset: u64,
    ) -> Result<(), render_core::RhiError> {
        let entry = self
            .buffers
            .get_mut(buf.index, buf.generation)
            .ok_or(render_core::RhiError::InvalidHandle)?;
        let alloc = entry
            .allocation
            .as_mut()
            .ok_or_else(|| render_core::RhiError::Backend {
                detail: "no alloc".into(),
            })?;
        let slice = alloc
            .mapped_slice_mut()
            .ok_or_else(|| render_core::RhiError::Backend {
                detail: "not mapped".into(),
            })?;
        let end = (offset as usize + data.len()).min(slice.len());
        slice[offset as usize..end].copy_from_slice(&data[..end - offset as usize]);
        Ok(())
    }

    fn create_texture(
        &mut self,
        _: &TextureDescriptor,
    ) -> Result<TextureHandle, render_core::RhiError> {
        Err(render_core::RhiError::Backend {
            detail: "not in Phase 2".into(),
        })
    }

    fn create_shader_module(
        &mut self,
        _desc: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, render_core::RhiError> {
        let d = &self.logical_device.device;
        // Get SPIR-V bytes.  The descriptor does not carry inline SPIR-V yet
        // (Phase 2), so fall back to the embedded MVP vertex shader as a
        // placeholder.  This establishes the storage pipeline.
        let spv = self
            .mvp_vert_spv
            .ok_or_else(|| render_core::RhiError::Backend {
                detail: "create_shader_module: no embedded shaders available".into(),
            })?;
        // SAFETY: `d` is a valid AshDevice; `spv` is valid SPIR-V.
        let sm = (unsafe { mk_sm(d, spv) }).map_err(|e| render_core::RhiError::Backend {
            detail: format!("create_shader_module: {e}"),
        })?;
        // Default to VERTEX stage; the descriptor does not carry stage info yet.
        let (idx, gen) = self
            .shader_modules
            .insert((sm, vk::ShaderStageFlags::VERTEX));
        Ok(ShaderModuleHandle::new(idx, gen))
    }

    fn create_render_pass(
        &mut self,
        desc: &RenderPassDescriptor,
    ) -> Result<RenderPassHandle, render_core::RhiError> {
        let d = &self.logical_device.device;
        let vk_fmt = match desc.color_attachments.first() {
            Some(TextureFormat::Bgra8Unorm) => vk::Format::B8G8R8A8_UNORM,
            Some(TextureFormat::Rgba8Unorm) => vk::Format::R8G8B8A8_UNORM,
            Some(TextureFormat::Rgba16Float) => vk::Format::R16G16B16A16_SFLOAT,
            _ => vk::Format::B8G8R8A8_UNORM,
        };
        let has_depth = desc.depth_stencil_format.is_some();

        // Build render pass using a flat approach to avoid ash lifetime issues
        let (rp, has_depth) = if has_depth {
            let atts = [
                vk::AttachmentDescription::default()
                    .format(vk_fmt)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::PRESENT_SRC_KHR),
                vk::AttachmentDescription::default()
                    .format(vk::Format::D32_SFLOAT)
                    .samples(vk::SampleCountFlags::TYPE_1)
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
                unsafe { d.create_render_pass(&rp_info, None) }.map_err(|r| {
                    render_core::RhiError::Backend {
                        detail: format!("{r:?}"),
                    }
                })?,
                true,
            )
        } else {
            let atts = [vk::AttachmentDescription::default()
                .format(vk_fmt)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
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
        Ok(RenderPassHandle::new(idx, gen))
    }

    fn create_framebuffer(
        &mut self,
        desc: &FramebufferDescriptor,
    ) -> Result<FramebufferHandle, render_core::RhiError> {
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
        let fb = if has_depth {
            let depth_view = self.depth_image_view.unwrap_or(vk::ImageView::null());
            let atts = [vk::ImageView::null(), depth_view];
            let fi = vk::FramebufferCreateInfo::default()
                .render_pass(rp)
                .attachments(&atts)
                .width(desc.width)
                .height(desc.height)
                .layers(1);
            // SAFETY: `d` is a valid AshDevice; `fi` references a valid render
            // pass and image views (null image views are allowed for placeholders);
            // `None` means no custom allocator.
            unsafe { d.create_framebuffer(&fi, None) }
        } else {
            let fi = vk::FramebufferCreateInfo::default()
                .render_pass(rp)
                .width(desc.width)
                .height(desc.height)
                .layers(1);
            // SAFETY: `d` is a valid AshDevice; `fi` references a valid render
            // pass; `None` means no custom allocator.
            unsafe { d.create_framebuffer(&fi, None) }
        }
        .map_err(|r| render_core::RhiError::Backend {
            detail: format!("{r:?}"),
        })?;
        let (idx, gen) = self.framebuffers.insert(fb);
        Ok(FramebufferHandle::new(idx, gen))
    }

    fn create_pipeline_layout(
        &mut self,
        desc: &PipelineLayoutDescriptor,
    ) -> Result<PipelineLayoutHandle, render_core::RhiError> {
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
        let mut set_layouts: Vec<vk::DescriptorSetLayout> = Vec::new();
        let mut owned_set_layouts: Vec<vk::DescriptorSetLayout> = Vec::new();

        if desc.bind_group_layouts.is_empty() {
            // Fallback: use existing per-frame + shadow layouts
            if let Some(dsl) = self.desc_set_layout_0 {
                set_layouts.push(dsl);
            }
            if let Some(sdl) = self.shadow_desc_layout {
                set_layouts.push(sdl);
            }
        } else {
            for bg in &desc.bind_group_layouts {
                let vk_bindings: Vec<vk::DescriptorSetLayoutBinding> = bg
                    .bindings
                    .iter()
                    .map(|b| {
                        vk::DescriptorSetLayoutBinding::default()
                            .binding(b.binding)
                            .descriptor_type(resource_kind_to_descriptor_type(&b.resource_kind))
                            .descriptor_count(1)
                            .stage_flags(
                                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                            )
                    })
                    .collect();
                let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&vk_bindings);
                // SAFETY: `d` is a valid AshDevice; `info` describes a valid
                // descriptor set layout; `None` means no custom allocator.
                let sl = unsafe { d.create_descriptor_set_layout(&info, None) }.map_err(|r| {
                    render_core::RhiError::Backend {
                        detail: format!("create descriptor set layout: {r:?}"),
                    }
                })?;
                owned_set_layouts.push(sl);
                // Ensure the vector is large enough for this set_index
                while (set_layouts.len() as u8) <= bg.set_index {
                    set_layouts.push(vk::DescriptorSetLayout::null());
                }
                set_layouts[bg.set_index as usize] = sl;
            }
        }

        let info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&pc_ranges);
        // SAFETY: `d` is a valid AshDevice; `info` describes a valid pipeline
        // layout with descriptor set layouts and push constant ranges; `None`
        // means no custom allocator.
        let layout = unsafe { d.create_pipeline_layout(&info, None) }.map_err(|r| {
            render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            }
        })?;
        let (idx, gen) = self.pipeline_layouts.insert(PlEntry {
            layout,
            set_layouts: owned_set_layouts,
            _device: d.clone(),
        });
        Ok(PipelineLayoutHandle::new(idx, gen))
    }

    fn create_pipeline(
        &mut self,
        desc: &PipelineDescriptor,
    ) -> Result<PipelineHandle, render_core::RhiError> {
        let d = &self.logical_device.device;
        let main = c"main";

        // ── Shader stages ──────────────────────────────────────────────
        // If the descriptor provides shader module handles, resolve them
        // from the shader_modules slab.  Otherwise fall back to the
        // embedded MVP vertex/fragment SPIR-V.
        let (sr, destroy_temp_modules) = if desc.shader_modules.is_empty() {
            // Fallback: use mvp_vert_spv / mvp_frag_spv
            let (vert, frag) = (self.mvp_vert_spv, self.mvp_frag_spv);
            let (vs, fs) = (
                vert.ok_or_else(|| render_core::RhiError::Backend {
                    detail: "no vert spv".into(),
                })?,
                frag.ok_or_else(|| render_core::RhiError::Backend {
                    detail: "no frag spv".into(),
                })?,
            );
            // SAFETY: `d` is a valid AshDevice; `vs`/`fs` contain valid SPIR-V.
            let vm = (unsafe { mk_sm(d, vs) }).map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
            let fm = (unsafe { mk_sm(d, fs) }).map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
            let stages: Vec<vk::PipelineShaderStageCreateInfo> = vec![
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(vm)
                    .name(main),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(fm)
                    .name(main),
            ];
            // These temp modules must be destroyed after pipeline creation.
            let temp_modules = vec![vm, fm];
            (stages, Some(temp_modules))
        } else {
            // Resolve shader modules from handles
            let stages: Vec<vk::PipelineShaderStageCreateInfo> = desc
                .shader_modules
                .iter()
                .map(|handle| {
                    let (sm, stage) = self
                        .shader_modules
                        .get(handle.index, handle.generation)
                        .copied()
                        .ok_or(render_core::RhiError::InvalidHandle)?;
                    Ok(vk::PipelineShaderStageCreateInfo::default()
                        .stage(stage)
                        .module(sm)
                        .name(main))
                })
                .collect::<Result<Vec<_>, _>>()?;
            // Modules are owned by the slab; do NOT destroy them here.
            (stages, None)
        };

        // ── Vertex input state ─────────────────────────────────────────
        let stride = desc.vertex_layout.stride_bytes;
        let vb = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(stride)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let va: Vec<vk::VertexInputAttributeDescription> = desc
            .vertex_layout
            .attributes
            .iter()
            .enumerate()
            .map(|(i, a)| vk::VertexInputAttributeDescription {
                location: i as u32,
                binding: 0,
                format: vfmt(&a.format),
                offset: a.offset_bytes,
            })
            .collect();
        let vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vb)
            .vertex_attribute_descriptions(&va);

        // ── Input assembly (topology from descriptor) ─────────────────
        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(parse_topology(&desc.topology));

        // ── Viewport state ─────────────────────────────────────────────
        let vs2 = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        // ── Rasterization state (polygon mode + cull mode from desc) ──
        let cull_mode = match desc.raster_state.cull_mode.as_deref() {
            Some("front") => vk::CullModeFlags::FRONT,
            Some("back") => vk::CullModeFlags::BACK,
            Some("none") | None => vk::CullModeFlags::NONE,
            _ => vk::CullModeFlags::NONE,
        };
        let front_face = match desc.raster_state.front_face.as_deref() {
            Some("clockwise") => vk::FrontFace::CLOCKWISE,
            _ => vk::FrontFace::COUNTER_CLOCKWISE,
        };
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(parse_polygon_mode(&desc.polygon_mode))
            .cull_mode(cull_mode)
            .front_face(front_face)
            .line_width(1.0);

        // ── Multisample state (sample count from desc) ─────────────────
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(parse_sample_count(desc.sample_count));

        // ── Color blend state ──────────────────────────────────────────
        let blend_attachment = match &desc.blend_state.mode {
            Some(mode) => blend_attachment_from_mode(mode),
            None => blend_attachment_from_mode("Opaque"),
        };
        let cba = [blend_attachment];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);

        // ── Dynamic state ──────────────────────────────────────────────
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);

        // ── Render pass ────────────────────────────────────────────────
        // If the descriptor carries a handle, resolve it; otherwise create
        // an inline render pass from the descriptor's render targets.
        let rp = match desc.render_pass {
            Some(h) => self
                .render_passes
                .get(h.index, h.generation)
                .copied()
                .ok_or(render_core::RhiError::InvalidHandle)?,
            None => {
                // Check cached mvp_rp first, then create inline
                if let Some(rp_) = self.mvp_rp {
                    rp_
                } else {
                    let fmt = match desc.render_targets.first() {
                        Some(TextureFormat::Bgra8Unorm) => vk::Format::B8G8R8A8_UNORM,
                        Some(TextureFormat::Rgba8Unorm) => vk::Format::R8G8B8A8_UNORM,
                        _ => vk::Format::B8G8R8A8_SRGB,
                    };
                    let at = vk::AttachmentDescription::default()
                        .format(fmt)
                        .samples(parse_sample_count(desc.sample_count))
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
                    // SAFETY: `d` is a valid AshDevice; `rpi` describes a
                    // valid render pass; `None` means no custom allocator.
                    unsafe { d.create_render_pass(&rpi, None) }.map_err(|r| {
                        render_core::RhiError::Backend {
                            detail: format!("{r:?}"),
                        }
                    })?
                }
            }
        };

        // ── Pipeline layout ────────────────────────────────────────────
        let pll = match desc.pipeline_layout {
            Some(h) => self
                .pipeline_layouts
                .get(h.index, h.generation)
                .map(|e| e.layout)
                .unwrap_or(vk::PipelineLayout::null()),
            None => vk::PipelineLayout::null(),
        };

        // ── Depth stencil state ────────────────────────────────────────
        let depth_enabled = desc.depth_state.write_enabled || desc.depth_state.compare.is_some();
        let ds_state = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(depth_enabled)
            .depth_write_enable(desc.depth_state.write_enabled)
            .depth_compare_op(compare_op(&desc.depth_state.compare));

        // ── Build the pipeline ─────────────────────────────────────────
        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs2)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .depth_stencil_state(&ds_state)
            .color_blend_state(&cb)
            .dynamic_state(&ds)
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid
        // graphics pipeline; `self.pipeline_cache` may be null; `None` means
        // no custom allocator.
        let pipeline = unsafe { d.create_graphics_pipelines(self.pipeline_cache, &[pinfo], None) }
            .map_err(|(_, r)| render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            })?[0];

        // Destroy temporary shader modules created in the fallback path.
        if let Some(modules) = destroy_temp_modules {
            // SAFETY: modules were created by this device and are no longer
            // needed after pipeline creation; `None` means no custom allocator.
            for m in modules {
                unsafe {
                    d.destroy_shader_module(m, None);
                }
            }
        }

        let (idx, gen) = self.pipelines.insert(PipeEntry { pipeline });
        Ok(PipelineHandle::new(idx, gen))
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) {
        if let Some(entry) = self.pipelines.remove(handle.index, handle.generation) {
            self.retire_pipeline(entry.pipeline);
        }
    }

    fn begin_frame(
        &mut self,
        _: SwapchainHandle,
    ) -> Result<(u32, Box<dyn CmdEncoderTrait>), render_core::RhiError> {
        self.ensure_sc()
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
        if self.frame_sync.is_empty() {
            self.build_frames()
                .map_err(|e| render_core::RhiError::Backend {
                    detail: format!("{e}"),
                })?;
        }
        let fi = self.current_frame;
        let (ii, _) = self
            .acquire(fi)
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
        self.last_image_index = ii;
        self.begin_cb(fi)
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
        let f = &self.frame_sync[fi];
        let desc_set = self
            .frame_desc_sets
            .get(fi)
            .copied()
            .unwrap_or(vk::DescriptorSet::null());
        let encoder = Box::new(VkCmdEncoder {
            device: self.logical_device.device.clone(),
            cmd: f.command_buffer,
            // Snapshot slab entries into owned Vec caches — no raw pointers.
            pipeline_cache: self
                .pipelines
                .slots
                .iter()
                .map(|s| s.as_ref().map(|(g, e)| (*g, e.pipeline)))
                .collect(),
            buffer_cache: self
                .buffers
                .slots
                .iter()
                .map(|s| s.as_ref().map(|(g, e)| (*g, e.buffer)))
                .collect(),
            render_pass_cache: self.render_passes.slots.clone(),
            framebuffer_cache: self.framebuffers.slots.clone(),
            pipeline_layout_cache: self
                .pipeline_layouts
                .slots
                .iter()
                .map(|s| s.as_ref().map(|(g, e)| (*g, e.layout)))
                .collect(),
            current_desc_set: desc_set,
        });

        // Pre-bind the shadow descriptor set at set=1 (if available) so that
        // subsequent encoder operations do not leave it unbound.  The encoder
        // later binds the UBO at set=0 via `bind_descriptor_sets`.
        if let Some(sds) = self.shadow_desc_set {
            if let Some(bind_pll) = self.shadow_bind_layout {
                let shadow_sets = [sds];
                // SAFETY: command buffer is in recording state; descriptor set,
                // pipeline layout, and command buffer are valid Vulkan objects
                // created by the same device.
                unsafe {
                    self.logical_device.device.cmd_bind_descriptor_sets(
                        f.command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        bind_pll,
                        1,
                        &shadow_sets,
                        &[],
                    );
                }
            }
        }

        Ok((ii, encoder))
    }

    fn end_frame(
        &mut self,
        _: SwapchainHandle,
        _: Box<dyn CmdEncoderTrait>,
        ii: u32,
    ) -> Result<RendererStatistics, render_core::RhiError> {
        let fi = self.current_frame;
        let subopt =
            self.submit_and_present(fi, ii)
                .map_err(|e| render_core::RhiError::Backend {
                    detail: format!("{e}"),
                })?;
        if subopt {
            // SAFETY: `self.logical_device` is alive by type invariant
            // (ManuallyDrop ensures destruction order).
            unsafe { let _ = self.logical_device.device.device_wait_idle(); };
            self.swapchain = None;
        }
        self.current_frame = (fi + 1) % 2;
        Ok(RendererStatistics {
            draw_calls: 1,
            triangles: 0,
            gpu_frame_ms: 0.0,
        })
    }

    fn recreate_swapchain(
        &mut self,
        _: SwapchainHandle,
        w: u32,
        h: u32,
    ) -> Result<(), render_core::RhiError> {
        // SAFETY: `self.logical_device` is alive by type invariant
        // (ManuallyDrop ensures destruction order).
        unsafe { let _ = self.logical_device.device.device_wait_idle(); };
        self.window_width = w.max(1);
        self.window_height = h.max(1);
        self.swapchain = None;
        Ok(())
    }

    fn wait_idle(&self) {
        // SAFETY: `self.logical_device` is alive by type invariant
        // (ManuallyDrop ensures destruction order).
        unsafe { let _ = self.logical_device.device.device_wait_idle(); };
    }

    fn read_pixels(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, render_core::RhiError> {
        // Flush all pending GPU work so the swapchain images are in a
        // deterministic layout (PRESENT_SRC_KHR after the last render pass).
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures destruction order).
        unsafe { let _ = self.logical_device.device.device_wait_idle(); };

        let sc = self
            .swapchain
            .as_ref()
            .ok_or_else(|| render_core::RhiError::Backend {
                detail: "no swapchain".into(),
            })?;

        // Validate the requested region against the swapchain extent.
        if x + width > sc.extent.width || y + height > sc.extent.height || width == 0 || height == 0
        {
            return Err(render_core::RhiError::Backend {
                detail: format!(
                    "readback region ({x},{y}) {width}×{height} exceeds swapchain {}×{}",
                    sc.extent.width, sc.extent.height
                ),
            });
        }

        // Pixel buffer: 4 bytes per pixel (RGBA return format).
        let pixel_size: vk::DeviceSize = 4;
        let buffer_size = (width as vk::DeviceSize) * (height as vk::DeviceSize) * pixel_size;

        let d = &self.logical_device;
        let device = &d.device;

        // -----------------------------------------------------------------
        // 1. Create a staging buffer (GPU write → CPU read).
        //    NOTE: The swapchain images MUST have been created with
        //    VK_IMAGE_USAGE_TRANSFER_SRC_BIT for vkCmdCopyImageToBuffer to
        //    work.  Add this to the usage flags in swapchain::new().
        // -----------------------------------------------------------------
        // SAFETY: `device` is a valid AshDevice; buffer creation describes a
        // valid TRANSFER_DST buffer; `None` means no custom allocator.
        let staging_buffer = unsafe {
            device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(buffer_size)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .map_err(|r| render_core::RhiError::Backend {
            detail: format!("create staging buffer: {r:?}"),
        })?;

        // SAFETY: `staging_buffer` was just created by this device; querying
        // memory requirements for a valid buffer is safe.
        let req = unsafe { device.get_buffer_memory_requirements(staging_buffer) };
        let alloc_handle = d.allocator();
        let mut staging_alloc = alloc_handle
            .lock()
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("allocator lock: {e}"),
            })?
            .allocate(&AllocationCreateDesc {
                name: "read_pixels staging",
                requirements: req,
                location: MemoryLocation::GpuToCpu,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| {
                // SAFETY: buffer was just created by this device and is not
                // in use; destroying it on allocation failure is correct.
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("alloc staging: {e}"),
                }
            })?;

        // SAFETY: `staging_buffer` was created by this device; `staging_alloc`
        // was created for this buffer's memory requirements; memory and offset
        // are valid.
        if let Err(r) = unsafe {
            device.bind_buffer_memory(
                staging_buffer,
                staging_alloc.memory(),
                staging_alloc.offset(),
            )
        } {
            // SAFETY: buffer/allocation were just created and are not in use
            // after the failed bind; cleanup is safe.
            if let Ok(mut guard) = alloc_handle.lock() {
                guard.free(&mut staging_alloc);
            }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            return Err(render_core::RhiError::Backend {
                detail: format!("bind staging: {r:?}"),
            });
        }

        // -----------------------------------------------------------------
        // 2. One-shot command pool + command buffer.
        // -----------------------------------------------------------------
        // SAFETY: `device` is a valid AshDevice; the queue family index is
        // valid for this device; `None` means no custom allocator.
        let cmd_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(d.queue_family_index)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )
        }
        .map_err(|r| {
            if let Ok(mut guard) = alloc_handle.lock() {
                guard.free(&mut staging_alloc);
            }
            // SAFETY: cleanup only happen on error; all handles are valid.
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("create pool: {r:?}"),
            }
        })?;

        // SAFETY: `cmd_pool` was just created and is valid; allocation info
        // correctly references the pool with PRIMARY level and 1 buffer.
        let cmd_buffer = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(cmd_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(|r| {
            // SAFETY: cleanup only on error; all handles created so far are valid.
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() {
                guard.free(&mut staging_alloc);
            }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("alloc cb: {r:?}"),
            }
        })?[0];

        // -----------------------------------------------------------------
        // 3. Record the copy command buffer.
        // -----------------------------------------------------------------
        // SAFETY: command buffer is in the initial state (just allocated from
        // a transient pool); begin transitions it to recording state.
        unsafe {
            device.begin_command_buffer(
                cmd_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
        }
        .map_err(|r| {
            // SAFETY: cleanup only on error; all handles created so far are valid.
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() {
                guard.free(&mut staging_alloc);
            }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("begin cb: {r:?}"),
            }
        })?;

        // Use the last acquired image index (tracked from render_model_frame).
        let img_idx = self.last_image_index.min(sc.images.len() as u32 - 1);
        let swapchain_image = sc.images[img_idx as usize];

        // 3a. PRESENT_SRC_KHR → TRANSFER_SRC_OPTIMAL
        let to_transfer_barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(swapchain_image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
        // SAFETY: command buffer is in recording state; barrier references a
        // live swapchain image; stage and access masks match the layout
        // transition semantics.
        unsafe {
            device.cmd_pipeline_barrier(
                cmd_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_transfer_barrier],
            );
        }

        // 3b. Copy the requested region from image → staging buffer.
        let copy_region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_offset(vk::Offset3D {
                x: x as i32,
                y: y as i32,
                z: 0,
            })
            .image_extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            });
        // SAFETY: both image and buffer are valid Vulkan objects; image is in
        // TRANSFER_SRC_OPTIMAL layout; copy region is within bounds.
        unsafe {
            device.cmd_copy_image_to_buffer(
                cmd_buffer,
                swapchain_image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                staging_buffer,
                &[copy_region],
            );
        }

        // 3c. TRANSFER_SRC_OPTIMAL → PRESENT_SRC_KHR (restore).
        let to_present_barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(swapchain_image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::empty());
        // SAFETY: command buffer is still recording; image is live; restoring
        // the original layout matches the swapchain contract.
        unsafe {
            device.cmd_pipeline_barrier(
                cmd_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_present_barrier],
            );
        }

        // SAFETY: command buffer is in recording state; after this call it
        // transitions to completed state, ready for submission.
        unsafe { device.end_command_buffer(cmd_buffer) }.map_err(|r| {
            // SAFETY: cleanup only on error; all handles created so far are valid.
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() {
                guard.free(&mut staging_alloc);
            }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("end cb: {r:?}"),
            }
        })?;

        // -----------------------------------------------------------------
        // 4. Submit and wait for completion.
        // -----------------------------------------------------------------
        // SAFETY: `device` is a valid AshDevice; fence is created with default
        // (unsignaled) state; `None` means no custom allocator.
        let fence =
            unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|r| {
                // SAFETY: cleanup only on error; all handles are valid.
                unsafe { device.destroy_command_pool(cmd_pool, None) };
                if let Ok(mut guard) = alloc_handle.lock() {
                    guard.free(&mut staging_alloc);
                }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("create fence: {r:?}"),
                }
            })?;

        let cmd_buffers = [cmd_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_buffers);
        // SAFETY: `d.queue` is a valid VkQueue; command buffer is in completed
        // state; fence is valid and unsignaled; submit info is correctly
        // structured.
        unsafe { device.queue_submit(d.queue, &[submit_info], fence) }.map_err(|r| {
            // SAFETY: cleanup only on error; all handles are valid.
            unsafe { device.destroy_fence(fence, None) };
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() {
                guard.free(&mut staging_alloc);
            }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("queue submit: {r:?}"),
            }
        })?;

        // SAFETY: fence is valid and associated with the submitted work;
        // waiting with `u64::MAX` timeout and `true` (waitAll) is standard.
        unsafe { device.wait_for_fences(&[fence], true, u64::MAX) }.map_err(|r| {
            // SAFETY: cleanup only on error; all handles are valid.
            unsafe { device.destroy_fence(fence, None) };
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() {
                guard.free(&mut staging_alloc);
            }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("wait fence: {r:?}"),
            }
        })?;

        // SAFETY: fence has been waited on and is no longer needed; destroying
        // a signaled fence is safe.
        unsafe { device.destroy_fence(fence, None) };

        // -----------------------------------------------------------------
        // 5. Map staging buffer and copy pixel data to a Vec<u8>.
        // -----------------------------------------------------------------
        let raw_pixels = match staging_alloc.mapped_slice_mut() {
            Some(slice) => slice[..buffer_size as usize].to_vec(),
            None => {
                // SAFETY: cleanup only on error; all handles are valid.
                unsafe { device.destroy_command_pool(cmd_pool, None) };
                if let Ok(mut guard) = alloc_handle.lock() {
                    guard.free(&mut staging_alloc);
                }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                return Err(render_core::RhiError::Backend {
                    detail: "staging buffer is not CPU mapped".into(),
                });
            }
        };

        // -----------------------------------------------------------------
        // 6. Convert BGRA → RGBA if the swapchain uses a B8G8R8A8 format.
        //    The custom allocator's GpuToCpu allocations are host-mapped, so the
        //    raw data is available immediately after fence wait.
        // -----------------------------------------------------------------
        let result: Vec<u8> =
            if sc.format == vk::Format::B8G8R8A8_UNORM || sc.format == vk::Format::B8G8R8A8_SRGB {
                raw_pixels
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect()
            } else {
                raw_pixels
            };

        // -----------------------------------------------------------------
        // 7. Clean up temporary resources.
        // -----------------------------------------------------------------
        // SAFETY: all objects were created from this device and are no longer
        // in use after fence wait; reverse order of creation is respected.
        unsafe { device.destroy_command_pool(cmd_pool, None) };
        if let Ok(mut guard) = alloc_handle.lock() {
            guard.free(&mut staging_alloc);
        }
        unsafe { device.destroy_buffer(staging_buffer, None) };

        Ok(result)
    }
}
