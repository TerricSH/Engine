//! `render_core::Device` trait implementation for VulkanDevice.
//!
//! This file implements the render-core abstraction layer, covering buffer
//! creation, render-pass / framebuffer / pipeline creation, frame lifecycle
//! (begin/end), and pixel readback.

use ash::vk;
use std::ops::Range;

use render_core::{
    self, AdapterInfo, BindGroupLayoutDescriptor, BufferDescriptor, BufferHandle, BufferUsage,
    CommandEncoder as CmdEncoderTrait, FramebufferDescriptor, FramebufferHandle,
    PipelineDescriptor, PipelineHandle, PipelineLayoutDescriptor, PipelineLayoutHandle,
    RenderPassDescriptor, RenderPassHandle, RendererStatistics, ShaderModuleDescriptor,
    ShaderModuleHandle, ShaderStage, SurfaceDescriptor, SurfaceHandle, SwapchainDescriptor,
    SwapchainHandle, TextureDescriptor, TextureFormat, TextureHandle, TextureUsage,
};

use crate::allocator::{AllocationCreateDesc, MemoryLocation};

use super::{
    blend_attachment_from_mode, compare_op, default_dep,
    encoder::VkCmdEncoder,
    mk_sm, parse_polygon_mode, parse_sample_count, parse_topology,
    resource_kind_to_descriptor_type,
    slab::{BufEntry, FbEntry, PipeEntry, PlEntry, TexEntry},
    vfmt, VulkanDevice,
};

fn fallback_pipeline_set_layouts(
    frame: Option<vk::DescriptorSetLayout>,
    shadow: Option<vk::DescriptorSetLayout>,
    material: Option<vk::DescriptorSetLayout>,
) -> Result<Vec<vk::DescriptorSetLayout>, render_core::RhiError> {
    let slots = [frame, shadow, material];
    let Some(last_used) = slots.iter().rposition(Option::is_some) else {
        return Ok(Vec::new());
    };

    slots
        .into_iter()
        .take(last_used + 1)
        .enumerate()
        .map(|(set_index, layout)| match layout {
            Some(layout) if layout != vk::DescriptorSetLayout::null() => Ok(layout),
            _ => Err(render_core::RhiError::InvalidDescriptor {
                field: "bind_group_layouts".into(),
                reason: format!(
                    "descriptor set layout {set_index} must be initialized before later sets"
                ),
            }),
        })
        .collect()
}

fn validate_contiguous_bind_group_layouts(
    layouts: &[BindGroupLayoutDescriptor],
) -> Result<(), render_core::RhiError> {
    let Some(max_set) = layouts.iter().map(|layout| layout.set_index).max() else {
        return Ok(());
    };
    let mut seen = vec![false; max_set as usize + 1];
    for layout in layouts {
        let slot = &mut seen[layout.set_index as usize];
        if *slot {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "bind_group_layouts".into(),
                reason: format!("duplicate descriptor set layout {}", layout.set_index),
            });
        }
        *slot = true;
    }
    if let Some(missing) = seen.iter().position(|present| !present) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "bind_group_layouts".into(),
            reason: format!("missing descriptor set layout {missing}"),
        });
    }
    Ok(())
}

fn ordered_bind_group_layouts(
    layouts: &[BindGroupLayoutDescriptor],
) -> Result<Vec<&BindGroupLayoutDescriptor>, render_core::RhiError> {
    validate_contiguous_bind_group_layouts(layouts)?;
    let mut ordered: Vec<_> = layouts.iter().collect();
    ordered.sort_by_key(|layout| layout.set_index);
    Ok(ordered)
}

fn vulkan_descriptor_bindings(
    layout: &BindGroupLayoutDescriptor,
) -> Result<Vec<vk::DescriptorSetLayoutBinding<'static>>, render_core::RhiError> {
    let mut binding_indices = std::collections::BTreeSet::new();
    layout
        .bindings
        .iter()
        .map(|binding| {
            if !binding_indices.insert(binding.binding) {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "bind_group_layouts.bindings".into(),
                    reason: format!(
                        "descriptor set {} contains duplicate binding {}",
                        layout.set_index, binding.binding
                    ),
                });
            }
            Ok(vk::DescriptorSetLayoutBinding::default()
                .binding(binding.binding)
                .descriptor_type(resource_kind_to_descriptor_type(&binding.resource_kind)?)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT))
        })
        .collect()
}

fn color_attachment_format(
    requested: Option<&TextureFormat>,
    swapchain_format: Option<vk::Format>,
) -> vk::Format {
    match requested {
        Some(TextureFormat::Bgra8Unorm) => swapchain_format.unwrap_or(vk::Format::B8G8R8A8_UNORM),
        Some(TextureFormat::Rgba8Unorm) => vk::Format::R8G8B8A8_UNORM,
        Some(TextureFormat::Rgba16Float) => vk::Format::R16G16B16A16_SFLOAT,
        _ => swapchain_format.unwrap_or(vk::Format::B8G8R8A8_UNORM),
    }
}

