macro_rules! impl_encoder_pass {
    () => {
        fn begin_render_pass(
            &mut self,
            render_pass: RenderPassHandle,
            framebuffer: FramebufferHandle,
            area: (u32, u32, u32, u32),
            clear_color: [f32; 4],
            clear_depth: Option<f32>,
        ) {
            if self.current_render_pass.is_some() {
                self.record_error("nested OpenGL render passes are not supported");
                return;
            }
            let Some((fb, framebuffer_render_pass)) = self.resolve_framebuffer(framebuffer) else {
                self.record_error("begin_render_pass received an invalid framebuffer handle");
                return;
            };
            if framebuffer_render_pass != render_pass {
                self.record_error("framebuffer was created for a different render pass");
                return;
            }
            if area.2 == 0
                || area.3 == 0
                || area.0 > i32::MAX as u32
                || area.1 > i32::MAX as u32
                || area.2 > i32::MAX as u32
                || area.3 > i32::MAX as u32
            {
                self.record_error(
                    "render-pass area must be non-zero and fit OpenGL signed integers",
                );
                return;
            }
            if clear_color.iter().any(|channel| !channel.is_finite())
                || clear_depth
                    .is_some_and(|depth| !depth.is_finite() || !(0.0..=1.0).contains(&depth))
            {
                self.record_error(
                    "render-pass clear values must be finite and depth must be in 0..=1",
                );
                return;
            }

            // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
            // the framebuffer handle was created by the same context, and the
            // encoder is dropped before the device.
            unsafe {
                self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
                self.gl.clear_color(
                    clear_color[0],
                    clear_color[1],
                    clear_color[2],
                    clear_color[3],
                );
                if let Some(d) = clear_depth {
                    self.gl.clear_depth_f64(d as f64);
                    self.gl.depth_mask(true);
                }
                self.gl
                    .viewport(area.0 as i32, area.1 as i32, area.2 as i32, area.3 as i32);
                let mut clear_mask = glow::COLOR_BUFFER_BIT;
                if clear_depth.is_some() {
                    clear_mask |= glow::DEPTH_BUFFER_BIT;
                }
                self.gl.clear(clear_mask);
            }

            self.current_framebuffer = Some(fb);
            self.current_render_pass = Some(render_pass);
        }

        fn bind_pipeline(&mut self, pipeline: PipelineHandle) {
            let Some(pipeline) = self.resolve_pipeline(pipeline) else {
                self.record_error("bind_pipeline received an invalid pipeline handle");
                return;
            };
            if let (Some(active), Some(expected)) = (self.current_render_pass, pipeline.render_pass)
            {
                if active != expected {
                    self.record_error("pipeline is incompatible with the active render pass");
                    return;
                }
            }
            // SAFETY: all state in the snapshot was validated at pipeline creation
            // and all GL objects belong to this current context.
            unsafe {
                self.gl.use_program(Some(pipeline.gl_program));
                self.gl.bind_vertex_array(Some(pipeline.gl_vertex_array));
                if let Some(cull_face) = pipeline.raster_state.cull_face {
                    self.gl.enable(glow::CULL_FACE);
                    self.gl.cull_face(cull_face);
                } else {
                    self.gl.disable(glow::CULL_FACE);
                }
                self.gl.front_face(pipeline.raster_state.front_face);
                if !self.gl.version().is_embedded {
                    self.gl
                        .polygon_mode(glow::FRONT_AND_BACK, pipeline.raster_state.polygon_mode);
                }
                if pipeline.depth_state.enabled {
                    self.gl.enable(glow::DEPTH_TEST);
                    self.gl.depth_func(pipeline.depth_state.compare);
                } else {
                    self.gl.disable(glow::DEPTH_TEST);
                }
                self.gl.depth_mask(pipeline.depth_state.write_enabled);
                match pipeline.blend_state {
                    PipelineBlendState::Disabled => self.gl.disable(glow::BLEND),
                    PipelineBlendState::Alpha => {
                        self.gl.enable(glow::BLEND);
                        self.gl.blend_equation(glow::FUNC_ADD);
                        self.gl.blend_func_separate(
                            glow::SRC_ALPHA,
                            glow::ONE_MINUS_SRC_ALPHA,
                            glow::ONE,
                            glow::ONE_MINUS_SRC_ALPHA,
                        );
                    }
                    PipelineBlendState::PremultipliedAlpha => {
                        self.gl.enable(glow::BLEND);
                        self.gl.blend_equation(glow::FUNC_ADD);
                        self.gl.blend_func_separate(
                            glow::ONE,
                            glow::ONE_MINUS_SRC_ALPHA,
                            glow::ONE,
                            glow::ONE_MINUS_SRC_ALPHA,
                        );
                    }
                    PipelineBlendState::Additive => {
                        self.gl.enable(glow::BLEND);
                        self.gl.blend_equation(glow::FUNC_ADD);
                        self.gl.blend_func(glow::ONE, glow::ONE);
                    }
                }
                if pipeline.multisample {
                    self.gl.enable(glow::MULTISAMPLE);
                } else {
                    self.gl.disable(glow::MULTISAMPLE);
                }
                if let Some(index_buffer) = self.current_index_buffer {
                    self.gl
                        .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buffer));
                }
            }
            if let Some((vertex_buffer, offset, _)) = self.current_vertex_buffer {
                self.configure_vertex_input(&pipeline, vertex_buffer, offset);
            }
            self.current_program = Some(pipeline.gl_program);
            self.current_pipeline = Some(pipeline);
        }
    };
}

pub(super) use impl_encoder_pass;
