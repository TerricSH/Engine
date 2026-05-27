use engine_core::{EngineConfig, EngineRuntime};
use engine_renderer::{
    validate_frame_input, LightItem, LightKind, RenderFrameInput, RenderView, ShadowMode,
    ViewCompose, IDENTITY_MAT4,
};
use engine_scene::{sample_scene, validate_scene};

// ============================================================================
// Core engine lifecycle tests
// ============================================================================

#[test]
fn engine_core_creates_runtime() {
    let config = EngineConfig::default();
    let runtime = EngineRuntime::new(config);
    assert_eq!(runtime.config().application_name, "engine");
}

#[test]
fn engine_config_defaults_are_correct() {
    let config = EngineConfig::default();
    assert_eq!(config.application_name, "engine");
}

#[test]
fn engine_render_frame_no_scene_fails() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let result = runtime.render_frame(0);
    assert!(result.is_err());
    let diagnostics = result.unwrap_err();
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0018"),
        "expected SC0018 (no scene loaded), got: {:?}",
        diagnostics
    );
}

#[test]
fn engine_load_scene_and_render_succeeds() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.load_scene(sample_scene());
    let result = runtime.render_frame(0);
    assert!(
        result.is_ok(),
        "render_frame with sample scene should succeed: {:?}",
        result
    );
    let stats = result.unwrap();
    assert_eq!(stats.visible_drawables, 1);
    assert_eq!(stats.draw_calls, 1);
}

#[test]
fn sample_scene_validates_correctly() {
    let scene = sample_scene();
    let diagnostics = validate_scene(&scene);
    assert!(
        diagnostics.is_empty(),
        "sample scene should validate: {:?}",
        diagnostics
    );
}

// ============================================================================
// Scene validation tests (invalid scenes)
// ============================================================================

#[test]
fn scene_with_duplicate_entity_id_fails_validation() {
    let mut scene = sample_scene();
    scene.entities.push(scene.entities[0].clone());
    let diagnostics = validate_scene(&scene);
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0015"),
        "expected SC0015 for duplicate entity, got: {:?}",
        diagnostics
    );
}

#[test]
fn scene_with_missing_parent_fails_validation() {
    let mut scene = sample_scene();
    // Add an entity referencing a non-existent parent
    let mut orphan_entity = scene.entities[0].clone();
    orphan_entity.persistent_id = "orphan-entity".to_string();
    orphan_entity.parent = Some("non-existent-parent".to_string());
    scene.entities.push(orphan_entity);
    let diagnostics = validate_scene(&scene);
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0016"),
        "expected SC0016 for missing parent, got: {:?}",
        diagnostics
    );
}

#[test]
fn scene_with_invalid_camera_fails_validation() {
    // Case 1: active_camera points to a non-existent entity
    let mut scene = sample_scene();
    scene.scene_settings.active_camera = Some("non-existent-camera".to_string());
    let diagnostics = validate_scene(&scene);
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0017"),
        "expected SC0017 for non-existent active_camera, got: {:?}",
        diagnostics
    );

    // Case 2: active_camera points to a disabled entity
    let mut scene2 = sample_scene();
    scene2.entities[0].enabled = false; // disable the camera entity
    let diagnostics2 = validate_scene(&scene2);
    assert!(
        diagnostics2.iter().any(|d| d.code == "SC0017"),
        "expected SC0017 for disabled camera entity, got: {:?}",
        diagnostics2
    );

    // Case 3: active_camera points to entity whose engine.camera component is disabled
    let mut scene3 = sample_scene();
    if let Some(cam) = scene3.entities[0].components.get_mut("engine.camera") {
        cam.enabled = false;
    }
    let diagnostics3 = validate_scene(&scene3);
    assert!(
        diagnostics3.iter().any(|d| d.code == "SC0017"),
        "expected SC0017 for disabled camera component, got: {:?}",
        diagnostics3
    );
}

// ============================================================================
// Renderer validation tests (vendor-specific logic)
// ============================================================================

