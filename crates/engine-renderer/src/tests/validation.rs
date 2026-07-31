// ============================================================================
// validate_frame_input tests
// ============================================================================

#[test]
fn planetary_lens_manual_mode_preserves_authored_values() {
    let lens = super::PlanetaryLensSettings {
        enabled: true,
        mode: super::PlanetaryLensMode::Manual,
        barrel_distortion: 0.12,
        horizon_curvature: -0.08,
        atmosphere_intensity: 0.4,
        chromatic_aberration: 0.006,
        ..super::PlanetaryLensSettings::default()
    };

    assert_eq!(lens.altitude_intensity(None), 1.0);
    assert_eq!(lens.resolved_for_camera_altitude(Some(-500.0)), lens);
}

#[test]
fn planetary_lens_legacy_payload_defaults_to_manual_mode() {
    let lens: super::PlanetaryLensSettings = serde_json::from_str(
        r#"{
            "enabled": true,
            "barrel_distortion": 0.03,
            "horizon_curvature": 0.02,
            "atmosphere_intensity": 0.08,
            "chromatic_aberration": 0.002
        }"#,
    )
    .expect("legacy planetary lens payload");

    assert_eq!(lens.mode, super::PlanetaryLensMode::Manual);
    assert_eq!(lens.altitude_fade_start, 1_000.0);
    assert_eq!(lens.altitude_fade_end, 20_000.0);
    assert_eq!(lens.resolved_for_camera_altitude(None), lens);
}

#[test]
fn planetary_lens_camera_altitude_uses_smoothstep_and_fails_closed() {
    let lens = super::PlanetaryLensSettings {
        enabled: true,
        mode: super::PlanetaryLensMode::CameraAltitude,
        altitude_fade_start: 100.0,
        altitude_fade_end: 300.0,
        barrel_distortion: 0.12,
        horizon_curvature: 0.08,
        atmosphere_intensity: 0.4,
        chromatic_aberration: 0.006,
    };

    assert_eq!(lens.altitude_intensity(Some(100.0)), 0.0);
    assert_eq!(lens.altitude_intensity(Some(200.0)), 0.5);
    assert_eq!(lens.altitude_intensity(Some(300.0)), 1.0);
    assert_eq!(lens.altitude_intensity(None), 0.0);
    assert_eq!(lens.altitude_intensity(Some(f64::NAN)), 0.0);

    let resolved = lens.resolved_for_camera_altitude(Some(200.0));
    assert!(resolved.enabled);
    assert!((resolved.barrel_distortion - 0.06).abs() < f32::EPSILON);
    assert!((resolved.horizon_curvature - 0.04).abs() < f32::EPSILON);
    assert!((resolved.atmosphere_intensity - 0.2).abs() < f32::EPSILON);
    assert!((resolved.chromatic_aberration - 0.003).abs() < f32::EPSILON);

    assert!(!lens.resolved_for_camera_altitude(None).enabled);
}

#[test]
fn planetary_lens_rejects_invalid_altitude_fade_bounds() {
    let mut input = valid_frame();
    input
        .render_options
        .post_process
        .planetary_lens
        .altitude_fade_start = 300.0;
    input
        .render_options
        .post_process
        .planetary_lens
        .altitude_fade_end = 100.0;

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RV0025" && diagnostic.message.contains("altitude fade bounds")
    }));
}

#[test]
fn validate_empty_views_produces_rv0013() {
    let input = RenderFrameInput::empty(0);
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0013"),
        "expected RV0013 for empty views, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_duplicate_view_ids_produces_rv0014() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![
        RenderView {
            view_id: 0,
            camera_entity: None,
            viewport: super::Rect::FULL,
            viewport_rect_normalized: super::Rect::FULL,
            view_matrix: IDENTITY_MAT4,
            projection_matrix: IDENTITY_MAT4,
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
        },
        RenderView {
            view_id: 0, // duplicate
            camera_entity: None,
            viewport: super::Rect::FULL,
            viewport_rect_normalized: super::Rect::FULL,
            view_matrix: IDENTITY_MAT4,
            projection_matrix: IDENTITY_MAT4,
            clear_flags: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: ViewCompose::Base {
                clear: ClearFlags::ColorAndDepth,
                clear_color: [0.0, 0.0, 0.0, 1.0],
            },
            stack_order: 1,
            frustum: None,
        },
    ];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0014"),
        "expected RV0014 for duplicate views, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_invalid_bone_palette_produces_rv0020() {
    let mut input = valid_frame();
    input.skinned_items.push(SkinnedItem {
        entity: None,
        mesh: AssetId::new("mesh.skinned"),
        material: AssetId::new("material.skin"),
        skeleton: AssetId::new("skeleton.humanoid"),
        bone_palette: vec![IDENTITY_MAT4],
        bone_palette_layout: BonePaletteLayout::Full4x4 { count: 2 },
        morph_target_set: None,
        morph_weights: Vec::new(),
        world_transform: IDENTITY_MAT4,
        bounds: AxisAlignedBox::UNIT,
        render_layer: "default".into(),
        cast_shadows: true,
        sort_key: 0,
    });

    let diagnostics = validate_frame_input(&input);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "RV0020"));
}

#[test]
fn validate_missing_base_view_produces_rv0007() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 1,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Overlay {
            base_view_id: 99, // non-existent base view
            blend_mode: BlendMode::AlphaBlend,
        },
        stack_order: 0,
        frustum: None,
    }];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0007"),
        "expected RV0007 for overlay with missing base view, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_unsupported_shadow_mode_for_point_light_produces_rv0015() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
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
    }];
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Point,
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
        range: 10.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Hard, // not supported for Point lights
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0015"),
        "expected RV0015 for unsupported shadow mode on point light, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_unsupported_shadow_mode_for_spot_light_produces_rv0015() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
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
    }];
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Spot,
        color: [1.0, 1.0, 1.0],
        intensity: 5.0,
        range: 20.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Soft, // not supported for Spot lights
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0015"),
        "expected RV0015 for unsupported shadow mode on spot light, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_directional_shadow_mode_is_accepted() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
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
    }];
    // Directional lights support Hard/Soft shadow modes — no RV0015 expected
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Directional,
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
        range: 100.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Hard,
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        !diagnostics.iter().any(|d| d.code == "RV0015"),
        "directional light with Hard shadow should NOT produce RV0015, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_zero_light_intensity_produces_rv0016() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
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
    }];
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Directional,
        color: [1.0, 1.0, 1.0],
        intensity: 0.0, // zero — should warn
        range: 100.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Off,
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0016"),
        "expected RV0016 for zero intensity light, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_negative_light_intensity_produces_rv0016() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
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
    }];
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Point,
        color: [1.0, 1.0, 1.0],
        intensity: -1.0, // negative → should warn
        range: 10.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Off,
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0016"),
        "expected RV0016 for negative intensity light, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_invalid_contract_version_produces_rv0012() {
    let mut input = RenderFrameInput::empty(0);
    input.contract_version = "bad-version".to_string();
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
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
    }];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0012"),
        "expected RV0012 for invalid contract, got: {:?}",
        diagnostics
    );
}

#[test]
fn validate_valid_input_produces_no_diagnostics() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: super::Rect::FULL,
        viewport_rect_normalized: super::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
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
    }];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.is_empty(),
        "valid input should produce no diagnostics, got: {:?}",
        diagnostics
    );
}
