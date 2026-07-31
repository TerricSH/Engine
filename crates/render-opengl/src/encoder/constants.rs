macro_rules! impl_encoder_constants {
    () => {
    fn push_constants(
        &mut self,
        layout: PipelineLayoutHandle,
        stage_flags: u32,
        offset: u32,
        data: &[u8],
    ) {
        if data.is_empty() {
            return;
        }
        if !offset.is_multiple_of(4) || !data.len().is_multiple_of(4) {
            self.record_error("push-constant offset and byte length must be four-byte aligned");
            return;
        }
        if stage_flags == 0 {
            self.record_error("push constants require at least one shader-stage flag");
            return;
        }
        if !self.validate_current_layout(layout) {
            return;
        }
        let Some(layout_descriptor) = self.resolve_layout(layout) else {
            self.record_error("push_constants received an invalid pipeline layout handle");
            return;
        };
        let Some(end) = offset.checked_add(data.len() as u32) else {
            self.record_error("push-constant range overflowed u32");
            return;
        };
        let covered = layout_descriptor.push_constant_ranges.iter().any(|range| {
            let Some(range_end) = range.offset.checked_add(range.size) else {
                return false;
            };
            offset >= range.offset
                && end <= range_end
                && (range.stage_flags & stage_flags) == stage_flags
        });
        if !covered {
            self.record_error(format!(
                "push-constant range {offset}..{end} for stages {stage_flags:#x} is not declared by the pipeline layout"
            ));
            return;
        }

        let Some(pipeline) = self.current_pipeline.clone() else {
            self.record_error("push constants require a bound pipeline");
            return;
        };
        if let Some(push_buffer) = pipeline.push_constant_buffer {
            if end > push_buffer.size_bytes {
                self.record_error(format!(
                    "push-constant range {offset}..{end} exceeds {}-byte OpenGL uniform buffer",
                    push_buffer.size_bytes
                ));
                return;
            }
            // SAFETY: buffer ownership and range bounds were validated above.
            unsafe {
                self.gl
                    .bind_buffer(glow::UNIFORM_BUFFER, Some(push_buffer.gl_buffer));
                self.gl
                    .buffer_sub_data_u8_slice(glow::UNIFORM_BUFFER, offset as i32, data);
                self.gl.bind_buffer_base(
                    glow::UNIFORM_BUFFER,
                    push_buffer.binding,
                    Some(push_buffer.gl_buffer),
                );
                self.gl.bind_buffer(glow::UNIFORM_BUFFER, None);
            }
            return;
        }

        let mut uploaded = 0usize;
        for uniform in &pipeline.push_uniforms {
            let uniform_end = uniform.offset.saturating_add(uniform.kind.size_bytes());
            if uniform.offset < offset || uniform_end > end {
                continue;
            }
            let start = (uniform.offset - offset) as usize;
            let finish = start + uniform.kind.size_bytes() as usize;
            if self.upload_push_uniform(uniform, &data[start..finish]) {
                uploaded += 1;
            }
        }
        if uploaded == 0 {
            self.record_error(
                "GLSL program has no push-constant UBO or compatible direct uniforms; use a `PushConstants`/`PC`/`DrawPush` std140 block or offset-named uniforms such as `u_pc_0`",
            );
        } else {
            tracing::trace!(
                target: "opengl",
                offset,
                bytes = data.len(),
                uniforms = uploaded,
                "mapped RHI push constants to GLSL uniforms"
            );
        }
    }

    fn end_render_pass(&mut self) {
        if self.current_render_pass.is_none() {
            self.record_error("end_render_pass called without an active render pass");
            return;
        }
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // binding None (the default framebuffer) is always valid regardless of
        // context state; the encoder is dropped before the device.
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        self.current_framebuffer = None;
        self.current_render_pass = None;
    }
    };
}

pub(super) use impl_encoder_constants;
