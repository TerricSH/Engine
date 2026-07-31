//! Editor pipeline integration tests.
//!
//! These tests exercise the full editor -> scene -> extraction pipeline,
//! verifying that editor operations on a scene produce correct rendering
//! input after extraction.  All tests require the `tooling-editor` feature.
//!
//! # Test categories
//!
//! | Area | Files | What it covers |
//! |------|-------|----------------|
//! | EditorScene lifecycle | `editor_pipeline.rs` | Command execute/undo/redo, save/load round-trip, extraction after editing |
//! | Cross-crate | `editor_pipeline.rs` | Editor -> extraction -> renderer validation |
//!
//! The canonical test fixture is `engine_scene::sample_scene()` (2 entities:
//! `camera-main` + `cube-01`).

#![cfg(feature = "tooling-editor")]

use std::collections::BTreeMap;

use engine_core::game_loop::GameLoop;
use engine_core::EngineConfig;
use engine_editor::commands::{
    AddComponent, AddEntity, RemoveComponent, RemoveEntity, SetComponentField, SetEntityName,
};
use engine_editor::io;
use engine_editor::{EditorError, EditorPlayMode, EditorPlaySession, EditorScene};
use engine_renderer::{validate_frame_input, RenderFrameInput};
use engine_scene::{
    extract_renderer_input_from_world, sample_scene, validate_scene, ComponentRecord, EntityRecord,
    Scene,
};
use engine_serialize::{AssetId, SchemaVersion, Value};

// ============================================================================
// Helpers
// ============================================================================

/// A unique persistent ID generator for test entities.
fn unique_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{prefix}-{n}")
}

/// Create a minimal entity record with just a name.
fn make_entity(name: &str) -> EntityRecord {
    EntityRecord {
        persistent_id: unique_id(name),
        parent: None,
        name: Some(name.to_string()),
        enabled: true,
        components: BTreeMap::new(),
    }
}

/// Create a renderable component record for a cube.
fn make_renderable(visible: bool) -> ComponentRecord {
    let mut fields = BTreeMap::new();
    fields.insert("mesh".to_string(), Value::Asset(AssetId::new("mesh-cube")));
    fields.insert(
        "material".to_string(),
        Value::Asset(AssetId::new("mat-default")),
    );
    fields.insert("visible".to_string(), Value::Bool(visible));
    fields.insert("cast_shadows".to_string(), Value::Bool(true));
    fields.insert(
        "render_layer".to_string(),
        Value::Str("Default".to_string()),
    );
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

/// Create a camera component record.
fn make_camera() -> ComponentRecord {
    let mut fields = BTreeMap::new();
    fields.insert(
        "projection".to_string(),
        Value::Enum("Perspective".to_string()),
    );
    fields.insert("near".to_string(), Value::Float32(0.1));
    fields.insert("far".to_string(), Value::Float32(100.0));
    fields.insert(
        "fov_y".to_string(),
        Value::Float32(std::f32::consts::FRAC_PI_3),
    ); // 60 degrees.
    fields.insert(
        "clear_color".to_string(),
        Value::Color([0.0, 0.0, 0.0, 1.0]),
    );
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

/// Extract renderer input from an EditorScene.
fn extract(scene: &Scene) -> Result<RenderFrameInput, Vec<engine_serialize::Diagnostic>> {
    let diagnostics = engine_scene::validate_scene(scene);
    if diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.severity,
            engine_serialize::DiagnosticSeverity::Error
                | engine_serialize::DiagnosticSeverity::Fatal
        )
    }) {
        return Err(diagnostics);
    }
    let world = engine_scene::World::from_scene(scene);
    extract_renderer_input_from_world(&world, 0)
}

// ============================================================================
// EditorScene lifecycle
// ============================================================================

include!("editor_pipeline/commands.rs");
include!("editor_pipeline/extraction_and_io.rs");
include!("editor_pipeline/validation_and_play.rs");
