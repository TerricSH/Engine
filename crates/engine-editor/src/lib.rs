#![forbid(unsafe_code)]

use thiserror::Error;

// ---------------------------------------------------------------------------
// EditorError – always available
// ---------------------------------------------------------------------------

/// Errors that can occur during editor operations.
#[derive(Error, Debug)]
pub enum EditorError {
    /// A panel with the requested name does not exist.
    #[error("panel not found: {0}")]
    PanelNotFound(String),

    /// No scene is currently loaded or the requested scene is missing.
    #[error("scene not found")]
    SceneNotFound,

    /// The requested asset is not available.
    #[error("asset not found")]
    AssetNotFound,

    /// Editor initialisation failed with a contextual message.
    #[error("init failed: {0}")]
    InitFailed(String),
}

// ---------------------------------------------------------------------------
// Stub – available when the `tooling-editor` feature is NOT enabled
// ---------------------------------------------------------------------------

/// Placeholder type exposed when the `tooling-editor` feature is disabled.
///
/// Cannot be constructed outside this crate.  Match on this to handle the
/// no-editor case at compile time.
#[non_exhaustive]
pub struct EditorDisabled {
    pub(crate) _private: (),
}

// ---------------------------------------------------------------------------
// Full editor implementation behind the `tooling-editor` feature gate
// ---------------------------------------------------------------------------

#[cfg(feature = "tooling-editor")]
mod panels;
#[cfg(feature = "tooling-editor")]
mod editor_ui;
#[cfg(feature = "tooling-editor")]
mod editor_core;

#[cfg(feature = "tooling-editor")]
pub use panels::{EditorPanel, SceneViewPanel, InspectorPanel, AssetBrowserPanel};
#[cfg(feature = "tooling-editor")]
pub use editor_ui::EditorUi;
#[cfg(feature = "tooling-editor")]
pub use editor_core::Editor;

#[cfg(test)]
mod tests {
    use super::*;

    // ── EditorError tests ────────────────────────────────────────────────

    #[test]
    fn editor_error_panel_not_found_display() {
        let err = EditorError::PanelNotFound("SceneView".to_string());
        assert_eq!(err.to_string(), "panel not found: SceneView");
    }

    #[test]
    fn editor_error_scene_not_found_display() {
        let err = EditorError::SceneNotFound;
        assert_eq!(err.to_string(), "scene not found");
    }

    #[test]
    fn editor_error_asset_not_found_display() {
        let err = EditorError::AssetNotFound;
        assert_eq!(err.to_string(), "asset not found");
    }

    #[test]
    fn editor_error_init_failed_display() {
        let err = EditorError::InitFailed("missing config".to_string());
        assert_eq!(err.to_string(), "init failed: missing config");
    }

    // ── EditorDisabled tests ─────────────────────────────────────────────

    #[test]
    fn editor_disabled_is_non_exhaustive() {
        // Can only construct via the crate-internal field
        let _disabled = EditorDisabled { _private: () };
    }

    // ── EditorUi tests (behind tooling-editor feature) ───────────────────

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_new_creates_context() {
        let ui = EditorUi::new();
        // Can't inspect fields directly, but reset should not panic
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_text_field_returns_none() {
        let mut ui = EditorUi::new();
        assert_eq!(ui.text_field("label", "value"), None);
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_button_returns_false() {
        let mut ui = EditorUi::new();
        assert!(!ui.button("Click me"));
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_slider_f32_returns_none() {
        let mut ui = EditorUi::new();
        assert_eq!(ui.slider_f32("slider", 0.5, 0.0, 1.0), None);
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_checkbox_passthrough() {
        let mut ui = EditorUi::new();
        assert!(ui.checkbox("check", true));
        assert!(!ui.checkbox("check", false));
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_color_edit_returns_none() {
        let mut ui = EditorUi::new();
        assert_eq!(ui.color_edit("color", [1.0, 0.0, 0.0, 1.0]), None);
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_separator_does_not_panic() {
        let mut ui = EditorUi::new();
        ui.separator();
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_collapsing_header_returns_default() {
        let mut ui = EditorUi::new();
        assert!(ui.collapsing_header("header", true));
        assert!(!ui.collapsing_header("header2", false));
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_reset_does_not_panic() {
        let mut ui = EditorUi::new();
        ui.text_field("a", "1");
        ui.button("b");
        ui.separator();
        ui.reset(); // Should reset without error
        // After reset, should behave like new
        assert_eq!(ui.text_field("c", "3"), None);
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_ui_default() {
        let ui = EditorUi::default();
        let _ = ui; // Just verify Default impl compiles
    }

    // ── Editor panel tests (behind tooling-editor feature) ───────────────

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn scene_view_panel_new() {
        use crate::SceneViewPanel;
        let panel = SceneViewPanel::new("Scene");
        assert_eq!(panel.name(), "Scene");
        assert!(panel.visible());
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn inspector_panel_new() {
        use crate::InspectorPanel;
        let panel = InspectorPanel::new("Inspector");
        assert_eq!(panel.name(), "Inspector");
        assert!(panel.visible());
        assert!(panel.selected_entity().is_none());
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn asset_browser_panel_new() {
        use crate::AssetBrowserPanel;
        let panel = AssetBrowserPanel::new("Browser");
        assert_eq!(panel.name(), "Browser");
        assert_eq!(panel.current_path(), "/");
        assert!(panel.entries().is_empty());
    }
}
