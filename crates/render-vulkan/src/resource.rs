//! Device resources used by the Gate 2 textured-object validation path.

use ash::vk;
use ash::Device as AshDevice;
use crate::allocator::{Allocation, AllocationCreateDesc, AllocationScheme, MemoryLocation};

use crate::allocator::SharedAllocator;
use crate::device::Device;
use crate::error::{VkResult, VulkanError};

const TEXTURE_WIDTH: u32 = 4;
const TEXTURE_HEIGHT: u32 = 4;
const TEXTURE_PIXELS: [u8; 64] = [
    255, 0, 0, 255, 255, 255, 0, 255, 0, 255, 0, 255, 0, 192, 255, 255, 255, 0, 255, 255, 255, 255,
    255, 255, 0, 0, 0, 255, 255, 128, 0, 255, 0, 96, 255, 255, 128, 0, 255, 255, 255, 255, 255,
    255, 0, 0, 0, 255, 0, 64, 192, 255, 255, 255, 255, 255, 255, 0, 128, 255, 0, 255, 128, 255,
];

#[derive(Clone, Copy)]
struct TexturedVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

const TEXTURED_VERTICES: [TexturedVertex; 4] = [
    TexturedVertex {
        position: [-0.72, -0.72],
        uv: [0.0, 1.0],
    },
    TexturedVertex {
        position: [0.72, -0.72],
        uv: [1.0, 1.0],
    },
    TexturedVertex {
        position: [0.72, 0.72],
        uv: [1.0, 0.0],
    },
    TexturedVertex {
        position: [-0.72, 0.72],
        uv: [0.0, 0.0],
    },
];

const TEXTURED_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

pub struct TexturedQuadResources {
    pub vertex_buffer: BufferResource,
    pub index_buffer: BufferResource,
    pub index_count: u32,
    pub _texture: TextureResource,
    pub descriptor_set_layout: vk::DescriptorSetLayout,
    pub descriptor_set: vk::DescriptorSet,
    descriptor_pool: vk::DescriptorPool,
    device: AshDevice,
}

impl TexturedQuadResources {
    pub unsafe fn new(device: &Device) -> VkResult<Self> {
        let vertex_bytes = textured_vertex_bytes();
        let index_bytes = textured_index_bytes();

        // SAFETY: device is valid and the byte slices remain alive during upload.
        let vertex_buffer = unsafe {
            BufferResource::new_with_data(
                device,
                "gate2 textured quad vertices",
                &vertex_bytes,
                vk::BufferUsageFlags::VERTEX_BUFFER,
            )
        }?;
        // SAFETY: device is valid and the byte slices remain alive during upload.
        let index_buffer = unsafe {
            BufferResource::new_with_data(
                device,
                "gate2 textured quad indices",
                &index_bytes,
                vk::BufferUsageFlags::INDEX_BUFFER,
            )
        }?;

        // SAFETY: device and queue are valid; upload command waits before returning.
        let texture = unsafe { TextureResource::new(device) }?;
        // SAFETY: descriptor objects are created against the live device.
        let descriptor_set_layout = unsafe { create_descriptor_set_layout(&device.device) }?;
        // SAFETY: descriptor pool is created against the live device.
        let descriptor_pool = unsafe { create_descriptor_pool(&device.device) }?;
        // SAFETY: descriptor pool/layout are valid and compatible.
        let descriptor_set = unsafe {
            allocate_descriptor_set(&device.device, descriptor_pool, descriptor_set_layout)?
        };
        update_descriptor_set(&device.device, descriptor_set, &texture);

        Ok(Self {
            vertex_buffer,
            index_buffer,
            index_count: TEXTURED_INDICES.len() as u32,
            _texture: texture,
            descriptor_set_layout,
            descriptor_set,
            descriptor_pool,
            device: device.device.clone(),
        })
    }
}

