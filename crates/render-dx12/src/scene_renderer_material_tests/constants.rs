fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[test]
fn material_constants_match_hlsl_root_constant_layout() {
    let constants = material_constants([0.1, 0.2, 0.3, 0.4], 0.5, 0.6, 0.7);
    let values: Vec<f32> = (0..8)
        .map(|index| read_f32(&constants, index * 4))
        .collect();
    assert_eq!(values, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.0]);
}

#[test]
fn short_material_binding_preserves_defaults_for_missing_values() {
    let constants =
        material_constants_from_bytes(&0.25_f32.to_ne_bytes(), false, &Transparency::Opaque, false);
    assert_eq!(read_f32(&constants, 0), 0.25);
    assert_eq!(read_f32(&constants, 16), 0.0);
    assert_eq!(read_f32(&constants, 20), 1.0);
    assert_eq!(read_f32(&constants, 24), 1.0);
    assert_eq!(
        emissive_constants_from_bytes(&0.25_f32.to_ne_bytes(), 0),
        default_emissive_constants()
    );
}

#[test]
fn emissive_constants_match_hlsl_tail_layout() {
    let constants = emissive_constants([0.2, 0.4, 0.6], 30);
    assert_eq!(read_f32(&constants, 0), 0.2);
    assert_eq!(read_f32(&constants, 4), 0.4);
    assert_eq!(read_f32(&constants, 8), 0.6);
    assert_eq!(read_f32(&constants, 12), 30.0);

    let mut binding = vec![0_u8; 48];
    binding[32..48].copy_from_slice(&constants);
    assert_eq!(emissive_constants_from_bytes(&binding, 30), constants);
}

#[test]
fn radial_geomorph_matches_vulkan_draw_constant_contract() {
    assert_eq!(radial_morph_constants(None), [0; 32]);
    let constants = radial_morph_constants(Some(&engine_renderer::RadialVertexMorph {
        factor: 0.75,
        delta_scale: 24.0,
        local_origin: [10.0, -20.0, 30.0],
    }));
    assert_eq!(read_f32(&constants, 0), 0.75);
    assert_eq!(read_f32(&constants, 4), 24.0);
    assert_eq!(read_f32(&constants, 8), 1.0);
    assert_eq!(read_f32(&constants, 16), 10.0);
    assert_eq!(read_f32(&constants, 20), -20.0);
    assert_eq!(read_f32(&constants, 24), 30.0);

    let shader = include_str!("../shaders.hlsl");
    assert!(shader.contains("float3 radial_geomorph_position"));
    assert!(shader.contains("float4 radial_morph"));
    assert!(shader.contains("float4 morph_origin"));
    assert!(shader.contains(
        "float3 local_position = radial_geomorph_position(input.position, input.normal);"
    ));
    assert_eq!(
        shader
            .matches("radial_geomorph_position(input.position, input.normal)")
            .count(),
        2,
        "forward and shadow static vertices must use the same radial morph"
    );

    let forward = include_str!("../scene_renderer/forward.rs");
    let shadow = include_str!("../scene_renderer/shadow.rs");
    assert!(forward.contains("vertex_draw_binding(drawable_index)"));
    assert!(shadow.contains("vertex_draw_binding(drawable_index)"));
    assert!(forward.contains("bind_uniform_buffer_offset("));
    assert!(shadow.contains("bind_uniform_buffer_offset("));
}

#[test]
fn triplanar_mapping_matches_vulkan_draw_constant_contract() {
    assert_eq!(triplanar_mapping_constants(None), [0; 32]);
    let mapping = engine_renderer::TriplanarMaterialMapping {
        local_origin: [10.0, -20.0, 30.0],
        meters_per_tile: 8.0,
        blend_sharpness: 6.0,
    };
    let constants = vertex_draw_constants(None, Some(&mapping));
    assert_eq!(read_f32(&constants, 32), 1.0);
    assert_eq!(read_f32(&constants, 36), 0.125);
    assert_eq!(read_f32(&constants, 40), 6.0);
    assert_eq!(read_f32(&constants, 48), 10.0);
    assert_eq!(read_f32(&constants, 52), -20.0);
    assert_eq!(read_f32(&constants, 56), 30.0);

    let shader = include_str!("../shaders.hlsl");
    assert!(shader.contains("float4 material_mapping"));
    assert!(shader.contains("float4 mapping_origin"));
    assert!(shader.contains("sample_triplanar_normal("));
    for texture in [
        "base_color_map",
        "normal_map",
        "metallic_roughness_map",
        "occlusion_map",
        "emissive_map",
    ] {
        assert!(
            shader.contains(texture),
            "{texture} must remain part of the projected PBR path"
        );
    }
}

