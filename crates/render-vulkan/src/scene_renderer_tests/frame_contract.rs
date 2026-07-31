#[test]
fn shadow_index_binding_preserves_uploaded_index_width() {
    assert_eq!(vulkan_index_type(IndexFormat::U16), vk::IndexType::UINT16);
    assert_eq!(vulkan_index_type(IndexFormat::U32), vk::IndexType::UINT32);
}

#[test]
fn device_lifecycle_does_not_allocate_unused_indirect_or_cull_buffers() {
    let device_state = include_str!("../device_impl/mod.rs");
    let construction = include_str!("../device_impl/base/construction.rs");
    let runtime = include_str!("../device_impl/base/runtime.rs");
    let renderer_lifecycle = include_str!("../scene_renderer/lifecycle.rs");
    let device_drop = include_str!("../device_impl/drop.rs");
    let renderer_support = include_str!("../scene_renderer/support.rs");

    for source in [
        device_state,
        construction,
        runtime,
        renderer_lifecycle,
        device_drop,
        renderer_support,
    ] {
        assert!(!source.contains("indirect_draw_buffer"));
        assert!(!source.contains("indirect_draw_alloc"));
        assert!(!source.contains("cull_args_buffer"));
        assert!(!source.contains("cull_args_alloc"));
        assert!(!source.contains("create_indirect_buffers"));
        assert!(!source.contains("destroy_indirect_buffers"));
        assert!(!source.contains("IndirectDrawCommand"));
        assert!(!source.contains("MAX_INDIRECT_DRAWS"));
    }

    let encoder = include_str!("../device_impl/encoder.rs");
    assert!(encoder.contains("fn draw_indexed_indirect("));
    assert!(encoder.contains("cmd_draw_indexed_indirect"));
}

#[test]
fn hdr_builders_keep_local_failure_guards_armed_until_commit() {
    let targets = include_str!("../device_impl/hdr/targets.rs");
    assert!(targets.contains("fn allocate_hdr_target_memory("));
    assert!(targets.contains("fn free_hdr_target_allocation("));
    assert!(
        targets
            .matches("free_hdr_target_allocation(&allocator")
            .count()
            >= 7
    );

    let forward = include_str!("../device_impl/hdr/forward.rs");
    assert!(forward.contains("struct HdrForwardBuildGuard"));
    assert!(forward.contains("impl Drop for HdrForwardBuildGuard"));
    assert!(forward.contains("build.track_pipelines(&partial_pipelines)"));
    assert!(forward.contains("build.framebuffer = Some(fb);"));
    assert!(forward.contains("build.commit();"));

    let cleanup = include_str!("../device_impl/hdr/cleanup.rs");
    assert!(cleanup.contains("free_hdr_target_allocation(&allocator"));
}

#[test]
fn fallback_extraction_stats_count_static_and_skinned_drawables() {
    let mut input = RenderFrameInput::empty(1);
    input.drawables.push(RenderableItem {
        entity: None,
        mesh: AssetId::new("mesh_static"),
        material: AssetId::new("material_static"),
        world_transform: [0.0; 16],
        bounds: engine_renderer::AxisAlignedBox::UNIT,
        render_layer: "default".to_string(),
        cast_shadows: true,
        sort_key: 0,
        radial_vertex_morph: None,
        triplanar_material_mapping: None,
    });
    input.skinned_items.push(SkinnedItem {
        entity: None,
        mesh: AssetId::new("mesh_skinned"),
        material: AssetId::new("material_skinned"),
        skeleton: AssetId::new("skeleton"),
        bone_palette: Vec::new(),
        bone_palette_layout: engine_renderer::BonePaletteLayout::Full4x4 { count: 0 },
        morph_target_set: None,
        morph_weights: Vec::new(),
        world_transform: [0.0; 16],
        bounds: engine_renderer::AxisAlignedBox::UNIT,
        render_layer: "default".to_string(),
        cast_shadows: true,
        sort_key: 1,
    });

    assert_eq!(extraction_stats(&input).visible_drawables, 2);
}

#[test]
fn structured_extraction_stats_are_preserved() {
    let mut input = RenderFrameInput::empty(2);
    input.extraction_stats = Some(engine_renderer::ExtractionStats {
        visible_drawables: 3,
        culled_drawables: 5,
        visible_lights: 2,
        culled_lights: 7,
    });
    let mut frame = FrameStats::default();

    apply_extraction_stats(&mut frame, &input);

    assert_eq!(frame.visible_drawables, 3);
    assert_eq!(frame.culled_drawables, 5);
    assert_eq!(frame.visible_lights, 2);
    assert_eq!(frame.culled_lights, 7);
}

