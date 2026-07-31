use super::*;

impl SceneRenderer {
    /// Execute the directional shadow (CSM) pass.
    pub(super) fn execute_shadow_pass(
        &mut self,
        input: &RenderFrameInput,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        let Some(shadow_light) = input.lights.iter().find(|light| {
            light.kind == LightKind::Directional
                && matches!(light.shadow_mode, ShadowMode::Hard | ShadowMode::Soft)
        }) else {
            // No directional light requested a shadow map this frame. Do not
            // manufacture a fixed light or issue stale/fake shadow draws.
            apply_extraction_stats(stats, input);
            return Ok(());
        };

        let light_direction = VulkanDevice::normalize_shadow_light_direction(glam::Vec3::from(
            shadow_light.direction,
        ))
        .map_err(|error| {
            vec![Diagnostic::new(
                "RV0286",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("invalid directional shadow light: {error}"),
            )]
        })?;

        let view = input.views.first().ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0287",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional shadow pass requires a RenderView",
            )]
        })?;
        let view_mat = Mat4::from_cols_array(&view.view_matrix);
        let proj_mat = Mat4::from_cols_array(&view.projection_matrix);
        let (near, far) = VulkanDevice::derive_rh_zo_clip_planes(&proj_mat).map_err(|error| {
            vec![Diagnostic::new(
                "RV0288",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot derive directional-shadow clip planes: {error}"),
            )]
        })?;
        let (cascade_splits, light_vps) =
            VulkanDevice::compute_cascade_data(&view_mat, &proj_mat, near, far, light_direction)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0289",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot compute directional-shadow cascades: {error}"),
                    )]
                })?;

        let rp = self.device.shadow_rp.unwrap_or(vk::RenderPass::null());
        let pll = self
            .device
            .shadow_pipeline_layout
            .unwrap_or(vk::PipelineLayout::null());
        let pl = self.device.shadow_pipeline.unwrap_or(vk::Pipeline::null());
        if rp == vk::RenderPass::null()
            || pll == vk::PipelineLayout::null()
            || pl == vk::Pipeline::null()
        {
            return Err(vec![Diagnostic::new(
                "RV0226",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional shadow pass resources are incomplete",
            )]);
        }

        const SHADOW_SIZE: u32 = 2048;
        const CASCADE_COUNT: usize = 3;

        let splits_bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(&cascade_splits as *const _ as *const u8, 16) };
        self.device.write_ubo_current(splits_bytes, 176);

        for (i, lvp) in light_vps.iter().enumerate() {
            let arr: [[f32; 4]; 4] = lvp.to_cols_array_2d();
            let vp_bytes: &[u8] =
                unsafe { std::slice::from_raw_parts(&arr as *const _ as *const u8, 64) };
            self.device
                .write_ubo_current(vp_bytes, 192 + (i as u64 * 64));
        }

        let d = &self.device.logical_device.device;
        let fi = self.device.current_frame;
        let cmd = self.device.frame_sync[fi].command_buffer;

        let clear_value = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };
        let clear_values = [clear_value];

        #[allow(clippy::needless_range_loop)]
        for cascade in 0..CASCADE_COUNT {
            let fb = match self.device.shadow_fbs.get(cascade).copied() {
                Some(fb) => fb,
                None => continue,
            };

            let rpbi = vk::RenderPassBeginInfo::default()
                .render_pass(rp)
                .framebuffer(fb)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D {
                        width: SHADOW_SIZE,
                        height: SHADOW_SIZE,
                    },
                })
                .clear_values(&clear_values);
            unsafe {
                d.cmd_begin_render_pass(cmd, &rpbi, vk::SubpassContents::INLINE);
            }

            let vp = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: SHADOW_SIZE as f32,
                height: SHADOW_SIZE as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            unsafe {
                d.cmd_set_viewport(cmd, 0, &[vp]);
                d.cmd_set_scissor(
                    cmd,
                    0,
                    &[vk::Rect2D {
                        offset: vk::Offset2D { x: 0, y: 0 },
                        extent: vk::Extent2D {
                            width: SHADOW_SIZE,
                            height: SHADOW_SIZE,
                        },
                    }],
                );
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pl);
            }

            let light_vp = light_vps[cascade];

            // Shadow draws with batching (drawables pre-sorted by mesh)
            let mut last_shadow_mesh: Option<&str> = None;
            #[allow(unused_assignments)]
            let mut shadow_cached_vb = vk::Buffer::null();
            #[allow(unused_assignments)]
            let mut shadow_cached_ib = vk::Buffer::null();
            let mut shadow_cached_index_count = 0u32;
            for drawable in &input.drawables {
                if !drawable.cast_shadows {
                    last_shadow_mesh = None;
                    continue;
                }
                if matches!(
                    self.material_binding_for_drawable(input, &drawable.material)?
                        .transparency,
                    engine_renderer::Transparency::Blend | engine_renderer::Transparency::Additive
                ) {
                    last_shadow_mesh = None;
                    continue;
                }

                let mesh_id = &drawable.mesh.id;

                if Some(mesh_id.as_str()) != last_shadow_mesh {
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
                            if vk_vb == vk::Buffer::null() || vk_ib == vk::Buffer::null() {
                                last_shadow_mesh = None;
                                continue;
                            }
                            shadow_cached_vb = vk_vb;
                            shadow_cached_ib = vk_ib;
                            shadow_cached_index_count = m.index_count;
                            let shadow_index_type = vulkan_index_type(m.index_format);
                            last_shadow_mesh = Some(mesh_id.as_str());
                            // Bind VB/IB
                            let vbs = [shadow_cached_vb];
                            let offsets = [0u64];
                            unsafe {
                                d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                                d.cmd_bind_index_buffer(
                                    cmd,
                                    shadow_cached_ib,
                                    0,
                                    shadow_index_type,
                                );
                            }
                        }
                        None => {
                            tracing::trace!(
                                target: "scene_renderer",
                                mesh = mesh_id,
                                "skipping un-cached mesh in shadow pass"
                            );
                            last_shadow_mesh = None;
                            continue;
                        }
                    }
                }

                let world = Mat4::from_cols_array(&drawable.world_transform);
                let mvp = light_vp * world;
                unsafe {
                    let mut pc_bytes = [0u8; 128];
                    for (index, value) in mvp.to_cols_array().into_iter().enumerate() {
                        let offset = index * 4;
                        pc_bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
                    }
                    if let Some(morph) = &drawable.radial_vertex_morph {
                        for (index, value) in [morph.factor, morph.delta_scale, 1.0, 0.0]
                            .into_iter()
                            .enumerate()
                        {
                            let offset = 64 + index * 4;
                            pc_bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
                        }
                        for (index, value) in morph
                            .local_origin
                            .into_iter()
                            .chain(std::iter::once(0.0))
                            .enumerate()
                        {
                            let offset = 80 + index * 4;
                            pc_bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
                        }
                    }
                    d.cmd_push_constants(cmd, pll, vk::ShaderStageFlags::VERTEX, 0, &pc_bytes);
                    d.cmd_draw_indexed(cmd, shadow_cached_index_count, 1, 0, 0, 0);
                }

                stats.draw_calls += 1;
                stats.triangles += shadow_cached_index_count as u64 / 3;
            }

            unsafe {
                d.cmd_end_render_pass(cmd);
            }
        }

        // Global barrier: cascade layers -> shader readable
        if let Some(sm) = self.device.shadow_map {
            let barrier = vk::ImageMemoryBarrier::default()
                .image(sm)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: CASCADE_COUNT as u32,
                })
                .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
                .new_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
            unsafe {
                d.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
            }
        }

        apply_extraction_stats(stats, input);
        Ok(())
    }
}