#[test]
fn vertex_draw_arena_uses_one_aligned_record_per_drawable() {
    let morph = engine_renderer::RadialVertexMorph {
        factor: 0.5,
        delta_scale: 10.0,
        local_origin: [1.0, 2.0, 3.0],
    };
    let mapping = engine_renderer::TriplanarMaterialMapping {
        local_origin: [4.0, 5.0, 6.0],
        meters_per_tile: 8.0,
        blend_sharpness: 4.0,
    };
    let arena =
        vertex_draw_arena_constants([(Some(&morph), None), (None, Some(&mapping)), (None, None)]);

    assert_eq!(arena.len(), 3 * VERTEX_DRAW_CONSTANT_STRIDE);
    assert_eq!(vertex_draw_constant_offset(0), Some(0));
    assert_eq!(vertex_draw_constant_offset(1), Some(256));
    assert_eq!(vertex_draw_constant_offset(2), Some(512));
    assert_eq!(read_f32(&arena, 0), 0.5);
    assert_eq!(read_f32(&arena, 256 + 32), 1.0);
    assert_eq!(read_f32(&arena, 256 + 36), 0.125);
    assert_eq!(&arena[512..768], &[0; 256]);
}

#[test]
fn dx12_cbv_offsets_require_a_complete_aligned_record() {
    assert!(crate::encoder::valid_constant_buffer_offset(256, 0));
    assert!(crate::encoder::valid_constant_buffer_offset(768, 512));
    assert!(!crate::encoder::valid_constant_buffer_offset(255, 0));
    assert!(!crate::encoder::valid_constant_buffer_offset(768, 1));
    assert!(!crate::encoder::valid_constant_buffer_offset(768, 768));
    assert!(!crate::encoder::valid_constant_buffer_offset(
        u64::MAX,
        u64::MAX - 255
    ));
}

#[test]
fn vertex_draw_arena_is_prepared_after_gpu_wait_and_aborted_on_failure() {
    let backend = include_str!("../scene_renderer/backend.rs");
    let begin = backend
        .find("self.device.begin_frame(self.swapchain)")
        .expect("DX12 device frame begin");
    let prepare = backend
        .find("self.prepare_vertex_draw_arena(input)")
        .expect("vertex-draw arena preparation");
    let activate = backend
        .find("self.active_frame = Some(Dx12FrameState")
        .expect("active frame installation");
    assert!(begin < prepare && prepare < activate);
    assert!(
        backend[prepare..activate].contains("self.device.abort_frame(encoder)"),
        "an arena failure after command-list reset must close the unsubmitted frame"
    );
}

#[test]
fn skinned_vertex_draw_keeps_bones_after_the_zeroed_projection_header() {
    let palette = [glam::Mat4::IDENTITY.to_cols_array()];
    let constants = bone_palette_constants(&palette);

    assert_eq!(constants.len(), 4_352);
    assert_eq!(&constants[..64], &[0; 64]);
    assert_eq!(read_f32(&constants, 64), 1.0);
    assert_eq!(read_f32(&constants, 64 + 5 * 4), 1.0);
    assert_eq!(read_f32(&constants, 64 + 10 * 4), 1.0);
    assert_eq!(read_f32(&constants, 64 + 15 * 4), 1.0);
}

#[test]
fn advanced_constants_preserve_quantized_parameters() {
    let parameters = engine_renderer::AdvancedMaterialParameters {
        clearcoat: 0.25,
        clearcoat_roughness: 0.5,
        subsurface: 0.75,
        anisotropy: -0.25,
        subsurface_color: [1.0, 0.5, 0.0],
        sheen_color: [0.0, 0.25, 1.0],
        rim_color: [0.1, 0.2, 0.3],
        rim_power: 6.0,
    };
    let constants = advanced_constants(parameters);
    let packed_weights = u32::from_ne_bytes(constants[0..4].try_into().unwrap());
    assert!(((packed_weights & 255) as f32 / 255.0 - 0.25).abs() < 0.003);
    assert!((((packed_weights >> 8) & 255) as f32 / 255.0 - 0.5).abs() < 0.003);
    assert!((((packed_weights >> 16) & 255) as f32 / 255.0 - 0.75).abs() < 0.003);
    let anisotropy = ((packed_weights >> 24) & 255) as f32 / 255.0 * 2.0 - 1.0;
    assert!((anisotropy + 0.25).abs() < 0.005);
    let packed_subsurface = u32::from_ne_bytes(constants[4..8].try_into().unwrap());
    assert_eq!(packed_subsurface & 255, 255);
    assert!((((packed_subsurface >> 8) & 255) as f32 / 255.0 - 0.5).abs() < 0.003);
}

