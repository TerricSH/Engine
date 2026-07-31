macro_rules! impl_encoder_bindings {
    () => {
        fn bind_vertex_buffers(&mut self, buffers: &[BufferHandle], offsets: &[u64]) {
            if buffers.len() != offsets.len() {
                self.record_error("vertex buffer and offset counts do not match");
                return;
            }
            if buffers.len() > 1 {
                self.record_error(
                    "OpenGL RHI vertex layouts currently describe one interleaved vertex buffer",
                );
                return;
            }
            let Some((&handle, &offset)) = buffers.first().zip(offsets.first()) else {
                self.current_vertex_buffer = None;
                return;
            };
            let Some(slot) = self.buffer_slot(handle) else {
                self.record_error("bind_vertex_buffers received an invalid buffer handle");
                return;
            };
            if slot.usage.0 & BufferUsage::VERTEX.0 == 0 {
                self.record_error("bind_vertex_buffers received a buffer without VERTEX usage");
                return;
            }
            if offset >= slot.size_bytes && slot.size_bytes != 0 {
                self.record_error(format!(
                    "vertex buffer offset {offset} is outside {}-byte buffer",
                    slot.size_bytes
                ));
                return;
            }
            let gl_buffer = slot.gl_buffer;
            let buffer_size = slot.size_bytes;
            self.current_vertex_buffer = Some((gl_buffer, offset, buffer_size));
            if let Some(pipeline) = &self.current_pipeline {
                self.configure_vertex_input(pipeline, gl_buffer, offset);
            }
        }

        fn bind_index_buffer(
            &mut self,
            buffer: BufferHandle,
            offset: u64,
            index_format: IndexFormat,
        ) {
            let Some(slot) = self.buffer_slot(buffer) else {
                self.record_error("bind_index_buffer received an invalid buffer handle");
                return;
            };
            if slot.usage.0 & BufferUsage::INDEX.0 == 0 {
                self.record_error("bind_index_buffer received a buffer without INDEX usage");
                return;
            }
            let alignment = match index_format {
                IndexFormat::U16 => 2,
                IndexFormat::U32 => 4,
            };
            if !offset.is_multiple_of(alignment) || offset >= slot.size_bytes {
                self.record_error(format!(
                    "index buffer offset {offset} is invalid for {:?} in a {}-byte buffer",
                    index_format, slot.size_bytes
                ));
                return;
            }
            let gl_buffer = slot.gl_buffer;
            let buffer_size = slot.size_bytes;
            if let Some(pipeline) = &self.current_pipeline {
                // ELEMENT_ARRAY_BUFFER binding is VAO state.
                unsafe {
                    self.gl.bind_vertex_array(Some(pipeline.gl_vertex_array));
                    self.gl
                        .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(gl_buffer));
                }
            }
            self.current_index_buffer = Some(gl_buffer);
            self.current_index_buffer_size = buffer_size;
            self.current_index_offset = offset;
            self.current_index_format = index_format;
        }

        fn bind_descriptor_sets(
            &mut self,
            _pipeline_layout: PipelineLayoutHandle,
            _first_set: u32,
            sets: &[DescriptorSetHandle],
            dynamic_offsets: &[u32],
        ) -> Result<(), RhiError> {
            if sets.is_empty() {
                if dynamic_offsets.is_empty() {
                    return Ok(());
                }
                return Err(RhiError::InvalidDescriptor {
                    field: "descriptor_sets.dynamic_offsets".to_string(),
                    reason: "dynamic offsets were supplied without descriptor sets".to_string(),
                });
            }
            Err(RhiError::UnsupportedFeature {
                feature: "OpenGL descriptor-set allocation; use the typed texture/uniform bridges"
                    .to_string(),
            })
        }

        fn bind_sampled_texture(
            &mut self,
            pipeline_layout: PipelineLayoutHandle,
            texture: TextureHandle,
        ) -> bool {
            if !self.validate_current_layout(pipeline_layout) {
                return false;
            }
            let Some(unit) = self.resource_binding(
                pipeline_layout,
                &[
                    "sampled_texture",
                    "texture",
                    "sampled_image",
                    "combined_image_sampler",
                ],
            ) else {
                self.record_error("pipeline layout does not declare a sampled-texture binding");
                return false;
            };
            self.bind_texture_unit(texture, unit, 0)
        }

        fn bind_sampled_texture_pair(
            &mut self,
            pipeline_layout: PipelineLayoutHandle,
            base_color: TextureHandle,
            shadow_map: TextureHandle,
        ) -> bool {
            if !self.validate_current_layout(pipeline_layout) {
                return false;
            }
            let Some(first_unit) = self.resource_binding(
                pipeline_layout,
                &["sampled_texture_pair", "texture_pair", "sampled_image_pair"],
            ) else {
                self.record_error(
                    "pipeline layout does not declare a sampled-texture-pair binding",
                );
                return false;
            };
            let Some(shadow_unit) = first_unit.checked_add(1) else {
                self.record_error("sampled-texture-pair unit overflowed u32");
                return false;
            };
            // Resolve both handles before changing GL state so this bridge is
            // fail-closed rather than leaving a half-updated texture pair.
            if self.texture_slot(base_color).is_none() || self.texture_slot(shadow_map).is_none() {
                self.record_error(
                    "sampled-texture-pair binding received an invalid texture handle",
                );
                return false;
            }
            self.bind_texture_unit(base_color, first_unit, 0)
                && self.bind_texture_unit(shadow_map, shadow_unit, 1)
        }

        fn bind_uniform_buffer(
            &mut self,
            pipeline_layout: PipelineLayoutHandle,
            buffer: BufferHandle,
        ) -> bool {
            if !self.validate_current_layout(pipeline_layout) {
                return false;
            }
            let Some(binding) = self.resource_binding(pipeline_layout, &["uniform_buffer", "ubo"])
            else {
                self.record_error("pipeline layout does not declare a uniform-buffer binding");
                return false;
            };
            let Some(buffer) = self.buffer_slot(buffer) else {
                self.record_error("uniform-buffer binding received an invalid buffer handle");
                return false;
            };
            if buffer.usage.0 & BufferUsage::UNIFORM.0 == 0 {
                self.record_error("uniform-buffer binding received a buffer without UNIFORM usage");
                return false;
            }
            let max_bindings = unsafe {
                self.gl
                    .get_parameter_i32(glow::MAX_UNIFORM_BUFFER_BINDINGS)
                    .max(0) as u32
            };
            if binding >= max_bindings {
                self.record_error(format!(
                    "uniform-buffer binding {binding} exceeds OpenGL limit {max_bindings}"
                ));
                return false;
            }
            unsafe {
                self.gl
                    .bind_buffer_base(glow::UNIFORM_BUFFER, binding, Some(buffer.gl_buffer));
            }
            true
        }

        fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, min_depth: f32, max_depth: f32) {
            if ![x, y, w, h, min_depth, max_depth]
                .iter()
                .all(|value| value.is_finite())
                || w <= 0.0
                || h <= 0.0
                || x < i32::MIN as f32
                || x > i32::MAX as f32
                || y < i32::MIN as f32
                || y > i32::MAX as f32
                || w > i32::MAX as f32
                || h > i32::MAX as f32
                || !(0.0..=1.0).contains(&min_depth)
                || !(0.0..=1.0).contains(&max_depth)
                || min_depth > max_depth
            {
                self.record_error("viewport dimensions/depth range are invalid for OpenGL");
                return;
            }
            // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
            // the encoder is dropped before the device.
            unsafe {
                self.gl.viewport(x as i32, y as i32, w as i32, h as i32);
                self.gl.depth_range_f32(min_depth, max_depth);
            }
        }

        fn set_scissor(&mut self, x: i32, y: i32, w: u32, h: u32) {
            if w == 0 || h == 0 || w > i32::MAX as u32 || h > i32::MAX as u32 {
                self.record_error(
                    "scissor width and height must be non-zero OpenGL signed integers",
                );
                return;
            }
            // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
            // the encoder is dropped before the device.
            unsafe {
                self.gl.enable(glow::SCISSOR_TEST);
                self.gl.scissor(x, y, w as i32, h as i32);
            }
        }
    };
}

pub(super) use impl_encoder_bindings;