fn ui_batch(texture: Option<&str>, clip_rect: engine_renderer::Rect) -> UiBatch {
    UiBatch {
        canvas_id: "editor".into(),
        z_order: 0,
        clip_rect,
        texture: texture.map(AssetId::new),
        vertices: vec![
            engine_renderer::UiVertex {
                position: [10.0, 20.0],
                uv: [0.0, 0.0],
                color: [255, 128, 0, 255],
            },
            engine_renderer::UiVertex {
                position: [20.0, 20.0],
                uv: [1.0, 0.0],
                color: [255, 128, 0, 255],
            },
            engine_renderer::UiVertex {
                position: [20.0, 30.0],
                uv: [1.0, 1.0],
                color: [255, 128, 0, 255],
            },
        ],
        indices: vec![0, 1, 2],
        material: AssetId::new("ui-material"),
    }
}

#[test]
fn ui_preparation_keeps_batch_order_and_one_draw_per_batch() {
    let clip = engine_renderer::Rect {
        min: [0.0, 0.0],
        max: [200.0, 100.0],
    };
    let batches = vec![ui_batch(Some("first"), clip), ui_batch(None, clip)];

    let prepared = prepare_ui_overlay(&batches, 200, 100).unwrap();

    assert_eq!(prepared.draws.len(), 2);
    assert_eq!(prepared.draws[0].texture_id.as_deref(), Some("first"));
    assert_eq!(prepared.draws[1].texture_id, None);
    assert_eq!(prepared.draws[0].first_vertex, 0);
    assert_eq!(prepared.draws[1].first_vertex, 3);
    assert_eq!(prepared.vertex_bytes.len(), 6 * UI_VERTEX_STRIDE);
}

#[test]
fn empty_ui_preparation_has_no_overlay_draws() {
    let prepared = prepare_ui_overlay(&[], 1280, 720).unwrap();

    assert!(prepared.draws.is_empty());
    assert!(prepared.vertex_bytes.is_empty());
}

#[test]
fn ui_preparation_clamps_fractional_clip_to_the_swapchain() {
    let batch = ui_batch(
        None,
        engine_renderer::Rect {
            min: [-3.4, 8.2],
            max: [500.7, 120.1],
        },
    );

    let prepared = prepare_ui_overlay(&[batch], 320, 100).unwrap();

    assert_eq!(
        prepared.draws[0].scissor,
        UiScissor {
            x: 0,
            y: 8,
            width: 320,
            height: 92,
        }
    );
}

#[test]
fn ui_preparation_rejects_an_out_of_bounds_index() {
    let mut batch = ui_batch(
        None,
        engine_renderer::Rect {
            min: [0.0, 0.0],
            max: [100.0, 100.0],
        },
    );
    batch.indices[2] = 99;

    let error = prepare_ui_overlay(&[batch], 100, 100).unwrap_err();

    assert!(error.contains("index 99"));
    assert!(error.contains("outside 3 vertices"));
}

#[test]
fn missing_ui_texture_check_ignores_textureless_batches() {
    let clip = engine_renderer::Rect {
        min: [0.0, 0.0],
        max: [100.0, 100.0],
    };
    let batches = vec![ui_batch(None, clip), ui_batch(Some("missing"), clip)];

    assert_eq!(
        first_missing_ui_texture(&batches, |id| id == "known"),
        Some("missing")
    );
    assert_eq!(first_missing_ui_texture(&batches, |_| true), None);
}

#[test]
fn ui_fragment_shader_multiplies_texture_and_vertex_color() {
    let source = include_str!("../../shaders/ui_overlay.frag");
    assert!(source.contains("texture(ui_texture, out_uv) * out_color"));
}

#[test]
fn ui_vertex_shader_preserves_top_left_editor_coordinates() {
    let source = include_str!("../../shaders/ui_overlay.vert");
    assert!(source.contains("float y = (in_position.y / pc.screen_size.y) * 2.0 - 1.0;"));
    assert!(!source.contains("float y = -(in_position.y / pc.screen_size.y) * 2.0 + 1.0;"));
}

#[test]
fn skybox_shaders_generate_a_cube_and_sample_the_environment() {
    let vertex = include_str!("../../shaders/skybox.vert");
    let fragment = include_str!("../../shaders/skybox.frag");
    assert!(vertex.contains("CUBE_POSITIONS[gl_VertexIndex]"));
    assert!(vertex.contains("vec4(direction, 0.0)"));
    assert!(fragment.contains("samplerCube u_environment_map"));
    assert!(fragment.contains("texture(u_environment_map"));
}