#[test]
fn portable_material_bytes_feed_dx12_advanced_constants() {
    let mut bytes = vec![0_u8; 112];
    for (offset, value) in [
        (48, 0.2_f32),
        (52, 0.3),
        (56, 0.4),
        (60, -0.5),
        (64, 1.0),
        (68, 0.5),
        (72, 0.0),
        (80, 0.1),
        (84, 0.2),
        (88, 0.3),
        (96, 0.4),
        (100, 0.5),
        (104, 0.6),
        (108, 4.0),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
    }
    let constants = advanced_constants_from_bytes(&bytes);
    let packed_weights = u32::from_ne_bytes(constants[0..4].try_into().unwrap());
    assert!(((packed_weights & 255) as f32 / 255.0 - 0.2).abs() < 0.003);
    let anisotropy = ((packed_weights >> 24) & 255) as f32 / 255.0 * 2.0 - 1.0;
    assert!((anisotropy + 0.5).abs() < 0.005);
}

#[test]
fn tone_map_constants_match_portable_post_process_contract() {
    let mut input = shadow_input([0.0, -1.0, 0.0]);
    input.render_options.exposure_ev100 =
        Some(engine_renderer::backend_shared::ARTISTIC_LIGHTING_REFERENCE_EV100 + 2.0);
    input.render_options.post_process.bloom.enabled = true;
    input.render_options.post_process.color_grading.enabled = true;
    input.render_options.post_process.vignette.enabled = true;
    input.render_options.post_process.planetary_lens.enabled = true;
    let constants = tone_map_constants(&input).expect("valid settings");
    assert_eq!(u32::from_ne_bytes(constants[0..4].try_into().unwrap()), 0);
    assert_eq!(read_f32(&constants, 4), 0.25);
    assert_eq!(
        u32::from_ne_bytes(constants[12..16].try_into().unwrap()),
        0b1_0111
    );
    assert_eq!(
        read_f32(&constants, 16),
        input.render_options.post_process.bloom.threshold
    );
    assert_eq!(
        read_f32(&constants, 112),
        input.render_options.post_process.vignette.intensity
    );
    assert_eq!(
        read_f32(&constants, 52),
        input
            .render_options
            .post_process
            .planetary_lens
            .barrel_distortion
    );
    assert_eq!(
        read_f32(&constants, 124),
        input
            .render_options
            .post_process
            .planetary_lens
            .chromatic_aberration
    );
    input.render_options.transparency_mode = engine_renderer::TransparencyMode::WeightedBlendedOit;
    let oit_constants = tone_map_constants(&input).expect("valid OIT settings");
    assert_eq!(
        u32::from_ne_bytes(oit_constants[12..16].try_into().unwrap()),
        0b1_1111
    );
    let shader = include_str!("../tone_map.hlsl");
    assert!(shader.contains("EFFECT_PLANETARY_LENS"));
    assert!(shader.contains("planetary_lens_uv"));
    assert!(shader.contains("resolved_planetary_lens"));
}

#[test]
fn material_texture_flags_follow_portable_slot_order() {
    let texture_ids = [
        None,
        Some("normal".to_string()),
        Some("metallic-roughness".to_string()),
        Some("occlusion".to_string()),
        Some("emissive".to_string()),
    ];
    assert_eq!(material_texture_flags_from_ids(&texture_ids), 30);
}

#[test]
fn material_flags_encode_texture_mask_cutoff_and_surface_variant() {
    assert_eq!(material_surface_flags(false, &Transparency::Opaque), 0.0);
    assert_eq!(material_surface_flags(true, &Transparency::Blend), 1.0);
    assert_eq!(
        material_surface_flags(false, &Transparency::Masked { cutoff: 0.4 }),
        2.2
    );
    assert_eq!(
        material_surface_flags(true, &Transparency::Masked { cutoff: 0.4 }),
        3.2
    );
    assert_eq!(
        surface_variant_index(&Transparency::Opaque, false, false),
        0
    );
    assert_eq!(surface_variant_index(&Transparency::Opaque, true, false), 1);
    assert_eq!(surface_variant_index(&Transparency::Blend, false, false), 2);
    assert_eq!(surface_variant_index(&Transparency::Blend, true, false), 3);
    assert_eq!(
        surface_variant_index(&Transparency::Additive, false, false),
        4
    );
    assert_eq!(
        surface_variant_index(&Transparency::Additive, true, false),
        5
    );
    assert_eq!(surface_variant_index(&Transparency::Blend, false, true), 6);
    assert_eq!(surface_variant_index(&Transparency::Blend, true, true), 7);
    let weighted = material_constants_from_bytes(&[], true, &Transparency::Blend, true);
    assert_eq!(read_f32(&weighted, 28), 9.0);
}
