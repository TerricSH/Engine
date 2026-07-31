#[test]
fn tone_map_push_constants_cover_modes_exposure_and_target_encoding() {
    let aces = tone_map_push_constants(
        engine_renderer::ToneMapping::Aces,
        None,
        engine_renderer::PostProcessSettings::default(),
        vk::Format::B8G8R8A8_SRGB,
        false,
    )
    .unwrap();
    assert_eq!(aces.mode, TONE_MAP_MODE_ACES);
    assert_eq!(aces.exposure, 1.0);
    assert_eq!(aces.output_is_srgb, 1);

    let reinhard = tone_map_push_constants(
        engine_renderer::ToneMapping::Reinhard,
        Some(ARTISTIC_LIGHTING_REFERENCE_EV100 + 2.0),
        engine_renderer::PostProcessSettings::default(),
        vk::Format::B8G8R8A8_UNORM,
        false,
    )
    .unwrap();
    assert_eq!(reinhard.mode, TONE_MAP_MODE_REINHARD);
    assert_eq!(reinhard.exposure, 0.25);
    assert_eq!(reinhard.output_is_srgb, 0);

    let identity = tone_map_push_constants(
        engine_renderer::ToneMapping::None,
        Some(ARTISTIC_LIGHTING_REFERENCE_EV100 - 1.0),
        engine_renderer::PostProcessSettings::default(),
        vk::Format::R8G8B8A8_SRGB,
        false,
    )
    .unwrap();
    assert_eq!(identity.mode, TONE_MAP_MODE_NONE);
    assert_eq!(identity.exposure, 2.0);
    assert_eq!(identity.output_is_srgb, 1);

    let bytes = identity.to_bytes();
    assert_eq!(bytes.len(), ToneMapPushConstants::SIZE);
    assert_eq!(
        u32::from_ne_bytes(bytes[0..4].try_into().unwrap()),
        TONE_MAP_MODE_NONE
    );
    assert_eq!(f32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 2.0);
    assert_eq!(u32::from_ne_bytes(bytes[8..12].try_into().unwrap()), 1);
    assert_eq!(u32::from_ne_bytes(bytes[12..16].try_into().unwrap()), 0);

    let default_physical_camera = tone_map_push_constants(
        engine_renderer::ToneMapping::Aces,
        Some((16.0_f32 * 16.0 / (1.0 / 60.0)).log2()),
        engine_renderer::PostProcessSettings::default(),
        vk::Format::B8G8R8A8_SRGB,
        false,
    )
    .unwrap();
    assert!((default_physical_camera.exposure - 1.0).abs() < 1.0e-5);

    let mut effects = engine_renderer::PostProcessSettings::default();
    effects.bloom.enabled = true;
    effects.color_grading.enabled = true;
    effects.vignette.enabled = true;
    effects.planetary_lens.enabled = true;
    let configured = tone_map_push_constants(
        engine_renderer::ToneMapping::Aces,
        None,
        effects,
        vk::Format::B8G8R8A8_SRGB,
        false,
    )
    .unwrap();
    assert_eq!(configured.effect_flags, 0b1_0111);
    assert_eq!(configured.bloom[0], effects.bloom.threshold);
    assert_eq!(configured.vignette[0], effects.vignette.intensity);
    assert_eq!(
        configured.contrast[1],
        effects.planetary_lens.barrel_distortion
    );
    assert_eq!(
        configured.contrast[2],
        effects.planetary_lens.horizon_curvature
    );
    assert_eq!(
        configured.contrast[3],
        effects.planetary_lens.atmosphere_intensity
    );
    assert_eq!(
        configured.vignette[3],
        effects.planetary_lens.chromatic_aberration
    );
}

#[test]
fn tone_map_push_constants_reject_non_finite_exposure() {
    for exposure in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let error = tone_map_push_constants(
            engine_renderer::ToneMapping::Aces,
            Some(exposure),
            engine_renderer::PostProcessSettings::default(),
            vk::Format::B8G8R8A8_SRGB,
            false,
        )
        .unwrap_err();
        assert!(error.contains("must be finite"));
    }

    let overflow = tone_map_push_constants(
        engine_renderer::ToneMapping::Aces,
        Some(-1000.0),
        engine_renderer::PostProcessSettings::default(),
        vk::Format::B8G8R8A8_SRGB,
        false,
    )
    .unwrap_err();
    assert!(overflow.contains("non-finite exposure multiplier"));
}

#[test]
fn tone_map_fragment_shader_declares_all_runtime_branches() {
    let source = include_str!("../../shaders/tonemap.frag");
    assert!(source.contains("layout(push_constant)"));
    assert!(source.contains("aces_narkowicz"));
    assert!(source.contains("TONE_MAP_REINHARD"));
    assert!(source.contains("TONE_MAP_NONE"));
    assert!(source.contains("tone_map.output_is_srgb == 0u"));
    assert!(source.contains("EFFECT_WEIGHTED_OIT"));
    assert!(source.contains("EFFECT_PLANETARY_LENS"));
    assert!(source.contains("planetary_lens_uv"));
    assert!(source.contains("oit_accumulation"));
    assert!(source.contains("oit_optical_depth"));
    let forward = include_str!("../../shaders/forward.frag");
    assert!(forward.contains("out_oit_accumulation"));
    assert!(forward.contains("out_oit_optical_depth"));
}
