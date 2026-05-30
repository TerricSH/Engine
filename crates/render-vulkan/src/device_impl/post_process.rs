//! Post-processing pipeline: bloom + SSAO.
//!
//! Phase 4.5 adds two compute-sahder-based effects as custom render-graph
//! passes:
//!
//! * **Bloom** — a 2×2 downsample chain (4 iterations) followed by an
//!   upsample composite that accumulates the result back onto the HDR
//!   colour image before tone-mapping.
//! * **SSAO** — screen-space ambient occlusion computed from the depth
//!   buffer using a random kernel sampled in a hemisphere around each
//!   pixel.  The occlusion factor is written to an R8 texture.
//!
//! Both passes degrade gracefully when the required SPIR-V is unavailable
//! (all pipelines remain `None` and dispatch is a no-op).

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{mk_sm, VulkanDevice};

// ---------------------------------------------------------------------------
// Inline SPIR-V constants
// ---------------------------------------------------------------------------
//
// These are placeholders that will be replaced with real compute shaders.
// When empty the pipeline-creation code returns `MissingShader` and both
// passes silently become no-ops.
//
// TODO(Gate 4.5): compile bloom_downsample.comp, bloom_upsample.comp and
// ssao.comp from GLSL and embed the .spv output here.

/// Compute shader: downsample a texture by 2× (box filter).
pub(crate) const BLOOM_DOWNSAMPLE_COMP_SPV: &[u8] = &[];
/// Compute shader: upsample + blend with the previous mip level.
pub(crate) const BLOOM_UPSAMPLE_COMP_SPV: &[u8] = &[];
/// Compute shader: screen-space ambient occlusion from depth.
pub(crate) const SSAO_COMPUTE_COMP_SPV: &[u8] = &[];

// ============================================================================
// Bloom implementation
// ============================================================================

/// Number of bloom mip levels (downsample iterations).
const BLOOM_MIP_LEVELS: u32 = 4;
/// Minimum dimension for the smallest bloom mip (width or height).
const BLOOM_MIN_SIZE: u32 = 64;
/// Bloom intensity scaling for the upsample composite.
const BLOOM_INTENSITY: f32 = 0.3;

impl VulkanDevice {
    /// Create (or recreate) all bloom resources.
    ///
    /// Idempotent — returns `Ok(())` if already initialized.
    pub(crate) fn create_bloom_resources(&mut self) -> VkResult<()> {
        if !self.bloom_images.is_empty() {
            return Ok(());
        }

        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();
        let extent = self.swapchain_extent;
        if extent.width == 0 || extent.height == 0 {
            return Ok(());
        }

        // ---- 1. Sampler (linear, clamp-to-edge) ----
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .min_lod(0.0)
            .max_lod(BLOOM_MIP_LEVELS as f32)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false);
        let sampler = unsafe { d.create_sampler(&sampler_info, None) }
            .map_err(|r| VulkanError::vk("create_bloom_sampler", r))?;

        // ---- 2. Mip-chain images + views ----
        let mip_count = compute_bloom_mip_count(extent.width, extent.height);

        let mut images = Vec::with_capacity(mip_count as usize);
        let mut views = Vec::with_capacity(mip_count as usize);
        let mut allocations = Vec::with_capacity(mip_count as usize);

        let mut w = extent.width;
        let mut h = extent.height;

