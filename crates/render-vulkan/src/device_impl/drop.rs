//! Drop impl for VulkanDevice — destroys all GPU resources in the correct order.

use ash::vk;

use super::VulkanDevice;

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures it is not dropped before this destructor runs).
        unsafe { let _ = self.logical_device.device.device_wait_idle(); };
        self.drain_all_retired_pipelines();
        let d = &self.logical_device.device;
        for fb in self.mvp_framebuffers.drain(..) {
            // SAFETY: `fb` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        for fb in self.model_framebuffers.drain(..) {
            // SAFETY: `fb` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        if let Some(p) = self.mvp_pipeline.take() {
            // SAFETY: `p` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.mvp_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(p) = self.model_pipeline.take() {
            // SAFETY: `p` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.model_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(rp) = self.mvp_rp.take() {
            // SAFETY: `rp` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        if let Some(rp) = self.model_rp.take() {
            // SAFETY: `rp` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        for fs in self.frame_sync.drain(..) {
            // SAFETY: all handles in `fs` were created by this device and are
            // not yet destroyed; destruction order does not matter among
            // fences, semaphores, and pools.
            unsafe {
                d.destroy_fence(fs.in_flight_fence, None);
                d.destroy_semaphore(fs.image_available, None);
                d.destroy_semaphore(fs.render_finished, None);
                d.destroy_command_pool(fs.command_pool, None);
            }
        }
        for (_, e) in self.pipelines.slots.drain(..).flatten() {
            // SAFETY: `e.pipeline` was created by this device.
            unsafe {
                d.destroy_pipeline(e.pipeline, None);
            }
        }
        for (_, mut e) in self.buffers.slots.drain(..).flatten() {
            // SAFETY: `e.buffer` was created by this device.
            unsafe {
                d.destroy_buffer(e.buffer, None);
            }
            if let Some(mut a) = e.allocation.take() {
                if let Ok(mut guard) = e.allocator.lock() {
                    guard.free(&mut a);
                }
            }
        }
        for (_, rp) in self.render_passes.slots.drain(..).flatten() {
            // SAFETY: `rp` was created by this device.
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        for (_, fb) in self.framebuffers.slots.drain(..).flatten() {
            // SAFETY: `fb` was created by this device.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        for (_, e) in self.pipeline_layouts.slots.drain(..).flatten() {
            // SAFETY: `e.layout` and `e.set_layouts` were created by
            // this device.
            for sl in e.set_layouts {
                unsafe {
                    d.destroy_descriptor_set_layout(sl, None);
                }
            }
            unsafe {
                d.destroy_pipeline_layout(e.layout, None);
            }
        }

        // Destroy shader modules stored in the handle slab.
        for (_, (sm, _)) in self.shader_modules.slots.drain(..).flatten() {
            // SAFETY: `sm` was created by this device.
            unsafe {
                d.destroy_shader_module(sm, None);
            }
        }

        // Save pipeline cache data to disk before destroying.
        self.save_pipeline_cache();

        // Destroy pipeline cache if it was created (non-null).
        if self.pipeline_cache != vk::PipelineCache::null() {
            // SAFETY: `self.pipeline_cache` was created by this device.
            unsafe {
                d.destroy_pipeline_cache(self.pipeline_cache, None);
            }
        }

        // Destroy material descriptor infra (set=2 pool + layout).
        if let Some(pool) = self.material_desc_pool.take() {
            // SAFETY: pool frees its descriptor sets automatically.
            unsafe { d.destroy_descriptor_pool(pool, None); }
        }
        if let Some(layout) = self.material_desc_set_layout.take() {
            // SAFETY: layout was created by this device.
            unsafe { d.destroy_descriptor_set_layout(layout, None); }
        }

        self.destroy_light_ssbo();
        self.destroy_material_textures();
        // Destroy post-process resources (bloom + SSAO) before HDR and
        // shadow since they may reference HDR colour / depth images.
        self.destroy_post_process_resources();
        self.destroy_hdr_resources();
        self.destroy_env_resources();
        self.destroy_shadow_resources();
        self.destroy_descriptor_infra();
        self.destroy_depth_texture();
        drop(self.swapchain.take());
        // SAFETY: all device-child objects have been destroyed above.
        // Destroy VkDevice before VkInstance per Vulkan spec.
        unsafe { self.logical_device.device.destroy_device(None) };
        // Drop the allocator (Device::drop would do this, but we use
        // ManuallyDrop so it won't run automatically).
        drop(self.logical_device.allocator.take());
        drop(self.surface.take());
        drop(self.instance.take());
    }
}