impl Drop for TexturedQuadResources {
    fn drop(&mut self) {
        // SAFETY: VulkanRenderer waits for the device to be idle before dropping resources.
        unsafe {
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
        }
    }
}

pub struct BufferResource {
    pub buffer: vk::Buffer,
    allocation: Option<Allocation>,
    allocator: SharedAllocator,
    device: AshDevice,
}

impl BufferResource {
    unsafe fn new(
        device: &Device,
        name: &'static str,
        size: vk::DeviceSize,
        usage: vk::BufferUsageFlags,
        location: MemoryLocation,
    ) -> VkResult<Self> {
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: buffer_info outlives this call.
        let buffer = unsafe { device.device.create_buffer(&buffer_info, None) }
            .map_err(|result| VulkanError::vk("create_buffer", result))?;
        // SAFETY: buffer is valid.
        let requirements = unsafe { device.device.get_buffer_memory_requirements(buffer) };
        let allocator = device.allocator();
        let mut allocation = allocator
            .lock().map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&AllocationCreateDesc {
                name,
                requirements,
                location,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|err| VulkanError::Allocation(err.to_string()))?;
        // SAFETY: allocation was created for this buffer's requirements.
        let bind_result = unsafe {
            device
                .device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
        };
        if let Err(result) = bind_result {
            if let Ok(mut guard) = allocator.lock() {
                let _ = guard.free(&mut allocation);
            }
            // SAFETY: buffer was created above and is not bound to live resources.
            unsafe { device.device.destroy_buffer(buffer, None) };
            return Err(VulkanError::vk("bind_buffer_memory", result));
        }

        Ok(Self {
            buffer,
            allocation: Some(allocation),
            allocator,
            device: device.device.clone(),
        })
    }

    unsafe fn new_with_data(
        device: &Device,
        name: &'static str,
        data: &[u8],
        usage: vk::BufferUsageFlags,
    ) -> VkResult<Self> {
        // SAFETY: device is valid; CpuToGpu allocations are host-mapped by gpu-allocator.
        let mut resource = unsafe {
            Self::new(
                device,
                name,
                data.len() as vk::DeviceSize,
                usage,
                MemoryLocation::CpuToGpu,
            )
        }?;
        resource.write(data, name)?;
        Ok(resource)
    }

    fn write(&mut self, data: &[u8], name: &'static str) -> VkResult<()> {
        let allocation = self
            .allocation
            .as_mut()
            .ok_or(VulkanError::Loader("buffer allocation not initialized".into()))?;
        let Some(slice) = allocation.mapped_slice_mut() else {
            return Err(VulkanError::MemoryNotMapped(name));
        };
        slice[..data.len()].copy_from_slice(data);
        Ok(())
    }
}

impl Drop for BufferResource {
    fn drop(&mut self) {
        // SAFETY: VulkanRenderer waits for the device to be idle before dropping resources.
        unsafe { self.device.destroy_buffer(self.buffer, None) };
        if let Some(mut allocation) = self.allocation.take() {
            if let Ok(mut guard) = self.allocator.lock() {
                let _ = guard.free(&mut allocation);
            }
        }
    }
}

pub struct TextureResource {
    pub image_view: vk::ImageView,
    pub sampler: vk::Sampler,
    image: vk::Image,
    allocation: Option<Allocation>,
    allocator: SharedAllocator,
    device: AshDevice,
}