        for _level in 0..mip_count {
            w = (w / 2).max(1);
            h = (h / 2).max(1);

            // Image
            let image_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(vk::Format::R16G16B16A16_SFLOAT)
                .extent(vk::Extent3D {
                    width: w,
                    height: h,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(
                    vk::ImageUsageFlags::SAMPLED
                        | vk::ImageUsageFlags::STORAGE
                        | vk::ImageUsageFlags::TRANSFER_SRC
                        | vk::ImageUsageFlags::TRANSFER_DST,
                )
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let image = unsafe { d.create_image(&image_info, None) }
                .map_err(|r| VulkanError::vk("create_bloom_mip_image", r))?;
            let req = unsafe { d.get_image_memory_requirements(image) };
            let allocation = allocator
                .lock()
                .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
                .allocate(&crate::allocator::AllocationCreateDesc {
                    name: "bloom-mip",
                    requirements: req,
                    location: crate::allocator::MemoryLocation::GpuOnly,
                    linear: false,
                    allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
                })
                .map_err(|e| VulkanError::Allocation(e.to_string()))?;
            unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
                .map_err(|r| VulkanError::vk("bind_bloom_mip", r))?;

            // Image view (sampled + storage)
            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(vk::Format::R16G16B16A16_SFLOAT)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            let image_view = unsafe { d.create_image_view(&view_info, None) }
                .map_err(|r| VulkanError::vk("create_bloom_mip_view", r))?;

            images.push(image);
            views.push(image_view);
            allocations.push(allocation);
        }

        // ---- 3. Single descriptor set (updated per dispatch step) ----
        // Layout: binding 0 = input (sampled image), binding 1 = output (storage image).
        let ds_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let ds_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&ds_bindings);
        let ds_layout = unsafe { d.create_descriptor_set_layout(&ds_layout_info, None) }
            .map_err(|r| VulkanError::vk("create_bloom_ds_layout", r))?;

        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: 1,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_IMAGE,
                descriptor_count: 1,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_bloom_ds_pool", r))?;

        let ds_layouts = [ds_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&ds_layouts);
        let sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_bloom_ds", r))?;
        let desc_set = sets[0];

        // ---- 3. Pipeline layout ----
        let pc_range = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::COMPUTE,
            offset: 0,
            size: 16, // vec4: (width, height, intensity_pad, level_pad)
        }];
        let set_layouts = [ds_layout];
        let pll_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&pc_range);
        let pll = unsafe { d.create_pipeline_layout(&pll_info, None) }
            .map_err(|r| VulkanError::vk("cpl_bloom", r))?;

        // ---- 4. Compute pipelines ----
        let mut down_pipes = Vec::with_capacity(mip_count as usize);
        let mut up_pipes = Vec::with_capacity(mip_count as usize);

        if !BLOOM_DOWNSAMPLE_COMP_SPV.is_empty() {
            let sm_down = unsafe { mk_sm(d, BLOOM_DOWNSAMPLE_COMP_SPV)? };
            let ci = vk::ComputePipelineCreateInfo::default()
                .stage(
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::COMPUTE)
                        .module(sm_down)
                        .name(c"main"),
                )
                .layout(pll);
            let pipe =
                unsafe { d.create_compute_pipelines(vk::PipelineCache::null(), &[ci], None) }
                    .map_err(|(_, r)| VulkanError::vk("cgp_bloom_down", r))?;
            down_pipes = pipe;
            unsafe {
                d.destroy_shader_module(sm_down, None);
            }
        }

        if !BLOOM_UPSAMPLE_COMP_SPV.is_empty() {
            let sm_up = unsafe { mk_sm(d, BLOOM_UPSAMPLE_COMP_SPV)? };
            let ci = vk::ComputePipelineCreateInfo::default()
                .stage(
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::COMPUTE)
                        .module(sm_up)
                        .name(c"main"),
                )
                .layout(pll);
            let pipe =
                unsafe { d.create_compute_pipelines(vk::PipelineCache::null(), &[ci], None) }
                    .map_err(|(_, r)| VulkanError::vk("cgp_bloom_up", r))?;
            up_pipes = pipe;
            unsafe {
                d.destroy_shader_module(sm_up, None);
            }
        }

        // ---- Store ----
        self.bloom_sampler = Some(sampler);
        self.bloom_images = images;
        self.bloom_image_views = views;
        self.bloom_allocations = allocations;
        self.bloom_desc_sets = vec![desc_set]; // single DS, updated per dispatch
        self.bloom_desc_pool = Some(pool);
        self.bloom_desc_layout = Some(ds_layout);
        self.bloom_pipeline_layout = Some(pll);
        self.bloom_downsample_pipelines = down_pipes;
        self.bloom_upsample_pipelines = up_pipes;

        Ok(())
    }

    /// Transition all bloom mip images to the given layout using a
    /// pipeline barrier.
    ///
    /// Called before and after the bloom pass to prepare/clean up the
    /// image layouts.  Requires a valid frame index `fi` whose command
    /// buffer is in the recording state.
    #[allow(dead_code)]
    pub(crate) fn barrier_bloom_images(
        &self,
        fi: usize,
        old_layout: vk::ImageLayout,
        new_layout: vk::ImageLayout,
        src_access: vk::AccessFlags,
        dst_access: vk::AccessFlags,
        src_stage: vk::PipelineStageFlags,
        dst_stage: vk::PipelineStageFlags,
    ) {
        let d = &self.logical_device.device;
        let cmd = self.frame_sync[fi].command_buffer;
        let mut barriers = Vec::with_capacity(self.bloom_images.len());
        for &img in &self.bloom_images {
            barriers.push(
                vk::ImageMemoryBarrier::default()
                    .image(img)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .src_access_mask(src_access)
                    .dst_access_mask(dst_access)
                    .old_layout(old_layout)
                    .new_layout(new_layout),
            );
        }
        if !barriers.is_empty() {
            unsafe {
                d.cmd_pipeline_barrier(
                    cmd,
                    src_stage,
                    dst_stage,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &barriers,
                );
            }
        }
    }

    /// Dispatch the full bloom chain: downsample 4× → upsample composite.
    ///
    /// Reads from the HDR color image (level 0 source) and writes the
    /// bloom result back to it via the upsample composite.
    ///
    /// No-op when bloom pipelines haven't been initialized.
    pub(crate) fn dispatch_bloom(&self, fi: usize) {
        let pll = match self.bloom_pipeline_layout {
            Some(l) => l,
            None => return,
        };
        let Some(ds) = self.bloom_desc_sets.first().copied() else {
            return;
        };
        let sampler = match self.bloom_sampler {
            Some(s) => s,
            None => return,
        };
        let d = &self.logical_device.device;
        let cmd = self.frame_sync[fi].command_buffer;

        let mip_count = self.bloom_image_views.len() as u32;
        if mip_count < 2 {
            return; // need at least 2 levels for a chain
        }

        // ---- Helper: update descriptor for a specific source→target pair ----
        let update_bloom_ds =
            |ds: vk::DescriptorSet, src_view: vk::ImageView, dst_view: vk::ImageView| {
                let src_image_info = [vk::DescriptorImageInfo::default()
                    .sampler(sampler)
                    .image_view(src_view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                let dst_image_info = [vk::DescriptorImageInfo::default()
                    .image_view(dst_view)
                    .image_layout(vk::ImageLayout::GENERAL)];
                let writes = [
                    vk::WriteDescriptorSet::default()
                        .dst_set(ds)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                        .image_info(&src_image_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(ds)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                        .image_info(&dst_image_info),
                ];
                unsafe {
                    d.update_descriptor_sets(&writes, &[]);
                }
            };

        // ---- Downsample chain ----
        for level in 0..mip_count.saturating_sub(1) {
            let pipe = self
                .bloom_downsample_pipelines
                .first()
                .copied()
                .unwrap_or(vk::Pipeline::null());
            if pipe == vk::Pipeline::null() {
                continue;
            }

            // Update DS: read level N as sampled, write level N+1 as storage
            update_bloom_ds(
                ds,
                self.bloom_image_views[level as usize],
                self.bloom_image_views[level as usize + 1],
            );

            let img_w = (self.bloom_image_width(level) / 16).max(1);
            let img_h = (self.bloom_image_height(level) / 16).max(1);

            unsafe {
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipe);
                d.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, pll, 0, &[ds], &[]);
                // Push constants: (width, height, 0, 0)
                let pc = [img_w as f32, img_h as f32, 0.0f32, 0.0f32];
                let pc_bytes: &[u8] = std::slice::from_raw_parts(&pc as *const _ as *const u8, 16);
                d.cmd_push_constants(cmd, pll, vk::ShaderStageFlags::COMPUTE, 0, pc_bytes);
                d.cmd_dispatch(cmd, img_w, img_h, 1);
            }
        }

        // ---- Upsample composite ----
        for level in (1..mip_count).rev() {
            let pipe = self
                .bloom_upsample_pipelines
                .first()
                .copied()
                .unwrap_or(vk::Pipeline::null());
            if pipe == vk::Pipeline::null() {
                continue;
            }

            // Update DS: read level N as sampled, write level N-1 as storage
            update_bloom_ds(
                ds,
                self.bloom_image_views[level as usize],
                self.bloom_image_views[level as usize - 1],
            );

            let img_w = (self.bloom_image_width(level - 1) / 16).max(1);
            let img_h = (self.bloom_image_height(level - 1) / 16).max(1);

            unsafe {
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipe);
                d.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, pll, 0, &[ds], &[]);
                let pc = [img_w as f32, img_h as f32, BLOOM_INTENSITY, 0.0f32];
                let pc_bytes: &[u8] = std::slice::from_raw_parts(&pc as *const _ as *const u8, 16);
                d.cmd_push_constants(cmd, pll, vk::ShaderStageFlags::COMPUTE, 0, pc_bytes);
                d.cmd_dispatch(cmd, img_w, img_h, 1);
            }
        }

        // Restore DS binding 0 to HDR for the tone-mapping pass
        if let Some(hdr_view) = self.hdr_color_view {
            let hdr_image_info = [vk::DescriptorImageInfo::default()
                .sampler(sampler)
                .image_view(hdr_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let writes = [vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&hdr_image_info)];
            unsafe {
                d.update_descriptor_sets(&writes, &[]);
            }
        }
    }

    // -- helpers --

    fn bloom_image_width(&self, level: u32) -> u32 {
        let mut w = self.swapchain_extent.width;
        for _ in 0..level {
            w = (w / 2).max(1);
        }
        w
    }

    fn bloom_image_height(&self, level: u32) -> u32 {
        let mut h = self.swapchain_extent.height;
        for _ in 0..level {
            h = (h / 2).max(1);
        }
        h
    }
}