#[test]
fn renderer_rejects_empty_views() {
    let input = RenderFrameInput::empty(0);
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0013"),
        "expected RV0013 for empty views, got: {:?}",
        diagnostics
    );
}

#[test]
fn renderer_rejects_duplicate_view_ids() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![
        RenderView {
            view_id: 0,
            camera_entity: None,
            viewport: engine_renderer::Rect::FULL,
            viewport_rect_normalized: engine_renderer::Rect::FULL,
            view_matrix: IDENTITY_MAT4,
            projection_matrix: IDENTITY_MAT4,
            clear_flags: engine_renderer::ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: ViewCompose::Base {
                clear: engine_renderer::ClearFlags::ColorAndDepth,
                clear_color: [0.0, 0.0, 0.0, 1.0],
            },
            stack_order: 0,
            frustum: None,
        },
        RenderView {
            view_id: 0, // duplicate
            camera_entity: None,
            viewport: engine_renderer::Rect::FULL,
            viewport_rect_normalized: engine_renderer::Rect::FULL,
            view_matrix: IDENTITY_MAT4,
            projection_matrix: IDENTITY_MAT4,
            clear_flags: engine_renderer::ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: ViewCompose::Base {
                clear: engine_renderer::ClearFlags::ColorAndDepth,
                clear_color: [0.0, 0.0, 0.0, 1.0],
            },
            stack_order: 1,
            frustum: None,
        },
    ];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0014"),
        "expected RV0014 for duplicate view IDs, got: {:?}",
        diagnostics
    );
}

#[test]
fn renderer_detects_missing_base_view() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 1,
        camera_entity: None,
        viewport: engine_renderer::Rect::FULL,
        viewport_rect_normalized: engine_renderer::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: engine_renderer::ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Overlay {
            base_view_id: 99, // missing base view
            blend_mode: engine_renderer::BlendMode::AlphaBlend,
        },
        stack_order: 0,
        frustum: None,
    }];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0007"),
        "expected RV0007 for missing base view, got: {:?}",
        diagnostics
    );
}

#[test]
fn renderer_warns_on_unsupported_shadow_mode() {
    let mut input = RenderFrameInput::empty(0);
    // Need at least one view to avoid RV0013
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: engine_renderer::Rect::FULL,
        viewport_rect_normalized: engine_renderer::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: engine_renderer::ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: engine_renderer::ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    // Add a point light with ShadowMode::Hard (unsupported for point lights)
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Point,
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
        range: 10.0,
        position: [0.0, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: ShadowMode::Hard,
    });
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0015"),
        "expected RV0015 for unsupported shadow mode, got: {:?}",
        diagnostics
    );
}

#[test]
fn renderer_warns_on_zero_intensity_light() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: engine_renderer::Rect::FULL,
        viewport_rect_normalized: engine_renderer::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: engine_renderer::ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: engine_renderer::ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    input.lights.push(LightItem {
        entity: None,
        kind: LightKind::Directional,
        color: [1.0, 1.0, 1.0],
        intensity: 0.0, // zero intensity
        range: 10.0,
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
fn renderer_rejects_invalid_contract_version() {
    let mut input = RenderFrameInput::empty(0);
    input.contract_version = "invalid-v999".to_string();
    // Need at least one view to isolate the contract error
    input.views = vec![RenderView {
        view_id: 0,
        camera_entity: None,
        viewport: engine_renderer::Rect::FULL,
        viewport_rect_normalized: engine_renderer::Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: engine_renderer::ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: engine_renderer::ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: 0,
        frustum: None,
    }];
    let diagnostics = validate_frame_input(&input);
    assert!(
        diagnostics.iter().any(|d| d.code == "RV0012"),
        "expected RV0012 for invalid contract version, got: {:?}",
        diagnostics
    );
}

// ============================================================================
// End-to-end pipeline test
// ============================================================================

#[test]
fn full_pipeline_load_scene_and_extract() {
    let scene = sample_scene();
    assert!(validate_scene(&scene).is_empty());

    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.load_scene(scene);
    let result = runtime.render_frame(0);
    assert!(result.is_ok(), "full pipeline failed: {:?}", result);
}
