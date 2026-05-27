use super::{extract_renderer_input, sample_scene, validate_scene, EntityRecord};
use std::collections::BTreeMap;

// ============================================================================
// Basic validation and extraction
// ============================================================================

#[test]
fn sample_scene_validates_and_extracts() {
    let scene = sample_scene();
    assert!(validate_scene(&scene).is_empty());
    let input = extract_renderer_input(&scene, 7).expect("sample scene extracts");
    assert_eq!(input.frame_index, 7);
    assert_eq!(input.views.len(), 1);
    assert_eq!(input.drawables.len(), 1);
}

// ============================================================================
// Scene validation: duplicate entity IDs (SC0015)
// ============================================================================

#[test]
fn scene_with_duplicate_entity_id_produces_sc0015() {
    let mut scene = sample_scene();
    scene.entities.push(scene.entities[0].clone());
    let diagnostics = validate_scene(&scene);
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0015"),
        "expected SC0015 for duplicate entity ID, got: {:?}",
        diagnostics
    );
}

#[test]
fn scene_with_multiple_duplicate_entities_produces_multiple_sc0015() {
    let mut scene = sample_scene();
    // Clone the first entity twice
    scene.entities.push(scene.entities[0].clone());
    scene.entities.push(scene.entities[0].clone());
    let diagnostics = validate_scene(&scene);
    let sc0015_count = diagnostics.iter().filter(|d| d.code == "SC0015").count();
    assert!(
        sc0015_count >= 2,
        "expected at least 2 SC0015 for triple duplicate, got {}",
        sc0015_count
    );
}

// ============================================================================
// Scene validation: missing parent (SC0016)
// ============================================================================

#[test]
fn scene_with_missing_parent_produces_sc0016() {
    let mut scene = sample_scene();
    let orphan = EntityRecord {
        persistent_id: "orphan".to_string(),
        parent: Some("non-existent-parent".to_string()),
        name: Some("Orphan".to_string()),
        enabled: true,
        components: BTreeMap::new(),
    };
    scene.entities.push(orphan);
    let diagnostics = validate_scene(&scene);
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0016"),
        "expected SC0016 for missing parent, got: {:?}",
        diagnostics
    );
}

#[test]
fn scene_with_valid_parent_does_not_produce_sc0016() {
    let mut scene = sample_scene();
    // Create a child that references the existing camera entity as parent
    let child = EntityRecord {
        persistent_id: "child-of-camera".to_string(),
        parent: Some("camera-main".to_string()),
        name: Some("Child".to_string()),
        enabled: true,
        components: BTreeMap::new(),
    };
    scene.entities.push(child);
    let diagnostics = validate_scene(&scene);
    assert!(
        !diagnostics.iter().any(|d| d.code == "SC0016"),
        "valid parent should not produce SC0016, got: {:?}",
        diagnostics
    );
}

// ============================================================================
// Scene validation: invalid active camera (SC0017)
// ============================================================================

#[test]
fn scene_with_non_existent_active_camera_produces_sc0017() {
    let mut scene = sample_scene();
    scene.scene_settings.active_camera = Some("non-existent-camera".to_string());
    let diagnostics = validate_scene(&scene);
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0017"),
        "expected SC0017 for non-existent active_camera, got: {:?}",
        diagnostics
    );
}

#[test]
fn scene_with_disabled_camera_entity_produces_sc0017() {
    let mut scene = sample_scene();
    scene.entities[0].enabled = false;
    let diagnostics = validate_scene(&scene);
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0017"),
        "expected SC0017 for disabled camera entity, got: {:?}",
        diagnostics
    );
}

#[test]
fn scene_with_disabled_camera_component_produces_sc0017() {
    let mut scene = sample_scene();
    if let Some(cam) = scene.entities[0].components.get_mut("engine.camera") {
        cam.enabled = false;
    }
    let diagnostics = validate_scene(&scene);
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0017"),
        "expected SC0017 for disabled camera component, got: {:?}",
        diagnostics
    );
}

#[test]
fn scene_without_active_camera_validates_successfully() {
    let mut scene = sample_scene();
    scene.scene_settings.active_camera = None;
    let diagnostics = validate_scene(&scene);
    // No SC0017 since active_camera is None (no camera to validate)
    assert!(
        !diagnostics.iter().any(|d| d.code == "SC0017"),
        "no active_camera should not produce SC0017, got: {:?}",
        diagnostics
    );
}

// ============================================================================
// Extraction failure for scenes with validation errors
// ============================================================================

#[test]
fn extraction_fails_for_scene_with_validation_errors() {
    let mut scene = sample_scene();
    scene.entities.push(scene.entities[0].clone()); // duplicate
    let result = extract_renderer_input(&scene, 0);
    assert!(result.is_err(), "extraction should fail for invalid scene");
    let diagnostics = result.unwrap_err();
    assert!(
        diagnostics.iter().any(|d| d.code == "SC0015"),
        "expected SC0015 in extraction error, got: {:?}",
        diagnostics
    );
}

#[test]
fn extraction_fails_for_scene_with_missing_camera() {
    let mut scene = sample_scene();
    scene.entities.clear(); // remove camera entity
    scene.scene_settings.active_camera = None;
    let result = extract_renderer_input(&scene, 0);
    assert!(result.is_err(), "extraction should fail with no camera");
}