/// Compute the number of bloom mip levels given the input dimensions.
fn compute_bloom_mip_count(width: u32, height: u32) -> u32 {
    let mut count = 0u32;
    let mut w = width;
    let mut h = height;
    while (w > BLOOM_MIN_SIZE || h > BLOOM_MIN_SIZE) && count < BLOOM_MIP_LEVELS {
        w /= 2;
        h /= 2;
        count += 1;
    }
    count.max(1) // at least one mip
}

// ============================================================================
// SSAO implementation
// ============================================================================

/// Size of the SSAO noise texture (4×4).
const SSAO_NOISE_DIM: u32 = 4;
/// SSAO kernel size (number of samples).
const SSAO_KERNEL_SIZE: u32 = 16;
/// SSAO radius in view-space units.
const SSAO_RADIUS: f32 = 0.5;
/// SSAO power exponent (contrast).
const SSAO_POWER: f32 = 1.0;

impl VulkanDevice {
    /// Create (or recreate) all SSAO resources.
    ///
    /// Idempotent — returns `Ok(())` if already initialized.
    pub(crate) fn create_ssao_resources(&mut self) -> VkResult<()> {
        if self.ssao_pipeline.is_some() {
            return Ok(());
        }

        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();
        let extent = self.swapchain_extent;
        if extent.width == 0 || extent.height == 0 {
            return Ok(());
        }

        // ---- 1. Depth sampler (nearest, clamp-to-edge) ----
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .min_lod(0.0)
            .max_lod(1.0)
            .anisotropy_enable(false);
        let depth_sampler = unsafe { d.create_sampler(&sampler_info, None) }
            .map_err(|r| VulkanError::vk("create_ssao_depth_sampler", r))?;

        // ---- 2. Noise texture (4×4 random rotation vectors, RGBA8) ----
        let noise_pixels = generate_ssao_noise();
        let noise_size = (SSAO_NOISE_DIM * SSAO_NOISE_DIM * 4) as usize;
        let noise_image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width: SSAO_NOISE_DIM,
                height: SSAO_NOISE_DIM,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let noise_image = unsafe { d.create_image(&noise_image_info, None) }
            .map_err(|r| VulkanError::vk("create_ssao_noise_image", r))?;
        let req_n = unsafe { d.get_image_memory_requirements(noise_image) };
        let noise_allocation = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "ssao-noise",
                requirements: req_n,
                location: crate::allocator::MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        unsafe {
            d.bind_image_memory(
                noise_image,
                noise_allocation.memory(),
                noise_allocation.offset(),
            )
        }
        .map_err(|r| VulkanError::vk("bind_ssao_noise", r))?;

        let noise_view_info = vk::ImageViewCreateInfo::default()
            .image(noise_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let noise_view = unsafe { d.create_image_view(&noise_view_info, None) }
            .map_err(|r| VulkanError::vk("create_ssao_noise_view", r))?;

        // Upload noise data via one-shot staging (OK here because
        // ensure_sc pre-dates per-frame command-buffer recording).
        let staging_desc = vk::BufferCreateInfo::default()
            .size(noise_size as u64)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let staging_buf = unsafe { d.create_buffer(&staging_desc, None) }
            .map_err(|r| VulkanError::vk("create_ssao_staging", r))?;
        let req_s = unsafe { d.get_buffer_memory_requirements(staging_buf) };
        let mut staging_alloc = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "ssao-staging",
                requirements: req_s,
                location: crate::allocator::MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        unsafe {
            d.bind_buffer_memory(staging_buf, staging_alloc.memory(), staging_alloc.offset())
        }
        .map_err(|r| VulkanError::vk("bind_ssao_staging", r))?;
        // Copy noise data into staging buffer
        let staging_ptr = unsafe {
            d.map_memory(
                staging_alloc.memory(),
                staging_alloc.offset(),
                noise_size as u64,
                vk::MemoryMapFlags::empty(),
            )
        }
        .map_err(|r| VulkanError::vk("map_ssao_staging", r))?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                noise_pixels.as_ptr(),
                staging_ptr as *mut u8,
                noise_size,
            );
            d.unmap_memory(staging_alloc.memory());
        }

