//! VulkanDevice methods used by the GPU resource reload coordinator.
//!
//! These are separated from the pipeline/texture creation code to keep
//! concerns distinct — the reload path creates new resources, atomically
//! swaps them into the device state, and returns the old handles so the
//! coordinator can keep them alive for the required number of frames.

use std::ops::Range;

use ash::vk;
use ash::Device as AshDevice;

use crate::allocator::{Allocation, SharedAllocator};
use crate::error::{VkResult, VulkanError};

use super::{GpuTexture, VulkanDevice};

/// Returned by [`VulkanDevice::replace_shadow_map`] — the old shadow-map
/// resources that should be queued for deferred destruction.
#[allow(clippy::type_complexity)]
type ShadowMapSwapSet = (
    vk::Image,
    vk::ImageView,
    Vec<vk::ImageView>,
    Option<Allocation>,
    Vec<vk::Framebuffer>,
);

/// Color-space interpretation for an RGBA8 sampled texture upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SampledTextureColorSpace {
    Linear,
    Srgb,
}

impl SampledTextureColorSpace {
    const fn vk_format(self) -> vk::Format {
        match self {
            Self::Linear => vk::Format::R8G8B8A8_UNORM,
            Self::Srgb => vk::Format::R8G8B8A8_SRGB,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SampledTextureFilter {
    Nearest,
    Linear,
}

impl SampledTextureFilter {
    const fn vk_filter(self) -> vk::Filter {
        match self {
            Self::Nearest => vk::Filter::NEAREST,
            Self::Linear => vk::Filter::LINEAR,
        }
    }

    const fn vk_mipmap_mode(self) -> vk::SamplerMipmapMode {
        match self {
            Self::Nearest => vk::SamplerMipmapMode::NEAREST,
            Self::Linear => vk::SamplerMipmapMode::LINEAR,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SampledTextureAddressMode {
    Repeat,
    ClampToEdge,
    MirroredRepeat,
}

impl SampledTextureAddressMode {
    const fn vk_address_mode(self) -> vk::SamplerAddressMode {
        match self {
            Self::Repeat => vk::SamplerAddressMode::REPEAT,
            Self::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
            Self::MirroredRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SampledTextureSamplerDescriptor {
    pub(crate) min_filter: SampledTextureFilter,
    pub(crate) mag_filter: SampledTextureFilter,
    pub(crate) mip_filter: SampledTextureFilter,
    pub(crate) address_u: SampledTextureAddressMode,
    pub(crate) address_v: SampledTextureAddressMode,
    pub(crate) address_w: SampledTextureAddressMode,
}

impl SampledTextureSamplerDescriptor {
    pub(crate) const fn linear_repeat() -> Self {
        Self {
            min_filter: SampledTextureFilter::Linear,
            mag_filter: SampledTextureFilter::Linear,
            mip_filter: SampledTextureFilter::Linear,
            address_u: SampledTextureAddressMode::Repeat,
            address_v: SampledTextureAddressMode::Repeat,
            address_w: SampledTextureAddressMode::Repeat,
        }
    }
}

impl Default for SampledTextureSamplerDescriptor {
    fn default() -> Self {
        Self::linear_repeat()
    }
}

/// Complete description for a precomputed RGBA8 mip chain.
pub(crate) struct SampledTextureDescriptor<'a> {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) mip_count: u8,
    pub(crate) data: &'a [u8],
    pub(crate) color_space: SampledTextureColorSpace,
    pub(crate) sampler: SampledTextureSamplerDescriptor,
}

impl<'a> SampledTextureDescriptor<'a> {
    pub(crate) const fn rgba8(
        width: u32,
        height: u32,
        mip_count: u8,
        data: &'a [u8],
        color_space: SampledTextureColorSpace,
        sampler: SampledTextureSamplerDescriptor,
    ) -> Self {
        Self {
            width,
            height,
            mip_count,
            data,
            color_space,
            sampler,
        }
    }

    pub(crate) const fn rgba8_unorm(
        width: u32,
        height: u32,
        mip_count: u8,
        data: &'a [u8],
    ) -> Self {
        Self::rgba8(
            width,
            height,
            mip_count,
            data,
            SampledTextureColorSpace::Linear,
            SampledTextureSamplerDescriptor::linear_repeat(),
        )
    }

    #[allow(dead_code)]
    pub(crate) const fn rgba8_srgb(width: u32, height: u32, mip_count: u8, data: &'a [u8]) -> Self {
        Self::rgba8(
            width,
            height,
            mip_count,
            data,
            SampledTextureColorSpace::Srgb,
            SampledTextureSamplerDescriptor::linear_repeat(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UploadMip {
    level: u32,
    width: u32,
    height: u32,
    byte_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TextureUploadPlan {
    mips: Vec<UploadMip>,
    total_bytes: usize,
}

fn invalid_texture_data(reason: impl Into<String>) -> VulkanError {
    VulkanError::Loader(format!("invalid sampled texture upload: {}", reason.into()))
}

fn build_texture_upload_plan(
    width: u32,
    height: u32,
    mip_count: u8,
    data_len: usize,
) -> VkResult<TextureUploadPlan> {
    if width == 0 || height == 0 {
        return Err(invalid_texture_data(format!(
            "width and height must be non-zero (got {width}x{height})"
        )));
    }
    if mip_count == 0 {
        return Err(invalid_texture_data("mip_count must be at least 1"));
    }

    let max_dimension = width.max(height);
    let max_mip_count = u8::try_from(u32::BITS - max_dimension.leading_zeros())
        .map_err(|_| invalid_texture_data("maximum mip count is not representable"))?;
    if mip_count > max_mip_count {
        return Err(invalid_texture_data(format!(
            "mip_count {mip_count} exceeds the {max_mip_count} levels available for {width}x{height}"
        )));
    }

    let mut mips = Vec::with_capacity(mip_count as usize);
    let mut offset = 0usize;
    for level in 0..u32::from(mip_count) {
        let mip_width = (width >> level).max(1);
        let mip_height = (height >> level).max(1);
        let byte_len_u64 = u64::from(mip_width)
            .checked_mul(u64::from(mip_height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| invalid_texture_data(format!("mip {level} RGBA8 byte size overflow")))?;
        let byte_len = usize::try_from(byte_len_u64).map_err(|_| {
            invalid_texture_data(format!(
                "mip {level} byte size {byte_len_u64} is not representable on this platform"
            ))
        })?;
        let end = offset.checked_add(byte_len).ok_or_else(|| {
            invalid_texture_data(format!("mip {level} byte range overflows address space"))
        })?;
        mips.push(UploadMip {
            level,
            width: mip_width,
            height: mip_height,
            byte_range: offset..end,
        });
        offset = end;
    }

    if data_len != offset {
        return Err(invalid_texture_data(format!(
            "RGBA8 mip chain requires exactly {offset} bytes, got {data_len}"
        )));
    }

    Ok(TextureUploadPlan {
        mips,
        total_bytes: offset,
    })
}

fn free_allocation(allocator: &SharedAllocator, allocation: &mut Option<Allocation>, label: &str) {
    let Some(mut allocation) = allocation.take() else {
        return;
    };
    match allocator.lock() {
        Ok(mut guard) => guard.free(&mut allocation),
        Err(poisoned) => {
            tracing::error!(
                target: "vulkan::reload",
                resource = label,
                "allocator mutex was poisoned during upload cleanup"
            );
            poisoned.into_inner().free(&mut allocation);
        }
    }
}

/// Owns every partially-created object until a complete texture is returned.
struct PendingTextureUpload<'a> {
    device: &'a AshDevice,
    allocator: SharedAllocator,
    queue: vk::Queue,
    image: vk::Image,
    image_allocation: Option<Allocation>,
    view: vk::ImageView,
    sampler: vk::Sampler,
    staging_buffer: vk::Buffer,
    staging_allocation: Option<Allocation>,
    command_pool: vk::CommandPool,
    fence: vk::Fence,
    submitted: bool,
}

impl<'a> PendingTextureUpload<'a> {
    fn new(device: &'a AshDevice, allocator: SharedAllocator, queue: vk::Queue) -> Self {
        Self {
            device,
            allocator,
            queue,
            image: vk::Image::null(),
            image_allocation: None,
            view: vk::ImageView::null(),
            sampler: vk::Sampler::null(),
            staging_buffer: vk::Buffer::null(),
            staging_allocation: None,
            command_pool: vk::CommandPool::null(),
            fence: vk::Fence::null(),
            submitted: false,
        }
    }

    fn destroy_transient_resources(&mut self) {
        // SAFETY: every non-null handle is exclusively owned by this guard. A
        // command pool implicitly frees its command buffers.
        unsafe {
            if self.fence != vk::Fence::null() {
                self.device.destroy_fence(self.fence, None);
                self.fence = vk::Fence::null();
            }
            if self.command_pool != vk::CommandPool::null() {
                self.device.destroy_command_pool(self.command_pool, None);
                self.command_pool = vk::CommandPool::null();
            }
            if self.staging_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.staging_buffer, None);
                self.staging_buffer = vk::Buffer::null();
            }
        }
        free_allocation(
            &self.allocator,
            &mut self.staging_allocation,
            "texture staging buffer",
        );
    }

    fn finish(mut self) -> VkResult<GpuTexture> {
        self.destroy_transient_resources();
        let allocation = self.image_allocation.take().ok_or_else(|| {
            VulkanError::Loader("sampled texture upload lost its image allocation".into())
        })?;
        let texture = GpuTexture {
            image: self.image,
            view: self.view,
            allocation,
            sampler: self.sampler,
        };
        self.image = vk::Image::null();
        self.view = vk::ImageView::null();
        self.sampler = vk::Sampler::null();
        Ok(texture)
    }
}

impl Drop for PendingTextureUpload<'_> {
    fn drop(&mut self) {
        if self.submitted {
            // A failed fence wait does not prove the submitted command buffer
            // is idle. Best-effort queue idle keeps cleanup from racing it.
            // SAFETY: `queue` belongs to `device` and remains alive for the
            // guard's entire lifetime.
            if let Err(result) = unsafe { self.device.queue_wait_idle(self.queue) } {
                tracing::error!(
                    target: "vulkan::reload",
                    ?result,
                    "queue did not become idle while cleaning up a failed texture upload"
                );
            }
            self.submitted = false;
        }

        self.destroy_transient_resources();
        // SAFETY: non-null handles are exclusively owned by this guard and no
        // submitted command buffer can still reference them after the wait.
        unsafe {
            if self.sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.sampler, None);
                self.sampler = vk::Sampler::null();
            }
            if self.view != vk::ImageView::null() {
                self.device.destroy_image_view(self.view, None);
                self.view = vk::ImageView::null();
            }
            if self.image != vk::Image::null() {
                self.device.destroy_image(self.image, None);
                self.image = vk::Image::null();
            }
        }
        free_allocation(
            &self.allocator,
            &mut self.image_allocation,
            "sampled texture image",
        );
    }
}

impl VulkanDevice {
    // ------------------------------------------------------------------
    // Texture helpers
    // ------------------------------------------------------------------

    /// Create a complete sampled texture resource from an exact RGBA8 mip
    /// chain. No Vulkan object escapes until image, view, allocation, sampler,
    /// upload submission, and synchronization have all succeeded.
    pub(crate) fn create_sampled_texture_resource(
        &self,
        descriptor: SampledTextureDescriptor<'_>,
    ) -> VkResult<GpuTexture> {
        let plan = build_texture_upload_plan(
            descriptor.width,
            descriptor.height,
            descriptor.mip_count,
            descriptor.data.len(),
        )?;
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();
        let mut pending = PendingTextureUpload::new(d, allocator, self.logical_device.queue);
        let format = descriptor.color_space.vk_format();

        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: descriptor.width,
                height: descriptor.height,
                depth: 1,
            })
            .mip_levels(u32::from(descriptor.mip_count))
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is valid and the descriptor was validated above.
        pending.image = unsafe { d.create_image(&image_info, None) }
            .map_err(|result| VulkanError::vk("create_image (reload)", result))?;

        // SAFETY: `pending.image` was just created by this device.
        let image_requirements = unsafe { d.get_image_memory_requirements(pending.image) };
        let image_allocation = {
            let mut guard = pending
                .allocator
                .lock()
                .map_err(|error| VulkanError::Loader(format!("allocator lock: {error}")))?;
            guard
                .allocate(&crate::allocator::AllocationCreateDesc {
                    name: "reload-texture",
                    requirements: image_requirements,
                    location: crate::allocator::MemoryLocation::GpuOnly,
                    linear: false,
                    allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
                })
                .map_err(VulkanError::Allocation)?
        };
        pending.image_allocation = Some(image_allocation);
        let image_allocation = pending.image_allocation.as_ref().ok_or_else(|| {
            VulkanError::Loader("sampled texture image allocation disappeared".into())
        })?;
        // SAFETY: the allocation satisfies this image's memory requirements.
        unsafe {
            d.bind_image_memory(
                pending.image,
                image_allocation.memory(),
                image_allocation.offset(),
            )
        }
        .map_err(|result| VulkanError::vk("bind_image (reload)", result))?;

        let staging_size = vk::DeviceSize::try_from(plan.total_bytes).map_err(|_| {
            invalid_texture_data(format!(
                "staging size {} is not representable by Vulkan",
                plan.total_bytes
            ))
        })?;
        let staging_info = vk::BufferCreateInfo::default()
            .size(staging_size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is valid and `staging_size` is non-zero after validation.
        pending.staging_buffer = unsafe { d.create_buffer(&staging_info, None) }
            .map_err(|result| VulkanError::vk("create_staging_buf (reload)", result))?;
        // SAFETY: the staging buffer was just created by this device.
        let staging_requirements =
            unsafe { d.get_buffer_memory_requirements(pending.staging_buffer) };
        let staging_allocation = {
            let mut guard = pending
                .allocator
                .lock()
                .map_err(|error| VulkanError::Loader(format!("allocator lock: {error}")))?;
            guard
                .allocate(&crate::allocator::AllocationCreateDesc {
                    name: "reload-staging",
                    requirements: staging_requirements,
                    location: crate::allocator::MemoryLocation::CpuToGpu,
                    linear: true,
                    allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
                })
                .map_err(VulkanError::Allocation)?
        };
        pending.staging_allocation = Some(staging_allocation);
        let staging_allocation = pending.staging_allocation.as_ref().ok_or_else(|| {
            VulkanError::Loader("sampled texture staging allocation disappeared".into())
        })?;
        // SAFETY: the allocation satisfies this staging buffer's requirements.
        unsafe {
            d.bind_buffer_memory(
                pending.staging_buffer,
                staging_allocation.memory(),
                staging_allocation.offset(),
            )
        }
        .map_err(|result| VulkanError::vk("bind_staging (reload)", result))?;

        let mapped = pending
            .staging_allocation
            .as_mut()
            .and_then(Allocation::mapped_slice_mut)
            .ok_or(VulkanError::MemoryNotMapped("reload-staging"))?;
        if mapped.len() < plan.total_bytes {
            return Err(VulkanError::Allocation(format!(
                "reload staging allocation exposes {} bytes, expected {}",
                mapped.len(),
                plan.total_bytes
            )));
        }
        mapped[..plan.total_bytes].copy_from_slice(descriptor.data);

        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.logical_device.queue_family_index)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        // SAFETY: the queue-family index belongs to the logical device.
        pending.command_pool = unsafe { d.create_command_pool(&pool_info, None) }
            .map_err(|result| VulkanError::vk("create_cp (reload)", result))?;
        let command_buffers = unsafe {
            d.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pending.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(|result| VulkanError::vk("alloc_cb (reload)", result))?;
        let command_buffer = command_buffers.first().copied().ok_or_else(|| {
            VulkanError::Loader("Vulkan returned no reload command buffer".into())
        })?;

        let copy_regions = plan
            .mips
            .iter()
            .map(|mip| {
                Ok(vk::BufferImageCopy::default()
                    .buffer_offset(vk::DeviceSize::try_from(mip.byte_range.start).map_err(
                        |_| invalid_texture_data("mip staging offset is not representable"),
                    )?)
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: mip.level,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                    .image_extent(vk::Extent3D {
                        width: mip.width,
                        height: mip.height,
                        depth: 1,
                    }))
            })
            .collect::<VkResult<Vec<_>>>()?;
        let pre_barriers = plan
            .mips
            .iter()
            .map(|mip| {
                vk::ImageMemoryBarrier::default()
                    .image(pending.image)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: mip.level,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .src_access_mask(vk::AccessFlags::empty())
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            })
            .collect::<Vec<_>>();
        let post_barriers = plan
            .mips
            .iter()
            .map(|mip| {
                vk::ImageMemoryBarrier::default()
                    .image(pending.image)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: mip.level,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect::<Vec<_>>();

        // SAFETY: all handles are owned by the pending guard, and the command
        // buffer is fresh from its transient pool.
        unsafe {
            d.begin_command_buffer(
                command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(|result| VulkanError::vk("begin_cb (reload)", result))?;
            d.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &pre_barriers,
            );
            d.cmd_copy_buffer_to_image(
                command_buffer,
                pending.staging_buffer,
                pending.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &copy_regions,
            );
            d.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &post_barriers,
            );
            d.end_command_buffer(command_buffer)
                .map_err(|result| VulkanError::vk("end_cb (reload)", result))?;
        }

        // SAFETY: default fence creation is valid for this device.
        pending.fence = unsafe { d.create_fence(&vk::FenceCreateInfo::default(), None) }
            .map_err(|result| VulkanError::vk("create_fence (reload)", result))?;
        let command_buffers = [command_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        // SAFETY: command recording is complete, and all submitted resources
        // remain owned by `pending` until the fence is signalled.
        unsafe {
            d.queue_submit(self.logical_device.queue, &[submit_info], pending.fence)
                .map_err(|result| VulkanError::vk("queue_submit (reload)", result))?;
        }
        pending.submitted = true;
        // SAFETY: the fence belongs to the submitted upload.
        unsafe {
            d.wait_for_fences(&[pending.fence], true, u64::MAX)
                .map_err(|result| VulkanError::vk("wait_fences (reload)", result))?;
        }
        pending.submitted = false;

        let view_info = vk::ImageViewCreateInfo::default()
            .image(pending.image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: u32::from(descriptor.mip_count),
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: the upload is complete and the view covers valid mip levels.
        pending.view = unsafe { d.create_image_view(&view_info, None) }
            .map_err(|result| VulkanError::vk("create_image_view (reload)", result))?;

        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(descriptor.sampler.mag_filter.vk_filter())
            .min_filter(descriptor.sampler.min_filter.vk_filter())
            .mipmap_mode(descriptor.sampler.mip_filter.vk_mipmap_mode())
            .address_mode_u(descriptor.sampler.address_u.vk_address_mode())
            .address_mode_v(descriptor.sampler.address_v.vk_address_mode())
            .address_mode_w(descriptor.sampler.address_w.vk_address_mode())
            .min_lod(0.0)
            .max_lod(f32::from(descriptor.mip_count.saturating_sub(1)));
        // SAFETY: the sampler descriptor uses core Vulkan values only.
        pending.sampler = unsafe { d.create_sampler(&sampler_info, None) }
            .map_err(|result| VulkanError::vk("create_sampler (reload)", result))?;

        pending.finish()
    }

    // ------------------------------------------------------------------
    // Shadow-map swap
    // ------------------------------------------------------------------

    /// Atomically replace the CSM shadow-map texture (3-layer array).
    ///
    /// Returns the **old** resources so the caller can queue them for deferred
    /// destruction.
    #[allow(dead_code)]
    pub(crate) fn replace_shadow_map(
        &mut self,
        new_image: vk::Image,
        new_layered_view: vk::ImageView,
        new_layer_views: Vec<vk::ImageView>,
        new_allocation: Allocation,
    ) -> Option<ShadowMapSwapSet> {
        let old_image = self.shadow_map.take();
        let old_layered_view = self.shadow_map_view.take();
        let old_layer_views = std::mem::take(&mut self.shadow_layer_views);
        let old_alloc = self.shadow_allocation.take();
        let old_fbs = std::mem::take(&mut self.shadow_fbs);

        self.shadow_map = Some(new_image);
        self.shadow_map_view = Some(new_layered_view);
        self.shadow_layer_views = new_layer_views;
        self.shadow_allocation = Some(new_allocation);

        // Recreate cascade framebuffers from the new layer views.
        if let Some(rp) = self.shadow_rp {
            let mut new_fbs = Vec::with_capacity(self.shadow_layer_views.len());
            for &lv in &self.shadow_layer_views {
                // SAFETY: device is valid; framebuffer info references valid
                // render pass and layer image view.
                let fb = unsafe {
                    self.logical_device.device.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(rp)
                            .attachments(&[lv])
                            .width(2048)
                            .height(2048)
                            .layers(1),
                        None,
                    )
                }
                .ok()?;
                new_fbs.push(fb);
            }
            self.shadow_fbs = new_fbs;
        }

        // Update the shadow descriptor set to point at the new layered view.
        if let (Some(ds), Some(sampler)) = (self.shadow_desc_set, self.shadow_sampler) {
            let image_info = [vk::DescriptorImageInfo::default()
                .sampler(sampler)
                .image_view(self.shadow_map_view.unwrap_or(vk::ImageView::null()))
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
            let writes = [vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info)];
            // SAFETY: device is valid; descriptor set and sampler are valid.
            unsafe {
                self.logical_device
                    .device
                    .update_descriptor_sets(&writes, &[]);
            }
        }

        if let (Some(img), Some(vw)) = (old_image, old_layered_view) {
            Some((img, vw, old_layer_views, old_alloc, old_fbs))
        } else {
            None
        }
    }

    // ------------------------------------------------------------------
    // Pipeline recreation
    // ------------------------------------------------------------------

    /// Recreate the MVP triangle pipeline using new SPIR-V.
    ///
    /// Returns the **old** `(pipeline, pipeline_layout)` for deferred
    /// destruction.  On failure the old pipeline is kept.
    ///
    /// The SPIR-V byte slices are leaked into `'static` so they can be
    /// stored on the device (which expects `&'static [u8]`).  This is
    /// acceptable for hot-reload (small, infrequent allocations).
    pub(crate) fn recreate_mvp_pipeline(
        &mut self,
        vert_spirv: &[u8],
        frag_spirv: &[u8],
    ) -> VkResult<(vk::Pipeline, vk::PipelineLayout)> {
        let old_pipeline = self
            .mvp_pipeline
            .ok_or(VulkanError::Loader("MVP pipeline not created yet".into()))?;
        let old_layout = self.mvp_pipeline_layout.ok_or(VulkanError::Loader(
            "MVP pipeline layout not created".into(),
        ))?;

        let saved_vert = self.mvp_vert_spv.replace(vert_spirv.to_vec());
        let saved_frag = self.mvp_frag_spv.replace(frag_spirv.to_vec());

        // Rebuild (this reads self.mvp_vert_spv / mvp_frag_spv).
        match self.build_mvp() {
            Ok(()) => Ok((old_pipeline, old_layout)),
            Err(e) => {
                self.mvp_vert_spv = saved_vert;
                self.mvp_frag_spv = saved_frag;
                self.mvp_pipeline = Some(old_pipeline);
                self.mvp_pipeline_layout = Some(old_layout);
                Err(e)
            }
        }
    }

    /// Recreate the model forward pipeline using new SPIR-V.
    ///
    /// Returns the **old** `(pipeline, pipeline_layout)` for deferred
    /// destruction.  On failure the old pipeline is kept.
    pub(crate) fn recreate_model_pipeline(
        &mut self,
        vert_spirv: &[u8],
        frag_spirv: &[u8],
    ) -> VkResult<(vk::Pipeline, vk::PipelineLayout)> {
        let old_pipeline = self
            .model_pipeline
            .ok_or(VulkanError::Loader("model pipeline not created yet".into()))?;
        let old_layout = self.model_pipeline_layout.ok_or(VulkanError::Loader(
            "model pipeline layout not created".into(),
        ))?;

        // Note: model pipeline reuses the same mvp_vert_spv/mvp_frag_spv
        // fields (both MVP and model share the embedded forward shaders).
        let saved_vert = self.mvp_vert_spv.replace(vert_spirv.to_vec());
        let saved_frag = self.mvp_frag_spv.replace(frag_spirv.to_vec());

        match self.build_model_pipeline() {
            Ok(()) => Ok((old_pipeline, old_layout)),
            Err(e) => {
                self.mvp_vert_spv = saved_vert;
                self.mvp_frag_spv = saved_frag;
                self.model_pipeline = Some(old_pipeline);
                self.model_pipeline_layout = Some(old_layout);
                Err(e)
            }
        }
    }

    /// Recreate the shadow-mapping pipeline using new SPIR-V.
    ///
    /// Returns the **old** `(pipeline, pipeline_layout)` for deferred
    /// destruction.  On failure the old pipeline is kept.
    pub(crate) fn recreate_shadow_pipeline(
        &mut self,
        vert_spirv: &[u8],
        frag_spirv: &[u8],
    ) -> VkResult<(vk::Pipeline, vk::PipelineLayout)> {
        let old_pipeline = self.shadow_pipeline.ok_or(VulkanError::Loader(
            "shadow pipeline not created yet".into(),
        ))?;
        let old_layout = self.shadow_pipeline_layout.ok_or(VulkanError::Loader(
            "shadow pipeline layout not created".into(),
        ))?;

        // ── Create new shader modules ──────────────────────────────────
        let d = &self.logical_device.device;
        // SAFETY: `d` is a valid AshDevice; bytecode is valid SPIR-V.
        let vm = unsafe { super::mk_sm(d, vert_spirv)? };
        let fm = unsafe { super::mk_sm(d, frag_spirv)? };

        // ── Reuse the existing render pass for the shadow pipeline ─────
        let rp = self
            .shadow_rp
            .ok_or(VulkanError::Loader("shadow render pass not found".into()))?;

        // ── Create new pipeline layout (same as old) ───────────────────
        let pc_range = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX,
            offset: 0,
            size: 64,
        }];
        let pll_info = vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&pc_range);
        // SAFETY: `d` is a valid AshDevice.
        let new_layout = unsafe { d.create_pipeline_layout(&pll_info, None) }
            .map_err(|r| VulkanError::vk("cpl_shadow_reload", r))?;

        // ── Build the new pipeline ─────────────────────────────────────
        let main = c"main";
        let sr = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vm)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fm)
                .name(main),
        ];
        let stride = 32u32;
        let vb = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(stride)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let va = [vk::VertexInputAttributeDescription {
            location: 0,
            binding: 0,
            format: vk::Format::R32G32B32_SFLOAT,
            offset: 0,
        }];
        let vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vb)
            .vertex_attribute_descriptions(&va);
        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let vs = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0)
            .depth_bias_enable(true)
            .depth_bias_constant_factor(1.5)
            .depth_bias_slope_factor(1.5);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let cba: [vk::PipelineColorBlendAttachmentState; 0] = [];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);
        let ds_state = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);

        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .depth_stencil_state(&ds_state)
            .color_blend_state(&cb)
            .dynamic_state(&ds)
            .layout(new_layout)
            .render_pass(rp)
            .subpass(0);

        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid
        // graphics pipeline; `vk::PipelineCache::null()` is allowed.
        let new_pipeline =
            unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
                .map_err(|(_, r)| {
                    // Destroy the new layout since we failed.
                    unsafe {
                        d.destroy_pipeline_layout(new_layout, None);
                    }
                    // SAFETY: shader modules were created above.
                    unsafe {
                        d.destroy_shader_module(vm, None);
                        d.destroy_shader_module(fm, None);
                    }
                    VulkanError::vk("cgp_shadow_reload", r)
                })?[0];

