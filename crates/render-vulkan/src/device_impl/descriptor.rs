//! Descriptor infrastructure for VulkanDevice (set=0 per-frame UBO per FD-041).

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::VulkanDevice;

impl VulkanDevice {
    pub(crate) fn create_descriptor_infra(&mut self) -> VkResult<()> {
        if self.desc_set_layout_0.is_some() {
            return Ok(());
        } // already created
        let d = &self.logical_device.device;

        // Descriptor set layout: binding 0 = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER at set=0
        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let ds_layout = unsafe { d.create_descriptor_set_layout(&layout_info, None) }
            .map_err(|r| VulkanError::vk("create_ds_layout", r))?;

        // Descriptor pool: 2 sets (double buffering), 2 UBO descriptors
        let pool_sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::UNIFORM_BUFFER,
            descriptor_count: 2,
        }];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(2)
            .pool_sizes(&pool_sizes);
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_ds_pool", r))?;

        // Allocate 2 descriptor sets
        let layouts = [ds_layout, ds_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_ds", r))?;

        // Create 2 UBO buffers (CpuToGpu, sized to ubo_size)
        let mut ubos = Vec::with_capacity(2);
        let mut allocs = Vec::with_capacity(2);
        let allocator = self.logical_device.allocator();
        for i in 0..2 {
            let bi = vk::BufferCreateInfo::default()
                .size(self.ubo_size)
                .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buf = unsafe { d.create_buffer(&bi, None) }
                .map_err(|r| VulkanError::vk("create_ubo", r))?;
            let req = unsafe { d.get_buffer_memory_requirements(buf) };
            self.ubo_alignment = req.alignment;
            let allocation = allocator
                .lock()
                .unwrap()
                .allocate(&crate::allocator::AllocationCreateDesc {
                    name: ["frame-ubo-0", "frame-ubo-1"][i],
                    requirements: req,
                    location: crate::allocator::MemoryLocation::CpuToGpu,
                    linear: true,
                    allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
                })
                .map_err(|e| VulkanError::Allocation(e.to_string()))?;
            unsafe { d.bind_buffer_memory(buf, allocation.memory(), allocation.offset()) }
                .map_err(|r| VulkanError::vk("bind_ubo", r))?;
            ubos.push(buf);
            allocs.push(allocation);
        }

        // Write descriptor sets
        for (i, ds) in desc_sets.iter().enumerate() {
            let buf_info = [vk::DescriptorBufferInfo::default()
                .buffer(ubos[i])
                .offset(0)
                .range(self.ubo_size)];
            let writes = [vk::WriteDescriptorSet::default()
                .dst_set(*ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buf_info)];
            unsafe {
                d.update_descriptor_sets(&writes, &[]);
            }
        }

        self.desc_set_layout_0 = Some(ds_layout);
        self.desc_pool = Some(pool);
        self.frame_desc_sets = desc_sets;
        self.frame_ubos = ubos;
        self.ubo_allocations = allocs;
        Ok(())
    }

    /// Get the per-frame descriptor set for the current frame index.
    pub fn frame_descriptor_set(&self, frame_idx: usize) -> Option<vk::DescriptorSet> {
        self.frame_desc_sets.get(frame_idx).copied()
    }

    /// Get the per-frame UBO for the current frame index.
    pub fn frame_ubo(&self, frame_idx: usize) -> Option<vk::Buffer> {
        self.frame_ubos.get(frame_idx).copied()
    }

    /// Write default per-frame UBO data matching the new forward shader layout:
    ///
    /// | offset | field         | type   | bytes |
    /// |--------|---------------|--------|-------|
    /// |      0 | model         | mat4   |    64 |
    /// |     64 | view_proj     | mat4   |    64 |
    /// |    128 | light_dir     | vec4   |    16 |
    /// |    144 | light_color   | vec4   |    16 |
    /// |    160 | camera_pos    | vec4   |    16 |
    /// |    176 | light_view_proj | mat4 |    64 |
    ///
    /// Total: 240 bytes (fits in 256 B UBO).
    pub fn write_default_ubo(&mut self) {
        let fi = self.current_frame;
        let mut data = Vec::with_capacity(240);
        // Model matrix (identity for clip-space rendering)
        for i in 0..16 {
            let v = if i % 5 == 0 { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // View-proj matrix (identity as well)
        for i in 0..16 {
            let v = if i % 5 == 0 { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light direction (normalized, pointing down-left)
        for v in &[0.5f32, -0.707f32, 0.5f32, 0.0f32] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light color (bright white, intensity 1.5)
        for v in &[1.5f32, 1.5f32, 1.5f32, 1.5f32] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Camera position (world space)
        for v in &[0.0f32, 0.0f32, 2.0f32, 1.0f32] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light view-projection (identity until a real shadow pass writes it)
        for i in 0..16 {
            let v = if i % 5 == 0 { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        self.write_ubo(fi, &data, 0);
    }
    /// SAFETY: data must not exceed ubo_size - offset.
    pub fn write_ubo(&mut self, frame_idx: usize, data: &[u8], offset: u64) {
        if frame_idx >= self.ubo_allocations.len() {
            let _ = self.ensure_sc();
        }
        if let Some(allocation) = self.ubo_allocations.get_mut(frame_idx) {
            if let Some(slice) = allocation.mapped_slice_mut() {
                let start = offset as usize;
                let end = (start + data.len()).min(slice.len());
                slice[start..end].copy_from_slice(&data[..end - start]);
            }
        }
    }

    pub(crate) fn destroy_descriptor_infra(&mut self) {
        let d = &self.logical_device.device;
        for mut a in self.ubo_allocations.drain(..) {
            self.logical_device.allocator().lock().unwrap().free(&mut a);
        }
        for buf in self.frame_ubos.drain(..) {
            unsafe {
                d.destroy_buffer(buf, None);
            }
        }
        if let Some(pool) = self.desc_pool.take() {
            unsafe {
                d.destroy_descriptor_pool(pool, None);
            }
        }
        if let Some(layout) = self.desc_set_layout_0.take() {
            unsafe {
                d.destroy_descriptor_set_layout(layout, None);
            }
        }
    }
}
