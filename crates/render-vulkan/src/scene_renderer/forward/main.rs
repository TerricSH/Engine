use super::super::*;

impl SceneRenderer {
    pub(in super::super) fn execute_hdr_forward_pass(
        &mut self,
        input: &RenderFrameInput,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        let hdr_rp = self.device.hdr_forward_rp.unwrap_or(vk::RenderPass::null());
        let hdr_fb = self
            .device
            .hdr_forward_fb
            .unwrap_or(vk::Framebuffer::null());
        let hdr_pl = self
            .device
            .hdr_forward_pipeline
            .unwrap_or(vk::Pipeline::null());
        let hdr_pll = self
            .device
            .hdr_forward_pipeline_layout
            .unwrap_or(vk::PipelineLayout::null());
        if hdr_rp == vk::RenderPass::null()
            || hdr_fb == vk::Framebuffer::null()
            || hdr_pl == vk::Pipeline::null()
            || hdr_pll == vk::PipelineLayout::null()
        {
            return Err(vec![Diagnostic::new(
                "RV0225",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "HDR forward pass resources are incomplete",
            )]);
        }
        let weighted_oit = input.render_options.transparency_mode
            == engine_renderer::TransparencyMode::WeightedBlendedOit;

        // Clone device + cmd handles to avoid borrow-checker conflicts
        let d = self.device.logical_device.device.clone();
        let fi = self.device.current_frame;
        let cmd = self.device.frame_sync[fi].command_buffer;

        let render_view = input.views.first().ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0013",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "HDR forward pass requires a RenderView",
            )]
        })?;

        // The primary directional light remains in the compact frame UBO.
        // Every other light is assigned to conservative screen/depth clusters.
        let primary_directional = input
            .lights
            .iter()
            .find(|light| matches!(light.kind, LightKind::Directional));
        let additional_lights = input
            .lights
            .iter()
            .filter(|light| {
                primary_directional.is_none_or(|primary| !std::ptr::eq(*light, primary))
            })
            .collect::<Vec<_>>();
        let (direction, color, intensity) =
            primary_directional.map_or(([0.0, -1.0, 0.0], [0.0; 3], 0.0), |light| {
                (
                    normalize_direction(light.direction),
                    light.color,
                    light.intensity,
                )
            });
        let mut direction_bytes = [0; 16];
        let mut color_bytes = [0; 16];
        for (index, value) in direction.into_iter().enumerate() {
            direction_bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
        }
        for (index, value) in color.into_iter().enumerate() {
            color_bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
        }
        color_bytes[12..16].copy_from_slice(&intensity.to_ne_bytes());
        self.device.write_ubo(fi, &direction_bytes, 128);
        self.device.write_ubo(fi, &color_bytes, 144);

        let clustered =
            build_clustered_light_frame(&additional_lights, render_view, self.width, self.height);
        self.device.write_clustered_lighting_buffers(
            &clustered.light_bytes,
            &clustered.cluster_grid_bytes,
            &clustered.cluster_index_bytes,
        );
        let _cluster_metrics = (
            clustered.light_count,
            clustered.cluster_count,
            clustered.index_count,
            clustered.overflowed_assignments,
        );
        let prepared_particles =
            prepare_particle_instances(&input.particle_batches).map_err(|message| {
                vec![Diagnostic::new(
                    "RV0337",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    message,
                )]
            })?;
        let particle_instance_buffer = self.upload_particle_instance_stream(&prepared_particles)?;

        let scene_viewport = vulkan_viewport_rect(
            render_view.viewport_rect_normalized,
            self.width,
            self.height,
        )
        .map_err(|message| {
            vec![Diagnostic::new(
                "RV0318",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )]
        })?;
        // Render-pass load operations clear only the scene's render area. The
        // rest of the HDR attachment is intentionally never sampled by the
        // sub-viewport tone-map draw.
        let clear_color = render_view.clear_color;
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: clear_color,
                },
            },
            vk::ClearValue {
                color: vk::ClearColorValue { float32: [0.0; 4] },
            },
            vk::ClearValue {
                color: vk::ClearColorValue { float32: [0.0; 4] },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];
        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(hdr_rp)
            .framebuffer(hdr_fb)
            .render_area(scene_viewport.scissor)
            .clear_values(&clear_values);
        // SAFETY: command buffer is in recording state; RP, FB valid.
        unsafe {
            d.cmd_begin_render_pass(cmd, &rpbi, vk::SubpassContents::INLINE);
        }

        // SAFETY: `cmd` records inside the pass with dynamic viewport/scissor;
        // both call-scoped values match the render area.
        unsafe {
            d.cmd_set_viewport(cmd, 0, &[scene_viewport.viewport]);
            d.cmd_set_scissor(cmd, 0, &[scene_viewport.scissor]);
        }

        // Draw the environment cubemap before opaque geometry. The skybox
        // pipeline does not write depth, so all scene geometry naturally
        // replaces it while untouched pixels retain the environment.
        if input
            .views
            .first()
            .is_some_and(|view| view.clear_flags == engine_renderer::ClearFlags::Skybox)
        {
            let skybox_pipeline = self.device.hdr_skybox_pipeline.ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0321",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "HDR skybox pipeline is unavailable",
                )]
            })?;
            // SAFETY: the skybox pipeline is live and compatible with the active HDR pass.
            unsafe {
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, skybox_pipeline);
            }
            if let Some(desc_set) = self.device.frame_descriptor_set(fi) {
                // SAFETY: set/layout are compatible live handles and `cmd` records this pass.
                unsafe {
                    d.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        hdr_pll,
                        0,
                        &[desc_set],
                        &[],
                    );
                }
            }
            if let Some(desc_set) = self.device.shadow_desc_set {
                // SAFETY: the live shadow set matches set=1; the pass remains recording.
                unsafe {
                    d.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        hdr_pll,
                        1,
                        &[desc_set],
                        &[],
                    );
                    d.cmd_draw(cmd, 36, 1, 0, 0);
                }
                stats.draw_calls += 1;
                stats.triangles += 12;
            }
        }

        // Bind HDR forward pipeline
        // SAFETY: `hdr_pl` is live and compatible with the active HDR render pass.
        unsafe {
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, hdr_pl);
        }

        // Bind UBO descriptor set (set=0)
        if let Some(desc_set) = self.device.frame_descriptor_set(fi) {
            let sets = [desc_set];
            // SAFETY: the live frame set matches set=0 and its slice is call-scoped.
            unsafe {
                d.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    hdr_pll,
                    0,
                    &sets,
                    &[],
                );
            }
        }

        // Bind the shadow/environment/light descriptor set with the exact
        // layout used by the HDR pipeline. Earlier passes may have bound set=1
        // through a different pipeline layout, which does not guarantee that
        // the binding remains compatible after set=0 is rebound above.
        if let Some(desc_set) = self.device.shadow_desc_set {
            let sets = [desc_set];
            // SAFETY: the live shadow set matches set=1 and its slice is call-scoped.
            unsafe {
                d.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    hdr_pll,
                    1,
                    &sets,
                    &[],
                );
            }
        }

        // Opaque/masked drawables retain extraction order for batching. Blended
        // drawables render afterwards, back-to-front, with depth writes disabled.
        let camera_position = Mat4::from_cols_array(&render_view.view_matrix)
            .inverse()
            .w_axis
            .truncate();
        for transparent_phase in [false, true] {
            let mut ordered_drawables = Vec::with_capacity(input.drawables.len());
            let mut blended_drawables = Vec::new();
            for drawable in &input.drawables {
                let material = self.material_binding_for_drawable(input, &drawable.material)?;
                let transparent = matches!(
                    material.transparency,
                    engine_renderer::Transparency::Blend | engine_renderer::Transparency::Additive
                );
                if transparent != transparent_phase {
                    continue;
                }
                if transparent {
                    let translation = Vec3::new(
                        drawable.world_transform[12],
                        drawable.world_transform[13],
                        drawable.world_transform[14],
                    );
                    blended_drawables
                        .push(((translation - camera_position).length_squared(), drawable));
                } else {
                    ordered_drawables.push(drawable);
                }
            }
            let opaque_drawable_count = ordered_drawables.len();
            order_transparent_back_to_front(&mut blended_drawables, weighted_oit, |item| item.0);
            ordered_drawables.extend(blended_drawables.into_iter().map(|(_, drawable)| drawable));
            let prepared_static = prepare_static_instances(
                &ordered_drawables[..opaque_drawable_count],
            )
            .map_err(|message| {
                vec![Diagnostic::new(
                    "RV0348",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    message,
                )]
            })?;
            let static_instance_buffer = self.upload_static_instance_stream(&prepared_static)?;
            let static_draws_by_start = prepared_static
                .draws
                .iter()
                .copied()
                .map(|draw| (draw.first_drawable, draw))
                .collect::<BTreeMap<_, _>>();

            // Draw calls with dynamic batching.
            let mut last_material_id: Option<&str> = None;
            let mut last_mesh_id: Option<&str> = None;
            let mut current_material_pipeline = hdr_pl;
            #[allow(unused_assignments)]
            let mut cached_vb = vk::Buffer::null();
            #[allow(unused_assignments)]
            let mut cached_ib = vk::Buffer::null();
            #[allow(unused_assignments)]
            let mut cached_idx_ty = vk::IndexType::UINT32;
            let mut cached_index_count = 0u32;
            let mut drawable_index = 0_usize;
            while drawable_index < ordered_drawables.len() {
                if let (Some(instance_buffer), Some(instance_draw)) = (
                    static_instance_buffer,
                    static_draws_by_start.get(&drawable_index).copied(),
                ) {
                    let drawable = ordered_drawables[drawable_index];
                    let mesh = self.meshes.get(&drawable.mesh.id).cloned().ok_or_else(|| {
                        vec![Diagnostic::new(
                            "RV0349",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!(
                                "instanced drawable references mesh '{}' before upload",
                                drawable.mesh.id
                            ),
                        )]
                    })?;
                    if mesh.vertex_format != MeshVertexFormat::Pbr32 {
                        return Err(vec![Diagnostic::new(
                            "RV0350",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!(
                                "instanced mesh '{}' must use the Pbr32 vertex format",
                                drawable.mesh.id
                            ),
                        )]);
                    }
                    let vertex_buffer = self
                        .device
                        .buffers
                        .get(mesh.vertex_buffer.index, mesh.vertex_buffer.generation)
                        .map(|entry| entry.buffer)
                        .ok_or_else(|| {
                            vec![Diagnostic::new(
                                "RV0351",
                                DiagnosticSeverity::Fatal,
                                "scene_renderer",
                                "instanced mesh vertex buffer became invalid",
                            )]
                        })?;
                    let index_buffer = self
                        .device
                        .buffers
                        .get(mesh.index_buffer.index, mesh.index_buffer.generation)
                        .map(|entry| entry.buffer)
                        .ok_or_else(|| {
                            vec![Diagnostic::new(
                                "RV0352",
                                DiagnosticSeverity::Fatal,
                                "scene_renderer",
                                "instanced mesh index buffer became invalid",
                            )]
                        })?;
                    let material = self.material_binding_for_drawable(input, &drawable.material)?;
                    let instance_pipeline = if material.double_sided {
                        self.device.hdr_instanced_double_sided_pipeline
                    } else {
                        self.device.hdr_instanced_pipeline
                    }
                    .ok_or_else(|| {
                        vec![Diagnostic::new(
                            "RV0353",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "static instancing pipeline is unavailable",
                        )]
                    })?;
                    let mut material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
                    material_ubo.emissive[3] = Self::material_texture_flags(&material);
                    // SAFETY: `MaterialUBO` is fully initialized `repr(C)`
                    // all-`f32` storage; the byte slice cannot outlive it.
                    let material_bytes: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            &material_ubo as *const _ as *const u8,
                            std::mem::size_of::<MaterialUBO>(),
                        )
                    };
                    let (material_set, _) = self
                        .get_or_create_material_desc_set(&drawable.material.id, material_bytes)?;
                    self.bind_material_texture_if_changed(
                        &drawable.material.id,
                        &material,
                        material_set,
                    )?;
                    // SAFETY: handles are live/compatible, validated buffers cover
                    // the instance ranges, and `cmd` records the active pass.
                    unsafe {
                        d.cmd_bind_pipeline(
                            cmd,
                            vk::PipelineBindPoint::GRAPHICS,
                            instance_pipeline,
                        );
                        d.cmd_bind_descriptor_sets(
                            cmd,
                            vk::PipelineBindPoint::GRAPHICS,
                            hdr_pll,
                            2,
                            &[material_set],
                            &[],
                        );
                        d.cmd_bind_vertex_buffers(
                            cmd,
                            0,
                            &[vertex_buffer, instance_buffer],
                            &[0, 0],
                        );
                        d.cmd_bind_index_buffer(
                            cmd,
                            index_buffer,
                            0,
                            vulkan_index_type(mesh.index_format),
                        );
                        d.cmd_draw_indexed(
                            cmd,
                            mesh.index_count,
                            instance_draw.instance_count,
                            0,
                            0,
                            instance_draw.first_instance,
                        );
                    }
                    stats.draw_calls += 1;
                    stats.triangles +=
                        u64::from(mesh.index_count / 3) * u64::from(instance_draw.instance_count);
                    drawable_index += instance_draw.drawable_count;
                    last_material_id = None;
                    last_mesh_id = None;
                    current_material_pipeline = instance_pipeline;
                    continue;
                }

                let drawable = ordered_drawables[drawable_index];
                let mesh_id = &drawable.mesh.id;
                let material_id = &drawable.material.id;

                // Look up mesh buffers; cache across consecutive same-mesh drawables
                if Some(mesh_id.as_str()) != last_mesh_id {
                    if let Some(m) = self.meshes.get(mesh_id).cloned() {
                        let vk_vb = self
                            .device
                            .buffers
                            .get(m.vertex_buffer.index, m.vertex_buffer.generation)
                            .map(|e| e.buffer)
                            .unwrap_or(vk::Buffer::null());
                        let vk_ib = self
                            .device
                            .buffers
                            .get(m.index_buffer.index, m.index_buffer.generation)
                            .map(|e| e.buffer)
                            .unwrap_or(vk::Buffer::null());
                        if vk_vb == vk::Buffer::null() {
                            last_material_id = None;
                            last_mesh_id = None;
                            cached_index_count = 0;
                            drawable_index += 1;
                            continue;
                        }
                        cached_vb = vk_vb;
                        cached_ib = vk_ib;
                        cached_idx_ty = vulkan_index_type(m.index_format);
                        cached_index_count = m.index_count;
                        last_mesh_id = Some(mesh_id.as_str());
                        // Bind VB/IB
                        let vbs = [cached_vb];
                        let offsets = [0u64];
                        // SAFETY: generation-checked buffers are live and both
                        // binding slices remain valid through command recording.
                        unsafe {
                            d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                            d.cmd_bind_index_buffer(cmd, cached_ib, 0, cached_idx_ty);
                        }
                    } else {
                        tracing::trace!(
                            target: "scene_renderer",
                            mesh = mesh_id,
                            "skipping un-cached mesh in HDR forward pass"
                        );
                        last_material_id = None;
                        last_mesh_id = None;
                        drawable_index += 1;
                        continue;
                    }
                }
                // When the mesh is unchanged, the vertex and index buffers remain bound.

                // Skip material descriptor rebind when same as last drawable
                if Some(material_id.as_str()) != last_material_id {
                    let material = self.material_binding_for_drawable(input, &drawable.material)?;
                    let next_pipeline = match (&material.transparency, material.double_sided) {
                        (engine_renderer::Transparency::Blend, true) if weighted_oit => self
                            .device
                            .hdr_forward_oit_double_sided_pipeline
                            .unwrap_or(vk::Pipeline::null()),
                        (engine_renderer::Transparency::Blend, false) if weighted_oit => self
                            .device
                            .hdr_forward_oit_pipeline
                            .unwrap_or(vk::Pipeline::null()),
                        (engine_renderer::Transparency::Blend, true) => self
                            .device
                            .hdr_forward_blend_double_sided_pipeline
                            .unwrap_or(vk::Pipeline::null()),
                        (engine_renderer::Transparency::Blend, false) => self
                            .device
                            .hdr_forward_blend_pipeline
                            .unwrap_or(vk::Pipeline::null()),
                        (engine_renderer::Transparency::Additive, true) => self
                            .device
                            .hdr_forward_additive_double_sided_pipeline
                            .unwrap_or(vk::Pipeline::null()),
                        (engine_renderer::Transparency::Additive, false) => self
                            .device
                            .hdr_forward_additive_pipeline
                            .unwrap_or(vk::Pipeline::null()),
                        (_, true) => self
                            .device
                            .hdr_forward_double_sided_pipeline
                            .unwrap_or(vk::Pipeline::null()),
                        (_, false) => hdr_pl,
                    };
                    if next_pipeline == vk::Pipeline::null() {
                        return Err(vec![Diagnostic::new(
                            "RV0322",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!(
                                "material '{}' requires an unavailable surface pipeline",
                                material.material_id.id
                            ),
                        )]);
                    }
                    if next_pipeline != current_material_pipeline {
                        // SAFETY: `next_pipeline` is non-null/live and compatible with this pass.
                        unsafe {
                            d.cmd_bind_pipeline(
                                cmd,
                                vk::PipelineBindPoint::GRAPHICS,
                                next_pipeline,
                            );
                        }
                        current_material_pipeline = next_pipeline;
                    }
                    let mut material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
                    if weighted_oit && material.transparency == engine_renderer::Transparency::Blend
                    {
                        material_ubo.alpha_cutoff = -2.0;
                    }
                    material_ubo.emissive[3] = Self::material_texture_flags(&material);
                    // SAFETY: this fully initialized `repr(C)` all-`f32` value
                    // has no padding and the byte view is locally bounded.
                    let ubo_bytes: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            &material_ubo as *const _ as *const u8,
                            std::mem::size_of::<MaterialUBO>(),
                        )
                    };
                    let (mat_desc_set, _mat_buf) =
                        self.get_or_create_material_desc_set(material_id, ubo_bytes)?;
                    self.bind_material_texture_if_changed(material_id, &material, mat_desc_set)?;
                    let sets = [mat_desc_set];
                    // SAFETY: the live material set matches set=2; its slice is call-scoped.
                    unsafe {
                        d.cmd_bind_descriptor_sets(
                            cmd,
                            vk::PipelineBindPoint::GRAPHICS,
                            hdr_pll,
                            2,
                            &sets,
                            &[],
                        );
                    }
                    last_material_id = Some(material_id.as_str());
                }

                // Push constants: world transform, radial geomorph, then the
                // per-chunk continuous material projection (128 B total).
                let pc_bytes = static_draw_push_constants(drawable);
                // SAFETY: this byte range fits the declared vertex push-constant range.
                unsafe {
                    d.cmd_push_constants(cmd, hdr_pll, vk::ShaderStageFlags::VERTEX, 0, &pc_bytes);
                }

                // Draw indexed
                // SAFETY: compatible pipeline/descriptors and validated mesh buffers are bound.
                unsafe {
                    d.cmd_draw_indexed(cmd, cached_index_count, 1, 0, 0, 0);
                }

                stats.draw_calls += 1;
                stats.triangles += cached_index_count as u64 / 3;
                drawable_index += 1;
            }

            // Skinned items use the same surface variants and transparent ordering.
            let mut ordered_skinned = Vec::with_capacity(input.skinned_items.len());
            let mut blended_skinned = Vec::new();
            for item in &input.skinned_items {
                let material = self.material_binding_for_drawable(input, &item.material)?;
                let transparent = matches!(
                    material.transparency,
                    engine_renderer::Transparency::Blend | engine_renderer::Transparency::Additive
                );
                if transparent != transparent_phase {
                    continue;
                }
                if transparent {
                    let translation = Vec3::new(
                        item.world_transform[12],
                        item.world_transform[13],
                        item.world_transform[14],
                    );
                    blended_skinned.push(((translation - camera_position).length_squared(), item));
                } else {
                    ordered_skinned.push(item);
                }
            }
            order_transparent_back_to_front(&mut blended_skinned, weighted_oit, |item| item.0);
            ordered_skinned.extend(blended_skinned.into_iter().map(|(_, item)| item));

            // Skinned items (less batching opportunity due to unique per-item bone data)
            let mut last_skinned_mesh: Option<&str> = None;
            #[allow(unused_assignments)]
            let mut skinned_cached_vb = vk::Buffer::null();
            #[allow(unused_assignments)]
            let mut skinned_cached_ib = vk::Buffer::null();
            #[allow(unused_assignments)]
            let mut skinned_cached_idx_ty = vk::IndexType::UINT32;
            let mut skinned_cached_index_count = 0u32;
            let mut skinned_cached_vertex_count = 0u32;
            for skinned in ordered_skinned {
                let mesh_id = &skinned.mesh.id;
                let material_id = &skinned.material.id;

                // Cache VB/IB, skip on missing mesh
                if Some(mesh_id.as_str()) != last_skinned_mesh {
                    match self.meshes.get(mesh_id).cloned() {
                        Some(m) => {
                            let vk_vb = self
                                .device
                                .buffers
                                .get(m.vertex_buffer.index, m.vertex_buffer.generation)
                                .map(|e| e.buffer)
                                .unwrap_or(vk::Buffer::null());
                            let vk_ib = self
                                .device
                                .buffers
                                .get(m.index_buffer.index, m.index_buffer.generation)
                                .map(|e| e.buffer)
                                .unwrap_or(vk::Buffer::null());
                            if vk_vb == vk::Buffer::null() {
                                last_skinned_mesh = None;
                                continue;
                            }
                            skinned_cached_vb = vk_vb;
                            skinned_cached_ib = vk_ib;
                            skinned_cached_index_count = m.index_count;
                            skinned_cached_vertex_count = m.vertex_count;
                            skinned_cached_idx_ty = vulkan_index_type(m.index_format);
                            last_skinned_mesh = Some(mesh_id.as_str());
                            let vbs = [skinned_cached_vb];
                            let offsets = [0u64];
                            // SAFETY: generation-checked skinned buffers are live;
                            // binding slices remain valid for the recording call.
                            unsafe {
                                d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                                d.cmd_bind_index_buffer(
                                    cmd,
                                    skinned_cached_ib,
                                    0,
                                    skinned_cached_idx_ty,
                                );
                            }
                        }
                        None => {
                            tracing::trace!(
                                target: "scene_renderer",
                                mesh = mesh_id,
                                "skipping un-cached skinned mesh in HDR forward pass"
                            );
                            last_skinned_mesh = None;
                            continue;
                        }
                    }
                }

                // Per-item: material descriptor, bone buffer, skinned descriptor set
                let material = self.material_binding_for_drawable(input, &skinned.material)?;
                let next_pipeline = match (&material.transparency, material.double_sided) {
                    (engine_renderer::Transparency::Blend, true) if weighted_oit => self
                        .device
                        .hdr_forward_oit_double_sided_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (engine_renderer::Transparency::Blend, false) if weighted_oit => self
                        .device
                        .hdr_forward_oit_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (engine_renderer::Transparency::Blend, true) => self
                        .device
                        .hdr_forward_blend_double_sided_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (engine_renderer::Transparency::Blend, false) => self
                        .device
                        .hdr_forward_blend_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (engine_renderer::Transparency::Additive, true) => self
                        .device
                        .hdr_forward_additive_double_sided_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (engine_renderer::Transparency::Additive, false) => self
                        .device
                        .hdr_forward_additive_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (_, true) => self
                        .device
                        .hdr_forward_double_sided_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (_, false) => hdr_pl,
                };
                if next_pipeline == vk::Pipeline::null() {
                    return Err(vec![Diagnostic::new(
                        "RV0322",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "material '{}' requires an unavailable surface pipeline",
                            material.material_id.id
                        ),
                    )]);
                }
                if next_pipeline != current_material_pipeline {
                    // SAFETY: the selected non-null pipeline is live and pass-compatible.
                    unsafe {
                        d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, next_pipeline);
                    }
                    current_material_pipeline = next_pipeline;
                }
                let mut material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
                if weighted_oit && material.transparency == engine_renderer::Transparency::Blend {
                    material_ubo.alpha_cutoff = -2.0;
                }
                material_ubo.emissive[3] = Self::material_texture_flags(&material);
                // SAFETY: `MaterialUBO` is fully initialized `repr(C)` all-`f32`
                // storage and this byte view is bounded by its local lifetime.
                let ubo_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        &material_ubo as *const _ as *const u8,
                        std::mem::size_of::<MaterialUBO>(),
                    )
                };
                let (_, mat_buf) = self.get_or_create_material_desc_set(material_id, ubo_bytes)?;

                let skeleton_id = &skinned.skeleton.id;
                let bone_buf =
                    self.get_or_create_bone_buffer(skeleton_id, &skinned.bone_palette)?;
                let (morph_target_set_id, morph_buffer, morph_target_count, morph_vertex_count) =
                    if let Some(target_set_id) = &skinned.morph_target_set {
                        let target_set = self
                            .morph_target_sets
                            .get(&target_set_id.id)
                            .cloned()
                            .ok_or_else(|| {
                                vec![Diagnostic::new(
                                    "RV0330",
                                    DiagnosticSeverity::Error,
                                    "scene_renderer",
                                    format!(
                                    "skinned item references morph target set '{}' before upload",
                                    target_set_id.id
                                ),
                                )]
                            })?;
                        if target_set.vertex_count != skinned_cached_vertex_count
                            || skinned.morph_weights.len() > target_set.target_count as usize
                        {
                            return Err(vec![Diagnostic::new(
                            "RV0331",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!(
                                "morph target set '{}' is incompatible with mesh '{}' or its weights",
                                target_set_id.id, mesh_id
                            ),
                        )]);
                        }
                        (
                            Some(target_set_id.id.as_str()),
                            target_set.buffer,
                            target_set.target_count,
                            target_set.vertex_count,
                        )
                    } else {
                        (None, self.ensure_fallback_morph_buffer()?, 0, 0)
                    };

                let skinned_desc_set = self.get_or_create_skinned_desc_set(
                    material_id,
                    skeleton_id,
                    morph_target_set_id,
                    mat_buf,
                    bone_buf,
                    morph_buffer,
                )?;

                let skinned_cache_key = format!(
                    "{material_id}:{skeleton_id}:{}",
                    morph_target_set_id.unwrap_or("<none>")
                );
                self.bind_skinned_texture_if_changed(
                    &skinned_cache_key,
                    &material,
                    skinned_desc_set,
                )?;
                let sets = [skinned_desc_set];
                // SAFETY: the live skinned set matches set=2 and the slice is call-scoped.
                unsafe {
                    d.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        hdr_pll,
                        2,
                        &sets,
                        &[],
                    );
                }

                let mut pc_bytes = Vec::with_capacity(128);
                for value in &skinned.world_transform {
                    pc_bytes.extend_from_slice(&value.to_ne_bytes());
                }
                let mut morph_weights = [0.0f32; engine_renderer::MAX_MORPH_TARGETS];
                morph_weights[..skinned.morph_weights.len()]
                    .copy_from_slice(&skinned.morph_weights);
                for value in morph_weights {
                    pc_bytes.extend_from_slice(&value.to_ne_bytes());
                }
                for value in [morph_target_count, morph_vertex_count, 0, 0] {
                    pc_bytes.extend_from_slice(&value.to_ne_bytes());
                }
                pc_bytes.resize(128, 0);
                // SAFETY: the 128-byte range matches the layout; compatible
                // pipeline/descriptors and validated indexed buffers are bound.
                unsafe {
                    d.cmd_push_constants(cmd, hdr_pll, vk::ShaderStageFlags::VERTEX, 0, &pc_bytes);
                    d.cmd_draw_indexed(cmd, skinned_cached_index_count, 1, 0, 0, 0);
                }

                stats.draw_calls += 1;
                stats.triangles += skinned_cached_index_count as u64 / 3;
            }

            self.execute_particle_draws(
                input,
                stats,
                &prepared_particles,
                particle_instance_buffer,
                transparent_phase,
                hdr_pll,
            )?;
        }

        // End HDR render pass
        // SAFETY: this function began the still-active pass on recording `cmd`.
        unsafe {
            d.cmd_end_render_pass(cmd);
        }

        apply_extraction_stats(stats, input);
        Ok(())
    }
}