impl TextureResource {
    unsafe fn new(device: &Device) -> VkResult<Self> {
        // SAFETY: staging buffer is host-mapped and lives through the immediate copy.
        let staging = unsafe {
            BufferResource::new_with_data(
                device,
                "gate2 texture staging",
                &TEXTURE_PIXELS,
                vk::BufferUsageFlags::TRANSFER_SRC,
            )
        }?;

        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width: TEXTURE_WIDTH,
                height: TEXTURE_HEIGHT,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: image_info outlives this call.
        let image = unsafe { device.device.create_image(&image_info, None) }
            .map_err(|result| VulkanError::vk("create_image", result))?;
        // SAFETY: image is valid.
        let requirements = unsafe { device.device.get_image_memory_requirements(image) };
        let allocator = device.allocator();
        let allocation = allocator
            .lock().map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&AllocationCreateDesc {
                name: "gate2 textured quad image",
                requirements,
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|err| VulkanError::Allocation(err.to_string()))?;
        // SAFETY: allocation was created for this image's requirements.
        unsafe {
            device
                .device
                .bind_image_memory(image, allocation.memory(), allocation.offset())
        }
        .map_err(|result| VulkanError::vk("bind_image_memory", result))?;

        // SAFETY: upload commands complete before this function returns.
        unsafe { upload_texture(device, staging.buffer, image) }?;

        let image_view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(color_subresource_range());
        // SAFETY: image_view_info outlives this call.
        let image_view = unsafe { device.device.create_image_view(&image_view_info, None) }
            .map_err(|result| VulkanError::vk("create_image_view(texture)", result))?;

        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .min_lod(0.0)
            .max_lod(0.0);
        // SAFETY: sampler_info outlives this call.
        let sampler = unsafe { device.device.create_sampler(&sampler_info, None) }
            .map_err(|result| VulkanError::vk("create_sampler", result))?;

        Ok(Self {
            image_view,
            sampler,
            image,
            allocation: Some(allocation),
            allocator,
            device: device.device.clone(),
        })
    }
}

impl Drop for TextureResource {
    fn drop(&mut self) {
        // SAFETY: VulkanRenderer waits for the device to be idle before dropping resources.
        unsafe {
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
        }
        if let Some(mut allocation) = self.allocation.take() {
            if let Ok(mut guard) = self.allocator.lock() {
                let _ = guard.free(&mut allocation);
            }
        }
    }
}

unsafe fn create_descriptor_set_layout(device: &AshDevice) -> VkResult<vk::DescriptorSetLayout> {
    let bindings = [
        vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
    ];
    let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    // SAFETY: info outlives this call.
    unsafe { device.create_descriptor_set_layout(&info, None) }
        .map_err(|result| VulkanError::vk("create_descriptor_set_layout", result))
}

unsafe fn create_descriptor_pool(device: &AshDevice) -> VkResult<vk::DescriptorPool> {
    let pool_sizes = [
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::SAMPLED_IMAGE,
            descriptor_count: 1,
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::SAMPLER,
            descriptor_count: 1,
        },
    ];
    let info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(&pool_sizes);
    // SAFETY: info outlives this call.
    unsafe { device.create_descriptor_pool(&info, None) }
        .map_err(|result| VulkanError::vk("create_descriptor_pool", result))
}

