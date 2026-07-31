use super::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12SceneRenderer {
    pub(super) fn directional_shadow_frame_data(
        input: &RenderFrameInput,
    ) -> Result<Option<Dx12ShadowFrameData>, Vec<Diagnostic>> {
        let Some(light) = input.lights.iter().find(|light| {
            light.kind == engine_renderer::LightKind::Directional
                && matches!(light.shadow_mode, ShadowMode::Hard | ShadowMode::Soft)
        }) else {
            return Ok(None);
        };
        let view = input.views.first().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1247",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional shadow rendering requires a camera view",
            )]
        })?;
        let light_direction = glam::Vec3::from_array(light.direction);
        let length_squared = light_direction.length_squared();
        if !light_direction.is_finite() || !length_squared.is_finite() || length_squared <= 1.0e-12
        {
            return Err(vec![Diagnostic::new(
                "DX1248",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional shadow light direction must be finite and non-zero",
            )
            .entity(light.entity.clone())]);
        }
        let light_direction = light_direction / length_squared.sqrt();
        let camera_view = Mat4::from_cols_array(&view.view_matrix);
        let camera_projection = Mat4::from_cols_array(&view.projection_matrix);
        let camera_view_projection = camera_projection * camera_view;
        let determinant = camera_view_projection.determinant();
        if !determinant.is_finite() || determinant.abs() <= 1.0e-12 {
            return Err(vec![Diagnostic::new(
                "DX1249",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "camera view-projection matrix is not invertible for shadow fitting",
            )]);
        }
        let inverse = camera_view_projection.inverse();
        let mut world_corners = [glam::Vec3::ZERO; 8];
        let mut corner_index = 0;
        for depth in [0.0_f32, 1.0] {
            for y in [-1.0_f32, 1.0] {
                for x in [-1.0_f32, 1.0] {
                    let homogeneous = inverse * glam::vec4(x, y, depth, 1.0);
                    if !homogeneous.is_finite() || homogeneous.w.abs() <= 1.0e-8 {
                        return Err(vec![Diagnostic::new(
                            "DX1250",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "camera frustum is degenerate and cannot be fitted to a shadow map",
                        )]);
                    }
                    let corner = homogeneous.truncate() / homogeneous.w;
                    if !corner.is_finite() {
                        return Err(vec![Diagnostic::new(
                            "DX1250",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "camera frustum contains non-finite shadow corners",
                        )]);
                    }
                    world_corners[corner_index] = corner;
                    corner_index += 1;
                }
            }
        }
        let center = world_corners.iter().copied().sum::<glam::Vec3>() / 8.0;
        let radius = world_corners
            .iter()
            .map(|corner| corner.distance(center))
            .fold(0.0_f32, f32::max);
        if !center.is_finite() || !radius.is_finite() || radius <= 1.0e-5 {
            return Err(vec![Diagnostic::new(
                "DX1250",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "camera frustum has no finite extent for shadow fitting",
            )]);
        }
        let up = if light_direction.dot(glam::Vec3::Y).abs() > 0.99 {
            glam::Vec3::Z
        } else {
            glam::Vec3::Y
        };
        let light_position = center - light_direction * (radius * 2.0 + 1.0);
        let light_view = Mat4::look_at_rh(light_position, center, up);
        let mut minimum = glam::Vec3::splat(f32::MAX);
        let mut maximum = glam::Vec3::splat(f32::MIN);
        for corner in world_corners {
            let light_space = (light_view * corner.extend(1.0)).truncate();
            minimum = minimum.min(light_space);
            maximum = maximum.max(light_space);
        }
        let extent = maximum - minimum;
        if !extent.is_finite() || extent.min_element() <= 1.0e-5 {
            return Err(vec![Diagnostic::new(
                "DX1250",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional-light shadow bounds are degenerate",
            )]);
        }
        let pad_x = (extent.x * 0.025).max(1.0e-3);
        let pad_y = (extent.y * 0.025).max(1.0e-3);
        let pad_z = (extent.z * 0.025).max(1.0e-3);
        let near = (-maximum.z - pad_z).max(1.0e-4);
        let far = (-minimum.z + pad_z).max(near + 1.0e-3);
        let light_projection = Mat4::orthographic_rh(
            minimum.x - pad_x,
            maximum.x + pad_x,
            minimum.y - pad_y,
            maximum.y + pad_y,
            near,
            far,
        );
        Ok(Some(Dx12ShadowFrameData {
            light_view_projection: light_projection * light_view,
            light_direction_to_surface: -light_direction,
            soft: light.shadow_mode == ShadowMode::Soft,
        }))
    }

    pub(super) fn record_directional_shadow_pass(
        &mut self,
        input: &RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        let Some(shadow_data) = Self::directional_shadow_frame_data(input)? else {
            self.shadow_frame_data = None;
            return Ok(());
        };
        let missing_meshes: Vec<&str> = input
            .drawables
            .iter()
            .filter(|drawable| drawable.cast_shadows)
            .map(|drawable| drawable.mesh.id.as_str())
            .chain(
                input
                    .skinned_items
                    .iter()
                    .filter(|item| item.cast_shadows)
                    .map(|item| item.mesh.id.as_str()),
            )
            .filter(|mesh_id| !self.meshes.contains_key(*mesh_id))
            .collect();
        if !missing_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1251",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "shadow casters reference meshes that were not uploaded: {}",
                    missing_meshes.join(", ")
                ),
            )]);
        }
        let render_pass = self.shadow_render_pass.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 directional shadow render pass is unavailable",
            )]
        })?;
        let framebuffer = self.shadow_framebuffer.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 directional shadow framebuffer is unavailable",
            )]
        })?;
        let layout = self.shadow_pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 directional shadow root signature is unavailable",
            )]
        })?;
        let static_pipeline = self.shadow_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 directional shadow pipeline is unavailable",
            )]
        })?;
        let skinned_pipeline = self.skinned_shadow_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 skinned directional shadow pipeline is unavailable",
            )]
        })?;
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "shadow pass called without an active DX12 frame",
            )]
        })?;
        frame.encoder.begin_render_pass(
            render_pass,
            framebuffer,
            (0, 0, 2048, 2048),
            [0.0; 4],
            Some(1.0),
        );
        frame.encoder.bind_pipeline(static_pipeline);
        for (drawable_index, drawable) in input
            .drawables
            .iter()
            .enumerate()
            .filter(|(_, drawable)| drawable.cast_shadows)
        {
            if matches!(
                self.material_surface(input, &drawable.material).0,
                Transparency::Blend | Transparency::Additive
            ) {
                continue;
            }
            let (vertex_buffer, index_buffer, index_format, index_count) = {
                let mesh = &self.meshes[&drawable.mesh.id];
                (
                    mesh.vertex_buffer,
                    mesh.index_buffer,
                    mesh.index_format,
                    mesh.index_count,
                )
            };
            let world = Mat4::from_cols_array(&drawable.world_transform);
            let matrix = shadow_data.light_view_projection * world;
            let matrix_bytes = matrix_bytes(matrix);
            frame.encoder.push_constants(layout, 0x10, 0, &matrix_bytes);
            let Some((vertex_draw_buffer, vertex_draw_offset)) =
                self.vertex_draw_binding(drawable_index)
            else {
                self.active_frame = Some(frame);
                return Err(vec![Diagnostic::new(
                    "DX1286",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 shadow pass has no aligned vertex-draw arena binding",
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
                    "DX12 shadow pass could not bind its vertex-draw arena offset",
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
        frame.encoder.bind_pipeline(skinned_pipeline);
        for (item_index, item) in input
            .skinned_items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.cast_shadows)
        {
            if matches!(
                self.material_surface(input, &item.material).0,
                Transparency::Blend | Transparency::Additive
            ) {
                continue;
            }
            let mesh = self.meshes[&item.mesh.id].clone();
            let world = Mat4::from_cols_array(&item.world_transform);
            let matrix = shadow_data.light_view_projection * world;
            let matrix_bytes = matrix_bytes(matrix);
            frame.encoder.push_constants(layout, 0x10, 0, &matrix_bytes);
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
            let bone_buffer = match self.prepare_bone_buffer(&cache_key, &item.bone_palette) {
                Ok(buffer) => buffer,
                Err(diagnostics) => {
                    self.active_frame = Some(frame);
                    return Err(diagnostics);
                }
            };
            if !frame.encoder.bind_uniform_buffer(layout, bone_buffer) {
                self.active_frame = Some(frame);
                return Err(vec![Diagnostic::new(
                    "DX1252",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 skinned shadow pass could not bind its bone palette",
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
        frame.encoder.end_render_pass();
        self.active_frame = Some(frame);
        self.shadow_frame_data = Some(shadow_data);
        Ok(())
    }
}
