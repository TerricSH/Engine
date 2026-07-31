macro_rules! impl_encoder_draw {
    () => {
    fn draw(
        &mut self,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        let Some(pipeline) = &self.current_pipeline else {
            self.record_error("draw requires a bound pipeline");
            return;
        };
        let Some(active_render_pass) = self.current_render_pass else {
            self.record_error("draw must be issued inside a render pass");
            return;
        };
        if pipeline
            .render_pass
            .is_some_and(|expected| expected != active_render_pass)
        {
            self.record_error("draw pipeline is incompatible with the active render pass");
            return;
        }
        if !pipeline.vertex_attributes.is_empty() && self.current_vertex_buffer.is_none() {
            self.record_error(
                "draw requires a bound vertex buffer for the pipeline's vertex layout",
            );
            return;
        }
        if vertex_count == 0 || instance_count == 0 {
            return;
        }
        if vertex_count > i32::MAX as u32
            || first_vertex > i32::MAX as u32
            || instance_count > i32::MAX as u32
        {
            self.record_error("draw counts and first_vertex must fit OpenGL signed integers");
            return;
        }
        if !pipeline.vertex_attributes.is_empty() {
            let Some((_, base_offset, buffer_size)) = self.current_vertex_buffer else {
                unreachable!("vertex-buffer presence checked above");
            };
            let Some(vertex_end) = u64::from(first_vertex)
                .checked_add(u64::from(vertex_count))
                .and_then(|vertices| vertices.checked_mul(u64::from(pipeline.vertex_stride)))
                .and_then(|bytes| base_offset.checked_add(bytes))
            else {
                self.record_error("vertex draw range overflowed u64");
                return;
            };
            if vertex_end > buffer_size {
                self.record_error(format!(
                    "vertex draw requires bytes through {vertex_end}, beyond {buffer_size}-byte buffer"
                ));
                return;
            }
        }
        let topology = pipeline.topology;
        if first_instance > 0 && !self.supports_base_instance() {
            self.record_error("non-zero first_instance requires desktop OpenGL 4.2 or newer");
            return;
        }
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the encoder is dropped before the device; all bound GL state was
        // set by this encoder in the same frame.
        unsafe {
            if first_instance > 0 {
                self.gl.draw_arrays_instanced_base_instance(
                    topology,
                    first_vertex as i32,
                    vertex_count as i32,
                    instance_count as i32,
                    first_instance,
                );
            } else if instance_count > 1 {
                self.gl.draw_arrays_instanced(
                    topology,
                    first_vertex as i32,
                    vertex_count as i32,
                    instance_count as i32,
                );
            } else {
                self.gl
                    .draw_arrays(topology, first_vertex as i32, vertex_count as i32);
            }
        }
        self.frame_state
            .record_draw(topology, vertex_count, instance_count);
    }

    fn draw_indexed(
        &mut self,
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        let Some(pipeline) = &self.current_pipeline else {
            self.record_error("draw_indexed requires a bound pipeline");
            return;
        };
        let Some(active_render_pass) = self.current_render_pass else {
            self.record_error("draw_indexed must be issued inside a render pass");
            return;
        };
        if pipeline
            .render_pass
            .is_some_and(|expected| expected != active_render_pass)
        {
            self.record_error("indexed draw pipeline is incompatible with the active render pass");
            return;
        }
        if !pipeline.vertex_attributes.is_empty() && self.current_vertex_buffer.is_none() {
            self.record_error(
                "draw_indexed requires a bound vertex buffer for the pipeline's vertex layout",
            );
            return;
        }
        if self.current_index_buffer.is_none() {
            self.record_error("draw_indexed requires a bound index buffer");
            return;
        }
        if index_count == 0 || instance_count == 0 {
            return;
        }
        if index_count > i32::MAX as u32 || instance_count > i32::MAX as u32 {
            self.record_error("indexed draw counts must fit OpenGL signed integers");
            return;
        }
        let (index_type, index_size) = match self.current_index_format {
            IndexFormat::U16 => (glow::UNSIGNED_SHORT, 2u64),
            IndexFormat::U32 => (glow::UNSIGNED_INT, 4u64),
        };
        let Some(offset_bytes) = u64::from(first_index)
            .checked_mul(index_size)
            .and_then(|relative| self.current_index_offset.checked_add(relative))
        else {
            self.record_error("indexed draw byte offset overflowed u64");
            return;
        };
        if offset_bytes > i32::MAX as u64 {
            self.record_error(format!(
                "indexed draw byte offset {offset_bytes} exceeds OpenGL i32 range"
            ));
            return;
        }
        let Some(index_end) = u64::from(index_count)
            .checked_mul(index_size)
            .and_then(|byte_count| offset_bytes.checked_add(byte_count))
        else {
            self.record_error("indexed draw range overflowed u64");
            return;
        };
        if index_end > self.current_index_buffer_size {
            self.record_error(format!(
                "indexed draw range {offset_bytes}..{index_end} exceeds {}-byte index buffer",
                self.current_index_buffer_size
            ));
            return;
        }
        let topology = pipeline.topology;
        if first_instance > 0 && !self.supports_base_instance() {
            self.record_error("non-zero first_instance requires desktop OpenGL 4.2 or newer");
            return;
        }
        if vertex_offset != 0 && self.gl.version().is_embedded {
            self.record_error("non-zero vertex_offset is not supported by OpenGL ES/WebGL");
            return;
        }
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the encoder is dropped before the device; all bound GL state was
        // set by this encoder in the same frame.
        unsafe {
            if first_instance > 0 {
                self.gl.draw_elements_instanced_base_vertex_base_instance(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                    instance_count as i32,
                    vertex_offset,
                    first_instance,
                );
            } else if instance_count > 1 && vertex_offset != 0 {
                self.gl.draw_elements_instanced_base_vertex(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                    instance_count as i32,
                    vertex_offset,
                );
            } else if instance_count > 1 {
                self.gl.draw_elements_instanced(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                    instance_count as i32,
                );
            } else if vertex_offset != 0 {
                self.gl.draw_elements_base_vertex(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                    vertex_offset,
                );
            } else {
                self.gl.draw_elements(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                );
            }
        }
        self.frame_state
            .record_draw(topology, index_count, instance_count);
    }
    };
}

pub(super) use impl_encoder_draw;
