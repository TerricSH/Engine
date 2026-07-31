use super::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl BackendRenderer for Dx12SceneRenderer {
    fn supports_weighted_blended_oit(&self) -> bool {
        true
    }

    fn supports_gpu_particle_simulation(&self) -> bool {
        true
    }

    fn begin_frame(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        validate_dx12_frame_contract(input)?;
        if let Some(reason) = &self.fatal_frame_error {
            return Err(vec![Diagnostic::new(
                "DX1243",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                format!("DX12 renderer is in a failed frame state and must be recreated: {reason}"),
            )]);
        }
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1200",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "begin_frame called while another DX12 frame is active",
            )]);
        }
        self.shadow_frame_data = None;

        self.ensure_pipeline();
        let pipeline = self.pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 forward pipeline is unavailable; shader or PSO creation failed",
            )]
        })?;
        self.device
            .set_next_frame_clear_color(input.views[0].clear_color);
        let (image_index, mut encoder) =
            self.device.begin_frame(self.swapchain).map_err(|error| {
                vec![Diagnostic::new(
                    "DX1201",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("begin_frame failed: {error:?}"),
                )]
            })?;
        encoder.set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        encoder.set_scissor(0, 0, self.width, self.height);
        encoder.bind_pipeline(pipeline);
        if let Err(mut diagnostics) = self.prepare_vertex_draw_arena(input) {
            if let Err(error) = self.device.abort_frame(encoder) {
                let reason = format!(
                    "failed to abandon DX12 frame after vertex-draw arena error: {error:?}"
                );
                self.fatal_frame_error = Some(reason.clone());
                diagnostics.push(Diagnostic::new(
                    "DX1205",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    reason,
                ));
            }
            return Err(diagnostics);
        }
        let extraction_stats = extraction_stats(input);
        self.active_frame = Some(Dx12FrameState {
            image_index,
            encoder,
            draw_calls: 0,
            triangles: 0,
            visible_drawables: extraction_stats.visible_drawables,
            culled_drawables: extraction_stats.culled_drawables,
            visible_lights: extraction_stats.visible_lights,
            culled_lights: extraction_stats.culled_lights,
            hdr_pass_active: false,
        });
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &render_graph2::PassNode,
        barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        if let Some(unsupported) = barriers.iter().find(|barrier| {
            !matches!(
                barrier.resource_name.as_str(),
                "swapchain"
                    | "hdr_color"
                    | "oit_accumulation"
                    | "oit_optical_depth"
                    | "depth"
                    | "depth_stencil"
            )
        }) {
            return Err(vec![Diagnostic::new(
                "DX1248",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "DX12 cannot apply render-graph barrier for resource '{}'",
                    unsupported.resource_name
                ),
            )]);
        }
        // The accepted direct-output resources are transitioned by
        // Dx12Device::begin_frame/end_frame and render-pass attachment setup.
        Ok(())
    }

    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &render_graph2::PassNode,
        _stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        match &pass.kind {
            render_graph2::PassKind::OpaquePbrForward => {
                self.record_forward_pass(input, Some(pass.view_id))
            }
            render_graph2::PassKind::Present => self.record_ui_overlay_pass(input),
            render_graph2::PassKind::ToneMap => self.record_tone_map_pass(input),
            render_graph2::PassKind::DirectionalShadow => {
                self.record_directional_shadow_pass(input)
            }
            render_graph2::PassKind::Custom(name) => Err(vec![Diagnostic::new(
                "DX1246",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("custom render pass '{name}' is not registered by the DX12 backend"),
            )]),
        }
    }

    fn end_frame(&mut self, stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1204",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "end_frame called without an active DX12 frame",
            )]
        })?;
        frame.encoder.end_render_pass();
        let device_stats =
            match self
                .device
                .end_frame(self.swapchain, frame.encoder, frame.image_index)
            {
                Ok(stats) => stats,
                Err(error) => {
                    let reason = format!(
                        "end_frame failed after command-list ownership transfer: {error:?}"
                    );
                    self.fatal_frame_error = Some(reason.clone());
                    return Err(vec![Diagnostic::new(
                        "DX1202",
                        DiagnosticSeverity::Fatal,
                        "scene_renderer",
                        reason,
                    )]);
                }
            };
        stats.draw_calls = frame.draw_calls;
        stats.triangles = frame.triangles;
        stats.visible_drawables = frame.visible_drawables;
        stats.visible_lights = frame.visible_lights;
        stats.culled_drawables = frame.culled_drawables;
        stats.culled_lights = frame.culled_lights;
        stats.gpu_frame_ms = device_stats.gpu_frame_ms;
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        let Some(mut frame) = self.active_frame.take() else {
            return Ok(());
        };
        frame.encoder.end_render_pass();
        match self.device.abort_frame(frame.encoder) {
            Ok(()) => Ok(()),
            Err(error) => {
                let reason = format!("failed to abandon the active DX12 command list: {error:?}");
                self.fatal_frame_error = Some(reason.clone());
                Err(vec![Diagnostic::new(
                    "DX1205",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    reason,
                )])
            }
        }
    }

    fn upload_mesh(&mut self, upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1224",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload a mesh while a DX12 frame is active",
            )]);
        }

        let mesh_id = upload.mesh_id.id.clone();
        if let Some(existing) = self.meshes.get(&mesh_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }

        let vertex_buffer = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: upload.vertex_bytes.len() as u64,
                usage_flags: render_core::BufferUsage::VERTEX,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("mesh-{mesh_id}-vertices")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1220",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("upload_mesh create_buffer(vertices): {error:?}"),
                )]
            })?;
        if let Err(error) = self
            .device
            .write_buffer(vertex_buffer, &upload.vertex_bytes, 0)
        {
            self.device.destroy_buffer(vertex_buffer);
            return Err(vec![Diagnostic::new(
                "DX1221",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(vertices): {error:?}"),
            )]);
        }
        if let Err(error) = self
            .device
            .set_vertex_stride(vertex_buffer, upload.vertex_format.stride_bytes())
        {
            self.device.destroy_buffer(vertex_buffer);
            return Err(vec![Diagnostic::new(
                "DX1226",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh set vertex stride: {error:?}"),
            )]);
        }

        let index_buffer = match self.device.create_buffer(&BufferDescriptor {
            size_bytes: upload.index_bytes.len() as u64,
            usage_flags: render_core::BufferUsage::INDEX,
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-indices")),
        }) {
            Ok(buffer) => buffer,
            Err(error) => {
                self.device.destroy_buffer(vertex_buffer);
                return Err(vec![Diagnostic::new(
                    "DX1222",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("upload_mesh create_buffer(indices): {error:?}"),
                )]);
            }
        };
        if let Err(error) = self
            .device
            .write_buffer(index_buffer, &upload.index_bytes, 0)
        {
            self.device.destroy_buffer(index_buffer);
            self.device.destroy_buffer(vertex_buffer);
            return Err(vec![Diagnostic::new(
                "DX1223",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(indices): {error:?}"),
            )]);
        }

        let revision = match self
            .mesh_revisions
            .get(&mesh_id)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
        {
            Some(revision) => revision,
            None => {
                self.device.destroy_buffer(index_buffer);
                self.device.destroy_buffer(vertex_buffer);
                return Err(vec![Diagnostic::new(
                    "DX1225",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("mesh revision overflow for '{mesh_id}'"),
                )]);
            }
        };
        let index_format = upload.index_format;
        let mesh = Dx12GpuMesh {
            vertex_buffer,
            index_buffer,
            index_count: upload.index_count,
            index_format,
            vertex_format: upload.vertex_format,
            vertex_count: upload.vertex_count,
            vertex_bytes: upload.vertex_bytes,
            content_hash: upload.content_hash,
            revision,
        };

        // Keep the old resource live until every allocation and write for the
        // replacement has succeeded. Waiting only when replacing avoids
        // releasing buffers still referenced by an in-flight command list.
        if self.meshes.contains_key(&mesh_id) {
            self.device.wait_idle();
            self.clear_morphed_vertex_buffers();
            self.clear_particle_instance_buffers();
        }
        if let Some(old) = self.meshes.insert(mesh_id.clone(), mesh) {
            self.device.destroy_buffer(old.vertex_buffer);
            self.device.destroy_buffer(old.index_buffer);
        }
        self.mesh_revisions.insert(mesh_id, revision);
        Ok(UploadReceipt::new(revision))
    }

    fn upload_material(
        &mut self,
        upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1233",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload a material while a DX12 frame is active",
            )]);
        }
        for texture in upload.texture_references().into_iter().flatten() {
            if !self.textures.contains_key(&texture.id) {
                return Err(vec![Diagnostic::new(
                    "DX1234",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "DX12 material '{}' references texture '{}' before a successful upload",
                        upload.material_id.id, texture.id
                    ),
                )]);
            }
        }

        let material_id = upload.material_id.id.clone();
        if let Some(existing) = self.materials.get(&material_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .materials
            .get(&material_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        let texture_ids = upload
            .texture_references()
            .map(|texture| texture.map(|texture| texture.id.clone()));
        let texture_flags = material_texture_flags_from_ids(&texture_ids);
        self.materials.insert(
            material_id,
            Dx12MaterialState {
                constants: material_constants_from_upload(&upload),
                emissive_constants: emissive_constants(upload.emissive, texture_flags),
                advanced_constants: advanced_constants_from_upload(&upload),
                texture_ids,
                transparency: upload.transparency,
                double_sided: upload.double_sided,
                content_hash: upload.content_hash,
                revision,
            },
        );
        Ok(UploadReceipt::new(revision))
    }

    fn upload_texture(&mut self, upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1235",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload a texture while a DX12 frame is active",
            )]);
        }
        let texture_id = upload.texture_id.id.clone();
        if let Some(existing) = self.textures.get(&texture_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .textures
            .get(&texture_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        let handle = self
            .device
            .upload_sampled_rgba8(
                upload.width,
                upload.height,
                upload.color_space,
                &upload.mip_levels,
                upload.sampler,
            )
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1236",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("DX12 texture upload '{texture_id}' failed: {error:?}"),
                )]
            })?;
        if let Some(old) = self.textures.insert(
            texture_id,
            Dx12TextureState {
                handle,
                content_hash: upload.content_hash,
                revision,
            },
        ) {
            self.device.destroy_texture(old.handle);
        }
        Ok(UploadReceipt::new(revision))
    }

    fn upload_environment_map(
        &mut self,
        upload: EnvironmentMapUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1261",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload an environment map while a DX12 frame is active",
            )]);
        }
        let environment_id = upload.environment_id.id.clone();
        if let Some(existing) = self.environments.get(&environment_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .environments
            .get(&environment_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        let handle = self
            .device
            .upload_sampled_rgba16f_cube(&upload.mip_levels)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1262",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "DX12 environment-map upload '{}' failed: {error:?}",
                        environment_id
                    ),
                )]
            })?;
        let replacement = Dx12EnvironmentState {
            handle,
            mip_count: upload.mip_levels.len() as u32,
            content_hash: upload.content_hash,
            revision,
        };
        if let Some(old) = self.environments.insert(environment_id, replacement) {
            self.device.destroy_texture(old.handle);
        }
        Ok(UploadReceipt::new(revision))
    }

    fn upload_morph_target_set(
        &mut self,
        upload: MorphTargetSetUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1268",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload a morph target set while a DX12 frame is active",
            )]);
        }
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
        if self.morph_target_sets.contains_key(&target_set_id) {
            self.device.wait_idle();
            self.clear_morphed_vertex_buffers();
        }
        self.morph_target_sets.insert(
            target_set_id,
            Dx12MorphTargetSet {
                vertex_count: upload.vertex_count,
                targets: upload.targets,
                content_hash: upload.content_hash,
                revision,
            },
        );
        Ok(UploadReceipt::new(revision))
    }

    fn remove_resource(&mut self, removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1232",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot remove a resource while a DX12 frame is active",
            )]);
        }
        match removal.kind {
            ResourceKind::Mesh => {
                if let Some(mesh) = self.meshes.remove(&removal.resource_id.id) {
                    self.device.wait_idle();
                    self.clear_morphed_vertex_buffers();
                    self.clear_particle_instance_buffers();
                    self.device.destroy_buffer(mesh.vertex_buffer);
                    self.device.destroy_buffer(mesh.index_buffer);
                }
            }
            ResourceKind::Material => {
                self.materials.remove(&removal.resource_id.id);
            }
            ResourceKind::Texture => {
                if let Some(dependent) = self.materials.iter().find_map(|(id, material)| {
                    material
                        .texture_ids
                        .iter()
                        .flatten()
                        .any(|texture_id| texture_id == &removal.resource_id.id)
                        .then_some(id)
                }) {
                    return Err(vec![Diagnostic::new(
                        "DX1231",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "cannot remove DX12 texture '{}' while material '{}' references it",
                            removal.resource_id.id, dependent
                        ),
                    )]);
                }
                if let Some(texture) = self.textures.remove(&removal.resource_id.id) {
                    self.device.wait_idle();
                    self.device.destroy_texture(texture.handle);
                }
            }
            ResourceKind::EnvironmentMap => {
                if let Some(environment) = self.environments.remove(&removal.resource_id.id) {
                    self.device.wait_idle();
                    self.device.destroy_texture(environment.handle);
                }
            }
            ResourceKind::MorphTargetSet => {
                if self
                    .morph_target_sets
                    .remove(&removal.resource_id.id)
                    .is_some()
                {
                    self.device.wait_idle();
                    self.clear_morphed_vertex_buffers();
                }
            }
        }
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        self.resize_surface(width, height)
    }
}
