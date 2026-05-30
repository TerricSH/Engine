use super::{extract_renderer_input, sample_scene, validate_scene, EntityRecord, Scene};
use engine_serialize::SchemaVersion;
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

// ============================================================================
// Scene save/load round-trip
// ============================================================================

#[test]
fn scene_save_load_roundtrip() {
    let scene = sample_scene();
    let dir = std::env::temp_dir().join("engine-scene-tests");
    let path = dir.join("test_roundtrip.scene.ron");

    // Clean up any previous test file.
    let _ = std::fs::remove_file(&path);

    // Save
    scene.save_to_file(&path).expect("save should succeed");

    // Load
    let loaded = Scene::load_from_file(&path).expect("load should succeed");

    // Verify structural equality
    assert_eq!(loaded.schema_version, scene.schema_version);
    assert_eq!(loaded.scene_id, scene.scene_id);
    assert_eq!(loaded.name, scene.name);
    assert_eq!(loaded.entities.len(), scene.entities.len());
    assert_eq!(loaded.dependencies, scene.dependencies);

    // Verify entity data
    for orig_entity in &scene.entities {
        let found = loaded
            .entities
            .iter()
            .find(|e| e.persistent_id == orig_entity.persistent_id);
        assert!(
            found.is_some(),
            "missing entity {} after round-trip",
            orig_entity.persistent_id
        );
        if let Some(le) = found {
            assert_eq!(le.enabled, orig_entity.enabled);
            assert_eq!(le.components.len(), orig_entity.components.len());
            // Check first component's fields match
            for (comp_type, orig_comp) in &orig_entity.components {
                let loaded_comp = le
                    .components
                    .get(comp_type)
                    .expect("component type missing after round-trip");
                assert_eq!(loaded_comp.fields, orig_comp.fields);
            }
        }
    }

    // Clean up
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}

// ============================================================================
// Scene version check
// ============================================================================

#[test]
fn scene_version_match_returns_empty_diagnostics() {
    let scene = sample_scene();
    let diags = scene.check_version();
    assert!(
        diags.is_empty(),
        "expected empty diagnostics for matching version, got: {:?}",
        diags
    );
}

#[test]
fn scene_version_major_mismatch_returns_error() {
    let mut scene = sample_scene();
    scene.schema_version = SchemaVersion::new(1, 0, 0);
    let diags = scene.check_version();
    assert!(
        diags.iter().any(|d| d.code == "SC0020"),
        "expected SC0020 for major version mismatch, got: {:?}",
        diags
    );
}

#[test]
fn scene_version_minor_ahead_returns_warning() {
    let mut scene = sample_scene();
    scene.schema_version = SchemaVersion::new(0, 2, 0);
    let diags = scene.check_version();
    assert!(
        diags.iter().any(|d| d.code == "SC0021"),
        "expected SC0021 for newer minor version, got: {:?}",
        diags
    );
}

#[test]
fn scene_version_older_patch_is_compatible() {
    let mut scene = sample_scene();
    scene.schema_version = SchemaVersion::new(0, 1, 0); // same as current
    let diags = scene.check_version();
    assert!(
        diags.is_empty(),
        "expected no errors for older patch version, got: {:?}",
        diags
    );
}

// ============================================================================
// Scene collect_asset_dependencies
// ============================================================================

#[test]
fn scene_collect_asset_dependencies_includes_mesh_and_material() {
    let scene = sample_scene();
    let deps = scene.collect_asset_dependencies();
    // sample_scene has mesh-cube and mat-default
    assert!(
        deps.iter().any(|a| a.id == "mesh-cube"),
        "expected mesh-cube in deps, got: {:?}",
        deps
    );
    assert!(
        deps.iter().any(|a| a.id == "mat-default"),
        "expected mat-default in deps, got: {:?}",
        deps
    );
    // Should not have duplicates
    assert_eq!(deps.len(), 2, "expected exactly 2 unique dependencies");
}