fn texture_format(format: TextureFormat) -> vk::Format {
    match format {
        TextureFormat::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
        TextureFormat::Bgra8Unorm => vk::Format::B8G8R8A8_UNORM,
        TextureFormat::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
        TextureFormat::Depth32Float => vk::Format::D32_SFLOAT,
        _ => vk::Format::UNDEFINED,
    }
}

fn checked_buffer_write_range(
    buffer_size: u64,
    offset: u64,
    data_len: usize,
) -> Result<Range<usize>, render_core::RhiError> {
    let start = usize::try_from(offset).map_err(|_| render_core::RhiError::InvalidDescriptor {
        field: "write_buffer.range".into(),
        reason: format!("offset {offset} cannot be represented on this platform"),
    })?;
    let logical_size =
        usize::try_from(buffer_size).map_err(|_| render_core::RhiError::Backend {
            detail: format!("buffer size {buffer_size} cannot be represented on this platform"),
        })?;
    let end =
        start
            .checked_add(data_len)
            .ok_or_else(|| render_core::RhiError::InvalidDescriptor {
                field: "write_buffer.range".into(),
                reason: format!("offset {offset} plus {data_len} bytes overflows address space"),
            })?;
    if end > logical_size {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "write_buffer.range".into(),
            reason: format!(
                "write range {offset}..{} exceeds buffer size {buffer_size}",
                offset.saturating_add(data_len as u64)
            ),
        });
    }
    Ok(start..end)
}

fn vertex_format_size(format: &str) -> Option<u32> {
    match format {
        "float32x2" => Some(8),
        "float32x3" => Some(12),
        "float32x4" | "uint32x4" => Some(16),
        _ => None,
    }
}

fn validate_graphics_pipeline_descriptor(
    desc: &PipelineDescriptor,
) -> Result<(), render_core::RhiError> {
    let sample_count = desc.sample_count.unwrap_or(1);
    if !matches!(sample_count, 1 | 2 | 4 | 8) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.sample_count".into(),
            reason: format!("unsupported sample count {sample_count}"),
        });
    }
    if !matches!(
        desc.topology.as_deref(),
        None | Some("point_list")
            | Some("line_list")
            | Some("line_strip")
            | Some("triangle_list")
            | Some("triangle_strip")
            | Some("triangle_fan")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.topology".into(),
            reason: format!("unsupported topology {:?}", desc.topology),
        });
    }
    if !matches!(
        desc.polygon_mode.as_deref(),
        None | Some("fill") | Some("line") | Some("point")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.polygon_mode".into(),
            reason: format!("unsupported polygon mode {:?}", desc.polygon_mode),
        });
    }
    if !matches!(
        desc.raster_state.cull_mode.as_deref(),
        None | Some("none") | Some("front") | Some("back")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.raster_state.cull_mode".into(),
            reason: format!("unsupported cull mode {:?}", desc.raster_state.cull_mode),
        });
    }
    if !matches!(
        desc.raster_state.front_face.as_deref(),
        None | Some("clockwise") | Some("cw") | Some("counter_clockwise") | Some("ccw")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.raster_state.front_face".into(),
            reason: format!("unsupported front face {:?}", desc.raster_state.front_face),
        });
    }
    if !matches!(
        desc.depth_state.compare.as_deref(),
        None | Some("less") | Some("equal") | Some("lequal") | Some("greater") | Some("always")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.depth_state.compare".into(),
            reason: format!(
                "unsupported depth comparison {:?}",
                desc.depth_state.compare
            ),
        });
    }
    if !matches!(
        desc.blend_state.mode.as_deref(),
        None | Some("Opaque") | Some("Alpha") | Some("Additive") | Some("Multiply")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.blend_state.mode".into(),
            reason: format!("unsupported blend mode {:?}", desc.blend_state.mode),
        });
    }
    if desc.render_targets.contains(&TextureFormat::Depth32Float) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.render_targets".into(),
            reason: "Depth32Float is not a color render target".into(),
        });
    }
    if let Some(format) = desc.depth_state.format {
        if format != TextureFormat::Depth32Float {
            return Err(render_core::RhiError::UnsupportedFeature {
                feature: format!("Vulkan pipeline depth format {format:?}"),
            });
        }
    }
    if !desc.vertex_layout.attributes.is_empty() && desc.vertex_layout.stride_bytes == 0 {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.vertex_layout.stride_bytes".into(),
            reason: "a non-empty vertex layout requires a non-zero stride".into(),
        });
    }
    for attribute in &desc.vertex_layout.attributes {
        let size = vertex_format_size(&attribute.format).ok_or_else(|| {
            render_core::RhiError::InvalidDescriptor {
                field: "pipeline.vertex_layout.attributes.format".into(),
                reason: format!("unsupported vertex format '{}'", attribute.format),
            }
        })?;
        if attribute
            .offset_bytes
            .checked_add(size)
            .is_none_or(|end| end > desc.vertex_layout.stride_bytes)
        {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.vertex_layout.attributes.offset_bytes".into(),
                reason: format!(
                    "attribute '{}' exceeds vertex stride {}",
                    attribute.semantic, desc.vertex_layout.stride_bytes
                ),
            });
        }
    }
    let mut specialization_ids = std::collections::BTreeSet::new();
    if let Some(duplicate) = desc
        .specialization
        .iter()
        .find(|constant| !specialization_ids.insert(constant.id))
    {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.specialization".into(),
            reason: format!("duplicate specialization constant id {}", duplicate.id),
        });
    }
    Ok(())
}

