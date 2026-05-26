//! Per-frame command pools, command buffers, semaphores, and fences for
//! the standard frames-in-flight pattern.

use ash::vk;
use ash::Device as AshDevice;

use crate::error::{VkResult, VulkanError};

pub const FRAMES_IN_FLIGHT: usize = 2;

pub struct Frame {
    pub command_pool: vk::CommandPool,
    pub command_buffer: vk::CommandBuffer,
    pub image_available: vk::Semaphore,
    pub render_finished: vk::Semaphore,
    pub in_flight: vk::Fence,
}

pub struct FrameContext {
    pub frames: Vec<Frame>,
    pub current: usize,
    device: AshDevice,
}

impl FrameContext {
    /// # Safety
    /// `device` and `queue_family_index` must come from the owning [`Device`](crate::device::Device).
    pub unsafe fn new(device: AshDevice, queue_family_index: u32) -> VkResult<Self> {
        let mut frames = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for _ in 0..FRAMES_IN_FLIGHT {
            // SAFETY: device + family index valid.
            let frame = unsafe { create_frame(&device, queue_family_index) }?;
            frames.push(frame);
        }
        Ok(Self {
            frames,
            current: 0,
            device,
        })
    }

    #[allow(dead_code)]
    pub fn current_frame(&self) -> &Frame {
        &self.frames[self.current]
    }

    pub fn advance(&mut self) {
        self.current = (self.current + 1) % FRAMES_IN_FLIGHT;
    }
}

unsafe fn create_frame(device: &AshDevice, queue_family_index: u32) -> VkResult<Frame> {
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    // SAFETY: pool_info outlives this call.
    let command_pool = unsafe { device.create_command_pool(&pool_info, None) }
        .map_err(|r| VulkanError::vk("create_command_pool", r))?;

    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    // SAFETY: alloc_info outlives this call.
    let command_buffer = unsafe { device.allocate_command_buffers(&alloc_info) }
        .map_err(|r| VulkanError::vk("allocate_command_buffers", r))?[0];

    let sem_info = vk::SemaphoreCreateInfo::default();
    // SAFETY: sem_info outlives this call.
    let image_available = unsafe { device.create_semaphore(&sem_info, None) }
        .map_err(|r| VulkanError::vk("create_semaphore(image_available)", r))?;
    // SAFETY: sem_info outlives this call.
    let render_finished = unsafe { device.create_semaphore(&sem_info, None) }
        .map_err(|r| VulkanError::vk("create_semaphore(render_finished)", r))?;

    let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
    // SAFETY: fence_info outlives this call.
    let in_flight = unsafe { device.create_fence(&fence_info, None) }
        .map_err(|r| VulkanError::vk("create_fence(in_flight)", r))?;

    Ok(Frame {
        command_pool,
        command_buffer,
        image_available,
        render_finished,
        in_flight,
    })
}

impl Drop for FrameContext {
    fn drop(&mut self) {
        // SAFETY: caller already issued device_wait_idle before dropping.
        unsafe {
            for frame in &self.frames {
                self.device.destroy_fence(frame.in_flight, None);
                self.device.destroy_semaphore(frame.image_available, None);
                self.device.destroy_semaphore(frame.render_finished, None);
                self.device.destroy_command_pool(frame.command_pool, None);
            }
        }
    }
}
