fn shadow_input(direction: [f32; 3]) -> RenderFrameInput {
    let mut input = RenderFrameInput::empty(0);
    input.views.push(RenderView {
        view_id: 1,
        camera_entity: None,
        viewport: Rect::FULL,
        viewport_rect_normalized: Rect::FULL,
        view_matrix: Mat4::look_at_rh(
            glam::Vec3::new(0.0, 2.0, 5.0),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        )
        .to_cols_array(),
        projection_matrix: Mat4::perspective_rh(60_f32.to_radians(), 16.0 / 9.0, 0.1, 100.0)
            .to_cols_array(),
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    });
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Directional,
        color: [1.0; 3],
        intensity: 1.0,
        range: 0.0,
        position: [0.0; 3],
        direction,
        spot_angles: None,
        shadow_mode: ShadowMode::Soft,
    });
    input
}

#[test]
fn directional_shadow_fit_uses_camera_and_light() {
    let data = Dx12SceneRenderer::directional_shadow_frame_data(&shadow_input([0.4, -1.0, 0.2]))
        .expect("valid shadow fit")
        .expect("shadow light");
    assert!(data
        .light_view_projection
        .to_cols_array()
        .iter()
        .all(|value| value.is_finite()));
    assert!(data.soft);
    assert!((data.light_direction_to_surface.length() - 1.0).abs() < 1.0e-5);
}

#[test]
fn directional_shadow_fit_rejects_zero_light_direction() {
    let diagnostics =
        Dx12SceneRenderer::directional_shadow_frame_data(&shadow_input([0.0, 0.0, 0.0]))
            .expect_err("zero light direction must fail");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "DX1248"));
}

#[test]
fn dx12_frame_options_fail_closed_instead_of_being_ignored() {
    let mut input = shadow_input([0.0, -1.0, 0.0]);
    input.render_options.pass_graph_config.output_mode =
        engine_renderer::PassGraphOutputMode::HdrThenToneMap;
    assert!(validate_dx12_frame_contract(&input).is_ok());

    input.views[0].msaa_samples = 4;
    assert_eq!(
        validate_dx12_frame_contract(&input).unwrap_err()[0].code,
        "DX1249"
    );
    input.views[0].msaa_samples = 1;
    input.views[0].viewport.max = [0.5, 1.0];
    assert_eq!(
        validate_dx12_frame_contract(&input).unwrap_err()[0].code,
        "DX1250"
    );
    input.views[0].viewport = Rect::FULL;
    input.render_options.pass_graph_config.output_mode =
        engine_renderer::PassGraphOutputMode::DirectToSwapchain;
    assert_eq!(
        validate_dx12_frame_contract(&input).unwrap_err()[0].code,
        "DX1247"
    );
}

#[test]
fn dx12_scene_shader_keeps_linear_hdr_output() {
    let source = include_str!("../shaders.hlsl");
    assert!(source.contains("output.hdr = float4(color, sampled_base_color.a)"));
    assert!(source.contains("output.oit_accumulation"));
    assert!(source.contains("output.oit_optical_depth"));
    assert!(!source.contains("linear_to_srgb(color)"));
}

#[test]
fn dx12_scene_creates_forward_and_shadow_pipelines() {
    let adapter = DirectX12Backend::new()
        .enumerate_adapters()
        .expect("adapter enumeration")
        .into_iter()
        .next()
        .expect("DX12 adapter");
    let descriptor = DeviceDescriptor {
        required_limits: adapter.capabilities.limits.clone(),
        adapter,
        required_features: Vec::new(),
        debug_label: Some("scene-pipeline-smoke".into()),
        validation_mode: ValidationMode::Standard,
    };
    let device = Dx12Device::create(&descriptor).expect("DX12 device");
    let mut renderer =
        Dx12SceneRenderer::new(device, SwapchainHandle::new(u32::MAX, u32::MAX), 1280, 720);
    renderer.ensure_pipeline();
    assert!(renderer.pipeline.is_some());
    assert!(renderer.skinned_pipeline.is_some());
    assert!(renderer.shadow_pipeline.is_some());
    assert!(renderer.skinned_shadow_pipeline.is_some());
    assert!(renderer.shadow_texture.is_some());
    assert!(renderer.shadow_framebuffer.is_some());
    assert!(renderer.hdr_texture.is_some());
    assert!(renderer.oit_accum_texture.is_some());
    assert!(renderer.oit_optical_depth_texture.is_some());
    assert!(renderer.hdr_framebuffer.is_some());
    assert!(renderer.oit_pipeline.is_some());
    assert!(renderer.gpu_particle_oit_pipeline.is_some());
    assert!(renderer.tone_map_pipeline.is_some());
    assert!(renderer.fallback_environment.is_some());
    assert!(renderer.ui_pipeline.is_some());
    assert!(renderer.fallback_ui_texture.is_some());
}

