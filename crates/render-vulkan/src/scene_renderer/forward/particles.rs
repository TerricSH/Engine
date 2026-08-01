use super::super::*;

impl SceneRenderer {
    pub(super) fn execute_particle_draws(
        &mut self,
        input: &RenderFrameInput,
        stats: &mut FrameStats,
        prepared_particles: &PreparedParticleInstances,
        particle_instance_buffer: Option<vk::Buffer>,
        transparent_phase: bool,
        hdr_pll: vk::PipelineLayout,
    ) -> Result<(), Vec<Diagnostic>> {
        let weighted_oit = input.render_options.transparency_mode
            == engine_renderer::TransparencyMode::WeightedBlendedOit;
        let d = self.device.logical_device.device.clone();
        let frame_index = self.device.current_frame;
        let cmd = self.device.frame_sync[frame_index].command_buffer;
        let render_view = input.views.first().ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0013",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "HDR particle pass requires a RenderView",
            )]
        })?;
        let camera_position = Mat4::from_cols_array(&render_view.view_matrix)
            .inverse()
            .w_axis
            .truncate();

        // Particle emitters use a compact per-frame instance stream and one
        // draw per mesh/material batch. This replaces the former one-drawable
        // per-particle path while keeping authored materials in set=2.
        if transparent_phase && !input.particle_batches.is_empty() {
            let particle_pipeline = self.device.hdr_vfx_billboard_pipeline.ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0338",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "VFX billboard pipeline is unavailable",
                )]
            })?;
            let particle_additive_pipeline = self
                .device
                .hdr_vfx_billboard_additive_pipeline
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0338",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "additive VFX billboard pipeline is unavailable",
                    )]
                })?;
            let particle_oit_pipeline =
                self.device.hdr_vfx_billboard_oit_pipeline.ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0338",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "weighted-OIT VFX billboard pipeline is unavailable",
                    )]
                })?;
            let gpu_particle_pipeline =
                self.device.hdr_gpu_vfx_billboard_pipeline.ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0338",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "GPU VFX billboard pipeline is unavailable",
                    )]
                })?;
            let gpu_particle_additive_pipeline = self
                .device
                .hdr_gpu_vfx_billboard_additive_pipeline
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0338",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "additive GPU VFX billboard pipeline is unavailable",
                    )]
                })?;
            let gpu_particle_oit_pipeline = self
                .device
                .hdr_gpu_vfx_billboard_oit_pipeline
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0338",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "weighted-OIT GPU VFX billboard pipeline is unavailable",
                    )]
                })?;
            let mut current_particle_pipeline = vk::Pipeline::null();

            let camera_world = Mat4::from_cols_array(&render_view.view_matrix).inverse();
            let camera_right = camera_world.x_axis.truncate().normalize_or_zero();
            let camera_up = camera_world.y_axis.truncate().normalize_or_zero();
            let mut billboard_push = [0_u8; 128];
            for (index, value) in [
                camera_right.x,
                camera_right.y,
                camera_right.z,
                0.0,
                camera_up.x,
                camera_up.y,
                camera_up.z,
                0.0,
            ]
            .into_iter()
            .enumerate()
            {
                billboard_push[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
            }
            let mut particle_draws = prepared_particles.draws.clone();
            particle_draws.extend(
                input
                    .particle_batches
                    .iter()
                    .enumerate()
                    .filter_map(|(batch_index, batch)| {
                        batch.gpu_simulation.map(|simulation| PreparedParticleDraw {
                            batch_index,
                            first_instance: 0,
                            instance_count: simulation.draw_range().1,
                        })
                    })
                    .filter(|draw| draw.instance_count > 0),
            );
            order_transparent_back_to_front(
                &mut particle_draws,
                weighted_oit,
                |draw: &PreparedParticleDraw| {
                    let bounds = input.particle_batches[draw.batch_index].bounds;
                    let center = Vec3::new(
                        (bounds.min[0] + bounds.max[0]) * 0.5,
                        (bounds.min[1] + bounds.max[1]) * 0.5,
                        (bounds.min[2] + bounds.max[2]) * 0.5,
                    );
                    (center - camera_position).length_squared()
                },
            );

            for draw in particle_draws {
                let batch = &input.particle_batches[draw.batch_index];
                let mesh = self.meshes.get(&batch.mesh.id).cloned().ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0339",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "particle batch references mesh '{}' before upload",
                            batch.mesh.id
                        ),
                    )]
                })?;
                if mesh.vertex_format != MeshVertexFormat::Pbr32 {
                    return Err(vec![Diagnostic::new(
                        "RV0340",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "particle mesh '{}' must use the Pbr32 vertex format",
                            batch.mesh.id
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
                            "RV0341",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            "particle mesh vertex buffer became invalid",
                        )]
                    })?;
                let index_buffer = self
                    .device
                    .buffers
                    .get(mesh.index_buffer.index, mesh.index_buffer.generation)
                    .map(|entry| entry.buffer)
                    .ok_or_else(|| {
                        vec![Diagnostic::new(
                            "RV0342",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            "particle mesh index buffer became invalid",
                        )]
                    })?;
                let material = self.material_binding_for_drawable(input, &batch.material)?;
                let gpu_simulation = batch.gpu_simulation;
                let additive = material.transparency == engine_renderer::Transparency::Additive;
                let weighted_batch =
                    weighted_oit && material.transparency == engine_renderer::Transparency::Blend;
                let next_particle_pipeline =
                    match (gpu_simulation.is_some(), additive, weighted_batch) {
                        (false, true, _) => particle_additive_pipeline,
                        (false, false, true) => particle_oit_pipeline,
                        (false, false, false) => particle_pipeline,
                        (true, true, _) => gpu_particle_additive_pipeline,
                        (true, false, true) => gpu_particle_oit_pipeline,
                        (true, false, false) => gpu_particle_pipeline,
                    };
                if next_particle_pipeline != current_particle_pipeline {
                    // SAFETY: selected particle pipeline is live and compatible with the active pass.
                    unsafe {
                        d.cmd_bind_pipeline(
                            cmd,
                            vk::PipelineBindPoint::GRAPHICS,
                            next_particle_pipeline,
                        );
                    }
                    current_particle_pipeline = next_particle_pipeline;
                }
                let mut material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
                if weighted_batch {
                    material_ubo.alpha_cutoff = -2.0;
                }
                material_ubo.emissive[3] = Self::material_texture_flags(&material);
                // SAFETY: `MaterialUBO` is fully initialized `repr(C)` all-`f32`
                // storage; this byte view cannot outlive the local value.
                let material_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        &material_ubo as *const _ as *const u8,
                        std::mem::size_of::<MaterialUBO>(),
                    )
                };
                let (material_set, _) =
                    self.get_or_create_material_desc_set(&batch.material.id, material_bytes)?;
                self.bind_material_texture_if_changed(&batch.material.id, &material, material_set)?;
                // SAFETY: push range, compatible descriptors/pipeline, and all
                // generation-checked draw buffers are valid while `cmd` records.
                unsafe {
                    let particle_push = gpu_simulation
                        .map(engine_renderer::GpuParticleSimulation::parameter_bytes)
                        .unwrap_or(billboard_push);
                    d.cmd_push_constants(
                        cmd,
                        hdr_pll,
                        vk::ShaderStageFlags::VERTEX,
                        0,
                        &particle_push,
                    );
                    d.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        hdr_pll,
                        2,
                        &[material_set],
                        &[],
                    );
                    if gpu_simulation.is_some() {
                        d.cmd_bind_vertex_buffers(cmd, 0, &[vertex_buffer], &[0]);
                    } else {
                        let instance_buffer = particle_instance_buffer.ok_or_else(|| {
                            vec![Diagnostic::new(
                                "RV0343",
                                DiagnosticSeverity::Fatal,
                                "scene_renderer",
                                "CPU particle draw has no prepared instance buffer",
                            )]
                        })?;
                        d.cmd_bind_vertex_buffers(
                            cmd,
                            0,
                            &[vertex_buffer, instance_buffer],
                            &[0, 0],
                        );
                    }
                    d.cmd_bind_index_buffer(
                        cmd,
                        index_buffer,
                        0,
                        vulkan_index_type(mesh.index_format),
                    );
                    d.cmd_draw_indexed(
                        cmd,
                        mesh.index_count,
                        draw.instance_count,
                        0,
                        0,
                        draw.first_instance,
                    );
                }
                stats.draw_calls += 1;
                stats.triangles += u64::from(mesh.index_count / 3) * u64::from(draw.instance_count);
            }
        }
        Ok(())
    }
}