#[test]
fn vulkan_scene_renderer_rejects_direct_to_swapchain() {
    let diagnostics =
        validate_vulkan_output_mode(engine_renderer::PassGraphOutputMode::DirectToSwapchain)
            .unwrap_err();

    assert_eq!(diagnostics[0].code, "RV0310");
    assert!(diagnostics[0].message.contains("DirectToSwapchain"));
    assert!(
        validate_vulkan_output_mode(engine_renderer::PassGraphOutputMode::HdrThenToneMap).is_ok()
    );
}

#[test]
fn vulkan_scene_renderer_accepts_msaa_and_fails_closed_for_invalid_view_options() {
    let mut input = frame_with_custom_pass("custom_post");
    assert!(validate_vulkan_frame_contract(&input).is_ok());

    input.render_options.msaa_samples = 4;
    input.views[0].msaa_samples = 4;
    assert!(validate_vulkan_frame_contract(&input).is_ok());
    input.views[0].msaa_samples = 2;
    assert_eq!(
        validate_vulkan_frame_contract(&input).unwrap_err()[0].code,
        "RV0317"
    );
    input.render_options.msaa_samples = 1;
    input.views[0].msaa_samples = 1;

    let embedded = engine_renderer::Rect {
        min: [0.25, 0.125],
        max: [0.75, 0.875],
    };
    input.views[0].viewport = embedded;
    input.views[0].viewport_rect_normalized = embedded;
    assert!(validate_vulkan_frame_contract(&input).is_ok());

    input.views[0].viewport_rect_normalized.max = [0.5, 1.0];
    assert_eq!(
        validate_vulkan_frame_contract(&input).unwrap_err()[0].code,
        "RV0318"
    );
    input.views[0].viewport = engine_renderer::Rect::FULL;
    input.views[0].viewport_rect_normalized = engine_renderer::Rect::FULL;

    input.views[0].clear_flags = engine_renderer::ClearFlags::Skybox;
    assert!(validate_vulkan_frame_contract(&input).is_ok());

    input.views[0].clear_flags = engine_renderer::ClearFlags::Nothing;
    assert_eq!(
        validate_vulkan_frame_contract(&input).unwrap_err()[0].code,
        "RV0319"
    );
}

#[test]
fn normalized_scene_viewport_maps_to_matching_vulkan_viewport_and_scissor() {
    let mapped = vulkan_viewport_rect(
        engine_renderer::Rect {
            min: [0.25, 0.1],
            max: [0.75, 0.9],
        },
        1600,
        900,
    )
    .unwrap();
    assert_eq!(mapped.viewport.x, 400.0);
    assert_eq!(mapped.viewport.y, 90.0);
    assert_eq!(mapped.viewport.width, 800.0);
    assert_eq!(mapped.viewport.height, 720.0);
    assert_eq!(mapped.scissor.offset.x, 400);
    assert_eq!(mapped.scissor.offset.y, 90);
    assert_eq!(mapped.scissor.extent.width, 800);
    assert_eq!(mapped.scissor.extent.height, 720);

    let fractional = vulkan_viewport_rect(
        engine_renderer::Rect {
            min: [0.1, 0.1],
            max: [0.2, 0.2],
        },
        17,
        11,
    )
    .unwrap();
    assert_eq!(fractional.scissor.offset.x, 1);
    assert_eq!(fractional.scissor.offset.y, 1);
    assert_eq!(fractional.scissor.extent.width, 3);
    assert_eq!(fractional.scissor.extent.height, 2);

    assert!(vulkan_viewport_rect(engine_renderer::Rect::FULL, 0, 900).is_err());
}

#[test]
fn reflection_probe_selection_prefers_priority_then_distance() {
    let global = AssetId::new("environment.global");
    let near = AssetId::new("environment.near");
    let priority = AssetId::new("environment.priority");
    let settings = engine_renderer::EnvironmentSettings {
        environment_map: Some(global.clone()),
        reflection_probes: vec![
            engine_renderer::ReflectionProbe {
                entity: None,
                environment_map: near.clone(),
                position: [0.0; 3],
                half_extents: [5.0; 3],
                blend_distance: 1.0,
                priority: 0,
            },
            engine_renderer::ReflectionProbe {
                entity: None,
                environment_map: priority.clone(),
                position: [2.0, 0.0, 0.0],
                half_extents: [5.0; 3],
                blend_distance: 0.0,
                priority: 10,
            },
        ],
        ..engine_renderer::EnvironmentSettings::default()
    };

    assert_eq!(
        select_environment_map(&settings, Vec3::ZERO),
        Some(&priority)
    );
    assert_eq!(
        select_environment_map(&settings, Vec3::splat(100.0)),
        Some(&global)
    );
}
