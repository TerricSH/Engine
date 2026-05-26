//! Top-level Vulkan renderer that owns every Gate 2 MVP resource and
//! drives the frame lifecycle (acquire -> record -> submit -> present).

use ash::vk;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::adapter::{self, AdapterSelection};
use crate::device::Device;
use crate::error::{VkResult, VulkanError};
use crate::frame::FrameContext;
use crate::instance::Instance;
use crate::pipeline::Pipeline;
use crate::shaders_embedded::{TRIANGLE_FRAG_SPV, TRIANGLE_VERT_SPV};
use crate::surface::Surface;
use crate::swapchain::Swapchain;

#[derive(Clone, Debug)]
pub struct VulkanRendererDescriptor {
    pub display_handle: RawDisplayHandle,
    pub window_handle: RawWindowHandle,
    pub width: u32,
    pub height: u32,
    pub enable_validation: bool,
}

pub struct VulkanRenderer {
    // Drop order: pipeline -> frames -> swapchain -> device -> surface -> instance.
    pipeline: Option<Pipeline>,
    frames: Option<FrameContext>,
    swapchain: Option<Swapchain>,
    adapter: AdapterSelection,
    device: Device,
    surface: Surface,
    instance: Instance,

    requested_extent: vk::Extent2D,
    minimized: bool,
}

impl VulkanRenderer {
    pub fn new(descriptor: VulkanRendererDescriptor) -> VkResult<Self> {
        // SAFETY: descriptor carries valid display/window handles per caller contract.
        let instance =
            unsafe { Instance::new(descriptor.display_handle, descriptor.enable_validation) }?;
        // SAFETY: ditto.
        let surface = unsafe {
            Surface::new(
                &instance.entry,
                &instance.instance,
                descriptor.display_handle,
                descriptor.window_handle,
            )
        }?;
        // SAFETY: instance + surface are valid.
        let adapter =
            unsafe { adapter::select(&instance.instance, &surface.loader, surface.surface) }?;
        tracing::info!(
            target: "vulkan",
            device_type = ?adapter.properties.device_type,
            "physical device selected"
        );
        // SAFETY: instance + adapter valid.
        let device = unsafe { Device::new(&instance.instance, &adapter) }?;
        // SAFETY: device + queue family valid.
        let frames =
            unsafe { FrameContext::new(device.device.clone(), device.queue_family_index) }?;

        let requested_extent = vk::Extent2D {
            width: descriptor.width,
            height: descriptor.height,
        };

        let mut renderer = Self {
            pipeline: None,
            frames: Some(frames),
            swapchain: None,
            adapter,
            device,
            surface,
            instance,
            requested_extent,
            minimized: false,
        };
        // SAFETY: renderer state valid; build swapchain + pipeline now.
        unsafe { renderer.create_swapchain_chain()? };
        Ok(renderer)
    }