fn vulkan_specialization_data(
    constants: &[render_core::SpecConstant],
) -> (Vec<u8>, Vec<vk::SpecializationMapEntry>) {
    let mut data = Vec::with_capacity(constants.len() * std::mem::size_of::<u32>());
    let mut entries = Vec::with_capacity(constants.len());
    for constant in constants {
        let offset = data.len() as u32;
        let bytes = match constant.value {
            render_core::SpecValue::Bool(value) => u32::from(value).to_ne_bytes(),
            render_core::SpecValue::U32(value) => value.to_ne_bytes(),
            render_core::SpecValue::F32(value) => value.to_ne_bytes(),
        };
        data.extend_from_slice(&bytes);
        entries.push(vk::SpecializationMapEntry {
            constant_id: constant.id,
            offset,
            size: bytes.len(),
        });
    }
    (data, entries)
}

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
        self.ensure_sc()
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("create swapchain: {e}"),
            })?;
        Ok(SwapchainHandle::new(1, 1))
    }

    fn destroy_swapchain(&mut self, swapchain: SwapchainHandle) {
        if swapchain != SwapchainHandle::new(1, 1) || self.swapchain.is_none() {
            return;
        }
        self.wait_idle();
        self.destroy_swapchain_resources();
    }

    fn destroy_surface(&mut self, surface: SurfaceHandle) {
        if surface != SurfaceHandle::new(0, 1) || self.surface.is_none() {
            return;
        }
        self.wait_idle();
        self.destroy_swapchain_resources();
        drop(self.surface.take());
    }

    fn create_buffer(
        &mut self,
        desc: &BufferDescriptor,
    ) -> Result<BufferHandle, render_core::RhiError> {
        if desc.size_bytes == 0 {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "buffer.size_bytes".into(),
                reason: "buffer size must be non-zero".into(),
            });
        }
        let known_usage_mask = BufferUsage::VERTEX.0
            | BufferUsage::INDEX.0
            | BufferUsage::UNIFORM.0
            | BufferUsage::STORAGE.0
            | BufferUsage::COPY_SRC.0
            | BufferUsage::COPY_DST.0
            | BufferUsage::INDIRECT.0;
        if desc.usage_flags.0 == 0 || desc.usage_flags.0 & !known_usage_mask != 0 {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "buffer.usage_flags".into(),
                reason: "at least one recognized buffer usage flag is required".into(),
            });
        }
        let d = &self.logical_device;
        let size = desc.size_bytes;
        let mut usage = vk::BufferUsageFlags::empty();
        if desc.usage_flags.0 & BufferUsage::VERTEX.0 != 0 {
            usage |= vk::BufferUsageFlags::VERTEX_BUFFER;
        }
        if desc.usage_flags.0 & BufferUsage::INDEX.0 != 0 {
            usage |= vk::BufferUsageFlags::INDEX_BUFFER;
        }
        if desc.usage_flags.0 & BufferUsage::UNIFORM.0 != 0 {
            usage |= vk::BufferUsageFlags::UNIFORM_BUFFER;
        }
        if desc.usage_flags.0 & BufferUsage::STORAGE.0 != 0 {
            usage |= vk::BufferUsageFlags::STORAGE_BUFFER;
        }
        if desc.usage_flags.0 & BufferUsage::COPY_SRC.0 != 0 {
            usage |= vk::BufferUsageFlags::TRANSFER_SRC;
        }
        if desc.usage_flags.0 & BufferUsage::COPY_DST.0 != 0 {
            usage |= vk::BufferUsageFlags::TRANSFER_DST;
        }
        if desc.usage_flags.0 & BufferUsage::INDIRECT.0 != 0 {
            usage |= vk::BufferUsageFlags::INDIRECT_BUFFER;
        }
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
        let location = match desc.memory_hint {
            render_core::MemoryHint::GpuOnly => MemoryLocation::GpuOnly,
            render_core::MemoryHint::CpuToGpu => MemoryLocation::CpuToGpu,
            render_core::MemoryHint::GpuToCpu => MemoryLocation::GpuToCpu,
            render_core::MemoryHint::CpuOnly => MemoryLocation::CpuOnly,
        };
        let allocation_result = match alloc_handle.lock() {
            Ok(mut allocator) => allocator.allocate(&AllocationCreateDesc {
                name: "device-buffer",
                requirements: req,
                location,
            }),
            Err(error) => {
                // SAFETY: no allocation was made, so the buffer is unbound and
                // exclusively owned by this function.
                unsafe {
                    d.device.destroy_buffer(buffer, None);
                }
                return Err(render_core::RhiError::Backend {
                    detail: format!("allocator lock: {error}"),
                });
            }
        };
        let mut allocation = match allocation_result {
            Ok(allocation) => allocation,
            Err(error) => {
                // SAFETY: allocation failed, so the newly-created buffer is
                // unbound and has no other owner.
                unsafe {
                    d.device.destroy_buffer(buffer, None);
                }
                return Err(render_core::RhiError::Backend { detail: error });
            }
        };
        // SAFETY: `buffer` was created by this device; `allocation` was created
        // for this buffer's memory requirements; the memory and offset are valid.
        if let Err(r) = unsafe {
            d.device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
        } {
            match alloc_handle.lock() {
                Ok(mut alloc_guard) => alloc_guard.free(&mut allocation),
                Err(poisoned) => {
                    tracing::error!(
                        target: "vulkan::resources",
                        "allocator mutex was poisoned after buffer bind failure"
                    );
                    poisoned.into_inner().free(&mut allocation);
                }
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
            size,
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
        let write_range = checked_buffer_write_range(entry.size, offset, data.len())?;
        let alloc = entry
            .allocation
            .as_mut()
            .ok_or_else(|| render_core::RhiError::Backend {
                detail: "no alloc".into(),
            })?;
        let slice =
            alloc
                .mapped_slice_mut()
                .ok_or_else(|| render_core::RhiError::UnsupportedFeature {
                    feature: "write_buffer requires host-visible buffer memory".into(),
                })?;
        if write_range.end > slice.len() {
            return Err(render_core::RhiError::Backend {
                detail: format!(
                    "mapped allocation is {} bytes but buffer write requires {} bytes",
                    slice.len(),
                    write_range.end
                ),
            });
        }
        slice[write_range].copy_from_slice(data);
        Ok(())
    }

    fn destroy_buffer(&mut self, buffer: BufferHandle) {
        let Some(entry) = self.buffers.remove(buffer.index, buffer.generation) else {
            tracing::warn!(
                target: "vulkan::resources",
                index = buffer.index,
                generation = buffer.generation,
                "ignored destroy request for an invalid or already-destroyed buffer handle"
            );
            return;
        };
        entry.destroy(&self.logical_device.device);
    }

    fn create_texture(
        &mut self,
        desc: &TextureDescriptor,
    ) -> Result<TextureHandle, render_core::RhiError> {
        if desc.width == 0
            || desc.height == 0
            || desc.depth_or_layers == 0
            || desc.mip_levels == 0
            || desc.sample_count == 0
        {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "texture".into(),
                reason: "dimensions, layers, mip levels and sample count must be non-zero".into(),
            });
        }
        if !matches!(desc.sample_count, 1 | 2 | 4 | 8) {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "texture.sample_count".into(),
                reason: format!("unsupported sample count {}", desc.sample_count),
            });
        }
        if desc.sample_count > 1 && desc.mip_levels > 1 {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "texture.mip_levels".into(),
                reason: "multisampled textures must have exactly one mip level".into(),
            });
        }
        let mut usage = vk::ImageUsageFlags::empty();
        if desc.usage_flags.0 & TextureUsage::SAMPLED.0 != 0 {
            usage |= vk::ImageUsageFlags::SAMPLED;
        }
        if desc.usage_flags.0 & TextureUsage::COLOR_ATTACHMENT.0 != 0 {
            usage |= vk::ImageUsageFlags::COLOR_ATTACHMENT;
        }
        if desc.usage_flags.0 & TextureUsage::DEPTH_ATTACHMENT.0 != 0 {
            usage |= vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT;
        }
        if desc.usage_flags.0 & TextureUsage::COPY_SRC.0 != 0 {
            usage |= vk::ImageUsageFlags::TRANSFER_SRC;
        }
        if desc.usage_flags.0 & TextureUsage::COPY_DST.0 != 0 {
            usage |= vk::ImageUsageFlags::TRANSFER_DST;
        }
        if usage.is_empty() {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "texture.usage_flags".into(),
                reason: "at least one texture usage flag is required".into(),
            });
        }
        let is_depth = desc.format == TextureFormat::Depth32Float;
        if is_depth && desc.usage_flags.0 & TextureUsage::COLOR_ATTACHMENT.0 != 0 {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "texture.usage_flags".into(),
                reason: "Depth32Float cannot be a color attachment".into(),
            });
        }
        if !is_depth && desc.usage_flags.0 & TextureUsage::DEPTH_ATTACHMENT.0 != 0 {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "texture.usage_flags".into(),
                reason: "only Depth32Float can be a depth attachment".into(),
            });
        }
        let d = &self.logical_device.device;
        let format = texture_format(desc.format);
        if format == vk::Format::UNDEFINED {
            return Err(render_core::RhiError::UnsupportedFeature {
                feature: format!("Vulkan texture format {:?}", desc.format),
            });
        }
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: desc.width,
                height: desc.height,
                depth: 1,
            })
            .mip_levels(desc.mip_levels)
            .array_layers(desc.depth_or_layers)
            .samples(parse_sample_count(Some(desc.sample_count)))
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let image = unsafe { d.create_image(&image_info, None) }.map_err(|result| {
            render_core::RhiError::Backend {
                detail: format!("create texture image: {result:?}"),
            }
        })?;
        let requirements = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let allocation_result = match allocator.lock() {
            Ok(mut guard) => guard.allocate(&AllocationCreateDesc {
                name: "rhi-texture",
                requirements,
                location: MemoryLocation::GpuOnly,
            }),
            Err(error) => {
                unsafe { d.destroy_image(image, None) };
                return Err(render_core::RhiError::Backend {
                    detail: format!("texture allocator lock: {error}"),
                });
            }
        };
        let mut allocation = match allocation_result {
            Ok(allocation) => allocation,
            Err(error) => {
                unsafe { d.destroy_image(image, None) };
                return Err(render_core::RhiError::Backend { detail: error });
            }
        };
        if let Err(result) =
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            unsafe { d.destroy_image(image, None) };
            match allocator.lock() {
                Ok(mut guard) => guard.free(&mut allocation),
                Err(poisoned) => poisoned.into_inner().free(&mut allocation),
            }
            return Err(render_core::RhiError::Backend {
                detail: format!("bind texture image: {result:?}"),
            });
        }
        let aspect = if is_depth {
            vk::ImageAspectFlags::DEPTH
        } else {
            vk::ImageAspectFlags::COLOR
        };
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(if desc.depth_or_layers == 1 {
                vk::ImageViewType::TYPE_2D
            } else {
                vk::ImageViewType::TYPE_2D_ARRAY
            })
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: aspect,
                base_mip_level: 0,
                level_count: desc.mip_levels,
                base_array_layer: 0,
                layer_count: desc.depth_or_layers,
            });
        let view = match unsafe { d.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                unsafe { d.destroy_image(image, None) };
                match allocator.lock() {
                    Ok(mut guard) => guard.free(&mut allocation),
                    Err(poisoned) => poisoned.into_inner().free(&mut allocation),
                }
                return Err(render_core::RhiError::Backend {
                    detail: format!("create texture view: {result:?}"),
                });
            }
        };
        let sampler = if desc.usage_flags.0 & TextureUsage::SAMPLED.0 != 0 {
            match unsafe {
                d.create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .max_lod(desc.mip_levels.saturating_sub(1) as f32),
                    None,
                )
            } {
                Ok(sampler) => Some(sampler),
                Err(result) => {
                    unsafe {
                        d.destroy_image_view(view, None);
                        d.destroy_image(image, None);
                    }
                    match allocator.lock() {
                        Ok(mut guard) => guard.free(&mut allocation),
                        Err(poisoned) => poisoned.into_inner().free(&mut allocation),
                    }
                    return Err(render_core::RhiError::Backend {
                        detail: format!("create texture sampler: {result:?}"),
                    });
                }
            }
        } else {
            None
        };
        let (index, generation) = self.rhi_textures.insert(TexEntry {
            image,
            view,
            sampler,
            format,
            width: desc.width,
            height: desc.height,
            sample_count: desc.sample_count,
            allocator,
            allocation: Some(allocation),
        });
        Ok(TextureHandle::new(index, generation))
    }

    fn destroy_texture(&mut self, texture: TextureHandle) {
        let Some(entry) = self.rhi_textures.remove(texture.index, texture.generation) else {
            return;
        };
        entry.destroy(&self.logical_device.device);
    }

    fn create_shader_module(
        &mut self,
        desc: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, render_core::RhiError> {
        if desc.format != render_core::ShaderFormat::SpirV || desc.source_bytes.is_empty() {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "shader_module".into(),
                reason: "Vulkan requires non-empty SPIR-V source bytes".into(),
            });
        }
        if !desc.source_bytes.len().is_multiple_of(4) {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "shader_module.source_bytes".into(),
                reason: "SPIR-V byte length must be divisible by four".into(),
            });
        }
        if desc.source_bytes.len() < 4
            || u32::from_le_bytes(desc.source_bytes[0..4].try_into().expect("four-byte slice"))
                != 0x0723_0203
        {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "shader_module.source_bytes".into(),
                reason: "shader source does not start with the SPIR-V magic number".into(),
            });
        }
        if desc.entry_points.as_slice() != ["main"] {
            return Err(render_core::RhiError::UnsupportedFeature {
                feature: "Vulkan shader modules currently require one 'main' entry point".into(),
            });
        }
        let d = &self.logical_device.device;
        let sm = (unsafe { mk_sm(d, &desc.source_bytes) }).map_err(|e| {
            render_core::RhiError::Backend {
                detail: format!("create_shader_module: {e}"),
            }
        })?;
        let stage = match desc.stage {
            ShaderStage::Vertex => vk::ShaderStageFlags::VERTEX,
            ShaderStage::Fragment => vk::ShaderStageFlags::FRAGMENT,
            ShaderStage::Compute => vk::ShaderStageFlags::COMPUTE,
        };
        let (idx, gen) = self.shader_modules.insert((sm, stage));
        Ok(ShaderModuleHandle::new(idx, gen))
    }

    fn destroy_shader_module(&mut self, module: ShaderModuleHandle) {
        if let Some((shader, _)) = self.shader_modules.remove(module.index, module.generation) {
            unsafe {
                self.logical_device
                    .device
                    .destroy_shader_module(shader, None);
            }
        }
    }

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
            (
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
                unsafe {
                    self.logical_device
                        .device
                        .destroy_descriptor_set_layout(set_layout, None);
                }
            }
            unsafe {
                self.logical_device
                    .device
                    .destroy_pipeline_layout(layout.layout, None);
            }
        }
    }

    fn create_pipeline(
        &mut self,
        desc: &PipelineDescriptor,
    ) -> Result<PipelineHandle, render_core::RhiError> {
        validate_graphics_pipeline_descriptor(desc)?;
        let d = &self.logical_device.device;
        let main = c"main";

        if desc.shader_modules.is_empty() {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "shader_modules".into(),
                reason: "a graphics pipeline requires explicit shader module handles".into(),
            });
        }
        if desc.render_pass.is_none() {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "render_pass".into(),
                reason: "a graphics pipeline requires an explicit render pass".into(),
            });
        }
        if desc.pipeline_layout.is_none() {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline_layout".into(),
                reason: "a graphics pipeline requires an explicit pipeline layout".into(),
            });
        }

        // ── Shader stages ──────────────────────────────────────────────
        // If the descriptor provides shader module handles, resolve them
        // from the shader_modules slab.  Otherwise fall back to the
        // embedded vertex/fragment SPIR-V.  Skinned pipelines use the
        // dedicated skinned vertex shader (detected by the presence of a
        // uint32x4 vertex attribute, which indicates joints).
        let render_pass_handle = desc.render_pass.expect("render pass validated above");
        let rp = self
            .render_passes
            .get(render_pass_handle.index, render_pass_handle.generation)
            .copied()
            .ok_or(render_core::RhiError::InvalidHandle)?;
        let expected_colors = self
            .rp_color_formats
            .get(&render_pass_handle.index)
            .ok_or(render_core::RhiError::InvalidHandle)?;
        let requested_colors: Vec<_> = desc
            .render_targets
            .iter()
            .copied()
            .map(texture_format)
            .collect();
        if requested_colors != *expected_colors {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.render_targets".into(),
                reason: "pipeline color formats do not match the render pass".into(),
            });
        }
        let expected_samples = self
            .rp_sample_counts
            .get(&render_pass_handle.index)
            .copied()
            .ok_or(render_core::RhiError::InvalidHandle)?;
        if desc.sample_count.unwrap_or(1) != expected_samples {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.sample_count".into(),
                reason: format!(
                    "pipeline sample count {} does not match render pass sample count {expected_samples}",
                    desc.sample_count.unwrap_or(1)
                ),
            });
        }
        let render_pass_depth = self
            .rp_depth_formats
            .get(&render_pass_handle.index)
            .copied();
        let requested_depth = desc.depth_state.format.map(texture_format);
        if requested_depth.is_some() && requested_depth != render_pass_depth {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.depth_state.format".into(),
                reason: "pipeline depth format does not match the render pass".into(),
            });
        }
        if (desc.depth_state.write_enabled || desc.depth_state.compare.is_some())
            && render_pass_depth.is_none()
        {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.depth_state".into(),
                reason: "depth testing requires a render pass depth attachment".into(),
            });
        }
        let pipeline_layout_handle = desc
            .pipeline_layout
            .expect("pipeline layout validated above");
        let pll = self
            .pipeline_layouts
            .get(
                pipeline_layout_handle.index,
                pipeline_layout_handle.generation,
            )
            .map(|entry| entry.layout)
            .ok_or(render_core::RhiError::InvalidHandle)?;

        let (specialization_data, specialization_entries) =
            vulkan_specialization_data(&desc.specialization);
        let specialization_info = vk::SpecializationInfo::default()
            .map_entries(&specialization_entries)
            .data(&specialization_data);
        let has_specialization = !specialization_entries.is_empty();
        let mut has_vertex = false;
        let mut has_fragment = false;
        let mut sr = Vec::with_capacity(desc.shader_modules.len());
        for handle in &desc.shader_modules {
            let (shader, stage) = self
                .shader_modules
                .get(handle.index, handle.generation)
                .copied()
                .ok_or(render_core::RhiError::InvalidHandle)?;
            if stage == vk::ShaderStageFlags::VERTEX {
                if has_vertex {
                    return Err(render_core::RhiError::InvalidDescriptor {
                        field: "pipeline.shader_modules".into(),
                        reason: "graphics shader stages must not be duplicated".into(),
                    });
                }
                has_vertex = true;
            } else if stage == vk::ShaderStageFlags::FRAGMENT {
                if has_fragment {
                    return Err(render_core::RhiError::InvalidDescriptor {
                        field: "pipeline.shader_modules".into(),
                        reason: "graphics shader stages must not be duplicated".into(),
                    });
                }
                has_fragment = true;
            } else {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "pipeline.shader_modules".into(),
                    reason: "graphics pipelines accept only vertex and fragment shaders".into(),
                });
            }
            let mut stage_info = vk::PipelineShaderStageCreateInfo::default()
                .stage(stage)
                .module(shader)
                .name(main);
            if has_specialization {
                stage_info = stage_info.specialization_info(&specialization_info);
            }
            sr.push(stage_info);
        }
        if !has_vertex || (!desc.render_targets.is_empty() && !has_fragment) {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.shader_modules".into(),
                reason: "a vertex shader and, for color output, a fragment shader are required"
                    .into(),
            });
        }

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
            Some("clockwise" | "cw") => vk::FrontFace::CLOCKWISE,
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
        let cba = vec![blend_attachment; desc.render_targets.len()];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);

        // ── Dynamic state ──────────────────────────────────────────────
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);

        // ── Render pass ────────────────────────────────────────────────
        // If the descriptor carries a handle, resolve it; otherwise create
        // an inline render pass from the descriptor's render targets.
        // ── Pipeline layout ────────────────────────────────────────────
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
            framebuffer_cache: self
                .framebuffers
                .slots
                .iter()
                .map(|slot| {
                    slot.as_ref().map(|(generation, entry)| {
                        (
                            *generation,
                            entry.framebuffer,
                            entry.color_attachment_count,
                            entry.has_depth,
                        )
                    })
                })
                .collect(),
            pipeline_layout_cache: self
                .pipeline_layouts
                .slots
                .iter()
                .map(|s| s.as_ref().map(|(g, e)| (*g, e.layout)))
                .collect(),
            current_desc_set: desc_set,
            render_pass_active: false,
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
            unsafe {
                let _ = self.logical_device.device.device_wait_idle();
            };
            // Keep the swapchain and every framebuffer that references its
            // image views alive until SceneRenderer starts the next frame. It
            // can then destroy its own framebuffers before device-owned HDR/UI
            // resources and the swapchain are torn down in dependency order.
            self.swapchain_recreate_pending = true;
        }
        self.current_frame = (fi + 1) % 2;
        if let Some(instance) = self.instance.as_ref() {
            let validation_errors = instance.validation_error_count();
            if validation_errors > 0 {
                return Err(render_core::RhiError::Backend {
                    detail: format!("Vulkan validation reported {validation_errors} error(s)"),
                });
            }
        }
        Ok(RendererStatistics {
            // Pass implementations own draw accounting. The device lifecycle
            // itself records no scene draw and must not fabricate one.
            draw_calls: 0,
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
        unsafe {
            let _ = self.logical_device.device.device_wait_idle();
        };
        self.window_width = w.max(1);
        self.window_height = h.max(1);
        self.swapchain = None;
        Ok(())
    }

    fn wait_idle(&self) {
        // SAFETY: `self.logical_device` is alive by type invariant
        // (ManuallyDrop ensures destruction order).
        unsafe {
            let _ = self.logical_device.device.device_wait_idle();
        };
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
        unsafe {
            let _ = self.logical_device.device.device_wait_idle();
        };

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

        // Use the last image acquired by the canonical frame lifecycle.
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

#[cfg(test)]
mod tests {
    use ash::vk::Handle;

    use super::*;

    fn layout(raw: u64) -> vk::DescriptorSetLayout {
        vk::DescriptorSetLayout::from_raw(raw)
    }

    #[test]
    fn fallback_pipeline_layouts_require_a_contiguous_prefix() {
        let err = fallback_pipeline_set_layouts(None, None, Some(layout(3)))
            .expect_err("set 2 without sets 0 and 1 must be rejected");
        assert!(matches!(
            err,
            render_core::RhiError::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn fallback_pipeline_layouts_preserve_initialized_set_order() {
        let layouts =
            fallback_pipeline_set_layouts(Some(layout(1)), Some(layout(2)), Some(layout(3)))
                .expect("contiguous layouts should be accepted");
        assert_eq!(layouts, vec![layout(1), layout(2), layout(3)]);
        assert!(layouts
            .iter()
            .all(|layout| *layout != vk::DescriptorSetLayout::null()));
    }

    #[test]
    fn explicit_pipeline_layouts_reject_set_index_gaps() {
        let layouts = [BindGroupLayoutDescriptor {
            set_index: 1,
            bindings: Vec::new(),
        }];
        let err = validate_contiguous_bind_group_layouts(&layouts)
            .expect_err("set 1 without set 0 must be rejected");
        assert!(matches!(
            err,
            render_core::RhiError::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn explicit_pipeline_layouts_are_ordered_by_set_index() {
        let layouts = [
            BindGroupLayoutDescriptor {
                set_index: 1,
                bindings: Vec::new(),
            },
            BindGroupLayoutDescriptor {
                set_index: 0,
                bindings: Vec::new(),
            },
        ];
        let ordered = ordered_bind_group_layouts(&layouts)
            .expect("contiguous layouts should be sorted by set index");
        assert_eq!(ordered[0].set_index, 0);
        assert_eq!(ordered[1].set_index, 1);
    }

    #[test]
    fn descriptor_bindings_reject_duplicates_and_unknown_resource_kinds() {
        let duplicate = BindGroupLayoutDescriptor {
            set_index: 0,
            bindings: vec![
                render_core::BindGroupLayoutBinding {
                    binding: 1,
                    resource_kind: "uniform_buffer".into(),
                },
                render_core::BindGroupLayoutBinding {
                    binding: 1,
                    resource_kind: "sampler".into(),
                },
            ],
        };
        assert!(vulkan_descriptor_bindings(&duplicate).is_err());

        let unknown = BindGroupLayoutDescriptor {
            set_index: 0,
            bindings: vec![render_core::BindGroupLayoutBinding {
                binding: 0,
                resource_kind: "mystery_resource".into(),
            }],
        };
        assert!(vulkan_descriptor_bindings(&unknown).is_err());
        assert_eq!(
            resource_kind_to_descriptor_type("sampler").unwrap(),
            vk::DescriptorType::SAMPLER
        );
    }

    #[test]
    fn graphics_pipeline_validation_rejects_silent_fallback_inputs() {
        let descriptor = PipelineDescriptor {
            topology: Some("unknown".into()),
            ..PipelineDescriptor::default()
        };
        assert!(validate_graphics_pipeline_descriptor(&descriptor).is_err());

        let descriptor = PipelineDescriptor {
            vertex_layout: render_core::VertexLayout {
                stride_bytes: 8,
                attributes: vec![render_core::VertexAttribute {
                    semantic: "position".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                }],
            },
            ..PipelineDescriptor::default()
        };
        assert!(validate_graphics_pipeline_descriptor(&descriptor).is_err());
    }

    #[test]
    fn specialization_data_uses_four_byte_vulkan_scalars() {
        let constants = [
            render_core::SpecConstant {
                id: 4,
                value: render_core::SpecValue::Bool(true),
            },
            render_core::SpecConstant {
                id: 9,
                value: render_core::SpecValue::F32(2.5),
            },
        ];
        let (data, entries) = vulkan_specialization_data(&constants);
        assert_eq!(data.len(), 8);
        assert_eq!(&data[..4], &1u32.to_ne_bytes());
        assert_eq!(&data[4..], &2.5f32.to_ne_bytes());
        assert_eq!(entries[0].constant_id, 4);
        assert_eq!(entries[0].size, 4);
        assert_eq!(entries[1].offset, 4);
    }

    #[test]
    fn bgra_present_targets_use_the_actual_swapchain_format() {
        assert_eq!(
            color_attachment_format(
                Some(&TextureFormat::Bgra8Unorm),
                Some(vk::Format::B8G8R8A8_SRGB),
            ),
            vk::Format::B8G8R8A8_SRGB
        );
    }

    #[test]
    fn buffer_write_range_accepts_exact_end_and_empty_end_write() {
        assert_eq!(checked_buffer_write_range(16, 4, 12).unwrap(), 4..16);
        assert_eq!(checked_buffer_write_range(16, 16, 0).unwrap(), 16..16);
    }

    #[test]
    fn buffer_write_range_rejects_out_of_bounds_without_truncation() {
        let error = checked_buffer_write_range(16, 15, 2).unwrap_err();
        assert!(matches!(
            error,
            render_core::RhiError::InvalidDescriptor { .. }
        ));
        assert!(checked_buffer_write_range(16, 17, 0).is_err());
        assert!(checked_buffer_write_range(u64::MAX, u64::MAX, 1).is_err());
    }
}
