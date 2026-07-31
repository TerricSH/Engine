use super::*;

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
        let mut allocation = {
            let mut allocator_guard = match allocator.lock() {
                Ok(guard) => guard,
                Err(error) => {
                    // The image has not reached `self` yet, so this function
                    // owns the rollback for every failure after creation.
                    unsafe { d.destroy_image(image, None) };
                    return Err(VulkanError::Loader(format!("allocator lock: {error}")));
                }
            };
            match allocator_guard.allocate(&crate::allocator::AllocationCreateDesc {
                name: "hdr-color",
                requirements: req,
                location: crate::allocator::MemoryLocation::GpuOnly,
            }) {
                Ok(allocation) => allocation,
                Err(error) => {
                    unsafe { d.destroy_image(image, None) };
                    return Err(VulkanError::Allocation(error.to_string()));
                }
            }
        };
        // SAFETY: `image` was created by this device; `allocation` was created
        // for this image's memory requirements.
        if let Err(result) =
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            unsafe { d.destroy_image(image, None) };
            free_hdr_target_allocation(&allocator, &mut allocation);
            return Err(VulkanError::vk("bind_hdr_image", result));
        }

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
        let image_view = match unsafe { d.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                unsafe { d.destroy_image(image, None) };
                free_hdr_target_allocation(&allocator, &mut allocation);
                return Err(VulkanError::vk("create_hdr_image_view", result));
            }
        };

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
        let sampler = match unsafe { d.create_sampler(&sampler_info, None) } {
            Ok(sampler) => sampler,
            Err(result) => {
                unsafe {
                    d.destroy_image_view(image_view, None);
                    d.destroy_image(image, None);
                }
                free_hdr_target_allocation(&allocator, &mut allocation);
                return Err(VulkanError::vk("create_hdr_sampler", result));
            }
        };

        self.hdr_color_image = Some(image);
        self.hdr_color_view = Some(image_view);
        self.hdr_color_allocation = Some(allocation);
        self.hdr_color_sampler = Some(sampler);

        Ok(())
    }

    pub(super) fn create_oit_color_target(
        &self,
        allocation_name: &'static str,
    ) -> VkResult<(vk::Image, vk::ImageView, crate::allocator::Allocation)> {
        let d = &self.logical_device.device;
        let extent = self.swapchain_extent;
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
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|result| VulkanError::vk("create_oit_image", result))?;
        let requirements = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let mut allocation =
            allocate_hdr_target_memory(d, &allocator, image, allocation_name, requirements)?;
        if let Err(result) =
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            unsafe { d.destroy_image(image, None) };
            free_hdr_target_allocation(&allocator, &mut allocation);
            return Err(VulkanError::vk("bind_oit_image", result));
        }
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
        let image_view = match unsafe { d.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                unsafe { d.destroy_image(image, None) };
                free_hdr_target_allocation(&allocator, &mut allocation);
                return Err(VulkanError::vk("create_oit_image_view", result));
            }
        };
        Ok((image, image_view, allocation))
    }

    pub(super) fn create_msaa_color_target(
        &self,
        allocation_name: &'static str,
    ) -> VkResult<(vk::Image, vk::ImageView, crate::allocator::Allocation)> {
        let d = &self.logical_device.device;
        let extent = self.swapchain_extent;
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
            .samples(self.hdr_msaa_samples)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|result| VulkanError::vk("create_hdr_msaa_image", result))?;
        let requirements = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let mut allocation =
            allocate_hdr_target_memory(d, &allocator, image, allocation_name, requirements)?;
        if let Err(result) =
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            unsafe { d.destroy_image(image, None) };
            free_hdr_target_allocation(&allocator, &mut allocation);
            return Err(VulkanError::vk("bind_hdr_msaa_image", result));
        }
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
        let image_view = match unsafe { d.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                unsafe { d.destroy_image(image, None) };
                free_hdr_target_allocation(&allocator, &mut allocation);
                return Err(VulkanError::vk("create_hdr_msaa_image_view", result));
            }
        };
        Ok((image, image_view, allocation))
    }

    pub(super) fn create_hdr_msaa_depth_target(
        &self,
    ) -> VkResult<(vk::Image, vk::ImageView, crate::allocator::Allocation)> {
        let d = &self.logical_device.device;
        let extent = self.swapchain_extent;
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(self.hdr_msaa_samples)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|result| VulkanError::vk("create_hdr_msaa_depth_image", result))?;
        let requirements = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let mut allocation =
            allocate_hdr_target_memory(d, &allocator, image, "hdr-msaa-depth", requirements)?;
        if let Err(result) =
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            unsafe { d.destroy_image(image, None) };
            free_hdr_target_allocation(&allocator, &mut allocation);
            return Err(VulkanError::vk("bind_hdr_msaa_depth_image", result));
        }
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let image_view = match unsafe { d.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                unsafe { d.destroy_image(image, None) };
                free_hdr_target_allocation(&allocator, &mut allocation);
                return Err(VulkanError::vk("create_hdr_msaa_depth_view", result));
            }
        };
        Ok((image, image_view, allocation))
    }
}

pub(super) fn free_hdr_target_allocation(
    allocator: &crate::allocator::SharedAllocator,
    allocation: &mut crate::allocator::Allocation,
) {
    match allocator.lock() {
        Ok(mut guard) => guard.free(allocation),
        // A poisoned mutex still contains the allocator.  Recovery here is
        // required to keep the Vulkan memory lifetime symmetric with the
        // image lifetime on an error path.
        Err(poisoned) => poisoned.into_inner().free(allocation),
    }
}

fn allocate_hdr_target_memory(
    device: &ash::Device,
    allocator: &crate::allocator::SharedAllocator,
    image: vk::Image,
    allocation_name: &'static str,
    requirements: vk::MemoryRequirements,
) -> VkResult<crate::allocator::Allocation> {
    let mut allocator_guard = match allocator.lock() {
        Ok(guard) => guard,
        Err(error) => {
            unsafe { device.destroy_image(image, None) };
            return Err(VulkanError::Loader(format!("allocator lock: {error}")));
        }
    };
    match allocator_guard.allocate(&crate::allocator::AllocationCreateDesc {
        name: allocation_name,
        requirements,
        location: crate::allocator::MemoryLocation::GpuOnly,
    }) {
        Ok(allocation) => Ok(allocation),
        Err(error) => {
            unsafe { device.destroy_image(image, None) };
            Err(VulkanError::Allocation(error.to_string()))
        }
    }
}