        // SAFETY: shader modules are no longer needed after pipeline creation.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }

        // Store the new pipeline/layout on the device.
        self.shadow_pipeline = Some(new_pipeline);
        self.shadow_pipeline_layout = Some(new_layout);

        Ok((old_pipeline, old_layout))
    }

    // ------------------------------------------------------------------
    // Frame-boundary reload processing
    // ------------------------------------------------------------------

    /// Process pending reloads at the start of a frame (before acquire).
    ///
    /// Calls [`GpuReloadCoordinator::apply_next`] for one pending reload, if
    /// any.
    pub fn process_reloads(
        &mut self,
        coordinator: &mut crate::reload::GpuReloadCoordinator,
    ) -> Result<bool, VulkanError> {
        coordinator.apply_next(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba8_mip_plan_has_exact_ranges_and_dimensions() {
        let plan = build_texture_upload_plan(4, 4, 3, 84).unwrap();
        assert_eq!(plan.total_bytes, 84);
        assert_eq!(
            plan.mips,
            vec![
                UploadMip {
                    level: 0,
                    width: 4,
                    height: 4,
                    byte_range: 0..64,
                },
                UploadMip {
                    level: 1,
                    width: 2,
                    height: 2,
                    byte_range: 64..80,
                },
                UploadMip {
                    level: 2,
                    width: 1,
                    height: 1,
                    byte_range: 80..84,
                },
            ]
        );
    }

    #[test]
    fn rgba8_mip_plan_rejects_truncated_or_trailing_data() {
        assert!(build_texture_upload_plan(4, 4, 3, 83).is_err());
        assert!(build_texture_upload_plan(4, 4, 3, 85).is_err());
    }

    #[test]
    fn rgba8_mip_plan_rejects_invalid_dimensions_and_counts() {
        assert!(build_texture_upload_plan(0, 1, 1, 0).is_err());
        assert!(build_texture_upload_plan(1, 0, 1, 0).is_err());
        assert!(build_texture_upload_plan(1, 1, 0, 0).is_err());
        assert!(build_texture_upload_plan(4, 4, 4, 88).is_err());
    }

    #[test]
    fn sampled_texture_color_space_selects_matching_vulkan_format() {
        assert_eq!(
            SampledTextureColorSpace::Linear.vk_format(),
            vk::Format::R8G8B8A8_UNORM
        );
        assert_eq!(
            SampledTextureColorSpace::Srgb.vk_format(),
            vk::Format::R8G8B8A8_SRGB
        );
    }

    #[test]
    fn sampled_texture_sampler_state_maps_to_vulkan_values() {
        assert_eq!(
            SampledTextureFilter::Nearest.vk_filter(),
            vk::Filter::NEAREST
        );
        assert_eq!(
            SampledTextureFilter::Linear.vk_mipmap_mode(),
            vk::SamplerMipmapMode::LINEAR
        );
        assert_eq!(
            SampledTextureAddressMode::Repeat.vk_address_mode(),
            vk::SamplerAddressMode::REPEAT
        );
        assert_eq!(
            SampledTextureAddressMode::ClampToEdge.vk_address_mode(),
            vk::SamplerAddressMode::CLAMP_TO_EDGE
        );
        assert_eq!(
            SampledTextureAddressMode::MirroredRepeat.vk_address_mode(),
            vk::SamplerAddressMode::MIRRORED_REPEAT
        );
    }
}
