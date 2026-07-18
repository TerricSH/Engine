#![forbid(unsafe_code)]

use thiserror::Error;

// ---------------------------------------------------------------------------
// EditorError – always available
// ---------------------------------------------------------------------------

/// Errors that can occur during editor operations.
#[derive(Error, Debug)]
pub enum EditorError {
    /// No scene is currently loaded or the requested scene is missing.
    #[error("scene not found")]
    SceneNotFound,

    /// Editor initialisation failed with a contextual message.
    #[error("init failed: {0}")]
    InitFailed(String),

    /// An entity with the requested ID was not found in the scene.
    #[error("entity not found: {0}")]
    EntityNotFound(String),

    /// A component of the requested type was not found on the entity.
    #[error("component not found: {0}")]
    ComponentNotFound(String),

    /// A field-setting command targeted a field that is not part of the
    /// serialized component record.
    #[error("component field not found: {component_type}.{field_name}")]
    ComponentFieldNotFound {
        component_type: String,
        field_name: String,
    },

    /// The entity already owns a component of the requested type.
    #[error("component already exists: {0}")]
    ComponentAlreadyExists(String),

    /// A hierarchy edit would create a cycle or reference an invalid parent.
    #[error("invalid hierarchy: {0}")]
    InvalidHierarchy(String),

    /// Serialized entity records cannot form a self-contained clipboard.
    #[error("invalid entity clipboard: {0}")]
    InvalidEntityClipboard(String),

    /// An entity operation would introduce a duplicate persistent ID.
    #[error("entity persistent ID already exists: {0}")]
    EntityAlreadyExists(String),

    /// Entity clipboard serialization or deserialization failed.
    #[error("entity clipboard serialization failed: {0}")]
    EntityClipboardSerialization(String),

    /// Serialized component data is missing required metadata or targets a
    /// different component type.
    #[error("invalid component clipboard: {0}")]
    InvalidComponentClipboard(String),

    /// Component clipboard serialization or deserialization failed.
    #[error("component clipboard serialization failed: {0}")]
    ComponentClipboardSerialization(String),

    /// A command returned successfully but its resulting Scene failed the
    /// canonical authoring validation boundary.
    #[error("scene command {operation} rejected: {reason}")]
    SceneCommandRejected { operation: String, reason: String },

    /// An I/O operation (read, write, create directory, …) failed.
    #[error("I/O error: {0}")]
    IoFailed(String),
}

// ---------------------------------------------------------------------------
// Full editor implementation behind the `tooling-editor` feature gate
// ---------------------------------------------------------------------------

