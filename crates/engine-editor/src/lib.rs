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

    /// An entity with the requested ID was not found in the scene.
    #[error("entity not found: {0}")]
    EntityNotFound(String),

    /// A component of the requested type was not found on the entity.
    #[error("component not found: {0}")]
    ComponentNotFound(String),

    /// An I/O operation (read, write, create directory, …) failed.
    #[error("I/O error: {0}")]
    IoFailed(String),

    /// The `dotnet` CLI was not found on `PATH`.
    #[error("dotnet CLI not found on PATH")]
    BuildDotnetNotFound,

    /// A C# project build failed with a message.
    #[error("build failed: {0}")]
    BuildFailed(String),
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
pub mod build;
#[cfg(feature = "tooling-editor")]
pub mod commands;
#[cfg(feature = "tooling-editor")]
pub mod diagnostics;
#[cfg(feature = "tooling-editor")]
mod editor_core;
#[cfg(feature = "tooling-editor")]
mod editor_ui;
#[cfg(feature = "tooling-editor")]
pub mod hierarchy;
#[cfg(feature = "tooling-editor")]
pub mod inspector;
#[cfg(feature = "tooling-editor")]
pub mod io;
#[cfg(feature = "tooling-editor")]
mod panels;
#[cfg(feature = "tooling-editor")]
pub mod plugin;
#[cfg(feature = "tooling-editor")]
pub mod scene_view;
#[cfg(feature = "tooling-editor")]
pub mod script_build;
#[cfg(feature = "tooling-editor")]
mod script_inspector;

#[cfg(feature = "tooling-editor")]
pub use build::{build_csharp_project, BuildError};
#[cfg(feature = "tooling-editor")]
pub use commands::{
    AddComponent, AddEntity, Command, CommandHistory, RemoveComponent, RemoveEntity,
    SetComponentField, SetEntityName,
};
#[cfg(feature = "tooling-editor")]
pub use diagnostics::{DiagnosticEntry, DiagnosticsPanel};
#[cfg(feature = "tooling-editor")]
pub use editor_core::Editor;
#[cfg(feature = "tooling-editor")]
pub use editor_ui::EditorUi;
#[cfg(feature = "tooling-editor")]
pub use hierarchy::HierarchyPanel;
#[cfg(feature = "tooling-editor")]
pub use inspector::InspectorPanel;
#[cfg(feature = "tooling-editor")]
pub use io::{default_scene_path, load_scene, save_scene};
#[cfg(feature = "tooling-editor")]
pub use panels::{
    AssetBrowserPanel, EditorPanel, InspectorPanel as LegacyInspectorPanel, SceneViewPanel,
};
#[cfg(feature = "tooling-editor")]
pub use plugin::{
    ComponentInspector, EditorPlugin, EditorPluginMeta, EditorPluginRegistry, PanelFactory,
};
#[cfg(feature = "tooling-editor")]
pub use scene_view::{orbit_projection_matrix, orbit_view_matrix};
#[cfg(feature = "tooling-editor")]
pub use script_build::{BuildResult, ScriptBuildManager};
#[cfg(feature = "tooling-editor")]
pub use script_inspector::ScriptInspector;
// Note: `build::BuildResult` is intentionally not re-exported here because
// `script_build::BuildResult` already provides a similar type.  Use
// `engine_editor::build::BuildResult` to access the build module's version.

// ---------------------------------------------------------------------------
// EditorScene – scene + undo/redo + selection
// ---------------------------------------------------------------------------

/// Owns a [`Scene`] together with its undo/redo history and the currently
/// selected entity.
///
/// This is the primary integration point for scene editing: panels produce
/// [`Command`]s, and `EditorScene` executes them (pushing them into the
/// command history).
#[cfg(feature = "tooling-editor")]
pub struct EditorScene {
    /// The underlying ECS scene.
    pub scene: engine_scene::Scene,
    /// Undo/redo history.
    pub history: CommandHistory,
    /// Currently selected entity ID.
    pub selected_entity: Option<PersistentId>,
    /// Diagnostics panel for displaying scene/asset/script errors.
    pub diagnostics: DiagnosticsPanel,
}