unsafe fn allocate_descriptor_set(
    device: &AshDevice,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> VkResult<vk::DescriptorSet> {
    let layouts = [descriptor_set_layout];
    let info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(&layouts);
    // SAFETY: info outlives this call.
    let sets = unsafe { device.allocate_descriptor_sets(&info) }
        .map_err(|result| VulkanError::vk("allocate_descriptor_sets", result))?;
    Ok(sets[0])
}

fn update_descriptor_set(
    device: &AshDevice,
    descriptor_set: vk::DescriptorSet,
    texture: &TextureResource,
) {
    let sampled_image_info = [vk::DescriptorImageInfo::default()
        .image_view(texture.image_view)
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
    let sampler_info = [vk::DescriptorImageInfo::default().sampler(texture.sampler)];
    let writes = [
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(&sampled_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .image_info(&sampler_info),
    ];
    // SAFETY: descriptor set and image resources are valid for the update call.
    unsafe { device.update_descriptor_sets(&writes, &[]) };
}

unsafe fn upload_texture(
    device: &Device,
    staging_buffer: vk::Buffer,
    image: vk::Image,
) -> VkResult<()> {
    // SAFETY: the immediate command buffer is submitted and waited before returning.
    unsafe {
        submit_immediate(device, |command_buffer| {
            transition_image(
                &device.device,
                command_buffer,
                image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            );

            let copy_region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D {
                    width: TEXTURE_WIDTH,
                    height: TEXTURE_HEIGHT,
                    depth: 1,
                });
            device.device.cmd_copy_buffer_to_image(
                command_buffer,
                staging_buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[copy_region],
            );

            transition_image(
                &device.device,
                command_buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            );
        })
    }
}

unsafe fn submit_immediate<F>(device: &Device, record: F) -> VkResult<()>
where
    F: FnOnce(vk::CommandBuffer),
{
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(device.queue_family_index)
        .flags(vk::CommandPoolCreateFlags::TRANSIENT);
    // SAFETY: pool_info outlives this call.
    let command_pool = unsafe { device.device.create_command_pool(&pool_info, None) }
        .map_err(|result| VulkanError::vk("create_command_pool(upload)", result))?;

    let mut fence = vk::Fence::null();
    let result = (|| {
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: alloc_info outlives this call.
        let command_buffer = unsafe { device.device.allocate_command_buffers(&alloc_info) }
            .map_err(|result| VulkanError::vk("allocate_command_buffers(upload)", result))?[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: command buffer is valid and currently reset.
        unsafe {
            device
                .device
                .begin_command_buffer(command_buffer, &begin_info)
        }
        .map_err(|result| VulkanError::vk("begin_command_buffer(upload)", result))?;
        record(command_buffer);
        // SAFETY: command buffer recording was begun above.
        unsafe { device.device.end_command_buffer(command_buffer) }
            .map_err(|result| VulkanError::vk("end_command_buffer(upload)", result))?;

        let command_buffers = [command_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        let fence_info = vk::FenceCreateInfo::default();
        // SAFETY: fence_info has no borrowed data.
        fence = unsafe { device.device.create_fence(&fence_info, None) }
            .map_err(|result| VulkanError::vk("create_fence(upload)", result))?;
        // SAFETY: queue, command buffer, and fence are valid.
        unsafe {
            device
                .device
                .queue_submit(device.queue, &[submit_info], fence)
        }
        .map_err(|result| VulkanError::vk("queue_submit(upload)", result))?;
        // SAFETY: fence is valid and associated with the upload submit.
        unsafe { device.device.wait_for_fences(&[fence], true, u64::MAX) }
            .map_err(|result| VulkanError::vk("wait_for_fences(upload)", result))?;
        Ok(())
    })();

    // SAFETY: objects were created from this device and are no longer in use after fence wait or failed submit.
    unsafe {
        if fence != vk::Fence::null() {
            device.device.destroy_fence(fence, None);
        }
        device.device.destroy_command_pool(command_pool, None);
    }
    result
}

unsafe fn transition_image(
    device: &AshDevice,
    command_buffer: vk::CommandBuffer,
    image: vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
) {
    let (src_access_mask, dst_access_mask, src_stage, dst_stage) = match (old_layout, new_layout) {
        (vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL) => (
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
        ),
        (vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL) => (
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        ),
        _ => (
            vk::AccessFlags::empty(),
            vk::AccessFlags::empty(),
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
        ),
    };
    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(old_layout)
        .new_layout(new_layout)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(color_subresource_range())
        .src_access_mask(src_access_mask)
        .dst_access_mask(dst_access_mask);
    // SAFETY: command buffer is recording; barrier references a live image.
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            src_stage,
            dst_stage,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }
}

fn color_subresource_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

fn textured_vertex_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(TEXTURED_VERTICES.len() * 16);
    for vertex in TEXTURED_VERTICES {
        for value in vertex.position.into_iter().chain(vertex.uv) {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    bytes
}

fn textured_index_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(TEXTURED_INDICES.len() * 2);
    for index in TEXTURED_INDICES {
        bytes.extend_from_slice(&index.to_ne_bytes());
    }
    bytes
}
