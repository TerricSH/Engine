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
                    // SAFETY: the image is unbound, unused, and exclusively
                    // owned after allocator locking failed.
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
                    // SAFETY: allocation failed before binding/submission, so
                    // the device-created image remains exclusively owned here.
                    unsafe { d.destroy_image(image, None) };
                    return Err(VulkanError::Allocation(error.to_string()));
                }
            }
        };
        // SAFETY: `image` was created by this device; `allocation` was created
        // for this image's memory requirements.
        if let Err(result) =
            // SAFETY: same contract as above; this is adjacent to the unsafe
            // conditional expression.
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            // SAFETY: binding failed before GPU use; destroy the unbound image
            // before releasing the associated allocation.
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
                // SAFETY: no view was created and the bound image has not been
                // submitted; destroy it before freeing bound memory.
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
                // SAFETY: sampler creation failed before exposure/submission;
                // the device-created view and image are exclusively owned here.
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
        // SAFETY: `d` is live and this complete create-info describes a
        // single-sample RGBA16F attachment matching the current extent.
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|result| VulkanError::vk("create_oit_image", result))?;
        // SAFETY: the image was just created by `d` and remains live.
        let requirements = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let mut allocation =
            allocate_hdr_target_memory(d, &allocator, image, allocation_name, requirements)?;
        // SAFETY: the allocation satisfies the queried requirements and belongs
        // to the same device; no use begins until binding succeeds.
        if let Err(result) =
            // SAFETY: same contract as above; this is adjacent to the unsafe
            // conditional expression.
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            // SAFETY: binding failed before GPU use, leaving the image
            // exclusively owned and safe to destroy before memory release.
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
        // SAFETY: the bound RGBA16F image is live and the view selects its only
        // color mip/layer using a matching format.
        let image_view = match unsafe { d.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                // SAFETY: view creation failed before submission; destroy the
                // exclusively-owned image before freeing its memory.
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
        // SAFETY: `d` is live and `image_info` describes a valid multisampled
        // RGBA16F color attachment at the current extent/sample count.
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|result| VulkanError::vk("create_hdr_msaa_image", result))?;
        // SAFETY: `image` was just created by `d` and remains live.
        let requirements = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let mut allocation =
            allocate_hdr_target_memory(d, &allocator, image, allocation_name, requirements)?;
        // SAFETY: the device allocation meets the queried image requirements;
        // no command references either handle before binding succeeds.
        if let Err(result) =
            // SAFETY: same contract as above; this is adjacent to the unsafe
            // conditional expression.
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            // SAFETY: failed binding leaves the image unused and exclusively
            // owned; destroy it before the allocation is released.
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
        // SAFETY: the live multisample image and view have matching format,
        // color aspect, and a single valid mip/layer range.
        let image_view = match unsafe { d.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                // SAFETY: the view was not created and no GPU work was
                // submitted; destroy the image before freeing its memory.
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
        // SAFETY: `d` is live and `image_info` describes a valid multisampled
        // D32 depth attachment matching the HDR extent/sample count.
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|result| VulkanError::vk("create_hdr_msaa_depth_image", result))?;
        // SAFETY: the depth image was just created by `d` and remains live.
        let requirements = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let mut allocation =
            allocate_hdr_target_memory(d, &allocator, image, "hdr-msaa-depth", requirements)?;
        // SAFETY: allocation was chosen from this image's requirements, both
        // handles belong to `d`, and no command can use the image before binding.
        if let Err(result) =
            // SAFETY: same contract as above; this is adjacent to the unsafe
            // conditional expression.
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
        {
            // SAFETY: binding failed before GPU use; destroy the exclusively
            // owned image before freeing its allocation.
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
        // SAFETY: the live D32 image and view use matching depth aspect/format
        // and select the only valid mip/layer.
        let image_view = match unsafe { d.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                // SAFETY: view creation failed before submission; destroy the
                // image before releasing its bound allocation.
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
            // SAFETY: locking failed before allocation/binding; `image` is an
            // unused device-created handle exclusively transferred to this helper.
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
            // SAFETY: allocation failed before binding or GPU submission; the
            // helper still exclusively owns the unbound image.
            unsafe { device.destroy_image(image, None) };
            Err(VulkanError::Allocation(error.to_string()))
        }
    }
}