#[cfg(feature = "tooling-editor")]
impl EditorScene {
    /// Wrap an existing [`Scene`] in a new editor scene.
    pub fn new(scene: engine_scene::Scene) -> Self {
        Self {
            scene,
            history: CommandHistory::new(),
            selected_entity: None,
            diagnostics: DiagnosticsPanel::new("Diagnostics"),
        }
    }

    /// Mutable access to the diagnostics panel.
    pub fn diagnostics_mut(&mut self) -> &mut DiagnosticsPanel {
        &mut self.diagnostics
    }

    /// Execute a command on the scene and push it onto the undo stack.
    pub fn execute(&mut self, cmd: Box<dyn Command>) -> Result<(), EditorError> {
        self.history.push(cmd, &mut self.scene)
    }

    /// Undo the last command.
    pub fn undo(&mut self) -> Result<(), EditorError> {
        self.history.undo(&mut self.scene)
    }

    /// Redo the last-undone command.
    pub fn redo(&mut self) -> Result<(), EditorError> {
        self.history.redo(&mut self.scene)
    }

    /// Whether the history has been dirtied since the last [`save`] or
    /// [`mark_clean`].
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// Save the scene to the given path (defaults to
    /// `assets/scenes/{scene_id}.scene.ron`).
    pub fn save(&self, path: Option<&std::path::Path>) -> Result<(), EditorError> {
        let p = match path {
            Some(p) => p.to_path_buf(),
            None => std::path::PathBuf::from(io::default_scene_path(&self.scene)),
        };
        io::save_scene(&self.scene, &p)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Type alias for the common PersistentId string
// ---------------------------------------------------------------------------

/// Convenience alias for [`engine_serialize::PersistentId`].
pub type PersistentId = engine_serialize::PersistentId;

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
    fn legacy_inspector_panel_new() {
        use crate::LegacyInspectorPanel;
        let panel = LegacyInspectorPanel::new("Inspector");
        assert_eq!(panel.name(), "Inspector");
        assert!(panel.visible());
        assert!(panel.selected_entity().is_none());
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn new_inspector_panel_new() {
        use crate::InspectorPanel;
        let mut panel = InspectorPanel::new("Inspector");
        assert_eq!(panel.name(), "Inspector");
        assert!(panel.visible());
        // ui() should not panic even with no selected entity
        let mut ui = crate::EditorUi::new();
        let scene = engine_scene::sample_scene();
        let cmds = panel.ui(&mut ui, &scene, None);
        assert!(cmds.is_empty());
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

    // ── Command tests ───────────────────────────────────────────────────

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn command_history_new_is_not_dirty() {
        let history = crate::CommandHistory::new();
        assert!(!history.is_dirty());
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn set_entity_name_execute_and_undo() {
        use crate::commands::SetEntityName;
        let mut scene = engine_scene::sample_scene();
        let entity_id = "camera-main".to_string();

        let mut cmd = SetEntityName::new(entity_id.clone(), Some("Renamed".to_string()));
        cmd.execute(&mut scene).unwrap();

        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == entity_id)
            .unwrap();
        assert_eq!(entity.name.as_deref(), Some("Renamed"));

        cmd.undo(&mut scene).unwrap();
        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == entity_id)
            .unwrap();
        assert_eq!(entity.name.as_deref(), Some("Main Camera"));
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn set_component_field_execute_and_undo() {
        use crate::commands::SetComponentField;
        use engine_serialize::Value;

        let mut scene = engine_scene::sample_scene();
        let entity_id = "cube-01".to_string();
        let comp_type = "engine.renderable".to_string();

        let mut cmd = SetComponentField::new(
            entity_id.clone(),
            comp_type.clone(),
            "visible".to_string(),
            Value::Bool(false),
        );
        cmd.execute(&mut scene).unwrap();

        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == entity_id)
            .unwrap();
        let comp = entity.components.get(&comp_type).unwrap();
        assert_eq!(comp.fields.get("visible"), Some(&Value::Bool(false)));

        cmd.undo(&mut scene).unwrap();
        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == entity_id)
            .unwrap();
        let comp = entity.components.get(&comp_type).unwrap();
        assert_eq!(comp.fields.get("visible"), Some(&Value::Bool(true)));
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn add_entity_execute_and_undo() {
        use crate::commands::AddEntity;
        use engine_scene::EntityRecord;
        use std::collections::BTreeMap;

        let mut scene = engine_scene::sample_scene();
        let count_before = scene.entities.len();

        let entity = EntityRecord {
            persistent_id: "new-entity".to_string(),
            parent: None,
            name: Some("New".to_string()),
            enabled: true,
            components: BTreeMap::new(),
        };

        let mut cmd = AddEntity::new(entity);
        cmd.execute(&mut scene).unwrap();
        assert_eq!(scene.entities.len(), count_before + 1);
        assert!(scene
            .entities
            .iter()
            .any(|e| e.persistent_id == "new-entity"));

        cmd.undo(&mut scene).unwrap();
        assert_eq!(scene.entities.len(), count_before);
        assert!(!scene
            .entities
            .iter()
            .any(|e| e.persistent_id == "new-entity"));
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn remove_entity_execute_and_undo() {
        use crate::commands::RemoveEntity;

        let mut scene = engine_scene::sample_scene();
        let count_before = scene.entities.len();

        let mut cmd = RemoveEntity::new(&"cube-01".to_string(), &scene);
        cmd.execute(&mut scene).unwrap();
        assert_eq!(scene.entities.len(), count_before - 1);
        assert!(!scene.entities.iter().any(|e| e.persistent_id == "cube-01"));

        cmd.undo(&mut scene).unwrap();
        assert_eq!(scene.entities.len(), count_before);
        assert!(scene.entities.iter().any(|e| e.persistent_id == "cube-01"));
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn add_component_execute_and_undo() {
        use crate::commands::AddComponent;
        use engine_scene::ComponentRecord;
        use engine_serialize::{SchemaVersion, Value};
        use std::collections::BTreeMap;

        let mut scene = engine_scene::sample_scene();
        let entity_id = "camera-main".to_string();
        let comp_type = "test.component".to_string();

        let mut fields = BTreeMap::new();
        fields.insert("value".to_string(), Value::Int(42));
        let comp = ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields,
        };

        let mut cmd = AddComponent::new(entity_id.clone(), comp_type.clone(), comp);
        cmd.execute(&mut scene).unwrap();

        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == entity_id)
            .unwrap();
        assert!(entity.components.contains_key(&comp_type));

        cmd.undo(&mut scene).unwrap();
        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == entity_id)
            .unwrap();
        assert!(!entity.components.contains_key(&comp_type));
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn remove_component_execute_and_undo() {
        use crate::commands::RemoveComponent;

        let mut scene = engine_scene::sample_scene();
        let entity_id = "cube-01".to_string();
        let comp_type = "engine.renderable".to_string();

        let mut cmd = RemoveComponent::new(entity_id.clone(), comp_type.clone());
        cmd.execute(&mut scene).unwrap();

        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == entity_id)
            .unwrap();
        assert!(!entity.components.contains_key(&comp_type));

        cmd.undo(&mut scene).unwrap();
        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == entity_id)
            .unwrap();
        assert!(entity.components.contains_key(&comp_type));
    }