#[cfg(feature = "tooling-editor")]
pub mod animation_preview;
#[cfg(feature = "tooling-editor")]
pub mod asset_browser;
#[cfg(feature = "tooling-editor")]
pub mod commands;
#[cfg(feature = "tooling-editor")]
pub mod component_catalog;
#[cfg(feature = "tooling-editor")]
pub mod diagnostics;
#[cfg(feature = "tooling-editor")]
pub mod gizmo;
#[cfg(feature = "tooling-editor")]
pub mod gizmo_overlay;
#[cfg(feature = "tooling-editor")]
pub mod io;
#[cfg(feature = "tooling-editor")]
pub mod material_editor;
#[cfg(feature = "tooling-editor")]
mod panels;
#[cfg(feature = "tooling-editor")]
pub mod performance;
#[cfg(feature = "tooling-editor")]
mod play_mode;
#[cfg(feature = "tooling-editor")]
pub mod prefab_authoring;
#[cfg(feature = "tooling-editor")]
pub use commands::{
    AddComponent, AddEntity, Command, CommandBatch, CommandHistory, ComponentClipboard,
    DuplicateEntitySubtree, EntityClipboard, EntityPasteParent, MoveEntitySibling,
    PasteEntityRecords, RemoveComponent, RemoveEntity, ReplaceComponent, SetComponentEnabled,
    SetComponentField, SetEntityEnabled, SetEntityName, SetEntityParent, SetSceneSettings,
    SiblingMove,
};
#[cfg(feature = "tooling-editor")]
pub use diagnostics::{DiagnosticEntry, DiagnosticsPanel};
#[cfg(feature = "tooling-editor")]
pub use io::{default_scene_path, load_scene, save_scene};
#[cfg(feature = "tooling-editor")]
pub use panels::{SceneViewAction, SceneViewPanel};
#[cfg(feature = "tooling-editor")]
pub use play_mode::{EditorPlayMode, EditorPlaySession};
#[cfg(feature = "tooling-editor")]
pub use prefab_authoring::{
    create_prefab_asset_from_scene, load_prefab_source, prefab_from_scene_subtree,
    prepare_prefab_instantiation, prepare_prefab_instantiation_from_registry,
    prepare_prefab_instantiation_from_source, prepare_unpack_prefab, CreatedPrefabAsset,
    PrefabAssetCreateRequest, PrefabAuthoringError, PrefabInstantiationPlan, PrefabUnpackMode,
    PrefabUnpackPlan, PREFAB_SOURCE_SUFFIX,
};
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
    /// Active scene-level Transform gizmo gesture, if any. Preview edits are
    /// deliberately kept out of command history until the gesture commits.
    gizmo_drag: Option<gizmo::SceneGizmoDrag>,
}

#[cfg(feature = "tooling-editor")]
impl EditorScene {
    /// Wrap an existing [`Scene`] in a new editor scene.
    pub fn new(scene: engine_scene::Scene) -> Self {
        Self {
            scene,
            history: CommandHistory::new(),
            selected_entity: None,
            diagnostics: DiagnosticsPanel::new(),
            gizmo_drag: None,
        }
    }

    /// Wrap a scene and install the same shared component registry used by the
    /// runtime's strict Scene -> World loader.
    ///
    /// The initial scene is preflighted before the editor accepts the
    /// registry, so subsequent execute/undo/redo operations cannot start from
    /// an unmaterializable extension-component state.
    pub fn new_with_component_registry(
        scene: engine_scene::Scene,
        component_registry: std::sync::Arc<engine_scene::ComponentRegistry>,
    ) -> Result<Self, EditorError> {
        let mut editor = Self::new(scene);
        editor.install_component_registry(component_registry)?;
        Ok(editor)
    }

    /// Install or replace the registry used for strict command preflight.
    pub fn install_component_registry(
        &mut self,
        component_registry: std::sync::Arc<engine_scene::ComponentRegistry>,
    ) -> Result<(), EditorError> {
        self.history
            .install_component_registry(&self.scene, component_registry)
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
        if self.is_transform_gizmo_drag_active() {
            self.cancel_transform_gizmo_drag();
            return Ok(());
        }
        self.history.undo(&mut self.scene)
    }

    /// Redo the last-undone command.
    pub fn redo(&mut self) -> Result<(), EditorError> {
        if self.is_transform_gizmo_drag_active() {
            self.cancel_transform_gizmo_drag();
            return Ok(());
        }
        self.history.redo(&mut self.scene)
    }

    /// Whether the history has been dirtied since the last [`save`] or
    /// [`mark_clean`].
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// Save the scene to the given path (defaults to
    /// `assets/scenes/{scene_id}.scene.ron`).
    pub fn save(&mut self, path: Option<&std::path::Path>) -> Result<(), EditorError> {
        let p = match path {
            Some(p) => p.to_path_buf(),
            None => std::path::PathBuf::from(io::default_scene_path(&self.scene)),
        };
        io::save_scene(&self.scene, &p)?;
        self.history.mark_clean();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Type alias for the common PersistentId string
// ---------------------------------------------------------------------------

/// Convenience alias for [`engine_serialize::PersistentId`].
pub type PersistentId = engine_serialize::PersistentId;
