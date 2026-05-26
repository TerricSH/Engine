//! Logical device + single graphics/present queue.

use ash::khr::swapchain;
use ash::vk;
use ash::{Device as AshDevice, Instance as AshInstance};

use crate::adapter::AdapterSelection;
use crate::error::{VkResult, VulkanError};

pub struct Device {
    pub device: AshDevice,
    pub queue: vk::Queue,
    pub queue_family_index: u32,
}

impl Device {
    /// # Safety
    /// `instance` and `adapter.physical_device` must be valid.
    pub unsafe fn new(instance: &AshInstance, adapter: &AdapterSelection) -> VkResult<Self> {
        let queue_priorities = [1.0_f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(adapter.queue_family_index)
            .queue_priorities(&queue_priorities);
        let queue_infos = [queue_info];

        let device_extensions = [swapchain::NAME.as_ptr()];
        let features = vk::PhysicalDeviceFeatures::default();
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&device_extensions)
            .enabled_features(&features);

        // SAFETY: all referenced slices outlive this call.
        let device = unsafe { instance.create_device(adapter.physical_device, &device_info, None) }
            .map_err(|r| VulkanError::vk("create_device", r))?;
        // SAFETY: device + queue family index are valid.
        let queue = unsafe { device.get_device_queue(adapter.queue_family_index, 0) };

        tracing::info!(
            target: "vulkan",
            queue_family = adapter.queue_family_index,
            "logical device created"
        );

        Ok(Self {
            device,
            queue,
            queue_family_index: adapter.queue_family_index,
        })
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: pipeline/swapchain/frames are dropped before Device per
        // VulkanRenderer field order.
        unsafe { self.device.destroy_device(None) }
    }
}
