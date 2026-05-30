//! Logical device + single graphics/present queue + optional compute queue.

use std::sync::{Arc, Mutex};

use crate::adapter::AdapterSelection;
use crate::allocator::{SharedAllocator, VulkanAllocator};
use crate::error::{VkResult, VulkanError};
use ash::khr::swapchain;
use ash::vk;
use ash::{Device as AshDevice, Instance as AshInstance};

pub struct Device {
    pub device: AshDevice,
    pub queue: vk::Queue,
    pub queue_family_index: u32,
    /// Optional dedicated compute queue (may share family with graphics).
    pub compute_queue: Option<vk::Queue>,
    /// Queue family index of the compute queue (same as graphics if no
    /// dedicated compute family was found).
    pub compute_queue_family_index: u32,
    pub(crate) allocator: Option<SharedAllocator>,
}

impl Device {
    /// # Safety
    /// `instance` and `adapter.physical_device` must be valid.
    pub unsafe fn new(instance: &AshInstance, adapter: &AdapterSelection) -> VkResult<Self> {
        let priorities = [1.0_f32];

        // ── Queue creation infos ────────────────────────────────────────
        // Always create the graphics queue.  If the compute queue family is
        // different from the graphics family, request a second queue.
        let gfx_qfi = adapter.queue_family_index;
        let compute_qfi = adapter.compute_queue_family_index.unwrap_or(gfx_qfi);

        let gfx_queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(gfx_qfi)
            .queue_priorities(&priorities);

        let queue_infos = if compute_qfi != gfx_qfi {
            let compute_queue_info = vk::DeviceQueueCreateInfo::default()
                .queue_family_index(compute_qfi)
                .queue_priorities(&priorities);
            vec![gfx_queue_info, compute_queue_info]
        } else {
            vec![gfx_queue_info]
        };

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
        let queue = unsafe { device.get_device_queue(gfx_qfi, 0) };
        let compute_queue = if compute_qfi != gfx_qfi {
            // SAFETY: the compute queue family index is valid and a queue was
            // requested during device creation.
            Some(unsafe { device.get_device_queue(compute_qfi, 0) })
        } else {
            // No dedicated compute family; share the graphics queue.
            Some(queue)
        };

        tracing::info!(
            target: "vulkan",
            queue_family = gfx_qfi,
            compute_queue_family = compute_qfi,
            dedicated_compute = compute_qfi != gfx_qfi,
            "logical device created"
        );

        let mem_props = instance.get_physical_device_memory_properties(adapter.physical_device);
        // SAFETY: device is valid; memory_properties came from the physical device.
        let allocator = unsafe { VulkanAllocator::new(device.clone(), mem_props) };

        Ok(Self {
            device,
            queue,
            queue_family_index: gfx_qfi,
            compute_queue,
            compute_queue_family_index: compute_qfi,
            allocator: Some(Arc::new(Mutex::new(allocator))),
        })
    }

    pub fn allocator(&self) -> SharedAllocator {
        self.allocator
            .as_ref()
            .expect("allocator is alive until Device::drop")
            .clone()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        drop(self.allocator.take());
        // SAFETY: pipeline/swapchain/frames are dropped before Device per
        // VulkanRenderer field order.
        unsafe { self.device.destroy_device(None) }
    }
}
