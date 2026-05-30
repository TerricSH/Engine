//! Environment cubemap for IBL (Phase 2.3).
//!
//! Creates a 1×1 placeholder cubemap (6 faces, R8G8B8A8_UNORM) filled with
//! solid gray (0.03, 0.03, 0.03) and updates the shadow descriptor set
//! binding=1 so that the forward fragment shader can sample it for diffuse
//! IBL (irradiance) and specular IBL (prefiltered environment map).

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::VulkanDevice;

impl VulkanDevice {
    /// Create a 1×1 environment cubemap with solid gray data (placeholder).
    ///
    /// Idempotent: returns `Ok(())` if the cubemap already exists. Must be
    /// called after [`ensure_shadow`] so that the descriptor set (set=1)
    /// is available for binding=1 to be updated.
    pub(crate) fn create_env_cubemap(&mut self) -> VkResult<()> {
        if self.env_cubemap.is_some() {
            return Ok(());
        }
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();

        // ---- 1. Create cubemap image (1×1, 6 layers, CUBE_COMPATIBLE) ----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width: 1,
                height: 1,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(6)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .flags(vk::ImageCreateFlags::CUBE_COMPATIBLE);
        // SAFETY: `d` is a valid AshDevice; `image_info` describes a valid
        // 2D array image with CUBE_COMPATIBLE flag.
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|r| VulkanError::vk("create_env_image", r))?;

        // SAFETY: `image` was just created by this device.
        let req = unsafe { d.get_image_memory_requirements(image) };
        let allocation = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "env-cubemap",
                requirements: req,
                location: crate::allocator::MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: `image` was created by this device; `allocation` was created
        // for this image's memory requirements.
        unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_env_image", r))?;