#[test]
fn dx12_vertex_draw_arena_reuses_capacity_and_grows_once() {
    let adapter = DirectX12Backend::new()
        .enumerate_adapters()
        .expect("adapter enumeration")
        .into_iter()
        .next()
        .expect("DX12 adapter");
    let descriptor = DeviceDescriptor {
        required_limits: adapter.capabilities.limits.clone(),
        adapter,
        required_features: Vec::new(),
        debug_label: Some("vertex-draw-arena-smoke".into()),
        validation_mode: ValidationMode::Standard,
    };
    let device = Dx12Device::create(&descriptor).expect("DX12 device");
    let mut renderer =
        Dx12SceneRenderer::new(device, SwapchainHandle::new(u32::MAX, u32::MAX), 64, 64);
    let drawable = |factor| engine_renderer::RenderableItem {
        entity: None,
        mesh: engine_renderer::AssetId::new("mesh"),
        material: engine_renderer::AssetId::new("material"),
        world_transform: engine_renderer::IDENTITY_MAT4,
        bounds: engine_renderer::AxisAlignedBox::UNIT,
        render_layer: "Default".to_string(),
        cast_shadows: true,
        sort_key: 0,
        radial_vertex_morph: Some(engine_renderer::RadialVertexMorph {
            factor,
            delta_scale: 10.0,
            local_origin: [0.0; 3],
        }),
        triplanar_material_mapping: None,
    };
    let mut input = RenderFrameInput::empty(0);
    input.drawables.push(drawable(0.25));

    let initial = renderer
        .prepare_vertex_draw_arena(&input)
        .expect("initial arena")
        .expect("one drawable needs an arena");
    assert_eq!(
        renderer.vertex_draw_buffer.as_ref().unwrap().capacity,
        VERTEX_DRAW_CONSTANT_STRIDE
    );

    input.drawables[0]
        .radial_vertex_morph
        .as_mut()
        .unwrap()
        .factor = 0.75;
    let reused = renderer
        .prepare_vertex_draw_arena(&input)
        .expect("updated arena")
        .expect("one drawable needs an arena");
    assert_eq!(reused, initial);
    assert_eq!(
        read_f32(&renderer.vertex_draw_buffer.as_ref().unwrap().bytes, 0),
        0.75
    );

    input.drawables = vec![drawable(0.1), drawable(0.2), drawable(0.3)];
    let grown = renderer
        .prepare_vertex_draw_arena(&input)
        .expect("grown arena")
        .expect("three drawables need an arena");
    assert_ne!(grown, initial);
    assert_eq!(
        renderer.vertex_draw_buffer.as_ref().unwrap().capacity,
        4 * VERTEX_DRAW_CONSTANT_STRIDE
    );

    input.drawables.truncate(2);
    let reused_after_shrink = renderer
        .prepare_vertex_draw_arena(&input)
        .expect("shrunk arena")
        .expect("two drawables need an arena");
    assert_eq!(reused_after_shrink, grown);
}

#[test]
fn dx12_uploads_hdr_cubemap_and_tracks_revision() {
    let adapter = DirectX12Backend::new()
        .enumerate_adapters()
        .expect("adapter enumeration")
        .into_iter()
        .next()
        .expect("DX12 adapter");
    let descriptor = DeviceDescriptor {
        required_limits: adapter.capabilities.limits.clone(),
        adapter,
        required_features: Vec::new(),
        debug_label: Some("environment-upload-smoke".into()),
        validation_mode: ValidationMode::Standard,
    };
    let device = Dx12Device::create(&descriptor).expect("DX12 device");
    let mut renderer =
        Dx12SceneRenderer::new(device, SwapchainHandle::new(u32::MAX, u32::MAX), 64, 64);
    let one = 0x3c00_u16.to_le_bytes();
    let pixel = [
        one[0], one[1], one[0], one[1], one[0], one[1], one[0], one[1],
    ];
    let receipt = renderer
        .upload_environment_map(EnvironmentMapUpload {
            environment_id: engine_renderer::AssetId::new("sky"),
            format: engine_renderer::EnvironmentMapFormat::Rgba16Float,
            mip_levels: vec![engine_renderer::EnvironmentCubeMip {
                face_size: 1,
                faces: vec![pixel.to_vec(); 6],
            }],
            content_hash: [7; 32],
        })
        .expect("environment upload");
    assert_eq!(receipt.revision, 1);
    assert_eq!(renderer.environments["sky"].mip_count, 1);
}

