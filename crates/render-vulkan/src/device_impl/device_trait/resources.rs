macro_rules! vulkan_device_resource_methods {
    () => {
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
        // SAFETY: `d` is live and the validated RHI descriptor was translated
        // into a complete image create-info with supported dimensions/usage.
        let image = unsafe { d.create_image(&image_info, None) }.map_err(|result| {
            render_core::RhiError::Backend {
                detail: format!("create texture image: {result:?}"),
            }
        })?;
        // SAFETY: `image` was just created by `d` and remains live.
        let requirements = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let allocation_result = match allocator.lock() {
            Ok(mut guard) => guard.allocate(&AllocationCreateDesc {
                name: "rhi-texture",
                requirements,
                location: MemoryLocation::GpuOnly,
            }),
            Err(error) => {
                // SAFETY: allocator locking failed before memory was bound;
                // `image` is unused and exclusively owned by this function.
                unsafe { d.destroy_image(image, None) };
                return Err(render_core::RhiError::Backend {
                    detail: format!("texture allocator lock: {error}"),
                });
            }
        };
        let mut allocation = match allocation_result {
            Ok(allocation) => allocation,
            Err(error) => {
                // SAFETY: allocation failed, leaving the newly-created image
                // unbound and exclusively owned here.
                unsafe { d.destroy_image(image, None) };
                return Err(render_core::RhiError::Backend { detail: error });
            }
        };
        // SAFETY: `allocation` was selected from the queried requirements;
        // image and memory belong to `d` and no GPU use has begun.
        if let Err(result) =
            // SAFETY: same contract as above; this annotation is adjacent to
            // the unsafe conditional expression.
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            // SAFETY: binding failed and no GPU command can reference `image`;
            // destroy it before freeing the still-owned allocation.
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
        // SAFETY: the bound image is live; format/view type/aspect and
        // mip/layer range were derived from its validated descriptor.
        let view = match unsafe { d.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                // SAFETY: view creation failed, so the image has no dependent
                // view and has never been submitted; it is exclusively owned.
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
            // SAFETY: `d` is live and the sampler description contains valid
            // finite Vulkan enum/range values without external pointers.
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
                    // SAFETY: both handles were created by `d`, remain
                    // exclusively owned here, and no command uses them yet.
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
        // SAFETY: `d` is live and `mk_sm` validates the supplied byte length;
        // the shader bytes remain borrowed for the duration of the call.
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
            // SAFETY: slab removal gives exclusive ownership; pipelines retain
            // compiled code rather than the module, so this live handle can die.
            unsafe {
                self.logical_device
                    .device
                    .destroy_shader_module(shader, None);
            }
        }
    }
    };
}

pub(super) use vulkan_device_resource_methods;
