//! Per-frame command pools, command buffers, semaphores, fences, and
//! secondary command buffer support for multi-threaded recording.
#![allow(dead_code)]

use std::sync::Mutex;

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
    /// Thread-safe secondary command buffer pool.
    secondary_pool: Mutex<SecondaryPool>,
}

/// A thread-safe pool for recording secondary command buffers.
struct SecondaryPool {
    pool: vk::CommandPool,
    /// Free list of available secondary buffers (index, generation).
    free: Vec<(u32, u32)>,
    /// All allocated handles, `None` = slot free.
    slots: Vec<Option<(u32, vk::CommandBuffer)>>,
    device: AshDevice,
}

impl SecondaryPool {
    unsafe fn new(device: &AshDevice, queue_family_index: u32) -> VkResult<Self> {
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let pool = unsafe { device.create_command_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("secondary_cmd_pool", r))?;
        Ok(Self { pool, free: Vec::new(), slots: Vec::new(), device: device.clone() })
    }

    /// Allocate or recycle a secondary command buffer.
    fn alloc(&mut self) -> VkResult<(u32, u32, vk::CommandBuffer)> {
        if let Some((idx, gen)) = self.free.pop() {
            let cb = self.slots[idx as usize].as_ref().map(|s| s.1).unwrap();
            return Ok((idx, gen, cb));
        }
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.pool)
            .level(vk::CommandBufferLevel::SECONDARY)
            .command_buffer_count(1);
        let cbs = unsafe { self.device.allocate_command_buffers(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_secondary", r))?;
        let idx = self.slots.len() as u32;
        self.slots.push(Some((1, cbs[0])));
        Ok((idx, 1, cbs[0]))
    }

    /// Return a secondary buffer to the free list.
    fn free(&mut self, index: u32, generation: u32) {
        self.free.push((index, generation));
        // Increment generation so stale handles are rejected.
        if let Some(Some(ref mut slot)) = self.slots.get_mut(index as usize) {
            slot.0 = generation.wrapping_add(1);
        }
    }
}

impl Drop for SecondaryPool {
    fn drop(&mut self) {
        unsafe { self.device.destroy_command_pool(self.pool, None); }
    }
}

/// A handle to a secondary command buffer that can be recorded on any thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecondaryBufferHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
    pub(crate) vk_buffer: vk::CommandBuffer,
}

impl SecondaryBufferHandle {
    /// Begin recording into this secondary buffer.
    ///
    /// # Safety
    /// Must be called from a single thread at a time for this buffer.
    pub unsafe fn begin(&self, device: &AshDevice, inherit: &vk::CommandBufferInheritanceInfo) -> VkResult<()> {
        let info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
            .inheritance_info(inherit);
        unsafe { device.begin_command_buffer(self.vk_buffer, &info) }
            .map_err(|r| VulkanError::vk("begin_secondary", r))
    }

    /// End recording.
    ///
    /// # Safety
    /// Buffer must be in recording state.
    pub unsafe fn end(&self, device: &AshDevice) -> VkResult<()> {
        unsafe { device.end_command_buffer(self.vk_buffer) }
            .map_err(|r| VulkanError::vk("end_secondary", r))
    }
}

impl FrameContext {
    /// # Safety
    /// `device` and `queue_family_index` must come from the owning [`Device`](crate::device::Device).
    pub unsafe fn new(device: AshDevice, queue_family_index: u32) -> VkResult<Self> {
        let mut frames = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for _ in 0..FRAMES_IN_FLIGHT {
            let frame = unsafe { create_frame(&device, queue_family_index) }?;
            frames.push(frame);
        }
        let secondary_pool = unsafe { SecondaryPool::new(&device, queue_family_index) }?;
        Ok(Self { frames, current: 0, device, secondary_pool: Mutex::new(secondary_pool) })
    }

    pub fn _current_frame(&self) -> &Frame {
        &self.frames[self.current]
    }

    pub fn advance(&mut self) {
        self.current = (self.current + 1) % FRAMES_IN_FLIGHT;
    }

    /// Allocate a secondary command buffer for thread-safe recording.
    pub fn allocate_secondary(&self) -> VkResult<SecondaryBufferHandle> {
        let mut pool = self.secondary_pool.lock().unwrap();
        let (index, generation, vk_buffer) = pool.alloc()?;
        Ok(SecondaryBufferHandle { index, generation, vk_buffer })
    }

    /// Return a secondary buffer to the pool after execution.
    pub fn free_secondary(&self, handle: SecondaryBufferHandle) {
        let mut pool = self.secondary_pool.lock().unwrap();
        pool.free(handle.index, handle.generation);
    }

    /// The device handle.
    pub fn device(&self) -> &AshDevice {
        &self.device
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
    let image_available = unsafe { device.create_semaphore(&sem_info, None) }
        .map_err(|r| VulkanError::vk("create_semaphore(image_available)", r))?;
    let render_finished = unsafe { device.create_semaphore(&sem_info, None) }
        .map_err(|r| VulkanError::vk("create_semaphore(render_finished)", r))?;

    let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
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
        // Secondary pool is dropped via its own Drop.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_handle_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SecondaryBufferHandle>();
    }

    #[test]
    fn secondary_handle_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<SecondaryBufferHandle>();
    }
}