        // ---- 2. Image view (CUBE, all 6 layers) ----
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::CUBE)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 6,
            });
        // SAFETY: `d` is a valid AshDevice; `view_info` references a valid
        // image with CUBE view type; `None` means no custom allocator.
        let image_view = unsafe { d.create_image_view(&view_info, None) }
            .map_err(|r| VulkanError::vk("create_env_image_view", r))?;

        // ---- 3. Sampler (linear, clamp-to-edge) ----
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
            .map_err(|r| VulkanError::vk("create_env_sampler", r))?;

        // ---- 4. Upload placeholder gray data via staging buffer ----
        self.upload_env_placeholder(image)?;

        // ---- 5. Update descriptor set binding=1 ----
        self.update_env_descriptor_set();

        // ---- Store ----
        self.env_cubemap = Some(image);
        self.env_cubemap_view = Some(image_view);
        self.env_cubemap_allocation = Some(allocation);
        self.env_sampler = Some(sampler);

        Ok(())
    }

    /// Upload solid gray (0.03, 0.03, 0.03) to each cubemap face via a
    /// staging buffer and a one-shot command buffer submission.
    fn upload_env_placeholder(&mut self, image: vk::Image) -> VkResult<()> {
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();

        // Gray pixel in R8G8B8A8_UNORM: 0.03 × 255 ≈ 7.65 → 8
        const GRAY_PIXEL: [u8; 4] = [8, 8, 8, 255];
        const FACE_COUNT: u32 = 6;
        const PIXEL_SIZE: u32 = 4; // R8G8B8A8
        const TOTAL_BYTES: u64 = (FACE_COUNT * PIXEL_SIZE) as u64;

        // ---- Staging buffer (CpuToGpu, TRANSFER_SRC) ----
        let buf_info = vk::BufferCreateInfo::default()
            .size(TOTAL_BYTES)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is a valid AshDevice; `buf_info` describes a valid
        // buffer; `None` means no custom allocator.
        let staging_buf = unsafe { d.create_buffer(&buf_info, None) }
            .map_err(|r| VulkanError::vk("create_env_staging_buf", r))?;
        // SAFETY: `staging_buf` was just created by this device.
        let req = unsafe { d.get_buffer_memory_requirements(staging_buf) };
        let mut staging_alloc = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "env-staging",
                requirements: req,
                location: crate::allocator::MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: `staging_buf` and `staging_alloc` are compatible.
        unsafe {
            d.bind_buffer_memory(staging_buf, staging_alloc.memory(), staging_alloc.offset())
        }
        .map_err(|r| VulkanError::vk("bind_env_staging_buf", r))?;

        // Write one gray pixel per face into the mapped staging buffer
        if let Some(slice) = staging_alloc.mapped_slice_mut() {
            let step = PIXEL_SIZE as usize;
            for i in 0..FACE_COUNT as usize {
                slice[i * step..(i + 1) * step].copy_from_slice(&GRAY_PIXEL);
            }
        }

        // ---- Temporary command pool + buffer for transfer ----
        let qfi = self.adapter.queue_family_index;
        let cp_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(qfi)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        // SAFETY: `d` is a valid AshDevice; `cp_info` describes a valid pool;
        // `None` means no custom allocator.
        let cmd_pool = unsafe { d.create_command_pool(&cp_info, None) }
            .map_err(|r| VulkanError::vk("create_env_cmd_pool", r))?;

        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(cmd_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: `d` is a valid AshDevice; `cmd_pool` is valid; `alloc_info`
        // is correctly structured.
        let cmd_bufs = unsafe { d.allocate_command_buffers(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_env_cmd_buf", r))?;
        let cmd = cmd_bufs[0];

        // Begin one-shot command buffer
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: `cmd` is a valid command buffer in initial state.
        unsafe { d.begin_command_buffer(cmd, &begin_info) }
            .map_err(|r| VulkanError::vk("begin_env_cmd_buf", r))?;

        // Barrier 1: UNDEFINED → TRANSFER_DST_OPTIMAL
        let barrier_undef_to_transfer = vk::ImageMemoryBarrier::default()
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: FACE_COUNT,
            })
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL);
        // SAFETY: `cmd` is in recording state; image, barrier, and stage masks
        // are valid.
        unsafe {
            d.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_undef_to_transfer],
            );
        }

        // Copy staging buffer → cubemap, one region per face layer
        let buffer_copy_regions: Vec<vk::BufferImageCopy> = (0..FACE_COUNT)
            .map(|layer| {
                vk::BufferImageCopy::default()
                    .buffer_offset((layer * PIXEL_SIZE) as u64)
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: layer,
                        layer_count: 1,
                    })
                    .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                    .image_extent(vk::Extent3D {
                        width: 1,
                        height: 1,
                        depth: 1,
                    })
            })
            .collect();
        // SAFETY: `cmd` is in recording state; `staging_buf` and `image` are
        // valid; copy regions are within bounds for both buffer and image.
        unsafe {
            d.cmd_copy_buffer_to_image(
                cmd,
                staging_buf,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &buffer_copy_regions,
            );
        }

        // Barrier 2: TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
        let barrier_transfer_to_read = vk::ImageMemoryBarrier::default()
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: FACE_COUNT,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        // SAFETY: `cmd` is in recording state; image, barrier, and stage masks
        // are valid.
        unsafe {
            d.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_transfer_to_read],
            );
        }

        // End command buffer
        // SAFETY: `cmd` is in recording state.
        unsafe { d.end_command_buffer(cmd) }
            .map_err(|r| VulkanError::vk("end_env_cmd_buf", r))?;

        // Submit with a temporary fence
        let fence_info = vk::FenceCreateInfo::default();
        // SAFETY: `d` is a valid AshDevice; `fence_info` describes a default
        // fence; `None` means no custom allocator.
        let fence = unsafe { d.create_fence(&fence_info, None) }
            .map_err(|r| VulkanError::vk("create_env_fence", r))?;

        let cmd_bufs = [cmd];
        let submit_info = [vk::SubmitInfo::default().command_buffers(&cmd_bufs)];
        // SAFETY: `d` is a valid AshDevice; `self.logical_device.queue` is a
        // valid queue; `submit_info` and `fence` are valid.
        unsafe {
            d.queue_submit(self.logical_device.queue, &submit_info, fence)
        }
        .map_err(|r| VulkanError::vk("submit_env_upload", r))?;

        // Wait for completion
        // SAFETY: `d` is a valid AshDevice; `fence` was just signalled by the
        // submit above; `true` = wait-all; `u64::MAX` = infinite timeout.
        unsafe { d.wait_for_fences(&[fence], true, u64::MAX) }
            .map_err(|r| VulkanError::vk("wait_env_fence", r))?;

        // ---- Cleanup temporary resources ----
        // SAFETY: All handles were created by this device; the fence has
        // completed, so no in-flight work references them.
        unsafe {
            d.destroy_fence(fence, None);
            d.destroy_command_pool(cmd_pool, None);
            d.destroy_buffer(staging_buf, None);
        }
        // Free staging allocation
        if let Ok(mut guard) = allocator.lock() {
            let mut a = staging_alloc;
            guard.free(&mut a);
        }

        Ok(())
    }

    /// Destroy all environment cubemap resources (reverse order of creation).
    pub(crate) fn destroy_env_resources(&mut self) {
        let d = &self.logical_device.device;

        if let Some(s) = self.env_sampler.take() {
            // SAFETY: `s` was created by this device and is still alive.
            unsafe { d.destroy_sampler(s, None); }
        }
        if let Some(iv) = self.env_cubemap_view.take() {
            // SAFETY: `iv` was created by this device and is still alive.
            unsafe { d.destroy_image_view(iv, None); }
        }
        if let Some(img) = self.env_cubemap.take() {
            // SAFETY: `img` was created by this device and is still alive.
            unsafe { d.destroy_image(img, None); }
        }
        if let Some(mut a) = self.env_cubemap_allocation.take() {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                guard.free(&mut a);
            }
        }
    }

    /// Write the environment cubemap (image view + sampler) into the shadow
    /// descriptor set at binding=1.
    ///
    /// Silently returns if the descriptor set or cubemap resources are not
    /// yet created, so it is safe to call at any point after
    /// [`create_shadow_resources`].
    fn update_env_descriptor_set(&self) {
        let Some(ds) = self.shadow_desc_set else { return };
        let Some(sampler) = self.env_sampler else { return };
        let Some(image_view) = self.env_cubemap_view else { return };
        let d = &self.logical_device.device;

        let image_info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(image_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(ds)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&image_info)];
        // SAFETY: `d` is a valid AshDevice; descriptor set, sampler, and image
        // view are valid; binding=1 exists in the descriptor set layout.
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }
    }
}
