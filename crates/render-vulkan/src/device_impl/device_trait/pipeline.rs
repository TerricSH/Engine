macro_rules! vulkan_device_pipeline_methods {
    () => {
    fn create_pipeline(
        &mut self,
        desc: &PipelineDescriptor,
    ) -> Result<PipelineHandle, render_core::RhiError> {
        validate_graphics_pipeline_descriptor(desc)?;
        let d = &self.logical_device.device;
        let main = c"main";

        if desc.shader_modules.is_empty() {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "shader_modules".into(),
                reason: "a graphics pipeline requires explicit shader module handles".into(),
            });
        }
        if desc.render_pass.is_none() {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "render_pass".into(),
                reason: "a graphics pipeline requires an explicit render pass".into(),
            });
        }
        if desc.pipeline_layout.is_none() {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline_layout".into(),
                reason: "a graphics pipeline requires an explicit pipeline layout".into(),
            });
        }

        // ── Shader stages ──────────────────────────────────────────────
        // If the descriptor provides shader module handles, resolve them
        // from the shader_modules slab.  Otherwise fall back to the
        // embedded vertex/fragment SPIR-V.  Skinned pipelines use the
        // dedicated skinned vertex shader (detected by the presence of a
        // uint32x4 vertex attribute, which indicates joints).
        let render_pass_handle = desc.render_pass.expect("render pass validated above");
        let rp = self
            .render_passes
            .get(render_pass_handle.index, render_pass_handle.generation)
            .copied()
            .ok_or(render_core::RhiError::InvalidHandle)?;
        let expected_colors = self
            .rp_color_formats
            .get(&render_pass_handle.index)
            .ok_or(render_core::RhiError::InvalidHandle)?;
        let requested_colors: Vec<_> = desc
            .render_targets
            .iter()
            .copied()
            .map(texture_format)
            .collect();
        if requested_colors != *expected_colors {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.render_targets".into(),
                reason: "pipeline color formats do not match the render pass".into(),
            });
        }
        let expected_samples = self
            .rp_sample_counts
            .get(&render_pass_handle.index)
            .copied()
            .ok_or(render_core::RhiError::InvalidHandle)?;
        if desc.sample_count.unwrap_or(1) != expected_samples {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.sample_count".into(),
                reason: format!(
                    "pipeline sample count {} does not match render pass sample count {expected_samples}",
                    desc.sample_count.unwrap_or(1)
                ),
            });
        }
        let render_pass_depth = self
            .rp_depth_formats
            .get(&render_pass_handle.index)
            .copied();
        let requested_depth = desc.depth_state.format.map(texture_format);
        if requested_depth.is_some() && requested_depth != render_pass_depth {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.depth_state.format".into(),
                reason: "pipeline depth format does not match the render pass".into(),
            });
        }
        if (desc.depth_state.write_enabled || desc.depth_state.compare.is_some())
            && render_pass_depth.is_none()
        {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.depth_state".into(),
                reason: "depth testing requires a render pass depth attachment".into(),
            });
        }
        let pipeline_layout_handle = desc
            .pipeline_layout
            .expect("pipeline layout validated above");
        let pll = self
            .pipeline_layouts
            .get(
                pipeline_layout_handle.index,
                pipeline_layout_handle.generation,
            )
            .map(|entry| entry.layout)
            .ok_or(render_core::RhiError::InvalidHandle)?;

        let (specialization_data, specialization_entries) =
            vulkan_specialization_data(&desc.specialization);
        let specialization_info = vk::SpecializationInfo::default()
            .map_entries(&specialization_entries)
            .data(&specialization_data);
        let has_specialization = !specialization_entries.is_empty();
        let mut has_vertex = false;
        let mut has_fragment = false;
        let mut sr = Vec::with_capacity(desc.shader_modules.len());
        for handle in &desc.shader_modules {
            let (shader, stage) = self
                .shader_modules
                .get(handle.index, handle.generation)
                .copied()
                .ok_or(render_core::RhiError::InvalidHandle)?;
            if stage == vk::ShaderStageFlags::VERTEX {
                if has_vertex {
                    return Err(render_core::RhiError::InvalidDescriptor {
                        field: "pipeline.shader_modules".into(),
                        reason: "graphics shader stages must not be duplicated".into(),
                    });
                }
                has_vertex = true;
            } else if stage == vk::ShaderStageFlags::FRAGMENT {
                if has_fragment {
                    return Err(render_core::RhiError::InvalidDescriptor {
                        field: "pipeline.shader_modules".into(),
                        reason: "graphics shader stages must not be duplicated".into(),
                    });
                }
                has_fragment = true;
            } else {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "pipeline.shader_modules".into(),
                    reason: "graphics pipelines accept only vertex and fragment shaders".into(),
                });
            }
            let mut stage_info = vk::PipelineShaderStageCreateInfo::default()
                .stage(stage)
                .module(shader)
                .name(main);
            if has_specialization {
                stage_info = stage_info.specialization_info(&specialization_info);
            }
            sr.push(stage_info);
        }
        if !has_vertex || (!desc.render_targets.is_empty() && !has_fragment) {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.shader_modules".into(),
                reason: "a vertex shader and, for color output, a fragment shader are required"
                    .into(),
            });
        }

        // ── Vertex input state ─────────────────────────────────────────
        let stride = desc.vertex_layout.stride_bytes;
        let vb = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(stride)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let va: Vec<vk::VertexInputAttributeDescription> = desc
            .vertex_layout
            .attributes
            .iter()
            .enumerate()
            .map(|(i, a)| vk::VertexInputAttributeDescription {
                location: i as u32,
                binding: 0,
                format: vfmt(&a.format),
                offset: a.offset_bytes,
            })
            .collect();
        let vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vb)
            .vertex_attribute_descriptions(&va);

        // ── Input assembly (topology from descriptor) ─────────────────
        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(parse_topology(&desc.topology));

        // ── Viewport state ─────────────────────────────────────────────
        let vs2 = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        // ── Rasterization state (polygon mode + cull mode from desc) ──
        let cull_mode = match desc.raster_state.cull_mode.as_deref() {
            Some("front") => vk::CullModeFlags::FRONT,
            Some("back") => vk::CullModeFlags::BACK,
            Some("none") | None => vk::CullModeFlags::NONE,
            _ => vk::CullModeFlags::NONE,
        };
        let front_face = match desc.raster_state.front_face.as_deref() {
            Some("clockwise" | "cw") => vk::FrontFace::CLOCKWISE,
            _ => vk::FrontFace::COUNTER_CLOCKWISE,
        };
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(parse_polygon_mode(&desc.polygon_mode))
            .cull_mode(cull_mode)
            .front_face(front_face)
            .line_width(1.0);

        // ── Multisample state (sample count from desc) ─────────────────
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(parse_sample_count(desc.sample_count));

        // ── Color blend state ──────────────────────────────────────────
        let blend_attachment = match &desc.blend_state.mode {
            Some(mode) => blend_attachment_from_mode(mode),
            None => blend_attachment_from_mode("Opaque"),
        };
        let cba = vec![blend_attachment; desc.render_targets.len()];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);

        // ── Dynamic state ──────────────────────────────────────────────
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);

        // ── Render pass ────────────────────────────────────────────────
        // If the descriptor carries a handle, resolve it; otherwise create
        // an inline render pass from the descriptor's render targets.
        // ── Pipeline layout ────────────────────────────────────────────
        // ── Depth stencil state ────────────────────────────────────────
        let depth_enabled = desc.depth_state.write_enabled || desc.depth_state.compare.is_some();
        let ds_state = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(depth_enabled)
            .depth_write_enable(desc.depth_state.write_enabled)
            .depth_compare_op(compare_op(&desc.depth_state.compare));

        // ── Build the pipeline ─────────────────────────────────────────
        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs2)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .depth_stencil_state(&ds_state)
            .color_blend_state(&cb)
            .dynamic_state(&ds)
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid
        // graphics pipeline; `self.pipeline_cache` may be null; `None` means
        // no custom allocator.
        let pipeline = unsafe { d.create_graphics_pipelines(self.pipeline_cache, &[pinfo], None) }
            .map_err(|(_, r)| render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            })?[0];

        let (idx, gen) = self.pipelines.insert(PipeEntry { pipeline });
        Ok(PipelineHandle::new(idx, gen))
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) {
        if let Some(entry) = self.pipelines.remove(handle.index, handle.generation) {
            self.retire_pipeline(entry.pipeline);
        }
    }
    };
}

pub(super) use vulkan_device_pipeline_methods;
