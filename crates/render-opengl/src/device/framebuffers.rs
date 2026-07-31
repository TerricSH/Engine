macro_rules! impl_device_framebuffers {
    () => {
    fn create_framebuffer(
        &mut self,
        descriptor: &FramebufferDescriptor,
    ) -> Result<FramebufferHandle, RhiError> {
        if descriptor.width == 0 || descriptor.height == 0 {
            return Err(invalid_descriptor(
                "framebuffer.extent",
                "width and height must both be non-zero",
            ));
        }
        let render_pass = &self
            .render_passes
            .get(descriptor.render_pass.index)
            .filter(|slot| slot.generation == descriptor.render_pass.generation)
            .ok_or(RhiError::InvalidHandle)?
            .value
            ._descriptor;
        if descriptor.color_attachments.len() != render_pass.color_attachments.len() {
            return Err(invalid_descriptor(
                "framebuffer.color_attachments",
                format!(
                    "render pass declares {} color attachments but framebuffer supplies {}",
                    render_pass.color_attachments.len(),
                    descriptor.color_attachments.len()
                ),
            ));
        }

        let mut color_textures = Vec::with_capacity(descriptor.color_attachments.len());
        for (index, (&handle, &expected_format)) in descriptor
            .color_attachments
            .iter()
            .zip(render_pass.color_attachments.iter())
            .enumerate()
        {
            let texture = &self
                .textures
                .get(handle.index)
                .filter(|slot| slot.generation == handle.generation)
                .ok_or(RhiError::InvalidHandle)?
                .value;
            if texture.usage.0 & TextureUsage::COLOR_ATTACHMENT.0 == 0 {
                return Err(invalid_descriptor(
                    "framebuffer.color_attachments",
                    format!("attachment {index} was not created with COLOR_ATTACHMENT usage"),
                ));
            }
            if texture.format != expected_format {
                return Err(invalid_descriptor(
                    "framebuffer.color_attachments",
                    format!(
                        "attachment {index} format {:?} does not match render-pass format {expected_format:?}",
                        texture.format
                    ),
                ));
            }
            if texture.width != descriptor.width || texture.height != descriptor.height {
                return Err(invalid_descriptor(
                    "framebuffer.extent",
                    format!(
                        "attachment {index} is {}x{}, expected {}x{}",
                        texture.width, texture.height, descriptor.width, descriptor.height
                    ),
                ));
            }
            color_textures.push(texture.gl_texture);
        }

        let depth_texture = match (
            descriptor.depth_stencil_attachment,
            render_pass.depth_stencil_format,
        ) {
            (Some(handle), Some(expected_format)) => {
                let texture = &self
                    .textures
                    .get(handle.index)
                    .filter(|slot| slot.generation == handle.generation)
                    .ok_or(RhiError::InvalidHandle)?
                    .value;
                if texture.usage.0 & TextureUsage::DEPTH_ATTACHMENT.0 == 0 {
                    return Err(invalid_descriptor(
                        "framebuffer.depth_stencil_attachment",
                        "depth attachment was not created with DEPTH_ATTACHMENT usage",
                    ));
                }
                if texture.format != expected_format {
                    return Err(invalid_descriptor(
                        "framebuffer.depth_stencil_attachment",
                        format!(
                            "depth format {:?} does not match render-pass format {expected_format:?}",
                            texture.format
                        ),
                    ));
                }
                if texture.width != descriptor.width || texture.height != descriptor.height {
                    return Err(invalid_descriptor(
                        "framebuffer.extent",
                        format!(
                            "depth attachment is {}x{}, expected {}x{}",
                            texture.width, texture.height, descriptor.width, descriptor.height
                        ),
                    ));
                }
                Some(texture.gl_texture)
            }
            (None, None) => None,
            (Some(_), None) => {
                return Err(invalid_descriptor(
                    "framebuffer.depth_stencil_attachment",
                    "framebuffer supplies depth but render pass does not declare it",
                ));
            }
            (None, Some(_)) => {
                return Err(invalid_descriptor(
                    "framebuffer.depth_stencil_attachment",
                    "render pass declares depth but framebuffer does not supply it",
                ));
            }
        };

        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the device is not yet destroyed; the returned framebuffer handle is
        // checked for errors before use.
        let gl_framebuffer = unsafe {
            self.gl
                .create_framebuffer()
                .map_err(|e| RhiError::Backend { detail: e })?
        };

        // SAFETY: `self.gl` is a valid `glow::Context`; `gl_framebuffer` was
        // just created by the same context, and the device is not yet destroyed.
        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(gl_framebuffer));
        }

        // Attach the fully validated texture set and reject incomplete FBOs.
        unsafe {
            let mut draw_buffers = Vec::with_capacity(color_textures.len());
            for (index, texture) in color_textures.iter().enumerate() {
                let attachment = glow::COLOR_ATTACHMENT0 + index as u32;
                self.gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    attachment,
                    glow::TEXTURE_2D,
                    Some(*texture),
                    0,
                );
                draw_buffers.push(attachment);
            }
            if draw_buffers.is_empty() {
                self.gl.draw_buffer(glow::NONE);
                self.gl.read_buffer(glow::NONE);
            } else {
                self.gl.draw_buffers(&draw_buffers);
            }
            if let Some(texture) = depth_texture {
                self.gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_ATTACHMENT,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
            }
            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                self.gl.delete_framebuffer(gl_framebuffer);
                return Err(RhiError::ValidationFailed {
                    detail: format!("OpenGL framebuffer is incomplete (status {status:#x})"),
                });
            }
        }

        let (idx, gen) = self.framebuffers.alloc(FramebufferSlot {
            gl_framebuffer,
            render_pass: descriptor.render_pass,
            _width: descriptor.width,
            _height: descriptor.height,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_framebuffer(&mut self, handle: FramebufferHandle) {
        let slot = self.framebuffers.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                // SAFETY: `self.gl` is a valid `glow::Context` created by this
                // device; `slot.value.gl_framebuffer` was created by the same
                // context and is not in use elsewhere (generation matched).
                unsafe { self.gl.delete_framebuffer(slot.value.gl_framebuffer) };
            }
        }
        self.framebuffers.free(handle.index);
    }

    // ██ pipeline layouts ██████████████████████████████████████████████████████████████████

    fn create_pipeline_layout(
        &mut self,
        descriptor: &PipelineLayoutDescriptor,
    ) -> Result<PipelineLayoutHandle, RhiError> {
        let max_push_bytes = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_UNIFORM_BLOCK_SIZE)
                .max(0) as u32
        };
        for range in &descriptor.push_constant_ranges {
            if range.size == 0 {
                return Err(invalid_descriptor(
                    "pipeline_layout.push_constant_ranges.size",
                    "push-constant ranges must not be empty",
                ));
            }
            if !range.offset.is_multiple_of(4) || !range.size.is_multiple_of(4) {
                return Err(invalid_descriptor(
                    "pipeline_layout.push_constant_ranges",
                    "push-constant offsets and sizes must be four-byte aligned",
                ));
            }
            if range.stage_flags == 0 {
                return Err(invalid_descriptor(
                    "pipeline_layout.push_constant_ranges.stage_flags",
                    "at least one shader stage must be selected",
                ));
            }
            let end = range.offset.checked_add(range.size).ok_or_else(|| {
                invalid_descriptor(
                    "pipeline_layout.push_constant_ranges",
                    "push-constant range overflowed u32",
                )
            })?;
            if end > max_push_bytes {
                return Err(RhiError::UnsupportedLimit {
                    limit: "OpenGL push-constant uniform block bytes".to_string(),
                    requested: end as u64,
                    available: max_push_bytes as u64,
                });
            }
        }

        let max_uniform_bindings = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_UNIFORM_BUFFER_BINDINGS)
                .max(0) as u32
        };
        let max_texture_units = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_COMBINED_TEXTURE_IMAGE_UNITS)
                .max(0) as u32
        };
        let mut sets = std::collections::BTreeSet::new();
        for set in &descriptor.bind_group_layouts {
            if !sets.insert(set.set_index) {
                return Err(invalid_descriptor(
                    "pipeline_layout.bind_group_layouts.set_index",
                    format!("descriptor set {} is duplicated", set.set_index),
                ));
            }
            let mut bindings = std::collections::BTreeSet::new();
            for binding in &set.bindings {
                if !bindings.insert(binding.binding) {
                    return Err(invalid_descriptor(
                        "pipeline_layout.bind_group_layouts.bindings",
                        format!(
                            "descriptor set {} repeats binding {}",
                            set.set_index, binding.binding
                        ),
                    ));
                }
                let Some(gl_binding) = gl_binding_point(set.set_index, binding.binding) else {
                    return Err(invalid_descriptor(
                        "pipeline_layout.bind_group_layouts.bindings",
                        "flattened OpenGL binding point overflowed u32",
                    ));
                };
                let kind = binding.resource_kind.trim().to_ascii_lowercase();
                match kind.as_str() {
                    "uniform_buffer" | "ubo" if gl_binding >= max_uniform_bindings => {
                        return Err(RhiError::UnsupportedLimit {
                            limit: "OpenGL uniform buffer binding point".to_string(),
                            requested: gl_binding as u64 + 1,
                            available: max_uniform_bindings as u64,
                        });
                    }
                    "sampled_texture" | "texture" | "sampled_image" | "combined_image_sampler"
                        if gl_binding >= max_texture_units =>
                    {
                        return Err(RhiError::UnsupportedLimit {
                            limit: "OpenGL texture unit".to_string(),
                            requested: gl_binding as u64 + 1,
                            available: max_texture_units as u64,
                        });
                    }
                    "sampled_texture_pair" | "texture_pair" | "sampled_image_pair"
                        if gl_binding
                            .checked_add(1)
                            .is_none_or(|second| second >= max_texture_units) =>
                    {
                        return Err(RhiError::UnsupportedLimit {
                            limit: "OpenGL texture units for sampled pair".to_string(),
                            requested: gl_binding as u64 + 2,
                            available: max_texture_units as u64,
                        });
                    }
                    "uniform_buffer"
                    | "ubo"
                    | "sampled_texture"
                    | "texture"
                    | "sampled_image"
                    | "combined_image_sampler"
                    | "sampled_texture_pair"
                    | "texture_pair"
                    | "sampled_image_pair"
                    | "sampler"
                    | "sampler_pair" => {}
                    _ => {
                        return Err(RhiError::IncompatibleBindLayout {
                            reason: format!(
                                "OpenGL does not support resource kind `{}`",
                                binding.resource_kind
                            ),
                        });
                    }
                }
            }
        }
        let (idx, gen) = self.pipeline_layouts.alloc(PipelineLayoutSlot {
            descriptor: descriptor.clone(),
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_pipeline_layout(&mut self, handle: PipelineLayoutHandle) {
        self.pipeline_layouts.free(handle.index);
    }

    // ██ pipelines █████████████████████████████████████████████████████████████████████████
    };
}

pub(super) use impl_device_framebuffers;
