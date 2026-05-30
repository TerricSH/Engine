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

    /// Write default per-frame UBO data matching the CSM forward shader layout:
    ///
    /// | offset | field          | type   | bytes |
    /// |--------|----------------|--------|-------|
    /// |      0 | model          | mat4   |    64 |
    /// |     64 | view_proj      | mat4   |    64 |
    /// |    128 | light_dir      | vec4   |    16 |
    /// |    144 | light_color    | vec4   |    16 |
    /// |    160 | camera_pos     | vec4   |    16 |
    /// |    176 | cascade_splits | vec4   |    16 |
    /// |    192 | light_vp[0]    | mat4   |    64 |
    /// |    256 | light_vp[1]    | mat4   |    64 |
    /// |    320 | light_vp[2]    | mat4   |    64 |
    ///
    /// Total: 384 bytes (fits in 512 B UBO).
    pub fn write_default_ubo(&mut self) {
        let fi = self.current_frame;
        let mut data = Vec::with_capacity(384);
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
        // Cascade splits (default: far=100, split0=1, split1=10, split2=100)
        for v in &[1.0f32, 10.0f32, 100.0f32, 100.0f32] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light VP[0] (identity until the shadow pass writes cascade data)
        for i in 0..16 {
            let v = if i % 5 == 0 { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light VP[1] (identity)
        for i in 0..16 {
            let v = if i % 5 == 0 { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light VP[2] (identity)
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

    // ======================================================================
    // Material descriptor infra (set=2, per-drawable material UBO)
    // ======================================================================

    /// Create descriptor set layout + pool for material resources (set=2).
    ///
    /// Layout (set=2):
    ///   binding=0: UNIFORM_BUFFER  (MaterialUBO — base_color, metallic, etc.)
    ///   binding=1: COMBINED_IMAGE_SAMPLER (base color texture)
    ///
    /// Pool: up to 256 descriptor sets, each with 1 UBO + 1 sampler descriptor.
    ///
    /// Idempotent: returns `Ok(())` if already created.
    pub(crate) fn create_material_descriptor_infra(&mut self) -> VkResult<()> {
        if self.material_desc_set_layout.is_some() {
            return Ok(());
        }
        let d = &self.logical_device.device;

        // Layout: UBO at binding=0, combined image sampler at binding=1,
        // bone UBO at binding=2 (used by skinned vertex shader).
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX),
        ];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let ds_layout = unsafe { d.create_descriptor_set_layout(&layout_info, None) }
            .map_err(|r| VulkanError::vk("create_material_ds_layout", r))?;

        // Pool: up to 256 material descriptor sets, each with UBO + sampler + bone UBO
        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: 512,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: 256,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(256)
            .pool_sizes(&pool_sizes)
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET);
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_material_ds_pool", r))?;

        self.material_desc_set_layout = Some(ds_layout);
        self.material_desc_pool = Some(pool);
        Ok(())
    }

    /// Allocate a new material descriptor set for the given buffer.
    ///
    /// # Panics
    ///
    /// Panics if `create_material_descriptor_infra` has not been called first.
    pub(crate) fn allocate_material_descriptor_set(
        &self,
        buffer: vk::Buffer,
        ubo_size: vk::DeviceSize,
    ) -> VkResult<vk::DescriptorSet> {
        let d = &self.logical_device.device;
        let layout = self
            .material_desc_set_layout
            .expect("material_desc_set_layout not created");
        let pool = self
            .material_desc_pool
            .expect("material_desc_pool not created");

        let layouts = [layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_material_ds", r))?;
        let desc_set = desc_sets[0];

        // Write the descriptor: binding 0 → uniform buffer
        let buf_info = [vk::DescriptorBufferInfo::default()
            .buffer(buffer)
            .offset(0)
            .range(ubo_size)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(desc_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .buffer_info(&buf_info)];
        // SAFETY: `d` is a valid AshDevice; descriptor set, buffer are valid.
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }

        Ok(desc_set)
    }

    /// Allocate a material descriptor set with an additional bone UBO at binding=2.
    ///
    /// Used by skinned-item rendering: allocates from the material pool, writes
    /// the material UBO at binding=0 and the bone palette UBO at binding=2.
    /// The texture binding (binding=1) is left unwritten and can be updated
    /// later via [`bind_material_texture`](Self::bind_material_texture).
    pub(crate) fn allocate_skinned_material_descriptor_set(
        &self,
        material_buffer: vk::Buffer,
        material_ubo_size: vk::DeviceSize,
        bone_buffer: vk::Buffer,
        bone_ubo_size: vk::DeviceSize,
    ) -> VkResult<vk::DescriptorSet> {
        let d = &self.logical_device.device;
        let layout = self
            .material_desc_set_layout
            .expect("material_desc_set_layout not created");
        let pool = self
            .material_desc_pool
            .expect("material_desc_pool not created");

        let layouts = [layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_skinned_material_ds", r))?;
        let desc_set = desc_sets[0];

        // Write binding 0 → material UBO, binding 2 → bone palette UBO
        let buf_info = [
            vk::DescriptorBufferInfo::default()
                .buffer(material_buffer)
                .offset(0)
                .range(material_ubo_size),
            vk::DescriptorBufferInfo::default()
                .buffer(bone_buffer)
                .offset(0)
                .range(bone_ubo_size),
        ];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buf_info[0..1]),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buf_info[1..2]),
        ];
        // SAFETY: `d` is a valid AshDevice; descriptor set and buffers are valid.
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }

        Ok(desc_set)
    }

    // ======================================================================
    // Light SSBO (set=1, binding=2) — clustered lighting for Phase 4.3
    // ======================================================================

    /// Create the storage buffer for additional lights (up to max_lights).
    ///
    /// Must be called AFTER `ensure_shadow()` so that the set=1 descriptor set
    /// exists. Idempotent: returns `Ok(())` if already created.
    pub(crate) fn create_light_ssbo(&mut self) -> VkResult<()> {
        if self.light_ssbo.is_some() {
            return Ok(());
        }
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();

        // 80 bytes per light (per the Light GPU struct: 4 × vec4 = 64 B plus
        // padding; actual GLSL struct is 64 B, but we allocate 80 for safety)
        let buffer_size = self.max_lights as u64 * 64;

        // Create the storage buffer
        let bi = vk::BufferCreateInfo::default()
            .size(buffer_size)
            .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is a valid AshDevice; `bi` describes a valid buffer.
        let buf = unsafe { d.create_buffer(&bi, None) }
            .map_err(|r| VulkanError::vk("create_light_ssbo", r))?;
        let req = unsafe { d.get_buffer_memory_requirements(buf) };
        let allocation = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "light-ssbo",
                requirements: req,
                location: crate::allocator::MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: `buf` was created by this device; `allocation` is compatible.
        unsafe { d.bind_buffer_memory(buf, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_light_ssbo", r))?;

        // Update set=1 descriptor at binding=2 to point to the SSBO
        if let Some(ds) = self.shadow_desc_set {
            let buf_info = [vk::DescriptorBufferInfo::default()
                .buffer(buf)
                .offset(0)
                .range(buffer_size)];
            let writes = [vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&buf_info)];
            // SAFETY: `d` is a valid AshDevice; descriptor set and buffer are
            // valid; binding=2 exists in the set=1 layout.
            unsafe {
                d.update_descriptor_sets(&writes, &[]);
            }
        }

        self.light_ssbo = Some(buf);
        self.light_ssbo_allocation = Some(allocation);
        self.light_ssbo_size = buffer_size;

        Ok(())
    }

    /// Write data into the light SSBO at the given byte offset.
    ///
    /// Silently returns if the SSBO has not been created yet.
    pub(crate) fn write_light_ssbo(&mut self, data: &[u8], offset: u64) {
        if let Some(allocation) = &mut self.light_ssbo_allocation {
            if let Some(slice) = allocation.mapped_slice_mut() {
                let start = offset as usize;
                let end = (start + data.len()).min(slice.len());
                slice[start..end].copy_from_slice(&data[..end - start]);
            }
        }
    }

    /// Destroy the light SSBO buffer and free its allocation.
    pub(crate) fn destroy_light_ssbo(&mut self) {
        let d = &self.logical_device.device;
        if let Some(buf) = self.light_ssbo.take() {
            // SAFETY: `buf` was created by this device and is no longer referenced.
            unsafe {
                d.destroy_buffer(buf, None);
            }
        }
        if let Some(mut a) = self.light_ssbo_allocation.take() {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                guard.free(&mut a);
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
