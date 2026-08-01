//! Depth texture management for VulkanDevice.

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::VulkanDevice;

impl VulkanDevice {
    pub(crate) fn create_depth_texture(&mut self) -> VkResult<()> {
        let d = &self.logical_device.device;
        let extent = self
            .swapchain
            .as_ref()
            .map(|s| s.extent)
            .unwrap_or(self.swapchain_extent);
        if extent.width == 0 || extent.height == 0 {
            return Ok(());
        }

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
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is live and `image_info` describes a supported 2D depth
        // attachment whose dimensions were taken from the current swapchain.
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|r| VulkanError::vk("create_depth_image", r))?;
        // SAFETY: `image` was just created by `d` and has not been destroyed.
        let req = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let allocation = allocator
            .lock()
            .map_err(|error| VulkanError::Loader(format!("depth allocator lock: {error}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "depth-buffer",
                requirements: req,
                location: crate::allocator::MemoryLocation::GpuOnly,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: the allocation satisfies `image`'s queried requirements and
        // both the image and memory belong to this logical device.
        unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_depth_image", r))?;

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
        // SAFETY: `image` is a live D32 image and `view_info` selects its only
        // mip/layer using the matching depth aspect and format.
        let image_view = unsafe { d.create_image_view(&view_info, None) }
            .map_err(|r| VulkanError::vk("create_depth_image_view", r))?;

        self.depth_image = Some(image);
        self.depth_image_view = Some(image_view);
        self.depth_allocation = Some(allocation);
        Ok(())
    }

    pub(crate) fn destroy_depth_texture(&mut self) {
        let d = &self.logical_device.device;
        if let Some(iv) = self.depth_image_view.take() {
            // SAFETY: this device created and exclusively owns `iv`; callers
            // synchronize resize/drop before destroying depth resources.
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(img) = self.depth_image.take() {
            // SAFETY: the dependent view was destroyed above, `img` belongs to
            // this device, and resize/drop synchronization prevents GPU use.
            unsafe {
                d.destroy_image(img, None);
            }
        }
        if let Some(mut a) = self.depth_allocation.take() {
            let allocator = self.logical_device.allocator();
            match allocator.lock() {
                Ok(mut guard) => guard.free(&mut a),
                Err(poisoned) => {
                    tracing::error!(
                        target: "vulkan::depth",
                        "allocator mutex was poisoned while freeing the depth allocation"
                    );
                    poisoned.into_inner().free(&mut a);
                }
            };
        }
    }
}
