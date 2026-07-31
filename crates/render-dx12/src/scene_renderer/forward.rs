use super::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12SceneRenderer {
    pub(super) fn record_forward_pass(
        &mut self,
        input: &RenderFrameInput,
        view_id: Option<u32>,
    ) -> Result<(), Vec<Diagnostic>> {
        let view = view_id
            .and_then(|id| input.views.iter().find(|view| view.view_id == id))
            .or_else(|| input.views.first());
        if view.is_none()
            && (!input.drawables.is_empty()
                || !input.skinned_items.is_empty()
                || !input.particle_batches.is_empty())
        {
            return Err(vec![Diagnostic::new(
                "DX1211",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot render drawables without a camera view",
            )]);
        }
        let missing_meshes: Vec<&str> = input
            .drawables
            .iter()
            .map(|drawable| &drawable.mesh)
            .chain(input.skinned_items.iter().map(|item| &item.mesh))
            .chain(input.particle_batches.iter().map(|batch| &batch.mesh))
            .filter_map(|mesh| (!self.meshes.contains_key(&mesh.id)).then_some(mesh.id.as_str()))
            .collect();
        if !missing_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1212",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "drawables reference meshes that were not uploaded: {}",
                    missing_meshes.join(", ")
                ),
            )]);
        }
        let invalid_static_meshes: Vec<&str> = input
            .drawables
            .iter()
            .filter_map(|drawable| {
                (self.meshes[&drawable.mesh.id].vertex_format != RendererMeshVertexFormat::Pbr32)
                    .then_some(drawable.mesh.id.as_str())
            })
            .collect();
        if !invalid_static_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1217",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "static drawables require Pbr32 meshes, got skinned layout for: {}",
                    invalid_static_meshes.join(", ")
                ),
            )]);
        }
        let invalid_skinned_meshes: Vec<&str> = input
            .skinned_items
            .iter()
            .filter_map(|item| {
                (self.meshes[&item.mesh.id].vertex_format != RendererMeshVertexFormat::Skinned64)
                    .then_some(item.mesh.id.as_str())
            })
            .collect();
        if !invalid_skinned_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1214",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "skinned drawables require Skinned64 meshes, got static layout for: {}",
                    invalid_skinned_meshes.join(", ")
                ),
            )]);
        }
        let invalid_particle_meshes: Vec<&str> = input
            .particle_batches
            .iter()
            .filter_map(|batch| {
                (self.meshes[&batch.mesh.id].vertex_format != RendererMeshVertexFormat::Pbr32)
                    .then_some(batch.mesh.id.as_str())
            })
            .collect();
        if !invalid_particle_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1272",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "particle batches require Pbr32 meshes: {}",
                    invalid_particle_meshes.join(", ")
                ),
            )]);
        }
        for item in &input.skinned_items {
            let count = match item.bone_palette_layout {
                engine_renderer::BonePaletteLayout::Full4x4 { count } => count,
                engine_renderer::BonePaletteLayout::Packed3x4 { .. } => {
                    return Err(vec![Diagnostic::new(
                        "DX1227",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skinning currently requires Full4x4 bone palettes",
                    )]);
                }
            };
            if count as usize != item.bone_palette.len()
                || item.bone_palette.is_empty()
                || item.bone_palette.len() > 64
                || !item
                    .bone_palette
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite())
            {
                return Err(vec![Diagnostic::new(
                    "DX1228",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "skinned item '{}' must contain 1..=64 finite Full4x4 bones and a matching count",
                        item.skeleton.id
                    ),
                )]);
            }
        }
        for (material_id, entity) in input
            .drawables
            .iter()
            .map(|item| (&item.material, item.entity.as_ref()))
            .chain(
                input
                    .skinned_items
                    .iter()
                    .map(|item| (&item.material, item.entity.as_ref())),
            )
            .chain(
                input
                    .particle_batches
                    .iter()
                    .map(|batch| (&batch.material, batch.emitter.as_ref())),
            )
        {
            let texture_ids = self.material_texture_ids(input, material_id);
            for texture_id in texture_ids.iter().flatten() {
                if !self.textures.contains_key(texture_id) {
                    return Err(vec![Diagnostic::new(
                        "DX1215",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "material '{}' references texture '{}' before a successful DX12 upload",
                            material_id.id, texture_id
                        ),
                    )
                    .entity(entity.cloned())]);
                }
            }
        }
        let layout = self.pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1213",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 pipeline layout is unavailable",
            )]
        })?;
        let missing_pipeline = || {
            vec![Diagnostic::new(
                "DX1203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 forward surface pipelines are unavailable",
            )]
        };
        let static_pipelines = [
            self.pipeline.ok_or_else(missing_pipeline)?,
            self.double_sided_pipeline.ok_or_else(missing_pipeline)?,
            self.blend_pipeline.ok_or_else(missing_pipeline)?,
            self.blend_double_sided_pipeline
                .ok_or_else(missing_pipeline)?,
            self.additive_pipeline.ok_or_else(missing_pipeline)?,
            self.additive_double_sided_pipeline
                .ok_or_else(missing_pipeline)?,
            self.oit_pipeline.ok_or_else(missing_pipeline)?,
            self.oit_double_sided_pipeline
                .ok_or_else(missing_pipeline)?,
        ];
        let weighted_oit = input.render_options.transparency_mode
            == engine_renderer::TransparencyMode::WeightedBlendedOit;
        let shadow_texture = self.shadow_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 shadow texture is unavailable",
            )]
        })?;
        let shadow_frame_data = self.shadow_frame_data;
        let hdr_render_pass = self.hdr_render_pass.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1255",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 HDR forward render pass is unavailable",
            )]
        })?;
        let hdr_framebuffer = self.hdr_framebuffer.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1255",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 HDR forward framebuffer is unavailable",
            )]
        })?;
        let skinned_pipelines = if input.skinned_items.is_empty() {
            None
        } else {
            let missing = || {
                vec![Diagnostic::new(
                    "DX1229",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 skinned surface pipelines are unavailable",
                )]
            };
            Some([
                self.skinned_pipeline.ok_or_else(missing)?,
                self.skinned_double_sided_pipeline.ok_or_else(missing)?,
                self.skinned_blend_pipeline.ok_or_else(missing)?,
                self.skinned_blend_double_sided_pipeline
                    .ok_or_else(missing)?,
                self.skinned_additive_pipeline.ok_or_else(missing)?,
                self.skinned_additive_double_sided_pipeline
                    .ok_or_else(missing)?,
                self.skinned_oit_pipeline.ok_or_else(missing)?,
                self.skinned_oit_double_sided_pipeline.ok_or_else(missing)?,
            ])
        };
        let particle_pipelines = if input.particle_batches.is_empty() {
            None
        } else {
            let missing = || {
                vec![Diagnostic::new(
                    "DX1273",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 particle billboard pipelines are unavailable",
                )]
            };
            Some([
                self.particle_pipeline.ok_or_else(missing)?,
                self.particle_additive_pipeline.ok_or_else(missing)?,
                self.particle_oit_pipeline.ok_or_else(missing)?,
                self.gpu_particle_pipeline.ok_or_else(missing)?,
                self.gpu_particle_additive_pipeline.ok_or_else(missing)?,
                self.gpu_particle_oit_pipeline.ok_or_else(missing)?,
            ])
        };
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "execute_pass called without an active DX12 frame",
            )]
        })?;

        let mut current_pipeline = static_pipelines[0];
        let clear_color = view.map_or([0.0, 0.0, 0.0, 1.0], |view| view.clear_color);
        if !frame.hdr_pass_active {
            frame.encoder.begin_render_pass_with_color_clears(
                hdr_render_pass,
                hdr_framebuffer,
                (0, 0, self.width, self.height),
                &[clear_color, [0.0; 4], [0.0; 4]],
                Some(1.0),
            );
            frame.hdr_pass_active = true;
        }
        frame.encoder.bind_pipeline(current_pipeline);
        let viewport = view.map_or(engine_renderer::Rect::FULL, |view| {
            view.viewport_rect_normalized
        });
        let viewport = match prepare_normalized_viewport(viewport, self.width, self.height) {
            Ok(viewport) => viewport,
            Err(error) => {
                self.active_frame = Some(frame);
                return Err(vec![Diagnostic::new(
                    "DX1250",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("DX12 viewport preparation failed: {error}"),
                )]);
            }
        };
        frame.encoder.set_viewport(
            viewport.x,
            viewport.y,
            viewport.width,
            viewport.height,
            0.0,
            1.0,
        );
        frame.encoder.set_scissor(
            viewport.scissor.x,
            viewport.scissor.y,
            viewport.scissor.width,
            viewport.scissor.height,
        );

        if let Some(view) = view {
            let view_matrix = Mat4::from_cols_array(&view.view_matrix);
            let projection_matrix = Mat4::from_cols_array(&view.projection_matrix);
            let camera_position = view_matrix.inverse().w_axis.truncate();
            let cluster_buffers = match self.prepare_clustered_light_buffers(input, view) {
                Ok(buffers) => buffers,
                Err(diagnostics) => {
                    self.active_frame = Some(frame);
                    return Err(diagnostics);
                }
            };
            let (environment_texture, environment_constants) =
                match self.environment_binding(input, camera_position) {
                    Ok(binding) => binding,
                    Err(diagnostics) => {
                        self.active_frame = Some(frame);
                        return Err(diagnostics);
                    }
                };
            if view.clear_flags == engine_renderer::ClearFlags::Skybox
                && select_environment_map(&input.render_options.environment, camera_position)
                    .is_some()
            {
                let Some(skybox_pipeline) = self.skybox_pipeline else {
                    self.active_frame = Some(frame);
                    return Err(vec![Diagnostic::new(
                        "DX1282",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skybox pipeline is unavailable",
                    )]);
                };
                frame.encoder.bind_pipeline(skybox_pipeline);
                let inverse_view_projection = (projection_matrix * view_matrix).inverse();
                frame.encoder.push_constants(
                    layout,
                    0x10,
                    0,
                    &matrix_bytes(inverse_view_projection),
                );
                frame.encoder.push_constants(
                    layout,
                    0x10,
                    64,
                    &float4_bytes([camera_position.x, camera_position.y, camera_position.z, 1.0]),
                );
                frame
                    .encoder
                    .push_constants(layout, 0x20, 224, &environment_constants);
                let empty_textures: [Option<String>; 5] = std::array::from_fn(|_| None);
                let texture_table = self.material_texture_table(
                    &empty_textures,
                    shadow_texture,
                    environment_texture,
                );
                if !frame
                    .encoder
                    .bind_scene_resource_set(layout, &texture_table, &cluster_buffers)
                {
                    self.active_frame = Some(frame);
                    return Err(vec![Diagnostic::new(
                        "DX1283",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skybox pass could not bind the selected environment",
                    )]);
                }
                frame.encoder.draw(3, 1, 0, 0);
                frame.draw_calls += 1;
                frame.triangles += 1;
                frame.encoder.bind_pipeline(current_pipeline);
            }
            for transparent_phase in [false, true] {
                let mut ordered_drawables = Vec::with_capacity(input.drawables.len());
                let mut blended_drawables = Vec::new();
                for (drawable_index, drawable) in input.drawables.iter().enumerate() {
                    let (transparency, _) = self.material_surface(input, &drawable.material);
                    let transparent =
                        matches!(transparency, Transparency::Blend | Transparency::Additive);
                    if transparent != transparent_phase {
                        continue;
                    }
                    if transparent {
                        let translation = Mat4::from_cols_array(&drawable.world_transform)
                            .w_axis
                            .truncate();
                        blended_drawables.push((
                            (translation - camera_position).length_squared(),
                            drawable_index,
                            drawable,
                        ));
                    } else {
                        ordered_drawables.push((drawable_index, drawable));
                    }
                }
                order_transparent_back_to_front(&mut blended_drawables, weighted_oit, |item| {
                    item.0
                });
                ordered_drawables.extend(
                    blended_drawables
                        .into_iter()
                        .map(|(_, drawable_index, drawable)| (drawable_index, drawable)),
                );

                for (drawable_index, drawable) in ordered_drawables {
                    // Existence was validated before recording any draw command.
                    let (vertex_buffer, index_buffer, index_format, index_count) = {
                        let mesh = &self.meshes[&drawable.mesh.id];
                        (
                            mesh.vertex_buffer,
                            mesh.index_buffer,
                            mesh.index_format,
                            mesh.index_count,
                        )
                    };
                    let world_matrix = Mat4::from_cols_array(&drawable.world_transform);
                    let (transparency, double_sided) =
                        self.material_surface(input, &drawable.material);
                    let next_pipeline = static_pipelines
                        [surface_variant_index(&transparency, double_sided, weighted_oit)];
                    if next_pipeline != current_pipeline {
                        frame.encoder.bind_pipeline(next_pipeline);
                        current_pipeline = next_pipeline;
                    }
                    let mvp = (projection_matrix * view_matrix * world_matrix).to_cols_array();
                    let mut mvp_bytes = [0_u8; 64];
                    for (destination, value) in mvp_bytes.chunks_exact_mut(4).zip(mvp) {
                        destination.copy_from_slice(&value.to_ne_bytes());
                    }
                    frame.encoder.push_constants(layout, 0x10, 0, &mvp_bytes);
                    let Some((vertex_draw_buffer, vertex_draw_offset)) =
                        self.vertex_draw_binding(drawable_index)
                    else {
                        self.active_frame = Some(frame);
                        return Err(vec![Diagnostic::new(
                            "DX1286",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "DX12 forward pass has no aligned vertex-draw arena binding",
                        )]);
                    };
                    if !frame.encoder.bind_uniform_buffer_offset(
                        layout,
                        vertex_draw_buffer,
                        vertex_draw_offset,
                    ) {
                        self.active_frame = Some(frame);
                        return Err(vec![Diagnostic::new(
                            "DX1286",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "DX12 forward pass could not bind its vertex-draw arena offset",
                        )]);
                    }
                    let input_material = input
                        .materials
                        .iter()
                        .find(|binding| binding.material_id == drawable.material);
                    let texture_ids = self.material_texture_ids(input, &drawable.material);
                    let texture_flags = material_texture_flags_from_ids(&texture_ids);
                    let mut material_constants = input_material
                        .map(|binding| {
                            material_constants_from_bytes(
                                &binding.uniforms.bytes,
                                texture_ids[0].is_some(),
                                &binding.transparency,
                                weighted_oit,
                            )
                        })
                        .or_else(|| {
                            self.materials
                                .get(&drawable.material.id)
                                .map(|material| material.constants)
                        })
                        .unwrap_or_else(default_material_constants);
                    set_material_surface_flags(
                        &mut material_constants,
                        texture_ids[0].is_some(),
                        &transparency,
                        weighted_oit,
                    );
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 64, &material_constants);
                    let (light_matrix, shadow_parameters, light_direction) =
                        shadow_scene_constants(shadow_frame_data, world_matrix);
                    frame
                        .encoder
                        .push_constants(layout, 0x30, 96, &light_matrix);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 160, &shadow_parameters);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 176, &light_direction);
                    let emissive_constants = input_material
                        .map(|binding| {
                            emissive_constants_from_bytes(&binding.uniforms.bytes, texture_flags)
                        })
                        .or_else(|| {
                            self.materials
                                .get(&drawable.material.id)
                                .map(|material| material.emissive_constants)
                        })
                        .unwrap_or_else(default_emissive_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 192, &emissive_constants);
                    let advanced_constants = input_material
                        .map(|binding| advanced_constants_from_bytes(&binding.uniforms.bytes))
                        .or_else(|| {
                            self.materials
                                .get(&drawable.material.id)
                                .map(|material| material.advanced_constants)
                        })
                        .unwrap_or_else(default_advanced_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 208, &advanced_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 224, &environment_constants);
                    let texture_table = self.material_texture_table(
                        &texture_ids,
                        shadow_texture,
                        environment_texture,
                    );
                    if !frame.encoder.bind_scene_resource_set(
                        layout,
                        &texture_table,
                        &cluster_buffers,
                    ) {
                        self.active_frame = Some(frame);
                        return Err(vec![Diagnostic::new(
                        "DX1216",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 forward pass could not bind its seven-texture material/environment table",
                    )]);
                    }
                    frame.encoder.bind_vertex_buffers(&[vertex_buffer], &[0]);
                    frame
                        .encoder
                        .bind_index_buffer(index_buffer, 0, index_format);
                    frame.encoder.draw_indexed(index_count, 1, 0, 0, 0);
                    frame.draw_calls += 1;
                    frame.triangles += u64::from(index_count / 3);
                }

                let mut ordered_skinned = Vec::with_capacity(input.skinned_items.len());
                let mut blended_skinned = Vec::new();
                for (item_index, item) in input.skinned_items.iter().enumerate() {
                    let (transparency, _) = self.material_surface(input, &item.material);
                    let transparent =
                        matches!(transparency, Transparency::Blend | Transparency::Additive);
                    if transparent != transparent_phase {
                        continue;
                    }
                    if transparent {
                        let translation = Mat4::from_cols_array(&item.world_transform)
                            .w_axis
                            .truncate();
                        blended_skinned.push((
                            (translation - camera_position).length_squared(),
                            item_index,
                            item,
                        ));
                    } else {
                        ordered_skinned.push((item_index, item));
                    }
                }
                order_transparent_back_to_front(&mut blended_skinned, weighted_oit, |item| item.0);
                ordered_skinned.extend(
                    blended_skinned
                        .into_iter()
                        .map(|(_, item_index, item)| (item_index, item)),
                );

                for (item_index, item) in ordered_skinned {
                    let mesh = self.meshes[&item.mesh.id].clone();
                    let world_matrix = Mat4::from_cols_array(&item.world_transform);
                    let (transparency, double_sided) = self.material_surface(input, &item.material);
                    let next_pipeline = skinned_pipelines
                        .expect("skinned pipelines exist for a non-empty skinned list")
                        [surface_variant_index(&transparency, double_sided, weighted_oit)];
                    if next_pipeline != current_pipeline {
                        frame.encoder.bind_pipeline(next_pipeline);
                        current_pipeline = next_pipeline;
                    }
                    let mvp = (projection_matrix * view_matrix * world_matrix).to_cols_array();
                    let mut mvp_bytes = [0_u8; 64];
                    for (destination, value) in mvp_bytes.chunks_exact_mut(4).zip(mvp) {
                        destination.copy_from_slice(&value.to_ne_bytes());
                    }
                    frame.encoder.push_constants(layout, 0x10, 0, &mvp_bytes);

                    let input_material = input
                        .materials
                        .iter()
                        .find(|binding| binding.material_id == item.material);
                    let texture_ids = self.material_texture_ids(input, &item.material);
                    let texture_flags = material_texture_flags_from_ids(&texture_ids);
                    let mut material_constants = input_material
                        .map(|binding| {
                            material_constants_from_bytes(
                                &binding.uniforms.bytes,
                                texture_ids[0].is_some(),
                                &binding.transparency,
                                weighted_oit,
                            )
                        })
                        .or_else(|| {
                            self.materials
                                .get(&item.material.id)
                                .map(|material| material.constants)
                        })
                        .unwrap_or_else(default_material_constants);
                    set_material_surface_flags(
                        &mut material_constants,
                        texture_ids[0].is_some(),
                        &transparency,
                        weighted_oit,
                    );
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 64, &material_constants);
                    let (light_matrix, shadow_parameters, light_direction) =
                        shadow_scene_constants(shadow_frame_data, world_matrix);
                    frame
                        .encoder
                        .push_constants(layout, 0x30, 96, &light_matrix);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 160, &shadow_parameters);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 176, &light_direction);
                    let emissive_constants = input_material
                        .map(|binding| {
                            emissive_constants_from_bytes(&binding.uniforms.bytes, texture_flags)
                        })
                        .or_else(|| {
                            self.materials
                                .get(&item.material.id)
                                .map(|material| material.emissive_constants)
                        })
                        .unwrap_or_else(default_emissive_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 192, &emissive_constants);
                    let advanced_constants = input_material
                        .map(|binding| advanced_constants_from_bytes(&binding.uniforms.bytes))
                        .or_else(|| {
                            self.materials
                                .get(&item.material.id)
                                .map(|material| material.advanced_constants)
                        })
                        .unwrap_or_else(default_advanced_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 208, &advanced_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 224, &environment_constants);
                    let texture_table = self.material_texture_table(
                        &texture_ids,
                        shadow_texture,
                        environment_texture,
                    );
                    if !frame.encoder.bind_scene_resource_set(
                        layout,
                        &texture_table,
                        &cluster_buffers,
                    ) {
                        self.active_frame = Some(frame);
                        return Err(vec![Diagnostic::new(
                        "DX1216",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skinned pass could not bind its seven-texture material/environment table",
                    )]);
                    }

                    let cache_key = format!(
                        "{}:{}:{}",
                        item.skeleton.id,
                        item.entity.as_deref().unwrap_or("anonymous"),
                        item_index
                    );
                    let vertex_buffer = match item.morph_target_set.as_ref() {
                        Some(target_set) => {
                            match self.prepare_morphed_vertex_buffer(
                                &format!("{cache_key}:{}", item.mesh.id),
                                &mesh,
                                target_set,
                                &item.morph_weights,
                            ) {
                                Ok(buffer) => buffer,
                                Err(diagnostics) => {
                                    self.active_frame = Some(frame);
                                    return Err(diagnostics);
                                }
                            }
                        }
                        None => mesh.vertex_buffer,
                    };
                    let bone_buffer = match self.prepare_bone_buffer(&cache_key, &item.bone_palette)
                    {
                        Ok(buffer) => buffer,
                        Err(diagnostics) => {
                            self.active_frame = Some(frame);
                            return Err(diagnostics);
                        }
                    };
                    if !frame.encoder.bind_uniform_buffer(layout, bone_buffer) {
                        self.active_frame = Some(frame);
                        return Err(vec![Diagnostic::new(
                            "DX1230",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "DX12 skinned pipeline could not bind its bone palette",
                        )]);
                    }
                    frame.encoder.bind_vertex_buffers(&[vertex_buffer], &[0]);
                    frame
                        .encoder
                        .bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
                    frame.encoder.draw_indexed(mesh.index_count, 1, 0, 0, 0);
                    frame.draw_calls += 1;
                    frame.triangles += u64::from(mesh.index_count / 3);
                }

                if transparent_phase {
                    if let Some(particle_pipelines) = particle_pipelines {
                        let mut current_particle_pipeline = particle_pipelines[0];
                        frame.encoder.bind_pipeline(current_particle_pipeline);
                        let view_projection = projection_matrix * view_matrix;
                        let view_projection_bytes = matrix_bytes(view_projection);
                        let camera_world_bytes = matrix_bytes(view_matrix.inverse());
                        let shadow_disabled = float4_bytes([0.0, 0.0, 0.0, 0.0]);
                        let (_, _, light_direction) =
                            shadow_scene_constants(shadow_frame_data, Mat4::IDENTITY);
                        let mut ordered_batches: Vec<(
                            f32,
                            usize,
                            &engine_renderer::ParticleBatch,
                        )> = input
                            .particle_batches
                            .iter()
                            .enumerate()
                            .map(|(index, batch)| {
                                let center = glam::Vec3::from_array([
                                    (batch.bounds.min[0] + batch.bounds.max[0]) * 0.5,
                                    (batch.bounds.min[1] + batch.bounds.max[1]) * 0.5,
                                    (batch.bounds.min[2] + batch.bounds.max[2]) * 0.5,
                                ]);
                                ((center - camera_position).length_squared(), index, batch)
                            })
                            .collect();
                        order_transparent_back_to_front(
                            &mut ordered_batches,
                            weighted_oit,
                            |item| item.0,
                        );
                        for (_, batch_index, batch) in ordered_batches {
                            let (transparency, _) = self.material_surface(input, &batch.material);
                            let gpu_simulation = batch.gpu_simulation;
                            let weighted_batch =
                                weighted_oit && transparency == Transparency::Blend;
                            let next_particle_pipeline = match (
                                gpu_simulation.is_some(),
                                transparency == Transparency::Additive,
                                weighted_batch,
                            ) {
                                (false, true, _) => particle_pipelines[1],
                                (false, false, true) => particle_pipelines[2],
                                (false, false, false) => particle_pipelines[0],
                                (true, true, _) => particle_pipelines[4],
                                (true, false, true) => particle_pipelines[5],
                                (true, false, false) => particle_pipelines[3],
                            };
                            if next_particle_pipeline != current_particle_pipeline {
                                frame.encoder.bind_pipeline(next_particle_pipeline);
                                current_particle_pipeline = next_particle_pipeline;
                            }
                            let mesh = self.meshes[&batch.mesh.id].clone();
                            let cache_key = format!(
                                "{}:{}:{}",
                                batch.mesh.id,
                                batch.emitter.as_deref().unwrap_or("anonymous"),
                                batch_index
                            );
                            let instance_buffer = if gpu_simulation.is_none() {
                                match self
                                    .prepare_particle_instance_buffer(&cache_key, &batch.instances)
                                {
                                    Ok(Some(buffer)) => Some(buffer),
                                    Ok(None) => continue,
                                    Err(diagnostics) => {
                                        self.active_frame = Some(frame);
                                        return Err(diagnostics);
                                    }
                                }
                            } else {
                                None
                            };
                            let input_material = input
                                .materials
                                .iter()
                                .find(|binding| binding.material_id == batch.material);
                            let texture_ids = self.material_texture_ids(input, &batch.material);
                            let texture_flags = material_texture_flags_from_ids(&texture_ids);
                            let mut material_constants = input_material
                                .map(|binding| {
                                    material_constants_from_bytes(
                                        &binding.uniforms.bytes,
                                        texture_ids[0].is_some(),
                                        &binding.transparency,
                                        weighted_oit,
                                    )
                                })
                                .or_else(|| {
                                    self.materials
                                        .get(&batch.material.id)
                                        .map(|material| material.constants)
                                })
                                .unwrap_or_else(default_material_constants);
                            set_material_surface_flags(
                                &mut material_constants,
                                texture_ids[0].is_some(),
                                &transparency,
                                weighted_oit,
                            );
                            let emissive_constants = input_material
                                .map(|binding| {
                                    emissive_constants_from_bytes(
                                        &binding.uniforms.bytes,
                                        texture_flags,
                                    )
                                })
                                .or_else(|| {
                                    self.materials
                                        .get(&batch.material.id)
                                        .map(|material| material.emissive_constants)
                                })
                                .unwrap_or_else(default_emissive_constants);
                            let advanced_constants = input_material
                                .map(|binding| {
                                    advanced_constants_from_bytes(&binding.uniforms.bytes)
                                })
                                .or_else(|| {
                                    self.materials
                                        .get(&batch.material.id)
                                        .map(|material| material.advanced_constants)
                                })
                                .unwrap_or_else(default_advanced_constants);
                            frame
                                .encoder
                                .push_constants(layout, 0x10, 0, &view_projection_bytes);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 64, &material_constants);
                            frame
                                .encoder
                                .push_constants(layout, 0x10, 96, &camera_world_bytes);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 160, &shadow_disabled);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 176, &light_direction);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 192, &emissive_constants);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 208, &advanced_constants);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 224, &environment_constants);
                            let texture_table = self.material_texture_table(
                                &texture_ids,
                                shadow_texture,
                                environment_texture,
                            );
                            let mut particle_resources = cluster_buffers;
                            if let Some(simulation) = gpu_simulation {
                                particle_resources[3] = match self
                                    .prepare_gpu_particle_parameter_buffer(&cache_key, simulation)
                                {
                                    Ok(buffer) => buffer,
                                    Err(diagnostics) => {
                                        self.active_frame = Some(frame);
                                        return Err(diagnostics);
                                    }
                                };
                            }
                            if !frame.encoder.bind_scene_resource_set(
                                layout,
                                &texture_table,
                                &particle_resources,
                            ) {
                                self.active_frame = Some(frame);
                                return Err(vec![Diagnostic::new(
                            "DX1274",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "DX12 particle pass could not bind its material/environment table",
                        )]);
                            }
                            if let Some(instance_buffer) = instance_buffer {
                                frame.encoder.bind_vertex_buffers(
                                    &[mesh.vertex_buffer, instance_buffer],
                                    &[0, 0],
                                );
                            } else {
                                frame
                                    .encoder
                                    .bind_vertex_buffers(&[mesh.vertex_buffer], &[0]);
                            }
                            frame.encoder.bind_index_buffer(
                                mesh.index_buffer,
                                0,
                                mesh.index_format,
                            );
                            let instance_count = gpu_simulation.map_or_else(
                                || u32::try_from(batch.instances.len()).unwrap_or(u32::MAX),
                                |simulation| simulation.draw_range().1,
                            );
                            frame
                                .encoder
                                .draw_indexed(mesh.index_count, instance_count, 0, 0, 0);
                            frame.draw_calls += 1;
                            frame.triangles +=
                                u64::from(mesh.index_count / 3) * u64::from(instance_count);
                        }
                    }
                }
            }
        }

        self.active_frame = Some(frame);
        Ok(())
    }
}
