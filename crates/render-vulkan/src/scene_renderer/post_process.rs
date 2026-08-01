use super::*;

impl SceneRenderer {
    /// Execute the tone-mapping pass (HDR -> LDR to swapchain).
    pub(super) fn execute_tonemap_pass(
        &mut self,
        input: &RenderFrameInput,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if self.cur_enc.is_none() {
            return Err(vec![Diagnostic::new(
                "RV0227",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map pass requires an active frame encoder",
            )]);
        }

        let swapchain_format = self
            .device
            .swapchain
            .as_ref()
            .map(|swapchain| swapchain.format)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0228",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "tone-map pass requires an active swapchain format",
                )]
            })?;
        let tone_map_push = tone_map_push_constants(
            input.render_options.tone_mapping,
            input.render_options.exposure_ev100,
            input.render_options.post_process,
            swapchain_format,
            input.render_options.transparency_mode
                == engine_renderer::TransparencyMode::WeightedBlendedOit,
        )
        .map_err(|message| {
            vec![Diagnostic::new(
                "RV0320",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )
            .path("render_options.exposure_ev100")]
        })?;
        let tone_map_push_bytes = tone_map_push.to_bytes();
        let d = &self.device.logical_device.device;
        let fi = self.device.current_frame;
        let cmd = self.device.frame_sync[fi].command_buffer;

        let tone_rp = self.device.tone_rp.unwrap_or(vk::RenderPass::null());
        let tone_pl = self.device.tone_pipeline.unwrap_or(vk::Pipeline::null());
        let tone_pll = self
            .device
            .tone_pipeline_layout
            .unwrap_or(vk::PipelineLayout::null());
        let tone_ds = self
            .device
            .tone_desc_set
            .unwrap_or(vk::DescriptorSet::null());
        if tone_rp == vk::RenderPass::null()
            || tone_pl == vk::Pipeline::null()
            || tone_pll == vk::PipelineLayout::null()
            || tone_ds == vk::DescriptorSet::null()
        {
            return Err(vec![Diagnostic::new(
                "RV0228",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map pass resources are incomplete",
            )]);
        }

        let tone_fb = self
            .device
            .tone_framebuffers
            .get(self.cur_fb_index as usize)
            .copied()
            .unwrap_or(vk::Framebuffer::null());
        if tone_fb == vk::Framebuffer::null() {
            return Err(vec![Diagnostic::new(
                "RV0229",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map framebuffer is missing for the acquired swapchain image",
            )]);
        }

        let render_view = input.views.first().ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0013",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map pass requires a RenderView",
            )]
        })?;
        let scene_viewport = vulkan_viewport_rect(
            render_view.viewport_rect_normalized,
            self.width,
            self.height,
        )
        .map_err(|message| {
            vec![Diagnostic::new(
                "RV0318",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )]
        })?;
        let swapchain_clear = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        }];

        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(tone_rp)
            .framebuffer(tone_fb)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.width,
                    height: self.height,
                },
            })
            .clear_values(&swapchain_clear);
        // SAFETY: `cmd` is recording outside a pass; tone pass/framebuffer and
        // clear/render-area inputs are live and compatible.
        unsafe {
            d.cmd_begin_render_pass(cmd, &rpbi, vk::SubpassContents::INLINE);
        }

        // SAFETY: the pass is active; dynamic states are enabled, values match
        // the target, and `tone_pl` is a live compatible pipeline.
        unsafe {
            d.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: self.width as f32,
                    height: self.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            // Keep the full-surface viewport so the full-screen triangle's UV
            // coordinates sample the matching pixels in the HDR attachment.
            // Scissoring only the scene region prevents it from covering the
            // editor chrome or an authored letterbox region.
            d.cmd_set_scissor(cmd, 0, &[scene_viewport.scissor]);
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, tone_pl);
        }

        if tone_ds != vk::DescriptorSet::null() {
            let sets = [tone_ds];
            // SAFETY: the live descriptor set matches set=0 of `tone_pll` and
            // its slice remains alive through the recording call.
            unsafe {
                d.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    tone_pll,
                    0,
                    &sets,
                    &[],
                );
            }
        }

        // SAFETY: push bytes fit the declared fragment range of the live layout.
        unsafe {
            d.cmd_push_constants(
                cmd,
                tone_pll,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                &tone_map_push_bytes,
            );
        }

        // SAFETY: compatible pipeline/descriptors are bound in the active pass;
        // this records one draw and the matching end exactly once.
        unsafe {
            d.cmd_draw(cmd, 3, 1, 0, 0);
            d.cmd_end_render_pass(cmd);
        }

        stats.draw_calls += 1;
        stats.triangles += 1;
        Ok(())
    }

    // UI overlay rendering.

    /// Render UI in a dedicated load-op pass over the tone-mapped swapchain
    /// image. This is invoked by the graph's Present pass, so no later scene
    /// pass can overwrite the overlay.
    pub(super) fn execute_ui_overlay_pass(
        &mut self,
        batches: &[UiBatch],
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if batches.is_empty() {
            return Ok(());
        }
        let prepared = prepare_ui_overlay(batches, self.width, self.height).map_err(|message| {
            vec![Diagnostic::new(
                "RV0298",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )]
        })?;
        if prepared.draws.is_empty() {
            return Ok(());
        }

        let mut descriptor_sets = Vec::with_capacity(prepared.draws.len());
        for draw in &prepared.draws {
            let descriptor_set = self
                .device
                .ui_overlay_descriptor_set(draw.texture_id.as_deref())
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0299",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot bind UI texture: {error}"),
                    )]
                })?;
            descriptor_sets.push(descriptor_set);
        }

        let frame_index = self.device.current_frame;
        let required_bytes = u64::try_from(prepared.vertex_bytes.len()).map_err(|_| {
            vec![Diagnostic::new(
                "RV0300",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "UI vertex data exceeds the Vulkan buffer size contract",
            )]
        })?;
        if self.ui_vb_capacities[frame_index] < required_bytes {
            if let Some(old) = self.ui_vbs[frame_index].take() {
                self.device.destroy_buffer(old);
            }
            let vertex_buffer = self
                .device
                .create_buffer(&BufferDescriptor {
                    size_bytes: required_bytes,
                    usage_flags: render_core::BufferUsage::VERTEX,
                    memory_hint: MemoryHint::CpuToGpu,
                    debug_label: Some(format!("ui-overlay-vb-{frame_index}")),
                })
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0216",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("create UI vertex buffer: {error:?}"),
                    )]
                })?;
            self.ui_vbs[frame_index] = Some(vertex_buffer);
            self.ui_vb_capacities[frame_index] = required_bytes;
        }
        let vertex_buffer = self.ui_vbs[frame_index].ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0301",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "UI vertex buffer was not retained after creation",
            )]
        })?;
        self.device
            .write_buffer(vertex_buffer, &prepared.vertex_bytes, 0)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0217",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("write UI vertex buffer: {error:?}"),
                )]
            })?;

        let raw_vertex_buffer = self
            .device
            .buffers
            .get(vertex_buffer.index, vertex_buffer.generation)
            .map(|entry| entry.buffer)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0302",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    "UI vertex buffer handle became invalid before recording",
                )]
            })?;
        let render_pass = self.device.ui_overlay_rp.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0303",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "UI overlay render pass is not initialized",
            )]
        })?;
        let pipeline = self.device.ui_overlay_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0304",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "UI overlay pipeline is not initialized",
            )]
        })?;
        let pipeline_layout = self.device.ui_overlay_pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0305",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "UI overlay pipeline layout is not initialized",
            )]
        })?;
        let framebuffer = self
            .device
            .ui_overlay_framebuffers
            .get(self.cur_fb_index as usize)
            .copied()
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0306",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    "UI overlay framebuffer is missing for the acquired swapchain image",
                )]
            })?;
        let command_buffer = self
            .device
            .frame_sync
            .get(frame_index)
            .map(|frame| frame.command_buffer)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0307",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    "UI overlay command buffer is unavailable",
                )]
            })?;
        let d = &self.device.logical_device.device;
        let begin_info = vk::RenderPassBeginInfo::default()
            .render_pass(render_pass)
            .framebuffer(framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.width,
                    height: self.height,
                },
            });
        // SAFETY: command buffer/pass/framebuffer/pipeline/layout and raw vertex
        // buffer are live/compatible; prepared ranges/scissors were validated,
        // and this block records one balanced UI render pass.
        unsafe {
            d.cmd_begin_render_pass(command_buffer, &begin_info, vk::SubpassContents::INLINE);
            d.cmd_set_viewport(
                command_buffer,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: self.width as f32,
                    height: self.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            d.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
            d.cmd_bind_vertex_buffers(command_buffer, 0, &[raw_vertex_buffer], &[0]);
            let mut screen_size = [0u8; 8];
            screen_size[..4].copy_from_slice(&(self.width as f32).to_ne_bytes());
            screen_size[4..].copy_from_slice(&(self.height as f32).to_ne_bytes());
            d.cmd_push_constants(
                command_buffer,
                pipeline_layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                &screen_size,
            );
            for (draw, descriptor_set) in prepared.draws.iter().zip(descriptor_sets) {
                d.cmd_set_scissor(
                    command_buffer,
                    0,
                    &[vk::Rect2D {
                        offset: vk::Offset2D {
                            x: draw.scissor.x,
                            y: draw.scissor.y,
                        },
                        extent: vk::Extent2D {
                            width: draw.scissor.width,
                            height: draw.scissor.height,
                        },
                    }],
                );
                d.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipeline_layout,
                    0,
                    &[descriptor_set],
                    &[],
                );
                d.cmd_draw(command_buffer, draw.vertex_count, 1, draw.first_vertex, 0);
                stats.draw_calls = stats.draw_calls.saturating_add(1);
                stats.triangles = stats
                    .triangles
                    .saturating_add(u64::from(draw.vertex_count / 3));
            }
            d.cmd_end_render_pass(command_buffer);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Per-pass GPU timestamps (ENG-04)
    // ------------------------------------------------------------------
}
