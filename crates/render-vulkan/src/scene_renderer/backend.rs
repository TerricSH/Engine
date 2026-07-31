use super::*;

impl BackendRenderer for SceneRenderer {
    fn supports_weighted_blended_oit(&self) -> bool {
        true
    }

    fn supports_gpu_particle_simulation(&self) -> bool {
        true
    }

    fn configure_render_graph(
        &mut self,
        _input: &RenderFrameInput,
        graph: &mut engine_renderer::render_graph2::RenderGraph,
    ) -> Result<(), Vec<Diagnostic>> {
        apply_registered_custom_pass_declarations(&self.pass_registry, graph)
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &render_graph2::PassNode,
        barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        let fi = self.device.current_frame;
        self.device
            .apply_render_graph_barriers(fi, barriers)
            .map_err(|message| {
                vec![Diagnostic::new(
                    "RV0316",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    message,
                )]
            })
    }

    fn begin_frame(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        validate_vulkan_frame_contract(input)?;
        if self.cur_enc.is_some() || self.cur_sc.is_some() || self.cur_ii.is_some() {
            return Err(vec![Diagnostic::new(
                "RV0269",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "begin_frame called while another frame is active",
            )]);
        }
        if input.views.len() > 1 {
            return Err(vec![Diagnostic::new(
                "RV0290",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "Vulkan backend currently supports at most one render view, received {}",
                    input.views.len()
                ),
            )]);
        }
        let msaa = match input.render_options.msaa_samples {
            2 => vk::SampleCountFlags::TYPE_2,
            4 => vk::SampleCountFlags::TYPE_4,
            8 => vk::SampleCountFlags::TYPE_8,
            _ => vk::SampleCountFlags::TYPE_1,
        };
        let (sc_h, ii, enc) = match self.begin_frame_impl(input, msaa) {
            Ok(frame) => frame,
            Err(mut diagnostics) => {
                if let Err(mut recovery_diagnostics) = self.recover_failed_device_frame() {
                    diagnostics.append(&mut recovery_diagnostics);
                }
                return Err(diagnostics);
            }
        };
        self.cur_sc = Some(sc_h);
        self.cur_ii = Some(ii);
        self.cur_enc = Some(enc);
        // The device begin-frame already waited for this slot's in-flight
        // fence, so the previous frame recorded on the slot can be read back
        // without stalling; then the slot's pool is reset for this frame.
        self.gpu_timestamps_begin_frame(input.frame_index);
        Ok(())
    }

    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &render_graph2::PassNode,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if self.cur_enc.is_none() {
            return Err(vec![Diagnostic::new(
                "RV0224",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "execute_pass called without an active frame encoder",
            )]);
        }

        // Built-in passes own backend-specific Vulkan resources and must be
        // dispatched directly. In particular, the tone-map pass performs the
        // render-pass transition from UNDEFINED to PRESENT_SRC_KHR for the
        // acquired swapchain image. Custom passes continue to use the
        // pluggable registry.
        //
        // Every pass is bracketed by GPU timestamp writes (start at
        // TOP_OF_PIPE, end at BOTTOM_OF_PIPE) into the current slot's pool.
        let stamp_device = self.device.logical_device.device.clone();
        let stamp_cmd = self.device.frame_sync[self.device.current_frame].command_buffer;
        if let Some((query, slot)) = self.gpu_timestamp_pass_start(pass.name) {
            self.timestamp_pools.cmd_write(
                &stamp_device,
                stamp_cmd,
                query,
                slot,
                vk::PipelineStageFlags::TOP_OF_PIPE,
            );
        }
        let pass_result = match pass.kind {
            render_graph2::PassKind::OpaquePbrForward => {
                self.execute_hdr_forward_pass(input, stats)
            }
            render_graph2::PassKind::DirectionalShadow => self.execute_shadow_pass(input, stats),
            render_graph2::PassKind::ToneMap => self.execute_tonemap_pass(input, stats),
            render_graph2::PassKind::Present => {
                self.execute_ui_overlay_pass(&input.ui_batches, stats)
            }
            render_graph2::PassKind::Custom(name) => {
                let enc = self.cur_enc.as_mut().expect("encoder checked above");
                execute_registered_custom_pass(
                    &mut self.pass_registry,
                    name,
                    input,
                    &mut **enc,
                    stats,
                )
            }
        };
        if let Some((query, slot)) = self.gpu_timestamp_pass_end() {
            self.timestamp_pools.cmd_write(
                &stamp_device,
                stamp_cmd,
                query,
                slot,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            );
        }
        pass_result
    }

    fn end_frame(&mut self, stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        match (self.cur_sc.take(), self.cur_ii.take(), self.cur_enc.take()) {
            (Some(sc_h), Some(ii), Some(enc)) => {
                // SAFETY: the encoder was created by `begin_frame` and is still
                // valid; `end_frame` takes ownership and submits the command
                // buffer that has been recorded into during `execute_pass`.
                let s = match self.device.end_frame(sc_h, enc, ii) {
                    Ok(stats) => stats,
                    Err(error) => {
                        self.gpu_timestamps.abort_slot(self.device.current_frame);
                        let mut diagnostics = vec![Diagnostic::new(
                            "RV0209",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("end_frame: {error:?}"),
                        )];
                        if let Err(mut recovery_diagnostics) = self.recover_failed_device_frame() {
                            diagnostics.append(&mut recovery_diagnostics);
                        }
                        return Err(diagnostics);
                    }
                };
                // The frame was submitted: its timestamp queries are now
                // pending asynchronous read-back. Publish whatever batch the
                // begin-frame read-back produced into the statistics.
                self.gpu_timestamps_end_frame(stats);
                // Built-in Vulkan passes issue several draws directly because
                // they need backend-specific descriptor and render-pass state.
                // The generic encoder only accounts for the draws recorded
                // through its own methods (for example the tone-map pass), so
                // replacing the pass totals here would erase the scene work.
                stats.draw_calls = stats.draw_calls.saturating_add(s.draw_calls);
                stats.triangles = stats.triangles.saturating_add(s.triangles);
                stats.gpu_frame_ms = s.gpu_frame_ms;
            }
            (None, None, None) => {
                return Err(vec![Diagnostic::new(
                    "RV0267",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "end_frame called without an active frame",
                )]);
            }
            _ => {
                let mut diagnostics = vec![Diagnostic::new(
                    "RV0268",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "Vulkan frame state is internally inconsistent",
                )];
                if let Err(mut recovery_diagnostics) = self.recover_failed_device_frame() {
                    diagnostics.append(&mut recovery_diagnostics);
                }
                return Err(diagnostics);
            }
        }
        Ok(())
    }

    fn upload_mesh(&mut self, upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        let mesh_id = upload.mesh_id.id.clone();
        if let Some(existing) = self.meshes.get(&mesh_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .meshes
            .get(&mesh_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));

        let vb_usage = render_core::BufferUsage(
            render_core::BufferUsage::VERTEX.0 | render_core::BufferUsage::COPY_DST.0,
        );
        let vb_desc = render_core::BufferDescriptor {
            size_bytes: upload.vertex_bytes.len() as u64,
            usage_flags: vb_usage,
            memory_hint: render_core::MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-vertices")),
        };
        let vb = self.device.create_buffer(&vb_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh create_buffer(vertices): {e:?}"),
            )]
        })?;
        if let Err(error) = self.device.write_buffer(vb, &upload.vertex_bytes, 0) {
            self.device.destroy_buffer(vb);
            return Err(vec![Diagnostic::new(
                "RV0204",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(vertices): {error:?}"),
            )]);
        }

        let ib_usage = render_core::BufferUsage(
            render_core::BufferUsage::INDEX.0 | render_core::BufferUsage::COPY_DST.0,
        );
        let ib_desc = render_core::BufferDescriptor {
            size_bytes: upload.index_bytes.len() as u64,
            usage_flags: ib_usage,
            memory_hint: render_core::MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-indices")),
        };
        let ib = match self.device.create_buffer(&ib_desc) {
            Ok(buffer) => buffer,
            Err(error) => {
                self.device.destroy_buffer(vb);
                return Err(vec![Diagnostic::new(
                    "RV0205",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("upload_mesh create_buffer(indices): {error:?}"),
                )]);
            }
        };
        if let Err(error) = self.device.write_buffer(ib, &upload.index_bytes, 0) {
            self.device.destroy_buffer(ib);
            self.device.destroy_buffer(vb);
            return Err(vec![Diagnostic::new(
                "RV0206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(indices): {error:?}"),
            )]);
        }

        let index_format = upload.index_format;
        let mesh = GpuMesh {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count: upload.index_count,
            vertex_count: upload.vertex_count,
            index_format,
            vertex_format: upload.vertex_format,
            content_hash: upload.content_hash,
            revision,
        };

        if self.meshes.contains_key(&mesh_id) {
            if let Err(error) = self.device.wait_idle_checked() {
                self.device.destroy_buffer(vb);
                self.device.destroy_buffer(ib);
                return Err(vec![Diagnostic::new(
                    "RV0235",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("cannot replace in-flight mesh '{mesh_id}': {error:?}"),
                )]);
            }
        }
        if let Some(old) = self.meshes.insert(mesh_id, mesh) {
            self.device.destroy_buffer(old.vertex_buffer);
            self.device.destroy_buffer(old.index_buffer);
        }
        Ok(UploadReceipt::new(revision))
    }

    fn upload_texture(&mut self, upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        use crate::device_impl::reload::{
            SampledTextureAddressMode, SampledTextureColorSpace, SampledTextureDescriptor,
            SampledTextureFilter, SampledTextureSamplerDescriptor,
        };
        use crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID;

        let texture_id = upload.texture_id.id.clone();
        if texture_id == FALLBACK_MATERIAL_TEXTURE_ID {
            return Err(vec![Diagnostic::new(
                "RV0236",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "the renderer fallback texture ID is reserved",
            )]);
        }
        if let Some(existing) = self.texture_uploads.get(&texture_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .texture_uploads
            .get(&texture_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        let mut mip_bytes = Vec::new();
        for mip in &upload.mip_levels {
            mip_bytes.extend_from_slice(&mip.bytes);
        }
        let map_filter = |filter| match filter {
            SamplerFilter::Nearest => SampledTextureFilter::Nearest,
            SamplerFilter::Linear => SampledTextureFilter::Linear,
        };
        let map_address = |address| match address {
            SamplerAddressMode::Repeat => SampledTextureAddressMode::Repeat,
            SamplerAddressMode::ClampToEdge => SampledTextureAddressMode::ClampToEdge,
            SamplerAddressMode::MirroredRepeat => SampledTextureAddressMode::MirroredRepeat,
        };
        let descriptor = SampledTextureDescriptor::rgba8(
            upload.width,
            upload.height,
            u8::try_from(upload.mip_levels.len()).map_err(|_| {
                vec![Diagnostic::new(
                    "RV0237",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "texture mip count exceeds the Vulkan upload contract",
                )]
            })?,
            &mip_bytes,
            match upload.color_space {
                engine_renderer::ColorSpace::Linear => SampledTextureColorSpace::Linear,
                engine_renderer::ColorSpace::Srgb => SampledTextureColorSpace::Srgb,
            },
            SampledTextureSamplerDescriptor {
                min_filter: map_filter(upload.sampler.min_filter),
                mag_filter: map_filter(upload.sampler.mag_filter),
                mip_filter: map_filter(upload.sampler.mip_filter),
                address_u: map_address(upload.sampler.address_u),
                address_v: map_address(upload.sampler.address_v),
                address_w: map_address(upload.sampler.address_w),
            },
        );
        let new_texture = self
            .device
            .create_sampled_texture_resource(descriptor)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0238",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("texture upload failed for '{texture_id}': {error:?}"),
                )]
            })?;

        if self.device.textures.contains_key(&texture_id) {
            if let Err(error) = self.device.wait_idle_checked() {
                self.device.destroy_gpu_texture(new_texture);
                return Err(vec![Diagnostic::new(
                    "RV0239",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("cannot replace in-flight texture '{texture_id}': {error:?}"),
                )]);
            }
        }
        let descriptor_targets: Vec<(vk::DescriptorSet, u32)> = self
            .material_cache
            .values()
            .flat_map(|entry| {
                entry
                    .bound_texture_ids
                    .iter()
                    .enumerate()
                    .filter(|(_, bound)| *bound == &texture_id)
                    .map(|(index, _)| (entry.desc_set, MATERIAL_TEXTURE_BINDINGS[index]))
            })
            .chain(self.skinned_desc_cache.values().flat_map(|entry| {
                entry
                    .bound_texture_ids
                    .iter()
                    .enumerate()
                    .filter(|(_, bound)| *bound == &texture_id)
                    .map(|(index, _)| (entry.desc_set, MATERIAL_TEXTURE_BINDINGS[index]))
            }))
            .collect();
        let mut old_texture = self.device.textures.insert(texture_id.clone(), new_texture);
        let mut rebind_diagnostics = Vec::new();
        if let Err(error) = self
            .device
            .refresh_ui_overlay_texture_descriptor(&texture_id)
        {
            rebind_diagnostics.push(Diagnostic::new(
                "RV0311",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("failed to rebind replacement UI texture '{texture_id}': {error}"),
            ));
        }
        for (descriptor_set, binding) in descriptor_targets.iter().copied() {
            match self
                .device
                .bind_material_texture_at(&texture_id, binding, descriptor_set)
            {
                Ok(true) => {}
                Ok(false) => rebind_diagnostics.push(Diagnostic::new(
                    "RV0276",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "replacement texture '{texture_id}' disappeared before descriptor update"
                    ),
                )),
                Err(error) => rebind_diagnostics.push(Diagnostic::new(
                    "RV0277",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("failed to rebind replacement texture '{texture_id}': {error:?}"),
                )),
            }
        }
        if !rebind_diagnostics.is_empty() {
            let failed_texture = self.device.textures.remove(&texture_id);
            if let Some(previous_texture) = old_texture.take() {
                self.device
                    .textures
                    .insert(texture_id.clone(), previous_texture);
                if let Err(error) = self
                    .device
                    .refresh_ui_overlay_texture_descriptor(&texture_id)
                {
                    rebind_diagnostics.push(Diagnostic::new(
                        "RV0312",
                        DiagnosticSeverity::Fatal,
                        "scene_renderer",
                        format!(
                            "failed to restore UI texture '{texture_id}' descriptor after replacement rollback: {error}"
                        ),
                    ));
                }
                for (descriptor_set, binding) in descriptor_targets {
                    match self
                        .device
                        .bind_material_texture_at(&texture_id, binding, descriptor_set)
                    {
                        Ok(true) => {}
                        Ok(false) => rebind_diagnostics.push(Diagnostic::new(
                            "RV0278",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            format!(
                                "failed to restore texture '{texture_id}' after replacement rollback"
                            ),
                        )),
                        Err(error) => rebind_diagnostics.push(Diagnostic::new(
                            "RV0279",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            format!(
                                "failed to restore texture '{texture_id}' descriptor after replacement rollback: {error:?}"
                            ),
                        )),
                    }
                }
            }
            if let Some(failed_texture) = failed_texture {
                self.device.destroy_gpu_texture(failed_texture);
            }
            return Err(rebind_diagnostics);
        }
        if let Some(old_texture) = old_texture {
            self.device.destroy_gpu_texture(old_texture);
        }
        self.texture_uploads.insert(
            texture_id,
            UploadedResourceState {
                content_hash: upload.content_hash,
                revision,
            },
        );
        Ok(UploadReceipt::new(revision))
    }

    fn upload_environment_map(
        &mut self,
        upload: EnvironmentMapUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        let environment_id = upload.environment_id.id.clone();
        if let Some(existing) = self.environment_revisions.get(&environment_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .environment_revisions
            .get(&environment_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        self.environment_revisions.insert(
            environment_id.clone(),
            UploadedResourceState {
                content_hash: upload.content_hash,
                revision,
            },
        );
        self.environment_uploads
            .insert(environment_id.clone(), upload);
        if self.active_environment_id.as_deref() == Some(environment_id.as_str()) {
            self.active_environment_id = None;
        }
        Ok(UploadReceipt::new(revision))
    }

    fn upload_morph_target_set(
        &mut self,
        upload: engine_renderer::MorphTargetSetUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        let target_set_id = upload.target_set_id.id.clone();
        if let Some(existing) = self.morph_target_sets.get(&target_set_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .morph_target_sets
            .get(&target_set_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        let byte_count = upload
            .targets
            .len()
            .checked_mul(upload.vertex_count as usize)
            .and_then(|vertices| vertices.checked_mul(32))
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0326",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "morph target buffer size overflow",
                )]
            })?;
        let mut bytes = Vec::with_capacity(byte_count);
        for target in &upload.targets {
            for (position, normal) in target.position_deltas.iter().zip(&target.normal_deltas) {
                for value in [position[0], position[1], position[2], 0.0] {
                    bytes.extend_from_slice(&value.to_ne_bytes());
                }
                for value in [normal[0], normal[1], normal[2], 0.0] {
                    bytes.extend_from_slice(&value.to_ne_bytes());
                }
            }
        }
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: byte_count as u64,
                usage_flags: render_core::BufferUsage::STORAGE,
                memory_hint: MemoryHint::GpuOnly,
                debug_label: Some(format!("morph-{target_set_id}")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0327",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create morph target buffer '{target_set_id}': {error}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "RV0328",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload morph target buffer '{target_set_id}': {error}"),
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
                "RV0328",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("morph target buffer '{target_set_id}' has no Vulkan handle"),
            )]);
        }
        if self.morph_target_sets.contains_key(&target_set_id) {
            self.device.wait_idle_checked().map_err(|error| {
                vec![Diagnostic::new(
                    "RV0329",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("wait before replacing morph target set '{target_set_id}': {error}"),
                )]
            })?;
            let suffix = format!(":{target_set_id}");
            let descriptor_keys = self
                .skinned_desc_cache
                .keys()
                .filter(|key| key.ends_with(&suffix))
                .cloned()
                .collect::<Vec<_>>();
            for key in descriptor_keys {
                self.evict_skinned_descriptor_by_key(&key)?;
            }
        }
        let replacement = GpuMorphTargetSet {
            handle,
            buffer,
            vertex_count: upload.vertex_count,
            target_count: upload.targets.len() as u32,
            content_hash: upload.content_hash,
            revision,
        };
        if let Some(old) = self.morph_target_sets.insert(target_set_id, replacement) {
            self.device.destroy_buffer(old.handle);
        }
        Ok(UploadReceipt::new(revision))
    }

    fn upload_material(
        &mut self,
        upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        for texture in upload.texture_references().into_iter().flatten() {
            if !self.device.textures.contains_key(&texture.id) {
                return Err(vec![Diagnostic::new(
                    "RV0240",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "material '{}' references texture '{}' before a successful upload",
                        upload.material_id.id, texture.id
                    ),
                )]);
            }
        }
        let material_id = upload.material_id.id.clone();
        if let Some(existing) = self.uploaded_materials.get(&material_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .uploaded_materials
            .get(&material_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        self.uploaded_materials.insert(
            material_id,
            UploadedMaterialState {
                binding: uploaded_material_binding(&upload),
                content_hash: upload.content_hash,
                revision,
            },
        );
        Ok(UploadReceipt::new(revision))
    }

    fn remove_resource(&mut self, removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        let resource_id = removal.resource_id.id;
        match removal.kind {
            ResourceKind::Mesh => {
                if self.meshes.contains_key(&resource_id) {
                    self.device.wait_idle_checked().map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0241",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("cannot remove in-flight mesh '{resource_id}': {error:?}"),
                        )]
                    })?;
                }
                if let Some(mesh) = self.meshes.remove(&resource_id) {
                    self.device.destroy_buffer(mesh.vertex_buffer);
                    self.device.destroy_buffer(mesh.index_buffer);
                }
            }
            ResourceKind::Texture => {
                use crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID;
                if resource_id == FALLBACK_MATERIAL_TEXTURE_ID {
                    return Err(vec![Diagnostic::new(
                        "RV0242",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "the renderer fallback texture cannot be removed",
                    )]);
                }
                if let Some(dependent) = self.uploaded_materials.values().find(|material| {
                    material
                        .binding
                        .textures
                        .iter()
                        .any(|slot| slot.texture.id == resource_id)
                }) {
                    return Err(vec![Diagnostic::new(
                        "RV0270",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "texture '{resource_id}' is still referenced by material '{}'",
                            dependent.binding.material_id.id
                        ),
                    )]);
                }
                self.device.wait_idle_checked().map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0243",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot remove in-flight texture '{resource_id}': {error:?}"),
                    )]
                })?;
                let material_keys: Vec<(String, usize)> = self
                    .material_cache
                    .iter()
                    .flat_map(|(key, entry)| {
                        entry
                            .bound_texture_ids
                            .iter()
                            .enumerate()
                            .filter(|(_, bound)| bound.as_str() == resource_id)
                            .map(|(index, _)| (key.clone(), index))
                    })
                    .collect();
                let skinned_keys: Vec<(String, usize)> = self
                    .skinned_desc_cache
                    .iter()
                    .flat_map(|(key, entry)| {
                        entry
                            .bound_texture_ids
                            .iter()
                            .enumerate()
                            .filter(|(_, bound)| bound.as_str() == resource_id)
                            .map(|(index, _)| (key.clone(), index))
                    })
                    .collect();
                for (key, index) in &material_keys {
                    let descriptor_set = self.material_cache[key].desc_set;
                    let bound = self
                        .device
                        .bind_material_texture_at(
                            FALLBACK_MATERIAL_TEXTURE_ID,
                            MATERIAL_TEXTURE_BINDINGS[*index],
                            descriptor_set,
                        )
                        .map_err(|error| {
                            vec![Diagnostic::new(
                                "RV0280",
                                DiagnosticSeverity::Error,
                                "scene_renderer",
                                format!(
                                    "failed to detach texture '{resource_id}' from material '{key}': {error:?}"
                                ),
                            )]
                        })?;
                    if !bound {
                        return Err(vec![Diagnostic::new(
                            "RV0281",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            "fallback texture disappeared during resource removal",
                        )]);
                    }
                }
                for (key, index) in &skinned_keys {
                    let descriptor_set = self.skinned_desc_cache[key].desc_set;
                    let bound = self
                        .device
                        .bind_material_texture_at(
                            FALLBACK_MATERIAL_TEXTURE_ID,
                            MATERIAL_TEXTURE_BINDINGS[*index],
                            descriptor_set,
                        )
                        .map_err(|error| {
                            vec![Diagnostic::new(
                                "RV0282",
                                DiagnosticSeverity::Error,
                                "scene_renderer",
                                format!(
                                    "failed to detach texture '{resource_id}' from skinned material '{key}': {error:?}"
                                ),
                            )]
                        })?;
                    if !bound {
                        return Err(vec![Diagnostic::new(
                            "RV0283",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            "fallback texture disappeared during skinned resource removal",
                        )]);
                    }
                }
                for (key, index) in material_keys {
                    if let Some(entry) = self.material_cache.get_mut(&key) {
                        entry.bound_texture_ids[index] = FALLBACK_MATERIAL_TEXTURE_ID.to_owned();
                    }
                }
                for (key, index) in skinned_keys {
                    if let Some(entry) = self.skinned_desc_cache.get_mut(&key) {
                        entry.bound_texture_ids[index] = FALLBACK_MATERIAL_TEXTURE_ID.to_owned();
                    }
                }
                self.device
                    .release_ui_overlay_texture_descriptor(&resource_id)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0313",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!(
                                "failed to release UI descriptor for texture '{resource_id}': {error}"
                            ),
                        )]
                    })?;
                if let Some(texture) = self.device.textures.remove(&resource_id) {
                    self.device.destroy_gpu_texture(texture);
                }
                self.texture_uploads.remove(&resource_id);
            }
            ResourceKind::Material => {
                self.evict_material_by_id(&resource_id)?;
                self.uploaded_materials.remove(&resource_id);
            }
            ResourceKind::EnvironmentMap => {
                self.environment_uploads.remove(&resource_id);
                self.environment_revisions.remove(&resource_id);
                if self.active_environment_id.as_deref() == Some(resource_id.as_str()) {
                    self.active_environment_id = None;
                }
            }
            ResourceKind::MorphTargetSet => {
                let suffix = format!(":{resource_id}");
                let descriptor_keys = self
                    .skinned_desc_cache
                    .keys()
                    .filter(|key| key.ends_with(&suffix))
                    .cloned()
                    .collect::<Vec<_>>();
                for key in descriptor_keys {
                    self.evict_skinned_descriptor_by_key(&key)?;
                }
                if let Some(target_set) = self.morph_target_sets.remove(&resource_id) {
                    self.device.destroy_buffer(target_set.handle);
                }
            }
        }
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        if self.cur_enc.is_none() && self.cur_sc.is_none() && self.cur_ii.is_none() {
            return Ok(());
        }
        self.cur_enc.take();
        self.cur_sc.take();
        self.cur_ii.take();
        // The aborted frame's command buffer is reset without submission, so
        // its timestamp queries can never be read back; drop the slot state.
        self.gpu_timestamps.abort_slot(self.device.current_frame);
        self.recover_failed_device_frame()
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        if width == 0 || height == 0 {
            return Err(vec![Diagnostic::new(
                "RV0245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "Vulkan surface dimensions must be non-zero",
            )]);
        }
        if self.cur_enc.is_some() {
            return Err(vec![Diagnostic::new(
                "RV0246",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot resize while a frame is being recorded",
            )]);
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0247",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("failed to wait for Vulkan resize: {error:?}"),
            )]
        })?;
        self.device.destroy_scene_framebuffers(&self.framebuffers);
        self.framebuffers.clear();
        self.width = width;
        self.height = height;
        self.device.resize(width, height);
        Ok(())
    }

    fn set_gpu_timing_enabled(&mut self, enabled: bool) {
        self.gpu_timing_enabled = enabled;
        // Re-evaluate device support on the next frame so a runtime toggle
        // takes effect without recreating the renderer.
        self.gpu_timing_configured = false;
    }
}
