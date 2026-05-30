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
