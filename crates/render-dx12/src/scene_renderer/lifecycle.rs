use super::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12SceneRenderer {
    pub fn new(device: Dx12Device, swapchain: SwapchainHandle, width: u32, height: u32) -> Self {
        Self {
            device,
            meshes: HashMap::new(),
            materials: HashMap::new(),
            textures: HashMap::new(),
            environments: HashMap::new(),
            fallback_environment: None,
            fallback_ui_texture: None,
            bone_buffers: HashMap::new(),
            vertex_draw_buffer: None,
            morphed_vertex_buffers: HashMap::new(),
            morph_target_sets: HashMap::new(),
            particle_instance_buffers: HashMap::new(),
            gpu_particle_parameter_buffers: HashMap::new(),
            gpu_particle_dummy_buffer: None,
            clustered_light_buffer: None,
            clustered_grid_buffer: None,
            clustered_index_buffer: None,
            ui_vertex_buffer: None,
            mesh_revisions: HashMap::new(),
            width: width.max(1),
            height: height.max(1),
            swapchain,
            pipeline_layout: None,
            pipeline: None,
            double_sided_pipeline: None,
            blend_pipeline: None,
            blend_double_sided_pipeline: None,
            oit_pipeline: None,
            oit_double_sided_pipeline: None,
            additive_pipeline: None,
            additive_double_sided_pipeline: None,
            skinned_pipeline: None,
            skinned_double_sided_pipeline: None,
            skinned_blend_pipeline: None,
            skinned_blend_double_sided_pipeline: None,
            skinned_oit_pipeline: None,
            skinned_oit_double_sided_pipeline: None,
            skinned_additive_pipeline: None,
            skinned_additive_double_sided_pipeline: None,
            particle_pipeline: None,
            particle_additive_pipeline: None,
            particle_oit_pipeline: None,
            gpu_particle_pipeline: None,
            gpu_particle_additive_pipeline: None,
            gpu_particle_oit_pipeline: None,
            skybox_pipeline: None,
            hdr_texture: None,
            oit_accum_texture: None,
            oit_optical_depth_texture: None,
            hdr_depth_texture: None,
            hdr_render_pass: None,
            hdr_framebuffer: None,
            tone_map_pipeline_layout: None,
            tone_map_pipeline: None,
            ui_pipeline_layout: None,
            ui_pipeline: None,
            shadow_texture: None,
            shadow_render_pass: None,
            shadow_framebuffer: None,
            shadow_pipeline_layout: None,
            shadow_pipeline: None,
            skinned_shadow_pipeline: None,
            shadow_frame_data: None,
            active_frame: None,
            fatal_frame_error: None,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        self.resize_surface(width, height)
    }

    pub fn wait_idle(&self) {
        self.device.wait_idle();
    }

    pub(super) fn resize_surface(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<(), Vec<Diagnostic>> {
        if width == 0 || height == 0 {
            return Err(vec![Diagnostic::new(
                "DX1240",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("resize dimensions must be non-zero, got {width}x{height}"),
            )]);
        }
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1242",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot resize the DX12 surface while a frame is active",
            )]);
        }

        self.device
            .recreate_swapchain(self.swapchain, width, height)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1241",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("resize/recreate_swapchain failed: {error:?}"),
                )]
            })?;
        self.width = width;
        self.height = height;
        if let Some(framebuffer) = self.hdr_framebuffer.take() {
            self.device.destroy_framebuffer(framebuffer);
        }
        if let Some(texture) = self.hdr_texture.take() {
            self.device.destroy_texture(texture);
        }
        if let Some(texture) = self.oit_accum_texture.take() {
            self.device.destroy_texture(texture);
        }
        if let Some(texture) = self.oit_optical_depth_texture.take() {
            self.device.destroy_texture(texture);
        }
        if let Some(texture) = self.hdr_depth_texture.take() {
            self.device.destroy_texture(texture);
        }
        Ok(())
    }

    pub(super) fn ensure_hdr_targets(&mut self) -> Result<(), render_core::RhiError> {
        use render_core::{
            FramebufferDescriptor, RenderPassDescriptor, TextureDescriptor, TextureFormat,
            TextureUsage,
        };

        let render_pass = match self.hdr_render_pass {
            Some(render_pass) => render_pass,
            None => {
                let render_pass = self.device.create_render_pass(&RenderPassDescriptor {
                    color_attachments: vec![
                        TextureFormat::Rgba16Float,
                        TextureFormat::Rgba16Float,
                        TextureFormat::Rgba16Float,
                    ],
                    depth_stencil_format: Some(TextureFormat::Depth32Float),
                    sample_count: 1,
                    present_after: false,
                    debug_label: Some("dx12-hdr-forward".into()),
                })?;
                self.hdr_render_pass = Some(render_pass);
                render_pass
            }
        };
        if self.hdr_framebuffer.is_some()
            && self.hdr_texture.is_some()
            && self.oit_accum_texture.is_some()
            && self.oit_optical_depth_texture.is_some()
            && self.hdr_depth_texture.is_some()
        {
            return Ok(());
        }

        let hdr_texture = self.device.create_texture(&TextureDescriptor {
            width: self.width,
            height: self.height,
            depth_or_layers: 1,
            mip_levels: 1,
            format: TextureFormat::Rgba16Float,
            usage_flags: TextureUsage(TextureUsage::COLOR_ATTACHMENT.0 | TextureUsage::SAMPLED.0),
            sample_count: 1,
            debug_label: Some("dx12-hdr-color".into()),
        })?;
        let hdr_depth = match self.device.create_texture(&TextureDescriptor {
            width: self.width,
            height: self.height,
            depth_or_layers: 1,
            mip_levels: 1,
            format: TextureFormat::Depth32Float,
            usage_flags: TextureUsage(TextureUsage::DEPTH_ATTACHMENT.0),
            sample_count: 1,
            debug_label: Some("dx12-hdr-depth".into()),
        }) {
            Ok(texture) => texture,
            Err(error) => {
                self.device.destroy_texture(hdr_texture);
                return Err(error);
            }
        };
        let target_width = self.width;
        let target_height = self.height;
        let make_oit_texture = |device: &mut Dx12Device, label: &str| {
            device.create_texture(&TextureDescriptor {
                width: target_width,
                height: target_height,
                depth_or_layers: 1,
                mip_levels: 1,
                format: TextureFormat::Rgba16Float,
                usage_flags: TextureUsage(
                    TextureUsage::COLOR_ATTACHMENT.0 | TextureUsage::SAMPLED.0,
                ),
                sample_count: 1,
                debug_label: Some(label.into()),
            })
        };
        let oit_accum = match make_oit_texture(&mut self.device, "dx12-oit-accumulation") {
            Ok(texture) => texture,
            Err(error) => {
                self.device.destroy_texture(hdr_depth);
                self.device.destroy_texture(hdr_texture);
                return Err(error);
            }
        };
        let oit_optical_depth = match make_oit_texture(&mut self.device, "dx12-oit-optical-depth") {
            Ok(texture) => texture,
            Err(error) => {
                self.device.destroy_texture(oit_accum);
                self.device.destroy_texture(hdr_depth);
                self.device.destroy_texture(hdr_texture);
                return Err(error);
            }
        };
        let framebuffer = match self.device.create_framebuffer(&FramebufferDescriptor {
            render_pass,
            color_attachments: vec![hdr_texture, oit_accum, oit_optical_depth],
            depth_stencil_attachment: Some(hdr_depth),
            width: self.width,
            height: self.height,
            debug_label: Some("dx12-hdr-forward".into()),
        }) {
            Ok(framebuffer) => framebuffer,
            Err(error) => {
                self.device.destroy_texture(oit_optical_depth);
                self.device.destroy_texture(oit_accum);
                self.device.destroy_texture(hdr_depth);
                self.device.destroy_texture(hdr_texture);
                return Err(error);
            }
        };
        self.hdr_texture = Some(hdr_texture);
        self.oit_accum_texture = Some(oit_accum);
        self.oit_optical_depth_texture = Some(oit_optical_depth);
        self.hdr_depth_texture = Some(hdr_depth);
        self.hdr_framebuffer = Some(framebuffer);
        Ok(())
    }

    pub(super) fn ensure_environment_fallback(&mut self) -> Result<(), render_core::RhiError> {
        if self.fallback_environment.is_some() {
            return Ok(());
        }
        // Keep a type-correct TextureCube bound when a scene has no HDRI.
        // Its contribution is disabled through environment intensity.
        let one = 0x3c00_u16.to_le_bytes();
        let pixel = [
            one[0], one[1], one[0], one[1], one[0], one[1], one[0], one[1],
        ];
        let mip = engine_renderer::EnvironmentCubeMip {
            face_size: 1,
            faces: vec![pixel.to_vec(); 6],
        };
        self.fallback_environment = Some(self.device.upload_sampled_rgba16f_cube(&[mip])?);
        Ok(())
    }

    pub(super) fn ensure_ui_fallback(&mut self) -> Result<(), render_core::RhiError> {
        if self.fallback_ui_texture.is_some() {
            return Ok(());
        }
        self.fallback_ui_texture = Some(self.device.upload_sampled_rgba8(
            1,
            1,
            engine_renderer::ColorSpace::Linear,
            &[engine_renderer::TextureMipLevel {
                width: 1,
                height: 1,
                bytes: vec![255; 4],
            }],
            engine_renderer::SamplerDescriptor::default(),
        )?);
        Ok(())
    }
}