        // One-shot command buffer for the transfer
        let temp_pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.logical_device.queue_family_index)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let temp_pool = unsafe { d.create_command_pool(&temp_pool_info, None) }
            .map_err(|r| VulkanError::vk("create_ssao_temp_pool", r))?;
        let temp_cmds = unsafe {
            d.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(temp_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(|r| VulkanError::vk("alloc_ssao_temp_cb", r))?;
        let temp_cmd = temp_cmds[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe { d.begin_command_buffer(temp_cmd, &begin_info) }
            .map_err(|r| VulkanError::vk("begin_ssao_temp_cb", r))?;

        // Transition noise image → TRANSFER_DST_OPTIMAL
        let pre_barrier = vk::ImageMemoryBarrier::default()
            .image(noise_image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL);
        unsafe {
            d.cmd_pipeline_barrier(
                temp_cmd,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[pre_barrier],
            );
        }

        // Copy buffer → image
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D {
                width: SSAO_NOISE_DIM,
                height: SSAO_NOISE_DIM,
                depth: 1,
            });
        unsafe {
            d.cmd_copy_buffer_to_image(
                temp_cmd,
                staging_buf,
                noise_image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }

        // Transition noise → SHADER_READ_ONLY_OPTIMAL
        let post_barrier = vk::ImageMemoryBarrier::default()
            .image(noise_image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        unsafe {
            d.cmd_pipeline_barrier(
                temp_cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[post_barrier],
            );
        }

        unsafe { d.end_command_buffer(temp_cmd) }
            .map_err(|r| VulkanError::vk("end_ssao_temp_cb", r))?;

        // Submit and wait
        let temp_cmds_slice = [temp_cmd];
        let submit_info = vk::SubmitInfo::default().command_buffers(&temp_cmds_slice);
        unsafe {
            d.queue_submit(self.logical_device.queue, &[submit_info], vk::Fence::null())
                .map_err(|r| VulkanError::vk("submit_ssao_temp", r))?;
            d.device_wait_idle()
                .map_err(|r| VulkanError::vk("wait_ssao_temp", r))?;
        }

        // Clean up staging + temp pool
        unsafe {
            d.destroy_buffer(staging_buf, None);
            d.destroy_command_pool(temp_pool, None);
        }
        if let Ok(mut guard) = allocator.lock() {
            guard.free(&mut staging_alloc);
        }

        // ---- 3. Output texture (R8_UNORM, half-resolution) ----
        let out_w = (extent.width / 2).max(1);
        let out_h = (extent.height / 2).max(1);

        let out_image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8_UNORM)
            .extent(vk::Extent3D {
                width: out_w,
                height: out_h,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let out_image = unsafe { d.create_image(&out_image_info, None) }
            .map_err(|r| VulkanError::vk("create_ssao_output_image", r))?;
        let req_o = unsafe { d.get_image_memory_requirements(out_image) };
        let out_allocation = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "ssao-output",
                requirements: req_o,
                location: crate::allocator::MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        unsafe { d.bind_image_memory(out_image, out_allocation.memory(), out_allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_ssao_output", r))?;

        let out_view_info = vk::ImageViewCreateInfo::default()
            .image(out_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8_UNORM)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let out_view = unsafe { d.create_image_view(&out_view_info, None) }
            .map_err(|r| VulkanError::vk("create_ssao_output_view", r))?;

        // ---- 4. Descriptor set layout ----
        // binding 0 = depth (sampled)
        // binding 1 = noise (sampled)
        // binding 2 = output (storage)
        let ds_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let ds_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&ds_bindings);
        let ds_layout = unsafe { d.create_descriptor_set_layout(&ds_layout_info, None) }
            .map_err(|r| VulkanError::vk("create_ssao_ds_layout", r))?;

        // ---- 5. Descriptor pool + set ----
        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: 2,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_IMAGE,
                descriptor_count: 1,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_ssao_ds_pool", r))?;

        let ds_layouts = [ds_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&ds_layouts);
        let sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_ssao_ds", r))?;
        let desc_set = sets[0];

        // Write descriptors (depth + noise as sampled, output as storage)
        // Depth will be updated each frame (it changes), so write noise + output now.
        let noise_image_info = [vk::DescriptorImageInfo::default()
            .sampler(depth_sampler)
            .image_view(noise_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let out_image_info = [vk::DescriptorImageInfo::default()
            .image_view(out_view)
            .image_layout(vk::ImageLayout::GENERAL)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&noise_image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&out_image_info),
        ];
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }

        // ---- 6. Pipeline layout (push constants: kernel params) ----
        let pc_range = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::COMPUTE,
            offset: 0,
            size: 32, // vec4: (radius, power, kernel_size, pad) + 4×vec4 kernel samples
        }];
        let set_layouts = [ds_layout];
        let pll_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&pc_range);
        let pll = unsafe { d.create_pipeline_layout(&pll_info, None) }
            .map_err(|r| VulkanError::vk("cpl_ssao", r))?;

        // ---- 7. Compute pipeline ----
        if !SSAO_COMPUTE_COMP_SPV.is_empty() {
            let sm = unsafe { mk_sm(d, SSAO_COMPUTE_COMP_SPV)? };
            let ci = vk::ComputePipelineCreateInfo::default()
                .stage(
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::COMPUTE)
                        .module(sm)
                        .name(c"main"),
                )
                .layout(pll);
            let pipes =
                unsafe { d.create_compute_pipelines(vk::PipelineCache::null(), &[ci], None) }
                    .map_err(|(_, r)| VulkanError::vk("cgp_ssao", r))?;
            self.ssao_pipeline = Some(pipes[0]);
            unsafe {
                d.destroy_shader_module(sm, None);
            }
        }

        // ---- Store ----
        self.ssao_depth_sampler = Some(depth_sampler);
        self.ssao_noise_image = Some(noise_image);
        self.ssao_noise_view = Some(noise_view);
        self.ssao_noise_allocation = Some(noise_allocation);
        self.ssao_output_image = Some(out_image);
        self.ssao_output_view = Some(out_view);
        self.ssao_output_allocation = Some(out_allocation);
        self.ssao_desc_set = Some(desc_set);
        self.ssao_desc_pool = Some(pool);
        self.ssao_desc_layout = Some(ds_layout);
        self.ssao_pipeline_layout = Some(pll);

        Ok(())
    }

    /// Update the SSAO descriptor set with the current depth image view.
    pub(crate) fn update_ssao_depth_descriptor(&mut self) {
        let Some(ds) = self.ssao_desc_set else { return };
        let Some(sampler) = self.ssao_depth_sampler else {
            return;
        };
        let Some(depth_view) = self.depth_image_view else {
            return;
        };
        let d = &self.logical_device.device;

        let depth_image_info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(depth_view)
            .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(ds)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&depth_image_info)];
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }
    }

    /// Transition SSAO output to GENERAL layout for compute write access.
    pub(crate) fn barrier_ssao_output_for_compute(&self, fi: usize) {
        let Some(img) = self.ssao_output_image else {
            return;
        };
        let cmd = self.frame_sync[fi].command_buffer;
        let d = &self.logical_device.device;

        let barrier = vk::ImageMemoryBarrier::default()
            .image(img)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::SHADER_READ)
            .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::GENERAL);
        unsafe {
            d.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    /// Transition SSAO output back to SHADER_READ_ONLY_OPTIMAL after compute.
    pub(crate) fn barrier_ssao_output_for_read(&self, fi: usize) {
        let Some(img) = self.ssao_output_image else {
            return;
        };
        let cmd = self.frame_sync[fi].command_buffer;
        let d = &self.logical_device.device;

        let barrier = vk::ImageMemoryBarrier::default()
            .image(img)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        unsafe {
            d.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    /// Destroy all post-processing resources (bloom + SSAO).
    ///
    /// Should be called before shadow/HDR resource destruction to avoid
    /// dangling references.
    pub(crate) fn destroy_post_process_resources(&mut self) {
        let d = &self.logical_device.device;

        // ── Bloom ──────────────────────────────────────────────────────
        for pipe in self.bloom_downsample_pipelines.drain(..) {
            unsafe {
                d.destroy_pipeline(pipe, None);
            }
        }
        for pipe in self.bloom_upsample_pipelines.drain(..) {
            unsafe {
                d.destroy_pipeline(pipe, None);
            }
        }
        if let Some(l) = self.bloom_pipeline_layout.take() {
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(pool) = self.bloom_desc_pool.take() {
            unsafe {
                d.destroy_descriptor_pool(pool, None);
            }
        }
        if let Some(layout) = self.bloom_desc_layout.take() {
            unsafe {
                d.destroy_descriptor_set_layout(layout, None);
            }
        }
        if let Some(s) = self.bloom_sampler.take() {
            unsafe {
                d.destroy_sampler(s, None);
            }
        }
        for iv in self.bloom_image_views.drain(..) {
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        for img in self.bloom_images.drain(..) {
            unsafe {
                d.destroy_image(img, None);
            }
        }
        for mut a in self.bloom_allocations.drain(..) {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                guard.free(&mut a);
            }
        }

        // ── SSAO ───────────────────────────────────────────────────────
        if let Some(p) = self.ssao_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.ssao_pipeline_layout.take() {
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(pool) = self.ssao_desc_pool.take() {
            unsafe {
                d.destroy_descriptor_pool(pool, None);
            }
        }
        if let Some(layout) = self.ssao_desc_layout.take() {
            unsafe {
                d.destroy_descriptor_set_layout(layout, None);
            }
        }
        if let Some(s) = self.ssao_depth_sampler.take() {
            unsafe {
                d.destroy_sampler(s, None);
            }
        }
        if let Some(iv) = self.ssao_noise_view.take() {
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(img) = self.ssao_noise_image.take() {
            unsafe {
                d.destroy_image(img, None);
            }
        }
        if let Some(mut a) = self.ssao_noise_allocation.take() {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                guard.free(&mut a);
            }
        }
        if let Some(iv) = self.ssao_output_view.take() {
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(img) = self.ssao_output_image.take() {
            unsafe {
                d.destroy_image(img, None);
            }
        }
        if let Some(mut a) = self.ssao_output_allocation.take() {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                guard.free(&mut a);
            }
        }
    }

    /// Dispatch the SSAO compute shader.
    ///
    /// Reads depth buffer, writes occlusion factor to the R8 output texture.
    /// No-op when the SSAO pipeline hasn't been initialized.
    pub(crate) fn dispatch_ssao(&self, fi: usize) {
        let Some(pipe) = self.ssao_pipeline else {
            return;
        };
        let Some(pll) = self.ssao_pipeline_layout else {
            return;
        };
        let Some(ds) = self.ssao_desc_set else { return };
        let cmd = self.frame_sync[fi].command_buffer;
        let d = &self.logical_device.device;

        // Half-resolution dispatch
        let out_w = (self.swapchain_extent.width / 2).max(1);
        let out_h = (self.swapchain_extent.height / 2).max(1);
        let gw = (out_w / 16).max(1);
        let gh = (out_h / 16).max(1);

        unsafe {
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipe);
            d.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, pll, 0, &[ds], &[]);
            // Push constants: (radius, power, kernel_size, pad)
            let pc = [SSAO_RADIUS, SSAO_POWER, SSAO_KERNEL_SIZE as f32, 0.0];
            let pc_bytes: &[u8] = std::slice::from_raw_parts(&pc as *const _ as *const u8, 16);
            d.cmd_push_constants(cmd, pll, vk::ShaderStageFlags::COMPUTE, 0, pc_bytes);
            d.cmd_dispatch(cmd, gw, gh, 1);
        }
    }
}

/// Generate a 4×4 noise texture with random rotation vectors.
///
/// Each texel is a random 3D vector on the unit hemisphere, stored as
/// RGBA8_UNORM (x, y, z, 0 → mapped from [-1,1] to [0,255]).
fn generate_ssao_noise() -> Vec<u8> {
    // Deterministic pseudo-random sequence using a simple LCG.
    let mut rng_state: u32 = 0xdeadbeef;
    let mut next_f32 = || -> f32 {
        rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        (rng_state >> 16) as f32 / 65536.0
    };

    let mut pixels = Vec::with_capacity((SSAO_NOISE_DIM * SSAO_NOISE_DIM * 4) as usize);
    for _ in 0..SSAO_NOISE_DIM * SSAO_NOISE_DIM {
        // Random vector on the hemisphere (z > 0)
        let x = next_f32() * 2.0 - 1.0;
        let y = next_f32() * 2.0 - 1.0;
        let z = next_f32(); // [0, 1) → always positive hemisphere
                            // Normalise
        let len = (x * x + y * y + z * z).sqrt();
        let nx = (x / len * 0.5 + 0.5).clamp(0.0, 1.0);
        let ny = (y / len * 0.5 + 0.5).clamp(0.0, 1.0);
        let nz = (z / len * 0.5 + 0.5).clamp(0.0, 1.0);
        pixels.push((nx * 255.0) as u8);
        pixels.push((ny * 255.0) as u8);
        pixels.push((nz * 255.0) as u8);
        pixels.push(0u8);
    }
    pixels
}
