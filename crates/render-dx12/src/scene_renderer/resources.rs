use super::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12SceneRenderer {
    pub(super) fn prepare_vertex_draw_arena(
        &mut self,
        input: &RenderFrameInput,
    ) -> Result<Option<BufferHandle>, Vec<Diagnostic>> {
        let bytes = vertex_draw_arena_constants(input.drawables.iter().map(|drawable| {
            (
                drawable.radial_vertex_morph.as_ref(),
                drawable.triplanar_material_mapping.as_ref(),
            )
        }));
        if bytes.is_empty() {
            return Ok(None);
        }
        if let Some(existing) = self.vertex_draw_buffer.as_mut() {
            if existing.capacity >= bytes.len() {
                if existing.bytes != bytes {
                    self.device
                        .write_buffer(existing.handle, &bytes, 0)
                        .map_err(|error| {
                            vec![Diagnostic::new(
                                "DX1285",
                                DiagnosticSeverity::Error,
                                "scene_renderer",
                                format!("update DX12 vertex-draw arena failed: {error:?}"),
                            )]
                        })?;
                    existing.bytes = bytes;
                }
                return Ok(Some(existing.handle));
            }
        }
        // Grow below; the old arena remains valid until replacement
        // allocation and upload both succeed.
        let capacity = bytes
            .len()
            .next_power_of_two()
            .max(VERTEX_DRAW_CONSTANT_STRIDE);
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: capacity as u64,
                usage_flags: render_core::BufferUsage::UNIFORM,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some("vertex-draw-arena".to_string()),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1284",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create DX12 vertex-draw arena failed: {error:?}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1285",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload DX12 vertex-draw arena failed: {error:?}"),
            )]);
        }
        let old = self.vertex_draw_buffer.replace(Dx12DynamicVertexBuffer {
            handle,
            bytes,
            capacity,
        });
        if let Some(old) = old {
            self.device.destroy_buffer(old.handle);
        }
        Ok(Some(handle))
    }

    pub(super) fn vertex_draw_binding(&self, drawable_index: usize) -> Option<(BufferHandle, u64)> {
        Some((
            self.vertex_draw_buffer.as_ref()?.handle,
            vertex_draw_constant_offset(drawable_index)?,
        ))
    }

    pub(super) fn material_surface(
        &self,
        input: &RenderFrameInput,
        material_id: &engine_serialize::AssetId,
    ) -> (Transparency, bool) {
        input
            .materials
            .iter()
            .find(|binding| binding.material_id == *material_id)
            .map(|binding| (binding.transparency.clone(), binding.double_sided))
            .or_else(|| {
                self.materials
                    .get(&material_id.id)
                    .map(|material| (material.transparency.clone(), material.double_sided))
            })
            .unwrap_or((Transparency::Opaque, false))
    }

    pub(super) fn material_texture_ids(
        &self,
        input: &RenderFrameInput,
        material_id: &engine_serialize::AssetId,
    ) -> [Option<String>; 5] {
        input
            .materials
            .iter()
            .find(|binding| binding.material_id == *material_id)
            .map(|material| {
                MATERIAL_TEXTURE_BINDINGS.map(|binding| {
                    material
                        .textures
                        .iter()
                        .find(|slot| slot.binding == binding)
                        .map(|slot| slot.texture.id.clone())
                })
            })
            .or_else(|| {
                self.materials
                    .get(&material_id.id)
                    .map(|material| material.texture_ids.clone())
            })
            .unwrap_or_else(|| std::array::from_fn(|_| None))
    }

    pub(super) fn material_texture_table(
        &self,
        texture_ids: &[Option<String>; 5],
        shadow_texture: TextureHandle,
        environment_texture: TextureHandle,
    ) -> [TextureHandle; 7] {
        let resolve = |texture_id: &Option<String>| {
            texture_id
                .as_ref()
                .and_then(|texture_id| self.textures.get(texture_id))
                .map(|texture| texture.handle)
                .unwrap_or(shadow_texture)
        };
        [
            resolve(&texture_ids[0]),
            shadow_texture,
            resolve(&texture_ids[1]),
            resolve(&texture_ids[2]),
            resolve(&texture_ids[3]),
            resolve(&texture_ids[4]),
            environment_texture,
        ]
    }

    pub(super) fn environment_binding(
        &self,
        input: &RenderFrameInput,
        camera_position: glam::Vec3,
    ) -> Result<(TextureHandle, [u8; 16]), Vec<Diagnostic>> {
        let selected = select_environment_map(&input.render_options.environment, camera_position);
        let Some(environment_id) = selected else {
            let fallback = self.fallback_environment.ok_or_else(|| {
                vec![Diagnostic::new(
                    "DX1260",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 fallback environment is unavailable",
                )]
            })?;
            return Ok((fallback, float4_bytes([0.0, 0.0, 0.0, 0.0])));
        };
        let environment = self.environments.get(&environment_id.id).ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1259",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "environment map '{}' was selected before a successful DX12 upload",
                    environment_id.id
                ),
            )]
        })?;
        Ok((
            environment.handle,
            float4_bytes([
                input.render_options.environment.intensity,
                input.render_options.environment.rotation_radians,
                environment.mip_count.saturating_sub(1) as f32,
                1.0,
            ]),
        ))
    }

    pub(super) fn prepare_bone_buffer(
        &mut self,
        cache_key: &str,
        palette: &[[f32; 16]],
    ) -> Result<BufferHandle, Vec<Diagnostic>> {
        let bytes = bone_palette_constants(palette);
        if let Some(existing) = self.bone_buffers.get_mut(cache_key) {
            if existing.bytes != bytes {
                self.device
                    .write_buffer(existing.handle, &bytes, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "DX1219",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("update DX12 bone palette failed: {error:?}"),
                        )]
                    })?;
                existing.bytes = bytes;
            }
            return Ok(existing.handle);
        }
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: bytes.len() as u64,
                usage_flags: render_core::BufferUsage::UNIFORM,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("bones-{cache_key}")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1218",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create DX12 bone palette failed: {error:?}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1219",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload DX12 bone palette failed: {error:?}"),
            )]);
        }
        self.bone_buffers
            .insert(cache_key.to_owned(), Dx12BoneBuffer { handle, bytes });
        Ok(handle)
    }

    pub(super) fn prepare_morphed_vertex_buffer(
        &mut self,
        cache_key: &str,
        mesh: &Dx12GpuMesh,
        target_set_id: &engine_serialize::AssetId,
        weights: &[f32],
    ) -> Result<BufferHandle, Vec<Diagnostic>> {
        let target_set = self
            .morph_target_sets
            .get(&target_set_id.id)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "DX1263",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "morph target set '{}' was referenced before a successful DX12 upload",
                        target_set_id.id
                    ),
                )]
            })?;
        if target_set.vertex_count != mesh.vertex_count {
            return Err(vec![Diagnostic::new(
                "DX1264",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "morph target set '{}' has {} vertices but the skinned mesh has {}",
                    target_set_id.id, target_set.vertex_count, mesh.vertex_count
                ),
            )]);
        }
        if weights.iter().all(|weight| weight.abs() <= f32::EPSILON) {
            return Ok(mesh.vertex_buffer);
        }
        let stride = mesh.vertex_format.stride_bytes() as usize;
        let mut bytes = mesh.vertex_bytes.clone();
        for vertex_index in 0..mesh.vertex_count as usize {
            let base = vertex_index * stride;
            let read_vec3 = |source: &[u8], offset: usize| {
                glam::Vec3::new(
                    f32::from_ne_bytes(source[offset..offset + 4].try_into().unwrap()),
                    f32::from_ne_bytes(source[offset + 4..offset + 8].try_into().unwrap()),
                    f32::from_ne_bytes(source[offset + 8..offset + 12].try_into().unwrap()),
                )
            };
            let mut position = read_vec3(&mesh.vertex_bytes, base);
            let mut normal = read_vec3(&mesh.vertex_bytes, base + 12);
            for (target, weight) in target_set.targets.iter().zip(weights.iter().copied()) {
                position += glam::Vec3::from_array(target.position_deltas[vertex_index]) * weight;
                normal += glam::Vec3::from_array(target.normal_deltas[vertex_index]) * weight;
            }
            if normal.length_squared() > 1.0e-12 {
                normal = normal.normalize();
            }
            for (offset, value) in position
                .to_array()
                .into_iter()
                .chain(normal.to_array())
                .enumerate()
            {
                let start = base + offset * 4;
                bytes[start..start + 4].copy_from_slice(&value.to_ne_bytes());
            }
        }
        if let Some(existing) = self.morphed_vertex_buffers.get_mut(cache_key) {
            if existing.bytes != bytes {
                self.device
                    .write_buffer(existing.handle, &bytes, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "DX1266",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("update DX12 morphed vertex buffer failed: {error:?}"),
                        )]
                    })?;
                existing.bytes = bytes;
            }
            return Ok(existing.handle);
        }
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: bytes.len() as u64,
                usage_flags: render_core::BufferUsage::VERTEX,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("morph-{cache_key}")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1265",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create DX12 morphed vertex buffer failed: {error:?}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1266",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload DX12 morphed vertex buffer failed: {error:?}"),
            )]);
        }
        if let Err(error) = self
            .device
            .set_vertex_stride(handle, mesh.vertex_format.stride_bytes())
        {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1267",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("set DX12 morphed vertex stride failed: {error:?}"),
            )]);
        }
        self.morphed_vertex_buffers.insert(
            cache_key.to_owned(),
            Dx12DynamicVertexBuffer {
                handle,
                capacity: bytes.len(),
                bytes,
            },
        );
        Ok(handle)
    }

    pub(super) fn clear_morphed_vertex_buffers(&mut self) {
        for (_, buffer) in self.morphed_vertex_buffers.drain() {
            self.device.destroy_buffer(buffer.handle);
        }
    }

    pub(super) fn prepare_particle_instance_buffer(
        &mut self,
        cache_key: &str,
        instances: &[engine_renderer::ParticleInstance],
    ) -> Result<Option<BufferHandle>, Vec<Diagnostic>> {
        if instances.is_empty() {
            return Ok(None);
        }
        let mut bytes = Vec::with_capacity(instances.len() * 32);
        for instance in instances {
            for value in [
                instance.position[0],
                instance.position[1],
                instance.position[2],
                instance.size,
                instance.rotation_radians,
                instance.normalized_age,
            ] {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
            bytes.extend_from_slice(&instance.color);
            bytes.extend_from_slice(&0_u32.to_ne_bytes());
        }
        if let Some(existing) = self.particle_instance_buffers.get_mut(cache_key) {
            if existing.capacity >= bytes.len() {
                if existing.bytes != bytes {
                    self.device
                        .write_buffer(existing.handle, &bytes, 0)
                        .map_err(|error| {
                            vec![Diagnostic::new(
                                "DX1270",
                                DiagnosticSeverity::Error,
                                "scene_renderer",
                                format!("update DX12 particle instance stream failed: {error:?}"),
                            )]
                        })?;
                    existing.bytes = bytes;
                }
                return Ok(Some(existing.handle));
            }
        }
        if let Some(old) = self.particle_instance_buffers.remove(cache_key) {
            self.device.destroy_buffer(old.handle);
        }
        let capacity = bytes.len().next_power_of_two();
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: capacity as u64,
                usage_flags: render_core::BufferUsage::VERTEX,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("particles-{cache_key}")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1269",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create DX12 particle instance stream failed: {error:?}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1270",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload DX12 particle instance stream failed: {error:?}"),
            )]);
        }
        if let Err(error) = self.device.set_vertex_stride(handle, 32) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1271",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("set DX12 particle instance stride failed: {error:?}"),
            )]);
        }
        self.particle_instance_buffers.insert(
            cache_key.to_owned(),
            Dx12DynamicVertexBuffer {
                handle,
                bytes,
                capacity,
            },
        );
        Ok(Some(handle))
    }

    pub(super) fn clear_particle_instance_buffers(&mut self) {
        for (_, buffer) in self.particle_instance_buffers.drain() {
            self.device.destroy_buffer(buffer.handle);
        }
    }

    pub(super) fn prepare_gpu_particle_parameter_buffer(
        &mut self,
        cache_key: &str,
        simulation: engine_renderer::GpuParticleSimulation,
    ) -> Result<BufferHandle, Vec<Diagnostic>> {
        let bytes = simulation.parameter_bytes().to_vec();
        if let Some(existing) = self.gpu_particle_parameter_buffers.get_mut(cache_key) {
            if existing.bytes != bytes {
                self.device
                    .write_buffer(existing.handle, &bytes, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "DX1286",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("update DX12 GPU-particle parameters failed: {error:?}"),
                        )]
                    })?;
                existing.bytes = bytes;
            }
            return Ok(existing.handle);
        }
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: engine_renderer::GPU_PARTICLE_PARAMETER_SIZE as u64,
                usage_flags: render_core::BufferUsage::STORAGE,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("gpu-particles-{cache_key}")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1286",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create DX12 GPU-particle parameters failed: {error:?}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1286",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload DX12 GPU-particle parameters failed: {error:?}"),
            )]);
        }
        self.gpu_particle_parameter_buffers.insert(
            cache_key.to_owned(),
            Dx12DynamicVertexBuffer {
                handle,
                bytes,
                capacity: engine_renderer::GPU_PARTICLE_PARAMETER_SIZE,
            },
        );
        Ok(handle)
    }

    pub(super) fn prepare_clustered_light_buffers(
        &mut self,
        input: &RenderFrameInput,
        view: &engine_renderer::RenderView,
    ) -> Result<[BufferHandle; 4], Vec<Diagnostic>> {
        let light_refs = input.lights.iter().collect::<Vec<_>>();
        let clustered = engine_renderer::build_clustered_light_frame(
            &light_refs,
            view,
            self.width,
            self.height,
        );
        let light = update_dynamic_storage_buffer(
            &mut self.device,
            &mut self.clustered_light_buffer,
            &clustered.light_bytes,
            "dx12-clustered-lights",
        );
        let grid = update_dynamic_storage_buffer(
            &mut self.device,
            &mut self.clustered_grid_buffer,
            &clustered.cluster_grid_bytes,
            "dx12-cluster-grid",
        );
        let indices = update_dynamic_storage_buffer(
            &mut self.device,
            &mut self.clustered_index_buffer,
            &clustered.cluster_index_bytes,
            "dx12-cluster-indices",
        );
        let particle = update_dynamic_storage_buffer(
            &mut self.device,
            &mut self.gpu_particle_dummy_buffer,
            &[0_u8; engine_renderer::GPU_PARTICLE_PARAMETER_SIZE],
            "dx12-gpu-particle-dummy",
        );
        match (light, grid, indices, particle) {
            (Ok(light), Ok(grid), Ok(indices), Ok(particle)) => {
                Ok([light, grid, indices, particle])
            }
            results => Err(vec![Diagnostic::new(
                "DX1285",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "prepare DX12 scene storage buffers failed: light={:?}, grid={:?}, indices={:?}, particles={:?}",
                    results.0.err(),
                    results.1.err(),
                    results.2.err(),
                    results.3.err()
                ),
            )]),
        }
    }
}
