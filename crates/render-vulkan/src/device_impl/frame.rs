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
                    // Create post-processing resources (bloom + SSAO).
                    // These are idempotent and gracefully no-op when
                    // the required SPIR-V is unavailable.
                    let _ = self.create_bloom_resources();
                    let _ = self.create_ssao_resources();
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
        // valid VkSwapchainKHR; `self.frame_sync[fi].timeline_semaphore` is a
        // valid timeline semaphore (acquire_next_image increments its value by
        // 1 on signal); timeout parameters are standard Vulkan.
        let timeline_semaphore = self.frame_sync[fi].timeline_semaphore;
        let (ii, sub) = unsafe {
            sc.loader.acquire_next_image(
                sc.swapchain,
                u64::MAX,
                timeline_semaphore,
                vk::Fence::null(),
            )
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
    /// Updates `timeline_value` after a successful submit.
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

        // ── Timeline semaphore values ───────────────────────────────────
        // acquire_next_image has already signalled the timeline semaphore
        // to (timeline_value + 1).  We wait for that value in the submit
        // (ensuring the image is available) and then signal value+2 for
        // the next frame's CPU wait / acquire.
        let wait_value = f.timeline_value + 1; // image available
        let signal_value = f.timeline_value + 2; // render finished

        let ws = [f.timeline_semaphore];
        let wst = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let cbs = [f.command_buffer];
        let ss = [f.timeline_semaphore];

        // Build submit info with timeline extension chained.
        // (Bind array temporaries to locals to satisfy ash's borrow checker.)
        let wait_vals = [wait_value];
        let signal_vals = [signal_value];
        let mut timeline_info = vk::TimelineSemaphoreSubmitInfo::default()
            .wait_semaphore_values(&wait_vals)
            .signal_semaphore_values(&signal_vals);
        let si = vk::SubmitInfo::default()
            .wait_semaphores(&ws)
            .wait_dst_stage_mask(&wst)
            .command_buffers(&cbs)
            .signal_semaphores(&ss)
            .push_next(&mut timeline_info);
        // SAFETY: `queue` is a valid VkQueue; command buffer is in completed
        // state; semaphores and fence are valid; submit info is correctly
        // structured with timeline values.
        unsafe {
            d.queue_submit(self.logical_device.queue, &[si], f.in_flight_fence)
                .map_err(|r| VulkanError::vk("qs", r))?;
        }

        // ── Present ─────────────────────────────────────────────────────
        // NOTE: timeline semaphores work in VkPresentInfoKHR without a
        // TimelineSemaphoreSubmitInfo pNext — the driver waits for the last
        // signalled value (which is `signal_value` from our submit above).
        let sca = [sc.swapchain];
        let ia = [ii];
        let pi = vk::PresentInfoKHR::default()
            .wait_semaphores(&ss)
            .swapchains(&sca)
            .image_indices(&ia);
        // SAFETY: `queue` is valid; swapchain, semaphores, image indices,
        // and pNext chain are valid; `PresentInfoKHR` is correctly structured.
        let result = match unsafe { sc.loader.queue_present(self.logical_device.queue, &pi) } {
            Ok(false) => Ok(false),
            Ok(true) => Ok(true),
            Err(r) if r == vk::Result::ERROR_OUT_OF_DATE_KHR || r == vk::Result::SUBOPTIMAL_KHR => {
                Ok(true)
            }
            Err(r) => Err(VulkanError::vk("qp", r)),
        };

        // On success, bump the timeline value so the next begin_frame on this
        // slot waits for the render-finished signal.
        if result.is_ok() {
            // SAFETY: `fi` was validated at the top of this function; we write
            // through a &mut self reborrow.
            let fs = &mut self.frame_sync[fi];
            fs.timeline_value = signal_value;
        }

        result
    }

    /// Create frame-sync objects (fences, timeline semaphores, command
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
            let cbs = unsafe {
                d.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(cp)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
            }
            .map_err(|r| VulkanError::vk("acb", r))?;

            // Create timeline semaphore with initial value 0.
            let mut type_info = vk::SemaphoreTypeCreateInfo::default()
                .semaphore_type(vk::SemaphoreType::TIMELINE)
                .initial_value(0);
            let si = vk::SemaphoreCreateInfo::default().push_next(&mut type_info);
            // SAFETY: `d` is a valid AshDevice; `si` describes a valid timeline
            // semaphore; `None` means no custom allocator.
            let ts =
                unsafe { d.create_semaphore(&si, None) }.map_err(|r| VulkanError::vk("cts", r))?;

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
                timeline_semaphore: ts,
                timeline_value: 0,
                in_flight_fence: fl,
                command_pool: cp,
                command_buffer: cbs[0],
            });
        }
        Ok(())
    }
}
