use super::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12SceneRenderer {
    /// Create the minimal static PBR32 forward PSO used by this backend.
    pub(super) fn ensure_pipeline(&mut self) {
        use render_core::{
            BindGroupLayoutBinding, BindGroupLayoutDescriptor, PipelineDescriptor,
            PipelineLayoutDescriptor, PushConstantRange, ShaderFormat, ShaderModuleDescriptor,
            ShaderStage, TextureDescriptor, TextureUsage, VertexAttribute, VertexLayout,
        };

        if let Err(error) = self.ensure_hdr_targets() {
            tracing::error!(target: "scene_renderer", ?error, "create DX12 HDR targets failed");
            return;
        }
        if let Err(error) = self.ensure_environment_fallback() {
            tracing::error!(target: "scene_renderer", ?error, "create DX12 fallback environment failed");
            return;
        }
        if let Err(error) = self.ensure_ui_fallback() {
            tracing::error!(target: "scene_renderer", ?error, "create DX12 UI fallback failed");
            return;
        }

        if self.pipeline.is_some()
            && self.double_sided_pipeline.is_some()
            && self.blend_pipeline.is_some()
            && self.blend_double_sided_pipeline.is_some()
            && self.oit_pipeline.is_some()
            && self.oit_double_sided_pipeline.is_some()
            && self.additive_pipeline.is_some()
            && self.additive_double_sided_pipeline.is_some()
            && self.skinned_pipeline.is_some()
            && self.skinned_double_sided_pipeline.is_some()
            && self.skinned_blend_pipeline.is_some()
            && self.skinned_blend_double_sided_pipeline.is_some()
            && self.skinned_oit_pipeline.is_some()
            && self.skinned_oit_double_sided_pipeline.is_some()
            && self.skinned_additive_pipeline.is_some()
            && self.skinned_additive_double_sided_pipeline.is_some()
            && self.particle_pipeline.is_some()
            && self.particle_additive_pipeline.is_some()
            && self.particle_oit_pipeline.is_some()
            && self.gpu_particle_pipeline.is_some()
            && self.gpu_particle_additive_pipeline.is_some()
            && self.gpu_particle_oit_pipeline.is_some()
            && self.skybox_pipeline.is_some()
            && self.tone_map_pipeline.is_some()
            && self.fallback_environment.is_some()
            && self.ui_pipeline.is_some()
            && self.fallback_ui_texture.is_some()
            && self.shadow_pipeline.is_some()
            && self.skinned_shadow_pipeline.is_some()
            && self.shadow_texture.is_some()
            && self.shadow_framebuffer.is_some()
        {
            return;
        }

        let vs_bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/scene_vs.dxil"));
        let ps_bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/scene_ps.dxil"));
        if vs_bytes.is_empty() || ps_bytes.is_empty() {
            tracing::error!(
                target: "scene_renderer",
                "DXIL shaders are unavailable; DX12 rendering cannot start"
            );
            return;
        }

        let layout = match self
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 3,
                    offset: 0,
                    size: 240,
                }],
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "scene_resource_set".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampler_set7".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 1,
                            resource_kind: "uniform_buffer".into(),
                        },
                    ],
                }],
                debug_label: Some("scene_renderer".into()),
            }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "create_pipeline_layout failed");
                return;
            }
        };
        self.pipeline_layout = Some(layout);

        let vertex_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Vertex,
            source_bytes: vs_bytes.to_vec(),
            source_hash: [0; 32],
            entry_points: vec!["VSMain".into()],
            debug_label: Some("scene_renderer_vs".into()),
        }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "vertex shader creation failed");
                return;
            }
        };

        let pixel_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Fragment,
            source_bytes: ps_bytes.to_vec(),
            source_hash: [1; 32],
            entry_points: vec!["PSMain".into()],
            debug_label: Some("scene_renderer_ps".into()),
        }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "pixel shader creation failed");
                return;
            }
        };

        let skinned_vs_bytes: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/scene_skinned_vs.dxil"));
        let skinned_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: skinned_vs_bytes.to_vec(),
                source_hash: [2; 32],
                entry_points: vec!["SkinnedVSMain".into()],
                debug_label: Some("scene_renderer_skinned_vs".into()),
            },
        ) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "skinned vertex shader creation failed");
                return;
            }
        };

        let vertex_layout = VertexLayout {
            stride_bytes: 32,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
                VertexAttribute {
                    semantic: "TEXCOORD".into(),
                    format: "float32x2".into(),
                    offset_bytes: 24,
                },
            ],
        };
        let shadow_static_vertex_layout = vertex_layout.clone();
        let descriptor = PipelineDescriptor {
            shader_modules: vec![vertex_shader, pixel_shader],
            vertex_layout,
            pipeline_layout: Some(layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
            ],
            depth_state: render_core::DepthState {
                format: Some(render_core::TextureFormat::Depth32Float),
                write_enabled: true,
                compare: Some("less".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("back".into()),
                front_face: Some("ccw".into()),
            },
            render_pass: self.hdr_render_pass,
            ..PipelineDescriptor::default()
        };
        let static_variants = match create_surface_pipeline_variants(
            &mut self.device,
            &descriptor,
            "scene-static",
        ) {
            Ok(variants) => variants,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 static surface PSO creation failed");
                return;
            }
        };

        let skinned_vertex_layout = VertexLayout {
            stride_bytes: 64,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
                VertexAttribute {
                    semantic: "TEXCOORD".into(),
                    format: "float32x2".into(),
                    offset_bytes: 24,
                },
                VertexAttribute {
                    semantic: "JOINTS".into(),
                    format: "uint32x4".into(),
                    offset_bytes: 32,
                },
                VertexAttribute {
                    semantic: "WEIGHTS".into(),
                    format: "float32x4".into(),
                    offset_bytes: 48,
                },
            ],
        };
        let skinned_descriptor = PipelineDescriptor {
            shader_modules: vec![skinned_vertex_shader, pixel_shader],
            vertex_layout: skinned_vertex_layout,
            ..descriptor.clone()
        };
        let skinned_variants = match create_surface_pipeline_variants(
            &mut self.device,
            &skinned_descriptor,
            "scene-skinned",
        ) {
            Ok(variants) => variants,
            Err(error) => {
                for pipeline in static_variants {
                    self.device.destroy_pipeline(pipeline);
                }
                tracing::error!(target: "scene_renderer", ?error, "DX12 skinned surface PSO creation failed");
                return;
            }
        };
        self.pipeline = Some(static_variants[0]);
        self.double_sided_pipeline = Some(static_variants[1]);
        self.blend_pipeline = Some(static_variants[2]);
        self.blend_double_sided_pipeline = Some(static_variants[3]);
        self.additive_pipeline = Some(static_variants[4]);
        self.additive_double_sided_pipeline = Some(static_variants[5]);
        self.oit_pipeline = Some(static_variants[6]);
        self.oit_double_sided_pipeline = Some(static_variants[7]);
        self.skinned_pipeline = Some(skinned_variants[0]);
        self.skinned_double_sided_pipeline = Some(skinned_variants[1]);
        self.skinned_blend_pipeline = Some(skinned_variants[2]);
        self.skinned_blend_double_sided_pipeline = Some(skinned_variants[3]);
        self.skinned_additive_pipeline = Some(skinned_variants[4]);
        self.skinned_additive_double_sided_pipeline = Some(skinned_variants[5]);
        self.skinned_oit_pipeline = Some(skinned_variants[6]);
        self.skinned_oit_double_sided_pipeline = Some(skinned_variants[7]);

        let particle_vs_bytes: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/particle_vs.dxil"));
        let particle_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: particle_vs_bytes.to_vec(),
                source_hash: [7; 32],
                entry_points: vec!["ParticleVSMain".into()],
                debug_label: Some("dx12-particle-vs".into()),
            },
        ) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 particle vertex shader creation failed");
                return;
            }
        };
        let particle_vertex_layout = VertexLayout {
            stride_bytes: 32,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
                VertexAttribute {
                    semantic: "TEXCOORD".into(),
                    format: "float32x2".into(),
                    offset_bytes: 24,
                },
                VertexAttribute {
                    semantic: "INSTANCE_POSITION_SIZE".into(),
                    format: "float32x4".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "INSTANCE_ROTATION_AGE".into(),
                    format: "float32x2".into(),
                    offset_bytes: 16,
                },
                VertexAttribute {
                    semantic: "INSTANCE_COLOR".into(),
                    format: "uint32".into(),
                    offset_bytes: 24,
                },
            ],
        };
        let particle_descriptor = PipelineDescriptor {
            shader_modules: vec![particle_vertex_shader, pixel_shader],
            vertex_layout: particle_vertex_layout,
            pipeline_layout: Some(layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
            ],
            depth_state: render_core::DepthState {
                format: Some(render_core::TextureFormat::Depth32Float),
                write_enabled: false,
                compare: Some("less".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("none".into()),
                front_face: Some("ccw".into()),
            },
            blend_state: render_core::BlendState {
                mode: Some("alpha".into()),
            },
            render_pass: self.hdr_render_pass,
            debug_label: Some("dx12-particle-billboard".into()),
            ..PipelineDescriptor::default()
        };
        let particle_pipeline = match self.device.create_pipeline(&particle_descriptor) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 particle PSO creation failed");
                return;
            }
        };
        let mut additive_particle_descriptor = particle_descriptor.clone();
        additive_particle_descriptor.blend_state.mode = Some("additive".into());
        additive_particle_descriptor.debug_label = Some("dx12-particle-billboard-additive".into());
        let particle_additive_pipeline = match self
            .device
            .create_pipeline(&additive_particle_descriptor)
        {
            Ok(pipeline) => pipeline,
            Err(error) => {
                self.device.destroy_pipeline(particle_pipeline);
                tracing::error!(target: "scene_renderer", ?error, "DX12 additive particle PSO creation failed");
                return;
            }
        };
        self.particle_pipeline = Some(particle_pipeline);
        self.particle_additive_pipeline = Some(particle_additive_pipeline);
        let mut oit_particle_descriptor = particle_descriptor.clone();
        oit_particle_descriptor.blend_state.mode = Some("weighted_oit".into());
        oit_particle_descriptor.debug_label = Some("dx12-particle-billboard-oit".into());
        let particle_oit_pipeline = match self.device.create_pipeline(&oit_particle_descriptor) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 OIT particle PSO creation failed");
                return;
            }
        };
        self.particle_oit_pipeline = Some(particle_oit_pipeline);

        let gpu_particle_vs_bytes: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/gpu_particle_vs.dxil"));
        let gpu_particle_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: gpu_particle_vs_bytes.to_vec(),
                source_hash: [8; 32],
                entry_points: vec!["GpuParticleVSMain".into()],
                debug_label: Some("dx12-gpu-particle-vs".into()),
            },
        ) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 GPU-particle vertex shader creation failed");
                return;
            }
        };
        let mut gpu_particle_descriptor = particle_descriptor;
        gpu_particle_descriptor.shader_modules = vec![gpu_particle_vertex_shader, pixel_shader];
        gpu_particle_descriptor.vertex_layout = VertexLayout {
            stride_bytes: 32,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
                VertexAttribute {
                    semantic: "TEXCOORD".into(),
                    format: "float32x2".into(),
                    offset_bytes: 24,
                },
            ],
        };
        gpu_particle_descriptor.debug_label = Some("dx12-gpu-particle-billboard".into());
        let gpu_particle_pipeline = match self.device.create_pipeline(&gpu_particle_descriptor) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 GPU-particle PSO creation failed");
                return;
            }
        };
        gpu_particle_descriptor.blend_state.mode = Some("additive".into());
        gpu_particle_descriptor.debug_label = Some("dx12-gpu-particle-billboard-additive".into());
        let gpu_particle_additive_pipeline = match self
            .device
            .create_pipeline(&gpu_particle_descriptor)
        {
            Ok(pipeline) => pipeline,
            Err(error) => {
                self.device.destroy_pipeline(gpu_particle_pipeline);
                tracing::error!(target: "scene_renderer", ?error, "DX12 additive GPU-particle PSO creation failed");
                return;
            }
        };
        self.gpu_particle_pipeline = Some(gpu_particle_pipeline);
        self.gpu_particle_additive_pipeline = Some(gpu_particle_additive_pipeline);
        gpu_particle_descriptor.blend_state.mode = Some("weighted_oit".into());
        gpu_particle_descriptor.debug_label = Some("dx12-gpu-particle-billboard-oit".into());
        let gpu_particle_oit_pipeline = match self.device.create_pipeline(&gpu_particle_descriptor)
        {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 OIT GPU-particle PSO creation failed");
                return;
            }
        };
        self.gpu_particle_oit_pipeline = Some(gpu_particle_oit_pipeline);

        let skybox_vs = include_bytes!(concat!(env!("OUT_DIR"), "/skybox_vs.dxil"));
        let skybox_ps = include_bytes!(concat!(env!("OUT_DIR"), "/skybox_ps.dxil"));
        let skybox_vertex_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Vertex,
            source_bytes: skybox_vs.to_vec(),
            source_hash: [10; 32],
            entry_points: vec!["SkyboxVSMain".into()],
            debug_label: Some("dx12-skybox-vs".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skybox vertex shader creation failed");
                return;
            }
        };
        let skybox_pixel_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Fragment,
            source_bytes: skybox_ps.to_vec(),
            source_hash: [11; 32],
            entry_points: vec!["SkyboxPSMain".into()],
            debug_label: Some("dx12-skybox-ps".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skybox pixel shader creation failed");
                return;
            }
        };
        let skybox_pipeline = match self.device.create_pipeline(&PipelineDescriptor {
            shader_modules: vec![skybox_vertex_shader, skybox_pixel_shader],
            vertex_layout: VertexLayout::default(),
            pipeline_layout: Some(layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
            ],
            depth_state: render_core::DepthState {
                format: Some(render_core::TextureFormat::Depth32Float),
                write_enabled: false,
                compare: Some("less_equal".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("none".into()),
                front_face: Some("ccw".into()),
            },
            render_pass: self.hdr_render_pass,
            debug_label: Some("dx12-skybox".into()),
            ..PipelineDescriptor::default()
        }) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skybox PSO creation failed");
                return;
            }
        };
        self.skybox_pipeline = Some(skybox_pipeline);

        let tone_map_layout = match self
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 2,
                    offset: 0,
                    size: 128,
                }],
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampled_texture_triple".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampler_triple".into(),
                        },
                    ],
                }],
                debug_label: Some("dx12-tone-map".into()),
            }) {
            Ok(layout) => layout,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 tone-map root signature creation failed");
                return;
            }
        };
        let tone_map_vs = include_bytes!(concat!(env!("OUT_DIR"), "/tone_map_vs.dxil"));
        let tone_map_ps = include_bytes!(concat!(env!("OUT_DIR"), "/tone_map_ps.dxil"));
        let tone_map_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: tone_map_vs.to_vec(),
                source_hash: [5; 32],
                entry_points: vec!["ToneMapVSMain".into()],
                debug_label: Some("dx12-tone-map-vs".into()),
            },
        ) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 tone-map vertex shader creation failed");
                return;
            }
        };
        let tone_map_pixel_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Fragment,
                source_bytes: tone_map_ps.to_vec(),
                source_hash: [6; 32],
                entry_points: vec!["ToneMapPSMain".into()],
                debug_label: Some("dx12-tone-map-ps".into()),
            },
        ) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 tone-map pixel shader creation failed");
                return;
            }
        };
        let tone_map_pipeline = match self.device.create_pipeline(&PipelineDescriptor {
            shader_modules: vec![tone_map_vertex_shader, tone_map_pixel_shader],
            vertex_layout: VertexLayout::default(),
            pipeline_layout: Some(tone_map_layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![render_core::TextureFormat::Bgra8Unorm],
            depth_state: render_core::DepthState {
                format: None,
                write_enabled: false,
                compare: Some("always".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("none".into()),
                front_face: Some("ccw".into()),
            },
            debug_label: Some("dx12-tone-map".into()),
            ..PipelineDescriptor::default()
        }) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 tone-map PSO creation failed");
                return;
            }
        };
        self.tone_map_pipeline_layout = Some(tone_map_layout);
        self.tone_map_pipeline = Some(tone_map_pipeline);

        let ui_layout = match self
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 1,
                    offset: 0,
                    size: 8,
                }],
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampled_texture".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampler".into(),
                        },
                    ],
                }],
                debug_label: Some("dx12-ui".into()),
            }) {
            Ok(layout) => layout,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 UI root signature creation failed");
                return;
            }
        };
        let ui_vs = include_bytes!(concat!(env!("OUT_DIR"), "/ui_vs.dxil"));
        let ui_ps = include_bytes!(concat!(env!("OUT_DIR"), "/ui_ps.dxil"));
        let ui_vertex_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Vertex,
            source_bytes: ui_vs.to_vec(),
            source_hash: [8; 32],
            entry_points: vec!["UiVSMain".into()],
            debug_label: Some("dx12-ui-vs".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 UI vertex shader creation failed");
                return;
            }
        };
        let ui_pixel_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Fragment,
            source_bytes: ui_ps.to_vec(),
            source_hash: [9; 32],
            entry_points: vec!["UiPSMain".into()],
            debug_label: Some("dx12-ui-ps".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 UI pixel shader creation failed");
                return;
            }
        };
        let ui_pipeline = match self.device.create_pipeline(&PipelineDescriptor {
            shader_modules: vec![ui_vertex_shader, ui_pixel_shader],
            vertex_layout: VertexLayout {
                stride_bytes: 32,
                attributes: vec![
                    VertexAttribute {
                        semantic: "POSITION".into(),
                        format: "float32x2".into(),
                        offset_bytes: 0,
                    },
                    VertexAttribute {
                        semantic: "TEXCOORD".into(),
                        format: "float32x2".into(),
                        offset_bytes: 8,
                    },
                    VertexAttribute {
                        semantic: "COLOR".into(),
                        format: "float32x4".into(),
                        offset_bytes: 16,
                    },
                ],
            },
            pipeline_layout: Some(ui_layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![render_core::TextureFormat::Bgra8Unorm],
            depth_state: render_core::DepthState {
                format: None,
                write_enabled: false,
                compare: Some("always".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("none".into()),
                front_face: Some("ccw".into()),
            },
            blend_state: render_core::BlendState {
                mode: Some("alpha".into()),
            },
            debug_label: Some("dx12-ui".into()),
            ..PipelineDescriptor::default()
        }) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 UI PSO creation failed");
                return;
            }
        };
        self.ui_pipeline_layout = Some(ui_layout);
        self.ui_pipeline = Some(ui_pipeline);

        let shadow_texture = match self.device.create_texture(&TextureDescriptor {
            width: 2048,
            height: 2048,
            depth_or_layers: 1,
            mip_levels: 1,
            format: render_core::TextureFormat::Depth32Float,
            usage_flags: TextureUsage(TextureUsage::DEPTH_ATTACHMENT.0 | TextureUsage::SAMPLED.0),
            sample_count: 1,
            debug_label: Some("directional-shadow-map".into()),
        }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow texture creation failed");
                return;
            }
        };
        let shadow_render_pass = match self.device.create_render_pass(
            &render_core::RenderPassDescriptor {
                color_attachments: Vec::new(),
                depth_stencil_format: Some(render_core::TextureFormat::Depth32Float),
                sample_count: 1,
                present_after: false,
                debug_label: Some("directional-shadow-pass".into()),
            },
        ) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow render pass creation failed");
                return;
            }
        };
        let shadow_framebuffer = match self.device.create_framebuffer(
            &render_core::FramebufferDescriptor {
                render_pass: shadow_render_pass,
                color_attachments: Vec::new(),
                depth_stencil_attachment: Some(shadow_texture),
                width: 2048,
                height: 2048,
                debug_label: Some("directional-shadow-framebuffer".into()),
            },
        ) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow framebuffer creation failed");
                return;
            }
        };
        let shadow_layout = match self
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 1,
                    offset: 0,
                    size: 192,
                }],
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![BindGroupLayoutBinding {
                        binding: 1,
                        resource_kind: "uniform_buffer".into(),
                    }],
                }],
                debug_label: Some("directional-shadow".into()),
            }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow root signature creation failed");
                return;
            }
        };
        let shadow_vs_bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shadow_vs.dxil"));
        let shadow_vertex_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Vertex,
            source_bytes: shadow_vs_bytes.to_vec(),
            source_hash: [3; 32],
            entry_points: vec!["ShadowVSMain".into()],
            debug_label: Some("directional_shadow_vs".into()),
        }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow vertex shader creation failed");
                return;
            }
        };
        let skinned_shadow_vs_bytes: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/shadow_skinned_vs.dxil"));
        let skinned_shadow_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: skinned_shadow_vs_bytes.to_vec(),
                source_hash: [4; 32],
                entry_points: vec!["SkinnedShadowVSMain".into()],
                debug_label: Some("directional_shadow_skinned_vs".into()),
            },
        ) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skinned shadow vertex shader creation failed");
                return;
            }
        };
        let shadow_descriptor = PipelineDescriptor {
            shader_modules: vec![shadow_vertex_shader],
            vertex_layout: shadow_static_vertex_layout,
            pipeline_layout: Some(shadow_layout),
            topology: Some("triangle_list".into()),
            render_targets: Vec::new(),
            depth_state: render_core::DepthState {
                format: Some(render_core::TextureFormat::Depth32Float),
                write_enabled: true,
                compare: Some("less".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("back".into()),
                front_face: Some("ccw".into()),
            },
            ..PipelineDescriptor::default()
        };
        let shadow_pipeline = match self.device.create_pipeline(&shadow_descriptor) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow PSO creation failed");
                return;
            }
        };
        let skinned_shadow_descriptor = PipelineDescriptor {
            shader_modules: vec![skinned_shadow_vertex_shader],
            vertex_layout: skinned_descriptor.vertex_layout,
            ..shadow_descriptor
        };
        let skinned_shadow_pipeline = match self.device.create_pipeline(&skinned_shadow_descriptor)
        {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skinned shadow PSO creation failed");
                return;
            }
        };
        self.shadow_texture = Some(shadow_texture);
        self.shadow_render_pass = Some(shadow_render_pass);
        self.shadow_framebuffer = Some(shadow_framebuffer);
        self.shadow_pipeline_layout = Some(shadow_layout);
        self.shadow_pipeline = Some(shadow_pipeline);
        self.skinned_shadow_pipeline = Some(skinned_shadow_pipeline);
    }
}
