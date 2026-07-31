use super::super::*;

impl VulkanDevice {
    pub fn new(
        display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
        enable_validation: bool,
        cache_dir: Option<&std::path::Path>,
    ) -> Result<Self, VulkanError> {
        // SAFETY: `Instance::new` wraps the Vulkan C entry-point creation; the
        // returned value owns the instance handle.
        let instance = unsafe { Instance::new(display_handle, enable_validation) }?;
        // SAFETY: `Surface::new` calls Vulkan FFI to create a surface; handles
        // are valid and owned by the newly-created Surface value.
        let surface = unsafe {
            Surface::new(
                &instance.entry,
                &instance.instance,
                display_handle,
                window_handle,
            )
        }?;
        // SAFETY: `select` iterates physical devices and picks one; the
        // instance/physical-device handles are valid.
        let adapter = unsafe {
            crate::adapter::select(&instance.instance, &surface.loader, surface.surface)
        }?;
        // SAFETY: `VkLogicalDevice::new` creates a Vulkan logical device; all
        // inputs (instance, physical device) are valid.
        let ld = unsafe { VkLogicalDevice::new(&instance.instance, &adapter) }?;
        // SAFETY: `device_name` is a null-terminated `VkPhysicalDeviceProperties`
        // field guaranteed by the Vulkan spec to be a valid NUL-terminated char
        // array.
        let name = unsafe { CStr::from_ptr(adapter.properties.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();

        // ---- Determine max MSAA sample count ----
        let sample_flags = adapter.properties.limits.framebuffer_color_sample_counts
            & adapter.properties.limits.framebuffer_depth_sample_counts;
        let max_msaa = if sample_flags.contains(vk::SampleCountFlags::TYPE_8) {
            vk::SampleCountFlags::TYPE_8
        } else if sample_flags.contains(vk::SampleCountFlags::TYPE_4) {
            vk::SampleCountFlags::TYPE_4
        } else if sample_flags.contains(vk::SampleCountFlags::TYPE_2) {
            vk::SampleCountFlags::TYPE_2
        } else {
            vk::SampleCountFlags::TYPE_1
        };
        let max_sample_count_u8 = match max_msaa {
            vk::SampleCountFlags::TYPE_8 => 8u8,
            vk::SampleCountFlags::TYPE_4 => 4u8,
            vk::SampleCountFlags::TYPE_2 => 2u8,
            _ => 1u8,
        };

        let info = AdapterInfo {
            backend: BackendKind::Vulkan,
            name,
            vendor_id: Some(adapter.properties.vendor_id),
            device_id: Some(adapter.properties.device_id),
            driver_version: None,
            capabilities: render_core::BackendCapabilities {
                max_texture_dimension_2d: 16384,
                max_color_attachments: 8,
                supports_swapchain: true,
                supports_timestamps: false,
                supports_debug_markers: enable_validation,
                supported_shader_formats: vec![ShaderFormat::SpirV],
                supported_surface_formats: vec![TextureFormat::Bgra8Unorm],
                limits: ResourceLimits {
                    max_buffer_bytes: u64::MAX,
                    max_texture_array_layers: 256,
                    max_bind_groups: 4,
                    max_vertex_attributes: 16,
                    max_color_attachments: 8,
                    max_sample_count: max_sample_count_u8,
                },
            },
        };
        let mut device = Self {
            instance: Some(instance),
            surface: Some(surface),
            adapter,
            logical_device: ManuallyDrop::new(ld),
            swapchain: None,
            swapchain_extent: vk::Extent2D {
                width: width.max(1),
                height: height.max(1),
            },
            window_width: width.max(1),
            window_height: height.max(1),
            minimized: width == 0 || height == 0,
            swapchain_recreate_pending: false,
            forward_vert_spv: None,
            forward_frag_spv: None,
            skinned_vert_spv: None,
            vfx_billboard_vert_spv: None,
            gpu_vfx_billboard_vert_spv: None,
            vfx_billboard_frag_spv: None,
            instanced_vert_spv: None,
            skybox_vert_spv: None,
            skybox_frag_spv: None,
            compute_queue: None,
            compute_pool: None,
            compute_cmd_buffer: None,
            frame_sync: Vec::new(),
            current_frame: 0,
            retired_pipelines: vec![Vec::new(), Vec::new()],
            cached_adapter_info: info,
            last_image_index: 0,
            buffers: Slab::new(),
            rhi_textures: Slab::new(),
            pipelines: Slab::new(),
            render_passes: Slab::new(),
            framebuffers: Slab::new(),
            pipeline_layouts: Slab::new(),
            shader_modules: Slab::new(),
            pipeline_cache: vk::PipelineCache::null(),
            pso_cache_path: None,
            rp_has_depth: HashMap::new(),
            rp_color_formats: HashMap::new(),
            rp_depth_formats: HashMap::new(),
            rp_sample_counts: HashMap::new(),
            desc_set_layout_0: None,
            desc_pool: None,
            frame_desc_sets: Vec::new(),
            frame_ubos: Vec::new(),
            ubo_size: 512,
            ubo_allocations: Vec::new(),
            ubo_alignment: 256,
            depth_image: None,
            depth_image_view: None,
            depth_allocation: None,

            // Environment cubemap (IBL)
            env_cubemap: None,
            env_cubemap_view: None,
            env_cubemap_allocation: None,
            env_sampler: None,

            // Material descriptor infrastructure (set=2)
            material_desc_set_layout: None,
            material_desc_pool: None,

            // Shadow mapping
            shadow_map: None,
            shadow_map_view: None,
            shadow_layer_views: Vec::new(),
            shadow_allocation: None,
            shadow_sampler: None,
            shadow_rp: None,
            shadow_pipeline_layout: None,
            shadow_pipeline: None,
            shadow_fbs: Vec::new(),
            shadow_desc_set: None,
            shadow_desc_layout: None,
            shadow_desc_pool: None,
            shadow_bind_layout: None,

            // HDR offscreen rendering
            hdr_color_image: None,
            hdr_color_view: None,
            hdr_color_allocation: None,
            hdr_color_sampler: None,
            hdr_msaa_color_image: None,
            hdr_msaa_color_view: None,
            hdr_msaa_color_allocation: None,
            oit_accum_image: None,
            oit_accum_view: None,
            oit_accum_allocation: None,
            oit_msaa_accum_image: None,
            oit_msaa_accum_view: None,
            oit_msaa_accum_allocation: None,
            oit_optical_depth_image: None,
            oit_optical_depth_view: None,
            oit_optical_depth_allocation: None,
            oit_msaa_optical_depth_image: None,
            oit_msaa_optical_depth_view: None,
            oit_msaa_optical_depth_allocation: None,
            hdr_msaa_depth_image: None,
            hdr_msaa_depth_view: None,
            hdr_msaa_depth_allocation: None,
            tone_rp: None,
            tone_pipeline: None,
            tone_pipeline_layout: None,
            tone_framebuffers: Vec::new(),
            tone_desc_set: None,
            tone_desc_pool: None,
            tone_desc_layout: None,
            hdr_forward_rp: None,
            hdr_forward_pipeline: None,
            hdr_forward_double_sided_pipeline: None,
            hdr_forward_blend_pipeline: None,
            hdr_forward_blend_double_sided_pipeline: None,
            hdr_forward_oit_pipeline: None,
            hdr_forward_oit_double_sided_pipeline: None,
            hdr_forward_additive_pipeline: None,
            hdr_forward_additive_double_sided_pipeline: None,
            hdr_vfx_billboard_pipeline: None,
            hdr_vfx_billboard_additive_pipeline: None,
            hdr_vfx_billboard_oit_pipeline: None,
            hdr_gpu_vfx_billboard_pipeline: None,
            hdr_gpu_vfx_billboard_additive_pipeline: None,
            hdr_gpu_vfx_billboard_oit_pipeline: None,
            hdr_instanced_pipeline: None,
            hdr_instanced_double_sided_pipeline: None,
            hdr_skybox_pipeline: None,
            hdr_forward_pipeline_layout: None,
            hdr_forward_fb: None,
            hdr_msaa_samples: vk::SampleCountFlags::TYPE_1,

            // Material texture cache (Phase 3.1)
            textures: HashMap::new(),

            // Editor UI overlay
            ui_overlay_rp: None,
            ui_overlay_pipeline_layout: None,
            ui_overlay_pipeline: None,
            ui_overlay_desc_layout: None,
            ui_overlay_desc_pool: None,
            ui_overlay_desc_sets: HashMap::new(),
            ui_overlay_framebuffers: Vec::new(),

            // Light SSBO (Phase 4.3)
            light_ssbo: None,
            light_ssbo_allocation: None,
            light_ssbo_size: 0,
            cluster_grid_ssbo: None,
            cluster_grid_ssbo_allocation: None,
            cluster_index_ssbo: None,
            cluster_index_ssbo_allocation: None,
        };

        // Phase 3.3: Initialize PSO cache (load from disk if cache_dir provided).
        device.init_pipeline_cache(cache_dir);

        // Phase 5.2: Create compute queue, pool, and command buffer.
        {
            let d = &device.logical_device.device;
            let compute_queue = device.logical_device.compute_queue;
            let compute_qfi = device.logical_device.compute_queue_family_index;
            device.compute_queue = compute_queue;

            // SAFETY: `d` is a valid AshDevice; `compute_qfi` is a valid queue
            // family index for this device; `None` means no custom allocator.
            let cp = unsafe {
                d.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(compute_qfi)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
            }
            .map_err(|r| VulkanError::vk("ccp_compute", r))?;

            // SAFETY: `cp` was just created and is valid; allocate one primary
            // command buffer from it.
            let cbs = unsafe {
                d.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(cp)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
            }
            .map_err(|r| VulkanError::vk("acb_compute", r))?;

            device.compute_pool = Some(cp);
            device.compute_cmd_buffer = Some(cbs[0]);
        }

        Ok(device)
    }

    /// Create a new VulkanDevice without PSO cache persistence.
    /// Convenience wrapper that passes `cache_dir: None`.
    pub fn new_without_cache(
        display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
        enable_validation: bool,
    ) -> Result<Self, VulkanError> {
        Self::new(
            display_handle,
            window_handle,
            width,
            height,
            enable_validation,
            None,
        )
    }

    /// Initialize the pipeline cache, optionally loading from a file.
    pub fn init_pipeline_cache(&mut self, cache_dir: Option<&std::path::Path>) {
        let mut initial_data = Vec::new();
        let d = &self.logical_device.device;

        if let Some(dir) = cache_dir {
            let path = dir.join("pso_cache.bin");
            if let Ok(data) = std::fs::read(&path) {
                // Validate header: first 4 bytes should be "PSC\0" or similar
                // For now just try to use whatever was on disk; corrupt data
                // will be rejected by the driver at creation time.
                if data.len() >= 4 {
                    initial_data = data;
                }
                tracing::info!(size = initial_data.len(), "loaded PSO cache from disk");
            } else {
                tracing::debug!("no existing PSO cache file, starting fresh");
            }
            self.pso_cache_path = Some(path);
        }

        let ci = vk::PipelineCacheCreateInfo::default().initial_data(&initial_data);
        // SAFETY: `d` is a valid device; `ci` is correctly constructed.
        match unsafe { d.create_pipeline_cache(&ci, None) } {
            Ok(cache) => {
                self.pipeline_cache = cache;
                tracing::info!("pipeline cache created");
            }
            Err(r) => {
                tracing::warn!(error = %r, "failed to create pipeline cache, continuing without");
                self.pipeline_cache = vk::PipelineCache::null();
            }
        }
    }

    /// Save the pipeline cache to disk (call on shutdown or after bulk compiles).
    pub fn save_pipeline_cache(&self) {
        let Some(ref path) = self.pso_cache_path else {
            return;
        };
        if self.pipeline_cache == vk::PipelineCache::null() {
            return;
        }
        let d = &self.logical_device.device;
        // SAFETY: `d` is a valid device; `self.pipeline_cache` is valid or null.
        let data = match unsafe { d.get_pipeline_cache_data(self.pipeline_cache) } {
            Ok(d) => d,
            Err(r) => {
                tracing::warn!(error = %r, "failed to get pipeline cache data");
                return;
            }
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(path, &data) {
            Ok(_) => tracing::info!(bytes = data.len(), "PSO cache saved"),
            Err(e) => tracing::warn!(error = %e, "failed to save PSO cache"),
        }
    }
}
