use super::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12SceneRenderer {
    pub(super) fn record_tone_map_pass(
        &mut self,
        input: &RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        let layout = self.tone_map_pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 tone-map root signature is unavailable",
            )]
        })?;
        let pipeline = self.tone_map_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 tone-map pipeline is unavailable",
            )]
        })?;
        let hdr_texture = self.hdr_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 HDR color target is unavailable",
            )]
        })?;
        let oit_accum = self.oit_accum_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 OIT accumulation target is unavailable",
            )]
        })?;
        let oit_optical_depth = self.oit_optical_depth_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 OIT optical-depth target is unavailable",
            )]
        })?;
        let constants = tone_map_constants(input)?;
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map pass called without an active DX12 frame",
            )]
        })?;
        frame.encoder.end_render_pass();
        frame.encoder.bind_pipeline(pipeline);
        frame
            .encoder
            .set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        frame.encoder.set_scissor(0, 0, self.width, self.height);
        frame.encoder.push_constants(layout, 0x20, 0, &constants);
        if !frame
            .encoder
            .bind_sampled_texture_set(layout, &[hdr_texture, oit_accum, oit_optical_depth])
        {
            self.active_frame = Some(frame);
            return Err(vec![Diagnostic::new(
                "DX1257",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 tone-map pass could not bind the HDR/OIT target set",
            )]);
        }
        frame.encoder.draw(3, 1, 0, 0);
        frame.draw_calls += 1;
        frame.triangles += 1;
        self.active_frame = Some(frame);
        Ok(())
    }

    pub(super) fn record_ui_overlay_pass(
        &mut self,
        input: &RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        if input.ui_batches.is_empty() {
            return Ok(());
        }
        let prepared =
            prepare_dx12_ui(&input.ui_batches, self.width, self.height).map_err(|message| {
                vec![Diagnostic::new(
                    "DX1275",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    message,
                )]
            })?;
        if prepared.draws.is_empty() {
            return Ok(());
        }
        for draw in &prepared.draws {
            if let Some(texture_id) = draw.texture_id.as_ref() {
                if !self.textures.contains_key(texture_id) {
                    return Err(vec![Diagnostic::new(
                        "DX1276",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "UI batch references texture '{texture_id}' before a successful DX12 upload"
                        ),
                    )]);
                }
            }
        }
        let required = prepared.vertex_bytes.len();
        if self
            .ui_vertex_buffer
            .as_ref()
            .is_none_or(|buffer| buffer.capacity < required)
        {
            if let Some(old) = self.ui_vertex_buffer.take() {
                self.device.destroy_buffer(old.handle);
            }
            let capacity = required.next_power_of_two();
            let handle = self
                .device
                .create_buffer(&BufferDescriptor {
                    size_bytes: capacity as u64,
                    usage_flags: render_core::BufferUsage::VERTEX,
                    memory_hint: MemoryHint::CpuToGpu,
                    debug_label: Some("dx12-ui-overlay".into()),
                })
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "DX1277",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("create DX12 UI vertex stream failed: {error:?}"),
                    )]
                })?;
            if let Err(error) = self.device.set_vertex_stride(handle, 32) {
                self.device.destroy_buffer(handle);
                return Err(vec![Diagnostic::new(
                    "DX1278",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("set DX12 UI vertex stride failed: {error:?}"),
                )]);
            }
            self.ui_vertex_buffer = Some(Dx12DynamicVertexBuffer {
                handle,
                bytes: Vec::new(),
                capacity,
            });
        }
        let vertex_buffer = self
            .ui_vertex_buffer
            .as_mut()
            .expect("UI vertex buffer was created above");
        self.device
            .write_buffer(vertex_buffer.handle, &prepared.vertex_bytes, 0)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1279",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("write DX12 UI vertex stream failed: {error:?}"),
                )]
            })?;
        vertex_buffer.bytes = prepared.vertex_bytes;
        let vertex_handle = vertex_buffer.handle;
        let layout = self.ui_pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1280",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 UI root signature is unavailable",
            )]
        })?;
        let pipeline = self.ui_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1280",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 UI pipeline is unavailable",
            )]
        })?;
        let fallback = self.fallback_ui_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1280",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 UI fallback texture is unavailable",
            )]
        })?;
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "UI overlay pass called without an active DX12 frame",
            )]
        })?;
        frame.encoder.bind_pipeline(pipeline);
        frame
            .encoder
            .set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        let mut screen_size = [0_u8; 8];
        screen_size[..4].copy_from_slice(&(self.width as f32).to_ne_bytes());
        screen_size[4..].copy_from_slice(&(self.height as f32).to_ne_bytes());
        frame.encoder.push_constants(layout, 0x10, 0, &screen_size);
        frame.encoder.bind_vertex_buffers(&[vertex_handle], &[0]);
        for draw in prepared.draws {
            frame.encoder.set_scissor(
                draw.scissor.x,
                draw.scissor.y,
                draw.scissor.width,
                draw.scissor.height,
            );
            let texture = draw
                .texture_id
                .as_ref()
                .and_then(|id| self.textures.get(id))
                .map_or(fallback, |texture| texture.handle);
            if !frame.encoder.bind_sampled_texture(layout, texture) {
                self.active_frame = Some(frame);
                return Err(vec![Diagnostic::new(
                    "DX1281",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 UI pass could not bind its texture",
                )]);
            }
            frame
                .encoder
                .draw(draw.vertex_count, 1, draw.first_vertex, 0);
            frame.draw_calls += 1;
            frame.triangles += u64::from(draw.vertex_count / 3);
        }
        self.active_frame = Some(frame);
        Ok(())
    }
}