#[test]
fn dx12_applies_morphs_before_skinning_and_uploads_particle_instances() {
    let adapter = DirectX12Backend::new()
        .enumerate_adapters()
        .expect("adapter enumeration")
        .into_iter()
        .next()
        .expect("DX12 adapter");
    let descriptor = DeviceDescriptor {
        required_limits: adapter.capabilities.limits.clone(),
        adapter,
        required_features: Vec::new(),
        debug_label: Some("dynamic-character-vfx-smoke".into()),
        validation_mode: ValidationMode::Standard,
    };
    let device = Dx12Device::create(&descriptor).expect("DX12 device");
    let mut renderer =
        Dx12SceneRenderer::new(device, SwapchainHandle::new(u32::MAX, u32::MAX), 64, 64);
    let mut vertex_bytes = vec![0_u8; 64];
    for (offset, value) in [
        (0, 1.0_f32),
        (4, 0.0),
        (8, 0.0),
        (12, 0.0),
        (16, 1.0),
        (20, 0.0),
        (48, 1.0),
    ] {
        vertex_bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
    }
    renderer
        .upload_mesh(MeshUpload {
            mesh_id: engine_renderer::AssetId::new("face"),
            vertex_format: RendererMeshVertexFormat::Skinned64,
            vertex_count: 1,
            vertex_bytes,
            index_format: engine_renderer::IndexFormat::U16,
            index_count: 3,
            index_bytes: vec![0; 6],
            bounds: engine_renderer::AxisAlignedBox::UNIT,
            content_hash: [2; 32],
        })
        .expect("mesh upload");
    let target_set_id = engine_renderer::AssetId::new("face.morphs");
    renderer
        .upload_morph_target_set(MorphTargetSetUpload {
            target_set_id: target_set_id.clone(),
            vertex_count: 1,
            targets: vec![engine_renderer::MorphTarget {
                name: "smile".into(),
                position_deltas: vec![[1.0, 0.0, 0.0]],
                normal_deltas: vec![[0.0, 0.0, 1.0]],
            }],
            content_hash: [3; 32],
        })
        .expect("morph upload");
    let mesh = renderer.meshes["face"].clone();
    renderer
        .prepare_morphed_vertex_buffer("face:actor", &mesh, &target_set_id, &[0.5])
        .expect("morph deformation");
    let morphed = &renderer.morphed_vertex_buffers["face:actor"].bytes;
    assert!((read_f32(morphed, 0) - 1.5).abs() < 1.0e-6);
    assert!((read_f32(morphed, 16) - 0.8944272).abs() < 1.0e-5);
    assert!((read_f32(morphed, 20) - 0.4472136).abs() < 1.0e-5);

    let instances = [
        engine_renderer::ParticleInstance {
            position: [1.0, 2.0, 3.0],
            size: 0.5,
            rotation_radians: 0.25,
            normalized_age: 0.75,
            color: [255, 128, 64, 32],
        },
        engine_renderer::ParticleInstance {
            position: [4.0, 5.0, 6.0],
            size: 1.5,
            rotation_radians: 0.5,
            normalized_age: 0.25,
            color: [10, 20, 30, 40],
        },
    ];
    renderer
        .prepare_particle_instance_buffer("sparks", &instances)
        .expect("particle stream")
        .expect("non-empty stream");
    assert_eq!(renderer.particle_instance_buffers["sparks"].bytes.len(), 64);
    assert_eq!(
        read_f32(&renderer.particle_instance_buffers["sparks"].bytes, 12),
        0.5
    );
    assert_eq!(
        &renderer.particle_instance_buffers["sparks"].bytes[24..28],
        &[255, 128, 64, 32]
    );
}

#[test]
fn dx12_ui_preparation_expands_indices_and_preserves_clip_order() {
    let vertices = vec![
        engine_renderer::UiVertex {
            position: [10.0, 20.0],
            uv: [0.0, 0.0],
            color: [255, 0, 0, 255],
        },
        engine_renderer::UiVertex {
            position: [30.0, 20.0],
            uv: [1.0, 0.0],
            color: [0, 255, 0, 255],
        },
        engine_renderer::UiVertex {
            position: [30.0, 40.0],
            uv: [1.0, 1.0],
            color: [0, 0, 255, 255],
        },
    ];
    let prepared = prepare_dx12_ui(
        &[engine_renderer::UiBatch {
            canvas_id: "hud".into(),
            z_order: 0,
            clip_rect: engine_renderer::Rect {
                min: [5.2, 6.4],
                max: [50.1, 60.8],
            },
            texture: None,
            vertices,
            indices: vec![0, 1, 2],
            material: engine_renderer::AssetId::new("ui"),
        }],
        100,
        80,
    )
    .expect("valid UI");
    assert_eq!(prepared.vertex_bytes.len(), 3 * 32);
    assert_eq!(prepared.draws.len(), 1);
    assert_eq!(prepared.draws[0].first_vertex, 0);
    assert_eq!(prepared.draws[0].vertex_count, 3);
    assert_eq!(
        prepared.draws[0].scissor,
        engine_renderer::backend_shared::PixelRect {
            x: 5,
            y: 6,
            width: 46,
            height: 55,
        }
    );
    assert_eq!(read_f32(&prepared.vertex_bytes, 16), 1.0);
}
