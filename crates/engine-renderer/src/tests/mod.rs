use super::{
    validate_frame_input, BlendMode, ClearFlags, LightItem, LightKind, RenderFrameInput,
    RenderView, Renderer, ShadowMode, ViewCompose, IDENTITY_MAT4,
};

// ============================================================================
// Renderer::draw_scene tests
// ============================================================================

#[test]
fn empty_frame_is_rejected() {
    let input = RenderFrameInput::empty(0);
    assert!(Renderer::new().draw_scene(&input).is_err());
}

#[test]
fn valid_frame_with_view_succeeds() {
    let mut input = RenderFrameInput::empty(0);
    input.views.push(RenderView {
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
    });
    // No backend attached → returns mock stats, not an error
    assert!(Renderer::new().draw_scene(&input).is_ok());
}

#[test]
fn draw_scene_with_error_diagnostics_fails() {
    let input = RenderFrameInput::empty(0); // empty views → RV0013 error
    let result = Renderer::new().draw_scene(&input);
    assert!(result.is_err());
    let diagnostics = result.unwrap_err();
    assert!(diagnostics.iter().any(|d| d.code == "RV0013"));
}

// ============================================================================
// validate_frame_input tests
// ============================================================================

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
