macro_rules! impl_device_pipelines {
    () => {
    fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
    ) -> Result<PipelineHandle, RhiError> {
        if descriptor.shader_modules.is_empty() {
            return Err(invalid_descriptor(
                "pipeline.shader_modules",
                "at least one vertex shader module is required",
            ));
        }
        if !descriptor.specialization.is_empty() {
            return Err(RhiError::UnsupportedFeature {
                feature: "GLSL specialization constants".to_string(),
            });
        }

        let vertex_attributes = parse_vertex_layout(&descriptor.vertex_layout)?;
        let topology = parse_topology(descriptor.topology.as_deref())?;
        let raster_state = parse_raster_state(descriptor)?;
        if self.gl.version().is_embedded && raster_state.polygon_mode != glow::FILL {
            return Err(RhiError::UnsupportedFeature {
                feature: "non-fill polygon mode on OpenGL ES/WebGL".to_string(),
            });
        }
        let depth_state = parse_depth_state(descriptor)?;
        let blend_state = parse_blend_state(descriptor)?;
        let sample_count = descriptor.sample_count.unwrap_or(1);
        if sample_count == 0 {
            return Err(invalid_descriptor(
                "pipeline.sample_count",
                "sample count must be at least one",
            ));
        }
        if let Some(render_pass_handle) = descriptor.render_pass {
            let render_pass = &self
                .render_passes
                .get(render_pass_handle.index)
                .filter(|slot| slot.generation == render_pass_handle.generation)
                .ok_or(RhiError::InvalidHandle)?
                .value
                ._descriptor;
            if descriptor.render_targets != render_pass.color_attachments {
                return Err(invalid_descriptor(
                    "pipeline.render_targets",
                    "pipeline color formats do not match the referenced render pass",
                ));
            }
            if sample_count != render_pass.sample_count {
                return Err(invalid_descriptor(
                    "pipeline.sample_count",
                    format!(
                        "pipeline requests {sample_count} samples but render pass requests {}",
                        render_pass.sample_count
                    ),
                ));
            }
            if descriptor.depth_state.format != render_pass.depth_stencil_format {
                return Err(invalid_descriptor(
                    "pipeline.depth_state.format",
                    "pipeline depth format does not match the referenced render pass",
                ));
            }
        }

        let layout_descriptor = match descriptor.pipeline_layout {
            Some(handle) => Some(
                self.pipeline_layouts
                    .get(handle.index)
                    .filter(|slot| slot.generation == handle.generation)
                    .ok_or(RhiError::InvalidHandle)?
                    .value
                    .descriptor
                    .clone(),
            ),
            None => None,
        };
        if descriptor.pipeline_layout.is_none() && !descriptor.bind_layouts.is_empty() {
            return Err(invalid_descriptor(
                "pipeline.pipeline_layout",
                "resource bind layouts require an explicit pipeline layout handle on OpenGL",
            ));
        }
        if let Some(layout) = &layout_descriptor {
            if !descriptor.bind_layouts.is_empty()
                && descriptor.bind_layouts != layout.bind_group_layouts
            {
                return Err(RhiError::IncompatibleBindLayout {
                    reason: "pipeline bind_layouts differ from the referenced pipeline layout"
                        .to_string(),
                });
            }
        }

        #[derive(Debug)]
        struct ModuleSource {
            stage: ShaderStage,
            source: String,
            label: String,
        }
        let mut modules = Vec::with_capacity(descriptor.shader_modules.len());
        let mut vertex_count = 0u32;
        let mut fragment_count = 0u32;
        for handle in &descriptor.shader_modules {
            let module = &self
                .shader_modules
                .get(handle.index)
                .filter(|slot| slot.generation == handle.generation)
                .ok_or(RhiError::InvalidHandle)?
                .value;
            if module.format != ShaderFormat::Glsl || module.entry_point != "main" {
                return Err(RhiError::ValidationFailed {
                    detail: "corrupt OpenGL shader module metadata".to_string(),
                });
            }
            shader_stage_to_gl(module.stage)?;
            match module.stage {
                ShaderStage::Vertex => vertex_count += 1,
                ShaderStage::Fragment => fragment_count += 1,
                ShaderStage::Compute => unreachable!("compute stage rejected above"),
            }
            let source = std::str::from_utf8(&module.source_bytes)
                .map_err(|error| RhiError::ValidationFailed {
                    detail: format!("stored GLSL source became invalid UTF-8: {error}"),
                })?
                .to_owned();
            let hash_prefix = module
                .source_hash
                .iter()
                .take(4)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            modules.push(ModuleSource {
                stage: module.stage,
                source,
                label: format!("{:?}@{hash_prefix}", module.stage),
            });
        }
        if vertex_count != 1
            || fragment_count > 1
            || (!descriptor.render_targets.is_empty() && fragment_count != 1)
        {
            return Err(invalid_descriptor(
                "pipeline.shader_modules",
                format!(
                    "graphics pipelines require exactly one vertex shader and, for color output, exactly one fragment shader; got {vertex_count} vertex and {fragment_count} fragment modules"
                ),
            ));
        }

        // SAFETY: the GL context is current and all handles created below are
        // either transferred into the resource slab or deleted on every error
        // path before this function returns.
        let gl_program = unsafe {
            self.gl
                .create_program()
                .map_err(|detail| RhiError::Backend { detail })?
        };
        let mut attached = Vec::<glow::Shader>::new();
        let cleanup_program = |attached: &[glow::Shader]| {
            // SAFETY: every shader in `attached` is attached to `gl_program`
            // and all objects were created by this context.
            unsafe {
                for shader in attached {
                    self.gl.detach_shader(gl_program, *shader);
                    self.gl.delete_shader(*shader);
                }
                self.gl.delete_program(gl_program);
            }
        };

        for module in &modules {
            let gl_shader =
                match unsafe { self.gl.create_shader(shader_stage_to_gl(module.stage)?) } {
                    Ok(shader) => shader,
                    Err(detail) => {
                        cleanup_program(&attached);
                        return Err(RhiError::Backend { detail });
                    }
                };
            // SAFETY: `gl_shader` is live and belongs to this current context.
            unsafe {
                self.gl.shader_source(gl_shader, &module.source);
                self.gl.compile_shader(gl_shader);
                if !self.gl.get_shader_compile_status(gl_shader) {
                    let log = self.gl.get_shader_info_log(gl_shader);
                    self.gl.delete_shader(gl_shader);
                    cleanup_program(&attached);
                    return Err(RhiError::ValidationFailed {
                        detail: format!("{} GLSL compilation failed: {log}", module.label),
                    });
                }
                self.gl.attach_shader(gl_program, gl_shader);
            }
            attached.push(gl_shader);
        }

        // SAFETY: all attached shaders and the program are live GL objects
        // created by the same current context.
        unsafe {
            self.gl.link_program(gl_program);
            if !self.gl.get_program_link_status(gl_program) {
                let log = self.gl.get_program_info_log(gl_program);
                cleanup_program(&attached);
                return Err(RhiError::ValidationFailed {
                    detail: format!("OpenGL pipeline link failed: {log}"),
                });
            }
            for shader in &attached {
                self.gl.detach_shader(gl_program, *shader);
                self.gl.delete_shader(*shader);
            }
        }

        let gl_vertex_array = match unsafe { self.gl.create_vertex_array() } {
            Ok(vertex_array) => vertex_array,
            Err(detail) => {
                // SAFETY: `gl_program` linked successfully and is not stored yet.
                unsafe { self.gl.delete_program(gl_program) };
                return Err(RhiError::Backend { detail });
            }
        };
        // A VAO owns enable state. Pointer formats are installed when the
        // actual vertex buffer is bound because PipelineDescriptor does not
        // contain buffer handles.
        unsafe {
            self.gl.bind_vertex_array(Some(gl_vertex_array));
            for attribute in &vertex_attributes {
                self.gl.enable_vertex_attrib_array(attribute.location);
            }
            self.gl.bind_vertex_array(None);
        }

        let mut sampler_uniforms = Vec::new();
        let mut push_uniforms = Vec::new();
        // SAFETY: the linked program is valid; reflection queries do not mutate
        // application-visible resources.
        unsafe {
            let uniform_count = self.gl.get_active_uniforms(gl_program);
            for index in 0..uniform_count {
                let Some(active) = self.gl.get_active_uniform(gl_program, index) else {
                    continue;
                };
                if is_sampler_type(active.utype) {
                    if let Some(location) = self.gl.get_uniform_location(gl_program, &active.name) {
                        sampler_uniforms.push(SamplerUniformBinding {
                            name: active.name,
                            location,
                        });
                    }
                    continue;
                }
                let Some(kind) = push_uniform_kind(active.utype) else {
                    continue;
                };
                let Some(base_offset) = push_uniform_offset(&active.name) else {
                    continue;
                };
                let base_name = active.name.strip_suffix("[0]").unwrap_or(&active.name);
                for element in 0..active.size.max(1) as u32 {
                    let name = if active.size > 1 {
                        format!("{base_name}[{element}]")
                    } else {
                        active.name.clone()
                    };
                    if let Some(location) = self.gl.get_uniform_location(gl_program, &name) {
                        push_uniforms.push(PushUniformBinding {
                            name,
                            offset: base_offset + element * kind.size_bytes(),
                            kind,
                            location,
                        });
                    }
                }
            }
        }
        sampler_uniforms.sort_by_key(|uniform| sampler_sort_key(&uniform.name));
        push_uniforms.sort_by_key(|uniform| uniform.offset);

        let mut push_constant_buffer = None;
        if let Some(layout) = &layout_descriptor {
            let push_size = layout
                .push_constant_ranges
                .iter()
                .filter_map(|range| range.offset.checked_add(range.size))
                .max()
                .unwrap_or(0);
            let mut push_blocks = Vec::new();
            let mut ordinary_blocks = Vec::new();
            unsafe {
                let block_count = self
                    .gl
                    .get_program_parameter_i32(gl_program, glow::ACTIVE_UNIFORM_BLOCKS)
                    .max(0) as u32;
                for block_index in 0..block_count {
                    let name = self
                        .gl
                        .get_active_uniform_block_name(gl_program, block_index);
                    if is_push_constant_block(&name) {
                        push_blocks.push((block_index, name));
                    } else {
                        ordinary_blocks.push((block_index, name));
                    }
                }
            }
            if push_blocks.len() > 1 {
                unsafe {
                    self.gl.delete_vertex_array(gl_vertex_array);
                    self.gl.delete_program(gl_program);
                }
                return Err(invalid_descriptor(
                    "pipeline.shader_modules",
                    "linked program exposes more than one push-constant uniform block",
                ));
            }

            let mut uniform_binding_points = layout
                .bind_group_layouts
                .iter()
                .flat_map(|set| {
                    set.bindings.iter().filter_map(move |binding| {
                        let kind = binding.resource_kind.trim().to_ascii_lowercase();
                        (kind == "uniform_buffer" || kind == "ubo")
                            .then(|| gl_binding_point(set.set_index, binding.binding))
                            .flatten()
                    })
                })
                .collect::<Vec<_>>();
            uniform_binding_points.sort_unstable();
            uniform_binding_points.dedup();

            let max_bindings = unsafe {
                self.gl
                    .get_parameter_i32(glow::MAX_UNIFORM_BUFFER_BINDINGS)
                    .max(0) as u32
            };
            if uniform_binding_points
                .iter()
                .any(|binding| *binding >= max_bindings)
            {
                unsafe {
                    self.gl.delete_vertex_array(gl_vertex_array);
                    self.gl.delete_program(gl_program);
                }
                return Err(RhiError::UnsupportedLimit {
                    limit: "OpenGL uniform buffer binding point".to_string(),
                    requested: uniform_binding_points.iter().copied().max().unwrap_or(0) as u64 + 1,
                    available: max_bindings as u64,
                });
            }
            unsafe {
                for ((block_index, _), binding) in
                    ordinary_blocks.iter().zip(uniform_binding_points.iter())
                {
                    self.gl
                        .uniform_block_binding(gl_program, *block_index, *binding);
                }
            }

            if push_size > 0 {
                if let Some((block_index, block_name)) = push_blocks.first() {
                    let Some(binding) = (0..max_bindings)
                        .rev()
                        .find(|candidate| !uniform_binding_points.contains(candidate))
                    else {
                        unsafe {
                            self.gl.delete_vertex_array(gl_vertex_array);
                            self.gl.delete_program(gl_program);
                        }
                        return Err(RhiError::UnsupportedLimit {
                            limit: "OpenGL push-constant UBO binding".to_string(),
                            requested: 1,
                            available: 0,
                        });
                    };
                    let gl_buffer = match unsafe { self.gl.create_buffer() } {
                        Ok(buffer) => buffer,
                        Err(detail) => {
                            unsafe {
                                self.gl.delete_vertex_array(gl_vertex_array);
                                self.gl.delete_program(gl_program);
                            }
                            return Err(RhiError::Backend { detail });
                        }
                    };
                    unsafe {
                        self.gl.bind_buffer(glow::UNIFORM_BUFFER, Some(gl_buffer));
                        self.gl.buffer_data_size(
                            glow::UNIFORM_BUFFER,
                            push_size as i32,
                            glow::DYNAMIC_DRAW,
                        );
                        self.gl
                            .uniform_block_binding(gl_program, *block_index, binding);
                        self.gl
                            .bind_buffer_base(glow::UNIFORM_BUFFER, binding, Some(gl_buffer));
                        self.gl.bind_buffer(glow::UNIFORM_BUFFER, None);
                    }
                    tracing::debug!(
                        target: "opengl",
                        block = %block_name,
                        binding,
                        size = push_size,
                        "mapped RHI push constants to an OpenGL uniform buffer"
                    );
                    push_constant_buffer = Some(PushConstantBuffer {
                        gl_buffer,
                        binding,
                        size_bytes: push_size,
                    });
                }
            }
        }

        let (idx, gen) = self.pipelines.alloc(PipelineSlot {
            gl_program,
            gl_vertex_array,
            vertex_stride: descriptor.vertex_layout.stride_bytes,
            vertex_attributes,
            topology,
            raster_state,
            depth_state,
            blend_state,
            multisample: sample_count > 1,
            render_pass: descriptor.render_pass,
            pipeline_layout: descriptor.pipeline_layout,
            push_constant_buffer,
            push_uniforms,
            sampler_uniforms,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) {
        let slot = self.pipelines.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                // SAFETY: `self.gl` is a valid `glow::Context` created by this
                // device; all objects were created by the same context and are
                // no longer reachable after this handle is freed.
                unsafe {
                    if let Some(push_constants) = slot.value.push_constant_buffer {
                        self.gl.delete_buffer(push_constants.gl_buffer);
                    }
                    self.gl.delete_vertex_array(slot.value.gl_vertex_array);
                    self.gl.delete_program(slot.value.gl_program);
                };
            }
        }
        self.pipelines.free(handle.index);
    }

    // ██ frame lifecycle █████████████████████████████████████████████████████████████████
    };
}

pub(super) use impl_device_pipelines;
