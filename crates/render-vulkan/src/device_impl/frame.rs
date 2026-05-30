//! Frame lifecycle management for VulkanDevice.
//!
//! Handles swapchain lazy initialization, image acquisition, command-buffer
//! lifecycle, submission, presentation, and frame-sync object creation.

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{slab::FrameSync, VulkanDevice};

impl VulkanDevice {
    /// Ensure a swapchain exists (lazily create one if absent).
    pub(crate) fn ensure_sc(&mut self) -> VkResult<()> {
        if self.swapchain.is_none() {
            let instance = self
                .instance
                .as_ref()
                .ok_or(VulkanError::Loader("instance not initialized".into()))?;
            let surface = self
                .surface
                .as_ref()
                .ok_or(VulkanError::Loader("surface not initialized".into()))?;
            // SAFETY: all handles (instance, device, physical device, surface)
            // are valid; `Swapchain::new` takes ownership of the cloned device.
            match unsafe {
                crate::swapchain::Swapchain::new(
                    &instance.instance,
                    self.logical_device.device.clone(),
                    self.adapter.physical_device,
                    self.logical_device.queue_family_index,
                    &surface.loader,
                    surface.surface,
                    self.window_width,
                    self.window_height,
                )
            } {
                Ok(sc) => {
                    self.swapchain_extent = sc.extent;
                    self.swapchain = Some(sc);
                    // Create depth texture matching swapchain
                    self.create_depth_texture()?;
                    // Create descriptor set infrastructure
                    self.create_descriptor_infra()?;
                    // Create material descriptor infrastructure (set=2)
                    self.create_material_descriptor_infra()?;
                    // Create shadow mapping resources
                    self.ensure_shadow()?;
                    // Create HDR offscreen + tone-mapping resources
                    self.ensure_hdr_resources()?;
                }
                Err(VulkanError::SurfaceMinimized) => {
                    self.minimized = true;
                    return Err(VulkanError::SurfaceMinimized);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Acquire the next swapchain image.
    pub(crate) fn acquire(&mut self, fi: usize) -> VkResult<(u32, bool)> {
        let frame_sync = &self.frame_sync[fi];
        let in_flight_fence = frame_sync.in_flight_fence;
        let image_available = frame_sync.image_available;
        // SAFETY: `f.in_flight_fence` is a valid fence created by this device;
        // waiting with `u64::MAX` timeout is safe.
        unsafe {
            self.logical_device
                .device
                .wait_for_fences(&[in_flight_fence], true, u64::MAX)
                .map_err(|r| VulkanError::vk("wf", r))?;
        }
        self.drain_retired_pipelines(fi);
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        // SAFETY: `sc.loader` is a valid swapchain loader; `sc.swapchain` is a
        // valid VkSwapchainKHR; `f.image_available` is a valid semaphore;
        // timeout parameters are standard Vulkan.
        let (ii, sub) = unsafe {
            sc.loader
                .acquire_next_image(sc.swapchain, u64::MAX, image_available, vk::Fence::null())
        }
        .map_err(|r| {
            if r == vk::Result::ERROR_OUT_OF_DATE_KHR {
                VulkanError::SwapchainOutOfDate
            } else {
                VulkanError::vk("aq", r)
            }
        })?;
        // SAFETY: `f.in_flight_fence` has been signaled (wait completed above);
        // resetting a signaled fence is valid.
        unsafe {
            self.logical_device
                .device
                .reset_fences(&[in_flight_fence])
                .map_err(|r| VulkanError::vk("rf", r))?;
        }
        Ok((ii, sub))
    }

    /// Reset and begin a command buffer for the given in-flight frame.
    pub(crate) fn begin_cb(&self, fi: usize) -> VkResult<()> {
        let f = &self.frame_sync[fi];
        // SAFETY: `f.command_buffer` is a valid command buffer allocated from a
        // pool with `RESET_COMMAND_BUFFER` flag; `f.command_pool` owns it.
        unsafe {
            self.logical_device
                .device
                .reset_command_buffer(f.command_buffer, vk::CommandBufferResetFlags::empty())
                .map_err(|r| VulkanError::vk("rcb", r))?;
            // SAFETY: after reset the command buffer is in the initial state;
            // `begin_command_buffer` transitions it to recording state.
            self.logical_device
                .device
                .begin_command_buffer(
                    f.command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(|r| VulkanError::vk("bcb", r))?;
        }
        Ok(())
    }

    /// End the command buffer, submit to the graphics queue, and present.
    pub(crate) fn submit_and_present(&self, fi: usize, ii: u32) -> VkResult<bool> {
        let d = &self.logical_device.device;
        let f = &self.frame_sync[fi];
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        // SAFETY: command buffer is in recording state; `end_command_buffer`
        // transitions it to completed state for submission.
        unsafe {
            d.end_command_buffer(f.command_buffer)
                .map_err(|r| VulkanError::vk("ecb", r))?;
        }
        let ws = [f.image_available];
        let wst = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let cbs = [f.command_buffer];
        let ss = [f.render_finished];
        let si = vk::SubmitInfo::default()
            .wait_semaphores(&ws)
            .wait_dst_stage_mask(&wst)
            .command_buffers(&cbs)
            .signal_semaphores(&ss);
        // SAFETY: `queue` is a valid VkQueue; command buffer is in completed
        // state; semaphores and fence are valid; submit info is correctly
        // structured.
        unsafe {
            d.queue_submit(self.logical_device.queue, &[si], f.in_flight_fence)
                .map_err(|r| VulkanError::vk("qs", r))?;
        }
        let sca = [sc.swapchain];
        let ia = [ii];
        // SAFETY: `queue` is valid; swapchain, semaphores, and image indices
        // are valid; `PresentInfoKHR` is correctly structured.
        match unsafe {
            sc.loader.queue_present(
                self.logical_device.queue,
                &vk::PresentInfoKHR::default()
                    .wait_semaphores(&ss)
                    .swapchains(&sca)
                    .image_indices(&ia),
            )
        } {
            Ok(false) => Ok(false),
            Ok(true) => Ok(true),
            Err(r) if r == vk::Result::ERROR_OUT_OF_DATE_KHR || r == vk::Result::SUBOPTIMAL_KHR => {
                Ok(true)
            }
            Err(r) => Err(VulkanError::vk("qp", r)),
        }
    }

    /// Create frame-sync objects (fences, semaphores, command pools/buffers) for
    /// double-buffering.
    pub(crate) fn build_frames(&mut self) -> VkResult<()> {
        let d = &self.logical_device.device;
        for _ in 0..2 {
            // SAFETY: `d` is a valid AshDevice; the queue family index is valid
            // for this device; `None` means no custom allocator.
            let cp = unsafe {
                d.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(self.logical_device.queue_family_index)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
            }
            .map_err(|r| VulkanError::vk("ccp", r))?;
            // SAFETY: `cp` was just created and is valid; allocation info
            // correctly references the pool with PRIMARY level.
            let cbs = unsafe {
                d.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(cp)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
            }
            .map_err(|r| VulkanError::vk("acb", r))?;
            let si = vk::SemaphoreCreateInfo::default();
            // SAFETY: `d` is a valid AshDevice; `si` describes a valid
            // semaphore; `None` means no custom allocator.
            let ia =
                unsafe { d.create_semaphore(&si, None) }.map_err(|r| VulkanError::vk("cs", r))?;
            // SAFETY: same as above for the render-finished semaphore.
            let rf =
                unsafe { d.create_semaphore(&si, None) }.map_err(|r| VulkanError::vk("cs", r))?;
            // SAFETY: `d` is a valid AshDevice; fence is created in SIGNALED
            // state; `None` means no custom allocator.
            let fl = unsafe {
                d.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
            }
            .map_err(|r| VulkanError::vk("cf", r))?;
            self.frame_sync.push(FrameSync {
                image_available: ia,
                render_finished: rf,
                in_flight_fence: fl,
                command_pool: cp,
                command_buffer: cbs[0],
            });
        }
        Ok(())
    }
}
