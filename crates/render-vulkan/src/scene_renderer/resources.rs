use super::*;

impl SceneRenderer {
    pub(super) fn validate_uploaded_meshes(
        &self,
        input: &RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        let validate_mesh = |mesh_id: &str, expected_format: MeshVertexFormat| {
            let Some(mesh) = self.meshes.get(mesh_id) else {
                return Err(vec![Diagnostic::new(
                    "RV0230",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("drawable references mesh '{mesh_id}' before a successful upload"),
                )]);
            };
            if mesh.vertex_format != expected_format {
                return Err(vec![Diagnostic::new(
                    "RV0292",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "mesh '{mesh_id}' has {:?} vertices but this draw requires {:?}",
                        mesh.vertex_format, expected_format
                    ),
                )]);
            }
            let vertex_is_live = self
                .device
                .buffers
                .get(mesh.vertex_buffer.index, mesh.vertex_buffer.generation)
                .is_some();
            let index_is_live = self
                .device
                .buffers
                .get(mesh.index_buffer.index, mesh.index_buffer.generation)
                .is_some();
            if !vertex_is_live || !index_is_live {
                return Err(vec![Diagnostic::new(
                    "RV0231",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("mesh '{mesh_id}' refers to released GPU buffers"),
                )]);
            }
            Ok(())
        };
        for mesh_id in input.drawables.iter().map(|item| &item.mesh.id) {
            validate_mesh(mesh_id, MeshVertexFormat::Pbr32)?;
        }
        for mesh_id in input.skinned_items.iter().map(|item| &item.mesh.id) {
            validate_mesh(mesh_id, MeshVertexFormat::Skinned64)?;
        }
        for material_id in input
            .drawables
            .iter()
            .map(|item| &item.material)
            .chain(input.skinned_items.iter().map(|item| &item.material))
        {
            let material = self.material_binding_for_drawable(input, material_id)?;
            self.selected_material_texture_ids(&material)?;
        }
        Ok(())
    }

    /// Look up or create a bone-palette UBO buffer for the given skeleton.
    /// The buffer is sized for up to 64 Mat4 entries (4096 bytes).
    /// The buffer contents are updated with the latest bone palette data each call.
    pub(super) fn get_or_create_bone_buffer(
        &mut self,
        skeleton_id: &str,
        bone_palette: &[[f32; 16]],
    ) -> Result<vk::Buffer, Vec<Diagnostic>> {
        // Build UBO data: up to 64 Mat4 entries (64 bytes each = 4096 bytes)
        let mut ubo_data = Vec::with_capacity(4096);
        for mat in bone_palette {
            for v in mat {
                ubo_data.extend_from_slice(&v.to_ne_bytes());
            }
        }
        ubo_data.resize(4096, 0u8);

        // Check the bone-buffer cache; if found, update data and return.
        if let Some(cached) = self.bone_palette_buffers.get(skeleton_id) {
            let handle = cached.handle;
            let vk_buffer = cached.vk_buffer;
            let needs_update = cached.ubo_data != ubo_data;
            // Promote in LRU order
            if let Some(pos) = self
                .bone_palette_buffers_order
                .iter()
                .position(|k| k == skeleton_id)
            {
                self.bone_palette_buffers_order.remove(pos);
                self.bone_palette_buffers_order
                    .push(skeleton_id.to_string());
            }
            if needs_update {
                self.device.wait_idle_checked().map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0254",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot update in-flight skeleton '{skeleton_id}': {error:?}"),
                    )]
                })?;
                self.device
                    .write_buffer(handle, &ubo_data, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0255",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("write_buffer(bone UBO): {error:?}"),
                        )]
                    })?;
                if let Some(cached) = self.bone_palette_buffers.get_mut(skeleton_id) {
                    cached.ubo_data.clone_from(&ubo_data);
                }
            }
            return Ok(vk_buffer);
        }

        // Create the buffer
        let buf_desc = BufferDescriptor {
            size_bytes: 4096,
            usage_flags: render_core::BufferUsage::UNIFORM,
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("bone-{skeleton_id}")),
        };
        let buf = self.device.create_buffer(&buf_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0218",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_buffer(bone UBO): {e:?}"),
            )]
        })?;
        if let Err(error) = self.device.write_buffer(buf, &ubo_data, 0) {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0219",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("write_buffer(bone UBO): {error:?}"),
            )]);
        }

        // Resolve raw Vulkan buffer handle
        let vk_buf = self
            .device
            .buffers
            .get(buf.index, buf.generation)
            .map(|e| e.buffer)
            .unwrap_or(vk::Buffer::null());
        if vk_buf == vk::Buffer::null() {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0220",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "bone UBO buffer handle invalid",
            )]);
        }

        if self.bone_palette_buffers.len() >= MAX_BONE_PALETTES {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0279",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "bone cache capacity was not reserved before frame recording",
            )]);
        }

        self.bone_palette_buffers.insert(
            skeleton_id.to_string(),
            CachedBoneBuffer {
                handle: buf,
                vk_buffer: vk_buf,
                ubo_data,
            },
        );
        self.bone_palette_buffers_order
            .push(skeleton_id.to_string());
        Ok(vk_buf)
    }

    pub(super) fn ensure_fallback_morph_buffer(&mut self) -> Result<vk::Buffer, Vec<Diagnostic>> {
        if let Some((_, buffer)) = self.fallback_morph_buffer {
            return Ok(buffer);
        }
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: 32,
                usage_flags: render_core::BufferUsage::STORAGE,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some("morph-target-fallback".to_string()),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0324",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create fallback morph buffer: {error}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &[0; 32], 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "RV0325",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("initialize fallback morph buffer: {error}"),
            )]);
        }
        let buffer = self
            .device
            .buffers
            .get(handle.index, handle.generation)
            .map(|entry| entry.buffer)
            .unwrap_or(vk::Buffer::null());
        if buffer == vk::Buffer::null() {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "RV0325",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "fallback morph buffer has no Vulkan handle",
            )]);
        }
        self.fallback_morph_buffer = Some((handle, buffer));
        Ok(buffer)
    }

    /// Get or create a combined material + bone descriptor set for a skinned drawable.
    /// The descriptor set has:
    ///   binding=0: material UBO
    ///   bindings=1,3..=6: material textures (updated after allocation)
    ///   binding=2: bone palette UBO
    pub(super) fn get_or_create_skinned_desc_set(
        &mut self,
        material_id: &str,
        skeleton_id: &str,
        morph_target_set_id: Option<&str>,
        mat_buffer: vk::Buffer,
        bone_buffer: vk::Buffer,
        morph_buffer: vk::Buffer,
    ) -> Result<vk::DescriptorSet, Vec<Diagnostic>> {
        let cache_key = format!(
            "{material_id}:{skeleton_id}:{}",
            morph_target_set_id.unwrap_or("<none>")
        );

        // Check cache
        if let Some(entry) = self.skinned_desc_cache.get(&cache_key) {
            // Promote in LRU order
            if let Some(pos) = self
                .skinned_desc_cache_order
                .iter()
                .position(|k| k == &cache_key)
            {
                self.skinned_desc_cache_order.remove(pos);
                self.skinned_desc_cache_order.push(cache_key.clone());
            }
            return Ok(entry.desc_set);
        }

        if self.skinned_desc_cache.len() >= MAX_BONE_PALETTES {
            return Err(vec![Diagnostic::new(
                "RV0280",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "skinned descriptor capacity was not reserved before frame recording",
            )]);
        }

        // Allocate a new skinned descriptor set from the material pool
        let desc_set = self
            .device
            .allocate_skinned_material_descriptor_set(
                mat_buffer,
                MATERIAL_UBO_SIZE as u64,
                bone_buffer,
                4096,
                morph_buffer,
            )
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0221",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("allocate_skinned_material_descriptor_set: {e:?}"),
                )]
            })?;

        // Insert into cache
        self.skinned_desc_cache.insert(
            cache_key.clone(),
            BonePaletteCacheEntry {
                desc_set,
                bound_texture_ids: std::array::from_fn(|_| {
                    crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID.to_string()
                }),
            },
        );
        self.skinned_desc_cache_order.push(cache_key);

        Ok(desc_set)
    }

    // ------------------------------------------------------------------
    // Material UBO helpers
    // ------------------------------------------------------------------

    /// Parse `ParamBlock` bytes into a [`MaterialUBO`].
    ///
    /// Expected byte layout (matching the shader's MaterialUBO):
    ///   [0..16)  base_color  - vec4 f32
    ///   [16..20) metallic    - f32
    ///   [20..24) roughness   - f32
    ///   [24..28) ao          - f32
    ///
    /// If `bytes` is empty or too short, sane defaults are used.
    pub(super) fn parse_material_ubo(bytes: &[u8]) -> MaterialUBO {
        let read_f32 = |offset: usize, fallback: f32| -> f32 {
            if offset + 4 <= bytes.len() {
                let value = f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
                if value.is_finite() {
                    value
                } else {
                    fallback
                }
            } else {
                fallback
            }
        };
        let read_vec4 = |offset: usize, fallback: [f32; 4]| -> [f32; 4] {
            if offset + 16 <= bytes.len() {
                [
                    f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap()),
                    f32::from_ne_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()),
                    f32::from_ne_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()),
                    f32::from_ne_bytes(bytes[offset + 12..offset + 16].try_into().unwrap()),
                ]
            } else {
                fallback
            }
        };
        MaterialUBO {
            base_color: read_vec4(0, [0.8, 0.6, 0.4, 1.0]),
            metallic: read_f32(16, 0.0).clamp(0.0, 1.0),
            roughness: read_f32(20, 1.0).clamp(0.04, 1.0),
            ao: read_f32(24, 1.0).clamp(0.0, 1.0),
            alpha_cutoff: read_f32(28, -1.0),
            emissive: read_vec4(32, [0.0; 4]),
            advanced0: read_vec4(48, [0.0, 0.2, 0.0, 0.0]),
            subsurface_color: read_vec4(64, [1.0, 0.35, 0.25, 0.0]),
            sheen_color: read_vec4(80, [0.0; 4]),
            rim_color_power: read_vec4(96, [0.0, 0.0, 0.0, 3.0]),
        }
    }

    pub(super) fn material_texture_flags(material: &MaterialBinding) -> f32 {
        MATERIAL_TEXTURE_BINDINGS
            .iter()
            .enumerate()
            .fold(0_u32, |flags, (index, binding)| {
                flags
                    | (u32::from(
                        material
                            .textures
                            .iter()
                            .any(|slot| slot.binding == *binding),
                    ) << index)
            }) as f32
    }

    pub(super) fn selected_material_texture_ids(
        &self,
        material: &MaterialBinding,
    ) -> Result<[String; 5], Vec<Diagnostic>> {
        material_texture_ids_for_descriptor(material, |texture_id| {
            self.device.textures.contains_key(texture_id)
        })
        .map_err(|diagnostic| vec![*diagnostic])
    }

    pub(super) fn bind_material_texture_if_changed(
        &mut self,
        material_id: &str,
        material: &MaterialBinding,
        descriptor_set: vk::DescriptorSet,
    ) -> Result<(), Vec<Diagnostic>> {
        let selected = self.selected_material_texture_ids(material)?;
        let current = self
            .material_cache
            .get(material_id)
            .map(|entry| entry.bound_texture_ids.clone())
            .unwrap_or_else(|| std::array::from_fn(|_| String::new()));
        if current == selected {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0261",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot update in-flight material texture: {error:?}"),
            )]
        })?;
        for (index, binding) in MATERIAL_TEXTURE_BINDINGS.into_iter().enumerate() {
            if current[index] == selected[index] {
                continue;
            }
            let bound = self
                .device
                .bind_material_texture_at(&selected[index], binding, descriptor_set)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0262",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "bind material texture '{}' at binding {binding}: {error:?}",
                            selected[index]
                        ),
                    )]
                })?;
            if !bound {
                return Err(vec![Diagnostic::new(
                    "RV0263",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "material texture '{}' disappeared before descriptor update",
                        selected[index]
                    ),
                )]);
            }
        }
        if let Some(entry) = self.material_cache.get_mut(material_id) {
            entry.bound_texture_ids = selected;
        }
        Ok(())
    }

    pub(super) fn bind_skinned_texture_if_changed(
        &mut self,
        cache_key: &str,
        material: &MaterialBinding,
        descriptor_set: vk::DescriptorSet,
    ) -> Result<(), Vec<Diagnostic>> {
        let selected = self.selected_material_texture_ids(material)?;
        let current = self
            .skinned_desc_cache
            .get(cache_key)
            .map(|entry| entry.bound_texture_ids.clone())
            .unwrap_or_else(|| std::array::from_fn(|_| String::new()));
        if current == selected {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0264",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot update in-flight skinned texture: {error:?}"),
            )]
        })?;
        for (index, binding) in MATERIAL_TEXTURE_BINDINGS.into_iter().enumerate() {
            if current[index] == selected[index] {
                continue;
            }
            let bound = self
                .device
                .bind_material_texture_at(&selected[index], binding, descriptor_set)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0265",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "bind skinned texture '{}' at binding {binding}: {error:?}",
                            selected[index]
                        ),
                    )]
                })?;
            if !bound {
                return Err(vec![Diagnostic::new(
                    "RV0266",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "skinned texture '{}' disappeared before descriptor update",
                        selected[index]
                    ),
                )]);
            }
        }
        if let Some(entry) = self.skinned_desc_cache.get_mut(cache_key) {
            entry.bound_texture_ids = selected;
        }
        Ok(())
    }

    /// Look up or create a material descriptor set + buffer for the given
    /// material.  Uses a LRU eviction policy capped at [`MAX_MATERIALS`].
    pub(super) fn get_or_create_material_desc_set(
        &mut self,
        material_id: &str,
        ubo_data: &[u8],
    ) -> Result<(vk::DescriptorSet, vk::Buffer), Vec<Diagnostic>> {
        let ubo_array: [u8; MATERIAL_UBO_SIZE] = ubo_data.try_into().map_err(|_| {
            vec![Diagnostic::new(
                "RV0250",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("material UBO must be exactly {MATERIAL_UBO_SIZE} bytes"),
            )]
        })?;
        // Check cache first (and move to front for LRU)
        if let Some(entry) = self.material_cache.get(material_id) {
            let desc_set = entry.desc_set;
            let buffer = entry.buffer;
            let handle = entry.handle;
            let old_data = entry.ubo_data;
            // Promote in LRU order (simple move-to-front)
            if let Some(pos) = self
                .material_cache_order
                .iter()
                .position(|k| k == material_id)
            {
                self.material_cache_order.remove(pos);
                self.material_cache_order.push(material_id.to_string());
            }
            if old_data.as_slice() != ubo_data {
                self.device.wait_idle_checked().map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0248",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot update in-flight material '{material_id}': {error:?}"),
                    )]
                })?;
                self.device
                    .write_buffer(handle, ubo_data, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0249",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("write_buffer(material UBO): {error:?}"),
                        )]
                    })?;
                if let Some(entry) = self.material_cache.get_mut(material_id) {
                    entry.ubo_data.copy_from_slice(ubo_data);
                }
            }
            return Ok((desc_set, buffer));
        }

        if self.material_cache.len() >= MAX_MATERIALS {
            return Err(vec![Diagnostic::new(
                "RV0281",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "material cache capacity was not reserved before frame recording",
            )]);
        }

        // Create a small UBO buffer for MaterialUBO.
        let buf_desc = BufferDescriptor {
            size_bytes: MATERIAL_UBO_SIZE as u64,
            usage_flags: render_core::BufferUsage::UNIFORM,
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("mat-ubo-{material_id}")),
        };
        let buf = self.device.create_buffer(&buf_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0214",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_buffer(material UBO): {e:?}"),
            )]
        })?;
        if let Err(error) = self.device.write_buffer(buf, ubo_data, 0) {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0215",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("write_buffer(material UBO): {error:?}"),
            )]);
        }

        // Resolve raw Vulkan buffer handle for the descriptor set
        let vk_buf = self
            .device
            .buffers
            .get(buf.index, buf.generation)
            .map(|e| e.buffer)
            .unwrap_or(vk::Buffer::null());
        if vk_buf == vk::Buffer::null() {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0216",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "material UBO buffer handle invalid",
            )]);
        }

        // Allocate and update descriptor set via the device
        let desc_set = match self
            .device
            .allocate_material_descriptor_set(vk_buf, MATERIAL_UBO_SIZE as u64)
        {
            Ok(desc_set) => desc_set,
            Err(error) => {
                self.device.destroy_buffer(buf);
                return Err(vec![Diagnostic::new(
                    "RV0217",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("allocate_material_descriptor_set: {error:?}"),
                )]);
            }
        };

        let entry = MaterialCacheEntry {
            desc_set,
            handle: buf,
            buffer: vk_buf,
            ubo_data: ubo_array,
            bound_texture_ids: std::array::from_fn(|_| {
                crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID.to_string()
            }),
        };
        self.material_cache.insert(material_id.to_string(), entry);
        self.material_cache_order.push(material_id.to_string());

        Ok((desc_set, vk_buf))
    }

    pub(super) fn evict_material_by_id(
        &mut self,
        material_id: &str,
    ) -> Result<(), Vec<Diagnostic>> {
        if !self.material_cache.contains_key(material_id) {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0251",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot evict in-flight material '{material_id}': {error:?}"),
            )]
        })?;

        let skinned_prefix = format!("{material_id}:");
        let skinned_keys: Vec<String> = self
            .skinned_desc_cache
            .keys()
            .filter(|key| key.starts_with(&skinned_prefix))
            .cloned()
            .collect();
        for key in skinned_keys {
            if let Some(entry) = self.skinned_desc_cache.remove(&key) {
                self.device
                    .free_material_descriptor_set(entry.desc_set)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0252",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("free skinned descriptor set: {error:?}"),
                        )]
                    })?;
            }
        }
        self.skinned_desc_cache_order
            .retain(|key| !key.starts_with(&skinned_prefix));

        self.material_cache_order.retain(|key| key != material_id);
        if let Some(entry) = self.material_cache.remove(material_id) {
            self.device
                .free_material_descriptor_set(entry.desc_set)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0253",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("free material descriptor set: {error:?}"),
                    )]
                })?;
            self.device.destroy_buffer(entry.handle);
        }
        Ok(())
    }

    pub(super) fn evict_skinned_descriptor_by_key(
        &mut self,
        cache_key: &str,
    ) -> Result<(), Vec<Diagnostic>> {
        if !self.skinned_desc_cache.contains_key(cache_key) {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0276",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot evict in-flight skinned descriptor: {error:?}"),
            )]
        })?;
        self.skinned_desc_cache_order.retain(|key| key != cache_key);
        if let Some(entry) = self.skinned_desc_cache.remove(cache_key) {
            self.device
                .free_material_descriptor_set(entry.desc_set)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0277",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("free skinned descriptor set: {error:?}"),
                    )]
                })?;
        }
        Ok(())
    }

    pub(super) fn evict_skeleton_by_id(
        &mut self,
        skeleton_id: &str,
    ) -> Result<(), Vec<Diagnostic>> {
        if !self.bone_palette_buffers.contains_key(skeleton_id) {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0278",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot evict in-flight skeleton '{skeleton_id}': {error:?}"),
            )]
        })?;
        let skeleton_marker = format!(":{skeleton_id}:");
        let descriptor_keys: Vec<String> = self
            .skinned_desc_cache
            .keys()
            .filter(|key| key.contains(&skeleton_marker))
            .cloned()
            .collect();
        for key in descriptor_keys {
            self.evict_skinned_descriptor_by_key(&key)?;
        }
        self.bone_palette_buffers_order
            .retain(|key| key != skeleton_id);
        if let Some(entry) = self.bone_palette_buffers.remove(skeleton_id) {
            self.device.destroy_buffer(entry.handle);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Mesh caching
    // ------------------------------------------------------------------

    // ------------------------------------------------------------------
    // Frame lifecycle helpers
    // ------------------------------------------------------------------
}
