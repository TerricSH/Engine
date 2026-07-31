macro_rules! impl_device_resources {
    () => {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.adapter
    }

    // Surface presentation stays fail-closed until construction accepts a
    // platform buffer-swap callback.

    fn create_surface(
        &mut self,
        _descriptor: &SurfaceDescriptor,
    ) -> Result<SurfaceHandle, RhiError> {
        Err(opengl_presentation_unsupported())
    }

    fn create_swapchain(
        &mut self,
        _descriptor: &SwapchainDescriptor,
    ) -> Result<SwapchainHandle, RhiError> {
        Err(opengl_presentation_unsupported())
    }

    fn destroy_surface(&mut self, handle: SurfaceHandle) {
        tracing::warn!(
            target: "opengl",
            ?handle,
            "ignored surface destruction: this backend cannot create OpenGL presentation surfaces"
        );
    }

    fn destroy_swapchain(&mut self, handle: SwapchainHandle) {
        tracing::warn!(
            target: "opengl",
            ?handle,
            "ignored swapchain destruction: this backend cannot create OpenGL presentation swapchains"
        );
    }

    // ██ buffers ████████████████████████████████████████████████████████████████████████████████

    fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> Result<BufferHandle, RhiError> {
        if descriptor.size_bytes == 0 || descriptor.size_bytes > i32::MAX as u64 {
            return Err(invalid_descriptor(
                "buffer.size_bytes",
                format!(
                    "OpenGL buffer size must be in 1..={}, received {}",
                    i32::MAX,
                    descriptor.size_bytes
                ),
            ));
        }
        // SAFETY: glow buffer creation.
        let gl_buffer = unsafe {
            self.gl
                .create_buffer()
                .map_err(|e| RhiError::Backend { detail: e })?
        };
        let target = buffer_target(descriptor.usage_flags);
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // `gl_buffer` was just created by the same context, and the device is
        // not yet destroyed.
        unsafe {
            self.gl.bind_buffer(target, Some(gl_buffer));
            self.gl
                .buffer_data_size(target, descriptor.size_bytes as i32, glow::STATIC_DRAW);
        }

        let (idx, gen) = self.buffers.alloc(BufferSlot {
            gl_buffer,
            size_bytes: descriptor.size_bytes,
            usage: descriptor.usage_flags,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn write_buffer(
        &mut self,
        buffer: BufferHandle,
        data: &[u8],
        offset: u64,
    ) -> Result<(), RhiError> {
        let slot = self
            .buffers
            .get(buffer.index)
            .filter(|s| s.generation == buffer.generation)
            .ok_or(RhiError::InvalidHandle)?;
        let end = offset.checked_add(data.len() as u64).ok_or_else(|| {
            invalid_descriptor("buffer.write", "offset plus data length overflowed")
        })?;
        if end > slot.value.size_bytes {
            return Err(invalid_descriptor(
                "buffer.write",
                format!(
                    "write range {offset}..{end} exceeds buffer size {}",
                    slot.value.size_bytes
                ),
            ));
        }
        let target = buffer_target(slot.value.usage);
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // `slot.value.gl_buffer` was created by the same context, the slot was
        // validated by generation check above, and the device is not yet destroyed.
        unsafe {
            self.gl.bind_buffer(target, Some(slot.value.gl_buffer));
            self.gl
                .buffer_sub_data_u8_slice(target, offset as i32, data);
        }
        Ok(())
    }

    fn destroy_buffer(&mut self, handle: BufferHandle) {
        let slot = self.buffers.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                // SAFETY: `self.gl` is a valid `glow::Context` created by this
                // device; `slot.value.gl_buffer` was created by the same context
                // and is not in use elsewhere (the handle generation matched).
                unsafe { self.gl.delete_buffer(slot.value.gl_buffer) };
            }
        }
        self.buffers.free(handle.index);
    }

    // ██ textures ███████████████████████████████████████████████████████████████████████████████

    fn create_texture(
        &mut self,
        descriptor: &TextureDescriptor,
    ) -> Result<TextureHandle, RhiError> {
        if descriptor.width == 0 || descriptor.height == 0 {
            return Err(invalid_descriptor(
                "texture.extent",
                "width and height must both be non-zero",
            ));
        }
        if descriptor.width > i32::MAX as u32 || descriptor.height > i32::MAX as u32 {
            return Err(invalid_descriptor(
                "texture.extent",
                "width and height must fit the OpenGL signed integer API",
            ));
        }
        if descriptor.depth_or_layers != 1 {
            return Err(RhiError::UnsupportedFeature {
                feature: "OpenGL array/3D textures through TextureDescriptor".to_string(),
            });
        }
        if descriptor.mip_levels != 1 {
            return Err(RhiError::UnsupportedFeature {
                feature: "OpenGL mipmapped texture allocation through TextureDescriptor"
                    .to_string(),
            });
        }
        if descriptor.sample_count != 1 {
            return Err(RhiError::UnsupportedFeature {
                feature: "OpenGL multisampled texture allocation through TextureDescriptor"
                    .to_string(),
            });
        }
        let (internal_fmt, fmt, pixel_type) = convert_texture_format(descriptor.format)?;
        // SAFETY: glow texture creation.
        let gl_texture = unsafe {
            self.gl
                .create_texture()
                .map_err(|e| RhiError::Backend { detail: e })?
        };
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // `gl_texture` was just created by the same context, and the device is
        // not yet destroyed.
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(gl_texture));
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal_fmt,
                descriptor.width as i32,
                descriptor.height as i32,
                0,
                fmt,
                pixel_type,
                glow::PixelUnpackData::Slice(None),
            );
        }

        let (idx, gen) = self.textures.alloc(TextureSlot {
            gl_texture,
            format: descriptor.format,
            width: descriptor.width,
            height: descriptor.height,
            usage: descriptor.usage_flags,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_texture(&mut self, handle: TextureHandle) {
        let slot = self.textures.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                // SAFETY: `self.gl` is a valid `glow::Context` created by this
                // device; `slot.value.gl_texture` was created by the same context
                // and is not in use elsewhere (the handle generation matched).
                unsafe { self.gl.delete_texture(slot.value.gl_texture) };
            }
        }
        self.textures.free(handle.index);
    }

    // ██ shader modules ██████████████████████████████████████████████████████████████████████

    fn create_shader_module(
        &mut self,
        descriptor: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, RhiError> {
        decode_glsl_source(descriptor)?;
        let (idx, gen) = self.shader_modules.alloc(ShaderModuleSlot {
            format: descriptor.format,
            stage: descriptor.stage,
            source_bytes: descriptor.source_bytes.clone(),
            entry_point: descriptor.entry_points[0].clone(),
            source_hash: descriptor.source_hash,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_shader_module(&mut self, handle: ShaderModuleHandle) {
        self.shader_modules.free(handle.index);
    }

    // ██ render passes ██████████████████████████████████████████████████████████████████████

    fn create_render_pass(
        &mut self,
        descriptor: &RenderPassDescriptor,
    ) -> Result<RenderPassHandle, RhiError> {
        if descriptor.sample_count != 1 {
            return Err(RhiError::UnsupportedFeature {
                feature: "OpenGL multisampled render passes through the generic RHI".to_string(),
            });
        }
        if descriptor.color_attachments.len() > u8::MAX as usize {
            return Err(invalid_descriptor(
                "render_pass.color_attachments",
                "attachment count exceeds the portable RHI limit",
            ));
        }
        let max_color_attachments = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_COLOR_ATTACHMENTS)
                .max(0) as usize
        };
        if descriptor.color_attachments.len() > max_color_attachments {
            return Err(RhiError::UnsupportedLimit {
                limit: "OpenGL color attachments".to_string(),
                requested: descriptor.color_attachments.len() as u64,
                available: max_color_attachments as u64,
            });
        }
        let (idx, gen) = self.render_passes.alloc(RenderPassSlot {
            _descriptor: descriptor.clone(),
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_render_pass(&mut self, handle: RenderPassHandle) {
        self.render_passes.free(handle.index);
    }

    // ██ framebuffers ███████████████████████████████████████████████████████████████████████
    };
}

pub(super) use impl_device_resources;
