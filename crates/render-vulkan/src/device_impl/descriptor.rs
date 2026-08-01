//! Descriptor infrastructure for VulkanDevice (set=0 per-frame UBO per FD-041).

use ash::vk;

use crate::error::{VkResult, VulkanError};
use engine_renderer::{
    LIGHT_GPU_SIZE, LIGHT_HEADER_SIZE, MAX_CLUSTERS, MAX_CLUSTER_LIGHT_INDICES, MAX_LIGHTS,
};

use super::{texture::FALLBACK_MATERIAL_TEXTURE_ID, VulkanDevice};

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
        // SAFETY: `d` is live and `layout_info` references a local binding array
        // for the duration of the call.
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
        // SAFETY: `d` is live and the pool sizes referenced by `pool_info`
        // remain valid for the call.
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_ds_pool", r))?;

        // Allocate 2 descriptor sets
        let layouts = [ds_layout, ds_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        // SAFETY: `pool` and both layout handles were created by `d`, and the
        // referenced layout array lives through allocation.
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
            // SAFETY: `d` is live and `bi` is a complete uniform-buffer create
            // description with a non-zero configured UBO size.
            let buf = unsafe { d.create_buffer(&bi, None) }
                .map_err(|r| VulkanError::vk("create_ubo", r))?;
            // SAFETY: `buf` was just created by `d` and remains live.
            let req = unsafe { d.get_buffer_memory_requirements(buf) };
            self.ubo_alignment = req.alignment;
            let allocation = allocator
                .lock()
                .map_err(|error| VulkanError::Loader(format!("UBO allocator lock: {error}")))?
                .allocate(&crate::allocator::AllocationCreateDesc {
                    name: ["frame-ubo-0", "frame-ubo-1"][i],
                    requirements: req,
                    location: crate::allocator::MemoryLocation::CpuToGpu,
                })
                .map_err(|e| VulkanError::Allocation(e.to_string()))?;
            // SAFETY: `allocation` was selected from `buf`'s requirements and
            // belongs to the same device; neither resource is concurrently used.
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
            // SAFETY: `ds` is allocated from this device and `buf_info` points
            // at a live UBO; all referenced slices outlive the update call.
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
    /// |    384 | environment    | vec4   |    16 |
    ///
    /// Total: 400 bytes (fits in 512 B UBO).
    pub fn write_default_ubo(&mut self) {
        let fi = self.current_frame;
        let mut data = Vec::with_capacity(400);
        // Model matrix (identity for clip-space rendering)
        for i in 0usize..16 {
            let v = if i.is_multiple_of(5) { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // View-proj matrix (identity as well)
        for i in 0usize..16 {
            let v = if i.is_multiple_of(5) { 1.0f32 } else { 0.0f32 };
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
        for i in 0usize..16 {
            let v = if i.is_multiple_of(5) { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light VP[1] (identity)
        for i in 0usize..16 {
            let v = if i.is_multiple_of(5) { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light VP[2] (identity)
        for i in 0usize..16 {
            let v = if i.is_multiple_of(5) { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Environment intensity=1, rotation sin=0/cos=1.
        for v in [1.0f32, 0.0, 1.0, 0.0] {
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
    /// Pool: 256 static-material sets plus 64 skinned-material sets.
    ///
    /// Idempotent: returns `Ok(())` if already created.
    pub(crate) fn create_material_descriptor_infra(&mut self) -> VkResult<()> {
        if self.material_desc_set_layout.is_some() {
            return self.create_fallback_material_texture();
        }
        let d = &self.logical_device.device;

        // Layout: UBO at binding=0, base color sampler at binding=1, bone UBO
        // at binding=2, then normal/MR/AO/emissive samplers at bindings 3..=6.
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
            vk::DescriptorSetLayoutBinding::default()
                .binding(3)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(4)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(5)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(6)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(7)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX),
        ];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        // SAFETY: `d` is live and the material bindings slice remains valid
        // throughout descriptor-layout creation.
        let ds_layout = unsafe { d.create_descriptor_set_layout(&layout_info, None) }
            .map_err(|r| VulkanError::vk("create_material_ds_layout", r))?;

        // Pool: 256 material sets plus 64 skinned sets. Every set owns a
        // five sampler descriptors; skinned sets consume a second UBO descriptor.
        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: 384,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: 1_600,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_BUFFER,
                descriptor_count: 64,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(320)
            .pool_sizes(&pool_sizes)
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET);
        // SAFETY: the pool description references live local storage and its
        // counts cover the engine's bounded material descriptor capacity.
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_material_ds_pool", r))?;

        self.material_desc_set_layout = Some(ds_layout);
        self.material_desc_pool = Some(pool);
        self.create_fallback_material_texture()
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
            .ok_or_else(|| VulkanError::Loader("material descriptor layout not created".into()))?;
        let pool = self
            .material_desc_pool
            .ok_or_else(|| VulkanError::Loader("material descriptor pool not created".into()))?;

        let layouts = [layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        // SAFETY: `pool` and `layout` are live handles from `d`; `layouts`
        // remains alive for the allocation call.
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_material_ds", r))?;
        let desc_set = desc_sets.first().copied().ok_or_else(|| {
            VulkanError::Loader("material descriptor allocation returned no set".into())
        })?;

        // Write the descriptor: binding 0 → uniform buffer
        let buf_info = [vk::DescriptorBufferInfo::default()
            .buffer(buffer)
            .offset(0)
            .range(ubo_size)];
        let fallback = self
            .textures
            .get(FALLBACK_MATERIAL_TEXTURE_ID)
            .ok_or_else(|| {
                VulkanError::Loader("fallback material texture not initialized".into())
            })?;
        let image_info = [vk::DescriptorImageInfo::default()
            .sampler(fallback.sampler)
            .image_view(fallback.view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buf_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(3)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(4)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(5)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(6)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
        ];
        // SAFETY: `d` is a valid AshDevice; descriptor set, buffer are valid.
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }

        Ok(desc_set)
    }

    /// Return one material/skinned descriptor set to the pool. Callers must
    /// ensure no submitted command buffer can still reference the set.
    pub(crate) fn free_material_descriptor_set(
        &self,
        descriptor_set: vk::DescriptorSet,
    ) -> VkResult<()> {
        if descriptor_set == vk::DescriptorSet::null() {
            return Ok(());
        }
        let pool = self
            .material_desc_pool
            .ok_or_else(|| VulkanError::Loader("material descriptor pool not created".into()))?;
        // SAFETY: the pool was created with FREE_DESCRIPTOR_SET and the set was
        // allocated from this exact pool.
        unsafe {
            self.logical_device
                .device
                .free_descriptor_sets(pool, &[descriptor_set])
        }
        .map_err(|result| VulkanError::vk("free_material_descriptor_set", result))
    }

    /// Allocate a material descriptor set with an additional bone UBO at binding=2.
    ///
    /// Used by skinned-item rendering: allocates from the material pool, writes
    /// the material UBO at binding=0 and the bone palette UBO at binding=2.
    /// The texture binding (binding=1) is left unwritten and can be updated
    /// later via [`bind_material_texture_at`](Self::bind_material_texture_at).
    pub(crate) fn allocate_skinned_material_descriptor_set(
        &self,
        material_buffer: vk::Buffer,
        material_ubo_size: vk::DeviceSize,
        bone_buffer: vk::Buffer,
        bone_ubo_size: vk::DeviceSize,
        morph_buffer: vk::Buffer,
    ) -> VkResult<vk::DescriptorSet> {
        let d = &self.logical_device.device;
        let layout = self
            .material_desc_set_layout
            .ok_or_else(|| VulkanError::Loader("material descriptor layout not created".into()))?;
        let pool = self
            .material_desc_pool
            .ok_or_else(|| VulkanError::Loader("material descriptor pool not created".into()))?;

        let layouts = [layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        // SAFETY: the free-capable material pool and layout are live handles
        // owned by `d`, and the input slice outlives this call.
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_skinned_material_ds", r))?;
        let desc_set = desc_sets.first().copied().ok_or_else(|| {
            VulkanError::Loader("skinned descriptor allocation returned no set".into())
        })?;

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
            vk::DescriptorBufferInfo::default()
                .buffer(morph_buffer)
                .offset(0)
                .range(vk::WHOLE_SIZE),
        ];
        let fallback = self
            .textures
            .get(FALLBACK_MATERIAL_TEXTURE_ID)
            .ok_or_else(|| {
                VulkanError::Loader("fallback material texture not initialized".into())
            })?;
        let image_info = [vk::DescriptorImageInfo::default()
            .sampler(fallback.sampler)
            .image_view(fallback.view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buf_info[0..1]),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buf_info[1..2]),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(3)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(4)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(5)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(6)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(desc_set)
                .dst_binding(7)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&buf_info[2..3]),
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
    // Clustered lighting buffers (set=1, bindings 2..=4)
    // ======================================================================

    /// Create the light, cluster-grid, and cluster-index storage buffers.
    ///
    /// The three buffers are installed as one transaction so the descriptor
    /// set never exposes a partially initialized clustered-lighting ABI.
    pub(crate) fn create_clustered_lighting_buffers(&mut self) -> VkResult<()> {
        if self.light_ssbo.is_some()
            && self.cluster_grid_ssbo.is_some()
            && self.cluster_index_ssbo.is_some()
        {
            return Ok(());
        }
        self.destroy_clustered_lighting_buffers();

        let light_size = (LIGHT_HEADER_SIZE + MAX_LIGHTS * LIGHT_GPU_SIZE) as u64;
        let grid_size = (MAX_CLUSTERS * 8) as u64;
        let index_size = (MAX_CLUSTER_LIGHT_INDICES * 4) as u64;
        let (light_buffer, light_allocation) =
            self.create_host_storage_buffer(light_size, "clustered-lights")?;
        let (grid_buffer, grid_allocation) =
            match self.create_host_storage_buffer(grid_size, "cluster-grid") {
                Ok(buffer) => buffer,
                Err(error) => {
                    self.destroy_host_storage_buffer(light_buffer, light_allocation);
                    return Err(error);
                }
            };
        let (index_buffer, index_allocation) =
            match self.create_host_storage_buffer(index_size, "cluster-light-indices") {
                Ok(buffer) => buffer,
                Err(error) => {
                    self.destroy_host_storage_buffer(grid_buffer, grid_allocation);
                    self.destroy_host_storage_buffer(light_buffer, light_allocation);
                    return Err(error);
                }
            };

        let descriptor_set = match self.shadow_desc_set {
            Some(descriptor_set) => descriptor_set,
            None => {
                self.destroy_host_storage_buffer(index_buffer, index_allocation);
                self.destroy_host_storage_buffer(grid_buffer, grid_allocation);
                self.destroy_host_storage_buffer(light_buffer, light_allocation);
                return Err(VulkanError::Loader(
                    "shadow descriptor set must exist before clustered lighting buffers".into(),
                ));
            }
        };
        let buffer_infos = [
            vk::DescriptorBufferInfo::default()
                .buffer(light_buffer)
                .offset(0)
                .range(light_size),
            vk::DescriptorBufferInfo::default()
                .buffer(grid_buffer)
                .offset(0)
                .range(grid_size),
            vk::DescriptorBufferInfo::default()
                .buffer(index_buffer)
                .offset(0)
                .range(index_size),
        ];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&buffer_infos[0..1]),
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(3)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&buffer_infos[1..2]),
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(4)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&buffer_infos[2..3]),
        ];
        // SAFETY: `descriptor_set` and all three storage buffers are live and
        // owned by this device; descriptor-info slices outlive the update.
        unsafe {
            self.logical_device
                .device
                .update_descriptor_sets(&writes, &[]);
        }

        self.light_ssbo = Some(light_buffer);
        self.light_ssbo_allocation = Some(light_allocation);
        self.light_ssbo_size = light_size;
        self.cluster_grid_ssbo = Some(grid_buffer);
        self.cluster_grid_ssbo_allocation = Some(grid_allocation);
        self.cluster_index_ssbo = Some(index_buffer);
        self.cluster_index_ssbo_allocation = Some(index_allocation);
        Ok(())
    }

    fn create_host_storage_buffer(
        &self,
        size: vk::DeviceSize,
        name: &'static str,
    ) -> VkResult<(vk::Buffer, crate::allocator::Allocation)> {
        let device = &self.logical_device.device;
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `device` is live and `buffer_info` describes a non-zero
        // exclusive storage buffer.
        let buffer = unsafe { device.create_buffer(&buffer_info, None) }
            .map_err(|result| VulkanError::vk("create_cluster_storage_buffer", result))?;
        // SAFETY: `buffer` was just created by `device` and remains live.
        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
        let allocation = match self
            .logical_device
            .allocator()
            .lock()
            .map_err(|error| VulkanError::Loader(format!("allocator lock: {error}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name,
                requirements,
                location: crate::allocator::MemoryLocation::CpuToGpu,
            }) {
            Ok(allocation) => allocation,
            Err(error) => {
                // SAFETY: allocation failed, so `buffer` is unbound, unused,
                // and still exclusively owned by this function.
                unsafe { device.destroy_buffer(buffer, None) };
                return Err(VulkanError::Allocation(error.to_string()));
            }
        };
        // SAFETY: the allocation was selected from `buffer`'s requirements and
        // both handles belong to `device`; no use begins before binding succeeds.
        if let Err(result) =
            // SAFETY: same contract as above; this comment is adjacent to the
            // unsafe expression used as the conditional scrutinee.
            unsafe {
                device.bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
            }
        {
            self.destroy_host_storage_buffer(buffer, allocation);
            return Err(VulkanError::vk("bind_cluster_storage_buffer", result));
        }
        Ok((buffer, allocation))
    }

    fn destroy_host_storage_buffer(
        &self,
        buffer: vk::Buffer,
        mut allocation: crate::allocator::Allocation,
    ) {
        // SAFETY: `buffer` is exclusively owned here, belongs to this device,
        // and callers stop its use before entering the destroy path.
        unsafe {
            self.logical_device.device.destroy_buffer(buffer, None);
        }
        if let Ok(mut allocator) = self.logical_device.allocator().lock() {
            allocator.free(&mut allocation);
        }
    }

    pub(crate) fn write_clustered_lighting_buffers(
        &mut self,
        light_data: &[u8],
        cluster_grid_data: &[u8],
        cluster_index_data: &[u8],
    ) {
        Self::write_mapped_buffer(&mut self.light_ssbo_allocation, light_data);
        Self::write_mapped_buffer(&mut self.cluster_grid_ssbo_allocation, cluster_grid_data);
        Self::write_mapped_buffer(&mut self.cluster_index_ssbo_allocation, cluster_index_data);
    }

    fn write_mapped_buffer(allocation: &mut Option<crate::allocator::Allocation>, data: &[u8]) {
        if let Some(slice) = allocation
            .as_mut()
            .and_then(crate::allocator::Allocation::mapped_slice_mut)
        {
            let count = data.len().min(slice.len());
            slice[..count].copy_from_slice(&data[..count]);
        }
    }

    pub(crate) fn destroy_clustered_lighting_buffers(&mut self) {
        for (buffer, allocation) in [
            (
                self.cluster_index_ssbo.take(),
                self.cluster_index_ssbo_allocation.take(),
            ),
            (
                self.cluster_grid_ssbo.take(),
                self.cluster_grid_ssbo_allocation.take(),
            ),
            (self.light_ssbo.take(), self.light_ssbo_allocation.take()),
        ] {
            if let (Some(buffer), Some(allocation)) = (buffer, allocation) {
                self.destroy_host_storage_buffer(buffer, allocation);
            }
        }
        self.light_ssbo_size = 0;
    }

    pub(crate) fn destroy_descriptor_infra(&mut self) {
        let d = &self.logical_device.device;
        // Vulkan requires buffers to be destroyed before their bound memory.
        for buf in self.frame_ubos.drain(..) {
            // SAFETY: every frame UBO was created by `d`; frame teardown waits
            // for in-flight work before descriptor infrastructure is destroyed.
            unsafe {
                d.destroy_buffer(buf, None);
            }
        }
        for mut a in self.ubo_allocations.drain(..) {
            let allocator = self.logical_device.allocator();
            match allocator.lock() {
                Ok(mut guard) => guard.free(&mut a),
                Err(poisoned) => {
                    tracing::error!(
                        target: "vulkan::descriptor",
                        "allocator mutex was poisoned while freeing a frame UBO allocation"
                    );
                    poisoned.into_inner().free(&mut a);
                }
            };
        }
        if let Some(pool) = self.desc_pool.take() {
            // SAFETY: `pool` belongs to `d`; destroying it invalidates its sets,
            // which are no longer referenced after frame teardown.
            unsafe {
                d.destroy_descriptor_pool(pool, None);
            }
        }
        if let Some(layout) = self.desc_set_layout_0.take() {
            // SAFETY: descriptor sets/pool using this layout were destroyed
            // above and `layout` is exclusively owned by this device.
            unsafe {
                d.destroy_descriptor_set_layout(layout, None);
            }
        }
    }
}