    // ── CommandHistory integration tests ─────────────────────────────────

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn command_history_push_and_undo() {
        use crate::commands::{CommandHistory, SetEntityName};

        let mut scene = engine_scene::sample_scene();
        let mut history = CommandHistory::new();

        let cmd = Box::new(SetEntityName::new(
            "camera-main".to_string(),
            Some("Cam".to_string()),
        ));
        history.push(cmd, &mut scene).unwrap();
        assert!(history.can_undo());
        assert!(!history.can_redo());
        assert!(history.is_dirty());

        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == "camera-main")
            .unwrap();
        assert_eq!(entity.name.as_deref(), Some("Cam"));

        history.undo(&mut scene).unwrap();
        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == "camera-main")
            .unwrap();
        assert_eq!(entity.name.as_deref(), Some("Main Camera"));

        assert!(history.can_redo());
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn command_history_redo() {
        use crate::commands::{CommandHistory, SetEntityName};

        let mut scene = engine_scene::sample_scene();
        let mut history = CommandHistory::new();

        let cmd = Box::new(SetEntityName::new(
            "camera-main".to_string(),
            Some("Cam".to_string()),
        ));
        history.push(cmd, &mut scene).unwrap();

        history.undo(&mut scene).unwrap();
        history.redo(&mut scene).unwrap();

        let entity = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == "camera-main")
            .unwrap();
        assert_eq!(entity.name.as_deref(), Some("Cam"));
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn command_history_mark_clean() {
        use crate::commands::CommandHistory;

        let mut scene = engine_scene::sample_scene();
        let mut history = CommandHistory::new();

        let cmd = Box::new(crate::commands::SetEntityName::new(
            "camera-main".to_string(),
            Some("Cam".to_string()),
        ));
        history.push(cmd, &mut scene).unwrap();
        assert!(history.is_dirty());

        history.mark_clean();
        assert!(!history.is_dirty());
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn command_history_clear() {
        use crate::commands::CommandHistory;

        let mut scene = engine_scene::sample_scene();
        let mut history = CommandHistory::new();

        let cmd = Box::new(crate::commands::SetEntityName::new(
            "camera-main".to_string(),
            Some("Cam".to_string()),
        ));
        history.push(cmd, &mut scene).unwrap();
        history.clear();

        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert!(!history.is_dirty());
    }

    // ── EditorScene tests ────────────────────────────────────────────────

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_scene_new_not_dirty() {
        let scene = engine_scene::sample_scene();
        let editor_scene = crate::EditorScene::new(scene);
        assert!(!editor_scene.is_dirty());
        assert!(editor_scene.selected_entity.is_none());
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_scene_execute_and_undo() {
        let scene = engine_scene::sample_scene();
        let mut editor_scene = crate::EditorScene::new(scene);

        let cmd = Box::new(crate::commands::SetEntityName::new(
            "camera-main".to_string(),
            Some("Renamed".to_string()),
        ));

        editor_scene.execute(cmd).unwrap();
        assert!(editor_scene.is_dirty());

        let entity = editor_scene
            .scene
            .entities
            .iter()
            .find(|e| e.persistent_id == "camera-main")
            .unwrap();
        assert_eq!(entity.name.as_deref(), Some("Renamed"));

        editor_scene.undo().unwrap();
        let entity = editor_scene
            .scene
            .entities
            .iter()
            .find(|e| e.persistent_id == "camera-main")
            .unwrap();
        assert_eq!(entity.name.as_deref(), Some("Main Camera"));

        editor_scene.redo().unwrap();
        let entity = editor_scene
            .scene
            .entities
            .iter()
            .find(|e| e.persistent_id == "camera-main")
            .unwrap();
        assert_eq!(entity.name.as_deref(), Some("Renamed"));
    }

    // ── Hierarchy panel tests ────────────────────────────────────────────

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn hierarchy_panel_new() {
        use crate::HierarchyPanel;
        let panel = HierarchyPanel::new("Hierarchy");
        assert_eq!(panel.name(), "Hierarchy");
        assert!(panel.visible());
        assert!(panel.selected().is_none());
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn hierarchy_panel_ui_returns_commands() {
        use crate::HierarchyPanel;
        let mut panel = HierarchyPanel::new("Hierarchy");
        let mut ui = crate::EditorUi::new();
        let scene = engine_scene::sample_scene();

        let cmds = panel.ui(&mut ui, &scene);
        // ui() on a scene with entities should return at least create/delete buttons
        // but no mutation commands without user interaction
        assert!(cmds.is_empty());
    }

    // ── EditorError extra variant tests ──────────────────────────────────

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_error_entity_not_found_display() {
        let err = EditorError::EntityNotFound("missing-entity".to_string());
        assert_eq!(err.to_string(), "entity not found: missing-entity");
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_error_component_not_found_display() {
        let err = EditorError::ComponentNotFound("missing-comp".to_string());
        assert_eq!(err.to_string(), "component not found: missing-comp");
    }

    #[cfg(feature = "tooling-editor")]
    #[test]
    fn editor_error_io_failed_display() {
        let err = EditorError::IoFailed("permission denied".to_string());
        assert_eq!(err.to_string(), "I/O error: permission denied");
    }
}
