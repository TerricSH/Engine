//! Environment cubemap for IBL (Phase 2.3).
//!
//! The renderer currently builds a small procedural sky/ground cubemap for its
//! built-in IBL source. This gives the IBL shader a deterministic, directional
//! input instead of the old one-pixel gray placeholder while keeping startup
//! independent from external assets.

use ash::{vk, Device as AshDevice};

use crate::allocator::{Allocation, SharedAllocator};
use crate::error::{VkResult, VulkanError};

use super::VulkanDevice;

const ENV_FACE_SIZE: u32 = 16;
const ENV_FACE_COUNT: u32 = 6;
const ENV_PIXEL_SIZE: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EnvironmentUploadPlan {
    face_bytes: u64,
    total_bytes: u64,
    face_offsets: [u64; ENV_FACE_COUNT as usize],
}

fn environment_upload_plan(face_size: u32) -> Option<EnvironmentUploadPlan> {
    if face_size == 0 {
        return None;
    }
    let face_bytes = u64::from(face_size)
        .checked_mul(u64::from(face_size))?
        .checked_mul(u64::from(ENV_PIXEL_SIZE))?;
    let total_bytes = face_bytes.checked_mul(u64::from(ENV_FACE_COUNT))?;
    let mut face_offsets = [0; ENV_FACE_COUNT as usize];
    for (face, offset) in face_offsets.iter_mut().enumerate() {
        *offset = u64::try_from(face).ok()?.checked_mul(face_bytes)?;
    }
    Some(EnvironmentUploadPlan {
        face_bytes,
        total_bytes,
        face_offsets,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PendingResourcePresence {
    image: bool,
    image_allocation: bool,
    image_view: bool,
    sampler: bool,
    staging_buffer: bool,
    staging_allocation: bool,
    command_pool: bool,
    fence: bool,
    submitted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingResourceState {
    Empty,
    Partial,
    UploadInFlight,
    ReadyToCommit,
}

fn classify_pending_resources(presence: PendingResourcePresence) -> PendingResourceState {
    if presence.submitted {
        return PendingResourceState::UploadInFlight;
    }
    let permanent = [
        presence.image,
        presence.image_allocation,
        presence.image_view,
        presence.sampler,
    ];
    let transient = [
        presence.staging_buffer,
        presence.staging_allocation,
        presence.command_pool,
        presence.fence,
    ];
    if permanent.iter().all(|owned| !owned) && transient.iter().all(|owned| !owned) {
        PendingResourceState::Empty
    } else if permanent.iter().all(|owned| *owned) && transient.iter().all(|owned| !owned) {
        PendingResourceState::ReadyToCommit
    } else {
        PendingResourceState::Partial
    }
}

fn free_allocation(
    device: &AshDevice,
    allocator: &SharedAllocator,
    allocation: &mut Option<Allocation>,
    label: &str,
) {
    let Some(mut allocation) = allocation.take() else {
        return;
    };
    if allocation.mapped_slice_mut().is_some() {
        // SAFETY: this guard exclusively owns the allocation, so no mapped
        // references can outlive this call.
        unsafe {
            device.unmap_memory(allocation.memory());
        }
    }
    match allocator.lock() {
        Ok(mut guard) => guard.free(&mut allocation),
        Err(poisoned) => {
            tracing::error!(
                target: "vulkan::env",
                resource = label,
                "allocator mutex was poisoned during environment cleanup"
            );
            poisoned.into_inner().free(&mut allocation);
        }
    }
}

struct EnvironmentResources {
    image: vk::Image,
    allocation: Allocation,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
}

/// Owns all partially-created environment resources until they are committed
/// to `VulkanDevice`. Command buffers are owned transitively by `command_pool`.
struct PendingEnvironment {
    device: AshDevice,
    allocator: SharedAllocator,
    queue: vk::Queue,
    image: vk::Image,
    image_allocation: Option<Allocation>,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
    staging_buffer: vk::Buffer,
    staging_allocation: Option<Allocation>,
    command_pool: vk::CommandPool,
    fence: vk::Fence,
    submitted: bool,
}

impl PendingEnvironment {
    fn new(device: AshDevice, allocator: SharedAllocator, queue: vk::Queue) -> Self {
        Self {
            device,
            allocator,
            queue,
            image: vk::Image::null(),
            image_allocation: None,
            image_view: vk::ImageView::null(),
            sampler: vk::Sampler::null(),
            staging_buffer: vk::Buffer::null(),
            staging_allocation: None,
            command_pool: vk::CommandPool::null(),
            fence: vk::Fence::null(),
            submitted: false,
        }
    }

    fn presence(&self) -> PendingResourcePresence {
        PendingResourcePresence {
            image: self.image != vk::Image::null(),
            image_allocation: self.image_allocation.is_some(),
            image_view: self.image_view != vk::ImageView::null(),
            sampler: self.sampler != vk::Sampler::null(),
            staging_buffer: self.staging_buffer != vk::Buffer::null(),
            staging_allocation: self.staging_allocation.is_some(),
            command_pool: self.command_pool != vk::CommandPool::null(),
            fence: self.fence != vk::Fence::null(),
            submitted: self.submitted,
        }
    }

    fn destroy_transient_resources(&mut self) {
        // SAFETY: every non-null handle is exclusively owned by this guard. A
        // command pool implicitly releases its command buffers.
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
            &self.device,
            &self.allocator,
            &mut self.staging_allocation,
            "environment staging buffer",
        );
    }

    fn commit(mut self) -> VkResult<EnvironmentResources> {
        self.destroy_transient_resources();
        if classify_pending_resources(self.presence()) != PendingResourceState::ReadyToCommit {
            return Err(VulkanError::Loader(
                "environment resources were incomplete at commit".into(),
            ));
        }
        let allocation = self.image_allocation.take().ok_or_else(|| {
            VulkanError::Loader("environment image allocation disappeared at commit".into())
        })?;
        let resources = EnvironmentResources {
            image: self.image,
            allocation,
            image_view: self.image_view,
            sampler: self.sampler,
        };
        self.image = vk::Image::null();
        self.image_view = vk::ImageView::null();
        self.sampler = vk::Sampler::null();
        Ok(resources)
    }
}

impl Drop for PendingEnvironment {
    fn drop(&mut self) {
        if self.submitted {
            // A failed fence wait does not prove that submitted resources are
            // idle. Best-effort queue idle prevents rollback from racing the
            // transfer command buffer.
            // SAFETY: `queue` belongs to `device` for the guard's lifetime.
            if let Err(result) = unsafe { self.device.queue_wait_idle(self.queue) } {
                tracing::error!(
                    target: "vulkan::env",
                    ?result,
                    "queue did not become idle during failed environment upload cleanup"
                );
            }
            self.submitted = false;
        }

        self.destroy_transient_resources();
        // SAFETY: all non-null handles are exclusively owned by this guard and
        // submitted work has been waited before reaching this block.
        unsafe {
            if self.sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.sampler, None);
                self.sampler = vk::Sampler::null();
            }
            if self.image_view != vk::ImageView::null() {
                self.device.destroy_image_view(self.image_view, None);
                self.image_view = vk::ImageView::null();
            }
            if self.image != vk::Image::null() {
                self.device.destroy_image(self.image, None);
                self.image = vk::Image::null();
            }
        }
        free_allocation(
            &self.device,
            &self.allocator,
            &mut self.image_allocation,
            "environment cubemap image",
        );
    }
}

fn procedural_environment_rgba8(face_size: u32) -> Vec<u8> {
    let pixels_per_face = face_size as usize * face_size as usize;
    let mut pixels = Vec::with_capacity(pixels_per_face * ENV_FACE_COUNT as usize * 4);
    let sun = normalize3([0.35, 0.82, 0.45]);

    for face in 0..ENV_FACE_COUNT {
        for y in 0..face_size {
            for x in 0..face_size {
                let u = 2.0 * (x as f32 + 0.5) / face_size as f32 - 1.0;
                let v = 2.0 * (y as f32 + 0.5) / face_size as f32 - 1.0;
                let direction = normalize3(match face {
                    0 => [1.0, -v, -u],
                    1 => [-1.0, -v, u],
                    2 => [u, 1.0, v],
                    3 => [u, -1.0, -v],
                    4 => [u, -v, 1.0],
                    _ => [-u, -v, -1.0],
                });

                let horizon = [0.30, 0.42, 0.58];
                let zenith = [0.055, 0.14, 0.32];
                let ground = [0.10, 0.085, 0.065];
                let up = direction[1].clamp(-1.0, 1.0);
                let mut color = if up >= 0.0 {
                    mix3(horizon, zenith, up.sqrt())
                } else {
                    mix3(horizon, ground, (-up).sqrt())
                };

                let sun_amount = dot3(direction, sun).max(0.0).powf(96.0);
                color[0] += 1.4 * sun_amount;
                color[1] += 1.15 * sun_amount;
                color[2] += 0.75 * sun_amount;
                pixels.extend(color.map(linear_to_unorm8));
                pixels.push(255);
            }
        }
    }
    pixels
}

fn normalize3(value: [f32; 3]) -> [f32; 3] {
    let length = dot3(value, value).sqrt();
    [value[0] / length, value[1] / length, value[2] / length]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn mix3(a: [f32; 3], b: [f32; 3], amount: f32) -> [f32; 3] {
    let t = amount.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn linear_to_unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl VulkanDevice {
    /// Create the built-in procedural environment cubemap.
    ///
    /// Idempotent: returns `Ok(())` if the cubemap already exists. Must be
    /// called after [`ensure_shadow`] so that the descriptor set (set=1)
    /// is available for binding=1 to be updated.
    pub(crate) fn create_env_cubemap(&mut self) -> VkResult<()> {
        if self.env_cubemap.is_some() {
            return Ok(());
        }
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();
        let mut pending =
            PendingEnvironment::new(d.clone(), allocator.clone(), self.logical_device.queue);

        // ---- 1. Create cubemap image (6 layers, CUBE_COMPATIBLE) ----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width: ENV_FACE_SIZE,
                height: ENV_FACE_SIZE,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(6)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .flags(vk::ImageCreateFlags::CUBE_COMPATIBLE);
        // SAFETY: `d` is a valid AshDevice; `image_info` describes a valid
        // 2D array image with CUBE_COMPATIBLE flag.
        pending.image = unsafe { d.create_image(&image_info, None) }
            .map_err(|r| VulkanError::vk("create_env_image", r))?;

        // SAFETY: `pending.image` was just created by this device.
        let req = unsafe { d.get_image_memory_requirements(pending.image) };
        let allocation = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "env-cubemap",
                requirements: req,
                location: crate::allocator::MemoryLocation::GpuOnly,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        pending.image_allocation = Some(allocation);
        let allocation = pending.image_allocation.as_ref().ok_or_else(|| {
            VulkanError::Loader("environment image allocation disappeared".into())
        })?;
        // SAFETY: `pending.image` was created by this device; `allocation` was
        // created for this image's memory requirements.
        unsafe { d.bind_image_memory(pending.image, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_env_image", r))?;

        // ---- 2. Image view (CUBE, all 6 layers) ----
        let view_info = vk::ImageViewCreateInfo::default()
            .image(pending.image)
            .view_type(vk::ImageViewType::CUBE)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 6,
            });
        // SAFETY: `d` is a valid AshDevice; `view_info` references a valid
        // image with CUBE view type; `None` means no custom allocator.
        pending.image_view = unsafe { d.create_image_view(&view_info, None) }
            .map_err(|r| VulkanError::vk("create_env_image_view", r))?;

        // ---- 3. Sampler (linear, clamp-to-edge) ----
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .min_lod(0.0)
            .max_lod(0.0)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false);
        // SAFETY: `d` is a valid AshDevice; `sampler_info` describes a valid
        // sampler; `None` means no custom allocator.
        pending.sampler = unsafe { d.create_sampler(&sampler_info, None) }
            .map_err(|r| VulkanError::vk("create_env_sampler", r))?;

        // ---- 4. Upload the deterministic procedural environment ----
        Self::upload_procedural_environment(&mut pending, self.adapter.queue_family_index)?;

        // ---- Commit, store, then update descriptor set binding=1 ----
        // No Vulkan object escapes the guard before creation, upload,
        // submission, and synchronization have all succeeded.
        let EnvironmentResources {
            image,
            allocation,
            image_view,
            sampler,
        } = pending.commit()?;
        // `update_env_descriptor_set` reads these handles from `self`, so the
        // resources must be visible there before the descriptor write.
        self.env_cubemap = Some(image);
        self.env_cubemap_view = Some(image_view);
        self.env_cubemap_allocation = Some(allocation);
        self.env_sampler = Some(sampler);
        if let Err(error) = self.update_env_descriptor_set() {
            self.destroy_env_resources();
            return Err(error);
        }

        Ok(())
    }

    /// Upload the built-in sky/ground environment through a staging buffer.
    fn upload_procedural_environment(
        pending: &mut PendingEnvironment,
        queue_family_index: u32,
    ) -> VkResult<()> {
        let d = &pending.device;
        let pixels = procedural_environment_rgba8(ENV_FACE_SIZE);
        let plan = environment_upload_plan(ENV_FACE_SIZE).ok_or_else(|| {
            VulkanError::Allocation("environment upload dimensions overflow".into())
        })?;
        let expected_bytes = usize::try_from(plan.total_bytes).map_err(|_| {
            VulkanError::Allocation(
                "environment upload size is not representable by the host".into(),
            )
        })?;
        if pixels.len() != expected_bytes {
            return Err(VulkanError::Allocation(format!(
                "environment pixel data contains {} bytes, expected {expected_bytes}",
                pixels.len()
            )));
        }

        // ---- Staging buffer (CpuToGpu, TRANSFER_SRC) ----
        let buf_info = vk::BufferCreateInfo::default()
            .size(plan.total_bytes)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is a valid AshDevice; `buf_info` describes a valid
        // buffer; `None` means no custom allocator.
        pending.staging_buffer = unsafe { d.create_buffer(&buf_info, None) }
            .map_err(|r| VulkanError::vk("create_env_staging_buf", r))?;
        // SAFETY: `pending.staging_buffer` was just created by this device.
        let req = unsafe { d.get_buffer_memory_requirements(pending.staging_buffer) };
        let staging_allocation = pending
            .allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "env-staging",
                requirements: req,
                location: crate::allocator::MemoryLocation::CpuToGpu,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        pending.staging_allocation = Some(staging_allocation);
        let staging_allocation = pending.staging_allocation.as_ref().ok_or_else(|| {
            VulkanError::Loader("environment staging allocation disappeared".into())
        })?;
        // SAFETY: `pending.staging_buffer` and `staging_allocation` are
        // compatible.
        unsafe {
            d.bind_buffer_memory(
                pending.staging_buffer,
                staging_allocation.memory(),
                staging_allocation.offset(),
            )
        }
        .map_err(|r| VulkanError::vk("bind_env_staging_buf", r))?;

        let slice = pending
            .staging_allocation
            .as_mut()
            .and_then(Allocation::mapped_slice_mut)
            .ok_or(VulkanError::MemoryNotMapped("environment staging"))?;
        if slice.len() < pixels.len() {
            return Err(VulkanError::Allocation(format!(
                "environment staging allocation exposes {} bytes, expected {}",
                slice.len(),
                pixels.len()
            )));
        }
        slice[..pixels.len()].copy_from_slice(&pixels);

        // ---- Temporary command pool + buffer for transfer ----
        let cp_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        // SAFETY: `d` is a valid AshDevice; `cp_info` describes a valid pool;
        // `None` means no custom allocator.
        pending.command_pool = unsafe { d.create_command_pool(&cp_info, None) }
            .map_err(|r| VulkanError::vk("create_env_cmd_pool", r))?;

        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(pending.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: `d` is a valid AshDevice; `pending.command_pool` is valid;
        // `alloc_info` is correctly structured.
        let cmd_bufs = unsafe { d.allocate_command_buffers(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_env_cmd_buf", r))?;
        let cmd = cmd_bufs.first().copied().ok_or_else(|| {
            VulkanError::Loader("Vulkan returned no environment upload command buffer".into())
        })?;

        // Begin one-shot command buffer
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: `cmd` is a valid command buffer in initial state.
        unsafe { d.begin_command_buffer(cmd, &begin_info) }
            .map_err(|r| VulkanError::vk("begin_env_cmd_buf", r))?;

        // Barrier 1: UNDEFINED → TRANSFER_DST_OPTIMAL
        let barrier_undef_to_transfer = vk::ImageMemoryBarrier::default()
            .image(pending.image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: ENV_FACE_COUNT,
            })
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL);
        // SAFETY: `cmd` is in recording state; image, barrier, and stage masks
        // are valid.
        unsafe {
            d.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_undef_to_transfer],
            );
        }

        // Copy staging buffer → cubemap, one region per face layer
        let buffer_copy_regions: Vec<vk::BufferImageCopy> = (0..ENV_FACE_COUNT)
            .map(|layer| {
                vk::BufferImageCopy::default()
                    .buffer_offset(plan.face_offsets[layer as usize])
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: layer,
                        layer_count: 1,
                    })
                    .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                    .image_extent(vk::Extent3D {
                        width: ENV_FACE_SIZE,
                        height: ENV_FACE_SIZE,
                        depth: 1,
                    })
            })
            .collect();
        // SAFETY: `cmd` is in recording state; the pending staging buffer and
        // image are valid; copy regions are within bounds for both resources.
        unsafe {
            d.cmd_copy_buffer_to_image(
                cmd,
                pending.staging_buffer,
                pending.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &buffer_copy_regions,
            );
        }

        // Barrier 2: TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
        let barrier_transfer_to_read = vk::ImageMemoryBarrier::default()
            .image(pending.image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: ENV_FACE_COUNT,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        // SAFETY: `cmd` is in recording state; image, barrier, and stage masks
        // are valid.
        unsafe {
            d.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_transfer_to_read],
            );
        }

        // End command buffer
        // SAFETY: `cmd` is in recording state.
        unsafe { d.end_command_buffer(cmd) }.map_err(|r| VulkanError::vk("end_env_cmd_buf", r))?;

        // Submit with a temporary fence
        let fence_info = vk::FenceCreateInfo::default();
        // SAFETY: `d` is a valid AshDevice; `fence_info` describes a default
        // fence; `None` means no custom allocator.
        pending.fence = unsafe { d.create_fence(&fence_info, None) }
            .map_err(|r| VulkanError::vk("create_env_fence", r))?;

        let cmd_bufs = [cmd];
        let submit_info = [vk::SubmitInfo::default().command_buffers(&cmd_bufs)];
        // SAFETY: `d` is a valid AshDevice; `pending.queue` is a valid queue;
        // `submit_info` and `pending.fence` are valid.
        // Mark this conservatively before the call: even an error such as
        // device loss is not proof that no implementation-owned work began.
        pending.submitted = true;
        unsafe { d.queue_submit(pending.queue, &submit_info, pending.fence) }
            .map_err(|r| VulkanError::vk("submit_env_upload", r))?;

        // Wait for completion
        // SAFETY: `d` is a valid AshDevice; `pending.fence` was submitted
        // above; `true` = wait-all; `u64::MAX` = infinite timeout.
        unsafe { d.wait_for_fences(&[pending.fence], true, u64::MAX) }
            .map_err(|r| VulkanError::vk("wait_env_fence", r))?;
        pending.submitted = false;

        // ---- Cleanup temporary resources ----
        pending.destroy_transient_resources();

        Ok(())
    }

    /// Destroy all environment cubemap resources (reverse order of creation).
    pub(crate) fn destroy_env_resources(&mut self) {
        let d = &self.logical_device.device;

        if let Some(s) = self.env_sampler.take() {
            // SAFETY: `s` was created by this device and is still alive.
            unsafe {
                d.destroy_sampler(s, None);
            }
        }
        if let Some(iv) = self.env_cubemap_view.take() {
            // SAFETY: `iv` was created by this device and is still alive.
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(img) = self.env_cubemap.take() {
            // SAFETY: `img` was created by this device and is still alive.
            unsafe {
                d.destroy_image(img, None);
            }
        }
        free_allocation(
            d,
            &self.logical_device.allocator(),
            &mut self.env_cubemap_allocation,
            "environment cubemap image",
        );
    }

    /// Write the environment cubemap (image view + sampler) into the shadow
    /// descriptor set at binding=1.
    ///
    /// Returns an error if the shadow descriptor infrastructure has not been
    /// created, because accepting that state would leave IBL silently unbound.
    fn update_env_descriptor_set(&self) -> VkResult<()> {
        let ds = self.shadow_desc_set.ok_or_else(|| {
            VulkanError::Loader("environment descriptor set is unavailable".into())
        })?;
        let sampler = self
            .env_sampler
            .ok_or_else(|| VulkanError::Loader("environment sampler is unavailable".into()))?;
        let image_view = self
            .env_cubemap_view
            .ok_or_else(|| VulkanError::Loader("environment cubemap view is unavailable".into()))?;
        let d = &self.logical_device.device;

        let image_info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(image_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(ds)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&image_info)];
        // SAFETY: `d` is a valid AshDevice; descriptor set, sampler, and image
        // view are valid; binding=1 exists in the descriptor set layout.
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_upload_plan_has_contiguous_six_face_layout() {
        let plan = environment_upload_plan(16).expect("valid environment dimensions");
        assert_eq!(plan.face_bytes, 16 * 16 * u64::from(ENV_PIXEL_SIZE));
        assert_eq!(
            plan.total_bytes,
            plan.face_bytes * u64::from(ENV_FACE_COUNT)
        );
        assert_eq!(
            plan.face_offsets,
            [
                0,
                plan.face_bytes,
                plan.face_bytes * 2,
                plan.face_bytes * 3,
                plan.face_bytes * 4,
                plan.face_bytes * 5,
            ]
        );
        assert!(environment_upload_plan(0).is_none());
    }

    #[test]
    fn pending_resource_state_requires_complete_committed_ownership() {
        assert_eq!(
            classify_pending_resources(PendingResourcePresence::default()),
            PendingResourceState::Empty
        );

        let partial = PendingResourcePresence {
            image: true,
            ..PendingResourcePresence::default()
        };
        assert_eq!(
            classify_pending_resources(partial),
            PendingResourceState::Partial
        );

        let ready = PendingResourcePresence {
            image: true,
            image_allocation: true,
            image_view: true,
            sampler: true,
            ..PendingResourcePresence::default()
        };
        assert_eq!(
            classify_pending_resources(ready),
            PendingResourceState::ReadyToCommit
        );
        assert_eq!(
            classify_pending_resources(PendingResourcePresence {
                submitted: true,
                ..ready
            }),
            PendingResourceState::UploadInFlight
        );
        assert_eq!(
            classify_pending_resources(PendingResourcePresence {
                staging_buffer: true,
                ..ready
            }),
            PendingResourceState::Partial
        );
    }

    #[test]
    fn procedural_environment_has_six_complete_rgba_faces() {
        let size = 8;
        let pixels = procedural_environment_rgba8(size);
        assert_eq!(
            pixels.len(),
            size as usize * size as usize * ENV_FACE_COUNT as usize * 4
        );
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn procedural_environment_is_directional_instead_of_flat() {
        let size = 8usize;
        let pixels = procedural_environment_rgba8(size as u32);
        let face_bytes = size * size * 4;
        let center = (size / 2 * size + size / 2) * 4;
        let sky = &pixels[2 * face_bytes + center..2 * face_bytes + center + 3];
        let ground = &pixels[3 * face_bytes + center..3 * face_bytes + center + 3];
        assert_ne!(sky, ground);
        assert!(
            sky[2] > ground[2],
            "sky should contain more blue than ground"
        );
    }
}
