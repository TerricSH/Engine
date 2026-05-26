//! Logical device + single graphics/present queue.

use std::cell::RefCell;
use std::rc::Rc;

use ash::khr::swapchain;
use ash::vk;
use ash::{Device as AshDevice, Instance as AshInstance};
use gpu_allocator::vulkan::{Allocator, AllocatorCreateDesc};

use crate::adapter::AdapterSelection;
use crate::error::{VkResult, VulkanError};

pub type SharedAllocator = Rc<RefCell<Allocator>>;

pub struct Device {
    pub device: AshDevice,
    pub queue: vk::Queue,
    pub queue_family_index: u32,
    allocator: Option<SharedAllocator>,
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

        let allocator = Allocator::new(&AllocatorCreateDesc {
            instance: instance.clone(),
            device: device.clone(),
            physical_device: adapter.physical_device,
            debug_settings: Default::default(),
            buffer_device_address: false,
            allocation_sizes: Default::default(),
        })
        .map_err(|err| VulkanError::Allocation(err.to_string()))?;

        Ok(Self {
            device,
            queue,
            queue_family_index: adapter.queue_family_index,
            allocator: Some(Rc::new(RefCell::new(allocator))),
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
