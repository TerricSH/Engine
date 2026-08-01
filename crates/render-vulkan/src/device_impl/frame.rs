//! Frame lifecycle management for VulkanDevice.
//!
//! Handles swapchain lazy initialization, image acquisition, command-buffer
//! lifecycle, submission, presentation, and frame-sync object creation.

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{slab::FrameSync, VulkanDevice};

impl VulkanDevice {
    /// Wait for all submitted device work and surface a Vulkan error instead
    /// of discarding it. Resource replacement and frame cancellation use this
    /// before destroying objects shared by both in-flight frame slots.
    pub(crate) fn wait_idle_checked(&self) -> VkResult<()> {
        // SAFETY: the logical device remains alive for `self`.
        unsafe { self.logical_device.device.device_wait_idle() }
            .map_err(|result| VulkanError::vk("device_wait_idle", result))
    }

    /// Cancel the currently recording, unsubmitted frame and repair its sync
    /// objects. `acquire` resets the frame fence and signals the image-available
    /// semaphore, so merely dropping the encoder would deadlock or reuse a
    /// signalled binary semaphore on the next frame.
    /// This is public so lightweight clients that drive [`VulkanDevice`]
    /// directly (without [`crate::SceneRenderer`]) can honour the same
    /// fail-and-recover frame contract.
    pub fn abort_current_frame_recording(&mut self) -> VkResult<()> {
        if self.frame_sync.is_empty() {
            return Ok(());
        }

        self.wait_idle_checked()?;
        let frame_index = self.current_frame;
        let frame = &self.frame_sync[frame_index];
        let device = &self.logical_device.device;

        // The command buffer was never submitted and the device is idle, so it
        // can return to the initial state regardless of where recording failed.
        // SAFETY: `wait_idle_checked` completed above; `frame.command_buffer`
        // belongs to a reset-capable pool and was never submitted this frame.
        unsafe {
            device
                .reset_command_buffer(
                    frame.command_buffer,
                    vk::CommandBufferResetFlags::RELEASE_RESOURCES,
                )
                .map_err(|result| VulkanError::vk("abort_reset_command_buffer", result))?;
        }

        // SAFETY: `device` is live; the default fence create-info contains no
        // borrowed storage and SIGNALED restores the pre-acquire frame state.
        let replacement_fence = unsafe {
            device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )
        }
        .map_err(|result| VulkanError::vk("abort_create_fence", result))?;
        let replacement_image_available =
            // SAFETY: `device` is live and the default create-info creates an
            // unowned binary semaphore suitable for image acquisition.
            match unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) } {
                Ok(semaphore) => semaphore,
                Err(result) => {
                    // SAFETY: the replacement fence was just created by this
                    // device and cannot have been submitted or shared.
                    unsafe { device.destroy_fence(replacement_fence, None) };
                    return Err(VulkanError::vk("abort_create_image_semaphore", result));
                }
            };
        let replacement_render_finished =
            // SAFETY: `device` is live and this fresh binary semaphore is not
            // yet referenced by a submission or present operation.
            match unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) } {
                Ok(semaphore) => semaphore,
                Err(result) => {
                    // SAFETY: both replacement handles were created in this
                    // transaction and have never been submitted or exposed.
                    unsafe {
                        device.destroy_semaphore(replacement_image_available, None);
                        device.destroy_fence(replacement_fence, None);
                    }
                    return Err(VulkanError::vk("abort_create_render_semaphore", result));
                }
            };

        let frame = &mut self.frame_sync[frame_index];
        let old_fence = std::mem::replace(&mut frame.in_flight_fence, replacement_fence);
        let old_image_available =
            std::mem::replace(&mut frame.image_available, replacement_image_available);
        let old_render_finished =
            std::mem::replace(&mut frame.render_finished, replacement_render_finished);
        // SAFETY: the device is idle; replacement above removed these handles
        // from frame state, giving this path exclusive ownership for destruction.
        unsafe {
            device.destroy_fence(old_fence, None);
            device.destroy_semaphore(old_image_available, None);
            device.destroy_semaphore(old_render_finished, None);
        }
        Ok(())
    }

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
        let in_flight_fence = self.frame_sync[fi].in_flight_fence;
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
        // valid VkSwapchainKHR; `image_available` is a binary semaphore as
        // required by `vkAcquireNextImageKHR`; timeout parameters are standard.
        let image_available = self.frame_sync[fi].image_available;
        // SAFETY: the swapchain/semaphore contract above holds and the returned
        // image index is consumed before either object can be retired.
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
    pub(crate) fn submit_and_present(&mut self, fi: usize, ii: u32) -> VkResult<bool> {
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

        // ── Binary semaphore synchronization ──────────────────────────
        // Wait for image acquisition and signal a separate binary semaphore
        // when rendering is ready for presentation.
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
        // structured with binary synchronization primitives.
        unsafe {
            d.queue_submit(self.logical_device.queue, &[si], f.in_flight_fence)
                .map_err(|r| VulkanError::vk("qs", r))?;
        }

        // ── Present ─────────────────────────────────────────────────────
        // Present waits for the binary semaphore signalled by queue submission.
        let sca = [sc.swapchain];
        let ia = [ii];
        let pi = vk::PresentInfoKHR::default()
            .wait_semaphores(&ss)
            .swapchains(&sca)
            .image_indices(&ia);
        // SAFETY: `queue` is valid; swapchain, semaphores, image indices,
        // and pNext chain are valid; `PresentInfoKHR` is correctly structured.
        match unsafe { sc.loader.queue_present(self.logical_device.queue, &pi) } {
            Ok(false) => Ok(false),
            Ok(true) => Ok(true),
            Err(r) if r == vk::Result::ERROR_OUT_OF_DATE_KHR || r == vk::Result::SUBOPTIMAL_KHR => {
                Ok(true)
            }
            Err(r) => Err(VulkanError::vk("qp", r)),
        }
    }

    /// Create frame-sync objects (fences, binary semaphores, command
    /// pools/buffers) for double-buffering.
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
            let cbs = match unsafe {
                d.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(cp)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
            } {
                Ok(cbs) => cbs,
                Err(r) => {
                    // SAFETY: `cp` was created above and owns no submitted work.
                    unsafe { d.destroy_command_pool(cp, None) };
                    return Err(VulkanError::vk("acb", r));
                }
            };

            let si = vk::SemaphoreCreateInfo::default();
            // SAFETY: `d` is a valid AshDevice; a default create-info produces
            // the binary semaphores required by acquire and present.
            let image_available = match unsafe { d.create_semaphore(&si, None) } {
                Ok(semaphore) => semaphore,
                Err(r) => {
                    // SAFETY: `cp` was created above and owns no submitted work.
                    unsafe { d.destroy_command_pool(cp, None) };
                    return Err(VulkanError::vk("create_image_available", r));
                }
            };
            // SAFETY: `d` is live and `si` creates a fresh binary semaphore;
            // it has no submission/present owner before being stored below.
            let render_finished = match unsafe { d.create_semaphore(&si, None) } {
                Ok(semaphore) => semaphore,
                Err(r) => {
                    // SAFETY: both handles were created above and are idle.
                    unsafe {
                        d.destroy_semaphore(image_available, None);
                        d.destroy_command_pool(cp, None);
                    }
                    return Err(VulkanError::vk("create_render_finished", r));
                }
            };

            // SAFETY: `d` is a valid AshDevice; fence is created in SIGNALED
            // state; `None` means no custom allocator.
            let fl = match unsafe {
                d.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
            } {
                Ok(fence) => fence,
                Err(r) => {
                    // SAFETY: all handles were created above and are idle.
                    unsafe {
                        d.destroy_semaphore(render_finished, None);
                        d.destroy_semaphore(image_available, None);
                        d.destroy_command_pool(cp, None);
                    }
                    return Err(VulkanError::vk("cf", r));
                }
            };
            self.frame_sync.push(FrameSync {
                image_available,
                render_finished,
                in_flight_fence: fl,
                command_pool: cp,
                command_buffer: cbs[0],
            });
        }
        Ok(())
    }
}