    /// Request a new logical drawable size. Actual swapchain recreation is
    /// deferred to the next [`render`](Self::render) call so it can wait
    /// for in-flight frames cleanly.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.requested_extent = vk::Extent2D { width, height };
        self.minimized = width == 0 || height == 0;
        if self.minimized {
            tracing::debug!(target: "vulkan", "window minimized; pausing rendering");
        }
        // Drop existing swapchain/pipeline so render() rebuilds them.
        self.pipeline = None;
        if let Some(swapchain) = &self.swapchain {
            // Wait for outstanding work tied to the old swapchain before dropping it.
            // SAFETY: device handle is valid.
            unsafe {
                let _ = self.device.device.device_wait_idle();
            }
            let _ = swapchain;
        }
        self.swapchain = None;
    }

    pub fn wait_idle(&self) {
        // SAFETY: device handle is valid.
        let _ = unsafe { self.device.device.device_wait_idle() };
    }

    /// Submit one frame. Returns `Ok(())` even when the frame is skipped
    /// because the surface is minimized.
    pub fn render(&mut self) -> VkResult<()> {
        if self.minimized {
            return Ok(());
        }
        if self.swapchain.is_none() || self.pipeline.is_none() {
            // SAFETY: state has been invalidated by resize; rebuild.
            match unsafe { self.create_swapchain_chain() } {
                Ok(()) => {}
                Err(VulkanError::SurfaceMinimized) => {
                    self.minimized = true;
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }

        // SAFETY: state is fully populated above.
        let outcome = unsafe { self.record_and_submit() };
        match outcome {
            Ok(()) => Ok(()),
            Err(VulkanError::SwapchainOutOfDate) | Err(VulkanError::SurfaceMinimized) => {
                tracing::debug!(target: "vulkan", "swapchain out of date; recreating");
                self.pipeline = None;
                self.swapchain = None;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    unsafe fn create_swapchain_chain(&mut self) -> VkResult<()> {
        // SAFETY: all handles in scope are valid.
        let swapchain = unsafe {
            Swapchain::new(
                &self.instance.instance,
                self.device.device.clone(),
                self.adapter.physical_device,
                self.device.queue_family_index,
                &self.surface.loader,
                self.surface.surface,
                self.requested_extent.width,
                self.requested_extent.height,
            )
        }?;
        // SAFETY: device + image views are valid.
        let pipeline = unsafe {
            Pipeline::new(
                self.device.device.clone(),
                swapchain.format,
                swapchain.extent,
                &swapchain.image_views,
                TRIANGLE_VERT_SPV,
                TRIANGLE_FRAG_SPV,
            )
        }?;
        self.swapchain = Some(swapchain);
        self.pipeline = Some(pipeline);
        self.minimized = false;
        Ok(())
    }

    unsafe fn record_and_submit(&mut self) -> VkResult<()> {
        let swapchain = self.swapchain.as_ref().expect("swapchain present");
        let pipeline = self.pipeline.as_ref().expect("pipeline present");
        let frames = self.frames.as_mut().expect("frames present");
        let device = &self.device.device;

        let frame = &frames.frames[frames.current];

        // 1. Wait for previous use of this frame slot to finish.
        // SAFETY: device + fence valid.
        unsafe {
            device
                .wait_for_fences(&[frame.in_flight], true, u64::MAX)
                .map_err(|r| VulkanError::vk("wait_for_fences", r))?;
        }

        // 2. Acquire next swapchain image.
        // SAFETY: swapchain + semaphore valid.
        let (image_index, suboptimal) = match unsafe {
            swapchain.loader.acquire_next_image(
                swapchain.swapchain,
                u64::MAX,
                frame.image_available,
                vk::Fence::null(),
            )
        } {
            Ok(v) => v,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                return Err(VulkanError::SwapchainOutOfDate);
            }
            Err(r) => return Err(VulkanError::vk("acquire_next_image", r)),
        };

        // 3. Reset fence now that we know we'll submit work.
        // SAFETY: device + fence valid.
        unsafe {
            device
                .reset_fences(&[frame.in_flight])
                .map_err(|r| VulkanError::vk("reset_fences", r))?;
        }

        // 4. Reset and record command buffer.
        // SAFETY: command buffer valid.
        unsafe {
            device
                .reset_command_buffer(frame.command_buffer, vk::CommandBufferResetFlags::empty())
                .map_err(|r| VulkanError::vk("reset_command_buffer", r))?;
        }
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: begin_info outlives the call.
        unsafe {
            device
                .begin_command_buffer(frame.command_buffer, &begin_info)
                .map_err(|r| VulkanError::vk("begin_command_buffer", r))?;
        }

        let clear_values = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.02, 0.02, 0.06, 1.0],
            },
        }];
        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(pipeline.render_pass)
            .framebuffer(pipeline.framebuffers[image_index as usize])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: pipeline.extent,
            })
            .clear_values(&clear_values);
        // SAFETY: render_pass_info outlives the call.
        unsafe {
            device.cmd_begin_render_pass(
                frame.command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );
        }

        let viewports = [vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: pipeline.extent.width as f32,
            height: pipeline.extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }];
        let scissors = [vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: pipeline.extent,
        }];
        // SAFETY: command buffer + viewport/scissor slices valid for the call.
        unsafe {
            device.cmd_set_viewport(frame.command_buffer, 0, &viewports);
            device.cmd_set_scissor(frame.command_buffer, 0, &scissors);
            device.cmd_bind_pipeline(
                frame.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.pipeline,
            );
            device.cmd_draw(frame.command_buffer, 3, 1, 0, 0);
            device.cmd_end_render_pass(frame.command_buffer);
            device
                .end_command_buffer(frame.command_buffer)
                .map_err(|r| VulkanError::vk("end_command_buffer", r))?;
        }

        // 5. Submit.
        let wait_semaphores = [frame.image_available];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let command_buffers = [frame.command_buffer];
        let signal_semaphores = [frame.render_finished];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores);
        // SAFETY: all slices outlive the call.
        unsafe {
            device
                .queue_submit(self.device.queue, &[submit_info], frame.in_flight)
                .map_err(|r| VulkanError::vk("queue_submit", r))?;
        }

        // 6. Present.
        let swapchains = [swapchain.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);
        // SAFETY: slices outlive the call.
        let present_result = unsafe {
            swapchain
                .loader
                .queue_present(self.device.queue, &present_info)
        };
        match present_result {
            Ok(false) if !suboptimal => {}
            Ok(_) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) | Err(vk::Result::SUBOPTIMAL_KHR) => {
                // Defer rebuild; current frame already accepted.
                frames.advance();
                return Err(VulkanError::SwapchainOutOfDate);
            }
            Err(r) => return Err(VulkanError::vk("queue_present", r)),
        }

        frames.advance();
        Ok(())
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        // Wait for the GPU to drain before tearing anything down.
        // SAFETY: device handle is valid.
        let _ = unsafe { self.device.device.device_wait_idle() };
    }
}
